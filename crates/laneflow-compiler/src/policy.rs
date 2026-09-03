//! 路权策略阶段的共同诊断与有界计量。

use crate::{CompileLimitDimension, CompileLimits, Diagnostic, DiagnosticBundle, SourceLocation};
pub(crate) mod model;

pub(crate) fn compare_identity_text(a: &str, b: &str) -> core::cmp::Ordering {
    (a.len() as u32)
        .to_le_bytes()
        .cmp(&(b.len() as u32).to_le_bytes())
        .then_with(|| a.as_bytes().cmp(b.as_bytes()))
}

/// 规则输入或静态解析不能成立的结构化原因。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PolicyViolation {
    InvalidKey,
    EmptyValue,
    InvalidRegulation,
    SourceDocument,
    DuplicateMember,
    DuplicateReference,
    EmptyClasses,
    MissingLocalReference,
    MissingEvidence,
    GapBinding,
    MissingRule,
    AmbiguousRule,
    SignalBinding,
    RightTurnRequired,
    LampTypeConflict,
    SelfYield,
    DisjointYield,
    YieldPriority,
    RegulationMismatch,
    ProtectedConflict,
}

pub(crate) fn error(
    key: &str,
    member: Option<&str>,
    violation: PolicyViolation,
    source: &SourceLocation,
) -> DiagnosticBundle {
    DiagnosticBundle::single(Diagnostic::invalid_policy(
        key,
        member,
        violation,
        source.clone(),
    ))
}

pub(crate) fn check_budget(
    limits: &CompileLimits,
    scratch: u64,
    live: u64,
) -> Result<(), DiagnosticBundle> {
    for (dimension, actual) in [
        (CompileLimitDimension::StageScratchBytes, scratch),
        (CompileLimitDimension::CompilerControlledLiveBytes, live),
    ] {
        let limit = limits.value(dimension);
        if actual > limit {
            return Err(DiagnosticBundle::single(
                Diagnostic::compile_limit_exceeded(dimension, limit, actual),
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_local_declaration(
    policy: &crate::declaration::RightOfWayPolicySetDeclaration,
) -> Result<(), DiagnosticBundle> {
    use PolicyViolation as V;
    let key = policy.header.stable_key.as_ref();
    let fail = |member: Option<&str>, violation| error(key, member, violation, &policy.header.span);
    if policy.regulation.validate().is_err() {
        return Err(fail(None, V::InvalidRegulation));
    }
    macro_rules! unique_members {
        ($members:expr) => {
            for pair in $members.windows(2) {
                if pair[0].key >= pair[1].key {
                    return Err(fail(Some(&pair[1].key), V::DuplicateMember));
                }
            }
        };
    }
    unique_members!(policy.evidence);
    unique_members!(policy.gap_profiles);
    unique_members!(policy.stream_rules);
    unique_members!(policy.gate_rules);
    let evidence = |rule: &str, values: &[std::sync::Arc<str>]| -> Result<(), DiagnosticBundle> {
        if values.is_empty() && policy.regulation.source.is_none() {
            return Err(fail(Some(rule), V::MissingEvidence));
        }
        if values.windows(2).any(|v| v[0] >= v[1]) {
            return Err(fail(Some(rule), V::DuplicateReference));
        }
        for value in values {
            if policy
                .evidence
                .binary_search_by(|entry| entry.key.cmp(value))
                .is_err()
            {
                return Err(fail(Some(rule), V::MissingLocalReference));
            }
        }
        Ok(())
    };
    for rule in &policy.stream_rules {
        if rule.classes.as_ref().is_some_and(|v| v.is_empty()) {
            return Err(fail(Some(&rule.key), V::EmptyClasses));
        }
        if rule.yield_to.is_empty() != rule.gap.is_none() {
            return Err(fail(Some(&rule.key), V::GapBinding));
        }
        if let Some(gap) = &rule.gap
            && policy
                .gap_profiles
                .binary_search_by(|v| v.key.cmp(gap))
                .is_err()
        {
            return Err(fail(Some(&rule.key), V::MissingLocalReference));
        }
        if duplicate_references(&rule.yield_to)
            || rule
                .classes
                .as_ref()
                .is_some_and(|v| duplicate_references(v))
        {
            return Err(fail(Some(&rule.key), V::DuplicateReference));
        }
        evidence(&rule.key, &rule.evidence)?;
    }
    for rule in &policy.gate_rules {
        if rule.classes.as_ref().is_some_and(|v| v.is_empty()) {
            return Err(fail(Some(&rule.key), V::EmptyClasses));
        }
        if rule
            .classes
            .as_ref()
            .is_some_and(|v| duplicate_references(v))
        {
            return Err(fail(Some(&rule.key), V::DuplicateReference));
        }
        evidence(&rule.key, &rule.evidence)?;
    }
    Ok(())
}

pub(crate) fn sort_references<K: laneflow_static_contract::EntityKindMarker>(
    values: &mut [crate::declaration::OwnedEntityReference<K>],
) {
    values.sort_unstable_by(|a, b| {
        a.module_namespace
            .cmp(&b.module_namespace)
            .then(a.target_address.cmp(&b.target_address))
    });
}
fn duplicate_references<K: laneflow_static_contract::EntityKindMarker>(
    values: &[crate::declaration::OwnedEntityReference<K>],
) -> bool {
    values.windows(2).any(|v| {
        v[0].module_namespace == v[1].module_namespace && v[0].target_address == v[1].target_address
    })
}
