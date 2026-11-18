//! 编译器资源账本核心（`docs/design/compiler-resource-ledger.md` 第 3 节）。
//!
//! 一个 [`Ledger`] 对应宿主发起的一次顶层操作，绑定宿主预算 `B`
//! （`CompilerControlledLiveBytes`），维护存续 `L` 与预留 `R`，任何时刻满足
//! `L + R ≤ B`。编译遍先以计划峰值 `P` 准入（[`Ledger::admit_pass`]），子编译遍从父编译遍
//! 未消费额度中划出（[`PassBudget::admit_sub`]）；编译遍名下同时存续的授予字节 `U` 不超过
//! `P`。受控存储只能经编译遍的存储入口（[`PassBudget::vec`]）创建：入口先取得授予再发起
//! 分配请求，授予随存储移动，存储释放时归还额度。
//!
//! 账本内部状态在互斥锁内维护，授予句柄可随存储跨线程移动；同一账本同一时刻只有一个
//! 活动的顶层编译遍，子编译遍按栈嵌套。账本错误不分配堆内存。

use std::alloc::Layout;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::OperationResourceBudget;

#[cfg(test)]
mod tests;

/// 编译遍计划峰值的受检累加器（资源账本设计第 3.2 节）。
///
/// 按授予请求的 `Layout` 累加；任一步无法表示时整份计划记为无法表示，准入时按宿主预算
/// 超限拒绝（观测值 `u64::MAX`）。互斥的子阶段用 [`Plan::max`] 取峰值。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Plan(Option<u64>);

impl Plan {
    /// 空计划。
    pub(crate) const ZERO: Self = Self(Some(0));

    /// 恰为 `bytes` 字节的计划。
    pub(crate) const fn of(bytes: u64) -> Self {
        Self(Some(bytes))
    }

    /// 加上 `count` 个 `T` 的定长序列请求。
    #[must_use]
    pub(crate) fn array<T>(self, count: usize) -> Self {
        self.plus(Self(
            Layout::array::<T>(count)
                .ok()
                .and_then(|layout| u64::try_from(layout.size()).ok()),
        ))
    }

    /// 加上一次按 `layout` 发起的分配请求；`None` 表示请求无法表示。
    #[must_use]
    pub(crate) fn layout(self, layout: Option<Layout>) -> Self {
        self.plus(Self(
            layout.and_then(|layout| u64::try_from(layout.size()).ok()),
        ))
    }

    /// 两份同时存续的计划之和。
    #[must_use]
    pub(crate) fn plus(self, other: Self) -> Self {
        Self(self.0.zip(other.0).and_then(|(a, b)| a.checked_add(b)))
    }

    /// 加上一张按 `entry_count` 项上界预留的哈希表请求（资源账本设计第 3.2 节）。
    ///
    /// 标准库不公开桶分配布局。计划为每个请求项预留八个桶，并额外计入每桶控制字节与
    /// 一组尾部控制区，覆盖小表最小桶数和负载因子取整，而不依赖哈希表迭代顺序。账本
    /// 计量的是这份保守请求，不是分配器实际给出的容量。
    #[must_use]
    pub(crate) fn hash_table<K, V>(self, entry_count: usize) -> Self {
        self.plus(Self(hash_table_bytes::<K, V>(entry_count)))
    }

    /// 两份互斥子阶段计划的峰值。
    #[must_use]
    pub(crate) fn max(self, other: Self) -> Self {
        Self(self.0.zip(other.0).map(|(a, b)| a.max(b)))
    }

    /// 计划字节；无法表示时为 `None`。
    pub(crate) const fn bytes(self) -> Option<u64> {
        self.0
    }
}

fn hash_table_bytes<K, V>(entry_count: usize) -> Option<u64> {
    if entry_count == 0 {
        return Some(0);
    }
    let bucket_bytes = u64::try_from(size_of::<(K, V)>()).ok()?.checked_add(1)?;
    u64::try_from(entry_count)
        .ok()?
        .checked_mul(8)?
        .checked_mul(bucket_bytes)?
        .checked_add(16)
}

/// 资源报告中工作集峰值的归属阶段（资源账本设计第 7 节）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LedgerStage {
    /// 编译管线。
    CompilePipeline,
    /// 单制品发射。
    SingleEmission,
    /// 差异发射。
    DiffEmission,
    /// 宿主直接调用的公开检查。
    PublicCheck,
}

impl LedgerStage {
    const COUNT: usize = 4;

