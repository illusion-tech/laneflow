//! LFRE v4 的策略字段预检；只借用 wire，不在预收费前建立集合。
use super::*;
use laneflow_road_editing_wire::runtime::Table;

pub(super) fn closed(table: Table<'_>, fields: usize, key: &str) -> Result<(), DiagnosticBundle> {
    let vtable = table.vtable();
    if (fields..vtable.num_fields()).any(|index| vtable.get_field(index) != 0) {
        return Err(semantic_error(
            "unknownField",
            RoadEditingInputViolation::InvalidCombination,
            key,
        ));
    }
    Ok(())
}

fn text(
    usage: &mut RoadEditingPreflightCounts,
    value: &str,
    limits: &CompileLimits,
) -> Result<(), DiagnosticBundle> {
    let observed = value.len() as u64;
    let limit = limits.value(CompileLimitDimension::SingleStringBytes);
    if observed > limit {
        return Err(limit_error(
            CompileLimitDimension::SingleStringBytes,
            limit,
            observed,
        ));
    }
    usage.string_item_count = usage.string_item_count.saturating_add(1);
    usage.total_string_bytes = usage.total_string_bytes.saturating_add(observed);
    Ok(())
}

pub(super) fn validate(
    usage: &mut RoadEditingPreflightCounts,
    root: wire::RoadEditingSource<'_>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &CompileLimits,
    key: &str,
) -> Result<(), DiagnosticBundle> {
    closed(root._tab, 30, key)?;
    for movement in root.movements() {
        closed(movement._tab, 6, key)?;
    }
    // 复用无分配查重前先界定比较量，不能给大批不同策略键引入平方级无界工作。
    let policy_count = root.right_of_way_policy_sets().len() as u64;
    let comparisons = policy_count.saturating_mul(policy_count.saturating_sub(1)) / 2;
    let relation_limit = limits.value(CompileLimitDimension::RelationOccurrenceCount);
    if comparisons > relation_limit {
        return Err(limit_error(
            CompileLimitDimension::RelationOccurrenceCount,
            relation_limit,
            comparisons,
        ));
    }
    ensure_unique_by(
        root.right_of_way_policy_sets().iter(),
        |value| value.policy_set_key(),
        "rightOfWayPolicySets.policySetKey",
        key,
    )?;
    for policy in root.right_of_way_policy_sets() {
        closed(policy._tab, 7, key)?;
        closed(policy.regulation()._tab, 3, key)?;
        usage.charge_declaration(EntityKind::RightOfWayPolicySet);
        usage.typed_ast_record_count = usage.typed_ast_record_count.saturating_add(1);
        usage.charge_token(policy.policy_set_key(), "policy.policySetKey", limits, key)?;
        let regulation = policy.regulation();
        for value in [
            Some(regulation.jurisdiction()),
            Some(regulation.version()),
            regulation.source(),
        ]
        .into_iter()
        .flatten()
        {
            if !crate::regulation::valid_text(value) {
                return Err(semantic_error(
                    "regulation",
                    RoadEditingInputViolation::InvalidCombination,
                    key,
                ));
            }
            text(usage, value, limits)?;
        }
        if let Some(canvas) = policy.canvas_selection() {
            text(usage, canvas, limits)?;
        }
        usage.charge_relation(3 + usize::from(regulation.source().is_some()));
        for count in [
            policy.evidence().len(),
            policy.gap_profiles().len(),
            policy.stream_rules().len(),
            policy.gate_rules().len(),
        ] {
            usage.typed_ast_record_count =
                usage.typed_ast_record_count.saturating_add(count as u64);
        }
        for value in policy.evidence() {
            closed(value._tab, 3, key)?;
            usage.charge_token(value.evidence_key(), "policyEvidence.key", limits, key)?;
            usage.charge_non_empty_text(value.locator(), "policyEvidence.locator", limits, key)?;
            if let Some(description) = value.description() {
                text(usage, description, limits)?;
            }
            usage.charge_relation(3 + usize::from(value.description().is_some()));
        }
        for value in policy.gap_profiles() {
            closed(value._tab, 5, key)?;
            usage.charge_token(value.profile_key(), "policyGapProfile.key", limits, key)?;
            usage.charge_non_empty_text(
                value.parameter_version(),
                "policyGapProfile.parameterVersion",
                limits,
                key,
            )?;
            if value.minimum_lead_gap_ms().is_none()
                || value.minimum_lag_gap_ms().is_none()
                || value.clearance_buffer_ms().is_none()
            {
                return Err(semantic_error(
                    "policyGapProfile.requiredScalar",
                    RoadEditingInputViolation::InvalidCombination,
                    key,
                ));
            }
            usage.charge_relation(6);
        }
        for value in policy.stream_rules() {
            closed(value._tab, 7, key)?;
            usage.charge_token(value.rule_key(), "policyStreamRule.key", limits, key)?;
            usage.charge_reference(
                value.stream(),
                2,
                true,
                "policyStreamRule.stream",
                namespace,
                imports,
                limits,
                key,
            )?;
            classes(
                usage,
                value.participant_classes(),
                namespace,
                imports,
                limits,
                key,
            )?;
            if value.priority().is_none()
                || value.yield_to_streams().is_empty() != value.gap_profile_key().is_none()
            {
                return Err(semantic_error(
                    "policyStreamRule.requiredScalarOrGap",
                    RoadEditingInputViolation::InvalidCombination,
                    key,
                ));
            }
            for target in value.yield_to_streams() {
                usage.charge_reference(
                    target,
                    2,
                    true,
                    "policyStreamRule.yieldTo",
                    namespace,
                    imports,
                    limits,
                    key,
                )?;
            }
            if let Some(gap) = value.gap_profile_key() {
                usage.charge_token(gap, "policyStreamRule.gap", limits, key)?;
            }
            for evidence in value.evidence_keys() {
                usage.charge_token(evidence, "policyStreamRule.evidence", limits, key)?;
            }
            usage.charge_relation(
                6 + usize::from(value.participant_classes().is_some())
                    + usize::from(value.gap_profile_key().is_some())
                    + value
                        .participant_classes()
                        .map_or(0, |classes| classes.len())
                    + value.yield_to_streams().len()
                    + value.evidence_keys().len(),
            );
        }
        for value in policy.gate_rules() {
            closed(value._tab, 6, key)?;
            usage.charge_token(value.rule_key(), "policyGateRule.key", limits, key)?;
            usage.charge_reference(
                value.gate(),
                4,
                true,
                "policyGateRule.gate",
                namespace,
                imports,
                limits,
                key,
            )?;
            classes(
                usage,
                value.participant_classes(),
                namespace,
                imports,
                limits,
                key,
            )?;
            if value
                .interpretation()
                .and_then(|v| crate::GateInterpretation::from_code(v.0))
                .is_none()
                || value
                    .prohibition()
                    .and_then(|v| crate::GateProhibition::from_code(v.0))
                    .is_none()
            {
                return Err(semantic_error(
                    "policyGateRule.requiredEnum",
                    RoadEditingInputViolation::InvalidCombination,
                    key,
                ));
            }
            for evidence in value.evidence_keys() {
                usage.charge_token(evidence, "policyGateRule.evidence", limits, key)?;
            }
            usage.charge_relation(
                6 + usize::from(value.participant_classes().is_some())
                    + value
                        .participant_classes()
                        .map_or(0, |classes| classes.len())
                    + value.evidence_keys().len(),
            );
        }
    }
    Ok(())
}

fn classes(
    usage: &mut RoadEditingPreflightCounts,
    values: Option<StringVector<'_>>,
    namespace: &str,
    imports: StringVector<'_>,
    limits: &CompileLimits,
    key: &str,
) -> Result<(), DiagnosticBundle> {
    if let Some(values) = values {
        if values.is_empty() {
            return Err(semantic_error(
                "policyRule.participantClasses",
                RoadEditingInputViolation::EmptyCollection,
                key,
            ));
        }
        for value in values {
            usage.charge_reference(
                value,
                1,
                true,
                "policyRule.participantClasses",
                namespace,
                imports,
                limits,
                key,
            )?;
        }
    }
    Ok(())
}
