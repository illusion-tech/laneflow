//! 验证会话：阶段协议、每轮覆盖义务、失败终态与保存恢复。
//!
//! 会话消费事实并产生结果，不执行交通命令、不持有可变世界（设计 §3.1）。
//! 固定阶段协议（§6）：
//!
//! ```text
//! Bound → BeforeCommands → ProcessingCommands → AfterCommands
//!       → AwaitingStepOutcome → （成功）BeforeCommands(下一轮) … → Finished
//! ```
//!
//! 每个边界只能声明一次命令批次；结果只在处理阶段、且未超量时按实际顺序
//! 消费（成功、Blocked、失败尝试都算消费，后两者不推进成功游标）；消费
//! 数恰等于声明数才能形成拍前；步进只能从“命令已结束”发起，结果恰消费
//! 一次且拍号严格接续；预期失败步进仅在原子性规约启用时受检并停留在同
//! 一“命令已结束”边界。
//!
//! 覆盖义务按轮核对（§3.3“只检查初态、漏遍历均不能通过”）：`EveryRound`
//! 规则在其阶段关闭点（初始边界轮在首个拍前形成时、拍前轮在步进发起时、
//! 成功步进轮在结果核销时）必须出现在本轮记录中，漏一轮即拒绝推进；
//! `OnEvent` 规则只在事件实际发生时运行，零触发如实记零，场景必须发生
//! 的义务由必需见证承担。触发次数与阶段完成记录分开表达。
//!
//! 规则发现违反使会话进入不可通过的失败终态——可继续记录，但 `finish`
//! 永远拒绝（§6）。通过由执行产生：完整窗口、每轮义务闭合、必需见证
//! 满足、无违反且无未消费步进结果，四者同时成立才产生回执；回执字段
//! 私有，只能由成功的 `finish` 构造。
//!
//! 检查点（§7）只在“命令执行前”或“全部命令执行后”两种确定边界创建，
//! 部分执行的批次既非前者也非后者，拒绝；材料绑定输入身份、规约/语义
//! 版本与全部必要状态——失败状态以显式枚举保存，缺失即拒绝，不使用
//! `serde(default)` 清零；恢复按受信规约重新绑定并对账见证全集，缺项
//! 立即拒绝，恢复不把边界当新成功步进重复累计。

use std::collections::{BTreeMap, BTreeSet};

use crate::spec::{
    CheckPhase, CheckedPlan, CoverageMode, PlanRejected, ProvidedModules, RuleDescriptor, RuleId,
    VerificationSpec, WitnessId,
};

/// 检查点结构版本；升级按显式版本规则处理，旧材料不编辑字段转标。
pub const CHECKPOINT_SCHEMA: u32 = 1;

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
    /// 初始边界重复确立。
    InitialBoundaryTwice,
    /// 未确立初始边界即进入命令阶段。
    CommandsBeforeInitialBoundary,
    /// 同一边界重复声明命令批次（未完成义务不得被覆盖）。
    CommandsAlreadyDeclared,
    /// 不在命令处理阶段消费结果（含越阶段消费）。
    CommandResultOutsideBatch,
    /// 消费数超过声明数。
    CommandResultOverBatch { declared: u32, consumed: u32 },
    /// 边界命令未全部处理即宣称拍前事实。
    CommandsOutstanding { declared: u32, consumed: u32 },
    /// 每轮义务未闭合：所属阶段关闭时该规则本轮未运行（§3.3 漏遍历
    /// 不得通过）。
    RoundCoverageMissing(RuleId),
    /// 规则在当前阶段不得登记（阶段与规则规定阶段不符）。
    WrongPhase { rule: RuleId, phase: CheckPhase },
    /// 步进未从“命令已结束”边界发起。
    StepFromWrongPhase,
    /// 没有等待中的步进结果却提交结果。
    StepOutcomeUnexpected,
    /// 步进结果拍号不接续（重复消费、跳跃或陈旧结果）。
    StaleStepTick { expected: u64, got: u64 },
    /// 规约未启用原子性/重试规约，不接受预期失败步进。
    AtomicityDisabled,
    /// 规则不在本次计划安装集内，覆盖核算不承认。
    UnknownRule(RuleId),
    /// 见证不在规约必需集内。
    UnknownWitness(WitnessId),
    /// 会话已终结。
    Finished,
}