    const fn index(self) -> usize {
        match self {
            Self::CompilePipeline => 0,
            Self::SingleEmission => 1,
            Self::DiffEmission => 2,
            Self::PublicCheck => 3,
        }
    }
}

/// 编译遍标识：报告归属阶段与记账点名称，供错误定位。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PassLabel {
    /// 工作集峰值的归属阶段。
    pub(crate) stage: LedgerStage,
    /// 记账点名称，只用于定位，不进入任何稳定输出。
    pub(crate) point: &'static str,
}

impl PassLabel {
    /// 构造编译遍标识。
    pub(crate) const fn new(stage: LedgerStage, point: &'static str) -> Self {
        Self { stage, point }
    }
}

/// 记账缺陷的种类。记账缺陷不是资源限额，换用配置档不能绕过。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AccountingDefect {
    /// 已有活动的顶层编译遍时再次准入顶层编译遍。
    NestedTopLevelPass,
    /// 子编译遍计划超出父编译遍未消费额度。
    SubPlanExceedsParent,
    /// 授予超出编译遍计划峰值。
    GrantExceedsPass,
    /// 结束或准入子编译遍的不是栈顶编译遍。
    InactivePass,
    /// 存储写入超出授予的逻辑容量。
    CapacityExceeded,
    /// 编译遍结束时仍有未转为输出的授予。
    UnreturnedGrant,
    /// 账本结束时存续或预留不为零，或仍有活动编译遍。
    LedgerNotSettled,
    /// 按计数求出的计划与实际写入量不一致：同一份构造代码两次运行的写入量不同。
    ShapeMismatch,
}

/// 账本错误。只携带维度数值与标识，不分配堆内存。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LedgerError {
    /// 计划准入超出宿主预算；`requested` 是 `L + R + P`，无法表示时为 `u64::MAX`。
    BudgetExceeded {
        /// 被拒绝的编译遍。
        label: PassLabel,
        /// 准入后将达到的存续与预留之和。
        requested: u64,
        /// 宿主预算 `B`。
        limit: u64,
    },
    /// 记账缺陷。
    Defect {
        /// 出现缺陷的编译遍。
        label: PassLabel,
        /// 缺陷种类。
        defect: AccountingDefect,
    },
    /// 分配器拒绝了已授予的请求；授予已回滚。
    AllocationFailure {
        /// 发起请求的编译遍。
        label: PassLabel,
    },
}

/// 账本在返回点封存的资源报告（资源账本设计第 7 节），单位为受控请求字节。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct LedgerReport {
    /// 账本生命周期内存续 `L` 的最大值。
    pub(crate) peak_live: u64,
    /// 账本生命周期内已承诺字节的最大值：存续与预留之和 `L + R`。
    pub(crate) peak_committed: u64,
    /// 各阶段工作集峰值：该阶段顶层编译遍及其子编译遍同时存续的授予字节之和的最大值。
    pub(crate) stage_peaks: [u64; LedgerStage::COUNT],
    /// 返回点转交宿主的字节。
    pub(crate) transferred: u64,
    /// 实际释放并归还的字节；分配器拒绝后回滚的授予不计入。
    pub(crate) released: u64,
}

impl LedgerReport {
    /// 返回某阶段的工作集峰值。
    pub(crate) const fn stage_peak(&self, stage: LedgerStage) -> u64 {
        self.stage_peaks[stage.index()]
    }
}

/// 编译遍标识：栈深度与账本内单调递增的序号。
///
/// 授予记录创建它的编译遍标识。该编译遍结束后，名下存续转给父编译遍；父编译遍是当时
/// 栈中更浅一层的编译遍，在子编译遍存活期间不会出栈。因此当前栈中深度不超过记录深度、
/// 序号不大于记录序号的最深编译遍，就是授予现在的归属；没有这样的编译遍时归属根账户
/// （不属于任何活动编译遍的存续）。归属由栈与序号直接推出，账本不为已结束的编译遍保留
/// 任何记录，反复准入编译遍不会使账本状态增长。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PassId {
    depth: usize,
    id: u64,
}

struct Frame {
    id: u64,
    label: PassLabel,
    plan: u64,
    in_use: u64,
    carved: u64,
}

impl Frame {
    /// 本编译遍尚可授予或划给子编译遍的额度。
    const fn unconsumed(&self) -> u64 {
        self.plan - self.in_use - self.carved
    }
}

