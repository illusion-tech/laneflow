//! 验证会话：阶段协议、检查覆盖、失败终态与保存恢复。
//!
//! 会话消费事实并产生结果，不执行交通命令、不持有可变世界（设计 §3.1）。
//! 固定阶段协议（§6）：绑定 → 初始边界 → 命令结果（成功、Blocked、失败
//! 尝试按实际顺序）→ 命令结束/拍前 → 步进结果恰消费一次 → 终结。成功
//! 步进按拍号严格接续计数；Blocked 与失败尝试只记尝试，不推进成功游标；
//! 预期失败步进仅在启用原子性规约时受检并停留在同一“命令已结束”边界。
//! 规则发现违反使会话进入不可通过的失败终态——可继续记录，但 `finish`
//! 永远拒绝（§6）。
//!
//! 通过由执行产生（§3.3）：完整窗口执行、每个必需规则按其规定阶段实际
//! 运行、必需场景见证满足、无违反且无未消费步进结果，四者同时成立才
//! 产生回执；仅 `required ⊆ installed` 不足以通过。执行完整性、性质
//! 结果、触发覆盖与必需见证在回执中分开表达。
//!
//! 检查点（§7）只在已确立边界且不等待步进结果时创建，材料绑定输入身份、
//! 规约/语义版本与全部必要状态；恢复时重新绑定并对账身份，缺失、未知、
//! 错配一律拒绝，不使用 `serde(default)` 清零，恢复不把边界当新成功步进
//! 重复累计。

use std::collections::{BTreeMap, BTreeSet};

use crate::spec::{
    CheckPhase, CheckedPlan, PlanRejected, ProvidedModules, RuleDescriptor, RuleId,
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
    /// 边界命令未全部处理即宣称拍前事实。
    CommandsOutstanding { declared: u32, consumed: u32 },
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

/// `finish` 拒绝原因，四类通过条件逐一对应（§3.3）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FinishRejected {
    /// 窗口未执行完整；缩短窗口不得通过。
    WindowIncomplete { done: u64, required: u64 },
    /// 必需规则安装后未按其规定阶段实际运行。
    CoverageMissing(RuleId),
    /// 必需场景见证一次都未发生。
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

/// 检查点创建拒绝：只允许在已确立边界、不处于等待步进结果的状态创建。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CheckpointRefused {
    /// 尚未确立任何边界。
    NoBoundary,
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
    /// 输入身份、规约名、域或语义版本与恢复预期不符。
    IdentityMismatch {
        field: &'static str,
    },
    /// 恢复时的实际安装模块不再满足规约（重新绑定的对账失败）。
    Plan(PlanRejected),
    /// 材料中的检查/见证名称在本次计划中不存在。
    UnknownCheck(String),
    UnknownWitness(String),
}

