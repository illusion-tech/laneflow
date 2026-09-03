//! W2 正式输入到受检三件套的集成证据。
mod diagnostics;

use super::*;
use crate::road_editing as re;
use crate::*;
use laneflow_static_contract::{EntityKindMarker, ParticipantStreamKind};

const TOPOLOGY: &str = "city/portable-full-spatial-conflict";
const DOCUMENT: &str = "policy.document";
mod w3_shared_policy;
const JUNCTION: &str = "conflict-junction";
static VEHICLE_CLASSES: std::sync::LazyLock<[ParticipantClassReference<'static>; 1]> =
    std::sync::LazyLock::new(|| [ParticipantClassReference::local("vehicle")]);

fn header(limits: &CompileLimits) -> SourceModuleHeader {
    SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "policy",
            source_document_key: DOCUMENT,
            generator_build_id: "fixture:w2",
            parameters_and_inputs_digest: [1; 32],
            frontend_options_digest: [2; 32],
            random_seed: None,
            provenance: "repository:w2-engineering-fixture",
        },
        limits,
    )
    .unwrap()
}
fn profile() -> IidmVehicleProfileInput {
    IidmVehicleProfileInput {
        length_meters: 4.5,
        desired_speed_meters_per_second: 12.,
        min_gap_meters: 2.,
        time_headway_seconds: 1.4,
        max_acceleration_meters_per_second_squared: 1.8,
        comfortable_deceleration_meters_per_second_squared: 2.,
        emergency_deceleration_meters_per_second_squared: 4.5,
    }
}
fn reference<'a, K: EntityKindMarker>(
    key: &'a str,
    owners: &'a [&'a str],
) -> OwnerQualifiedReference<'a, K> {
    OwnerQualifiedReference {
        target: EntityReference::imported(TOPOLOGY, key),
        owner_keys: owners,
    }
}
fn synthetic_policy(
    limits: &CompileLimits,
    mutate: impl FnOnce(&mut Vec<PolicyStreamRuleInput<'_>>, &mut Vec<PolicyGateRuleInput<'_>>),
) -> Result<SyntheticModule, DiagnosticBundle> {
    let mut builder = SyntheticModuleBuilder::new(header(limits), limits)?;
    builder.add_import(TOPOLOGY)?;
    builder.add_participant_class(ParticipantClassInput {
        participant_class_key: "vehicle",
        extends: None,
    })?;
    builder.add_vehicle_profile(VehicleProfileInput {
        vehicle_profile_key: "car",
        participant_class: ParticipantClassReference::local("vehicle"),
        iidm: profile(),
    })?;
    let span = SourceSpan::point(DOCUMENT.into(), 10, 3);
    let contributions = [span.clone()];
    let source = PolicyInputSource {
        primary: &span,
        contributing: &contributions,
    };
    let yield_to = [reference::<ParticipantStreamKind>(
        "conflict-stream-b",
        &[JUNCTION],
    )];
    let mut streams = vec![
        PolicyStreamRuleInput {
            rule_key: "a",
            stream: reference("conflict-stream-a", &[JUNCTION]),
            participant_classes: None,
            priority: 1,
            yield_to_streams: &yield_to,
            gap_profile_key: Some("gap"),
            evidence_keys: &["basis"],
            source,
        },
        PolicyStreamRuleInput {
            rule_key: "b",
            stream: reference("conflict-stream-b", &[JUNCTION]),
            participant_classes: None,
            priority: 2,
            yield_to_streams: &[],
            gap_profile_key: None,
            evidence_keys: &["basis"],
            source,
        },
    ];
    let mut gates = vec![
        PolicyGateRuleInput {
            rule_key: "a",
            gate: reference("admission", &[JUNCTION, "conflict-movement-a", "path"]),
            participant_classes: None,
            interpretation: GateInterpretation::Uncontrolled,
            prohibition: GateProhibition::None,
            evidence_keys: &["basis"],
            source,
        },
        PolicyGateRuleInput {
            rule_key: "b",
            gate: reference("admission", &[JUNCTION, "conflict-movement-b", "path"]),
            participant_classes: None,
            interpretation: GateInterpretation::Uncontrolled,
            prohibition: GateProhibition::None,
            evidence_keys: &["basis"],
            source,
        },
    ];
    mutate(&mut streams, &mut gates);
    builder.add_right_of_way_policy_set(RightOfWayPolicySetInput {
        policy_set_key: "rules",
        regulation: RegulationIdentity {
            jurisdiction: "engineering",
            version: "fixture-1",
            source: None,
        },
        evidence: &[PolicyEvidenceInput {
            evidence_key: "basis",
            locator: "repository:w2-fixture",
            description: None,
            source,
        }],
        gap_profiles: &[PolicyGapProfileInput {
            profile_key: "gap",
            parameter_version: "gap-1",
            minimum_lead_gap_ms: 0,
            minimum_lag_gap_ms: 0,
            clearance_buffer_ms: 0,
            source,
        }],
        stream_rules: &streams,
        gate_rules: &gates,
        source,
    })?;
    builder.finish()
}
fn editing_policy(limits: &CompileLimits) -> re::OwnedRoadEditingSourceBuffer {
    editing_policy_custom(limits, |_, _| {})
}
pub(crate) fn editing_policy_custom(
    limits: &CompileLimits,
    customize: impl FnOnce(
        &mut re::RoadEditingSourceModuleBuilder<'_>,
        &mut re::RightOfWayPolicySetInput,
    ),
) -> re::OwnedRoadEditingSourceBuffer {
    let header = re::RoadEditingModuleHeader::try_new(
        "policy",
        DOCUMENT,
        vec![TOPOLOGY.into()],
        re::RoadEditingProvenance::direct("repository:w2-engineering-fixture").unwrap(),
    )
    .unwrap();
    let mut builder = re::RoadEditingSourceModuleBuilder::new(
        header,
        GeometryAccuracyProfile::Balanced5Cm,
        GeometryDirectionProfile::Balanced2Deg,
        limits,
    )
    .unwrap();
    builder
        .add_declaration(re::RoadEditingDeclaration::ParticipantClass(
            re::ParticipantClassInput::try_new("vehicle").unwrap(),
        ))
        .unwrap();
    builder
        .add_declaration(re::RoadEditingDeclaration::VehicleProfile(
            re::VehicleProfileInput::try_new(
                "car",
                re::ParticipantClassReference::local("vehicle").unwrap(),
                re::IidmVehicleProfileInput::try_new(4.5, 12., 2., 1.4, 1.8, 2., 4.5).unwrap(),
            )
            .unwrap(),
        ))
        .unwrap();
    let stream = |suffix: &str| {
        re::ParticipantStreamReference::imported(
            TOPOLOGY,
            vec![JUNCTION.into()],
            format!("conflict-stream-{suffix}"),
        )
        .unwrap()
    };
    let gate = |suffix: &str| {
        re::ManeuverGateReference::imported(
            TOPOLOGY,
            vec![
                JUNCTION.into(),
                format!("conflict-movement-{suffix}"),
                "path".into(),
            ],
            "admission",
        )
        .unwrap()
    };
    let mut policy = re::RightOfWayPolicySetInput::try_new(
        "rules",
        RegulationIdentity::try_new("engineering", "fixture-1").unwrap(),
        vec![re::PolicyEvidenceInput::try_new("basis", "repository:w2-fixture", None).unwrap()],
        vec![re::PolicyGapProfileInput::try_new("gap", "gap-1", 0, 0, 0).unwrap()],
        vec![
            re::PolicyStreamRuleInput::try_new(
                "b",
                stream("b"),
                None,
                2,
                vec![],
                None,
                vec!["basis".into()],
            )
            .unwrap(),
            re::PolicyStreamRuleInput::try_new(
                "a",
                stream("a"),
                None,
                1,
                vec![stream("b")],
                Some("gap".into()),
                vec!["basis".into()],
            )
            .unwrap(),
        ],
        vec![
            re::PolicyGateRuleInput::try_new(
                "b",
                gate("b"),
                None,
                GateInterpretation::Uncontrolled,
                GateProhibition::None,
                vec!["basis".into()],
            )
            .unwrap(),
            re::PolicyGateRuleInput::try_new(
                "a",
                gate("a"),
                None,
                GateInterpretation::Uncontrolled,
                GateProhibition::None,
                vec!["basis".into()],
            )
            .unwrap(),
        ],
    )
    .unwrap();
    customize(&mut builder, &mut policy);
    builder
        .add_declaration(re::RoadEditingDeclaration::RightOfWayPolicySet(policy))
        .unwrap();
    re::RoadEditingSourceWriter::new(limits)
        .write(builder.finish().unwrap())
        .unwrap()
}
pub(crate) fn unit_with_policy(
    editing: bool,
    mutate: impl FnOnce(&mut Vec<PolicyStreamRuleInput<'_>>, &mut Vec<PolicyGateRuleInput<'_>>),
) -> Result<CompilationUnit, DiagnosticBundle> {
    unit_with_control(editing, None, None, mutate)
}
pub(crate) fn unit_with_control(
    editing: bool,
    direction: Option<ManeuverDirection>,
    aspects: Option<[SignalAspect; 2]>,
    mutate: impl FnOnce(&mut Vec<PolicyStreamRuleInput<'_>>, &mut Vec<PolicyGateRuleInput<'_>>),
) -> Result<CompilationUnit, DiagnosticBundle> {
    let limits = CompileLimits::p100_initial_v1();
    let topology = re::RoadEditingSourceWriter::new(&limits)
        .write(
            super::portable_fixture_tests::policy_topology_variant(&limits, direction, aspects)
                .finish()
                .unwrap(),
        )
        .unwrap();
    let mut unit = CompilationUnitBuilder::new(limits.clone());
    unit.add_road_editing_module(
        re::RoadEditingModuleInput::try_new(
            "portable-full-spatial-conflict.document",
            topology.as_bytes(),
            None,
        )
        .unwrap(),
    )?;
    if editing {
        let policy = editing_policy(&limits);
        unit.add_road_editing_module(
            re::RoadEditingModuleInput::try_new(DOCUMENT, policy.as_bytes(), None).unwrap(),
        )?;
    } else {
        unit.add_synthetic_module(synthetic_policy(&limits, mutate)?)?;
    }
    unit.build()
}
fn compile(
    mutate: impl FnOnce(&mut Vec<PolicyStreamRuleInput<'_>>, &mut Vec<PolicyGateRuleInput<'_>>),
) -> Result<CompilationOutput, DiagnosticBundle> {
    Compiler::new().compile(unit_with_policy(false, mutate)?)
}
fn violation(result: Result<CompilationOutput, DiagnosticBundle>, expected: PolicyViolation) {
    let errors = match result {
        Ok(_) => panic!("expected {expected:?}"),
        Err(e) => e,
    };
    assert!(errors.diagnostics().iter().any(|d|matches!(d.payload(),DiagnosticPayload::InvalidPolicy {violation,..} if *violation==expected)),"{errors:?}");
}