#[derive(Clone, Copy)]
enum Outcome {
    Released,
    Transferred,
    RolledBack,
}

struct State {
    budget: u64,
    live: u64,
    reserved: u64,
    report: LedgerReport,
    frames: Vec<Frame>,
    next_id: u64,
}

impl State {
    /// 授予现在的归属编译遍在栈中的位置；`None` 表示根账户。
    fn resolve(&self, pass: PassId) -> Option<usize> {
        let deepest = pass.depth.min(self.frames.len().checked_sub(1)?);
        (0..=deepest).rev().find(|&k| self.frames[k].id <= pass.id)
    }

    fn frame(&self, pass: PassId) -> Option<&Frame> {
        self.frames
            .get(pass.depth)
            .filter(|frame| frame.id == pass.id)
    }

    fn frame_mut(&mut self, pass: PassId) -> Option<&mut Frame> {
        self.frames
            .get_mut(pass.depth)
            .filter(|frame| frame.id == pass.id)
    }

    fn top_label(&self, pass: PassId) -> Result<PassLabel, AccountingDefect> {
        match self.frame(pass) {
            Some(frame) if pass.depth + 1 == self.frames.len() => Ok(frame.label),
            _ => Err(AccountingDefect::InactivePass),
        }
    }

    fn label_of(&self, pass: PassId) -> PassLabel {
        self.frame(pass).map_or(INACTIVE_LABEL, |frame| frame.label)
    }

    fn push_frame(&mut self, label: PassLabel, plan: u64) -> PassId {
        let pass = PassId {
            depth: self.frames.len(),
            id: self.next_id,
        };
        self.next_id += 1;
        self.frames.push(Frame {
            id: pass.id,
            label,
            plan,
            in_use: 0,
            carved: 0,
        });
        pass
    }

    fn record_peaks(&mut self) {
        self.report.peak_live = self.report.peak_live.max(self.live);
        self.record_committed(self.live.saturating_add(self.reserved));
        if let Some(stage) = self.frames.first().map(|frame| frame.label.stage) {
            let working_set = self.frames.iter().map(|frame| frame.in_use).sum::<u64>();
            let peak = &mut self.report.stage_peaks[stage.index()];
            *peak = (*peak).max(working_set);
        }
    }

    fn record_committed(&mut self, committed: u64) {
        self.report.peak_committed = self.report.peak_committed.max(committed);
    }

    fn grant(&mut self, pass: PassId, bytes: u64) -> Result<(), LedgerError> {
        let label = self.label_of(pass);
        let frame = self.frame_mut(pass).ok_or(LedgerError::Defect {
            label,
            defect: AccountingDefect::InactivePass,
        })?;
        if bytes > frame.unconsumed() {
            return Err(LedgerError::Defect {
                label,
                defect: AccountingDefect::GrantExceedsPass,
            });
        }
        frame.in_use += bytes;
        self.reserved -= bytes;
        self.live += bytes;
        self.record_peaks();
        Ok(())
    }

    fn give_back(&mut self, pass: Option<PassId>, bytes: u64, outcome: Outcome) {
        debug_assert!(self.live >= bytes, "账本存续不足以归还授予");
        self.live = self.live.saturating_sub(bytes);
        match outcome {
            Outcome::Released => self.report.released += bytes,
            Outcome::Transferred => self.report.transferred += bytes,
            Outcome::RolledBack => {}
        }
        if let Some(index) = pass.and_then(|pass| self.resolve(pass)) {
            let frame = &mut self.frames[index];
            debug_assert!(frame.in_use >= bytes, "编译遍在用量不足以归还授予");
            frame.in_use = frame.in_use.saturating_sub(bytes);
            self.reserved += bytes;
        }
    }

    /// 弹出栈顶编译遍：退还未消费额度，名下存续转给父编译遍或根账户。
    fn pop_frame(&mut self) {
        let Some(frame) = self.frames.pop() else {
            return;
        };
        debug_assert_eq!(frame.carved, 0, "子编译遍须先于父编译遍结束");
        let unconsumed = frame.plan - frame.in_use - frame.carved;
        match self.frames.last_mut() {
            Some(parent) => {
                // 子编译遍的计划额度原本计在父编译遍的划出量中：额度归还父编译遍，名下
                // 存续转为父编译遍的在用量，预留总量不变。
                parent.carved -= frame.plan;
                parent.in_use += frame.in_use;
            }
            None => self.reserved -= unconsumed,
        }
    }
}

