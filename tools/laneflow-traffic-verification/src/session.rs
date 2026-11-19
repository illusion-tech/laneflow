//! 验证会话：阶段协议、义务生命周期、失败终态与保存恢复。
//!
//! 会话消费事实并产生结果，不执行交通命令、不持有可变世界（设计 §3.1）。
//! 固定阶段协议（§6）：
//!
//! ```text
//! Bound → BeforeCommands → ProcessingCommands → AfterCommands
//!       → AwaitingStepOutcome → （成功）BeforeCommands(下一轮) … → Finished
//! ```
//!
//! 覆盖义务按**义务单位**分别建立与核销（§3.3"只检查初态、漏遍历均不能
//! 通过"）：五种单位的"轮"不是同一种东西——
//!
//! | 义务 | 履行单位 | 核销点 |
//! | --- | --- | --- |
//! | 初态检查 | 会话的初始边界（一次性） | 首个拍前形成时 |
//! | 命令结果检查 | 每一条实际命令结果 | 该条结果被接受时 |
//! | 拍前检查 | 每轮拍前边界（失败重试不重开） | 步进发起时 |
//! | 成功步进检查 | 每一次成功提交（首轮独立于初态） | 结果核销时 |
//! | 预期失败检查 | 每一次失败尝试 | 失败被接受时 |
//!
//! 初态与步进是两个独立义务实例：`RuleDescriptor::initial` 显式声明规则
//! 也参与初态观测（初态只履行规约明确声明的初态义务）；仅声明拍后规则
//! 的合法规约可以在第一步之前正常启动。标记存储在绑定期按固定规则索引
//! 分配并跨轮复用，无逐拍分配；累计触发计数与当前义务状态分别保存。
//!
//! 规则发现违反使会话进入不可通过的失败终态；`finish` 是消费式一次性
//! 操作——无论通过或拒绝，会话就此进入终态，不得补记后再次终结（§3.3/
//! §6）。回执字段私有，只能由成功的 `finish` 构造。
//!
//! 检查点（§7）保存当前开放义务与累计状态；恢复逐名校验并核对可达性
//! 不变量（步进/失败/命令标记在可建检查点的边界恒为空、初态标记仅存于
//! 首步之前、拍前标记仅在拍前阶段），保证连续执行与恢复执行等值。

use std::collections::{BTreeMap, BTreeSet};

use crate::spec::{
    CheckPhase, CheckedPlan, CoverageMode, PlanRejected, ProvidedModules, RuleDescriptor, RuleId,
    VerificationSpec, WitnessId,
};

/// 检查点结构版本；升级按显式版本规则处理，旧材料不编辑字段转标。
pub const CHECKPOINT_SCHEMA: u32 = 2;

/// 边界上的命令阶段：本边界命令执行前还是全部执行后（§7 不允许含义
/// 不明的“当前 tick”）。
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum CommandPhase {
    BeforeCommands,
    AfterCommands,
}

/// 一条边界命令的实际结果（§6 命令结果阶段）：按实际顺序逐条消费。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandResult {
    /// 成功执行：推进成功命令计数。
    Succeeded,
    /// Blocked：只增加尝试与拒绝统计，不推进成功命令计数、代次或位置。
    Blocked,
    /// 失败尝试：与成功一样被记录，不得跳过，也不得把尝试数当成功游标。
    FailedAttempt,
}

/// 阶段协议违规。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    InitialBoundaryTwice,
    CommandsBeforeInitialBoundary,
    /// 同一边界重复声明命令批次（未完成义务不得被覆盖）。
    CommandsAlreadyDeclared,
    /// 不在命令处理阶段消费结果（含越阶段消费）。
    CommandResultOutsideBatch,
    /// 消费数超过声明数。
    CommandResultOverBatch {
        declared: u32,
        consumed: u32,
    },
    /// 边界命令未全部处理即宣称拍前事实。
    CommandsOutstanding {
        declared: u32,
        consumed: u32,
    },
    /// 义务未闭合：所属单位关闭时该规则未在该单位内运行（§3.3 漏遍历
    /// 不得通过）。
    RoundCoverageMissing(RuleId),
    /// 规则在当前阶段不得登记（阶段与规则规定阶段不符）。
    WrongPhase {
        rule: RuleId,
        phase: CheckPhase,
    },
    StepFromWrongPhase,
    StepOutcomeUnexpected,
    /// 步进结果拍号不接续（重复消费、跳跃或陈旧结果）。
    StaleStepTick {
        expected: u64,
        got: u64,
    },
    /// 规约未启用原子性/重试规约，不接受预期失败步进。
    AtomicityDisabled,
    UnknownRule(RuleId),
    UnknownWitness(WitnessId),
    /// 会话已终结（终结为消费式一次性操作）。
    Finished,
}

/// `finish` 拒绝原因，通过条件逐一对应（§3.3）；任何 `finish` 调用都会
/// 使会话进入终态。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FinishRejected {
    /// 窗口未执行完整；缩短窗口不得通过。
    WindowIncomplete { done: u64, required: u64 },
    /// 必需场景见证一次都未发生（按受信规约的必需集合核对）。
    WitnessMissing(WitnessId),
    /// 会话已因规则违反进入不可通过的失败终态。
    Violated(RuleId),
    /// 尚有等待中的步进结果未消费。
    PendingStepOutcome,
    /// 边界命令未全部处理。
    CommandsOutstanding { declared: u32, consumed: u32 },
    /// 已进入的拍前阶段义务未闭合，不得借终结跳过（§3.3）。
    RoundCoverageMissing(RuleId),
    /// 重复终结（或前次终结被拒后的再次尝试——终结是一次性的）。
    DoubleFinish,
}

/// 检查点创建拒绝：只允许在两种确定边界创建（§7）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CheckpointRefused {
    NoBoundary,
    /// 批次部分执行：既非命令执行前，也非全部执行后。
    PartialCommandBatch,
    AwaitingStepOutcome,
    Finished,
}

/// 恢复拒绝：材料身份错配、结构不认识或必要状态缺失均不得静默接受。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RestoreRejected {
    WrongSchema {
        found: u32,
        expected: u32,
    },
    /// 输入身份、规约名、域、语义版本或窗口与恢复预期不符。
    IdentityMismatch {
        field: &'static str,
    },
    /// 恢复时的实际安装模块不再满足规约（重新绑定的对账失败）。
    Plan(PlanRejected),
    /// 材料中的检查/见证名称在本次计划中不存在。
    UnknownCheck(String),
    /// 材料中出现规约必需集之外的见证。
    UnknownWitness(String),
    /// 必需见证在材料中缺项（缺项与“存在且计数为零”含义不同）。
    WitnessMissingInCheckpoint(String),
    /// 失败状态与违反次数矛盾。
    ViolationStateInconsistent,
    /// 开放义务组合不是任何可保存边界的可达状态。
    ObligationStateInconsistent,
    /// 执行集合与触发计数不一致（计数为零却带历史、或两侧规则集不齐）。
    HistoryCountMismatch(String),
    /// 历史条目的阶段与已绑定规则的规定阶段不符。
    HistoryPhaseMismatch(String),
    /// 已完成进度可推导的最低触发次数未被材料满足（缺必需执行历史）。
    MinTriggerUnmet(String),
}

/// 会话内部阶段。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stage {
    Bound,
    BeforeCommands,
    ProcessingCommands,
    AfterCommands,
    AwaitingStepOutcome,
    Finished,
}

/// 各义务单位的完成标记：按绑定期固定的规则索引分配，跨轮复用，
/// 无逐拍分配。核销即按单位重置对应标记。
#[derive(Clone, Debug)]
struct Obligations {
    /// 初态义务（一次性）：InitialBoundary 规则与 `initial` 规则。
    initial: Vec<bool>,
    /// 当前这条命令结果的义务。
    command: Vec<bool>,
    /// 本轮拍前义务（失败重试不重开）。
    pre_tick: Vec<bool>,
    /// 当前这次成功步进的义务（首轮独立于初态）。
    step: Vec<bool>,
    /// 本次失败尝试的义务。
    failure: Vec<bool>,
}

impl Obligations {
    fn new(rule_count: usize) -> Self {
        Self {
            initial: vec![false; rule_count],
            command: vec![false; rule_count],
            pre_tick: vec![false; rule_count],
            step: vec![false; rule_count],
            failure: vec![false; rule_count],
        }
    }

    /// 某单位的已标记规则名（检查点序列化用；索引经安装表映射为稳定名）。
    fn marked_names(
        &self,
        pick: fn(&Self) -> &Vec<bool>,
        rules: &[RuleDescriptor],
    ) -> BTreeSet<String> {
        pick(self)
            .iter()
            .enumerate()
            .filter(|&(_, &flag)| flag)
            .map(|(index, _)| rules[index].id.0.to_owned())
            .collect()
    }
}

/// 验证会话。计数与历史只增不减；所有集合用有序容器，保证给定输入
/// 序列时回执与检查点字节确定。
#[derive(Clone, Debug)]
pub struct Session {
    plan: CheckedPlan,
    stage: Stage,
    /// 已确立边界的拍号（成功步进按严格接续核对方为 last_tick+1）。
    last_tick: u64,
    successful_steps: u64,
    declared_commands: u32,
    consumed_commands: u32,
    /// 本边界是否已声明过批次（每边界恰一次）。
    batch_declared: bool,
    successful_commands: u64,
    blocked_attempts: u64,
    failed_attempts: u64,
    /// 触发覆盖：每条已安装规则的实际运行次数（累计，与当前义务分离）。
    trigger_counts: BTreeMap<RuleId, u64>,
    /// (阶段, 规则) 全会话执行集（回执报告用；义务核销见 `obligations`）。
    executed_checks: BTreeSet<(CheckPhase, RuleId)>,
    obligations: Obligations,
    /// 本轮拍前义务是否开放（advance 置位、begin_step 核销；失败重试
    /// 不重开，同一受检输入不重复形成拍前事实）。
    pre_tick_pending: bool,
    witnesses: BTreeMap<WitnessId, u64>,
    violated: Option<RuleId>,
    violations: u64,
}

impl Session {
    /// 绑定规约与安装模块，建立会话（未确立初始边界）。
    pub fn start(spec: VerificationSpec, provided: ProvidedModules) -> Result<Self, PlanRejected> {
        let plan = CheckedPlan::bind(spec, provided)?;
        let witnesses = plan
            .spec
            .required_witnesses
            .iter()
            .copied()
            .map(|id| (id, 0))
            .collect();
        let obligations = Obligations::new(plan.installed_rules.len());
        Ok(Self {
            plan,
            stage: Stage::Bound,
            last_tick: 0,
            successful_steps: 0,
            declared_commands: 0,
            consumed_commands: 0,
            batch_declared: false,
            successful_commands: 0,
            blocked_attempts: 0,
            failed_attempts: 0,
            trigger_counts: BTreeMap::new(),
            executed_checks: BTreeSet::new(),
            obligations,
            pre_tick_pending: false,
            witnesses,
            violated: None,
            violations: 0,
        })
    }

