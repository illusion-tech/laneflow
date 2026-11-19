//! Synth 的正式验证规约构造入口（设计 §3.1：私有模块，按受检输入身份
//! 与执行目的选择，共享包不建全局枚举）。
//!
//! W1/W2 普通 run 的规约：五条必需规则（运动身份、车身分离、W 走廊
//! 顺序、信号许可、跨门事件配对）、四类必需事实、每辆 Active 恰一次
//! 过门的场景见证；窗口为受检计划的完整预热+观察长度，不允许短窗口。
//! L1 换道样本不在本规约内（设计 §10：L1 明确独立组合身份，首批不
//! 重写）；W3/W4 burst 的命令/重试/恢复接线由后续切片接入。

use laneflow_traffic_verification::session::Session;
use laneflow_traffic_verification::spec::{
    CheckPhase, CoverageMode, FactId, ProvidedModules, RuleDescriptor, RuleId, VerificationSpec,
    Window, WitnessId,
};

use crate::SEMANTIC_CONTRACT;

pub(crate) const DOMAIN: &str = "w-frozen-six-phase";

/// 逐拍运动与身份不变量（进度/速度/加减速/身份/绑定）。初态观测与
/// 每个成功步进是两个独立义务实例，本规则显式声明两处都运行。
pub(crate) const RULE_MOTION: RuleDescriptor = RuleDescriptor {
    id: RuleId("w-motion-identity"),
    domains: &[DOMAIN],
    phase: CheckPhase::PostSuccessfulStep,
    coverage: CoverageMode::EveryRound,
    initial: true,
};

/// 含微米余数的提交车身区间严格分离（共享判据，有序相邻路径）。
pub(crate) const RULE_BODY: RuleDescriptor = RuleDescriptor {
    id: RuleId("body-separation"),
    domains: &[DOMAIN],
    phase: CheckPhase::PostSuccessfulStep,
    coverage: CoverageMode::EveryRound,
    initial: true,
};

/// W 走廊顺序不反转/不超车（专项，独立于共享分离判据核验）。
pub(crate) const RULE_ORDER: RuleDescriptor = RuleDescriptor {
    id: RuleId("w-lane-order"),
    domains: &[DOMAIN],
    phase: CheckPhase::PostSuccessfulStep,
    coverage: CoverageMode::EveryRound,
    initial: true,
};

/// 拍初独立信号许可（D1 严格不等式，共享判据）。
pub(crate) const RULE_SIGNAL: RuleDescriptor = RuleDescriptor {
    id: RuleId("signal-permit"),
    domains: &[DOMAIN],
    phase: CheckPhase::PostSuccessfulStep,
    coverage: CoverageMode::OnEvent,
    initial: false,
};

/// entry→exit 与 live-order 中恰好一对 GateCrossed/ManeuverTraversal 事件。
pub(crate) const RULE_GATES: RuleDescriptor = RuleDescriptor {
    id: RuleId("gate-events"),
    domains: &[DOMAIN],
    phase: CheckPhase::PostSuccessfulStep,
    coverage: CoverageMode::OnEvent,
    initial: false,
};

const RULES: &[RuleDescriptor] = &[RULE_MOTION, RULE_BODY, RULE_ORDER, RULE_SIGNAL, RULE_GATES];
const FACTS: &[FactId] = &[
    FactId("motion"),
    FactId("identity"),
    FactId("signal-plan"),
    FactId("gate-events"),
];
const WITNESSES: &[WitnessId] = &[WitnessId("per-active-crossing")];

const PROVIDED: ProvidedModules = ProvidedModules {
    rules: RULES,
    facts: FACTS,
    witnesses: WITNESSES,
};

/// 场景见证：每辆 Active 完整窗口恰过一次门。
pub(crate) const WITNESS_CROSSING: WitnessId = WitnessId("per-active-crossing");

/// W1/W2 正式规约：`total` 为受检计划的完整窗口长度（预热+观察）。
pub(crate) fn w_spec(total: u64) -> VerificationSpec {
    VerificationSpec {
        name: "lf-synth-w-semantic-run",
        domain: DOMAIN,
        semantic_version: SEMANTIC_CONTRACT,
        required_rules: RULES,
        required_facts: FACTS,
        required_witnesses: WITNESSES,
        window: Window {
            successful_steps: total,
        },
        atomicity: false,
    }
}

/// 由受检窗口构造 W1/W2 会话（绑定失败即错误，不产出降级计划）。
pub(crate) fn start_session(total: u64) -> Result<Session, crate::Error> {
    Session::start(w_spec(total), PROVIDED)
        .map_err(|rejected| crate::Error::Invalid(format!("验证规约绑定失败：{rejected}")))
}

/// 输入身份：生成器配置摘要 + 来源提交（恢复与检查点对账用）。
pub(crate) fn input_identity(bundle: &laneflow_synth_generator::Bundle) -> String {
    let manifest = bundle.manifest();
    format!("{}@{}", manifest.config_digest, manifest.source_commit)
}