const INACTIVE_LABEL: PassLabel = PassLabel::new(LedgerStage::CompilePipeline, "inactive-pass");

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 一次顶层操作的资源账本。
pub(crate) struct Ledger {
    state: Arc<Mutex<State>>,
}

impl Ledger {
    /// 以配置档的宿主预算创建空账本。
    pub(crate) fn new(limits: &OperationResourceBudget) -> Self {
        Self::with_budget(
            limits.get(crate::OperationResourceDimension::CompilerControlledLiveBytes),
        )
    }

    /// 以给定宿主预算创建空账本。
    pub(crate) fn with_budget(budget: u64) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                budget,
                live: 0,
                reserved: 0,
                report: LedgerReport::default(),
                frames: Vec::new(),
                next_id: 0,
            })),
        }
    }

    /// 准入顶层编译遍：`L + R + P ≤ B` 成立时 `R += P`，否则按宿主预算拒绝，此时尚未分配。
    pub(crate) fn admit_pass(
        &self,
        label: PassLabel,
        plan: Plan,
    ) -> Result<PassBudget<'_>, LedgerError> {
        let mut state = lock(&self.state);
        if !state.frames.is_empty() {
            return Err(LedgerError::Defect {
                label,
                defect: AccountingDefect::NestedTopLevelPass,
            });
        }
        let requested = plan.bytes().and_then(|plan| {
            state
                .live
                .checked_add(state.reserved)?
                .checked_add(plan)
                .map(|requested| (plan, requested))
        });
        let plan = match requested {
            Some((plan, requested)) if requested <= state.budget => plan,
            _ => {
                return Err(LedgerError::BudgetExceeded {
                    label,
                    requested: requested.map_or(u64::MAX, |(_, requested)| requested),
                    limit: state.budget,
                });
            }
        };
        state.reserved += plan;
        let committed = state.live + state.reserved;
        state.record_committed(committed);
        let account = state.push_frame(label, plan);
        drop(state);
        Ok(PassBudget {
            ledger: self,
            account,
            label,
            finished: false,
        })
    }

    /// 计入按值移入本次操作的初始存续（资源账本设计第 3.1 节），返回归属根账户的授予。
    ///
    /// 初始存续是移入时已经存在的存储，不经计划准入；若它已超出宿主预算，此后任何计划
    /// 准入都按 `L + R + P` 拒绝，其间不发生受控分配。
    pub(crate) fn adopt(&self, bytes: u64) -> Grant {
        let mut state = lock(&self.state);
        state.live = state.live.saturating_add(bytes);
        state.record_peaks();
        drop(state);
        Grant {
            state: Some(Arc::clone(&self.state)),
            account: None,
            bytes,
        }
    }

    /// 当前存续 `L`。
    pub(crate) fn live(&self) -> u64 {
        lock(&self.state).live
    }

    /// 当前预留 `R`。
    pub(crate) fn reserved(&self) -> u64 {
        lock(&self.state).reserved
    }

    /// 结束账本并封存资源报告：须已无活动编译遍，且转交之后 `L = R = 0`。
    pub(crate) fn close(self) -> Result<LedgerReport, LedgerError> {
        let state = lock(&self.state);
        if !state.frames.is_empty() || state.live != 0 || state.reserved != 0 {
            return Err(LedgerError::Defect {
                label: state
                    .frames
                    .last()
                    .map_or(INACTIVE_LABEL, |frame| frame.label),
                defect: AccountingDefect::LedgerNotSettled,
            });
        }
        Ok(state.report)
    }
}

/// 测试用、额度不受限的编译遍；账本泄漏为 `'static`，只在测试中使用。
#[cfg(test)]
pub(crate) fn test_pass() -> PassBudget<'static> {
    let ledger: &'static Ledger = Box::leak(Box::new(Ledger::with_budget(u64::MAX)));
    ledger
        .admit_pass(
            PassLabel::new(LedgerStage::CompilePipeline, "test"),
            Plan::of(u64::MAX),
        )
        .expect("不受限的测试编译遍")
}

/// 一个已准入编译遍的预算。随作用域结束（未调用 [`PassBudget::finish`]）时按取消处理：
/// 退还未消费预留，名下仍存续的授予转给父编译遍或根账户，由其所有者析构时归还。
pub(crate) struct PassBudget<'l> {
    ledger: &'l Ledger,
    account: PassId,
    /// 准入时确定的编译遍标识；存储入口据此标注错误，无需再访问账本。
    label: PassLabel,
    finished: bool,
}

