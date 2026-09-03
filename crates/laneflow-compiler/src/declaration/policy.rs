//! 两个正式前端共用的策略语义；来源位置仅留在受检 AST 伴随记录。

use super::{DeclarationHeader, EntityReference, OwnedEntityReference};
use crate::{GateInterpretation, GateProhibition, RegulationIdentity, SourceLocation, SourceSpan};
use laneflow_static_contract::{ManeuverGateKind, ParticipantClassKind, ParticipantStreamKind};
use std::sync::Arc;

/// 保留已有有类型引用，同时显式携带目标来源的完整所有者键链。
/// Synthetic 模块级地址使用空链；引用 Road Editing 的 owner-scoped 声明时提供完整链。
#[derive(Debug)]
pub struct OwnerQualifiedReference<'a, K: laneflow_static_contract::EntityKindMarker> {
    pub target: EntityReference<'a, K>,
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
    pub primary: &'a SourceSpan,
    pub contributing: &'a [SourceSpan],
}

#[derive(Clone, Copy, Debug)]
pub struct PolicyEvidenceInput<'a> {
    pub evidence_key: &'a str,
    pub locator: &'a str,
    pub description: Option<&'a str>,
    pub source: PolicyInputSource<'a>,
}

#[derive(Clone, Copy, Debug)]
pub struct PolicyGapProfileInput<'a> {
    pub profile_key: &'a str,
    pub parameter_version: &'a str,
    pub minimum_lead_gap_ms: u64,
    pub minimum_lag_gap_ms: u64,
    pub clearance_buffer_ms: u64,
    pub source: PolicyInputSource<'a>,
}

#[derive(Clone, Copy, Debug)]
pub struct PolicyStreamRuleInput<'a> {
    pub rule_key: &'a str,
    pub stream: OwnerQualifiedReference<'a, ParticipantStreamKind>,
    pub participant_classes: Option<&'a [EntityReference<'a, ParticipantClassKind>]>,
    pub priority: i32,
    pub yield_to_streams: &'a [OwnerQualifiedReference<'a, ParticipantStreamKind>],
    pub gap_profile_key: Option<&'a str>,
    pub evidence_keys: &'a [&'a str],
    pub source: PolicyInputSource<'a>,
}

#[derive(Clone, Copy, Debug)]
pub struct PolicyGateRuleInput<'a> {
    pub rule_key: &'a str,
    pub gate: OwnerQualifiedReference<'a, ManeuverGateKind>,
    pub participant_classes: Option<&'a [EntityReference<'a, ParticipantClassKind>]>,
    pub interpretation: GateInterpretation,
    pub prohibition: GateProhibition,
    pub evidence_keys: &'a [&'a str],
    pub source: PolicyInputSource<'a>,
}

/// 一份完整策略及其 owner-local 成员，不允许跨模块追加匿名成员。
#[derive(Clone, Copy, Debug)]
pub struct RightOfWayPolicySetInput<'a> {
    pub policy_set_key: &'a str,
    pub regulation: RegulationIdentity<&'a str>,
    pub evidence: &'a [PolicyEvidenceInput<'a>],
    pub gap_profiles: &'a [PolicyGapProfileInput<'a>],
    pub stream_rules: &'a [PolicyStreamRuleInput<'a>],
    pub gate_rules: &'a [PolicyGateRuleInput<'a>],
    pub source: PolicyInputSource<'a>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PolicyDeclarationSource {
    pub primary: SourceLocation,
    pub contributing: Box<[SourceLocation]>,
}

pub(crate) struct PolicyEvidenceDeclaration {
    pub key: Arc<str>,
    pub locator: Arc<str>,
    pub description: Option<Arc<str>>,
    pub source: PolicyDeclarationSource,
}
pub(crate) struct PolicyGapProfileDeclaration {
    pub key: Arc<str>,
    pub parameter_version: Arc<str>,
    pub minimum_lead_gap_ms: u64,
    pub minimum_lag_gap_ms: u64,
    pub clearance_buffer_ms: u64,
    pub source: PolicyDeclarationSource,
}
pub(crate) struct PolicyStreamRuleDeclaration {
    pub key: Arc<str>,
    pub stream: OwnedEntityReference<ParticipantStreamKind>,
    pub classes: Option<Box<[OwnedEntityReference<ParticipantClassKind>]>>,
    pub priority: i32,
    pub yield_to: Box<[OwnedEntityReference<ParticipantStreamKind>]>,
    pub gap: Option<Arc<str>>,
    pub evidence: Box<[Arc<str>]>,
    pub source: PolicyDeclarationSource,
}
pub(crate) struct PolicyGateRuleDeclaration {
    pub key: Arc<str>,
    pub gate: OwnedEntityReference<ManeuverGateKind>,
    pub classes: Option<Box<[OwnedEntityReference<ParticipantClassKind>]>>,
    pub interpretation: GateInterpretation,
    pub prohibition: GateProhibition,
    pub evidence: Box<[Arc<str>]>,
    pub source: PolicyDeclarationSource,
}
pub(crate) struct RightOfWayPolicySetDeclaration {
    pub header: DeclarationHeader,
    pub regulation: RegulationIdentity<Arc<str>>,
    pub evidence: Box<[PolicyEvidenceDeclaration]>,
    pub gap_profiles: Box<[PolicyGapProfileDeclaration]>,
    pub stream_rules: Box<[PolicyStreamRuleDeclaration]>,
    pub gate_rules: Box<[PolicyGateRuleDeclaration]>,
    pub contributing: Box<[SourceLocation]>,
}

impl RightOfWayPolicySetDeclaration {
    pub(crate) fn sources(&self) -> impl Iterator<Item = &PolicyDeclarationSource> {
        self.evidence
            .iter()
            .map(|v| &v.source)
            .chain(self.gap_profiles.iter().map(|v| &v.source))
            .chain(self.stream_rules.iter().map(|v| &v.source))
            .chain(self.gate_rules.iter().map(|v| &v.source))
    }

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
