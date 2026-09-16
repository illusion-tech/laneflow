//! 两个正式前端共用的策略语义；来源位置仅留在受检 AST 伴随记录。

use super::{DeclarationHeader, EntityReference, OwnedEntityReference};
use crate::{GateInterpretation, GateProhibition, RegulationIdentity, SourceLocation, SourceSpan};
use laneflow_static_contract::{ManeuverGateKind, ParticipantClassKind, ParticipantStreamKind};
use std::sync::Arc;

/// 保留已有有类型引用，同时显式携带目标来源的完整所有者键链。
/// Synthetic 模块级地址使用空链；引用 Road Editing 的 owner-scoped 声明时提供完整链。
#[derive(Debug)]
pub struct OwnerQualifiedReference<'a, K: laneflow_static_contract::EntityKindMarker> {
    /// 被引用的目标实体。
    pub target: EntityReference<'a, K>,
    /// 目标来源的完整所有者键链；父先子后顺序。
    pub owner_keys: &'a [&'a str],
}
impl<K: laneflow_static_contract::EntityKindMarker> Copy for OwnerQualifiedReference<'_, K> {}
impl<K: laneflow_static_contract::EntityKindMarker> Clone for OwnerQualifiedReference<'_, K> {
    fn clone(&self) -> Self {
        *self
    }
}

/// Synthetic 规则的真实文本位置，可以让多个字段复用同一受检 span。
#[derive(Clone, Copy, Debug)]
pub struct PolicyInputSource<'a> {
    /// 字段的主受检文本 span。
    pub primary: &'a SourceSpan,
    /// 共同构成该字段语义的伴随受检 span。
    pub contributing: &'a [SourceSpan],
}

/// Synthetic 前端的策略证据条目输入。
#[derive(Clone, Copy, Debug)]
pub struct PolicyEvidenceInput<'a> {
    /// owner-local 证据条目键。
    pub evidence_key: &'a str,
    /// 定位法规条款的 locator 字符串。
    pub locator: &'a str,
    /// 可选的人类可读描述。
    pub description: Option<&'a str>,
    /// 该条目的真实文本位置。
    pub source: PolicyInputSource<'a>,
}

/// Synthetic 前端的冲突间隙参数档输入。
#[derive(Clone, Copy, Debug)]
pub struct PolicyGapProfileInput<'a> {
    /// owner-local 参数档键。
    pub profile_key: &'a str,
    /// 参数值集合的版本标识。
    pub parameter_version: &'a str,
    /// 最小领先间隙，单位毫秒。
    pub minimum_lead_gap_ms: u64,
    /// 最小滞后间隙，单位毫秒。
    pub minimum_lag_gap_ms: u64,
    /// 清空缓冲，单位毫秒。
    pub clearance_buffer_ms: u64,
    /// 该参数档的真实文本位置。
    pub source: PolicyInputSource<'a>,
}

/// Synthetic 前端的参与者流让行规则输入。
#[derive(Clone, Copy, Debug)]
pub struct PolicyStreamRuleInput<'a> {
    /// owner-local 规则键。
    pub rule_key: &'a str,
    /// 规则针对的目标参与者流。
    pub stream: OwnerQualifiedReference<'a, ParticipantStreamKind>,
    /// 可选的准入参与者类别过滤；缺省为兜底规则（rank 0）——与显式命中
    /// 选择器并存时后者胜出，并非等同「全部类别」过滤器。
    pub participant_classes: Option<&'a [EntityReference<'a, ParticipantClassKind>]>,
    /// 法规优先级；输入语义由来源法定义，LaneFlow 裁决固定按数值更大者更强。
    pub priority: i32,
    /// 让行目标参与者流列表。
    pub yield_to_streams: &'a [OwnerQualifiedReference<'a, ParticipantStreamKind>],
    /// 可选引用的间隙参数档键。
    pub gap_profile_key: Option<&'a str>,
    /// 引用的策略依据条目键列表。
    pub evidence_keys: &'a [&'a str],
    /// 该规则的真实文本位置。
    pub source: PolicyInputSource<'a>,
}