#[test]
fn official_frontends_emit_equivalent_policy_semantics_and_checked_sources() {
    let synthetic = compile(|_, _| {}).unwrap();
    let editing = Compiler::new()
        .compile(unit_with_policy(true, |_, _| {}).unwrap())
        .unwrap();
    assert_eq!(
        synthetic.lir().unit().semantic_digest,
        editing.lir().unit().semantic_digest
    );
    assert_eq!(synthetic.lir().unit().policies.len(), 1);
    let mut network_revision = None;
    for output in [&synthetic, &editing] {
        let candidate = emit_portable_candidate(
            output,
            &PortableEmissionProvenance::try_new("w2-policy-fixture").unwrap(),
            laneflow_format::FormatLimits::HARD,
            PortableDiffBase::Genesis,
        )
        .unwrap();
        if let Some(expected) = network_revision {
            assert_eq!(candidate.network_revision(), expected);
        } else {
            network_revision = Some(candidate.network_revision());
        }
        laneflow_format::check_post_emission_bundle(
            candidate.canonical_artifact().bytes(),
            candidate.source_map().bytes(),
            candidate.semantic_diff().bytes(),
            candidate.expected_semantic_diff_base(),
            laneflow_format::FormatLimits::HARD,
        )
        .unwrap();
    }
    let permuted = compile(|streams, gates| {
        streams.reverse();
        gates.reverse();
    })
    .unwrap();
    assert_eq!(
        synthetic.lir().unit().semantic_digest,
        permuted.lir().unit().semantic_digest
    );
}
#[test]
fn actual_enterable_rows_require_unique_local_rules_and_strict_target_priority() {
    violation(
        compile(|streams, _| {
            streams.pop();
        }),
        PolicyViolation::MissingRule,
    );
    violation(
        compile(|_, gates| {
            gates.pop();
        }),
        PolicyViolation::MissingRule,
    );
    violation(
        compile(|streams, _| {
            streams[1].priority = 1;
        }),
        PolicyViolation::YieldPriority,
    );
    violation(
        compile(|streams, _| {
            streams[0].stream = streams[1].stream;
        }),
        PolicyViolation::SelfYield,
    );
    violation(
        compile(|streams, _| {
            let mut duplicate = streams[1];
            duplicate.rule_key = "b-copy";
            streams.push(duplicate);
        }),
        PolicyViolation::AmbiguousRule,
    );
    violation(
        compile(|_, gates| {
            gates[0].prohibition = GateProhibition::OnRed;
        }),
        PolicyViolation::SignalBinding,
    );
}
#[test]
fn evidence_and_owner_address_are_validated_at_official_admission_or_binding() {
    violation(
        compile(|streams, _| {
            streams[0].evidence_keys = &[];
        }),
        PolicyViolation::MissingEvidence,
    );
    violation(
        compile(|streams, _| {
            streams[0].evidence_keys = &["absent"];
        }),
        PolicyViolation::MissingLocalReference,
    );
    assert!(
        compile(|_, gates| {
            gates[0].gate.owner_keys = &[];
        })
        .is_err()
    );
}

