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

// 本模块是 #1296 第一步的账本核心；发射、差异发射与公开检查入口在同一 Stack 的上层
// 迁入，届时去掉此项。
#![cfg_attr(not(test), allow(dead_code))]

use std::alloc::Layout;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::{CompileLimitDimension, CompileLimits};

#[cfg(test)]
mod tests;

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

/// 授予的记账归属：根账户（不属于任何活动编译遍的存续）、活动编译遍，或已结束编译遍
/// 转交给的账户。
#[derive(Clone, Copy)]
enum Account {
    Root,
    Active,
    Redirect(usize),
}

const ROOT: usize = 0;

struct Frame {
    account: usize,
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
    accounts: Vec<Account>,
}

impl State {
    fn resolve(&self, mut account: usize) -> usize {
        while let Account::Redirect(next) = self.accounts[account] {
            account = next;
        }
        account
    }

    fn frame_mut(&mut self, account: usize) -> Option<&mut Frame> {
        self.frames
            .iter_mut()
            .find(|frame| frame.account == account)
    }

    fn top_label(&self, account: usize) -> Result<PassLabel, AccountingDefect> {
        match self.frames.last() {
            Some(top) if top.account == account => Ok(top.label),
            _ => Err(AccountingDefect::InactivePass),
        }
    }

    fn label_of(&self, account: usize) -> PassLabel {
        self.frames
            .iter()
            .find(|frame| frame.account == account)
            .map_or(INACTIVE_LABEL, |frame| frame.label)
    }

    fn push_frame(&mut self, label: PassLabel, plan: u64) -> usize {
        let account = self.accounts.len();
        self.accounts.push(Account::Active);
        self.frames.push(Frame {
            account,
            label,
            plan,
            in_use: 0,
            carved: 0,
        });
        account
    }

    fn record_peaks(&mut self) {
        self.report.peak_live = self.report.peak_live.max(self.live);
        if let Some(stage) = self.frames.first().map(|frame| frame.label.stage) {
            let working_set = self.frames.iter().map(|frame| frame.in_use).sum::<u64>();
            let peak = &mut self.report.stage_peaks[stage.index()];
            *peak = (*peak).max(working_set);
        }
    }

    fn grant(&mut self, account: usize, bytes: u64) -> Result<(), LedgerError> {
        let label = self.label_of(account);
        let frame = self.frame_mut(account).ok_or(LedgerError::Defect {
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

    fn give_back(&mut self, account: usize, bytes: u64, outcome: Outcome) {
        debug_assert!(self.live >= bytes, "账本存续不足以归还授予");
        self.live = self.live.saturating_sub(bytes);
        match outcome {
            Outcome::Released => self.report.released += bytes,
            Outcome::Transferred => self.report.transferred += bytes,
            Outcome::RolledBack => {}
        }
        let account = self.resolve(account);
        if let Some(frame) = self.frame_mut(account) {
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
                self.accounts[frame.account] = Account::Redirect(parent.account);
            }
            None => {
                self.reserved -= unconsumed;
                self.accounts[frame.account] = Account::Redirect(ROOT);
            }
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
    pub(crate) fn new(limits: &CompileLimits) -> Self {
        Self::with_budget(limits.value(CompileLimitDimension::CompilerControlledLiveBytes))
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
                accounts: vec![Account::Root],
            })),
        }
    }

    /// 准入顶层编译遍：`L + R + P ≤ B` 成立时 `R += P`，否则按宿主预算拒绝，此时尚未分配。
    pub(crate) fn admit_pass(
        &self,
        label: PassLabel,
        plan: u64,
    ) -> Result<PassBudget<'_>, LedgerError> {
        let mut state = lock(&self.state);
        if !state.frames.is_empty() {
            return Err(LedgerError::Defect {
                label,
                defect: AccountingDefect::NestedTopLevelPass,
            });
        }
        let requested = state
            .live
            .checked_add(state.reserved)
            .and_then(|held| held.checked_add(plan));
        match requested {
            Some(requested) if requested <= state.budget => {}
            _ => {
                return Err(LedgerError::BudgetExceeded {
                    label,
                    requested: requested.unwrap_or(u64::MAX),
                    limit: state.budget,
                });
            }
        }
        state.reserved += plan;
        let account = state.push_frame(label, plan);
        drop(state);
        Ok(PassBudget {
            ledger: self,
            account,
            finished: false,
        })
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

/// 一个已准入编译遍的预算。随作用域结束（未调用 [`PassBudget::finish`]）时按取消处理：
/// 退还未消费预留，名下仍存续的授予转给父编译遍或根账户，由其所有者析构时归还。
pub(crate) struct PassBudget<'l> {
    ledger: &'l Ledger,
    account: usize,
    finished: bool,
}

impl PassBudget<'_> {
    /// 从本编译遍未消费额度中划出子编译遍计划；超出父额度属记账缺陷。
    pub(crate) fn admit_sub(
        &self,
        label: PassLabel,
        plan: u64,
    ) -> Result<PassBudget<'_>, LedgerError> {
        let mut state = lock(&self.ledger.state);
        let parent_label = state
            .top_label(self.account)
            .map_err(|defect| LedgerError::Defect { label, defect })?;
        let parent = state.frames.last_mut().expect("栈顶编译遍已在上一步确认");
        if plan > parent.unconsumed() {
            return Err(LedgerError::Defect {
                label: parent_label,
                defect: AccountingDefect::SubPlanExceedsParent,
            });
        }
        parent.carved += plan;
        let account = state.push_frame(label, plan);
        drop(state);
        Ok(PassBudget {
            ledger: self.ledger,
            account,
            finished: false,
        })
    }

    /// 本编译遍当前在用量 `U`。
    pub(crate) fn in_use(&self) -> u64 {
        let state = lock(&self.ledger.state);
        state
            .frames
            .iter()
            .find(|frame| frame.account == self.account)
            .map_or(0, |frame| frame.in_use)
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
        let (label, budget) = {
            let state = lock(&self.ledger.state);
            (state.label_of(self.account), state.budget)
        };
        // 无法表示的请求按宿主预算超限处理（资源账本设计第 3.2 节）。
        let bytes = Layout::array::<T>(capacity)
            .ok()
            .and_then(|layout| u64::try_from(layout.size()).ok())
            .ok_or(LedgerError::BudgetExceeded {
                label,
                requested: u64::MAX,
                limit: budget,
            })?;
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

    fn grant(&self, bytes: u64) -> Result<Grant, LedgerError> {
        lock(&self.ledger.state).grant(self.account, bytes)?;
        Ok(Grant {
            state: Some(Arc::clone(&self.ledger.state)),
            account: self.account,
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
struct Grant {
    state: Option<Arc<Mutex<State>>>,
    account: usize,
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

    /// 返回点转交宿主：额度移出 `L`，记为转交宿主。
    fn transfer_to_host(mut self) {
        self.settle(Outcome::Transferred);
    }
}

impl Drop for Grant {
    fn drop(&mut self) {
        self.settle(Outcome::Released);
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
    pub(crate) fn transfer_to_host(self) -> Vec<T> {
        let Self { items, grant, .. } = self;
        grant.transfer_to_host();
        items
    }
}

impl<T: Clone> BoundedVec<T> {
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

impl<T> DerefMut for BoundedVec<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.items
    }
}