    fn installed(&self, rule: RuleId) -> Option<&'static RuleDescriptor> {
        self.plan.installed_rules.iter().find(|r| r.id == rule)
    }

    fn index_of(&self, rule: RuleId) -> Option<usize> {
        self.plan.installed_rules.iter().position(|r| r.id == rule)
    }

    /// 核销指定单位的每轮义务：单位内所有 EveryRound 规则的标记必须
    /// 已置位，随后按单位重置标记（义务实例随单位关闭而消费）。
    fn close_obligations(
        &mut self,
        unit: fn(&mut Obligations) -> &mut Vec<bool>,
        belongs: impl Fn(&RuleDescriptor) -> bool,
    ) -> Result<(), ProtocolError> {
        for required in self.plan.spec.required_rules {
            if required.coverage != CoverageMode::EveryRound || !belongs(required) {
                continue;
            }
            let Some(index) = self.index_of(required.id) else {
                continue;
            };
            if !unit(&mut self.obligations)[index] {
                return Err(ProtocolError::RoundCoverageMissing(required.id));
            }
        }
        for flag in unit(&mut self.obligations).iter_mut() {
            *flag = false;
        }
        Ok(())
    }

    /// 初态义务归属：InitialBoundary 规则，以及显式声明参与初态观测的
    /// 规则（`initial`）。仅声明拍后规则的合法规约没有初态义务。
    fn belongs_initial(rule: &RuleDescriptor) -> bool {
        rule.phase == CheckPhase::InitialBoundary || rule.initial
    }

    /// 确立初始边界：核验身份与时钟的入口由 harness 完成，会话只登记
    /// 基准拍号，供后续成功步进严格接续核对。
    pub fn confirm_initial_boundary(&mut self, tick: u64) -> Result<(), ProtocolError> {
        match self.stage {
            Stage::Bound => {
                self.last_tick = tick;
                self.stage = Stage::BeforeCommands;
                Ok(())
            }
            Stage::Finished => Err(ProtocolError::Finished),
            _ => Err(ProtocolError::InitialBoundaryTwice),
        }
    }

    /// 声明本边界的命令总数并进入处理阶段；每个边界恰一次，未完成的
    /// 批次不得被重新声明覆盖。新批次开启时命令义务标记复位。
    pub fn declare_commands(&mut self, count: u32) -> Result<(), ProtocolError> {
        match self.stage {
            Stage::BeforeCommands => {
                if self.batch_declared {
                    return Err(ProtocolError::CommandsAlreadyDeclared);
                }
                self.declared_commands = count;
                self.consumed_commands = 0;
                self.batch_declared = true;
                for flag in self.obligations.command.iter_mut() {
                    *flag = false;
                }
                self.stage = Stage::ProcessingCommands;
                Ok(())
            }
            Stage::ProcessingCommands => Err(ProtocolError::CommandsAlreadyDeclared),
            Stage::Finished => Err(ProtocolError::Finished),
            _ => Err(ProtocolError::CommandsBeforeInitialBoundary),
        }
    }

    /// 按实际顺序消费一条命令结果：先核销**这一条**命令的义务
    /// （CommandResults 每轮规则须已为该条结果运行），再接受结果并
    /// 为下一条命令重开义务。Blocked 与失败尝试同样携带逐条义务；
    /// 不在处理阶段或超量时拒绝。
    pub fn note_command_result(&mut self, result: CommandResult) -> Result<(), ProtocolError> {
        match self.stage {
            Stage::ProcessingCommands => {
                if self.consumed_commands >= self.declared_commands {
                    return Err(ProtocolError::CommandResultOverBatch {
                        declared: self.declared_commands,
                        consumed: self.consumed_commands,
                    });
                }
                self.close_obligations(
                    |o| &mut o.command,
                    |rule| rule.phase == CheckPhase::CommandResults,
                )?;
                match result {
                    CommandResult::Succeeded => self.successful_commands += 1,
                    CommandResult::Blocked => self.blocked_attempts += 1,
                    CommandResult::FailedAttempt => self.failed_attempts += 1,
                }
                self.consumed_commands += 1;
                Ok(())
            }
            Stage::Finished => Err(ProtocolError::Finished),
            _ => Err(ProtocolError::CommandResultOutsideBatch),
        }
    }

    /// 命令结束 / 拍前：声明过的命令须全部消费；首个边界的关闭核销
    /// 一次性初态义务（初态与首轮步进义务是两个实例，初态标记就此
    /// 消费，不流入首个成功步进）；随后开放本轮拍前义务。
    pub fn advance_to_pre_tick(&mut self) -> Result<(), ProtocolError> {
        match self.stage {
            Stage::ProcessingCommands => {
                if self.consumed_commands < self.declared_commands {
                    return Err(ProtocolError::CommandsOutstanding {
                        declared: self.declared_commands,
                        consumed: self.consumed_commands,
                    });
                }
                if self.successful_steps == 0 {
                    self.close_obligations(|o| &mut o.initial, Self::belongs_initial)?;
                }
                self.pre_tick_pending = true;
                self.stage = Stage::AfterCommands;
                Ok(())
            }
            Stage::Finished => Err(ProtocolError::Finished),
            _ => Err(ProtocolError::CommandsBeforeInitialBoundary),
        }
    }

    /// 记录一条已安装规则的实际运行（触发覆盖 + 对应单位的义务标记）。
    /// 登记按规则规定阶段门禁：初态观测轮接受 InitialBoundary 规则与
    /// 显式声明参与初态的规则；处理阶段接受命令结果规则；拍前阶段接受
    /// 拍前规则；等待步进结果时接受成功步进与（启用原子性时）失败检查
    /// 规则。未知规则与阶段不符均拒绝。
    pub fn note_check(&mut self, rule: RuleId) -> Result<(), ProtocolError> {
        if let Stage::Finished = self.stage {
            return Err(ProtocolError::Finished);
        }
        let Some(descriptor) = self.installed(rule) else {
            return Err(ProtocolError::UnknownRule(rule));
        };
        let index = self.index_of(rule).expect("installed 描述符必有索引");
        let initial_round = self.stage == Stage::BeforeCommands && self.successful_steps == 0;
        let marked = match descriptor.phase {
            CheckPhase::InitialBoundary => {
                if !initial_round {
                    return Err(ProtocolError::WrongPhase {
                        rule,
                        phase: descriptor.phase,
                    });
                }
                &mut self.obligations.initial
            }
            CheckPhase::CommandResults => {
                // 登记必须对应仍开放的义务实例：本条命令已有结果待消费。
                if self.stage != Stage::ProcessingCommands
                    || self.consumed_commands >= self.declared_commands
                {
                    return Err(ProtocolError::WrongPhase {
                        rule,
                        phase: descriptor.phase,
                    });
                }
                &mut self.obligations.command
            }
            CheckPhase::PreTick => {
                // 登记必须对应仍开放的义务实例：拍前义务尚未核销
                // （失败重试边界不重开，已消费实例不得再登记）。
                if self.stage != Stage::AfterCommands || !self.pre_tick_pending {
                    return Err(ProtocolError::WrongPhase {
                        rule,
                        phase: descriptor.phase,
                    });
                }
                &mut self.obligations.pre_tick
            }
            CheckPhase::PostSuccessfulStep => {
                if self.stage == Stage::AwaitingStepOutcome {
                    &mut self.obligations.step
                } else if initial_round && descriptor.initial {
                    // 显式声明参与初态观测的规则：登记初态义务实例。
                    &mut self.obligations.initial
                } else {
                    return Err(ProtocolError::WrongPhase {
                        rule,
                        phase: descriptor.phase,
                    });
                }
            }
            CheckPhase::PostExpectedFailure => {
                if self.stage != Stage::AwaitingStepOutcome || !self.plan.spec.atomicity {
                    return Err(ProtocolError::WrongPhase {
                        rule,
                        phase: descriptor.phase,
                    });
                }
                &mut self.obligations.failure
            }
        };
        marked[index] = true;
        *self.trigger_counts.entry(rule).or_insert(0) += 1;
        self.executed_checks.insert((descriptor.phase, rule));
        Ok(())
    }

    /// 规则发现违反：会话进入不可通过的失败终态；可继续记录历史与
    /// 见证，但 `finish` 永远拒绝（§6：checker 失败≠受检 Runtime 失败）。
    pub fn note_violation(&mut self, rule: RuleId) {
        if self.violated.is_none() {
            self.violated = Some(rule);
        }
        self.violations += 1;
    }

    /// 记录场景见证发生一次；未知见证拒绝。
    pub fn note_witness(&mut self, witness: WitnessId) -> Result<(), ProtocolError> {
        if let Stage::Finished = self.stage {
            return Err(ProtocolError::Finished);
        }
        match self.witnesses.get_mut(&witness) {
            Some(count) => {
                *count += 1;
                Ok(())
            }
            None => Err(ProtocolError::UnknownWitness(witness)),
        }
    }

    /// 发起一次步进：只能从“命令已结束”边界发起；本轮拍前义务尚未
    /// 核销时先核销（失败重试不重开拍前义务——同一受检输入不重复形成
    /// 拍前事实）；此后等待步进结果，`finish` 与检查点创建被拒。
    pub fn begin_step(&mut self) -> Result<(), ProtocolError> {
        match self.stage {
            Stage::AfterCommands => {
                if self.pre_tick_pending {
                    self.close_obligations(
                        |o| &mut o.pre_tick,
                        |rule| rule.phase == CheckPhase::PreTick,
                    )?;
                    self.pre_tick_pending = false;
                }
                self.stage = Stage::AwaitingStepOutcome;
                Ok(())
            }
            Stage::Finished => Err(ProtocolError::Finished),
            _ => Err(ProtocolError::StepFromWrongPhase),
        }
    }

    /// 消费一次成功步进结果：拍号须严格接续；先核销**这一次**步进的
    /// 义务（首轮的步进标记独立于已消费的初态标记），再推进成功步数
    /// 并开启新一轮（命令与拍前状态复位）。
    pub fn note_successful_step(&mut self, outcome_tick: u64) -> Result<(), ProtocolError> {
        match self.stage {
            Stage::AwaitingStepOutcome => {
                let expected = self.last_tick + 1;
                if outcome_tick != expected {
                    return Err(ProtocolError::StaleStepTick {
                        expected,
                        got: outcome_tick,
                    });
                }
                self.close_obligations(
                    |o| &mut o.step,
                    |rule| rule.phase == CheckPhase::PostSuccessfulStep,
                )?;
                // 尝试结束是完整关闭：本次尝试中误登记的失败检查标记
                // 随尝试失效，不得供后续失败尝试复用。
                for flag in self.obligations.failure.iter_mut() {
                    *flag = false;
                }
                self.successful_steps += 1;
                self.last_tick = outcome_tick;
                self.declared_commands = 0;
                self.consumed_commands = 0;
                self.batch_declared = false;
                self.stage = Stage::BeforeCommands;
                Ok(())
            }
            Stage::Finished => Err(ProtocolError::Finished),
            _ => Err(ProtocolError::StepOutcomeUnexpected),
        }
    }

    /// 消费一次预期失败步进结果：仅原子性/重试规约启用时合法；先核销
    /// **本次尝试**的失败检查义务（第一次尝试的证明不能代替第二次），
    /// 不推进成功步数，停留在同一“命令已结束”边界重试（§6）。
    pub fn note_expected_failure(&mut self) -> Result<(), ProtocolError> {
        match self.stage {
            Stage::AwaitingStepOutcome if self.plan.spec.atomicity => {
                self.close_obligations(
                    |o| &mut o.failure,
                    |rule| rule.phase == CheckPhase::PostExpectedFailure,
                )?;
                // 尝试结束是完整关闭：本次尝试中误登记的成功检查标记
                // 随尝试失效（该次步进未提交），不得供重试复用。
                for flag in self.obligations.step.iter_mut() {
                    *flag = false;
                }
                self.stage = Stage::AfterCommands;
                Ok(())
            }
            Stage::Finished => Err(ProtocolError::Finished),
            _ => Err(ProtocolError::AtomicityDisabled),
        }
    }

    /// 终结：消费式一次性操作——无论通过或拒绝，会话就此进入终态，
    /// 不得补记后再次终结（§3.3/§6）。已进入的拍前阶段义务必须闭合；
    /// 必需见证按受信规约的必需集合核对。
    pub fn finish(&mut self) -> Result<PassReceipt, FinishRejected> {
        let outcome = self.finish_inner();
        self.stage = Stage::Finished;
        outcome
    }

    fn finish_inner(&mut self) -> Result<PassReceipt, FinishRejected> {
        if let Stage::Finished = self.stage {
            return Err(FinishRejected::DoubleFinish);
        }
        if let Some(rule) = self.violated {
            return Err(FinishRejected::Violated(rule));
        }
        if let Stage::AwaitingStepOutcome = self.stage {
            return Err(FinishRejected::PendingStepOutcome);
        }
        if let Stage::ProcessingCommands = self.stage {
            return Err(FinishRejected::CommandsOutstanding {
                declared: self.declared_commands,
                consumed: self.consumed_commands,
            });
        }
        // 已进入拍前阶段的义务必须闭合：终结不是跳过已开启义务的出口。
        if self.stage == Stage::AfterCommands && self.pre_tick_pending {
            let closed = self.close_obligations(
                |o| &mut o.pre_tick,
                |rule| rule.phase == CheckPhase::PreTick,
            );
            self.pre_tick_pending = false;
            closed.map_err(|e| match e {
                ProtocolError::RoundCoverageMissing(rule) => {
                    FinishRejected::RoundCoverageMissing(rule)
                }
                other => unreachable!("拍前核销只会产生覆盖缺失：{other:?}"),
            })?;
        }
        if self.successful_steps != self.plan.spec.window.successful_steps {
            return Err(FinishRejected::WindowIncomplete {
                done: self.successful_steps,
                required: self.plan.spec.window.successful_steps,
            });
        }
        for witness in self.plan.spec.required_witnesses {
            if self.witnesses.get(witness).copied().unwrap_or(0) == 0 {
                return Err(FinishRejected::WitnessMissing(*witness));
            }
        }
        let receipt = PassReceipt {
            spec_name: self.plan.spec.name,
            domain: self.plan.spec.domain,
            semantic_version: self.plan.spec.semantic_version,
            successful_steps: self.successful_steps,
            last_tick: self.last_tick,
            executed_checks: self.executed_checks.clone(),
            trigger_counts: self.trigger_counts.clone(),
            witnesses: self.witnesses.clone(),
            blocked_attempts: self.blocked_attempts,
            failed_attempts: self.failed_attempts,
        };
        Ok(receipt)
    }

    /// 在确定边界创建检查点：命令执行前（新轮开启）或全部执行后；
    /// 保存当前开放义务与累计状态，保证连续执行与恢复执行等值。
    pub fn checkpoint(&self, input_identity: &str) -> Result<SessionCheckpoint, CheckpointRefused> {
        let command_phase = match self.stage {
            Stage::BeforeCommands => CommandPhase::BeforeCommands,
            Stage::AfterCommands => CommandPhase::AfterCommands,
            Stage::ProcessingCommands => return Err(CheckpointRefused::PartialCommandBatch),
            Stage::AwaitingStepOutcome => return Err(CheckpointRefused::AwaitingStepOutcome),
            Stage::Finished => return Err(CheckpointRefused::Finished),
            Stage::Bound => return Err(CheckpointRefused::NoBoundary),
        };
        let rules = self.plan.installed_rules;
        Ok(SessionCheckpoint {
            schema: CHECKPOINT_SCHEMA,
            spec_name: self.plan.spec.name.to_owned(),
            domain: self.plan.spec.domain.to_owned(),
            semantic_version: self.plan.spec.semantic_version.to_owned(),
            input_identity: input_identity.to_owned(),
            successful_steps: self.successful_steps,
            window_steps: self.plan.spec.window.successful_steps,
            last_tick: self.last_tick,
            command_phase,
            declared_commands: self.declared_commands,
            consumed_commands: self.consumed_commands,
            successful_commands: self.successful_commands,
            blocked_attempts: self.blocked_attempts,
            failed_attempts: self.failed_attempts,
            trigger_counts: self
                .trigger_counts
                .iter()
                .map(|(rule, count)| (rule.0.to_owned(), *count))
                .collect(),
            executed_checks: self
                .executed_checks
                .iter()
                .map(|(phase, rule)| (*phase, rule.0.to_owned()))
                .collect(),
            witnesses: self
                .witnesses
                .iter()
                .map(|(witness, count)| (witness.0.to_owned(), *count))
                .collect(),
            violated: match self.violated {
                None => ViolationRecord::Clean,
                Some(rule) => ViolationRecord::Violated {
                    rule: rule.0.to_owned(),
                },
            },
            violations: self.violations,
            pre_tick_pending: self.pre_tick_pending,
            open_initial: self.obligations.marked_names(|o| &o.initial, rules),
            open_command: self.obligations.marked_names(|o| &o.command, rules),
            open_pre_tick: self.obligations.marked_names(|o| &o.pre_tick, rules),
            open_step: self.obligations.marked_names(|o| &o.step, rules),
            open_failure: self.obligations.marked_names(|o| &o.failure, rules),
        })
    }

    /// 从检查点恢复：重新绑定规约与本次安装模块，逐项对账身份与结构，
    /// 逐名映射检查/见证与开放义务，并核对开放义务的可达性不变量——
    /// 步进/失败/命令标记在可保存的边界恒为空（它们只在等待步进结果
    /// 或处理命令期间非空，而该处不可建检查点）；初态标记仅存于首步
    /// 之前；拍前标记与待核销标志仅存于拍前阶段。任何缺失、未知、
    /// 错配或不可达组合均拒绝（§7 不清零、不转标）。恢复后的计数即
    /// 材料中的计数，成功步数不因恢复而增加。
    #[allow(clippy::too_many_lines)]
    pub fn restore(
        checkpoint: &SessionCheckpoint,
        spec: VerificationSpec,
        provided: ProvidedModules,
        expected_input_identity: &str,
    ) -> Result<Self, RestoreRejected> {
        if checkpoint.schema != CHECKPOINT_SCHEMA {
            return Err(RestoreRejected::WrongSchema {
                found: checkpoint.schema,
                expected: CHECKPOINT_SCHEMA,
            });
        }
        if checkpoint.spec_name != spec.name
            || checkpoint.domain != spec.domain
            || checkpoint.semantic_version != spec.semantic_version
        {
            return Err(RestoreRejected::IdentityMismatch { field: "spec" });
        }
        if checkpoint.input_identity != expected_input_identity {
            return Err(RestoreRejected::IdentityMismatch { field: "input" });
        }
        if checkpoint.window_steps != spec.window.successful_steps {
            return Err(RestoreRejected::IdentityMismatch { field: "window" });
        }
        let violated = match &checkpoint.violated {
            ViolationRecord::Clean => {
                if checkpoint.violations != 0 {
                    return Err(RestoreRejected::ViolationStateInconsistent);
                }
                None
            }
            ViolationRecord::Violated { rule } => {
                if checkpoint.violations == 0 {
                    return Err(RestoreRejected::ViolationStateInconsistent);
                }
                Some(
                    provided
                        .rules
                        .iter()
                        .map(|rule| rule.id)
                        .find(|id| id.0 == rule.as_str())
                        .ok_or_else(|| RestoreRejected::UnknownCheck(rule.clone()))?,
                )
            }
        };
        let plan = CheckedPlan::bind(spec, provided).map_err(RestoreRejected::Plan)?;
        let name_index = |name: &str| -> Result<usize, RestoreRejected> {
            plan.installed_rules
                .iter()
                .position(|rule| rule.id.0 == name)
                .ok_or_else(|| RestoreRejected::UnknownCheck(name.to_owned()))
        };
        let mut trigger_counts = BTreeMap::new();
        let mut executed_checks = BTreeSet::new();
        for (phase, name) in &checkpoint.executed_checks {
            let rule = plan.installed_rules[name_index(name)?].id;
            executed_checks.insert((*phase, rule));
        }
        for (name, count) in &checkpoint.trigger_counts {
            let rule = plan.installed_rules[name_index(name)?].id;
            trigger_counts.insert(rule, *count);
        }
        // 见证按受信规约的必需集合对账：缺项视为材料被改动，立即拒绝。
        let mut witnesses = BTreeMap::new();
        for required in spec.required_witnesses {
            let count = checkpoint
                .witnesses
                .get(required.0)
                .copied()
                .ok_or_else(|| {
                    RestoreRejected::WitnessMissingInCheckpoint(required.0.to_owned())
                })?;
            witnesses.insert(*required, count);
        }
        if witnesses.len() != checkpoint.witnesses.len() {
            let extra = checkpoint
                .witnesses
                .keys()
                .find(|name| !spec.required_witnesses.iter().any(|w| w.0 == name.as_str()))
                .expect("长度差必有多余项");
            return Err(RestoreRejected::UnknownWitness(extra.clone()));
        }
        // 历史完整性：执行集合与触发计数互相一致（计数只会随登记递增，
        // 零计数不得带历史；两侧规则集必须相同），历史条目的阶段与已
        // 绑规则一致，且必需 EveryRound 规则满足已完成进度可推导的
        // 最低触发次数（初态参与与失败尝试会额外增加累计次数，故用
        // 下界而非相等；OnEvent 合法零触发不设下界；预期失败尝试次数
        // 不在材料中，PostExpectedFailure 只做一致性核对）。
        let executed_names: BTreeSet<&str> = checkpoint
            .executed_checks
            .iter()
            .map(|(_, name)| name.as_str())
            .collect();
        let trigger_names: BTreeSet<&str> = checkpoint
            .trigger_counts
            .keys()
            .map(String::as_str)
            .collect();
        if executed_names != trigger_names {
            return Err(RestoreRejected::HistoryCountMismatch(
                "执行集合与触发计数的规则集不一致".into(),
            ));
        }
        for (count, name) in checkpoint
            .trigger_counts
            .values()
            .zip(executed_names.iter())
        {
            if *count == 0 {
                return Err(RestoreRejected::HistoryCountMismatch((*name).to_owned()));
            }
        }
        for (phase, name) in &checkpoint.executed_checks {
            let index = name_index(name)?;
            if plan.installed_rules[index].phase != *phase {
                return Err(RestoreRejected::HistoryPhaseMismatch(name.clone()));
            }
        }
        let initial_round_closed = checkpoint.successful_steps > 0
            || checkpoint.command_phase == CommandPhase::AfterCommands;
        let pre_tick_closed_current =
            checkpoint.command_phase == CommandPhase::AfterCommands && !checkpoint.pre_tick_pending;
        // 已接受预期失败的重试边界：只能经 note_expected_failure 到达。
        let at_retry_boundary = pre_tick_closed_current;
        for required in spec.required_rules {
            if required.coverage != CoverageMode::EveryRound {
                continue;
            }
            // 当前义务中已登记的完成标记与累计历史来自同一次登记——
            // 标记本身证明对应历史至少一条。
            let has_current_mark = match required.phase {
                CheckPhase::PreTick => checkpoint.open_pre_tick.contains(required.id.0),
                CheckPhase::InitialBoundary => checkpoint.open_initial.contains(required.id.0),
                CheckPhase::PostSuccessfulStep if required.initial => {
                    checkpoint.open_initial.contains(required.id.0)
                }
                _ => false,
            };
            let min = minimum_triggers(
                required,
                checkpoint.successful_steps,
                initial_round_closed,
                pre_tick_closed_current,
                at_retry_boundary,
                has_current_mark,
                checkpoint.successful_commands
                    + checkpoint.blocked_attempts
                    + checkpoint.failed_attempts,
            );
            match checkpoint.trigger_counts.get(required.id.0).copied() {
                Some(count) if count < min => {
                    return Err(RestoreRejected::MinTriggerUnmet(required.id.0.to_owned()));
                }
                None if min > 0 => {
                    return Err(RestoreRejected::MinTriggerUnmet(required.id.0.to_owned()));
                }
                _ => {}
            }
        }
        // 边界状态不变量：命令阶段与游标一致、拍前标志与阶段一致、
        // 开放标记属于对应义务单位且位置可达。
        match checkpoint.command_phase {
            CommandPhase::BeforeCommands => {
                if checkpoint.declared_commands != 0 || checkpoint.consumed_commands != 0 {
                    return Err(RestoreRejected::ObligationStateInconsistent);
                }
            }
            CommandPhase::AfterCommands => {
                if checkpoint.declared_commands != checkpoint.consumed_commands {
                    return Err(RestoreRejected::ObligationStateInconsistent);
                }
                let cumulative = checkpoint.successful_commands
                    + checkpoint.blocked_attempts
                    + checkpoint.failed_attempts;
                if u64::from(checkpoint.consumed_commands) > cumulative {
                    return Err(RestoreRejected::ObligationStateInconsistent);
                }
                // 拍前义务已核销的拍前阶段只能经预期失败重试到达，
                // 未启用原子性规约的会话不可达。
                if !checkpoint.pre_tick_pending && !spec.atomicity {
                    return Err(RestoreRejected::ObligationStateInconsistent);
                }
            }
        }
        if !checkpoint.open_initial.is_empty()
            && (checkpoint.successful_steps != 0
                || checkpoint.command_phase != CommandPhase::BeforeCommands)
        {
            return Err(RestoreRejected::ObligationStateInconsistent);
        }
        for name in &checkpoint.open_initial {
            let rule = &plan.installed_rules[name_index(name)?];
            if rule.phase != CheckPhase::InitialBoundary && !rule.initial {
                return Err(RestoreRejected::ObligationStateInconsistent);
            }
        }
        for name in &checkpoint.open_pre_tick {
            let rule = &plan.installed_rules[name_index(name)?];
            if rule.phase != CheckPhase::PreTick {
                return Err(RestoreRejected::ObligationStateInconsistent);
            }
        }
        // 开放义务可达性不变量。
        if !checkpoint.open_step.is_empty()
            || !checkpoint.open_failure.is_empty()
            || !checkpoint.open_command.is_empty()
        {
            return Err(RestoreRejected::ObligationStateInconsistent);
        }
        if !checkpoint.open_initial.is_empty() && checkpoint.successful_steps != 0 {
            return Err(RestoreRejected::ObligationStateInconsistent);
        }
        let after_commands = checkpoint.command_phase == CommandPhase::AfterCommands;
        if checkpoint.pre_tick_pending && !after_commands {
            return Err(RestoreRejected::ObligationStateInconsistent);
        }
        if !checkpoint.open_pre_tick.is_empty() && (!after_commands || !checkpoint.pre_tick_pending)
        {
            return Err(RestoreRejected::ObligationStateInconsistent);
        }
        let mut obligations = Obligations::new(plan.installed_rules.len());
        for name in &checkpoint.open_initial {
            obligations.initial[name_index(name)?] = true;
        }
        for name in &checkpoint.open_pre_tick {
            obligations.pre_tick[name_index(name)?] = true;
        }
        Ok(Self {
            plan,
            stage: match checkpoint.command_phase {
                CommandPhase::BeforeCommands => Stage::BeforeCommands,
                CommandPhase::AfterCommands => Stage::AfterCommands,
            },
            last_tick: checkpoint.last_tick,
            successful_steps: checkpoint.successful_steps,
            declared_commands: checkpoint.declared_commands,
            consumed_commands: checkpoint.consumed_commands,
            batch_declared: checkpoint.declared_commands > 0 || checkpoint.consumed_commands > 0,
            successful_commands: checkpoint.successful_commands,
            blocked_attempts: checkpoint.blocked_attempts,
            failed_attempts: checkpoint.failed_attempts,
            trigger_counts,
            executed_checks,
            obligations,
            pre_tick_pending: checkpoint.pre_tick_pending,
            witnesses,
            violated,
            violations: checkpoint.violations,
        })
    }

    /// 当前成功步数（诊断与测试用）。
    pub fn successful_steps(&self) -> u64 {
        self.successful_steps
    }

    /// 首个违反的规则（诊断用）。
    pub fn violated(&self) -> Option<RuleId> {
        self.violated
    }

    /// 某规则的实际运行次数（接线测试与诊断用；未运行为 0）。
    pub fn trigger_count(&self, rule: RuleId) -> u64 {
        self.trigger_counts.get(&rule).copied().unwrap_or(0)
    }
}