fn controlled(
    direction: Option<ManeuverDirection>,
    aspects: [SignalAspect; 2],
    mutate: impl FnOnce(&mut Vec<PolicyStreamRuleInput<'_>>, &mut Vec<PolicyGateRuleInput<'_>>),
) -> Result<CompilationOutput, DiagnosticBundle> {
    Compiler::new().compile(unit_with_control(false, direction, Some(aspects), mutate)?)
}

#[test]
fn right_turn_requires_explicit_direction_and_physical_lamp_is_not_shadowable() {
    let circular = |_: &mut Vec<PolicyStreamRuleInput<'_>>,
                    gates: &mut Vec<PolicyGateRuleInput<'_>>| {
        for gate in gates {
            gate.interpretation = GateInterpretation::CnCircularRightTurn;
        }
    };
    violation(
        controlled(None, [SignalAspect::Red; 2], circular),
        PolicyViolation::RightTurnRequired,
    );
    violation(
        controlled(
            Some(ManeuverDirection::Straight),
            [SignalAspect::Red; 2],
            circular,
        ),
        PolicyViolation::RightTurnRequired,
    );
    let valid = controlled(
        Some(ManeuverDirection::Right),
        [SignalAspect::Red; 2],
        circular,
    )
    .unwrap();
    emit_portable_candidate(
        &valid,
        &PortableEmissionProvenance::try_new("w2-direction").unwrap(),
        laneflow_format::FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .unwrap();
    violation(
        controlled(
            Some(ManeuverDirection::Right),
            [SignalAspect::Red; 2],
            |_, gates| {
                for gate in gates.iter_mut() {
                    gate.interpretation = GateInterpretation::CnCircularRightTurn;
                }
                let mut shadowed = gates[0];
                shadowed.rule_key = "directional-shadow";
                shadowed.interpretation = GateInterpretation::DirectionalRightProtected;
                gates[0].participant_classes = Some(&*VEHICLE_CLASSES);
                gates.push(shadowed);
            },
        ),
        PolicyViolation::LampTypeConflict,
    );
}

