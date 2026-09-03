use super::*;
pub(super) fn encode<'a>(
    fbb: &mut runtime::FlatBufferBuilder<'a>,
    value: &RightOfWayPolicySetInput,
    namespace: &str,
) -> runtime::WIPOffset<wire::RightOfWayPolicySet<'a>> {
    let key = fbb.create_string(&value.key);
    let regulation = encode_access_regulation(fbb, &value.regulation);
    let mut evidence = Vec::with_capacity(value.evidence.len());
    for v in &value.evidence {
        let key = fbb.create_string(&v.key);
        let locator = fbb.create_string(&v.locator);
        let description = v.description.as_ref().map(|v| fbb.create_string(v));
        evidence.push(wire::PolicyEvidence::create(
            fbb,
            &wire::PolicyEvidenceArgs {
                evidence_key: Some(key),
                locator: Some(locator),
                description,
            },
        ));
    }
    let evidence = fbb.create_vector(&evidence);
    let mut gaps = Vec::with_capacity(value.gaps.len());
    for v in &value.gaps {
        let key = fbb.create_string(&v.key);
        let parameter_version = fbb.create_string(&v.parameter_version);
        gaps.push(wire::PolicyGapProfile::create(
            fbb,
            &wire::PolicyGapProfileArgs {
                profile_key: Some(key),
                parameter_version: Some(parameter_version),
                minimum_lead_gap_ms: Some(v.minimum_lead_gap_ms),
                minimum_lag_gap_ms: Some(v.minimum_lag_gap_ms),
                clearance_buffer_ms: Some(v.clearance_buffer_ms),
            },
        ));
    }
    let gap_profiles = fbb.create_vector(&gaps);
    let mut streams = Vec::with_capacity(value.streams.len());
    for v in &value.streams {
        let key = fbb.create_string(&v.key);
        let stream = create_reference(fbb, &v.stream);
        let classes = v.classes.as_ref().map(|v| references(fbb, v, namespace));
        let yield_to = references(fbb, &v.yield_to, namespace);
        let gap = v.gap.as_ref().map(|v| fbb.create_string(v));
        let evidence_keys = strings(fbb, &v.evidence);
        streams.push(wire::PolicyStreamRule::create(
            fbb,
            &wire::PolicyStreamRuleArgs {
                rule_key: Some(key),
                stream: Some(stream),
                participant_classes: classes,
                priority: Some(v.priority),
                yield_to_streams: Some(yield_to),
                gap_profile_key: gap,
                evidence_keys: Some(evidence_keys),
            },
        ));
    }
    let stream_rules = fbb.create_vector(&streams);
    let mut gates = Vec::with_capacity(value.gates.len());
    for v in &value.gates {
        let key = fbb.create_string(&v.key);
        let gate = create_reference(fbb, &v.gate);
        let classes = v.classes.as_ref().map(|v| references(fbb, v, namespace));
        let evidence_keys = strings(fbb, &v.evidence);
        gates.push(wire::PolicyGateRule::create(
            fbb,
            &wire::PolicyGateRuleArgs {
                rule_key: Some(key),
                gate: Some(gate),
                participant_classes: classes,
                interpretation: Some(wire::GateInterpretation(v.interpretation.code())),
                prohibition: Some(wire::GateProhibition(v.prohibition.code())),
                evidence_keys: Some(evidence_keys),
            },
        ));
    }
    let gate_rules = fbb.create_vector(&gates);
    let canvas = create_canvas(fbb, value.canvas_selection());
    wire::RightOfWayPolicySet::create(
        fbb,
        &wire::RightOfWayPolicySetArgs {
            policy_set_key: Some(key),
            regulation: Some(regulation),
            evidence: Some(evidence),
            gap_profiles: Some(gap_profiles),
            stream_rules: Some(stream_rules),
            gate_rules: Some(gate_rules),
            canvas_selection: canvas,
        },
    )
}
fn strings<'a>(
    fbb: &mut runtime::FlatBufferBuilder<'a>,
    values: &[Box<str>],
) -> runtime::WIPOffset<runtime::Vector<'a, runtime::ForwardsUOffset<&'a str>>> {
    let offsets: Vec<_> = values.iter().map(|v| fbb.create_string(v)).collect();
    fbb.create_vector(&offsets)
}
fn references<'a, K: laneflow_static_contract::EntityKindMarker>(
    fbb: &mut runtime::FlatBufferBuilder<'a>,
    values: &[RoadEditingReference<K>],
    namespace: &str,
) -> runtime::WIPOffset<runtime::Vector<'a, runtime::ForwardsUOffset<&'a str>>> {
    let mut sorted: Vec<_> = values.iter().collect();
    sorted.sort_unstable_by(|a, b| a.canonical_target_cmp(b, namespace));
    let offsets: Vec<_> = sorted
        .into_iter()
        .map(|v| create_reference(fbb, v))
        .collect();
    fbb.create_vector(&offsets)
}