/// 正式通过回执：执行完整性、性质结果（无违反）、触发覆盖与必需见证
/// 分开表达（§3.3）。字段私有，只能由成功的 [`Session::finish`] 构造，
/// 调用方不能自行填写或改写通过结果（设计 §3.3 私有构造入口）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PassReceipt {
    spec_name: &'static str,
    domain: &'static str,
    semantic_version: &'static str,
    successful_steps: u64,
    last_tick: u64,
    executed_checks: BTreeSet<(CheckPhase, RuleId)>,
    trigger_counts: BTreeMap<RuleId, u64>,
    witnesses: BTreeMap<WitnessId, u64>,
    blocked_attempts: u64,
    failed_attempts: u64,
}

impl PassReceipt {
    pub fn spec_name(&self) -> &'static str {
        self.spec_name
    }
    pub fn domain(&self) -> &'static str {
        self.domain
    }
    pub fn semantic_version(&self) -> &'static str {
        self.semantic_version
    }
    pub fn successful_steps(&self) -> u64 {
        self.successful_steps
    }
    pub fn last_tick(&self) -> u64 {
        self.last_tick
    }
    /// (规定阶段, 规则) 的实际执行集（报告用）。
    pub fn executed_checks(&self) -> &BTreeSet<(CheckPhase, RuleId)> {
        &self.executed_checks
    }
    /// 触发覆盖：每条已运行规则的实际运行次数（零触发如实为 0）。
    pub fn trigger_counts(&self) -> &BTreeMap<RuleId, u64> {
        &self.trigger_counts
    }
    /// 必需见证的发生次数。
    pub fn witnesses(&self) -> &BTreeMap<WitnessId, u64> {
        &self.witnesses
    }
    pub fn blocked_attempts(&self) -> u64 {
        self.blocked_attempts
    }
    pub fn failed_attempts(&self) -> u64 {
        self.failed_attempts
    }
}