#[test]
fn protected_coherence_uses_steady_phase_and_regulatory_deny() {
    let protected = |_: &mut Vec<PolicyStreamRuleInput<'_>>,
                     gates: &mut Vec<PolicyGateRuleInput<'_>>| {
        for gate in gates {
            gate.interpretation = GateInterpretation::ProtectedGroup;
        }
    };
    violation(
        controlled(None, [SignalAspect::Green; 2], protected),
        PolicyViolation::ProtectedConflict,
    );
    controlled(None, [SignalAspect::Green, SignalAspect::Red], protected).unwrap();
    controlled(None, [SignalAspect::Green; 2], |_, gates| {
        for gate in gates.iter_mut() {
            gate.interpretation = GateInterpretation::ProtectedGroup;
        }
        gates[1].prohibition = GateProhibition::Always;
    })
    .unwrap();
    // Protected 也必须完成 yield priority 校验，不能用绿灯掩盖非法策略。
    violation(
        controlled(
            None,
            [SignalAspect::Green, SignalAspect::Red],
            |streams, gates| {
                for gate in gates {
                    gate.interpretation = GateInterpretation::ProtectedGroup;
                }
                streams[1].priority = 0;
            },
        ),
        PolicyViolation::YieldPriority,
    );
}

fn raw_slot(bytes: &[u8], table: usize, field: usize) -> (usize, usize) {
    let offset = i32::from_le_bytes(bytes[table..table + 4].try_into().unwrap());
    let vtable = usize::try_from(table as i64 - i64::from(offset)).unwrap();
    let slot = vtable + 4 + field * 2;
    let offset = u16::from_le_bytes(bytes[slot..slot + 2].try_into().unwrap()) as usize;
    assert_ne!(offset, 0);
    (slot, table + offset)
}

#[test]
fn malformed_lfre_policy_is_rejected_and_admission_can_retry() {
    use laneflow_road_editing_wire::generated::lane_flow::road_editing::v1 as wire;
    let limits = CompileLimits::p100_initial_v1();
    let valid = editing_policy(&limits);
    let root = wire::size_prefixed_root_as_road_editing_source(valid.as_bytes()).unwrap();
    let policy = root.right_of_way_policy_sets().get(0);
    let gap = policy.gap_profiles().get(0)._tab.loc();
    let stream = policy.stream_rules().get(0)._tab.loc();
    let gate = policy.gate_rules().get(0)._tab.loc();
    let root_at = root._tab.loc();
    let mut cases = Vec::new();
    for (table, field) in [
        (gap, 2),
        (gap, 3),
        (gap, 4),
        (stream, 3),
        (gate, 3),
        (gate, 4),
        (root_at, 29),
    ] {
        let mut bytes = valid.as_bytes().to_vec();
        let (slot, _) = raw_slot(&bytes, table, field);
        bytes[slot..slot + 2].copy_from_slice(&0_u16.to_le_bytes());
        cases.push(bytes);
    }
    for field in [3, 4] {
        let mut bytes = valid.as_bytes().to_vec();
        let (_, at) = raw_slot(&bytes, gate, field);
        bytes[at] = 255;
        cases.push(bytes);
    }
    // 原始重复 key；writer 排序和构造器检查不能代替 reader 的共同准入检查。
    let mut duplicate = valid.as_bytes().to_vec();
    let key = policy.stream_rules().get(1).rule_key();
    let at = key.as_ptr() as usize - valid.as_bytes().as_ptr() as usize;
    duplicate[at] = b'a';
    cases.push(duplicate);
    // 合法 framing/vtable，附带当前合同未登记的字段。
    let mut unknown = valid.as_bytes().to_vec();
    let (first_slot, _) = raw_slot(&unknown, gate, 0);
    let vtable = first_slot - 4;
    let old_len = u16::from_le_bytes(unknown[vtable..vtable + 2].try_into().unwrap()) as usize;
    let mut extended = unknown[vtable..vtable + old_len].to_vec();
    extended[..2].copy_from_slice(&((old_len + 2) as u16).to_le_bytes());
    extended.extend_from_slice(&unknown[first_slot..first_slot + 2]);
    let new_at = unknown.len();
    unknown.extend_from_slice(&extended);
    unknown[gate..gate + 4]
        .copy_from_slice(&(i32::try_from(gate as i64 - new_at as i64).unwrap()).to_le_bytes());
    let length = (unknown.len() - 4) as u32;
    unknown[..4].copy_from_slice(&length.to_le_bytes());
    cases.push(unknown);
    let mut old = valid.as_bytes().to_vec();
    let (slot, _) = raw_slot(&old, root_at, 29);
    old[slot..slot + 2].copy_from_slice(&0_u16.to_le_bytes());
    let (_, at) = raw_slot(&old, root_at, 0);
    old[at..at + 4].copy_from_slice(&3_u32.to_le_bytes());
    let mut admission = CompilationUnitBuilder::new(limits.clone());
    let error = admission
        .add_road_editing_module(re::RoadEditingModuleInput::try_new(DOCUMENT, &old, None).unwrap())
        .err()
        .unwrap();
    assert!(matches!(
        error.diagnostics()[0].payload(),
        DiagnosticPayload::InvalidRoadEditingSource {
            violation: RoadEditingSourceViolation::UnsupportedFormatVersion {
                expected: 4,
                actual: 3
            },
            ..
        }
    ));
    for (index, bytes) in cases.iter().enumerate() {
        assert!(
            admission
                .add_road_editing_module(
                    re::RoadEditingModuleInput::try_new(DOCUMENT, bytes, None).unwrap()
                )
                .is_err(),
            "invalid case {index}"
        );
    }
    admission
        .add_road_editing_module(
            re::RoadEditingModuleInput::try_new(DOCUMENT, valid.as_bytes(), None).unwrap(),
        )
        .unwrap();
}