/// Synthetic 前端的机动门通行规则输入。
#[derive(Clone, Copy, Debug)]
pub struct PolicyGateRuleInput<'a> {
    /// owner-local 规则键。
    pub rule_key: &'a str,
    /// 规则针对的目标机动门。
    pub gate: OwnerQualifiedReference<'a, ManeuverGateKind>,
    /// 可选的准入参与者类别过滤；缺省为兜底规则（rank 0）——与显式命中
    /// 选择器并存时后者胜出，并非等同「全部类别」过滤器。
    pub participant_classes: Option<&'a [EntityReference<'a, ParticipantClassKind>]>,
    /// 该门与类别的封闭灯态解释声明。
    pub interpretation: GateInterpretation,
    /// 独立于灯型的显式门禁令。
    pub prohibition: GateProhibition,
    /// 引用的策略依据条目键列表。
    pub evidence_keys: &'a [&'a str],
    /// 该规则的真实文本位置。
    pub source: PolicyInputSource<'a>,
}

/// 一份完整策略及其 owner-local 成员，不允许跨模块追加匿名成员。
#[derive(Clone, Copy, Debug)]
pub struct RightOfWayPolicySetInput<'a> {
    /// 模块内唯一的策略集稳定键。
    pub policy_set_key: &'a str,
    /// 固定策略含义的法规身份。
    pub regulation: RegulationIdentity<&'a str>,
    /// 策略依据条目成员。
    pub evidence: &'a [PolicyEvidenceInput<'a>],
    /// 冲突间隙参数档成员。
    pub gap_profiles: &'a [PolicyGapProfileInput<'a>],
    /// 参与者流让行规则成员。
    pub stream_rules: &'a [PolicyStreamRuleInput<'a>],
    /// 机动门合规规则成员。
    pub gate_rules: &'a [PolicyGateRuleInput<'a>],
    /// 该策略集的真实文本位置。
    pub source: PolicyInputSource<'a>,
}

/// 策略成员声明的主来源位置与伴随来源位置。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PolicyDeclarationSource {
    /// 成员声明的主来源位置。
    pub primary: SourceLocation,
    /// 共同构成该成员语义的伴随来源位置。
    pub contributing: Box<[SourceLocation]>,
}

