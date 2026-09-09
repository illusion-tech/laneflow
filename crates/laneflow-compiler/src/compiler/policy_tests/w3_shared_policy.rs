use super::*;
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};

#[test]
fn same_class_physical_profiles_share_policy_rows_and_unused_classes_need_no_rules() {
    use laneflow_static_contract::{
        EntityKind, ManeuverGateOrdinal, ParticipantStreamOrdinal, RightOfWayPolicySetOrdinal,
        VehicleProfileOrdinal,
    };

    let mut retained = None;
    for profile_count in [1, 100] {
        let unit = unit_editing_custom(None, None, |builder, policy| {
            builder
                .add_declaration(re::RoadEditingDeclaration::ParticipantClass(
                    re::ParticipantClassInput::try_new("unused").unwrap(),
                ))
                .unwrap();
            for i in 1..profile_count {
                builder
                    .add_declaration(re::RoadEditingDeclaration::VehicleProfile(
                        re::VehicleProfileInput::try_new(
                            format!("variant-{i}"),
                            re::ParticipantClassReference::local("vehicle").unwrap(),
                            re::IidmVehicleProfileInput::try_new(
                                5.0 + f64::from(i) / 100.0,
                                12.,
                                2.,
                                1.4,
                                1.8,
                                2.,
                                4.5,
                            )
                            .unwrap(),
                        )
                        .unwrap(),
                    ))
                    .unwrap();
            }
            let classes = || {
                Some(
                    vec![re::ParticipantClassReference::local("vehicle").unwrap()]
                        .into_boxed_slice(),
                )
            };
            for rule in &mut policy.gates {
                rule.classes = classes();
            }
            for rule in &mut policy.streams {
                rule.classes = classes();
            }
        })
        .unwrap();
        let output = Compiler::new().compile(unit).unwrap();
        let candidate = emit_portable_candidate(
            &output,
            &PortableEmissionProvenance::try_new("class-sharing").unwrap(),
            laneflow_format::FormatLimits::HARD,
            PortableDiffBase::Genesis,
        )
        .unwrap();
        let checked = laneflow_format::check_canonical_network_input(
            candidate.canonical_artifact().bytes(),
            laneflow_format::FormatLimits::HARD,
        )
        .unwrap();
        let root = build_shared_network_revision(
            checked,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::Omit,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .unwrap();
        let profiles = root.identity().entity_count(EntityKind::VehicleProfile);
        assert_eq!(profiles, profile_count);
        let policy = root
            .policy()
            .policy(RightOfWayPolicySetOrdinal::from_raw(0))
            .unwrap();
        let mut lengths = std::collections::BTreeSet::new();
        for p in 0..profiles {
            let profile = root
                .traffic()
                .relations()
                .vehicle_profile(VehicleProfileOrdinal::from_raw(p))
                .unwrap();
            lengths.insert(profile.length_mm());
            for g in 0..root.identity().entity_count(EntityKind::ManeuverGate) {
                let gate = ManeuverGateOrdinal::from_raw(g);
                let cells = policy.gate_classes(gate);
                assert_eq!(cells.len(), 1);
                assert_eq!(cells[0].class(), profile.class());
                assert!(std::ptr::eq(
                    &cells[0],
                    policy.gate(gate, profile.class()).unwrap()
                ));
            }
            for s in 0..root.identity().entity_count(EntityKind::ParticipantStream) {
                let stream = ParticipantStreamOrdinal::from_raw(s);
                let cells = policy.stream_classes(stream);
                assert_eq!(cells.len(), 1);
                assert_eq!(cells[0].class(), profile.class());
                assert!(std::ptr::eq(
                    &cells[0],
                    policy.stream(stream, profile.class()).unwrap()
                ));
            }
        }
        assert_eq!(lengths.len(), profile_count as usize);
        let bytes = root.policy().retained_logical_bytes();
        assert_eq!(*retained.get_or_insert(bytes), bytes);
    }
}

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
        let root = build_shared_network_revision(
            checked,
            SharedNetworkBuildOptions::new(
                SpatialBuildOption::Omit,
                SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
            ),
        )
        .unwrap();
        // 正式编译的多策略范围必须各自返回原行与归因，不能按全局 owner 下标串表。
        use laneflow_static_contract::{
            EntityKind, ManeuverGateOrdinal, ParticipantClassOrdinal, ParticipantStreamOrdinal,
            RightOfWayPolicySetOrdinal,
        };
        assert_eq!(
            root.identity()
                .entity_count(EntityKind::RightOfWayPolicySet),
            2
        );
        let missing = ParticipantClassOrdinal::from_raw(u32::MAX);
        for p in 0..2 {
            let policy = root
                .policy()
                .policy(RightOfWayPolicySetOrdinal::from_raw(p))
                .unwrap();
            for g in 0..root.identity().entity_count(EntityKind::ManeuverGate) {
                let gate = ManeuverGateOrdinal::from_raw(g);
                for cell in policy.gate_classes(gate) {
                    assert!(core::ptr::eq(
                        policy.gate(gate, cell.class()).unwrap(),
                        cell
                    ));
                    assert_eq!(
                        policy.gate_attribution(gate, cell.class()).unwrap().policy,
                        policy.id()
                    );
                }
                assert!(policy.gate(gate, missing).is_none());
            }
            for s in 0..root.identity().entity_count(EntityKind::ParticipantStream) {
                let stream = ParticipantStreamOrdinal::from_raw(s);
                for cell in policy.stream_classes(stream) {
                    assert!(core::ptr::eq(
                        policy.stream(stream, cell.class()).unwrap(),
                        cell
                    ));
                    assert_eq!(
                        policy
                            .stream_attribution(stream, cell.class())
                            .unwrap()
                            .policy,
                        policy.id()
                    );
                }
                assert!(policy.stream(stream, missing).is_none());
            }
        }
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