#[test]
fn nearer_class_precedes_priority_and_access_denied_stream_needs_no_rule() {
    compile(|streams, _| {
        streams[1].priority = -100;
        let mut specific = streams[1];
        specific.rule_key = "specific-b";
        specific.participant_classes = Some(&*VEHICLE_CLASSES);
        specific.priority = 2;
        streams.push(specific);
    })
    .unwrap();
    let limits = CompileLimits::p100_initial_v1();
    let policy = editing_policy_custom(&limits, |builder, policy| {
        policy.streams = policy
            .streams
            .iter()
            .filter(|r| r.key.as_ref() != "b")
            .cloned()
            .collect();
        policy.gates = policy
            .gates
            .iter()
            .filter(|r| r.key.as_ref() != "b")
            .cloned()
            .collect();
        let target = re::ManeuverPathReference::imported(
            TOPOLOGY,
            vec![JUNCTION.into(), "conflict-movement-b".into()],
            "path",
        )
        .unwrap();
        builder
            .add_declaration(re::RoadEditingDeclaration::AccessRule(
                re::AccessRuleInput::try_new(
                    "deny-b",
                    re::RoadEditingAccessTarget::ManeuverPath(target),
                    AccessEffect::Deny,
                    vec![re::ParticipantClassReference::local("vehicle").unwrap()],
                    0,
                )
                .unwrap(),
            ))
            .unwrap();
    });
    let topology = re::RoadEditingSourceWriter::new(&limits)
        .write(
            super::portable_fixture_tests::policy_topology_builder(&limits)
                .finish()
                .unwrap(),
        )
        .unwrap();
    let mut unit = CompilationUnitBuilder::new(limits);
    for (document, bytes) in [
        (DOCUMENT, policy.as_bytes()),
        (
            "portable-full-spatial-conflict.document",
            topology.as_bytes(),
        ),
    ] {
        unit.add_road_editing_module(
            re::RoadEditingModuleInput::try_new(document, bytes, None).unwrap(),
        )
        .unwrap();
    }
    Compiler::new().compile(unit.build().unwrap()).unwrap();
}

fn compile_editing_custom(
    direction: Option<ManeuverDirection>,
    aspects: Option<[SignalAspect; 2]>,
    customize: impl FnOnce(
        &mut re::RoadEditingSourceModuleBuilder<'_>,
        &mut re::RightOfWayPolicySetInput,
    ),
) -> Result<CompilationOutput, DiagnosticBundle> {
    Compiler::new().compile(unit_editing_custom(direction, aspects, customize)?)
}

fn unit_editing_custom(
    direction: Option<ManeuverDirection>,
    aspects: Option<[SignalAspect; 2]>,
    customize: impl FnOnce(
        &mut re::RoadEditingSourceModuleBuilder<'_>,
        &mut re::RightOfWayPolicySetInput,
    ),
) -> Result<CompilationUnit, DiagnosticBundle> {
    let limits = CompileLimits::p100_initial_v1();
    let policy = editing_policy_custom(&limits, customize);
    let topology = re::RoadEditingSourceWriter::new(&limits)
        .write(
            super::portable_fixture_tests::policy_topology_variant(&limits, direction, aspects)
                .finish()
                .unwrap(),
        )
        .unwrap();
    let mut unit = CompilationUnitBuilder::new(limits);
    for (document, bytes) in [
        (DOCUMENT, policy.as_bytes()),
        (
            "portable-full-spatial-conflict.document",
            topology.as_bytes(),
        ),
    ] {
        unit.add_road_editing_module(
            re::RoadEditingModuleInput::try_new(document, bytes, None).unwrap(),
        )?;
    }
    unit.build()
}