/// 会话内部阶段： Bound → AtBoundary → (AwaitingStepOutcome → AtBoundary)* → Finished。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stage {
    Bound,
    AtBoundary { tick: u64, phase: CommandPhase },
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
    successful_commands: u64,
    blocked_attempts: u64,
    failed_attempts: u64,
    /// 触发覆盖：每条已安装规则的实际运行次数；(阶段, 规则) 进入覆盖集。
    trigger_counts: BTreeMap<RuleId, u64>,
    executed_checks: BTreeSet<(CheckPhase, RuleId)>,
    /// 场景见证的发生次数；零触发不凭空失败，场景要求的见证未发生
    /// 不得通过。
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
            successful_commands: 0,
            blocked_attempts: 0,
            failed_attempts: 0,
            trigger_counts: BTreeMap::new(),
            executed_checks: BTreeSet::new(),
            witnesses,
            violated: None,
            violations: 0,
        })
    }

    fn installed(&self, rule: RuleId) -> Option<&'static RuleDescriptor> {
        self.plan.installed_rules.iter().find(|r| r.id == rule)
    }

    /// 确立初始边界：核验身份与时钟的入口由 harness 完成，会话只登记
    /// 基准拍号，供后续成功步进严格接续核对。
    pub fn confirm_initial_boundary(&mut self, tick: u64) -> Result<(), ProtocolError> {
        match self.stage {
            Stage::Bound => {
                self.last_tick = tick;
                self.stage = Stage::AtBoundary {
                    tick,
                    phase: CommandPhase::BeforeCommands,
                };
                Ok(())
            }
            Stage::Finished => Err(ProtocolError::Finished),
            _ => Err(ProtocolError::InitialBoundaryTwice),
        }
    }

    /// 声明本边界的命令总数；命令阶段只能从 BeforeCommands 进入。
    pub fn declare_commands(&mut self, count: u32) -> Result<(), ProtocolError> {
        match self.stage {
            Stage::AtBoundary {
                phase: CommandPhase::BeforeCommands,
                ..
            } => {
                self.declared_commands = count;
                self.consumed_commands = 0;
                Ok(())
            }
            Stage::Finished => Err(ProtocolError::Finished),
            _ => Err(ProtocolError::CommandsBeforeInitialBoundary),
        }
    }

    /// 按实际顺序消费一条命令结果。Blocked 与失败尝试增加统计，不推进
    /// 成功命令计数（§6：Blocked 不伪造推进；不得跳过失败尝试）。
    pub fn note_command_result(&mut self, result: CommandResult) -> Result<(), ProtocolError> {
        if let Stage::Finished = self.stage {
            return Err(ProtocolError::Finished);
        }
        match result {
            CommandResult::Succeeded => self.successful_commands += 1,
            CommandResult::Blocked => self.blocked_attempts += 1,
            CommandResult::FailedAttempt => self.failed_attempts += 1,
        }
        self.consumed_commands += 1;
        Ok(())
    }

    /// 命令结束 / 拍前：本边界声明过的命令须全部处理之后才形成拍前
    /// 事实；调用方此后以拍前列做位移与 D1 起点（§6：不得使用命令前
    /// 旧位置）。
    pub fn advance_to_pre_tick(&mut self) -> Result<(), ProtocolError> {
        match self.stage {
            Stage::AtBoundary {
                tick,
                phase: CommandPhase::BeforeCommands,
            } => {
                if self.consumed_commands < self.declared_commands {
                    return Err(ProtocolError::CommandsOutstanding {
                        declared: self.declared_commands,
                        consumed: self.consumed_commands,
                    });
                }
                self.stage = Stage::AtBoundary {
                    tick,
                    phase: CommandPhase::AfterCommands,
                };
                Ok(())
            }
            Stage::Finished => Err(ProtocolError::Finished),
            _ => Err(ProtocolError::CommandsBeforeInitialBoundary),
        }
    }

    /// 记录一条已安装规则的实际运行（触发覆盖）。未知规则拒绝——覆盖
    /// 核算只承认本次计划安装的规则。
    pub fn note_check(&mut self, rule: RuleId) -> Result<(), ProtocolError> {
        if let Stage::Finished = self.stage {
            return Err(ProtocolError::Finished);
        }
        let Some(descriptor) = self.installed(rule) else {
            return Err(ProtocolError::UnknownRule(rule));
        };
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

    /// 发起一次步进：只能从“命令已结束”边界发起；此后会话处于等待
    /// 步进结果状态，`finish` 与检查点创建均被拒绝，直至结果消费。
    pub fn begin_step(&mut self) -> Result<(), ProtocolError> {
        match self.stage {
            Stage::AtBoundary {
                phase: CommandPhase::AfterCommands,
                ..
            } => {
                self.stage = Stage::AwaitingStepOutcome;
                Ok(())
            }
            Stage::Finished => Err(ProtocolError::Finished),
            _ => Err(ProtocolError::StepFromWrongPhase),
        }
    }

    /// 消费一次成功步进结果：拍号须为 last_tick+1（严格接续，拒绝重复
    /// 消费与陈旧结果），成功步数 +1，新边界回到命令执行前。
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
                self.successful_steps += 1;
                self.last_tick = outcome_tick;
                self.stage = Stage::AtBoundary {
                    tick: outcome_tick,
                    phase: CommandPhase::BeforeCommands,
                };
                Ok(())
            }
            Stage::Finished => Err(ProtocolError::Finished),
            _ => Err(ProtocolError::StepOutcomeUnexpected),
        }
    }

    /// 消费一次预期失败步进结果：仅原子性/重试规约启用时合法；不推进
    /// 成功步数，停留在同一“命令已结束”边界以相同受检输入重试（§6）。
    pub fn note_expected_failure(&mut self) -> Result<(), ProtocolError> {
        match self.stage {
            Stage::AwaitingStepOutcome if self.plan.spec.atomicity => {
                self.stage = Stage::AtBoundary {
                    tick: self.last_tick,
                    phase: CommandPhase::AfterCommands,
                };
                Ok(())
            }
            Stage::Finished => Err(ProtocolError::Finished),
            _ => Err(ProtocolError::AtomicityDisabled),
        }
    }

    /// 终结：满足全部通过条件才产生回执并进入终态（§3.3 四条件）。
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
        if self.consumed_commands < self.declared_commands {
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
        for required in self.plan.spec.required_rules {
            if !self
                .executed_checks
                .contains(&(required.phase, required.id))
            {
                return Err(FinishRejected::CoverageMissing(required.id));
            }
        }
        for (witness, count) in &self.witnesses {
            if *count == 0 {
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

    /// 在已确立边界创建检查点；等待步进结果中或已终结时拒绝（§7）。
    pub fn checkpoint(&self, input_identity: &str) -> Result<SessionCheckpoint, CheckpointRefused> {
        let Stage::AtBoundary { tick: _, phase } = self.stage else {
            return Err(match self.stage {
                Stage::AwaitingStepOutcome => CheckpointRefused::AwaitingStepOutcome,
                Stage::Finished => CheckpointRefused::Finished,
                Stage::Bound | Stage::AtBoundary { .. } => CheckpointRefused::NoBoundary,
            });
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
            command_phase: phase,
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
            violated: self.violated.map(|rule| rule.0.to_owned()),
            violations: self.violations,
        })
    }

    /// 从检查点恢复：重新绑定规约与本次安装模块，逐项对账身份与结构，
    /// 逐名映射检查与见证；任何缺失、未知或错配均拒绝（§7 不清零、
    /// 不转标）。恢复后的计数即材料中的计数，成功步数不因恢复而增加。
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
        let mut witnesses = BTreeMap::new();
        for (name, count) in &checkpoint.witnesses {
            let witness = spec
                .required_witnesses
                .iter()
                .copied()
                .find(|id| id.0 == name.as_str())
                .ok_or_else(|| RestoreRejected::UnknownWitness(name.clone()))?;
            witnesses.insert(witness, *count);
        }
        let violated = match checkpoint.violated.as_deref() {
            None => None,
            Some(name) => Some(
                plan.installed_rules
                    .iter()
                    .map(|rule| rule.id)
                    .find(|id| id.0 == name)
                    .ok_or_else(|| RestoreRejected::UnknownCheck(name.to_owned()))?,
            ),
        };
        Ok(Self {
            plan,
            stage: Stage::AtBoundary {
                tick: checkpoint.last_tick,
                phase: checkpoint.command_phase,
            },
            last_tick: checkpoint.last_tick,
            successful_steps: checkpoint.successful_steps,
            declared_commands: checkpoint.declared_commands,
            consumed_commands: checkpoint.consumed_commands,
            successful_commands: checkpoint.successful_commands,
            blocked_attempts: checkpoint.blocked_attempts,
            failed_attempts: checkpoint.failed_attempts,
            trigger_counts,
            executed_checks,
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
}

/// 正式通过回执：执行完整性、性质结果（无违反）、触发覆盖与必需见证
/// 分开表达（§3.3）。零触发记录在 `trigger_counts` 中，不冒充覆盖。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PassReceipt {
    pub spec_name: &'static str,
    pub domain: &'static str,
    pub semantic_version: &'static str,
    pub successful_steps: u64,
    pub last_tick: u64,
    /// (规定阶段, 规则) 的实际执行集。
    pub executed_checks: BTreeSet<(CheckPhase, RuleId)>,
    /// 触发覆盖：每条已运行规则的实际运行次数。
    pub trigger_counts: BTreeMap<RuleId, u64>,
    /// 必需见证的发生次数。
    pub witnesses: BTreeMap<WitnessId, u64>,
    pub blocked_attempts: u64,
    pub failed_attempts: u64,
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
    violated: Option<String>,
    violations: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{FactId, Window};

    const DOMAIN: &str = "w-frozen-six-phase";
    const RULE_BODY: RuleDescriptor = RuleDescriptor {
        id: RuleId("body-separation"),
        domains: &[DOMAIN],
        phase: CheckPhase::PostSuccessfulStep,
    };
    const RULE_SIGNAL: RuleDescriptor = RuleDescriptor {
        id: RuleId("signal-permit"),
        domains: &[DOMAIN],
        phase: CheckPhase::PreTick,
    };
    const RULES: &[RuleDescriptor] = &[RULE_BODY, RULE_SIGNAL];
    const FACTS: &[FactId] = &[FactId("motion"), FactId("identity")];
    const WITNESSES: &[WitnessId] = &[WitnessId("per-active-crossing")];
    const PROVIDED: ProvidedModules = ProvidedModules {
        rules: RULES,
        facts: FACTS,
        witnesses: WITNESSES,
    };

    fn spec(window: u64, atomicity: bool) -> VerificationSpec {
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

    /// 跑一个完整边界循环：零命令 → 拍前 → 步进 → 成功。
    fn run_step(session: &mut Session, tick: u64) {
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        session.note_check(RULE_SIGNAL.id).unwrap();
        session.begin_step().unwrap();
        session.note_check(RULE_BODY.id).unwrap();
        session.note_successful_step(tick).unwrap();
    }

    fn full_session(window: u64) -> Session {
        let mut session = Session::start(spec(window, false), PROVIDED).unwrap();
        session.confirm_initial_boundary(100).unwrap();
        session
    }

    #[test]
    fn finish_requires_full_window_and_produces_receipt() {
        let mut session = full_session(3);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        for tick in 101..=103 {
            run_step(&mut session, tick);
        }
        let receipt = session.finish().expect("完整窗口+覆盖+见证必须通过");
        assert_eq!(receipt.spec_name, "lf-synth-w1");
        assert_eq!(receipt.successful_steps, 3);
        assert_eq!(receipt.last_tick, 103);
        assert_eq!(receipt.semantic_version, "lf-synth-execution-2");
        // 触发覆盖与执行集分开表达：signal 每边界 3 次、body 每步 3 次。
        assert_eq!(receipt.trigger_counts[&RuleId("signal-permit")], 3);
        assert_eq!(receipt.trigger_counts[&RuleId("body-separation")], 3);
        assert_eq!(receipt.witnesses[&WitnessId("per-active-crossing")], 1);
        // 终结后重复 finish 拒绝，任何阶段方法拒绝。
        assert_eq!(session.finish(), Err(FinishRejected::DoubleFinish));
        assert_eq!(
            session.note_check(RULE_BODY.id),
            Err(ProtocolError::Finished)
        );
    }

    #[test]
    fn shortened_window_and_pending_outcome_reject_finish() {
        let mut session = full_session(3);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        run_step(&mut session, 101);
        // 窗口未满。
        assert_eq!(
            session.finish(),
            Err(FinishRejected::WindowIncomplete {
                done: 1,
                required: 3
            })
        );
        // 等待步进结果中的 finish 与检查点创建都被拒。
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        session.begin_step().unwrap();
        assert_eq!(session.finish(), Err(FinishRejected::PendingStepOutcome));
        assert_eq!(
            session.checkpoint("input-a"),
            Err(CheckpointRefused::AwaitingStepOutcome)
        );
    }

    #[test]
    fn blocked_and_failed_attempts_never_advance_success() {
        let mut session = full_session(1);
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
        session.note_check(RULE_BODY.id).unwrap();
        session.note_check(RULE_SIGNAL.id).unwrap();
        session.note_successful_step(101).unwrap();
        let receipt = session.finish().unwrap();
        assert_eq!(receipt.blocked_attempts, 1);
        assert_eq!(receipt.failed_attempts, 1);
        assert_eq!(receipt.successful_steps, 1, "Blocked/失败尝试不计成功步进");
    }

    #[test]
    fn commands_must_be_fully_consumed_before_pre_tick_and_finish() {
        let mut session = full_session(1);
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
        // 命令未清也不得通过 finish。
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
        let mut session = full_session(1);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        session.note_violation(RuleId("body-separation"));
        // 会话可继续记录（历史与见证不丢），但完整窗口后 finish 仍拒绝。
        run_step(&mut session, 101);
        assert_eq!(session.violated(), Some(RuleId("body-separation")));
        assert_eq!(
            session.finish(),
            Err(FinishRejected::Violated(RuleId("body-separation")))
        );
    }

    #[test]
    fn installed_but_never_run_rule_rejects_finish() {
        // signal-permit 安装且必需，但本会话从未运行：覆盖缺口拒绝。
        let mut session = full_session(1);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        session.begin_step().unwrap();
        session.note_check(RULE_BODY.id).unwrap();
        session.note_successful_step(101).unwrap();
        assert_eq!(
            session.finish(),
            Err(FinishRejected::CoverageMissing(RuleId("signal-permit")))
        );
    }

    #[test]
    fn scenario_witness_required_once_but_zero_trigger_is_not_a_failure() {
        // 必需见证一次未发生：拒绝；发生一次即满足，不凭空要求更多。
        let mut session = full_session(1);
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
        session.finish().expect("见证补齐后必须通过");
    }

    #[test]
    fn step_results_strictly_succeed_and_out_of_band_results_rejected() {
        let mut session = full_session(2);
        session.declare_commands(0).unwrap();
        session.advance_to_pre_tick().unwrap();
        session.begin_step().unwrap();
        // 重复消费同一拍、跳拍都拒绝。
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
        // 没有等待中的结果却提交。
        assert_eq!(
            session.note_successful_step(102),
            Err(ProtocolError::StepOutcomeUnexpected)
        );
        // 步进未从“命令已结束”边界发起。
        assert_eq!(session.begin_step(), Err(ProtocolError::StepFromWrongPhase));
    }

    #[test]
    fn expected_failure_needs_atomicity_and_stays_at_same_boundary() {
        // 未启用原子性规约：预期失败步进拒绝。
        let mut plain = full_session(1);
        plain.declare_commands(0).unwrap();
        plain.advance_to_pre_tick().unwrap();
        plain.begin_step().unwrap();
        assert_eq!(
            plain.note_expected_failure(),
            Err(ProtocolError::AtomicityDisabled)
        );
        // 启用后：同一“命令已结束”边界重试，成功步数不推进。
        let mut atomic = Session::start(spec(1, true), PROVIDED).unwrap();
        atomic.confirm_initial_boundary(50).unwrap();
        atomic
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        atomic.declare_commands(0).unwrap();
        atomic.advance_to_pre_tick().unwrap();
        atomic.begin_step().unwrap();
        atomic.note_expected_failure().unwrap();
        assert_eq!(atomic.successful_steps(), 0);
        atomic.begin_step().unwrap();
        atomic.note_check(RULE_BODY.id).unwrap();
        atomic.note_check(RULE_SIGNAL.id).unwrap();
        atomic.note_successful_step(51).unwrap();
        atomic.finish().expect("重试后成功路径必须可终结");
    }

    #[test]
    fn unknown_rule_or_witness_rejected_and_boundary_needed_first() {
        let mut session = Session::start(spec(1, false), PROVIDED).unwrap();
        // 初始边界未确立：命令阶段拒绝；重复确立拒绝。
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
        // 无边界不得建检查点。
        let fresh = Session::start(spec(1, false), PROVIDED).unwrap();
        assert_eq!(
            fresh.checkpoint("input-a"),
            Err(CheckpointRefused::NoBoundary)
        );
    }

    #[test]
    fn checkpoint_round_trip_resumes_without_double_counting() {
        let input = "lfca-sha256:fixed-input";
        let mut session = full_session(5);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        for tick in 101..=103 {
            run_step(&mut session, tick);
        }
        let checkpoint = session.checkpoint(input).unwrap();
        // 序列化往返：字节确定（有序容器），缺字段拒绝，未知字段拒绝。
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
        // 恢复：计数即材料计数，不重复累计；续跑两步共五步可终结。
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
        let mut session = full_session(2);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        run_step(&mut session, 101);
        let checkpoint = session.checkpoint(input).unwrap();
        // 错输入身份 / 错规约 / 错窗口。
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
        // 错 schema 版本与未知检查名。
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
        // 恢复时模块缺规则：绑定对账失败传导。
        let shrunk = ProvidedModules {
            rules: &[RULE_BODY],
            ..PROVIDED
        };
        assert!(matches!(
            Session::restore(&checkpoint, spec(2, false), shrunk, input),
            Err(RestoreRejected::Plan(_))
        ));
    }

    #[test]
    fn violated_state_survives_checkpoint_round_trip() {
        let input = "lfca-sha256:fixed-input";
        let mut session = full_session(1);
        session
            .note_witness(WitnessId("per-active-crossing"))
            .unwrap();
        session.note_violation(RuleId("signal-permit"));
        let checkpoint = session.checkpoint(input).unwrap();
        let mut resumed = Session::restore(&checkpoint, spec(1, false), PROVIDED, input).unwrap();
        run_step(&mut resumed, 101);
        assert_eq!(
            resumed.finish(),
            Err(FinishRejected::Violated(RuleId("signal-permit"))),
            "违反状态不得因恢复清零"
        );
    }
}