impl PassBudget<'_> {
    /// 从本编译遍未消费额度中划出子编译遍计划；超出父额度属记账缺陷。
    pub(crate) fn admit_sub(
        &self,
        label: PassLabel,
        plan: Plan,
    ) -> Result<PassBudget<'_>, LedgerError> {
        let mut state = lock(&self.ledger.state);
        let parent_label = state
            .top_label(self.account)
            .map_err(|defect| LedgerError::Defect { label, defect })?;
        let parent = state.frames.last_mut().expect("栈顶编译遍已在上一步确认");
        let plan = match plan.bytes() {
            Some(plan) if plan <= parent.unconsumed() => plan,
            _ => {
                return Err(LedgerError::Defect {
                    label: parent_label,
                    defect: AccountingDefect::SubPlanExceedsParent,
                });
            }
        };
        parent.carved += plan;
        let account = state.push_frame(label, plan);
        drop(state);
        Ok(PassBudget {
            ledger: self.ledger,
            account,
            label,
            finished: false,
        })
    }

    /// 本编译遍的标识。
    pub(crate) const fn label(&self) -> PassLabel {
        self.label
    }

    /// 本编译遍当前在用量 `U`。
    #[cfg(test)]
    pub(crate) fn in_use(&self) -> u64 {
        let state = lock(&self.ledger.state);
        state.frame(self.account).map_or(0, |frame| frame.in_use)
    }

    /// 结束编译遍：`output_bytes` 是转为输出、继续存续的授予字节，必须等于当前在用量；
    /// 其余授予须已全部释放。输出留在 `L` 中，转给父编译遍或根账户，不重复记账。
    pub(crate) fn finish(mut self, output_bytes: u64) -> Result<(), LedgerError> {
        let mut state = lock(&self.ledger.state);
        let label = state
            .top_label(self.account)
            .map_err(|defect| LedgerError::Defect {
                label: state.label_of(self.account),
                defect,
            })?;
        let in_use = state.frames.last().map_or(0, |frame| frame.in_use);
        self.finished = true;
        state.pop_frame();
        if in_use == output_bytes {
            Ok(())
        } else {
            Err(LedgerError::Defect {
                label,
                defect: AccountingDefect::UnreturnedGrant,
            })
        }
    }

    /// 以授予创建可用逻辑容量为 `capacity` 的空序列。授予按 `Layout::array::<T>` 的受检
    /// 大小计费；分配器拒绝时回滚授予并返回 [`LedgerError::AllocationFailure`]。
    pub(crate) fn vec<T>(&self, capacity: usize) -> Result<BoundedVec<T>, LedgerError> {
        let label = self.label;
        // 无法表示的请求按宿主预算超限处理（资源账本设计第 3.2 节）。
        let bytes = Layout::array::<T>(capacity)
            .ok()
            .and_then(|layout| u64::try_from(layout.size()).ok())
            .ok_or_else(|| self.unrepresentable())?;
        let grant = self.grant(bytes)?;
        let mut items = Vec::new();
        if items.try_reserve_exact(capacity).is_err() {
            grant.rollback();
            return Err(LedgerError::AllocationFailure { label });
        }
        Ok(BoundedVec {
            items,
            capacity,
            grant,
            label,
        })
    }

    /// 以授予创建可容纳 `capacity` 项的哈希表；授予按 [`Plan::hash_table`] 的保守请求计费，
    /// 存储按 `capacity` 一次预留，不自行扩容。
    pub(crate) fn hash_map<K: Eq + std::hash::Hash, V>(
        &self,
        capacity: usize,
    ) -> Result<BoundedHashMap<K, V>, LedgerError> {
        let label = self.label;
        let bytes = hash_table_bytes::<K, V>(capacity).ok_or_else(|| self.unrepresentable())?;
        let grant = self.grant(bytes)?;
        let mut items = HashMap::new();
        if items.try_reserve(capacity).is_err() {
            grant.rollback();
            return Err(LedgerError::AllocationFailure { label });
        }
        Ok(BoundedHashMap {
            items,
            capacity,
            _grant: grant,
            label,
        })
    }

    /// 以授予创建恰好容纳 `items` 全部元素的序列并逐项写入；授予按迭代器声明的长度计费。
    pub(crate) fn collect<T>(
        &self,
        items: impl ExactSizeIterator<Item = T>,
    ) -> Result<BoundedVec<T>, LedgerError> {
        let mut values = self.vec(items.len())?;
        values.try_extend(items)?;
        Ok(values)
    }

    /// 同 [`PassBudget::collect`]，逐项产出可能失败；首个错误原样返回，已授予的存储随之释放。
    pub(crate) fn try_collect<T, E: From<LedgerError>>(
        &self,
        items: impl ExactSizeIterator<Item = Result<T, E>>,
    ) -> Result<BoundedVec<T>, E> {
        let mut values = self.vec(items.len())?;
        for item in items {
            values.push(item?)?;
        }
        Ok(values)
    }

    /// 为随后由调用方发起的一次分配请求取得授予，按 `layout` 的大小计费。
    ///
    /// 只用于账本存储入口无法直接创建的存储（如 `Arc<[u8]>` 的引用计数分配）：调用方
    /// 取得授予后立即发起请求，并让授予与该存储同生命周期持有。
    pub(crate) fn grant_layout(&self, layout: Layout) -> Result<Grant, LedgerError> {
        let bytes = u64::try_from(layout.size()).map_err(|_| {
            let state = lock(&self.ledger.state);
            LedgerError::BudgetExceeded {
                label: state.label_of(self.account),
                requested: u64::MAX,
                limit: state.budget,
            }
        })?;
        self.grant(bytes)
    }

    /// 为记录内嵌的变长载荷集中取得一笔授予（资源账本设计第 3.4 节）。
    ///
    /// 只用于随外层记录逐项分配、无法由账本存储入口直接创建的内层载荷（如策略值中的
    /// 规则表、字符串副本）：载荷字节按计数计划求出，授予与外层表同生命周期持有。
    pub(crate) fn grant_bytes(&self, bytes: u64) -> Result<Grant, LedgerError> {
        self.grant(bytes)
    }

    /// 无法表示的请求：按宿主预算超限报告。
    fn unrepresentable(&self) -> LedgerError {
        LedgerError::BudgetExceeded {
            label: self.label,
            requested: u64::MAX,
            limit: lock(&self.ledger.state).budget,
        }
    }

    /// 零字节授予不改变 `L`，不访问账本，避免空存储（如空的伴随来源列表）逐个加锁。
    fn grant(&self, bytes: u64) -> Result<Grant, LedgerError> {
        if bytes == 0 {
            return Ok(Grant {
                state: None,
                account: Some(self.account),
                bytes: 0,
            });
        }
        lock(&self.ledger.state).grant(self.account, bytes)?;
        Ok(Grant {
            state: Some(Arc::clone(&self.ledger.state)),
            account: Some(self.account),
            bytes,
        })
    }
}