/// `finish` 拒绝原因，通过条件逐一对应（§3.3）。
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
    /// 重复终结。
    DoubleFinish,
}

/// 检查点创建拒绝：只允许在两种确定边界创建（§7）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CheckpointRefused {
    /// 尚未确立任何边界。
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
    /// 必需见证在材料中缺项（缺项与“存在且计数为零”含义不同，缺项
    /// 视为材料被改动，立即拒绝）。
    WitnessMissingInCheckpoint(String),
    /// 失败状态与违反次数矛盾（清零失败状态不得经恢复达成）。
    ViolationStateInconsistent,
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

/// 验证会话。计数与历史只增不减；所有集合用有序容器，保证给定输入
/// 序列时回执与检查点字节确定。
#[derive(Clone, Debug)]
pub struct Session {
    plan: CheckedPlan,
    stage: Stage,
    /// 已确立边界的拍号（成功步进按严格接续核对方为 last_tick+1）。
    last_tick: u64,
    successful_steps: u64,
    /// 本边界声明与已消费的命令数（成功+Blocked+失败尝试都算消费）。
    declared_commands: u32,
    consumed_commands: u32,
    /// 本边界是否已声明过批次（每边界恰一次）。
    batch_declared: bool,
    successful_commands: u64,
    blocked_attempts: u64,
    failed_attempts: u64,
    /// 触发覆盖：每条已安装规则的实际运行次数。
    trigger_counts: BTreeMap<RuleId, u64>,
    /// 阶段完成记录：(阶段, 规则) 全会话执行集（回执报告用；每轮义务
    /// 由 `round_noted` 在阶段关闭点核对，二者分开）。
    executed_checks: BTreeSet<(CheckPhase, RuleId)>,
    /// 本轮已运行的规则（阶段关闭时核对 EveryRound 义务后清空）。
    round_noted: BTreeSet<RuleId>,
    /// 场景见证的发生次数；按受信规约的必需集合初始化与核对。
    witnesses: BTreeMap<WitnessId, u64>,
    /// 首个违反的规则；一旦置位，`finish` 永远拒绝。
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
            round_noted: BTreeSet::new(),
            witnesses,
            violated: None,
            violations: 0,
        })
    }

    fn installed(&self, rule: RuleId) -> Option<&'static RuleDescriptor> {
        self.plan.installed_rules.iter().find(|r| r.id == rule)
    }

    /// 每轮义务核对：给定关闭点应收拢的阶段集合，`EveryRound` 规则必须
    /// 出现在本轮记录中。
    fn close_round_obligations(&self, closing: &[CheckPhase]) -> Result<(), ProtocolError> {
        for required in self.plan.spec.required_rules {
            if required.coverage != CoverageMode::EveryRound || !closing.contains(&required.phase) {
                continue;
            }
            if !self.round_noted.contains(&required.id) {
                return Err(ProtocolError::RoundCoverageMissing(required.id));
            }
        }
        Ok(())
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
    /// 批次不得被重新声明覆盖。
    pub fn declare_commands(&mut self, count: u32) -> Result<(), ProtocolError> {
        match self.stage {
            Stage::BeforeCommands => {
                if self.batch_declared {
                    return Err(ProtocolError::CommandsAlreadyDeclared);
                }
                self.declared_commands = count;
                self.consumed_commands = 0;
                self.batch_declared = true;
                self.stage = Stage::ProcessingCommands;
                Ok(())
            }
            Stage::ProcessingCommands => Err(ProtocolError::CommandsAlreadyDeclared),
            Stage::Finished => Err(ProtocolError::Finished),
            _ => Err(ProtocolError::CommandsBeforeInitialBoundary),
        }
    }

    /// 按实际顺序消费一条命令结果。只在处理阶段、且未超量时接受；
    /// Blocked 与失败尝试增加统计，不推进成功命令计数（§6：Blocked
    /// 不伪造推进；不得跳过失败尝试）。
    pub fn note_command_result(&mut self, result: CommandResult) -> Result<(), ProtocolError> {
        match self.stage {
            Stage::ProcessingCommands => {
                if self.consumed_commands >= self.declared_commands {
                    return Err(ProtocolError::CommandResultOverBatch {
                        declared: self.declared_commands,
                        consumed: self.consumed_commands,
                    });
                }
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

    /// 命令结束 / 拍前：声明过的命令须全部消费；首个边界的关闭同时核对
    /// 初始边界轮的每轮义务（初始观测与成功步进后检查共用首轮）。
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
                    self.close_round_obligations(&[
                        CheckPhase::InitialBoundary,
                        CheckPhase::PostSuccessfulStep,
                    ])?;
                }
                self.stage = Stage::AfterCommands;
                Ok(())
            }
            Stage::Finished => Err(ProtocolError::Finished),
            _ => Err(ProtocolError::CommandsBeforeInitialBoundary),
        }
    }

    /// 记录一条已安装规则的实际运行（触发覆盖）。规则须在与其规定阶段
    /// 匹配的会话阶段登记：初始边界轮允许初始边界检查与首轮成功步进后
    /// 检查（初始观测在第一个成功步进前发生）；未知规则拒绝。
    pub fn note_check(&mut self, rule: RuleId) -> Result<(), ProtocolError> {
        if let Stage::Finished = self.stage {
            return Err(ProtocolError::Finished);
        }
        let Some(descriptor) = self.installed(rule) else {
            return Err(ProtocolError::UnknownRule(rule));
        };
        let initial_round = self.stage == Stage::BeforeCommands && self.successful_steps == 0;
        let phase_ok = match descriptor.phase {
            CheckPhase::InitialBoundary => initial_round,
            CheckPhase::PostSuccessfulStep => {
                self.stage == Stage::AwaitingStepOutcome || initial_round
            }
            CheckPhase::PreTick => self.stage == Stage::AfterCommands,
            CheckPhase::CommandResults => self.stage == Stage::ProcessingCommands,
            CheckPhase::PostExpectedFailure => {
                self.stage == Stage::AwaitingStepOutcome && self.plan.spec.atomicity
            }
        };
        if !phase_ok {
            return Err(ProtocolError::WrongPhase {
                rule,
                phase: descriptor.phase,
            });
        }
        *self.trigger_counts.entry(rule).or_insert(0) += 1;
        self.round_noted.insert(rule);
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

    /// 发起一次步进：只能从“命令已结束”边界发起；发起前核对拍前轮的
    /// 每轮义务；此后会话处于等待步进结果状态，`finish` 与检查点创建
    /// 均被拒绝，直至结果消费。
    pub fn begin_step(&mut self) -> Result<(), ProtocolError> {
        match self.stage {
            Stage::AfterCommands => {
                self.close_round_obligations(&[CheckPhase::PreTick])?;
                self.stage = Stage::AwaitingStepOutcome;
                Ok(())
            }
            Stage::Finished => Err(ProtocolError::Finished),
            _ => Err(ProtocolError::StepFromWrongPhase),
        }
    }

    /// 消费一次成功步进结果：拍号须为 last_tick+1（严格接续，拒绝重复
    /// 消费与陈旧结果），先核对成功步进轮的每轮义务，再推进成功步数，
    /// 新边界回到命令执行前并开启新一轮。
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
                self.close_round_obligations(&[CheckPhase::PostSuccessfulStep])?;
                self.successful_steps += 1;
                self.last_tick = outcome_tick;
                self.round_noted.clear();
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

    /// 消费一次预期失败步进结果：仅原子性/重试规约启用时合法；不推进
    /// 成功步数也不核对成功步进义务（该次步进未提交），停留在同一
    /// “命令已结束”边界以相同受检输入重试（§6）。
    pub fn note_expected_failure(&mut self) -> Result<(), ProtocolError> {
        match self.stage {
            Stage::AwaitingStepOutcome if self.plan.spec.atomicity => {
                // 接受预期失败前，核对本次失败轮的失败检查义务
                // （PostExpectedFailure 每轮规则须已运行）。
                self.close_round_obligations(&[CheckPhase::PostExpectedFailure])?;
                self.stage = Stage::AfterCommands;
                Ok(())
            }
            Stage::Finished => Err(ProtocolError::Finished),
            _ => Err(ProtocolError::AtomicityDisabled),
        }
    }

    /// 终结：满足全部通过条件才产生回执并进入终态（§3.3）。
    pub fn finish(&mut self) -> Result<PassReceipt, FinishRejected> {
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
        if self.successful_steps != self.plan.spec.window.successful_steps {
            return Err(FinishRejected::WindowIncomplete {
                done: self.successful_steps,
                required: self.plan.spec.window.successful_steps,
            });
        }
        // 必需见证按受信规约的必需集合核对，不由会话内 map 的键决定
        // （恢复路径不因材料缺项悄悄免除义务）。
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
        self.stage = Stage::Finished;
        Ok(receipt)
    }

    /// 在确定边界创建检查点：命令执行前（新轮开启）或全部执行后；
    /// 批次部分执行与等待步进结果中拒绝（§7）。
    pub fn checkpoint(&self, input_identity: &str) -> Result<SessionCheckpoint, CheckpointRefused> {
        let command_phase = match self.stage {
            Stage::BeforeCommands => CommandPhase::BeforeCommands,
            Stage::AfterCommands => CommandPhase::AfterCommands,
            Stage::ProcessingCommands => return Err(CheckpointRefused::PartialCommandBatch),
            Stage::AwaitingStepOutcome => return Err(CheckpointRefused::AwaitingStepOutcome),
            Stage::Finished => return Err(CheckpointRefused::Finished),
            Stage::Bound => return Err(CheckpointRefused::NoBoundary),
        };
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
        })
    }

    /// 从检查点恢复：重新绑定规约与本次安装模块，逐项对账身份与结构，
    /// 逐名映射检查与见证并按受信规约核对见证全集；失败状态以显式
    /// 枚举对账且须与违反次数一致；任何缺失、未知或错配均拒绝（§7
    /// 不清零、不转标）。恢复后的计数即材料中的计数，成功步数不因
    /// 恢复而增加。
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
        let mut trigger_counts = BTreeMap::new();
        let mut executed_checks = BTreeSet::new();
        for (phase, name) in &checkpoint.executed_checks {
            let rule = plan
                .installed_rules
                .iter()
                .map(|rule| rule.id)
                .find(|id| id.0 == name.as_str())
                .ok_or_else(|| RestoreRejected::UnknownCheck(name.clone()))?;
            executed_checks.insert((*phase, rule));
        }
        for (name, count) in &checkpoint.trigger_counts {
            let rule = plan
                .installed_rules
                .iter()
                .map(|rule| rule.id)
                .find(|id| id.0 == name.as_str())
                .ok_or_else(|| RestoreRejected::UnknownCheck(name.clone()))?;
            trigger_counts.insert(rule, *count);
        }
        // 见证按受信规约的必需集合对账：缺项视为材料被改动，立即拒绝；
        // 多余名称同样拒绝。计数为零合法（义务仍未满足）。
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
            round_noted: BTreeSet::new(),
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
/// 名称以拥有字符串保存；恢复时逐名映射回计划并拒绝未知名称。
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
    };
    const RULE_BODY: RuleDescriptor = RuleDescriptor {
        id: RuleId("body-separation"),
        domains: &[DOMAIN],
        phase: CheckPhase::PostSuccessfulStep,
        coverage: CoverageMode::EveryRound,
    };
    const RULE_SIGNAL: RuleDescriptor = RuleDescriptor {
        id: RuleId("signal-permit"),
        domains: &[DOMAIN],
        phase: CheckPhase::PostSuccessfulStep,
        coverage: CoverageMode::OnEvent,
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

    /// 每轮义务规则的登记（成功步进轮）。
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

    /// 初始观测轮的登记（首个成功步进前，于 BeforeCommands 阶段完成）。
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
    fn finish_requires_full_window_and_produces_private_receipt() {
        let mut session = started(3);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        for tick in 101..=103 {
            run_step(&mut session, tick);
        }
        let receipt = session.finish().expect("完整窗口+每轮义务+见证必须通过");
        assert_eq!(receipt.spec_name(), "lf-synth-w1");
        assert_eq!(receipt.successful_steps(), 3);
        assert_eq!(receipt.last_tick(), 103);
        assert_eq!(receipt.semantic_version(), "lf-synth-execution-2");
        // 触发覆盖与执行集分开表达：OnEvent 零触发如实为 0。
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
        assert_eq!(session.finish(), Err(FinishRejected::DoubleFinish));
        assert_eq!(
            session.note_check(RULE_BODY.id),
            Err(ProtocolError::Finished)
        );
    }

    #[test]
    fn coverage_is_bound_to_rounds_not_to_membership() {
        // 审阅复现一：未确立初始边界就登记成功步进后检查——阶段不符拒绝。
        let mut session = Session::start(spec(1, false), PROVIDED).unwrap();
        assert_eq!(
            session.note_check(RULE_BODY.id),
            Err(ProtocolError::WrongPhase {
                rule: RULE_BODY.id,
                phase: CheckPhase::PostSuccessfulStep,
            })
        );
        // 审阅复现二：三拍窗口只在第一拍登记，第二拍漏检在轮关闭时暴露。
        let mut session = started(3);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        run_step(&mut session, 101);
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        session.begin_step().unwrap();
        // 第二拍：漏掉 body。
        session.note_check(RULE_MOTION.id).unwrap();
        assert_eq!(
            session.note_successful_step(102),
            Err(ProtocolError::RoundCoverageMissing(RULE_BODY.id))
        );
        // 补登记后仍可推进并通过（义务在关闭点核对，不在关闭点之外惩罚）。
        session.note_check(RULE_BODY.id).unwrap();
        session.note_successful_step(102).unwrap();
        run_step(&mut session, 103);
        session.finish().expect("补齐每轮义务后必须通过");
        // 初始观测轮漏检：首个拍前形成时拒绝。
        let mut skipped_initial = Session::start(spec(1, false), PROVIDED).unwrap();
        skipped_initial.confirm_initial_boundary(0).unwrap();
        skipped_initial.declare_commands(0).unwrap();
        assert_eq!(
            skipped_initial.advance_to_pre_tick(),
            Err(ProtocolError::RoundCoverageMissing(RULE_MOTION.id))
        );
    }

    #[test]
    fn out_of_phase_checks_are_rejected() {
        let mut session = started(1);
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        // 成功步进后检查不得在拍前阶段登记。
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
        // 新一轮的 BeforeCommands 上（非初始轮）也不得登记成功步进后检查。
        assert_eq!(
            session.note_check(RULE_BODY.id),
            Err(ProtocolError::WrongPhase {
                rule: RULE_BODY.id,
                phase: CheckPhase::PostSuccessfulStep,
            })
        );
    }

    #[test]
    fn command_batch_is_closed_against_redeclare_overuse_and_phase_escape() {
        let mut session = started(1);
        session.declare_commands(2).unwrap();
        session
            .note_command_result(CommandResult::Succeeded)
            .unwrap();
        // 未完成批次不得被重新声明覆盖。
        assert_eq!(
            session.declare_commands(0),
            Err(ProtocolError::CommandsAlreadyDeclared)
        );
        // 超额消费拒绝。
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
        // 越阶段消费（步进等待中报命令结果）拒绝。
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
        // 全部执行后可以建检查点（BeforeCommands 亦然，见往返测试）。
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
        session.note_command_result(CommandResult::Blocked).unwrap();
        session.advance_to_pre_tick().unwrap();
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
    fn scenario_witness_required_once_and_zero_trigger_is_not_a_failure() {
        // OnEvent 规则零触发合法（触发计数如实为 0），场景必须发生的
        // 义务由必需见证承担：见证未发生拒绝，发生即通过。
        let mut session = started(1);
        run_step(&mut session, 101);
        assert_eq!(
            session.finish(),
            Err(FinishRejected::WitnessMissing(WitnessId(
                "per-active-crossing"
            )))
        );
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        let receipt = session.finish().expect("见证补齐后必须通过");
        assert_eq!(
            receipt
                .trigger_counts()
                .get(&RULE_SIGNAL.id)
                .copied()
                .unwrap_or(0),
            0,
            "零触发如实记零"
        );
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
        session.note_successful_step(101).unwrap();
        assert_eq!(
            session.note_successful_step(102),
            Err(ProtocolError::StepOutcomeUnexpected)
        );
        assert_eq!(session.begin_step(), Err(ProtocolError::StepFromWrongPhase));
    }

    #[test]
    fn expected_failure_closes_failure_check_obligations() {
        // 原子性规约的失败检查（PostExpectedFailure 每轮规则）须在预期
        // 失败被接受前运行；漏跑在关闭点拒绝。
        const RULE_ATOMICITY: RuleDescriptor = RuleDescriptor {
            id: RuleId("retry-atomicity"),
            domains: &[DOMAIN],
            phase: CheckPhase::PostExpectedFailure,
            coverage: CoverageMode::EveryRound,
        };
        const RULES_RETRY: &[RuleDescriptor] = &[RULE_MOTION, RULE_BODY, RULE_ATOMICITY];
        let retry_spec = |window: u64, atomicity: bool| VerificationSpec {
            required_rules: RULES_RETRY,
            ..spec(window, atomicity)
        };
        let provided = ProvidedModules {
            rules: RULES_RETRY,
            ..PROVIDED
        };
        let mut session = Session::start(retry_spec(1, true), provided).unwrap();
        session.confirm_initial_boundary(50).unwrap();
        observe_initial(&mut session);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
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
            .expect("失败检查补齐后的重试路径必须可终结");
    }

    #[test]
    fn expected_failure_needs_atomicity_and_stays_at_same_boundary() {
        let mut plain = started(1);
        plain.declare_commands(0).unwrap();
        plain.advance_to_pre_tick().unwrap();
        plain.begin_step().unwrap();
        assert_eq!(
            plain.note_expected_failure(),
            Err(ProtocolError::AtomicityDisabled)
        );
        let mut atomic = Session::start(spec(1, true), PROVIDED).unwrap();
        atomic.confirm_initial_boundary(50).unwrap();
        observe_initial(&mut atomic);
        atomic
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        atomic.declare_commands(0).unwrap();
        atomic.advance_to_pre_tick().unwrap();
        atomic.begin_step().unwrap();
        atomic.note_expected_failure().unwrap();
        assert_eq!(atomic.successful_steps(), 0);
        atomic.begin_step().unwrap();
        note_round_rules(&mut atomic);
        atomic.note_successful_step(51).unwrap();
        atomic.finish().expect("重试后成功路径必须可终结");
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

    #[test]
    fn checkpoint_round_trip_resumes_without_double_counting() {
        let input = "lfca-sha256:fixed-input";
        let mut session = started(5);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        for tick in 101..=103 {
            run_step(&mut session, tick);
        }
        // 步进后处于新轮 BeforeCommands：属“命令执行前”确定边界。
        let checkpoint = session.checkpoint(input).unwrap();
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
        let mut resumed = Session::restore(&reloaded, spec(5, false), PROVIDED, input).unwrap();
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
        // 删除 violated 字段：反序列化直接失败（不存在“缺失即未失败”）。
        value.as_object_mut().unwrap().remove("violated");
        assert!(serde_json::from_value::<SessionCheckpoint>(value.clone()).is_err());
        // 显式 null 同样失败。
        value
            .as_object_mut()
            .unwrap()
            .insert("violated".into(), serde_json::Value::Null);
        assert!(serde_json::from_value::<SessionCheckpoint>(value).is_err());
        // Clean 配非零违反次数：矛盾拒绝（以未违反的干净检查点为底）。
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
        // 正常失败检查点往返：违反保持。
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
        // 材料中删除必需见证项：立即拒绝（缺项≠计数为零）。
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
        // 材料中多出未知见证：拒绝。
        let mut extra = checkpoint.clone();
        extra.witnesses.insert("ghost-witness".to_owned(), 1);
        assert_eq!(
            Session::restore(&extra, spec(1, false), PROVIDED, input).unwrap_err(),
            RestoreRejected::UnknownWitness("ghost-witness".into())
        );
        // 完整材料恢复后原会话通过；材料计数为零时义务仍未满足。
        let mut resumed = Session::restore(&checkpoint, spec(1, false), PROVIDED, input).unwrap();
        resumed.finish().expect("完整材料恢复后窗口已满且见证在档");
    }
}