/// Typed AST 中已拥有的策略证据条目声明。
pub(crate) struct PolicyEvidenceDeclaration {
    /// owner-local 证据条目键。
    pub key: Arc<str>,
    /// 定位法规条款的 locator 字符串。
    pub locator: Arc<str>,
    /// 可选的人类可读描述。
    pub description: Option<Arc<str>>,
    /// 该条目的来源记录。
    pub source: PolicyDeclarationSource,
}
/// Typed AST 中已拥有的间隙参数档声明。
pub(crate) struct PolicyGapProfileDeclaration {
    /// owner-local 参数档键。
    pub key: Arc<str>,
    /// 参数值集合的版本标识。
    pub parameter_version: Arc<str>,
    /// 最小领先间隙，单位毫秒。
    pub minimum_lead_gap_ms: u64,
    /// 最小滞后间隙，单位毫秒。
    pub minimum_lag_gap_ms: u64,
    /// 清空缓冲，单位毫秒。
    pub clearance_buffer_ms: u64,
    /// 该参数档的来源记录。
    pub source: PolicyDeclarationSource,
}
/// Typed AST 中已拥有的参与者流让行规则声明。
pub(crate) struct PolicyStreamRuleDeclaration {
    /// owner-local 规则键。
    pub key: Arc<str>,
    /// 规则针对的目标参与者流。
    pub stream: OwnedEntityReference<ParticipantStreamKind>,
    /// 可选的准入参与者类别过滤；缺省为兜底规则（rank 0）——与显式命中
    /// 选择器并存时后者胜出，并非等同「全部类别」过滤器。
    pub classes: Option<Box<[OwnedEntityReference<ParticipantClassKind>]>>,
    /// 法规优先级；输入语义由来源法定义，LaneFlow 裁决固定按数值更大者更强。
    pub priority: i32,
    /// 让行目标参与者流列表。
    pub yield_to: Box<[OwnedEntityReference<ParticipantStreamKind>]>,
    /// 可选引用的间隙参数档键。
    pub gap: Option<Arc<str>>,
    /// 引用的策略依据条目键列表。
    pub evidence: Box<[Arc<str>]>,
    /// 该规则的来源记录。
    pub source: PolicyDeclarationSource,
}
/// Typed AST 中已拥有的机动门通行规则声明。
pub(crate) struct PolicyGateRuleDeclaration {
    /// owner-local 规则键。
    pub key: Arc<str>,
    /// 规则针对的目标机动门。
    pub gate: OwnedEntityReference<ManeuverGateKind>,
    /// 可选的准入参与者类别过滤；缺省为兜底规则（rank 0）——与显式命中
    /// 选择器并存时后者胜出，并非等同「全部类别」过滤器。
    pub classes: Option<Box<[OwnedEntityReference<ParticipantClassKind>]>>,
    /// 该门与类别的封闭灯态解释声明。
    pub interpretation: GateInterpretation,
    /// 独立于灯型的显式门禁令。
    pub prohibition: GateProhibition,
    /// 引用的策略依据条目键列表。
    pub evidence: Box<[Arc<str>]>,
    /// 该规则的来源记录。
    pub source: PolicyDeclarationSource,
}
/// 已通过字段级检查的路权策略集 Typed AST 声明。
pub(crate) struct RightOfWayPolicySetDeclaration {
    /// 策略集声明头：稳定键与声明来源。
    pub header: DeclarationHeader,
    /// 固定策略含义的法规身份。
    pub regulation: RegulationIdentity<Arc<str>>,
    /// 策略依据条目成员。
    pub evidence: Box<[PolicyEvidenceDeclaration]>,
    /// 冲突间隙参数档成员。
    pub gap_profiles: Box<[PolicyGapProfileDeclaration]>,
    /// 参与者流让行规则成员。
    pub stream_rules: Box<[PolicyStreamRuleDeclaration]>,
    /// 机动门合规规则成员。
    pub gate_rules: Box<[PolicyGateRuleDeclaration]>,
    /// 策略集级别的伴随来源位置。
    pub contributing: Box<[SourceLocation]>,
}

impl RightOfWayPolicySetDeclaration {
    /// 按声明顺序迭代全部具名成员的来源记录。
    pub(crate) fn sources(&self) -> impl Iterator<Item = &PolicyDeclarationSource> {
        self.evidence
            .iter()
            .map(|v| &v.source)
            .chain(self.gap_profiles.iter().map(|v| &v.source))
            .chain(self.stream_rules.iter().map(|v| &v.source))
            .chain(self.gate_rules.iter().map(|v| &v.source))
    }

    /// 以声明内规范结构顺序访问全部来源位置。
    pub(crate) fn try_visit_source_locations<E>(
        &self,
        mut visit: impl FnMut(&SourceLocation) -> Result<(), E>,
    ) -> Result<(), E> {
        super::try_visit_declaration_header(&self.header, &mut visit)?;
        for location in &self.contributing {
            visit(location)?;
        }
        for source in self.sources() {
            visit(&source.primary)?;
            for location in &source.contributing {
                visit(location)?;
            }
        }
        for rule in &self.stream_rules {
            super::try_visit_reference(&rule.stream, &mut visit)?;
            super::try_visit_references(&rule.yield_to, &mut visit)?;
            if let Some(classes) = &rule.classes {
                super::try_visit_references(classes, &mut visit)?;
            }
        }
        for rule in &self.gate_rules {
            super::try_visit_reference(&rule.gate, &mut visit)?;
            if let Some(classes) = &rule.classes {
                super::try_visit_references(classes, &mut visit)?;
            }
        }
        Ok(())
    }
}