impl Drop for PassBudget<'_> {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let mut state = lock(&self.ledger.state);
        if state.top_label(self.account).is_ok() {
            state.pop_frame();
        }
    }
}

/// 一笔授予：随存储持有与移动，存储释放时归还额度。
pub(crate) struct Grant {
    state: Option<Arc<Mutex<State>>>,
    /// 创建授予的编译遍；`None` 表示根账户（[`Ledger::adopt`] 的初始存续）。
    account: Option<PassId>,
    bytes: u64,
}

impl Grant {
    fn settle(&mut self, outcome: Outcome) {
        if let Some(state) = self.state.take() {
            lock(&state).give_back(self.account, self.bytes, outcome);
        }
    }

    /// 分配器拒绝请求：回滚授予，不记为已释放。
    fn rollback(mut self) {
        self.settle(Outcome::RolledBack);
    }

    /// 授予的受控请求字节。
    pub(crate) const fn bytes(&self) -> u64 {
        self.bytes
    }

    /// 返回点转交宿主：额度移出 `L`，记为转交宿主。
    pub(crate) fn transfer_to_host(mut self) {
        self.settle(Outcome::Transferred);
    }
}

impl Drop for Grant {
    fn drop(&mut self) {
        self.settle(Outcome::Released);
    }
}

/// 阶段输出持有的授予：随输出移动、释放时归还，返回点转交宿主。
///
/// 输出结构的相等比较与调试输出只看语义内容，不比较授予；空值表示不受账本约束。
#[derive(Default)]
pub(crate) struct OutputGrant(Option<Grant>);

impl OutputGrant {
    /// 持有的授予字节；已转交宿主或为空时为零。
    pub(crate) fn bytes(&self) -> u64 {
        self.0.as_ref().map_or(0, Grant::bytes)
    }

    /// 持有一笔授予。
    pub(crate) const fn new(grant: Grant) -> Self {
        Self(Some(grant))
    }