#[test]
fn denied_policy_candidates_exhaust_work_budget_and_can_retry() {
    let mut unit = unit_editing_custom(None, None, |builder, _| {
        for index in 0..80 {
            builder
                .add_declaration(re::RoadEditingDeclaration::VehicleProfile(
                    re::VehicleProfileInput::try_new(
                        format!("car-{index}"),
                        re::ParticipantClassReference::local("vehicle").unwrap(),
                        re::IidmVehicleProfileInput::try_new(4.5, 12., 2., 1.4, 1.8, 2., 4.5)
                            .unwrap(),
                    )
                    .unwrap(),
                ))
                .unwrap();
        }
        for suffix in ["a", "b"] {
            builder
                .add_declaration(re::RoadEditingDeclaration::AccessRule(
                    re::AccessRuleInput::try_new(
                        format!("deny-{suffix}"),
                        re::RoadEditingAccessTarget::ManeuverPath(
                            re::ManeuverPathReference::imported(
                                TOPOLOGY,
                                vec![JUNCTION.into(), format!("conflict-movement-{suffix}")],
                                "path",
                            )
                            .unwrap(),
                        ),
                        AccessEffect::Deny,
                        vec![re::ParticipantClassReference::local("vehicle").unwrap()],
                        0,
                    )
                    .unwrap(),
                ))
                .unwrap();
        }
    })
    .unwrap();
    let hir = crate::hir::build_hir(&unit).unwrap();
    let mut mir = crate::mir::lower_to_mir(&unit, &hir).unwrap();
    let original = unit.limits.clone();
    unit.limits = original
        .clone()
        .with_test_admission_limit(CompileLimitDimension::RelationOccurrenceCount, 64);
    let error = crate::mir::validate_policies(&unit, &mut mir).unwrap_err();
    assert!(matches!(
        error.diagnostics()[0].payload(),
        DiagnosticPayload::CompileLimitExceeded {
            dimension: CompileLimitDimension::RelationOccurrenceCount,
            ..
        }
    ));
    unit.limits = original;
    crate::mir::validate_policies(&unit, &mut mir).unwrap();
}

#[test]
fn duplicate_policy_key_fails_during_raw_admission_and_can_retry() {
    use laneflow_road_editing_wire::generated::lane_flow::road_editing::v1 as wire;
    let limits = CompileLimits::p100_initial_v1();
    let source = editing_policy_custom(&limits, |builder, policy| {
        let mut other = policy.clone();
        other.key = "other".into();
        builder
            .add_declaration(re::RoadEditingDeclaration::RightOfWayPolicySet(other))
            .unwrap();
    });
    let root = wire::size_prefixed_root_as_road_editing_source(source.as_bytes()).unwrap();
    let a = root.right_of_way_policy_sets().get(0).policy_set_key();
    let b = root.right_of_way_policy_sets().get(1).policy_set_key();
    assert_eq!(a.len(), b.len());
    let at = b.as_ptr() as usize - source.as_bytes().as_ptr() as usize;
    let mut bytes = source.as_bytes().to_vec();
    bytes[at..at + a.len()].copy_from_slice(a.as_bytes());
    let mut builder = CompilationUnitBuilder::new(limits);
    let error = builder
        .add_road_editing_module(
            re::RoadEditingModuleInput::try_new(DOCUMENT, &bytes, None).unwrap(),
        )
        .err()
        .unwrap();
    assert!(matches!(
        error.diagnostics()[0].payload(),
        DiagnosticPayload::InvalidRoadEditingSource {
            violation: RoadEditingSourceViolation::InvalidSemanticValue(
                RoadEditingInputViolation::DuplicateValue
            ),
            ..
        }
    ));
    builder
        .add_road_editing_module(
            re::RoadEditingModuleInput::try_new(DOCUMENT, source.as_bytes(), None).unwrap(),
        )
        .unwrap();
}

