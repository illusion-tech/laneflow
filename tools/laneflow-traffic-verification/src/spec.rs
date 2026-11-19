//! 验证规约与受检计划的绑定。
//!
//! 规约（specification）声明本次要证明的性质、适用域、必需事实、必需
//! 规则、必需见证及完整窗口；正式规约由各 harness 的私有构造入口给出，
//! 共享包不建立包含全部 workload 的全局枚举。绑定（bind）把规约与本次
//! 实际安装的模块对账：必需项未被完整覆盖、事实提供者缺失、规则适用域
//! 与规约域不符分别报错，不以“不适用”代替（设计 §3.1）。规约是纯静态
//! 声明（`&'static` 载荷），可复制、可逐字段比较，不携带运行状态。

use std::collections::BTreeSet;

/// 必需规则的具名标识。名称由 harness 定义，共享包不解释其含义。
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RuleId(pub &'static str);

/// 必需事实提供者的具名标识。
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct FactId(pub &'static str);

/// 必需场景见证的具名标识。
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct WitnessId(pub &'static str);

/// 规则执行的规定阶段：覆盖核算按 (阶段, 规则) 记录，安装在绑定期检查，
/// 运行时按固定顺序执行，没有逐拍依赖图调度（设计 §3.1/§6）。
#[derive(
    Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, serde::Serialize, serde::Deserialize,
)]
pub enum CheckPhase {
    /// 初始边界：身份、时钟、信号与初始零事件的核对。
    InitialBoundary,
    /// 命令结果：按实际顺序消费成功、Blocked、失败回执。
    CommandResults,
    /// 命令结束 / 拍前：本边界命令全部处理后的拍前事实。
    PreTick,
    /// 成功步进后：消费本次步进结果一次。
    PostSuccessfulStep,
    /// 预期失败步进后：仅原子性/重试规约启用。
    PostExpectedFailure,
}

/// 完整窗口的终结边界：成功步进数与命令阶段的精确组合（设计 §6 以
/// 已完成 `k` 个成功步进的边界为基准）。
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Window {
    /// 规约要求的成功步进总数（窗口长度）。
    pub successful_steps: u64,
}

/// 验证规约：证明目标、适用域与全部必需项的静态声明。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerificationSpec {
    /// 规约名称（进入回执与检查点身份，区分诊断组合与正式 profile 由
    /// harness 构造入口保证）。
    pub name: &'static str,
    /// 适用域标签（如 W 冻结六相位域、Urban 静态政策域）。
    pub domain: &'static str,
    /// 语义身份（沿用既有 `semantic_contract` 一类正本，不另设版本）。
    pub semantic_version: &'static str,
    /// 必需规则：须全部安装且按各自阶段实际执行。
    pub required_rules: &'static [RuleDescriptor],
    /// 必需事实提供者。
    pub required_facts: &'static [FactId],
    /// 必需场景见证：场景明确要求的过门、预约或到达；零触发不凭空失败。
    pub required_witnesses: &'static [WitnessId],
    /// 完整窗口。
    pub window: Window,
    /// 是否启用原子性/重试规约：预期 Runtime 失败步进只在启用时受检，
    /// 且不计为成功步进（设计 §6）。
    pub atomicity: bool,
}

/// 覆盖语义：阶段完成义务与事件触发分开（设计 §3.3——某拍没有过门
/// 事件可以是合法的零触发，但每轮应检查的工作集仍须证明已处理）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoverageMode {
    /// 每轮义务：所属阶段的每一轮（初始边界轮 / 每个成功步进轮 / 每轮
    /// 拍前 / 每条命令）都必须实际运行；阶段关闭时核对，漏一轮即拒绝。
    EveryRound,
    /// 事件触发：仅在实际事件发生时运行；零触发合法并如实记录为 0，
    /// 场景必须发生的义务由必需见证承担，不以触发计数冒充覆盖。
    OnEvent,
}