    /// 返回点转交宿主；之后不再受账本约束。
    pub(crate) fn transfer_to_host(&mut self) {
        if let Some(grant) = self.0.take() {
            grant.transfer_to_host();
        }
    }
}

impl PartialEq for OutputGrant {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for OutputGrant {}

impl std::fmt::Debug for OutputGrant {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("OutputGrant")
    }
}

/// 由授予约束逻辑容量的序列；不自行扩容，写入超出授予容量属记账缺陷。
pub(crate) struct BoundedVec<T> {
    items: Vec<T>,
    capacity: usize,
    grant: Grant,
    label: PassLabel,
}

impl<T> BoundedVec<T> {
    /// 授予的逻辑容量。
    pub(crate) const fn capacity(&self) -> usize {
        self.capacity
    }

    /// 授予的受控请求字节。
    pub(crate) const fn granted_bytes(&self) -> u64 {
        self.grant.bytes
    }

    const fn overflow(&self) -> LedgerError {
        LedgerError::Defect {
            label: self.label,
            defect: AccountingDefect::CapacityExceeded,
        }
    }

    /// 追加一个元素；超出授予容量时不写入并返回记账缺陷。
    pub(crate) fn push(&mut self, value: T) -> Result<(), LedgerError> {
        if self.items.len() == self.capacity {
            return Err(self.overflow());
        }
        self.items.push(value);
        Ok(())
    }

    /// 返回点转交宿主：授予记为转交宿主，返回不再受账本约束的存储。
    #[cfg(test)]
    pub(crate) fn transfer_to_host(self) -> Vec<T> {
        let Self { items, grant, .. } = self;
        grant.transfer_to_host();
        items
    }
}

impl<T> BoundedVec<T> {
    /// 逐项追加迭代器产出的元素；超出授予容量时停在容量处并返回记账缺陷。
    pub(crate) fn try_extend(
        &mut self,
        values: impl IntoIterator<Item = T>,
    ) -> Result<(), LedgerError> {
        for value in values {
            self.push(value)?;
        }
        Ok(())
    }

    /// 移除并返回最后一个元素；逻辑容量不变。
    pub(crate) fn pop(&mut self) -> Option<T> {
        self.items.pop()
    }

    /// 清空元素；逻辑容量不变，授予继续持有。
    pub(crate) fn clear(&mut self) {
        self.items.clear();
    }

    /// 截断到 `len` 个元素；逻辑容量不变。
    pub(crate) fn truncate(&mut self, len: usize) {
        self.items.truncate(len);
    }

    /// 按序移出全部元素；逻辑容量与授予不变。
    pub(crate) fn drain(&mut self) -> std::vec::Drain<'_, T> {
        self.items.drain(..)
    }

    /// 只保留满足条件的元素；逻辑容量不变。
    pub(crate) fn retain(&mut self, keep: impl FnMut(&T) -> bool) {
        self.items.retain(keep);
    }

    /// 按键去掉相邻重复元素；逻辑容量不变。
    pub(crate) fn dedup_by_key<K: PartialEq>(&mut self, key: impl FnMut(&mut T) -> K) {
        self.items.dedup_by_key(key);
    }

    /// 去掉相邻重复元素；逻辑容量不变。
    pub(crate) fn dedup(&mut self)
    where
        T: PartialEq,
    {
        self.items.dedup();
    }
}

impl<T: Clone> BoundedVec<T> {
    /// 把长度调整为 `len`，新增位置填 `value`；超出授予容量时不改变并返回记账缺陷。
    pub(crate) fn resize(&mut self, len: usize, value: T) -> Result<(), LedgerError> {
        if len > self.capacity {
            return Err(self.overflow());
        }
        self.items.resize(len, value);
        Ok(())
    }

    /// 追加一段元素；超出授予容量时整体不写入并返回记账缺陷。
    pub(crate) fn extend_from_slice(&mut self, values: &[T]) -> Result<(), LedgerError> {
        if values.len() > self.capacity - self.items.len() {
            return Err(self.overflow());
        }
        self.items.extend_from_slice(values);
        Ok(())
    }
}

impl<T> Deref for BoundedVec<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        &self.items
    }
}

impl<'v, T> IntoIterator for &'v BoundedVec<T> {
    type Item = &'v T;
    type IntoIter = std::slice::Iter<'v, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

impl<'v, T> IntoIterator for &'v mut BoundedVec<T> {
    type Item = &'v mut T;
    type IntoIter = std::slice::IterMut<'v, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter_mut()
    }
}

