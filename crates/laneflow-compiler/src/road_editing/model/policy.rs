//! 道路编辑策略输入；字段私有，所有数值和封闭枚举由调用方显式提供。
use super::*;
use crate::{GateInterpretation, GateProhibition};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyEvidenceInput {
    pub(crate) key: Box<str>,
    pub(crate) locator: Box<str>,
    pub(crate) description: Option<Box<str>>,
}
impl PolicyEvidenceInput {
    pub fn try_new(
        key: impl Into<String>,
        locator: impl Into<String>,
        description: Option<String>,
    ) -> Result<Self, DiagnosticBundle> {
        let key = key.into();
        let locator = locator.into();
        validate_token(&key, "policyEvidence.key")?;
        validate_non_empty_text(&locator, "policyEvidence.locator")?;
        Ok(Self {
            key: key.into_boxed_str(),
            locator: locator.into_boxed_str(),
            description: description.map(String::into_boxed_str),
        })
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyGapProfileInput {
    pub(crate) key: Box<str>,
    pub(crate) parameter_version: Box<str>,
    pub(crate) minimum_lead_gap_ms: u64,
    pub(crate) minimum_lag_gap_ms: u64,
    pub(crate) clearance_buffer_ms: u64,
}
impl PolicyGapProfileInput {
    pub fn try_new(
        key: impl Into<String>,
        parameter_version: impl Into<String>,
        minimum_lead_gap_ms: u64,
        minimum_lag_gap_ms: u64,
        clearance_buffer_ms: u64,
    ) -> Result<Self, DiagnosticBundle> {
        let key = key.into();
        let parameter_version = parameter_version.into();
        validate_token(&key, "policyGapProfile.key")?;
        validate_non_empty_text(&parameter_version, "policyGapProfile.parameterVersion")?;
        Ok(Self {
            key: key.into_boxed_str(),
            parameter_version: parameter_version.into_boxed_str(),
            minimum_lead_gap_ms,
            minimum_lag_gap_ms,
            clearance_buffer_ms,
        })
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyStreamRuleInput {
    pub(crate) key: Box<str>,
    pub(crate) stream: ParticipantStreamReference,
    pub(crate) classes: Option<Box<[ParticipantClassReference]>>,
    pub(crate) priority: i32,
    pub(crate) yield_to: Box<[ParticipantStreamReference]>,
    pub(crate) gap: Option<Box<str>>,
    pub(crate) evidence: Box<[Box<str>]>,
}
impl PolicyStreamRuleInput {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        key: impl Into<String>,
        stream: ParticipantStreamReference,
        classes: Option<Vec<ParticipantClassReference>>,
        priority: i32,
        yield_to: Vec<ParticipantStreamReference>,
        gap: Option<String>,
        evidence: Vec<String>,
    ) -> Result<Self, DiagnosticBundle> {
        let key = key.into();
        validate_token(&key, "policyStreamRule.key")?;
        validate_classes(classes.as_deref())?;
        require_unique(&yield_to, "policyStreamRule.yieldToStreams")?;
        if yield_to.is_empty() != gap.is_none() {
            return Err(input_error(
                "policyStreamRule.gapProfileKey",
                RoadEditingInputViolation::InvalidCombination,
            ));
        }
        if let Some(gap) = &gap {
            validate_token(gap, "policyStreamRule.gapProfileKey")?;
        }
        let evidence = evidence_keys(evidence)?;
        Ok(Self {
            key: key.into_boxed_str(),
            stream,
            classes: classes.map(Vec::into_boxed_slice),
            priority,
            yield_to: yield_to.into_boxed_slice(),
            gap: gap.map(String::into_boxed_str),
            evidence,
        })
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyGateRuleInput {
    pub(crate) key: Box<str>,
    pub(crate) gate: ManeuverGateReference,
    pub(crate) classes: Option<Box<[ParticipantClassReference]>>,
    pub(crate) interpretation: GateInterpretation,
    pub(crate) prohibition: GateProhibition,
    pub(crate) evidence: Box<[Box<str>]>,
}
impl PolicyGateRuleInput {
    pub fn try_new(
        key: impl Into<String>,
        gate: ManeuverGateReference,
        classes: Option<Vec<ParticipantClassReference>>,
        interpretation: GateInterpretation,
        prohibition: GateProhibition,
        evidence: Vec<String>,
    ) -> Result<Self, DiagnosticBundle> {
        let key = key.into();
        validate_token(&key, "policyGateRule.key")?;
        validate_classes(classes.as_deref())?;
        Ok(Self {
            key: key.into_boxed_str(),
            gate,
            classes: classes.map(Vec::into_boxed_slice),
            interpretation,
            prohibition,
            evidence: evidence_keys(evidence)?,
        })
    }
}
fn validate_classes(classes: Option<&[ParticipantClassReference]>) -> Result<(), DiagnosticBundle> {
    if let Some(classes) = classes {
        require_non_empty(classes, "policyRule.participantClasses")?;
        require_unique(classes, "policyRule.participantClasses")?;
    }
    Ok(())
}
fn evidence_keys(mut values: Vec<String>) -> Result<Box<[Box<str>]>, DiagnosticBundle> {
    for value in &values {
        validate_token(value, "policyRule.evidenceKeys")?;
    }
    require_unique(&values, "policyRule.evidenceKeys")?;
    values.sort_unstable();
    Ok(values.into_iter().map(String::into_boxed_str).collect())
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RightOfWayPolicySetInput {
    pub(crate) key: Box<str>,
    pub(crate) regulation: RegulationIdentity,
    pub(crate) evidence: Box<[PolicyEvidenceInput]>,
    pub(crate) gaps: Box<[PolicyGapProfileInput]>,
    pub(crate) streams: Box<[PolicyStreamRuleInput]>,
    pub(crate) gates: Box<[PolicyGateRuleInput]>,
    canvas_selection: Option<Box<str>>,
}
impl RightOfWayPolicySetInput {
    pub fn try_new(
        key: impl Into<String>,
        regulation: RegulationIdentity,
        mut evidence: Vec<PolicyEvidenceInput>,
        mut gaps: Vec<PolicyGapProfileInput>,
        mut streams: Vec<PolicyStreamRuleInput>,
        mut gates: Vec<PolicyGateRuleInput>,
    ) -> Result<Self, DiagnosticBundle> {
        let key = key.into();
        validate_token(&key, "rightOfWayPolicySet.key")?;
        regulation.validate()?;
        macro_rules! sort_unique {
            ($values:ident) => {
                $values.sort_unstable_by(|a, b| a.key.cmp(&b.key));
                if $values.windows(2).any(|p| p[0].key == p[1].key) {
                    return Err(input_error(
                        "rightOfWayPolicySet.memberKey",
                        RoadEditingInputViolation::InvalidCombination,
                    ));
                }
            };
        }
        sort_unique!(evidence);
        sort_unique!(gaps);
        sort_unique!(streams);
        sort_unique!(gates);
        Ok(Self {
            key: key.into_boxed_str(),
            regulation,
            evidence: evidence.into_boxed_slice(),
            gaps: gaps.into_boxed_slice(),
            streams: streams.into_boxed_slice(),
            gates: gates.into_boxed_slice(),
            canvas_selection: None,
        })
    }
    #[must_use]
    pub fn policy_set_key(&self) -> &str {
        &self.key
    }
}
impl RightOfWayPolicySetInput {
    /// 附带宿主的可选选择标记；显式空值与缺省值保持不同，字节预算由 builder 检查。
    pub fn with_canvas_selection(
        mut self,
        value: impl Into<String>,
    ) -> Result<Self, DiagnosticBundle> {
        self.canvas_selection = Some(value.into().into_boxed_str());
        Ok(self)
    }

    #[must_use]
    pub fn canvas_selection(&self) -> Option<&str> {
        self.canvas_selection.as_deref()
    }
}