/// 失败状态的显式记录：缺失该字段时反序列化直接失败，`Clean` 与
/// `Violated` 是两种明确状态，不存在“缺失即未失败”的默认。
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ViolationRecord {
    Clean,
    Violated { rule: String },
}

/// 会话检查点的持久化材料（宿主恢复材料的组成部分，不进 Runtime LFRS）。
/// 名称以拥有字符串保存；恢复时逐名映射回计划并拒绝未知名称。开放义务
/// 与累计计数分开保存，保证连续执行与恢复执行等值。
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionCheckpoint {
    schema: u32,
    spec_name: String,
    domain: String,
    semantic_version: String,
    input_identity: String,
    successful_steps: u64,
    window_steps: u64,
    last_tick: u64,
    command_phase: CommandPhase,
    declared_commands: u32,
    consumed_commands: u32,
    successful_commands: u64,
    blocked_attempts: u64,
    failed_attempts: u64,
    trigger_counts: BTreeMap<String, u64>,
    executed_checks: BTreeSet<(CheckPhase, String)>,
    witnesses: BTreeMap<String, u64>,
    violated: ViolationRecord,
    violations: u64,
    /// 本轮拍前义务是否开放（失败重试不重开）。
    pre_tick_pending: bool,
    /// 各义务单位的当前开放标记（按稳定规则名编码）。
    open_initial: BTreeSet<String>,
    open_command: BTreeSet<String>,
    open_pre_tick: BTreeSet<String>,
    open_step: BTreeSet<String>,
    open_failure: BTreeSet<String>,
}