impl<T> DerefMut for BoundedVec<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.items
    }
}

impl<T> BoundedVec<T> {
    /// 冻结为定长切片：须恰好写满授予的逻辑容量，授予随切片持有。
    ///
    /// 写入量少于计划容量说明计数计划与构造代码不一致（[`AccountingDefect::ShapeMismatch`]），
    /// 此时不冻结，避免收缩容量时发起未经授予的分配请求。
    pub(crate) fn freeze(self) -> Result<Frozen<T>, LedgerError> {
        let Self {
            items,
            capacity,
            grant,
            label,
        } = self;
        if items.len() != capacity {
            return Err(LedgerError::Defect {
                label,
                defect: AccountingDefect::ShapeMismatch,
            });
        }
        Ok(Frozen {
            items: items.into_boxed_slice(),
            grant: OutputGrant::new(grant),
        })
    }
}

/// 持有授予的定长切片：随所有者移动、释放时归还，返回点转交宿主。
///
/// 相等比较与调试输出只看元素，不比较授予。
pub(crate) struct Frozen<T> {
    items: Box<[T]>,
    grant: OutputGrant,
}

impl<T> Frozen<T> {
    /// 持有的授予字节；已转交宿主或不受账本约束时为零。
    pub(crate) fn granted_bytes(&self) -> u64 {
        self.grant.bytes()
    }

    /// 返回点转交宿主；之后不再受账本约束。
    pub(crate) fn transfer_to_host(&mut self) {
        self.grant.transfer_to_host();
    }
}

impl<T> Default for Frozen<T> {
    fn default() -> Self {
        Self {
            items: Box::default(),
            grant: OutputGrant::default(),
        }
    }
}

impl<T> Deref for Frozen<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        &self.items
    }
}

impl<T> DerefMut for Frozen<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.items
    }
}

impl<'v, T> IntoIterator for &'v Frozen<T> {
    type Item = &'v T;
    type IntoIter = std::slice::Iter<'v, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

impl<'v, T> IntoIterator for &'v mut Frozen<T> {
    type Item = &'v mut T;
    type IntoIter = std::slice::IterMut<'v, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter_mut()
    }
}

/// 测试装配：不受账本约束的定长切片，用于直接改写阶段表。
#[cfg(test)]
impl<T> From<Vec<T>> for Frozen<T> {
    fn from(items: Vec<T>) -> Self {
        Self {
            items: items.into_boxed_slice(),
            grant: OutputGrant::default(),
        }
    }
}

#[cfg(test)]
impl<T> FromIterator<T> for Frozen<T> {
    fn from_iter<I: IntoIterator<Item = T>>(items: I) -> Self {
        items.into_iter().collect::<Vec<_>>().into()
    }
}

#[cfg(test)]
impl<T> Frozen<T> {
    /// 测试装配：取出元素并释放授予。
    pub(crate) fn into_vec(self) -> Vec<T> {
        self.items.into_vec()
    }
}

impl<T: PartialEq> PartialEq for Frozen<T> {
    fn eq(&self, other: &Self) -> bool {
        self.items == other.items
    }
}

impl<T: Eq> Eq for Frozen<T> {}

impl<T: std::fmt::Debug> std::fmt::Debug for Frozen<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.items.fmt(formatter)
    }
}

/// 由授予约束项数的哈希表；按容量一次预留，不自行扩容，插入超出容量属记账缺陷。
pub(crate) struct BoundedHashMap<K, V> {
    items: HashMap<K, V>,
    capacity: usize,
    _grant: Grant,
    label: PassLabel,
}

impl<K: Eq + std::hash::Hash, V> BoundedHashMap<K, V> {
    /// 插入一项并返回同键的旧值；新键超出授予容量时不写入并返回记账缺陷。
    pub(crate) fn insert(&mut self, key: K, value: V) -> Result<Option<V>, LedgerError> {
        if self.items.len() == self.capacity && !self.items.contains_key(&key) {
            return Err(LedgerError::Defect {
                label: self.label,
                defect: AccountingDefect::CapacityExceeded,
            });
        }
        Ok(self.items.insert(key, value))
    }
}

impl<K, V> Deref for BoundedHashMap<K, V> {
    type Target = HashMap<K, V>;

    fn deref(&self) -> &HashMap<K, V> {
        &self.items
    }
}