/// 单条规则的静态描述：标识、支持域、规定执行阶段、覆盖语义与初态
/// 参与声明。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuleDescriptor {
    pub id: RuleId,
    /// 本实现支持的适用域；须包含规约域才允许绑定。
    pub domains: &'static [&'static str],
    /// 规定执行阶段；安装项与规约要求的阶段须一致才允许绑定。
    pub phase: CheckPhase,
    /// 覆盖语义：每轮义务在阶段关闭时核对；事件触发只记实际次数。
    pub coverage: CoverageMode,
    /// 是否也参与初态观测：初态与每个成功步进是两个独立义务实例，
    /// 初态只履行显式声明的规则；仅声明拍后规则的合法规约在第一步
    /// 之前没有初态义务，可正常启动。
    pub initial: bool,
}

/// 本次实际安装的模块清单：与规约声明的必需项分别保存，绑定检查前者
/// 被后者完整覆盖（设计 §3.1）。
#[derive(Clone, Copy, Debug, Default)]
pub struct ProvidedModules {
    pub rules: &'static [RuleDescriptor],
    pub facts: &'static [FactId],
    pub witnesses: &'static [WitnessId],
}

/// 绑定拒绝原因：缺项、事实不足、域不支持与窗口无效分别报错，携带
/// 具体标识，不以“不适用”代替。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanRejected {
    MissingRule(RuleId),
    MissingFact(FactId),
    MissingWitness(WitnessId),
    DomainUnsupported {
        rule: RuleId,
        domain: &'static str,
    },
    /// 规约要求的阶段与安装实现的阶段不一致。
    PhaseMismatch {
        rule: RuleId,
        required: CheckPhase,
        installed: CheckPhase,
    },
    /// 安装清单内同一规则出现多个描述符，绑定不得静默选择其一。
    DuplicateRule(RuleId),
    /// `initial` 只对成功步进后阶段有意义；其他组合在绑定期拒绝，
    /// 不产出无法启动的计划。
    InitialPhaseUnsupported {
        rule: RuleId,
        phase: CheckPhase,
    },
    /// 规约要求参与初态观测而安装实现不支持（或相反），登记按安装、
    /// 核销按规约会产生无法启动的计划，绑定期对账拒绝。
    InitialCapabilityMismatch {
        rule: RuleId,
    },
    EmptyWindow,
}

impl core::fmt::Display for PlanRejected {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MissingRule(rule) => write!(f, "缺少必需规则实现：{}", rule.0),
            Self::MissingFact(fact) => write!(f, "缺少必需事实提供者：{}", fact.0),
            Self::MissingWitness(witness) => write!(f, "缺少必需见证提供者：{}", witness.0),
            Self::DomainUnsupported { rule, domain } => {
                write!(f, "规则 {} 不支持适用域 {}", rule.0, domain)
            }
            Self::PhaseMismatch {
                rule,
                required,
                installed,
            } => write!(
                f,
                "规则 {rule:?} 的阶段错配：要求 {required:?}，安装 {installed:?}"
            ),
            Self::DuplicateRule(rule) => write!(f, "安装清单内规则 {} 重复", rule.0),
            Self::InitialPhaseUnsupported { rule, phase } => {
                write!(f, "规则 {rule:?} 的 initial 声明与阶段 {phase:?} 不兼容")
            }
            Self::InitialCapabilityMismatch { rule } => {
                write!(f, "规则 {:?} 的初态参与声明与安装实现不一致", rule.0)
            }
            Self::EmptyWindow => write!(f, "完整窗口不得为零步"),
        }
    }
}

/// 受检计划：绑定成功后得到的不可变对账结果，进入会话时原样携带。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckedPlan {
    pub spec: VerificationSpec,
    /// 绑定时实际安装并核对通过的规则集（快照语义：会话运行期间不再增删）。
    pub(crate) installed_rules: &'static [RuleDescriptor],
}