#[test]
fn canonical_access_regulation_checks_every_policy_and_ignores_source_variants() {
    let build = |mismatch| {
        compile_editing_custom(None, None, |builder, policy| {
            for index in 0..4 {
                let regulation = RegulationIdentity::try_new("engineering", "fixture-1")
                    .unwrap()
                    .with_source(format!("repository:access-{index}"))
                    .unwrap();
                builder
                    .add_declaration(re::RoadEditingDeclaration::AccessRule(
                        re::AccessRuleInput::try_new(
                            format!("allow-{index}"),
                            re::RoadEditingAccessTarget::ManeuverPath(
                                re::ManeuverPathReference::imported(
                                    TOPOLOGY,
                                    vec![JUNCTION.into(), "conflict-movement-a".into()],
                                    "path",
                                )
                                .unwrap(),
                            ),
                            AccessEffect::Allow,
                            vec![re::ParticipantClassReference::local("vehicle").unwrap()],
                            0,
                        )
                        .unwrap()
                        .with_regulation(regulation),
                    ))
                    .unwrap();
                let mut other = policy.clone();
                other.key = format!("policy-{index}").into();
                if mismatch && index == 3 {
                    other.regulation =
                        RegulationIdentity::try_new("engineering", "fixture-2").unwrap();
                }
                builder
                    .add_declaration(re::RoadEditingDeclaration::RightOfWayPolicySet(other))
                    .unwrap();
            }
        })
    };
    build(false).unwrap();
    violation(build(true), PolicyViolation::RegulationMismatch);
}

#[test]
fn policy_diagnostic_preserves_member_kind_when_gate_and_stream_keys_match() {
    let check = |result: Result<CompilationOutput, DiagnosticBundle>,
                 expected,
                 relation,
                 field_id| {
        let errors = result.err().unwrap();
        let diagnostic = errors
            .diagnostics()
            .iter()
            .find(|d| {
                matches!(d.payload(),
            DiagnosticPayload::InvalidPolicy { violation, .. } if *violation == expected)
            })
            .unwrap();
        let Some(SourceLocation::RoadEditing(location)) = diagnostic.primary_location() else {
            panic!("expected editor location");
        };
        assert!(
            matches!(location.subject(), RoadEditingSubject::OwnerLocal { relation: actual, .. } if *actual == relation)
        );
        assert_eq!(
            location.property_path().unwrap().steps(),
            &[RoadEditingPropertyStep::TableField {
                table: RoadEditingTableKind::RightOfWayPolicySet,
                field_id,
            }]
        );
    };
    check(
        compile_editing_custom(None, None, |_, policy| {
            policy.gates[0].interpretation = GateInterpretation::ProtectedGroup;
        }),
        PolicyViolation::SignalBinding,
        RoadEditingRelationKind::PolicyGateRule,
        5,
    );
    check(
        compile_editing_custom(None, Some([SignalAspect::Red; 2]), |_, policy| {
            for gate in &mut policy.gates {
                gate.interpretation = GateInterpretation::CnCircularRightTurn;
            }
        }),
        PolicyViolation::RightTurnRequired,
        RoadEditingRelationKind::PolicyGateRule,
        5,
    );
    check(
        compile_editing_custom(
            Some(ManeuverDirection::Right),
            Some([SignalAspect::Red; 2]),
            |_, policy| {
                for gate in &mut policy.gates {
                    gate.interpretation = GateInterpretation::CnCircularRightTurn;
                }
                let mut shadow = policy.gates[0].clone();
                shadow.key = "a-shadow".into();
                shadow.interpretation = GateInterpretation::DirectionalRightPermissive;
                policy.gates = policy.gates.iter().cloned().chain([shadow]).collect();
                policy.streams[0].key = "a-shadow".into();
            },
        ),
        PolicyViolation::LampTypeConflict,
        RoadEditingRelationKind::PolicyGateRule,
        5,
    );
    check(
        compile_editing_custom(None, None, |_, policy| {
            policy.streams[0].priority = 2;
        }),
        PolicyViolation::YieldPriority,
        RoadEditingRelationKind::PolicyStreamRule,
        4,
    );
}

#[test]
fn physical_lamp_kind_is_shared_across_policies_and_class_prohibitions() {
    violation(
        compile_editing_custom(
            Some(ManeuverDirection::Right),
            Some([SignalAspect::Red; 2]),
            |builder, policy| {
                for gate in &mut policy.gates {
                    gate.interpretation = GateInterpretation::CnCircularRightTurn;
                }
                let mut other = policy.clone();
                other.key = "alternative".into();
                other.gates[0].interpretation = GateInterpretation::DirectionalRightPermissive;
                builder
                    .add_declaration(re::RoadEditingDeclaration::RightOfWayPolicySet(other))
                    .unwrap();
            },
        ),
        PolicyViolation::LampTypeConflict,
    );
    // 同一圆形灯允许不同车型采用不同禁止条件。
    compile_editing_custom(
        Some(ManeuverDirection::Right),
        Some([SignalAspect::Red; 2]),
        |_, policy| {
            for gate in &mut policy.gates {
                gate.interpretation = GateInterpretation::CnCircularRightTurn;
            }
            let mut specific = policy.gates[0].clone();
            specific.key = "class-prohibition".into();
            specific.classes = Some(
                vec![re::ParticipantClassReference::local("vehicle").unwrap()].into_boxed_slice(),
            );
            specific.prohibition = GateProhibition::Always;
            policy.gates = policy.gates.iter().cloned().chain([specific]).collect();
        },
    )
    .unwrap();
}

