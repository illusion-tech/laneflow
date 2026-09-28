//! #679 固定物理路网、车辆数和运动窗口，仅增长循环路线出现项。
use super::runtime_types as rt;
use laneflow_compiler::*;
use laneflow_format::{FormatLimits, check_post_emission_bundle};
use laneflow_static_contract::{
    EntityKind, ManeuverPathOrdinal, RightOfWayPolicySetId, SignalAspect, VehicleProfileId,
    VehicleProfileOrdinal,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};
use std::sync::Arc;

pub const STEPS: usize = 128;
pub const WARM: usize = 16;
pub const VEHICLES: u32 = 512;
pub const REPEATS: [usize; 3] = [8, 128, 2_048];
const NAMESPACE: &str = "research/route-query";

pub fn revision(red: bool) -> Arc<SharedNetworkRevision> {
    let limits = CompileLimits::single_network_1m_v2();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: NAMESPACE,
            source_document_key: "route-query.document",
            generator_build_id: "route-query-v1",
            parameters_and_inputs_digest: [u8::from(red); 32],
            frontend_options_digest: [0x79; 32],
            random_seed: Some(679),
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .unwrap();
    let mut module = SyntheticModuleBuilder::new(header, &limits).unwrap();
    for (class, profile) in [("road-user", "car"), ("restricted-user", "restricted")] {
        module
            .add_participant_class(ParticipantClassInput {
                participant_class_key: class,
                extends: None,
            })
            .unwrap();
        module
            .add_vehicle_profile(VehicleProfileInput {
                vehicle_profile_key: profile,
                participant_class: ParticipantClassReference::local(class),
                iidm: IidmVehicleProfileInput {
                    length_meters: 4.5,
                    desired_speed_meters_per_second: 13.75,
                    min_gap_meters: 2.0,
                    time_headway_seconds: 1.4,
                    max_acceleration_meters_per_second_squared: 1.8,
                    comfortable_deceleration_meters_per_second_squared: 2.0,
                    emergency_deceleration_meters_per_second_squared: 4.5,
                },
            })
            .unwrap();
    }
    for (edge, next) in [("entry", "storage"), ("storage", "exit"), ("exit", "entry")] {
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: edge,
                length_meters: 8_000.0,
                speed_limit_meters_per_second: 15.0,
                successors: &[LaneEdgeReference::local(next)],
            })
            .unwrap();
    }
    module
        .add_junction(JunctionInput {
            junction_key: "junction",
        })
        .unwrap()
        .add_movement(MovementInput {
            turn_direction: None,
            movement_key: "movement",
            junction: JunctionReference::local("junction"),
            directed_entry_approach_key: "in",
            directed_exit_approach_key: "out",
        })
        .unwrap()
        .add_maneuver_path(ManeuverPathInput {
            maneuver_path_key: "path",
            movement: MovementReference::local("movement"),
            entry_edge: LaneEdgeReference::local("entry"),
            internal_edges: &[LaneEdgeReference::local("storage")],
            exit_edge: LaneEdgeReference::local("exit"),
        })
        .unwrap();
    for (index, edge, gate, group) in [
        (0, "entry", "entry-gate", "entry-signal"),
        (1, "storage", "release-gate", "release-signal"),
    ] {
        module
            .add_stop_line(StopLineInput {
                stop_line_key: edge,
                lane_edge: LaneEdgeReference::local(edge),
            })
            .unwrap()
            .add_signal_group(SignalGroupInput {
                signal_group_key: group,
            })
            .unwrap()
            .add_maneuver_gate(ManeuverGateInput {
                maneuver_gate_key: gate,
                maneuver_path: ManeuverPathReference::local("path"),
                transition_index: index,
                stop_line: StopLineReference::local(edge),
                signal_control: SignalControlInput::Group(SignalGroupReference::local(group)),
            })
            .unwrap();
    }
    let groups = [
        SignalGroupReference::local("entry-signal"),
        SignalGroupReference::local("release-signal"),
    ];
    let states = [
        SignalGroupStateInput {
            signal_group: groups[0],
            aspect: SignalAspect::Green,
        },
        SignalGroupStateInput {
            signal_group: groups[1],
            aspect: if red {
                SignalAspect::Red
            } else {
                SignalAspect::Green
            },
        },
    ];
    module
        .add_signal_controller(SignalControllerInput {
            signal_controller_key: "controller",
            offset_ms: 0,
            signal_groups: &groups,
            phases: &[SignalPhaseInput {
                signal_phase_key: "fixed",
                duration_ms: 60_000,
                states: &states,
            }],
        })
        .unwrap();
    module
        .add_waiting_zone(WaitingZoneInput {
            waiting_zone_key: "waiting",
            maneuver_path: ManeuverPathReference::local("path"),
            entry_gate: ManeuverGateReference::local("entry-gate"),
            release_gate: ManeuverGateReference::local("release-gate"),
            max_occupancy: 1_024,
        })
        .unwrap();
    let span = module.policy_source_span();
    let source = PolicyInputSource {
        primary: &span,
        contributing: &[],
    };
    let mut rules: Vec<_> = ["entry-gate", "release-gate"]
        .iter()
        .map(|key| PolicyGateRuleInput {
            rule_key: key,
            gate: OwnerQualifiedReference {
                target: ManeuverGateReference::local(key),
                owner_keys: &[],
            },
            participant_classes: None,
            interpretation: GateInterpretation::ProtectedGroup,
            prohibition: GateProhibition::None,
            evidence_keys: &[],
            source,
        })
        .collect();
    let restricted = [ParticipantClassReference::local("restricted-user")];
    rules.push(PolicyGateRuleInput {
        rule_key: "restricted-entry",
        gate: OwnerQualifiedReference {
            target: ManeuverGateReference::local("entry-gate"),
            owner_keys: &[],
        },
        participant_classes: Some(&restricted),
        interpretation: GateInterpretation::ProtectedGroup,
        prohibition: GateProhibition::Always,
        evidence_keys: &[],
        source,
    });
    for policy_key in ["policy", "deny-policy"] {
        if policy_key == "deny-policy" {
            for rule in &mut rules {
                rule.prohibition = GateProhibition::Always;
            }
        }
        module
            .add_right_of_way_policy_set(RightOfWayPolicySetInput {
                policy_set_key: policy_key,
                regulation: RegulationIdentity {
                    jurisdiction: "engineering",
                    version: "route-query-v1",
                    source: Some("repository:route-query-v1"),
                },
                evidence: &[],
                gap_profiles: &[],
                stream_rules: &[],
                gate_rules: &rules,
                source,
            })
            .unwrap();
    }
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(module.finish().unwrap()).unwrap();
    let compiled = Compiler::new().compile(unit.build().unwrap()).unwrap();
    let candidate = emit_portable_candidate(
        &compiled,
        &PortableEmissionProvenance::try_new("route-query-v1").unwrap(),
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .unwrap();
    let checked = check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .unwrap();
    build_shared_network_revision(
        checked.canonical_network_input(),
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .unwrap()
}

pub fn world(root: &Arc<SharedNetworkRevision>, repeats: usize) -> (rt::TrafficWorld, u128) {
    world_with_policy(root, repeats, "policy")
}

pub fn world_with_policy(
    root: &Arc<SharedNetworkRevision>,
    repeats: usize,
    policy_key: &str,
) -> (rt::TrafficWorld, u128) {
    let origin = root.canonical_origin();
    let policy = RightOfWayPolicySetId::from_untyped(
        derive_canonical_stable_id_v1(
            EntityKind::RightOfWayPolicySet,
            NAMESPACE,
            policy_key,
            &CompileLimits::single_network_1m_v2(),
        )
        .unwrap(),
    );
    let mut world = rt::TrafficWorld::install(
        Arc::clone(root),
        rt::WorldConfig::new(VEHICLES, 4, 32_768, 32_768, 100),
        rt::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        rt::CommittedNetworkSource::Published {
            reference: rt::PublishedLfcaReference::new(
                "fixture://route-query",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .unwrap(),
        },
        679,
        rt::WorldPolicySelection::Pinned(rt::PolicyPin { policy }),
    )
    .unwrap();
    let edges = root
        .traffic()
        .maneuvers()
        .maneuver_path(ManeuverPathOrdinal::from_raw(0))
        .unwrap()
        .edges()
        .repeat(repeats);
    let start = std::time::Instant::now();
    let route = world
        .register_route(rt::RouteRegisterInput::new(edges))
        .unwrap();
    let build_ns = start.elapsed().as_nanos();
    for i in 0..VEHICLES {
        world
            .spawn_vehicle(
                rt::VehicleSpawnInput::new(profile(root, "car"), route, 0, (i + 1) * 10_000, 0)
                    .with_open_entrance(),
            )
            .unwrap();
    }
    for _ in 0..WARM {
        step(&mut world);
    }
    (world, build_ns)
}

pub fn step(world: &mut rt::TrafficWorld) {
    std::hint::black_box(world.step(rt::TickInput::new(100)).unwrap());
}

pub fn validate(world: &rt::TrafficWorld) -> u64 {
    assert_eq!(world.live_vehicles().len(), VEHICLES as usize);
    world
        .live_vehicles()
        .iter()
        .map(|handle| {
            let state = world.vehicle(*handle).unwrap();
            assert_eq!(state.status(), rt::VehicleStatus::Active);
            assert_eq!(state.route_edge_index(), 0);
            u64::from(state.progress_mm()) + u64::from(state.speed_mm_s())
        })
        .sum()
}

pub fn profile(root: &SharedNetworkRevision, key: &str) -> VehicleProfileOrdinal {
    let id = VehicleProfileId::from_untyped(
        derive_canonical_stable_id_v1(
            EntityKind::VehicleProfile,
            NAMESPACE,
            key,
            &CompileLimits::single_network_1m_v2(),
        )
        .unwrap(),
    );
    root.identity().ordinal(id).unwrap()
}
