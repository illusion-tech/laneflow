//! 测试夹具的宿主选择清单；生产 Runtime 不拥有任何默认策略。
use laneflow_runtime::{PolicyPin, WorldPolicySelection};
use laneflow_static_contract::{EntityKind, RightOfWayPolicySetId};
use laneflow_static_network::SharedNetworkRevision;

pub fn selection(revision: &SharedNetworkRevision) -> WorldPolicySelection {
    for (namespace, key) in [
        ("runtime-fixture-policy", "fixture-policy"),
        ("city/waiting-scale", "waiting-policy"),
        ("city/runtime-coverage", "signal-policy"),
        ("city/runtime-conflict", "conflict-policy"),
        ("laneflow/signalized-corridor", "protected-entry"),
    ] {
        let policy = RightOfWayPolicySetId::from_untyped(
            laneflow_compiler::derive_canonical_stable_id_v1(
                EntityKind::RightOfWayPolicySet,
                namespace,
                key,
                &laneflow_compiler::CompileLimits::p100_initial_v1(),
            )
            .unwrap(),
        );
        if revision.identity().ordinal(policy).is_some() {
            return WorldPolicySelection::Pinned(PolicyPin { policy });
        }
    }
    assert!(
        [
            EntityKind::ManeuverGate,
            EntityKind::ConflictZone,
            EntityKind::ParticipantStream
        ]
        .iter()
        .all(|kind| revision.traffic().entity_counts().count(*kind) == 0),
        "gate-bearing fixture must declare and explicitly list its policy"
    );
    WorldPolicySelection::NotRequired
}

/// 由测试夹具显式列出 Gate 与解释，不扫描源模型补默认语义。
#[allow(dead_code)]
pub fn add_gate_policy(
    builder: &mut laneflow_compiler::SyntheticModuleBuilder,
    key: &str,
    gates: &[(&str, laneflow_compiler::GateInterpretation)],
) {
    use laneflow_compiler::*;
    let span = builder.policy_source_span();
    let source = PolicyInputSource {
        primary: &span,
        contributing: &[],
    };
    let rules: Vec<_> = gates
        .iter()
        .map(|(key, interpretation)| PolicyGateRuleInput {
            rule_key: key,
            gate: OwnerQualifiedReference {
                target: ManeuverGateReference::local(key),
                owner_keys: &[],
            },
            participant_classes: None,
            interpretation: *interpretation,
            prohibition: GateProhibition::None,
            evidence_keys: &[],
            source,
        })
        .collect();
    builder
        .add_right_of_way_policy_set(RightOfWayPolicySetInput {
            policy_set_key: key,
            regulation: RegulationIdentity {
                jurisdiction: "engineering",
                version: "fixture-1",
                source: Some("repository:runtime-fixture-1"),
            },
            evidence: &[],
            gap_profiles: &[],
            stream_rules: &[],
            gate_rules: &rules,
            source,
        })
        .unwrap();
}