/// 由已完成进度推导某每轮义务规则的最低触发次数：初态轮（如已关闭）
/// 一次、每个已完成成功步进一次、当前已核销的拍前轮一次、每条已消费
/// 命令结果一次。预期失败尝试次数不在材料中，该阶段不推导下界。
fn minimum_triggers(
    rule: &RuleDescriptor,
    successful_steps: u64,
    initial_round_closed: bool,
    pre_tick_closed_current: bool,
    at_retry_boundary: bool,
    has_current_mark: bool,
    consumed_results: u64,
) -> u64 {
    match rule.phase {
        CheckPhase::PostSuccessfulStep => {
            successful_steps + u64::from(rule.initial && (initial_round_closed || has_current_mark))
        }
        CheckPhase::PreTick => {
            successful_steps + u64::from(pre_tick_closed_current || has_current_mark)
        }
        CheckPhase::CommandResults => consumed_results,
        CheckPhase::InitialBoundary => u64::from(initial_round_closed || has_current_mark),
        // 重试边界只能经已接受的预期失败到达，每次接受都核销过一次
        // 失败检查义务——该状态本身证明失败检查至少运行过一次。
        CheckPhase::PostExpectedFailure => u64::from(at_retry_boundary),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{CoverageMode, FactId, Window};

    const DOMAIN: &str = "w-frozen-six-phase";
    const RULE_MOTION: RuleDescriptor = RuleDescriptor {
        id: RuleId("w-motion-identity"),
        domains: &[DOMAIN],
        phase: CheckPhase::PostSuccessfulStep,
        coverage: CoverageMode::EveryRound,
        initial: true,
    };
    const RULE_BODY: RuleDescriptor = RuleDescriptor {
        id: RuleId("body-separation"),
        domains: &[DOMAIN],
        phase: CheckPhase::PostSuccessfulStep,
        coverage: CoverageMode::EveryRound,
        initial: true,
    };
    const RULE_SIGNAL: RuleDescriptor = RuleDescriptor {
        id: RuleId("signal-permit"),
        domains: &[DOMAIN],
        phase: CheckPhase::PostSuccessfulStep,
        coverage: CoverageMode::OnEvent,
        initial: false,
    };
    const RULE_PRE: RuleDescriptor = RuleDescriptor {
        id: RuleId("pre-tick-facts"),
        domains: &[DOMAIN],
        phase: CheckPhase::PreTick,
        coverage: CoverageMode::EveryRound,
        initial: false,
    };
    const RULE_CMD: RuleDescriptor = RuleDescriptor {
        id: RuleId("command-result-check"),
        domains: &[DOMAIN],
        phase: CheckPhase::CommandResults,
        coverage: CoverageMode::EveryRound,
        initial: false,
    };
    const RULE_ATOMICITY: RuleDescriptor = RuleDescriptor {
        id: RuleId("retry-atomicity"),
        domains: &[DOMAIN],
        phase: CheckPhase::PostExpectedFailure,
        coverage: CoverageMode::EveryRound,
        initial: false,
    };
    const RULES: &[RuleDescriptor] = &[RULE_MOTION, RULE_BODY, RULE_SIGNAL];
    const FACTS: &[FactId] = &[FactId("motion"), FactId("identity")];
    const WITNESSES: &[WitnessId] = &[WitnessId("per-active-crossing")];
    const PROVIDED: ProvidedModules = ProvidedModules {
        rules: RULES,
        facts: FACTS,
        witnesses: WITNESSES,
    };

    const fn spec(window: u64, atomicity: bool) -> VerificationSpec {
        VerificationSpec {
            name: "lf-synth-w1",
            domain: DOMAIN,
            semantic_version: "lf-synth-execution-2",
            required_rules: RULES,
            required_facts: FACTS,
            required_witnesses: WITNESSES,
            window: Window {
                successful_steps: window,
            },
            atomicity,
        }
    }

    fn note_round_rules(session: &mut Session) {
        session.note_check(RULE_MOTION.id).unwrap();
        session.note_check(RULE_BODY.id).unwrap();
    }

    /// 完整一轮：零命令 → 拍前 → 步进 → 每轮检查 → 成功。
    fn run_step(session: &mut Session, tick: u64) {
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        session.begin_step().unwrap();
        note_round_rules(session);
        session.note_successful_step(tick).unwrap();
    }

    /// 初始观测轮的登记（首个成功步进前；M/B 声明参与初态）。
    fn observe_initial(session: &mut Session) {
        note_round_rules(session);
    }

    fn started(window: u64) -> Session {
        let mut session = Session::start(spec(window, false), PROVIDED).unwrap();
        session.confirm_initial_boundary(100).unwrap();
        observe_initial(&mut session);
        session
    }

    #[test]
    fn happy_path_produces_private_receipt_and_finish_is_one_shot() {
        let mut session = started(3);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        for tick in 101..=103 {
            run_step(&mut session, tick);
        }
        let receipt = session.finish().expect("完整窗口+义务闭合+见证必须通过");
        assert_eq!(receipt.spec_name(), "lf-synth-w1");
        assert_eq!(receipt.successful_steps(), 3);
        assert_eq!(receipt.last_tick(), 103);
        // 初态轮 + 3 个成功步进轮各一次；OnEvent 零触发如实为 0。
        assert_eq!(receipt.trigger_counts()[&RULE_MOTION.id], 4);
        assert_eq!(receipt.trigger_counts()[&RULE_BODY.id], 4);
        assert_eq!(
            receipt
                .trigger_counts()
                .get(&RULE_SIGNAL.id)
                .copied()
                .unwrap_or(0),
            0
        );
        assert_eq!(receipt.witnesses()[&WitnessId("per-active-crossing")], 1);
        // 终结是一次性的：任何后续操作拒绝。
        assert_eq!(session.finish(), Err(FinishRejected::DoubleFinish));
        assert_eq!(
            session.note_check(RULE_BODY.id),
            Err(ProtocolError::Finished)
        );
    }

    #[test]
    fn initial_and_step_obligations_are_separate_instances() {
        // 探针一：仅声明拍后规则（不参与初态）的合法规约，第一步之前
        // 没有初态义务，可正常启动。
        const POST_ONLY: &[RuleDescriptor] = &[RuleDescriptor {
            id: RuleId("successful-outcome-only"),
            domains: &[DOMAIN],
            phase: CheckPhase::PostSuccessfulStep,
            coverage: CoverageMode::EveryRound,
            initial: false,
        }];
        let post_spec = VerificationSpec {
            required_rules: POST_ONLY,
            ..spec(1, false)
        };
        let post_provided = ProvidedModules {
            rules: POST_ONLY,
            ..PROVIDED
        };
        let mut session = Session::start(post_spec, post_provided).unwrap();
        session.confirm_initial_boundary(0).unwrap();
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        session.begin_step().unwrap();
        session
            .note_check(RuleId("successful-outcome-only"))
            .unwrap();
        session.note_successful_step(1).unwrap();
        assert_eq!(session.successful_steps(), 1);

        // 探针二：初态标记不得抵充首个成功步进的义务。
        let mut session = started(1);
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        session.begin_step().unwrap();
        // 首个步进：不登记任何每轮规则。
        assert_eq!(
            session.note_successful_step(101),
            Err(ProtocolError::RoundCoverageMissing(RULE_MOTION.id))
        );
        // 补登记后可推进（义务在关闭点核对）。
        note_round_rules(&mut session);
        session.note_successful_step(101).unwrap();

        // 探针三：初态义务缺失在首个拍前形成时拒绝（M 声明参与初态）。
        let mut skipped = Session::start(spec(1, false), PROVIDED).unwrap();
        skipped.confirm_initial_boundary(0).unwrap();
        skipped.note_check(RULE_BODY.id).unwrap();
        skipped.declare_commands(0).unwrap();
        assert_eq!(
            skipped.advance_to_pre_tick(),
            Err(ProtocolError::RoundCoverageMissing(RULE_MOTION.id))
        );

        // 探针四：窗口中途漏检在当轮关闭点暴露。
        let mut session = started(3);
        run_step(&mut session, 101);
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        session.begin_step().unwrap();
        session.note_check(RULE_MOTION.id).unwrap();
        assert_eq!(
            session.note_successful_step(102),
            Err(ProtocolError::RoundCoverageMissing(RULE_BODY.id))
        );
        session.note_check(RULE_BODY.id).unwrap();
        session.note_successful_step(102).unwrap();
        run_step(&mut session, 103);
    }

    #[test]
    fn out_of_phase_checks_are_rejected() {
        let mut session = started(1);
        // 未确立边界。
        let mut fresh = Session::start(spec(1, false), PROVIDED).unwrap();
        assert_eq!(
            fresh.note_check(RULE_BODY.id),
            Err(ProtocolError::WrongPhase {
                rule: RULE_BODY.id,
                phase: CheckPhase::PostSuccessfulStep,
            })
        );
        // 初态轮不接受不参与初态的规则。
        let mut initial = Session::start(spec(1, false), PROVIDED).unwrap();
        initial.confirm_initial_boundary(0).unwrap();
        assert_eq!(
            initial.note_check(RULE_SIGNAL.id),
            Err(ProtocolError::WrongPhase {
                rule: RULE_SIGNAL.id,
                phase: CheckPhase::PostSuccessfulStep,
            })
        );
        // 拍前阶段不接受成功步进后检查。
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        assert_eq!(
            session.note_check(RULE_BODY.id),
            Err(ProtocolError::WrongPhase {
                rule: RULE_BODY.id,
                phase: CheckPhase::PostSuccessfulStep,
            })
        );
        session.begin_step().unwrap();
        note_round_rules(&mut session);
        session.note_successful_step(101).unwrap();
        // 非初态的命令前阶段同样不接受。
        assert_eq!(
            session.note_check(RULE_BODY.id),
            Err(ProtocolError::WrongPhase {
                rule: RULE_BODY.id,
                phase: CheckPhase::PostSuccessfulStep,
            })
        );
    }

    #[test]
    fn command_results_are_checked_per_result() {
        const RULES_CMD: &[RuleDescriptor] = &[RULE_MOTION, RULE_BODY, RULE_CMD];
        let cmd_spec = VerificationSpec {
            required_rules: RULES_CMD,
            ..spec(1, false)
        };
        let cmd_provided = ProvidedModules {
            rules: RULES_CMD,
            ..PROVIDED
        };
        let mut session = Session::start(cmd_spec, cmd_provided).unwrap();
        session.confirm_initial_boundary(0).unwrap();
        observe_initial(&mut session);
        session.declare_commands(2).unwrap();
        // 第一条：检查运行，结果被接受并核销该条义务。
        session.note_check(RULE_CMD.id).unwrap();
        session
            .note_command_result(CommandResult::Succeeded)
            .unwrap();
        // 第二条：不检查——该条义务未闭合，拒绝接受结果。
        assert_eq!(
            session.note_command_result(CommandResult::Blocked),
            Err(ProtocolError::RoundCoverageMissing(RULE_CMD.id))
        );
        session.note_check(RULE_CMD.id).unwrap();
        session.note_command_result(CommandResult::Blocked).unwrap();
        session.advance_to_pre_tick().unwrap();
        session.begin_step().unwrap();
        note_round_rules(&mut session);
        session.note_successful_step(1).unwrap();
    }

    #[test]
    fn command_batch_is_closed_against_redeclare_overuse_and_phase_escape() {
        let mut session = started(1);
        session.declare_commands(2).unwrap();
        session
            .note_command_result(CommandResult::Succeeded)
            .unwrap();
        assert_eq!(
            session.declare_commands(0),
            Err(ProtocolError::CommandsAlreadyDeclared)
        );
        session.note_command_result(CommandResult::Blocked).unwrap();
        assert_eq!(
            session.note_command_result(CommandResult::Succeeded),
            Err(ProtocolError::CommandResultOverBatch {
                declared: 2,
                consumed: 2
            })
        );
        session.advance_to_pre_tick().unwrap();
        session.begin_step().unwrap();
        assert_eq!(
            session.note_command_result(CommandResult::Succeeded),
            Err(ProtocolError::CommandResultOutsideBatch)
        );
        note_round_rules(&mut session);
        session.note_successful_step(101).unwrap();
        // 部分执行批次的检查点：既非命令前也非命令后。
        session.declare_commands(2).unwrap();
        session
            .note_command_result(CommandResult::Succeeded)
            .unwrap();
        assert_eq!(
            session.checkpoint("input-a"),
            Err(CheckpointRefused::PartialCommandBatch)
        );
        session
            .note_command_result(CommandResult::Succeeded)
            .unwrap();
        session.advance_to_pre_tick().unwrap();
        assert!(session.checkpoint("input-a").is_ok());
    }

    #[test]
    fn blocked_and_failed_attempts_never_advance_success() {
        let mut session = started(1);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        session.declare_commands(3).unwrap();
        session.note_command_result(CommandResult::Blocked).unwrap();
        session
            .note_command_result(CommandResult::FailedAttempt)
            .unwrap();
        session
            .note_command_result(CommandResult::Succeeded)
            .unwrap();
        session.advance_to_pre_tick().unwrap();
        session.begin_step().unwrap();
        note_round_rules(&mut session);
        session.note_successful_step(101).unwrap();
        let receipt = session.finish().unwrap();
        assert_eq!(receipt.blocked_attempts(), 1);
        assert_eq!(receipt.failed_attempts(), 1);
        assert_eq!(receipt.successful_steps(), 1);
    }

    #[test]
    fn commands_must_be_fully_consumed_before_pre_tick_and_finish() {
        let mut session = started(1);
        session.declare_commands(2).unwrap();
        session
            .note_command_result(CommandResult::Succeeded)
            .unwrap();
        assert_eq!(
            session.advance_to_pre_tick(),
            Err(ProtocolError::CommandsOutstanding {
                declared: 2,
                consumed: 1
            })
        );
        assert_eq!(
            session.finish(),
            Err(FinishRejected::CommandsOutstanding {
                declared: 2,
                consumed: 1
            })
        );
        // 终结已被消费：会话进入终态，补命令也只能得到终态错误。
        assert_eq!(
            session.note_command_result(CommandResult::Blocked),
            Err(ProtocolError::Finished)
        );
    }

    #[test]
    fn violation_is_a_terminal_no_pass_state() {
        let mut session = started(1);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        session.note_violation(RULE_BODY.id);
        run_step(&mut session, 101);
        assert_eq!(session.violated(), Some(RULE_BODY.id));
        assert_eq!(
            session.finish(),
            Err(FinishRejected::Violated(RULE_BODY.id))
        );
    }

    #[test]
    fn witness_shortfall_freezes_the_session() {
        // 正式终结因缺见证被拒后，不得补记见证再次终结（§3.3/§6
        // 消费式一次性）。
        let mut session = started(1);
        run_step(&mut session, 101);
        assert_eq!(
            session.finish(),
            Err(FinishRejected::WitnessMissing(WitnessId(
                "per-active-crossing"
            )))
        );
        assert_eq!(
            session.note_witness(WitnessId("per-active-crossing")),
            Err(ProtocolError::Finished)
        );
        assert_eq!(session.finish(), Err(FinishRejected::DoubleFinish));
    }

    #[test]
    fn step_results_strictly_succeed_and_out_of_band_results_rejected() {
        let mut session = started(2);
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        session.begin_step().unwrap();
        assert_eq!(
            session.note_successful_step(100),
            Err(ProtocolError::StaleStepTick {
                expected: 101,
                got: 100
            })
        );
        assert_eq!(
            session.note_successful_step(102),
            Err(ProtocolError::StaleStepTick {
                expected: 101,
                got: 102
            })
        );
        session.note_check(RULE_MOTION.id).unwrap();
        session.note_check(RULE_BODY.id).unwrap();
        session.note_successful_step(101).unwrap();
        assert_eq!(
            session.note_successful_step(102),
            Err(ProtocolError::StepOutcomeUnexpected)
        );
        assert_eq!(session.begin_step(), Err(ProtocolError::StepFromWrongPhase));
    }

    #[test]
    fn failure_check_obligations_are_per_attempt() {
        const RULES_RETRY: &[RuleDescriptor] =
            &[RULE_MOTION, RULE_BODY, RULE_SIGNAL, RULE_ATOMICITY];
        let retry_spec = VerificationSpec {
            required_rules: RULES_RETRY,
            ..spec(1, true)
        };
        let retry_provided = ProvidedModules {
            rules: RULES_RETRY,
            ..PROVIDED
        };
        let mut session = Session::start(retry_spec, retry_provided).unwrap();
        session.confirm_initial_boundary(50).unwrap();
        observe_initial(&mut session);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        session.begin_step().unwrap();
        // 第一次尝试：失败检查运行后被接受。
        session.note_check(RULE_ATOMICITY.id).unwrap();
        session.note_expected_failure().unwrap();
        // 第二次尝试：不检查——本次尝试义务未闭合，拒绝接受。
        session.begin_step().unwrap();
        assert_eq!(
            session.note_expected_failure(),
            Err(ProtocolError::RoundCoverageMissing(RULE_ATOMICITY.id))
        );
        session.note_check(RULE_ATOMICITY.id).unwrap();
        session.note_expected_failure().unwrap();
        // 重试成功路径照常可终结。
        session.begin_step().unwrap();
        note_round_rules(&mut session);
        session.note_successful_step(51).unwrap();
        session
            .finish()
            .expect("失败检查逐次闭合后的重试路径必须可终结");
        // 未启用原子性规约：预期失败步进直接拒绝。
        let mut plain = started(1);
        plain.declare_commands(0).unwrap();
        plain.advance_to_pre_tick().unwrap();
        plain.begin_step().unwrap();
        assert_eq!(
            plain.note_expected_failure(),
            Err(ProtocolError::AtomicityDisabled)
        );
    }

    #[test]
    fn unknown_rule_or_witness_rejected_and_boundary_needed_first() {
        let mut session = Session::start(spec(1, false), PROVIDED).unwrap();
        assert_eq!(
            session.declare_commands(0),
            Err(ProtocolError::CommandsBeforeInitialBoundary)
        );
        session.confirm_initial_boundary(1).unwrap();
        assert_eq!(
            session.confirm_initial_boundary(2),
            Err(ProtocolError::InitialBoundaryTwice)
        );
        assert_eq!(
            session.note_check(RuleId("not-installed")),
            Err(ProtocolError::UnknownRule(RuleId("not-installed")))
        );
        assert_eq!(
            session.note_witness(WitnessId("not-required")),
            Err(ProtocolError::UnknownWitness(WitnessId("not-required")))
        );
        let fresh = Session::start(spec(1, false), PROVIDED).unwrap();
        assert_eq!(
            fresh.checkpoint("input-a"),
            Err(CheckpointRefused::NoBoundary)
        );
    }

    /// 带拍前规则的三步场景：连续执行与中途恢复执行最终回执等值。
    /// 恢复发生在某轮拍前阶段（拍前检查已登记、步进未发起），恢复后
    /// 不得重复登记，也不得丢义务。
    fn pre_tick_scenario(restore_at: Option<u64>, input: &str) -> PassReceipt {
        const RULES_PRE: &[RuleDescriptor] = &[RULE_MOTION, RULE_BODY, RULE_SIGNAL, RULE_PRE];
        let pre_spec = VerificationSpec {
            required_rules: RULES_PRE,
            ..spec(3, false)
        };
        let pre_provided = ProvidedModules {
            rules: RULES_PRE,
            ..PROVIDED
        };
        let mut session = Session::start(pre_spec, pre_provided).unwrap();
        session.confirm_initial_boundary(100).unwrap();
        observe_initial(&mut session);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        let mut resumed: Option<Session> = None;
        for tick in 101..=103 {
            {
                let active = resumed.as_mut().unwrap_or(&mut session);
                active.declare_commands(0).unwrap();
                active.advance_to_pre_tick().unwrap();
                active.note_check(RULE_PRE.id).unwrap();
            }
            if resumed.is_none() && restore_at == Some(tick) {
                let checkpoint = session.checkpoint(input).unwrap();
                resumed =
                    Some(Session::restore(&checkpoint, pre_spec, pre_provided, input).unwrap());
            }
            let active = resumed.as_mut().unwrap_or(&mut session);
            active.begin_step().unwrap();
            note_round_rules(active);
            active.note_successful_step(tick).unwrap();
        }
        let mut finished = resumed.unwrap_or(session);
        finished.finish().expect("两种执行路径都必须可终结")
    }

    #[test]
    fn checkpoint_round_trip_is_equivalent_to_continuous() {
        let continuous = pre_tick_scenario(None, "lfca-sha256:fixed-input");
        let resumed = pre_tick_scenario(Some(102), "lfca-sha256:fixed-input");
        // 回执逐字段等值：初态 + 每轮触发计数一致（恢复不重复累计、
        // 不丢义务）。
        assert_eq!(continuous, resumed);
        assert_eq!(continuous.trigger_counts()[&RULE_PRE.id], 3);
        assert_eq!(continuous.trigger_counts()[&RULE_MOTION.id], 4);
        // 序列化往返：缺字段、未知字段拒绝。
        let checkpoint = {
            let mut session = started(5);
            session
                .note_witness(WitnessId("per-active-crossing"))
                .unwrap();
            for tick in 101..=103 {
                run_step(&mut session, tick);
            }
            session.checkpoint("input-a").unwrap()
        };
        let bytes = serde_json::to_vec(&checkpoint).unwrap();
        let reloaded: SessionCheckpoint = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(reloaded, checkpoint);
        let as_value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let mut missing = as_value.clone();
        missing.as_object_mut().unwrap().remove("successful_steps");
        assert!(serde_json::from_value::<SessionCheckpoint>(missing).is_err());
        let mut unknown_field = as_value;
        unknown_field
            .as_object_mut()
            .unwrap()
            .insert("extra".into(), serde_json::Value::Null);
        assert!(serde_json::from_value::<SessionCheckpoint>(unknown_field).is_err());
        let mut resumed = Session::restore(&reloaded, spec(5, false), PROVIDED, "input-a").unwrap();
        assert_eq!(resumed.successful_steps(), 3);
        for tick in 104..=105 {
            run_step(&mut resumed, tick);
        }
        resumed.finish().expect("恢复续跑完整窗口必须可终结");
    }

    #[test]
    fn restore_rejects_wrong_identity_schema_and_unknown_names() {
        let input = "lfca-sha256:fixed-input";
        let mut session = started(2);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        run_step(&mut session, 101);
        let checkpoint = session.checkpoint(input).unwrap();
        assert_eq!(
            Session::restore(&checkpoint, spec(2, false), PROVIDED, "other-input").unwrap_err(),
            RestoreRejected::IdentityMismatch { field: "input" }
        );
        let other_name = VerificationSpec {
            name: "lf-synth-w4",
            ..spec(2, false)
        };
        assert_eq!(
            Session::restore(&checkpoint, other_name, PROVIDED, input).unwrap_err(),
            RestoreRejected::IdentityMismatch { field: "spec" }
        );
        let other_window = spec(9, false);
        assert_eq!(
            Session::restore(&checkpoint, other_window, PROVIDED, input).unwrap_err(),
            RestoreRejected::IdentityMismatch { field: "window" }
        );
        let mut tampered = checkpoint.clone();
        tampered.schema = 99;
        assert_eq!(
            Session::restore(&tampered, spec(2, false), PROVIDED, input).unwrap_err(),
            RestoreRejected::WrongSchema {
                found: 99,
                expected: CHECKPOINT_SCHEMA
            }
        );
        let mut unknown_check = checkpoint.clone();
        unknown_check
            .executed_checks
            .insert((CheckPhase::PreTick, "ghost-rule".to_owned()));
        assert_eq!(
            Session::restore(&unknown_check, spec(2, false), PROVIDED, input).unwrap_err(),
            RestoreRejected::UnknownCheck("ghost-rule".into())
        );
        let shrunk = ProvidedModules {
            rules: &[RULE_MOTION, RULE_BODY],
            ..PROVIDED
        };
        assert!(matches!(
            Session::restore(&checkpoint, spec(2, false), shrunk, input),
            Err(RestoreRejected::Plan(_))
        ));
    }

    #[test]
    fn restore_rejects_missing_or_inconsistent_failure_state() {
        let input = "lfca-sha256:fixed-input";
        let mut session = started(1);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        session.note_violation(RULE_SIGNAL.id);
        let checkpoint = session.checkpoint(input).unwrap();
        let bytes = serde_json::to_vec(&checkpoint).unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        value.as_object_mut().unwrap().remove("violated");
        assert!(serde_json::from_value::<SessionCheckpoint>(value.clone()).is_err());
        value
            .as_object_mut()
            .unwrap()
            .insert("violated".into(), serde_json::Value::Null);
        assert!(serde_json::from_value::<SessionCheckpoint>(value).is_err());
        let mut clean_session = started(1);
        clean_session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        run_step(&mut clean_session, 101);
        let mut inconsistent = clean_session.checkpoint(input).unwrap();
        inconsistent.violations = 1;
        assert_eq!(
            Session::restore(&inconsistent, spec(1, false), PROVIDED, input).unwrap_err(),
            RestoreRejected::ViolationStateInconsistent
        );
        let mut resumed = Session::restore(&checkpoint, spec(1, false), PROVIDED, input).unwrap();
        assert_eq!(resumed.violated(), Some(RULE_SIGNAL.id));
        assert_eq!(
            resumed.finish(),
            Err(FinishRejected::Violated(RULE_SIGNAL.id))
        );
    }

    #[test]
    fn restore_reconciles_required_witnesses_against_trusted_spec() {
        let input = "lfca-sha256:fixed-input";
        let mut session = started(1);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        run_step(&mut session, 101);
        let checkpoint = session.checkpoint(input).unwrap();
        let mut stripped = serde_json::to_value(&checkpoint).unwrap();
        stripped
            .as_object_mut()
            .unwrap()
            .get_mut("witnesses")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("per-active-crossing");
        let stripped: SessionCheckpoint = serde_json::from_value(stripped).unwrap();
        assert_eq!(
            Session::restore(&stripped, spec(1, false), PROVIDED, input).unwrap_err(),
            RestoreRejected::WitnessMissingInCheckpoint("per-active-crossing".into())
        );
        let mut extra = checkpoint.clone();
        extra.witnesses.insert("ghost-witness".to_owned(), 1);
        assert_eq!(
            Session::restore(&extra, spec(1, false), PROVIDED, input).unwrap_err(),
            RestoreRejected::UnknownWitness("ghost-witness".into())
        );
        let mut resumed = Session::restore(&checkpoint, spec(1, false), PROVIDED, input).unwrap();
        resumed.finish().expect("完整材料恢复后窗口已满且见证在档");
    }

    #[test]
    fn restore_rejects_unreachable_obligation_states() {
        let input = "lfca-sha256:fixed-input";
        let mut session = started(2);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        run_step(&mut session, 101);
        let checkpoint = session.checkpoint(input).unwrap();
        // 步进标记在可保存边界恒为空——材料中出现即不可达。
        let mut bogus_step = checkpoint.clone();
        bogus_step.open_step.insert(RULE_BODY.id.0.to_owned());
        assert_eq!(
            Session::restore(&bogus_step, spec(2, false), PROVIDED, input).unwrap_err(),
            RestoreRejected::ObligationStateInconsistent
        );
        // 命令与失败标记同理。
        let mut bogus_command = checkpoint.clone();
        bogus_command.open_command.insert(RULE_BODY.id.0.to_owned());
        assert_eq!(
            Session::restore(&bogus_command, spec(2, false), PROVIDED, input).unwrap_err(),
            RestoreRejected::ObligationStateInconsistent
        );
        // 拍前标记/待核销标志只可能出现在拍前（AfterCommands）阶段。
        let mut bogus_pre = checkpoint.clone();
        bogus_pre.open_pre_tick.insert(RULE_BODY.id.0.to_owned());
        assert_eq!(
            Session::restore(&bogus_pre, spec(2, false), PROVIDED, input).unwrap_err(),
            RestoreRejected::ObligationStateInconsistent
        );
        // 初态标记在首步之后不应存在。
        let mut bogus_initial = checkpoint.clone();
        bogus_initial
            .open_initial
            .insert(RULE_MOTION.id.0.to_owned());
        assert_eq!(
            Session::restore(&bogus_initial, spec(2, false), PROVIDED, input).unwrap_err(),
            RestoreRejected::ObligationStateInconsistent
        );
    }

    #[test]
    fn finish_closes_open_pre_tick_obligations() {
        const RULES_PRE: &[RuleDescriptor] = &[RULE_MOTION, RULE_BODY, RULE_SIGNAL, RULE_PRE];
        let pre_spec = VerificationSpec {
            required_rules: RULES_PRE,
            ..spec(1, false)
        };
        let pre_provided = ProvidedModules {
            rules: RULES_PRE,
            ..PROVIDED
        };
        // 完整窗口后进入最终边界拍前阶段：不履行拍前义务不得借终结跳过。
        let mut session = Session::start(pre_spec, pre_provided).unwrap();
        session.confirm_initial_boundary(100).unwrap();
        observe_initial(&mut session);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        session.note_check(RULE_PRE.id).unwrap();
        session.begin_step().unwrap();
        note_round_rules(&mut session);
        session.note_successful_step(101).unwrap();
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        assert_eq!(
            session.finish(),
            Err(FinishRejected::RoundCoverageMissing(RULE_PRE.id))
        );
        // 终结已消费：补记也只能得到终态错误。
        assert_eq!(session.finish(), Err(FinishRejected::DoubleFinish));
        // 对照：履行拍前义务后终结通过。
        let mut session = Session::start(
            pre_spec,
            ProvidedModules {
                rules: RULES_PRE,
                ..PROVIDED
            },
        )
        .unwrap();
        session.confirm_initial_boundary(100).unwrap();
        observe_initial(&mut session);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        session.note_check(RULE_PRE.id).unwrap();
        session.begin_step().unwrap();
        note_round_rules(&mut session);
        session.note_successful_step(101).unwrap();
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        session.note_check(RULE_PRE.id).unwrap();
        session.finish().expect("拍前义务闭合后的终结必须通过");
    }

    /// 拍前标记只属于仍开放的义务实例：失败重试边界不重开拍前义务，
    /// 已消费实例不得再登记，旧标记不得流入下一边界。
    #[test]
    fn pre_tick_marks_require_open_obligations() {
        const RULES_RETRY: &[RuleDescriptor] = &[
            RULE_MOTION,
            RULE_BODY,
            RULE_SIGNAL,
            RULE_PRE,
            RULE_ATOMICITY,
        ];
        let spec = VerificationSpec {
            required_rules: RULES_RETRY,
            ..spec(2, true)
        };
        let provided = ProvidedModules {
            rules: RULES_RETRY,
            ..PROVIDED
        };
        let mut session = Session::start(spec, provided).unwrap();
        session.confirm_initial_boundary(0).unwrap();
        observe_initial(&mut session);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        // 边界 k=0：拍前检查、失败检查、接受预期失败。
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        session.note_check(RULE_PRE.id).unwrap();
        session.begin_step().unwrap();
        session.note_check(RULE_ATOMICITY.id).unwrap();
        session.note_expected_failure().unwrap();
        // 重试边界（同一受检输入）：拍前义务已核销，再登记被拒。
        assert_eq!(
            session.note_check(RULE_PRE.id),
            Err(ProtocolError::WrongPhase {
                rule: RULE_PRE.id,
                phase: CheckPhase::PreTick,
            })
        );
        // 重试成功。
        session.begin_step().unwrap();
        note_round_rules(&mut session);
        session.note_successful_step(1).unwrap();
        // 边界 k=1：不执行本边界拍前检查——旧标记不存在，核销拒绝。
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        assert_eq!(
            session.begin_step(),
            Err(ProtocolError::RoundCoverageMissing(RULE_PRE.id))
        );
        // 补齐后继续并通过（含终局拍前闭合）。
        session.note_check(RULE_PRE.id).unwrap();
        session.begin_step().unwrap();
        note_round_rules(&mut session);
        session.note_successful_step(2).unwrap();
        session.finish().expect("各边界义务各自闭合后必须通过");
    }

    /// 命令标记只属于仍待消费的那条结果：批次耗尽后不得再登记，
    /// 自产检查点因此保持可恢复。
    #[test]
    fn command_marks_require_pending_result() {
        const RULES_CMD: &[RuleDescriptor] = &[RULE_MOTION, RULE_BODY, RULE_SIGNAL, RULE_CMD];
        let spec = VerificationSpec {
            required_rules: RULES_CMD,
            ..spec(1, false)
        };
        let provided = ProvidedModules {
            rules: RULES_CMD,
            ..PROVIDED
        };
        let mut session = Session::start(spec, provided).unwrap();
        session.confirm_initial_boundary(0).unwrap();
        observe_initial(&mut session);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        session.declare_commands(1).unwrap();
        session.note_check(RULE_CMD.id).unwrap();
        session
            .note_command_result(CommandResult::Succeeded)
            .unwrap();
        // 批次已耗尽：命令义务实例已关闭，再登记被拒。
        assert_eq!(
            session.note_check(RULE_CMD.id),
            Err(ProtocolError::WrongPhase {
                rule: RULE_CMD.id,
                phase: CheckPhase::CommandResults,
            })
        );
        session.advance_to_pre_tick().unwrap();
        session.begin_step().unwrap();
        note_round_rules(&mut session);
        session.note_successful_step(1).unwrap();
        session.finish().expect("逐条命令义务闭合后必须通过");
    }

    /// 一次尝试的结束是完整关闭：误登记到不相容尝试的标记随尝试失效，
    /// 不得供后续同类尝试复用（双向）。
    #[test]
    fn attempt_close_invalidates_cross_result_marks() {
        const RULES_RETRY: &[RuleDescriptor] =
            &[RULE_MOTION, RULE_BODY, RULE_SIGNAL, RULE_ATOMICITY];
        let spec = VerificationSpec {
            required_rules: RULES_RETRY,
            ..spec(2, true)
        };
        let provided = ProvidedModules {
            rules: RULES_RETRY,
            ..PROVIDED
        };
        // 方向一：失败尝试中误登记成功检查——接受失败后标记失效，
        // 重试成功必须重新执行本次成功检查。
        let mut session = Session::start(spec, provided).unwrap();
        session.confirm_initial_boundary(0).unwrap();
        observe_initial(&mut session);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        session.begin_step().unwrap();
        session.note_check(RULE_MOTION.id).unwrap();
        session.note_check(RULE_BODY.id).unwrap();
        session.note_check(RULE_ATOMICITY.id).unwrap();
        session.note_expected_failure().unwrap();
        session.begin_step().unwrap();
        // 重试不执行成功检查（误标记已随上次尝试失效）。
        assert_eq!(
            session.note_successful_step(1),
            Err(ProtocolError::RoundCoverageMissing(RULE_MOTION.id))
        );
        note_round_rules(&mut session);
        session.note_successful_step(1).unwrap();
        // 第二轮补满窗口。
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        session.begin_step().unwrap();
        note_round_rules(&mut session);
        session.note_successful_step(2).unwrap();
        session.finish().expect("双向关闭后的路径必须可终结");

        // 方向二：成功尝试中误登记失败检查——成功核销后标记失效，
        // 下一次失败尝试必须重新执行失败检查。
        let mut session = Session::start(spec, provided).unwrap();
        session.confirm_initial_boundary(0).unwrap();
        observe_initial(&mut session);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        session.begin_step().unwrap();
        note_round_rules(&mut session);
        session.note_check(RULE_ATOMICITY.id).unwrap();
        session.note_successful_step(1).unwrap();
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        session.begin_step().unwrap();
        // 不执行失败检查（上次尝试的误标记已失效）。
        assert_eq!(
            session.note_expected_failure(),
            Err(ProtocolError::RoundCoverageMissing(RULE_ATOMICITY.id))
        );
        session.note_check(RULE_ATOMICITY.id).unwrap();
        session.note_expected_failure().unwrap();
        session.begin_step().unwrap();
        note_round_rules(&mut session);
        session.note_successful_step(2).unwrap();
        session.finish().expect("双向关闭后的重试路径必须可终结");
    }

    /// `initial` 只对成功步进后阶段有意义；声明的组合不一致在绑定期
    /// 拒绝，不产出无法启动的计划。
    #[test]
    fn bind_rejects_inconsistent_initial_declarations() {
        use crate::spec::{CheckedPlan, PlanRejected};
        // PreTick + initial:true：阶段不兼容，规约侧与安装侧同样拒绝。
        const PRE_INITIAL: RuleDescriptor = RuleDescriptor {
            initial: true,
            ..RULE_PRE
        };
        const WITH_PRE_INITIAL: &[RuleDescriptor] = &[RULE_MOTION, RULE_BODY, PRE_INITIAL];
        const BASE: &[RuleDescriptor] = &[RULE_MOTION, RULE_BODY];
        assert_eq!(
            CheckedPlan::bind(
                VerificationSpec {
                    required_rules: WITH_PRE_INITIAL,
                    ..spec(1, false)
                },
                ProvidedModules {
                    rules: WITH_PRE_INITIAL,
                    ..PROVIDED
                },
            ),
            Err(PlanRejected::InitialPhaseUnsupported {
                rule: RULE_PRE.id,
                phase: CheckPhase::PreTick,
            })
        );
        // 规约要求参与初态、安装实现不支持：能力错配拒绝。
        const BODY_NO_INITIAL: RuleDescriptor = RuleDescriptor {
            initial: false,
            ..RULE_BODY
        };
        const NO_INITIAL: &[RuleDescriptor] = &[RULE_MOTION, BODY_NO_INITIAL];
        assert_eq!(
            CheckedPlan::bind(
                VerificationSpec {
                    required_rules: BASE,
                    ..spec(1, false)
                },
                ProvidedModules {
                    rules: NO_INITIAL,
                    ..PROVIDED
                },
            ),
            Err(PlanRejected::InitialCapabilityMismatch { rule: RULE_BODY.id })
        );
    }

    /// 自产检查点在每个受支持的边界都必须原样可恢复。
    #[test]
    fn self_produced_checkpoints_restore_at_supported_boundaries() {
        const RULES_RETRY: &[RuleDescriptor] = &[
            RULE_MOTION,
            RULE_BODY,
            RULE_SIGNAL,
            RULE_PRE,
            RULE_ATOMICITY,
        ];
        let spec = VerificationSpec {
            required_rules: RULES_RETRY,
            ..spec(2, true)
        };
        let provided = ProvidedModules {
            rules: RULES_RETRY,
            ..PROVIDED
        };
        let input = "lfca-sha256:fixed-input";
        let mut session = Session::start(spec, provided).unwrap();
        session.confirm_initial_boundary(0).unwrap();
        observe_initial(&mut session);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        // 边界一：新轮开启（BeforeCommands）。
        let before = session.checkpoint(input).unwrap();
        Session::restore(&before, spec, provided, input).expect("命令执行前的自产检查点必须可恢复");
        // 边界二：拍前阶段（义务开放、已登记）。
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        session.note_check(RULE_PRE.id).unwrap();
        let after_pending = session.checkpoint(input).unwrap();
        let mut resumed = Session::restore(&after_pending, spec, provided, input)
            .expect("拍前阶段（待核销）的自产检查点必须可恢复");
        resumed.begin_step().unwrap();
        note_round_rules(&mut resumed);
        resumed.note_successful_step(1).unwrap();
        // 边界三：失败重试后的拍前阶段（义务已核销、不重开）。
        resumed.declare_commands(0).unwrap();
        resumed.advance_to_pre_tick().unwrap();
        resumed.note_check(RULE_PRE.id).unwrap();
        resumed.begin_step().unwrap();
        resumed.note_check(RULE_ATOMICITY.id).unwrap();
        resumed.note_expected_failure().unwrap();
        let after_retry = resumed.checkpoint(input).unwrap();
        let mut final_run = Session::restore(&after_retry, spec, provided, input)
            .expect("失败重试边界的自产检查点必须可恢复");
        final_run.begin_step().unwrap();
        note_round_rules(&mut final_run);
        final_run.note_successful_step(2).unwrap();
        final_run.finish().expect("恢复续跑完整窗口必须可终结");
    }
    #[test]
    fn restore_rejects_missing_required_history() {
        let input = "lfca-sha256:fixed-input";
        // 一步窗口正常完成（初态 + 1 次成功步进）：M/B 各 2 次。
        let mut session = started(1);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        run_step(&mut session, 101);
        let checkpoint = session.checkpoint(input).unwrap();
        // 探针一：清空执行集合与触发计数——进度推得的最低次数不再满足。
        let mut emptied = checkpoint.clone();
        emptied.executed_checks.clear();
        emptied.trigger_counts.clear();
        assert_eq!(
            Session::restore(&emptied, spec(1, false), PROVIDED, input).unwrap_err(),
            RestoreRejected::MinTriggerUnmet(RULE_MOTION.id.0.to_owned())
        );
        // 探针二：保留条目、次数改零——计数为零却声称有历史本身矛盾。
        let mut zeroed = checkpoint.clone();
        zeroed.trigger_counts.insert(RULE_MOTION.id.0.to_owned(), 0);
        assert_eq!(
            Session::restore(&zeroed, spec(1, false), PROVIDED, input).unwrap_err(),
            RestoreRejected::HistoryCountMismatch(RULE_MOTION.id.0.to_owned())
        );
        // 探针三：历史条目阶段与规则规定阶段不符。
        let mut phase_swapped = checkpoint.clone();
        let name = RULE_MOTION.id.0.to_owned();
        phase_swapped
            .executed_checks
            .remove(&(CheckPhase::PostSuccessfulStep, name.clone()));
        phase_swapped
            .executed_checks
            .insert((CheckPhase::PreTick, name.clone()));
        assert_eq!(
            Session::restore(&phase_swapped, spec(1, false), PROVIDED, input).unwrap_err(),
            RestoreRejected::HistoryPhaseMismatch(name)
        );
        // 正向对照一：未开始步进的检查点可以没有拍后历史。
        let mut fresh = started(2);
        fresh
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        let initial_only = fresh.checkpoint(input).unwrap();
        assert!(Session::restore(&initial_only, spec(2, false), PROVIDED, input).is_ok());
        // 正向对照二：OnEvent 合法零触发允许无历史条目（本夹具初始即零触发）。
        assert!(!initial_only.trigger_counts.contains_key(RULE_SIGNAL.id.0));
    }

    #[test]
    fn restore_rejects_inconsistent_boundary_state() {
        let input = "lfca-sha256:fixed-input";
        let mut session = started(2);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        session.declare_commands(2).unwrap();
        session
            .note_command_result(CommandResult::Succeeded)
            .unwrap();
        session.note_command_result(CommandResult::Blocked).unwrap();
        session.advance_to_pre_tick().unwrap();
        session.begin_step().unwrap();
        note_round_rules(&mut session);
        session.note_successful_step(101).unwrap();
        let checkpoint = session.checkpoint(input).unwrap();
        // 探针一：AfterCommands 却有未消费完的命令游标（从新轮检查点
        // 显式构造该矛盾组合）。
        let mut under_consumed = checkpoint.clone();
        under_consumed.command_phase = CommandPhase::AfterCommands;
        under_consumed.declared_commands = 2;
        under_consumed.consumed_commands = 1;
        assert_eq!(
            Session::restore(&under_consumed, spec(2, false), PROVIDED, input).unwrap_err(),
            RestoreRejected::ObligationStateInconsistent
        );
        // 探针二：BeforeCommands 携带非零游标。
        let mut before_dirty = checkpoint.clone();
        before_dirty.command_phase = CommandPhase::BeforeCommands;
        before_dirty.declared_commands = 1;
        before_dirty.consumed_commands = 0;
        assert_eq!(
            Session::restore(&before_dirty, spec(2, false), PROVIDED, input).unwrap_err(),
            RestoreRejected::ObligationStateInconsistent
        );
        // 探针三：本批消费数超过累计命令尝试数。
        let mut over_cumulative = checkpoint.clone();
        over_cumulative.declared_commands += 3;
        over_cumulative.consumed_commands += 3;
        assert_eq!(
            Session::restore(&over_cumulative, spec(2, false), PROVIDED, input).unwrap_err(),
            RestoreRejected::ObligationStateInconsistent
        );
        // 探针四：非原子性会话不可能出现“拍前已核销”的拍前阶段
        // （BeforeCommands 上 pending=false 属正常新轮，故构造 After 形态）。
        let mut no_pending = checkpoint.clone();
        no_pending.command_phase = CommandPhase::AfterCommands;
        no_pending.pre_tick_pending = false;
        assert_eq!(
            Session::restore(&no_pending, spec(2, false), PROVIDED, input).unwrap_err(),
            RestoreRejected::ObligationStateInconsistent
        );
        // 探针五：AfterCommands（首边界拍前）携带已核销前的初态标记。
        let mut initial_at_after = {
            let mut s = started(2);
            s.note_witness(WitnessId("per-active-crossing")).unwrap();
            s.declare_commands(0).unwrap();
            s.advance_to_pre_tick().unwrap();
            s.checkpoint(input).unwrap()
        };
        // 正常路径此处 open_initial 已核销清空；伪造携带则不可达。
        initial_at_after
            .open_initial
            .insert(RULE_MOTION.id.0.to_owned());
        assert_eq!(
            Session::restore(&initial_at_after, spec(2, false), PROVIDED, input).unwrap_err(),
            RestoreRejected::ObligationStateInconsistent
        );
        // 探针六：开放标记归属错单位（拍前标记挂在非拍前规则上）。
        let mut wrong_unit = checkpoint.clone();
        wrong_unit.open_pre_tick.insert(RULE_MOTION.id.0.to_owned());
        assert_eq!(
            Session::restore(&wrong_unit, spec(2, false), PROVIDED, input).unwrap_err(),
            RestoreRejected::ObligationStateInconsistent
        );
        // 对照：原子性会话的“拍前已核销”拍前阶段（失败重试边界）可达。
        const RULES_RETRY: &[RuleDescriptor] = &[
            RULE_MOTION,
            RULE_BODY,
            RULE_SIGNAL,
            RULE_PRE,
            RULE_ATOMICITY,
        ];
        let retry_spec = VerificationSpec {
            required_rules: RULES_RETRY,
            ..spec(1, true)
        };
        let retry_provided = ProvidedModules {
            rules: RULES_RETRY,
            ..PROVIDED
        };
        let mut atomic = Session::start(retry_spec, retry_provided).unwrap();
        atomic.confirm_initial_boundary(0).unwrap();
        observe_initial(&mut atomic);
        atomic
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        atomic.declare_commands(0).unwrap();
        atomic.advance_to_pre_tick().unwrap();
        atomic.note_check(RULE_PRE.id).unwrap();
        atomic.begin_step().unwrap();
        atomic.note_check(RULE_ATOMICITY.id).unwrap();
        atomic.note_expected_failure().unwrap();
        let retry_boundary = atomic.checkpoint(input).unwrap();
        assert!(Session::restore(&retry_boundary, retry_spec, retry_provided, input).is_ok());
    }
    #[test]
    fn restore_rejects_history_missing_behind_current_marks() {
        let input = "lfca-sha256:fixed-input";
        // 探针一：当前拍前已登记 P、删除累计历史——标记本身证明历史
        // 至少一条，恢复拒绝；未损坏对照恢复后正常完成并保留 P=1。
        const RULES_PRE_ONLY: &[RuleDescriptor] = &[RULE_MOTION, RULE_BODY, RULE_SIGNAL, RULE_PRE];
        let pre_spec = VerificationSpec {
            required_rules: RULES_PRE_ONLY,
            ..spec(1, false)
        };
        let pre_provided = ProvidedModules {
            rules: RULES_PRE_ONLY,
            ..PROVIDED
        };
        let mut session = Session::start(pre_spec, pre_provided).unwrap();
        session.confirm_initial_boundary(0).unwrap();
        observe_initial(&mut session);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        session.note_check(RULE_PRE.id).unwrap();
        let checkpoint = session.checkpoint(input).unwrap();
        // 只删除 P 的累计历史（保留其他规则的已完成历史）：当前拍前
        // 标记 {P} 证明 P 至少登记过一次。
        let mut stripped = checkpoint.clone();
        stripped
            .executed_checks
            .retain(|(_, name)| name != RULE_PRE.id.0);
        stripped.trigger_counts.remove(RULE_PRE.id.0);
        assert_eq!(
            Session::restore(&stripped, pre_spec, pre_provided, input).unwrap_err(),
            RestoreRejected::MinTriggerUnmet(RULE_PRE.id.0.to_owned())
        );
        // 未损坏对照：恢复、完成、回执保留 P=1。
        let mut resumed = Session::restore(&checkpoint, pre_spec, pre_provided, input).unwrap();
        resumed.begin_step().unwrap();
        note_round_rules(&mut resumed);
        resumed.note_successful_step(1).unwrap();
        let receipt = resumed.finish().unwrap();
        assert_eq!(receipt.trigger_counts()[&RULE_PRE.id], 1);

        // 探针二：初态已登记 initial 规则、删除累计历史——下界由标记
        // 推出为 1，恢复拒绝。
        let mut fresh = started(1);
        fresh
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        let initial_marked = fresh.checkpoint(input).unwrap();
        assert!(initial_marked.open_initial.contains(RULE_MOTION.id.0));
        let mut stripped_initial = initial_marked.clone();
        stripped_initial.executed_checks.clear();
        stripped_initial.trigger_counts.clear();
        assert_eq!(
            Session::restore(&stripped_initial, spec(1, false), PROVIDED, input).unwrap_err(),
            RestoreRejected::MinTriggerUnmet(RULE_MOTION.id.0.to_owned())
        );

        // 探针三：已完成一步、第二轮拍前已登记（P 计 2），改 1——
        // 下界 = 已完成步进 1 + 当前标记 1 = 2，恢复拒绝。
        let mut session = Session::start(pre_spec, pre_provided).unwrap();
        session.confirm_initial_boundary(0).unwrap();
        observe_initial(&mut session);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        session.note_check(RULE_PRE.id).unwrap();
        session.begin_step().unwrap();
        note_round_rules(&mut session);
        session.note_successful_step(1).unwrap();
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        session.note_check(RULE_PRE.id).unwrap();
        let undercounted = session.checkpoint(input).unwrap();
        let mut short_by_one = undercounted.clone();
        short_by_one
            .trigger_counts
            .insert(RULE_PRE.id.0.to_owned(), 1);
        assert_eq!(
            Session::restore(&short_by_one, pre_spec, pre_provided, input).unwrap_err(),
            RestoreRejected::MinTriggerUnmet(RULE_PRE.id.0.to_owned())
        );
        // 未损坏对照（P=2）可恢复并完成。
        assert!(Session::restore(&undercounted, pre_spec, pre_provided, input).is_ok());
    }

    #[test]
    fn restore_rejects_missing_failure_history_at_retry_boundary() {
        let input = "lfca-sha256:fixed-input";
        const RULES_RETRY: &[RuleDescriptor] =
            &[RULE_MOTION, RULE_BODY, RULE_SIGNAL, RULE_ATOMICITY];
        let retry_spec = VerificationSpec {
            required_rules: RULES_RETRY,
            ..spec(1, true)
        };
        let retry_provided = ProvidedModules {
            rules: RULES_RETRY,
            ..PROVIDED
        };
        // 重试边界（已接受一次预期失败）：删除失败检查历史——重试
        // 边界本身证明失败检查至少运行过一次，恢复拒绝。
        let mut session = Session::start(retry_spec, retry_provided).unwrap();
        session.confirm_initial_boundary(0).unwrap();
        observe_initial(&mut session);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        session.begin_step().unwrap();
        session.note_check(RULE_ATOMICITY.id).unwrap();
        session.note_expected_failure().unwrap();
        let checkpoint = session.checkpoint(input).unwrap();
        let mut stripped = checkpoint.clone();
        stripped
            .executed_checks
            .retain(|(_, name)| name != RULE_ATOMICITY.id.0);
        stripped.trigger_counts.remove(RULE_ATOMICITY.id.0);
        assert_eq!(
            Session::restore(&stripped, retry_spec, retry_provided, input).unwrap_err(),
            RestoreRejected::MinTriggerUnmet(RULE_ATOMICITY.id.0.to_owned())
        );
        // 未损坏对照：恢复、重试成功、回执保留失败检查 1 次。
        let mut resumed = Session::restore(&checkpoint, retry_spec, retry_provided, input).unwrap();
        resumed.begin_step().unwrap();
        note_round_rules(&mut resumed);
        resumed.note_successful_step(1).unwrap();
        let receipt = resumed.finish().unwrap();
        assert_eq!(receipt.trigger_counts()[&RULE_ATOMICITY.id], 1);
        // 正向对照：未发生失败的合法材料，失败规则零触发仍可恢复。
        let mut clean = Session::start(retry_spec, retry_provided).unwrap();
        clean.confirm_initial_boundary(0).unwrap();
        observe_initial(&mut clean);
        clean.declare_commands(0).unwrap();
        clean.advance_to_pre_tick().unwrap();
        clean.begin_step().unwrap();
        note_round_rules(&mut clean);
        clean.note_successful_step(1).unwrap();
        let clean_checkpoint = clean.checkpoint(input).unwrap();
        assert!(
            !clean_checkpoint
                .trigger_counts
                .contains_key(RULE_ATOMICITY.id.0)
        );
        assert!(Session::restore(&clean_checkpoint, retry_spec, retry_provided, input).is_ok());
    }
}