impl CheckedPlan {
    /// 绑定：规约必需项与本次安装模块对账，任一不满足即拒绝，不产出
    /// 可运行的降级计划。
    pub fn bind(spec: VerificationSpec, provided: ProvidedModules) -> Result<Self, PlanRejected> {
        if spec.window.successful_steps == 0 {
            return Err(PlanRejected::EmptyWindow);
        }
        let facts: BTreeSet<FactId> = provided.facts.iter().copied().collect();
        for fact in spec.required_facts {
            if !facts.contains(fact) {
                return Err(PlanRejected::MissingFact(*fact));
            }
        }
        let witnesses: BTreeSet<WitnessId> = provided.witnesses.iter().copied().collect();
        for witness in spec.required_witnesses {
            if !witnesses.contains(witness) {
                return Err(PlanRejected::MissingWitness(*witness));
            }
        }
        let mut seen: BTreeSet<RuleId> = BTreeSet::new();
        for installed in provided.rules {
            if !seen.insert(installed.id) {
                return Err(PlanRejected::DuplicateRule(installed.id));
            }
            if installed.initial && installed.phase != CheckPhase::PostSuccessfulStep {
                return Err(PlanRejected::InitialPhaseUnsupported {
                    rule: installed.id,
                    phase: installed.phase,
                });
            }
        }
        for required in spec.required_rules {
            if required.initial && required.phase != CheckPhase::PostSuccessfulStep {
                return Err(PlanRejected::InitialPhaseUnsupported {
                    rule: required.id,
                    phase: required.phase,
                });
            }
            let installed = provided
                .rules
                .iter()
                .find(|rule| rule.id == required.id)
                .ok_or(PlanRejected::MissingRule(required.id))?;
            if !installed.domains.contains(&spec.domain) {
                return Err(PlanRejected::DomainUnsupported {
                    rule: required.id,
                    domain: spec.domain,
                });
            }
            if installed.phase != required.phase {
                return Err(PlanRejected::PhaseMismatch {
                    rule: required.id,
                    required: required.phase,
                    installed: installed.phase,
                });
            }
            if installed.initial != required.initial {
                return Err(PlanRejected::InitialCapabilityMismatch { rule: required.id });
            }
        }
        Ok(Self {
            spec,
            installed_rules: provided.rules,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOMAIN_W: &str = "w-frozen-six-phase";
    const DOMAIN_URBAN: &str = "urban-static-policy";

    const fn rule(
        id: &'static str,
        domains: &'static [&'static str],
        phase: CheckPhase,
    ) -> RuleDescriptor {
        RuleDescriptor {
            id: RuleId(id),
            domains,
            phase,
            coverage: CoverageMode::EveryRound,
            initial: false,
        }
    }

    const fn spec(
        rules: &'static [RuleDescriptor],
        witnesses: &'static [WitnessId],
    ) -> VerificationSpec {
        VerificationSpec {
            name: "lf-synth-w1",
            domain: DOMAIN_W,
            semantic_version: "lf-synth-execution-2",
            required_rules: rules,
            required_facts: &[FactId("motion"), FactId("identity")],
            required_witnesses: witnesses,
            window: Window {
                successful_steps: 8,
            },
            atomicity: false,
        }
    }

    const FULL_RULES: &[RuleDescriptor] = &[rule(
        "body-separation",
        &[DOMAIN_W],
        CheckPhase::PostSuccessfulStep,
    )];
    const FULL_FACTS: &[FactId] = &[FactId("motion"), FactId("identity"), FactId("extra")];
    const FULL_WITNESSES: &[WitnessId] = &[WitnessId("per-active-crossing")];

    fn full_modules() -> ProvidedModules {
        ProvidedModules {
            rules: FULL_RULES,
            facts: FULL_FACTS,
            witnesses: FULL_WITNESSES,
        }
    }

    #[test]
    fn bind_accepts_complete_installation() {
        let plan = CheckedPlan::bind(spec(FULL_RULES, FULL_WITNESSES), full_modules())
            .expect("完整安装必须绑定成功");
        assert_eq!(plan.spec.name, "lf-synth-w1");
    }

    #[test]
    fn bind_rejects_each_missing_piece_explicitly() {
        // 缺规则。
        let empty_rules = ProvidedModules {
            rules: &[],
            ..full_modules()
        };
        assert_eq!(
            CheckedPlan::bind(spec(FULL_RULES, FULL_WITNESSES), empty_rules),
            Err(PlanRejected::MissingRule(RuleId("body-separation")))
        );
        // 缺事实提供者。
        let no_motion = ProvidedModules {
            facts: &[FactId("identity")],
            ..full_modules()
        };
        assert_eq!(
            CheckedPlan::bind(spec(FULL_RULES, FULL_WITNESSES), no_motion),
            Err(PlanRejected::MissingFact(FactId("motion")))
        );
        // 缺场景见证提供者。
        let required_witness: &[WitnessId] = &[WitnessId("w4-green-probe")];
        assert_eq!(
            CheckedPlan::bind(spec(FULL_RULES, required_witness), full_modules()),
            Err(PlanRejected::MissingWitness(WitnessId("w4-green-probe")))
        );
    }

    #[test]
    fn bind_rejects_domain_mismatch_instead_of_marking_na() {
        // 规则实现只支持 Urban 政策域，规约域为 W：拒绝而非“不适用”。
        const URBAN_ONLY: &[RuleDescriptor] = &[rule(
            "body-separation",
            &[DOMAIN_URBAN],
            CheckPhase::PostSuccessfulStep,
        )];
        let urban_only = URBAN_ONLY;
        assert_eq!(
            CheckedPlan::bind(
                spec(urban_only, FULL_WITNESSES),
                ProvidedModules {
                    rules: urban_only,
                    ..full_modules()
                }
            ),
            Err(PlanRejected::DomainUnsupported {
                rule: RuleId("body-separation"),
                domain: DOMAIN_W,
            })
        );
    }

    #[test]
    fn bind_rejects_zero_window() {
        let zero = VerificationSpec {
            window: Window {
                successful_steps: 0,
            },
            ..spec(FULL_RULES, FULL_WITNESSES)
        };
        assert_eq!(
            CheckedPlan::bind(zero, full_modules()),
            Err(PlanRejected::EmptyWindow)
        );
    }

    #[test]
    fn bind_rejects_phase_mismatch_and_duplicate_rules() {
        // 规约要求 PostSuccessfulStep，安装实现声明 PreTick：阶段错配拒绝。
        let phase_wrong: &[RuleDescriptor] = &[RuleDescriptor {
            id: RuleId("body-separation"),
            domains: &[DOMAIN_W],
            phase: CheckPhase::PreTick,
            coverage: CoverageMode::EveryRound,
            initial: false,
        }];
        assert_eq!(
            CheckedPlan::bind(
                spec(FULL_RULES, FULL_WITNESSES),
                ProvidedModules {
                    rules: phase_wrong,
                    ..full_modules()
                }
            ),
            Err(PlanRejected::PhaseMismatch {
                rule: RuleId("body-separation"),
                required: CheckPhase::PostSuccessfulStep,
                installed: CheckPhase::PreTick,
            })
        );
        // 同一规则两个描述符：拒绝，不静默取第一个。
        const DUPLICATED: &[RuleDescriptor] = &[
            rule(
                "body-separation",
                &[DOMAIN_W],
                CheckPhase::PostSuccessfulStep,
            ),
            rule(
                "body-separation",
                &[DOMAIN_W],
                CheckPhase::PostSuccessfulStep,
            ),
        ];
        let duplicated = DUPLICATED;
        assert_eq!(
            CheckedPlan::bind(
                spec(duplicated, FULL_WITNESSES),
                ProvidedModules {
                    rules: duplicated,
                    ..full_modules()
                }
            ),
            Err(PlanRejected::DuplicateRule(RuleId("body-separation")))
        );
    }
}