#[test]
fn every_enterable_target_profile_must_have_strictly_higher_priority() {
    let customized = |builder: &mut re::RoadEditingSourceModuleBuilder<'_>,
                      policy: &mut re::RightOfWayPolicySetInput| {
        builder
            .add_declaration(re::RoadEditingDeclaration::ParticipantClass(
                re::ParticipantClassInput::try_new("bus")
                    .unwrap()
                    .with_extends(re::ParticipantClassReference::local("vehicle").unwrap()),
            ))
            .unwrap();
        builder
            .add_declaration(re::RoadEditingDeclaration::VehicleProfile(
                re::VehicleProfileInput::try_new(
                    "bus-profile",
                    re::ParticipantClassReference::local("bus").unwrap(),
                    re::IidmVehicleProfileInput::try_new(8., 12., 2., 1.4, 1.8, 2., 4.5).unwrap(),
                )
                .unwrap(),
            ))
            .unwrap();
        let mut bus = policy
            .streams
            .iter()
            .find(|r| r.key.as_ref() == "b")
            .unwrap()
            .clone();
        bus.key = "bus-target".into();
        bus.classes =
            Some(vec![re::ParticipantClassReference::local("bus").unwrap()].into_boxed_slice());
        bus.priority = 1;
        policy.streams = policy.streams.iter().cloned().chain([bus]).collect();
    };
    violation(
        compile_editing_custom(None, None, customized),
        PolicyViolation::YieldPriority,
    );
}

#[test]
fn inherited_evidence_and_source_only_edits_keep_semantic_diff_empty() {
    use laneflow_static_contract::PortableObjectKind;
    let compile_source = |canvas: Option<&str>| {
        compile_editing_custom(None, None, |_, policy| {
            policy.regulation = RegulationIdentity::try_new("engineering", "fixture-1")
                .unwrap()
                .with_source("repository:shared-evidence")
                .unwrap();
            policy.evidence = Box::new([]);
            for rule in &mut policy.streams {
                rule.evidence = Box::new([]);
            }
            for rule in &mut policy.gates {
                rule.evidence = Box::new([]);
            }
            if let Some(canvas) = canvas {
                *policy = policy.clone().with_canvas_selection(canvas).unwrap();
            }
        })
        .unwrap()
    };
    let a = compile_source(None);
    let b = compile_source(Some(""));
    let host_key = compile_source(Some("canvas::reserved"));
    assert_eq!(
        a.metrics().semantic_fingerprint(),
        b.metrics().semantic_fingerprint()
    );
    let provenance = PortableEmissionProvenance::try_new("w2-source-only").unwrap();
    let limits = laneflow_format::FormatLimits::HARD;
    let genesis =
        emit_portable_candidate(&a, &provenance, limits, PortableDiffBase::Genesis).unwrap();
    let host_candidate =
        emit_portable_candidate(&host_key, &provenance, limits, PortableDiffBase::Genesis).unwrap();
    assert_eq!(
        genesis.network_revision(),
        host_candidate.network_revision()
    );
    assert_ne!(
        genesis.source_map().bytes(),
        host_candidate.source_map().bytes()
    );
    let candidate = || {
        let base = laneflow_format::preflight_object_values(
            genesis.canonical_artifact().bytes(),
            PortableObjectKind::CanonicalArtifact,
            limits,
        )
        .unwrap();
        emit_portable_candidate(&b, &provenance, limits, PortableDiffBase::Artifact(base)).unwrap()
    };
    let changed = candidate();
    assert_eq!(genesis.network_revision(), changed.network_revision());
    assert_ne!(genesis.source_map().bytes(), changed.source_map().bytes());
    let diff = laneflow_format::preflight_object_values(
        changed.semantic_diff().bytes(),
        PortableObjectKind::SemanticDiff,
        limits,
    )
    .unwrap();
    for index in 1..7 {
        assert_eq!(
            diff.registry_view()
                .section(index)
                .unwrap()
                .table(0)
                .unwrap()
                .row_count(),
            0
        );
    }
    let provenance = PortablePublicationProvenance::new(
        PortablePublisherKind::LocalTool,
        "w2-publication",
        None,
        None,
    );
    let first = build_portable_publication_descriptor(changed, &provenance, limits).unwrap();
    let second = build_portable_publication_descriptor(candidate(), &provenance, limits).unwrap();
    assert_eq!(first.bytes(), second.bytes());
}

#[test]
fn compiler_retries_cleanly_after_policy_resource_exhaustion() {
    let mut compiler = Compiler::new();
    let mut unit = unit_with_policy(false, |_, _| {}).unwrap();
    unit.limits = unit
        .limits
        .clone()
        .with_test_admission_limit(CompileLimitDimension::RelationOccurrenceCount, 1);
    let errors = compiler.compile(unit).err().unwrap();
    assert!(errors.diagnostics().iter().any(|d| matches!(
        d.payload(),
        DiagnosticPayload::CompileLimitExceeded {
            dimension: CompileLimitDimension::RelationOccurrenceCount,
            ..
        }
    )));
    let retried = compiler
        .compile(unit_with_policy(false, |_, _| {}).unwrap())
        .unwrap();
    assert_eq!(
        retried.metrics().semantic_fingerprint(),
        compile(|_, _| {}).unwrap().metrics().semantic_fingerprint()
    );
}
