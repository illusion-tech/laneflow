use super::*;
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};

fn output(permuted: bool, signals: bool, overflow: bool) -> CompilationOutput {
    let mut unit = unit_editing_custom(
        signals.then_some(ManeuverDirection::Right),
        signals.then_some([SignalAspect::Green; 2]),
        |builder, policy| {
            if signals {
                for gate in &mut policy.gates {
                    gate.interpretation = GateInterpretation::CnCircularRightTurn;
                }
            }
            let mut other = policy.clone();
            other.key = "alternative".into();
            other.gaps[0].minimum_lead_gap_ms = if overflow { 9_007_199_254_740_991 } else { 100 };
            other.gaps[0].minimum_lag_gap_ms = 200;
            other.gaps[0].clearance_buffer_ms = 10;
            let a = other
                .streams
                .iter()
                .find(|r| r.key.as_ref() == "a")
                .unwrap()
                .stream
                .clone();
            for r in &mut other.streams {
                if r.key.as_ref() == "a" {
                    r.priority = 20;
                    r.yield_to = Box::new([]);
                    r.gap = None;
                } else {
                    r.priority = 10;
                    r.yield_to = vec![a.clone()].into_boxed_slice();
                    r.gap = Some("gap".into());
                }
            }
            other.gates[0].prohibition = GateProhibition::Always;
            if permuted {
                // 保持语义而交换两份策略的加入先后，以及各自局部声明顺序。
                core::mem::swap(policy, &mut other);
                policy.gates.reverse();
                policy.streams.reverse();
                other.gates.reverse();
                other.streams.reverse();
            }
            builder
                .add_declaration(re::RoadEditingDeclaration::RightOfWayPolicySet(other))
                .unwrap();
        },
    )
    .unwrap();
    unit.limits = CompileLimits::single_network_1m_v2();
    Compiler::new().compile(unit).unwrap()
}

#[test]
fn shared_policy_fixture_closes_and_is_declaration_order_invariant() {
    let mut expected = None;
    for permuted in [false, true] {
        let output = output(permuted, false, false);
        let candidate = emit_portable_candidate(
            &output,
            &PortableEmissionProvenance::try_new("w3-shared-policy").unwrap(),
            laneflow_format::FormatLimits::HARD,
            PortableDiffBase::Genesis,
        )
        .unwrap();
        let bytes = candidate.canonical_artifact().bytes();
        if let Some(expected) = &expected {
            assert_eq!(&candidate.network_revision(), expected);
        } else {
            expected = Some(candidate.network_revision());
        }
        let checked = laneflow_format::check_canonical_network_input(
            bytes,
            laneflow_format::FormatLimits::HARD,
        )
        .unwrap();
        build_shared_network_revision(
            checked,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::Omit,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .unwrap();
        if !permuted && std::env::var_os("DUMP_W3_POLICY").is_some() {
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/portable/lfca-world-policies");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("expected.lfca"), bytes).unwrap();
        } else if !permuted {
            assert!(
                bytes
                    == include_bytes!(
                        "../../../tests/fixtures/portable/lfca-world-policies/expected.lfca"
                    )
            );
        }
    }
}

#[test]
fn world_policy_boundary_fixtures_are_reproducible() {
    for (name, signals, overflow) in [("signal", true, false), ("overflow", false, true)] {
        let candidate = emit_portable_candidate(
            &output(false, signals, overflow),
            &PortableEmissionProvenance::try_new("w3-shared-policy").unwrap(),
            laneflow_format::FormatLimits::HARD,
            PortableDiffBase::Genesis,
        )
        .unwrap();
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/portable/lfca-world-policies");
        if std::env::var_os("DUMP_W3_POLICY").is_some() {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join(format!("{name}.lfca")),
                candidate.canonical_artifact().bytes(),
            )
            .unwrap();
        } else {
            let expected: &[u8] = if signals {
                include_bytes!("../../../tests/fixtures/portable/lfca-world-policies/signal.lfca")
            } else {
                include_bytes!("../../../tests/fixtures/portable/lfca-world-policies/overflow.lfca")
            };
            assert!(candidate.canonical_artifact().bytes() == expected, "{name}");
        }
    }
}
