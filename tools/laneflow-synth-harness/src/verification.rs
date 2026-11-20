//! Synth 的正式验证规约构造入口（设计 §3.1：私有模块，按受检输入身份
//! 与执行目的选择，共享包不建全局枚举）。
//!
//! W1/W2 普通 run 的规约：五条必需规则（运动身份、车身分离、W 走廊
//! 顺序、信号许可、跨门事件配对）、四类必需事实、每辆 Active 恰一次
//! 过门的场景见证；窗口为受检计划的完整预热+观察长度，不允许短窗口。
//! W3/W4 burst 的规约在此追加命令结果阶段规则与场景见证（到达屏障、
//! 停车/离位、W4 Green 探针——均为冻结计划的结构性事件，设计 §10；
//! W3 的 Blocked 容错语义由注入探针钉住，不作必需见证）。L1 换道样本
//! 不在本规约内（设计 §10：L1 明确独立组合身份，首批不重写）。

use laneflow_synth_generator::Case;
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

/// 边界命令的记录/游标/身份代次/命令后基线权限核验（设计 §6 命令结果
/// 阶段）：每条命令一个独立义务实例，成功与 Blocked 都要实际核验并
/// 按实际顺序消费结果。
pub(crate) const RULE_COMMANDS: RuleDescriptor = RuleDescriptor {
    id: RuleId("w-command-authority"),
    domains: &[DOMAIN],
    phase: CheckPhase::CommandResults,
    coverage: CoverageMode::EveryRound,
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

const BURST_RULES: &[RuleDescriptor] = &[
    RULE_MOTION,
    RULE_BODY,
    RULE_ORDER,
    RULE_SIGNAL,
    RULE_GATES,
    RULE_COMMANDS,
];
const BURST_FACTS: &[FactId] = &[
    FactId("motion"),
    FactId("identity"),
    FactId("signal-plan"),
    FactId("gate-events"),
    FactId("command-results"),
];

/// W3 场景见证：每辆初始 Active 恰一次过门、全部预约车到达、屏障停车、
/// 离位（冻结计划的到达屏障规则结构性地触发全部四类事件）。冻结 W3
/// 输入的完成替换在替换位（走廊 3 的 183.5–188 m）恒为空走廊、不被
/// Blocked；计划规则的 blocked 是容错口径（若被拒→同输入后边界重试），
/// 不列为必需见证，语义由注入探针钉住（§10 命令行）。
const W3_WITNESSES: &[WitnessId] = &[
    WitnessId("per-active-crossing"),
    WitnessId("w3-parking-arrival"),
    WitnessId("w3-park-committed"),
    WitnessId("w3-leave-committed"),
];

/// W4 场景见证：B1 指定过门且许可原因为 Green（强于通用许可，不允许
/// 任意合法黄灯替代，设计 §4.2/§10）。
const W4_WITNESSES: &[WitnessId] = &[
    WitnessId("per-active-crossing"),
    WitnessId("w4-green-probe"),
];

pub(crate) const WITNESS_ARRIVAL: WitnessId = WitnessId("w3-parking-arrival");
pub(crate) const WITNESS_PARK: WitnessId = WitnessId("w3-park-committed");
pub(crate) const WITNESS_LEAVE: WitnessId = WitnessId("w3-leave-committed");
pub(crate) const WITNESS_GREEN_PROBE: WitnessId = WitnessId("w4-green-probe");

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

/// W3/W4 burst 正式规约：在 W1/W2 五条必需规则上追加命令结果阶段规则，
/// 必需事实与场景见证按场景族选择（设计 §10）。`total` 为本次执行的
/// 完整窗口长度（正式 run 与参考 run 各自的终点）。
pub(crate) fn w_burst_spec(case: &Case, total: u64) -> VerificationSpec {
    let (name, witnesses) = match case {
        Case::W3 => ("lf-synth-w3-burst-semantic-run", W3_WITNESSES),
        Case::W4 => ("lf-synth-w4-burst-semantic-run", W4_WITNESSES),
        other => unreachable!("burst 规约只接受 W3/W4，收到 {other:?}"),
    };
    VerificationSpec {
        name,
        domain: DOMAIN,
        semantic_version: SEMANTIC_CONTRACT,
        required_rules: BURST_RULES,
        required_facts: BURST_FACTS,
        required_witnesses: witnesses,
        window: Window {
            successful_steps: total,
        },
        atomicity: false,
    }
}

/// 按场景族构造 burst 会话的安装模块清单（与规约声明分别保存，绑定期
/// 对账前者完整覆盖后者）。
fn burst_provided(case: &Case) -> ProvidedModules {
    ProvidedModules {
        rules: BURST_RULES,
        facts: BURST_FACTS,
        witnesses: match case {
            Case::W3 => W3_WITNESSES,
            Case::W4 => W4_WITNESSES,
            other => unreachable!("burst 安装清单只接受 W3/W4，收到 {other:?}"),
        },
    }
}

/// 构造并绑定 burst 会话，随后确立初始边界（正式 run 与恢复参考的共用
/// 入口；绑定失败即错误，不产出降级计划）。
pub(crate) fn bind_burst_session(
    sample: &mut crate::install::Sample,
    bundle: &laneflow_synth_generator::Bundle,
    case: &Case,
    total: u64,
) -> Result<(), crate::Error> {
    let mut session = Session::start(w_burst_spec(case, total), burst_provided(case))
        .map_err(|rejected| crate::Error::Invalid(format!("验证规约绑定失败：{rejected}")))?;
    session
        .confirm_initial_boundary(0)
        .map_err(|e| crate::Error::Invalid(format!("验证会话初始边界：{e:?}")))?;
    sample.session = Some(session);
    sample.input_identity = input_identity(bundle);
    Ok(())
}

/// 从宿主恢复材料重建 burst 会话（设计 §7：恢复时从同一正式构造入口
/// 重新绑定计划，逐项对账身份与结构；缺材料或身份错配拒绝）。
pub(crate) fn restore_burst_session(
    material: &serde_json::Value,
    bundle: &laneflow_synth_generator::Bundle,
    case: &Case,
    total: u64,
) -> Result<Session, crate::Error> {
    let checkpoint = serde_json::from_value::<
        laneflow_traffic_verification::session::SessionCheckpoint,
    >(material.clone())
    .map_err(|e| crate::Error::Invalid(format!("验证检查点材料解码失败：{e}")))?;
    Session::restore(
        &checkpoint,
        w_burst_spec(case, total),
        burst_provided(case),
        &input_identity(bundle),
    )
    .map_err(|rejected| crate::Error::Invalid(format!("验证会话恢复被拒：{rejected:?}")))
}

/// 输入身份：生成器配置摘要 + 来源提交（恢复与检查点对账用）。
pub(crate) fn input_identity(bundle: &laneflow_synth_generator::Bundle) -> String {
    let manifest = bundle.manifest();
    format!("{}@{}", manifest.config_digest, manifest.source_commit)
}
