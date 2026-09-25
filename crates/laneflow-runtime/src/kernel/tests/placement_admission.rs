use std::sync::Arc;

use laneflow_compiler::{
    CompilationUnitBuilder, CompileLimits, Compiler, GateInterpretation, GateProhibition,
    IidmVehicleProfileInput, JunctionInput, JunctionReference, LaneEdgeInput, LaneEdgeReference,
    ManeuverGateInput, ManeuverPathInput, ManeuverPathReference, MovementInput, MovementReference,
    OwnerQualifiedReference, ParticipantClassInput, ParticipantClassReference, PolicyGateRuleInput,
    PolicyInputSource, PortableDiffBase, PortableEmissionProvenance, RegulationIdentity,
    RightOfWayPolicySetInput, SignalControlInput, SignalControllerInput, SignalGroupInput,
    SignalGroupReference, SignalGroupStateInput, SignalPhaseInput, SourceModuleHeader,
    SourceModuleHeaderInput, StopLineInput, StopLineReference, SyntheticModuleBuilder,
    VehicleProfileInput, derive_canonical_stable_id_v1, emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_post_emission_bundle};
use laneflow_static_contract::{
    EntityKind, LaneEdgeId, LaneEdgeOrdinal, SignalAspect, VehicleProfileOrdinal,
};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};

use super::placement::{can_slow_to_before, can_stop_before};
use crate::{
    CommittedNetworkSource, ExecutionConfig, PublishedLfcaReference, ReplaceError, RouteHandle,
    RouteRegisterInput, SpawnError, TrafficWorld, VehicleSpawnInput, VehicleStatus, WorldConfig,
};

fn profile() -> IidmVehicleProfileInput {
    IidmVehicleProfileInput {
        length_meters: 4.5,
        desired_speed_meters_per_second: 15.0,
        min_gap_meters: 2.0,
        time_headway_seconds: 1.5,
        max_acceleration_meters_per_second_squared: 1.5,
        comfortable_deceleration_meters_per_second_squared: 2.0,
        emergency_deceleration_meters_per_second_squared: 4.0,
    }
}

fn revision(
    namespace: &str,
    configure: impl FnOnce(&mut SyntheticModuleBuilder),
) -> Arc<SharedNetworkRevision> {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: namespace,
            source_document_key: "placement.document",
            generator_build_id: "git:0123456789abcdef",
            parameters_and_inputs_digest: [0x74; 32],
            frontend_options_digest: [0x2; 32],
            random_seed: Some(742),
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .expect("header");
    let mut module = SyntheticModuleBuilder::new(header, &limits).expect("module");
    module
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "road-user",
            extends: None,
        })
        .expect("class")
        .add_vehicle_profile(VehicleProfileInput {
            vehicle_profile_key: "car",
            participant_class: ParticipantClassReference::local("road-user"),
            iidm: profile(),
        })
        .expect("profile");
    configure(&mut module);
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(module.finish().expect("module"))
        .expect("unit module");
    let output = Compiler::new()
        .compile(unit.build().expect("unit"))
        .expect("compile");
    let provenance =
        PortableEmissionProvenance::try_new("laneflow-placement-admission-v1").expect("provenance");
    let candidate = emit_portable_candidate(
        &output,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .expect("candidate");
    let checked = check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .expect("checked bundle");
    build_shared_network_revision(
        checked.canonical_network_input(),
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("revision")
}

fn install(revision: Arc<SharedNetworkRevision>) -> TrafficWorld {
    install_dt(revision, 100)
}

fn install_dt(revision: Arc<SharedNetworkRevision>, delta_ms: u64) -> TrafficWorld {
    let origin = *revision.canonical_origin();
    TrafficWorld::install(
        Arc::clone(&revision),
        WorldConfig::new(8, 4, 1_024, 1_024, delta_ms),
        ExecutionConfig::new(std::num::NonZeroU32::MIN),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://placement-admission",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("source"),
        },
        742,
        crate::test_policy::selection(&revision),
    )
    .expect("install")
}

fn register_named(world: &mut TrafficWorld, namespace: &str, keys: &[&str]) -> RouteHandle {
    let limits = CompileLimits::p100_initial_v1();
    let edges: Vec<_> = keys
        .iter()
        .map(|key| {
            let stable =
                derive_canonical_stable_id_v1(EntityKind::LaneEdge, namespace, key, &limits)
                    .expect("edge id");
            world
                .revision()
                .identity()
                .ordinal(LaneEdgeId::from_untyped(stable))
                .expect(key)
        })
        .collect();
    world
        .register_route(RouteRegisterInput::new(edges))
        .expect("route")
}

fn register_edges(world: &mut TrafficWorld, edges: &[u32]) -> RouteHandle {
    world
        .register_route(RouteRegisterInput::new(
            edges
                .iter()
                .copied()
                .map(LaneEdgeOrdinal::from_raw)
                .collect::<Vec<_>>(),
        ))
        .expect("route")
}

fn spawn(
    world: &mut TrafficWorld,
    route: RouteHandle,
    edge: u32,
    progress_mm: u32,
    speed_mm_s: u32,
) -> Result<crate::VehicleHandle, SpawnError> {
    world.spawn_vehicle(
        VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            edge,
            progress_mm,
            speed_mm_s,
        )
        .with_open_entrance(),
    )
}

fn add_edge(
    module: &mut SyntheticModuleBuilder,
    key: &str,
    length_m: f64,
    limit_m_s: f64,
    next: Option<&str>,
) {
    let next_ref;
    let successors: &[LaneEdgeReference] = if let Some(next) = next {
        next_ref = LaneEdgeReference::local(next);
        std::slice::from_ref(&next_ref)
    } else {
        &[]
    };
    module
        .add_lane_edge(LaneEdgeInput {
            lane_edge_key: key,
            length_meters: length_m,
            speed_limit_meters_per_second: limit_m_s,
            successors,
        })
        .expect(key);
}

fn add_signal_corridor(module: &mut SyntheticModuleBuilder, aspect: SignalAspect) {
    add_edge(module, "entry", 10.0, 10.0, Some("middle"));
    add_edge(module, "middle", 8.0, 10.0, Some("exit"));
    add_edge(module, "exit", 12.0, 10.0, None);
    let states = [SignalGroupStateInput {
        signal_group: SignalGroupReference::local("group-entry"),
        aspect,
    }];
    let groups = [SignalGroupReference::local("group-entry")];
    module
        .add_junction(JunctionInput {
            junction_key: "junction-main",
        })
        .expect("junction")
        .add_movement(MovementInput {
            turn_direction: None,
            movement_key: "movement-through",
            junction: JunctionReference::local("junction-main"),
            directed_entry_approach_key: "approach-westbound",
            directed_exit_approach_key: "approach-eastbound",
        })
        .expect("movement")
        .add_maneuver_path(ManeuverPathInput {
            maneuver_path_key: "path-main",
            movement: MovementReference::local("movement-through"),
            entry_edge: LaneEdgeReference::local("entry"),
            internal_edges: &[LaneEdgeReference::local("middle")],
            exit_edge: LaneEdgeReference::local("exit"),
        })
        .expect("path")
        .add_stop_line(StopLineInput {
            stop_line_key: "stop-entry",
            lane_edge: LaneEdgeReference::local("entry"),
        })
        .expect("stop")
        .add_signal_group(SignalGroupInput {
            signal_group_key: "group-entry",
        })
        .expect("group")
        .add_maneuver_gate(ManeuverGateInput {
            maneuver_gate_key: "gate-entry",
            maneuver_path: ManeuverPathReference::local("path-main"),
            transition_index: 0,
            stop_line: StopLineReference::local("stop-entry"),
            signal_control: SignalControlInput::Group(SignalGroupReference::local("group-entry")),
        })
        .expect("gate")
        .add_signal_controller(SignalControllerInput {
            signal_controller_key: "controller-main",
            offset_ms: 0,
            signal_groups: &groups,
            phases: &[SignalPhaseInput {
                signal_phase_key: "phase-shown",
                duration_ms: 10_000,
                states: &states,
            }],
        })
        .expect("controller");
    crate::test_policy::add_gate_policy(
        module,
        "fixture-policy",
        &[("gate-entry", GateInterpretation::ProtectedGroup)],
    );
}

#[test]
fn stopping_distance_matches_the_shared_red_light_bound() {
    assert!(!can_stop_before(10_000, 4.0, 2_000));
    assert!(can_stop_before(3_000, 4.0, 2_000));
    assert!(can_stop_before(0, 4.0, 0));
    assert!(!can_slow_to_before(15_000, 5_000, 4.0, 1_000));
    assert!(can_slow_to_before(15_000, 5_000, 4.0, 50_000));
}

#[test]
fn short_approach_accepts_a_speed_that_rest_could_not_reach() {
    let revision = revision("runtime/placement-plain", |module| {
        add_edge(module, "approach", 13.0, 15.0, None);
    });
    let mut world = install(revision);
    let route = register_edges(&mut world, &[0]);
    spawn(&mut world, route, 0, 11_000, 10_000).expect("域内加速距离不够不是拒绝理由");
    assert_eq!(world.live_vehicles().len(), 1);
}

#[test]
fn red_gate_rejects_only_when_emergency_braking_cannot_stop() {
    let red = revision("runtime-fixture-policy", |module| {
        add_signal_corridor(module, SignalAspect::Red);
    });
    let mut world = install(Arc::clone(&red));
    let route = register_named(
        &mut world,
        "runtime-fixture-policy",
        &["entry", "middle", "exit"],
    );
    let cursor = world.command_cursor();
    assert_eq!(
        spawn(&mut world, route, 0, 8_000, 10_000).unwrap_err(),
        SpawnError::StopConstraintUnsatisfiable
    );
    assert_eq!(world.command_cursor(), cursor);
    assert!(world.live_vehicles().is_empty());
    spawn(&mut world, route, 0, 8_000, 3_000).expect("紧急制动停得住就接受");

    let mut green = install(revision("runtime-fixture-policy", |module| {
        add_signal_corridor(module, SignalAspect::Green);
    }));
    let route = register_named(
        &mut green,
        "runtime-fixture-policy",
        &["entry", "middle", "exit"],
    );
    spawn(&mut green, route, 0, 8_000, 10_000).expect("绿灯不因短进口拒绝");
}

fn add_later_closed_gate(module: &mut SyntheticModuleBuilder) {
    add_edge(module, "before", 30.0, 15.0, Some("approach"));
    add_edge(module, "approach", 1.0, 15.0, Some("exit"));
    add_edge(module, "exit", 20.0, 15.0, None);
    module
        .add_junction(JunctionInput {
            junction_key: "junction-later",
        })
        .expect("junction")
        .add_movement(MovementInput {
            turn_direction: None,
            movement_key: "movement-later",
            junction: JunctionReference::local("junction-later"),
            directed_entry_approach_key: "approach-westbound",
            directed_exit_approach_key: "approach-eastbound",
        })
        .expect("movement")
        .add_maneuver_path(ManeuverPathInput {
            maneuver_path_key: "path-later",
            movement: MovementReference::local("movement-later"),
            entry_edge: LaneEdgeReference::local("approach"),
            internal_edges: &[],
            exit_edge: LaneEdgeReference::local("exit"),
        })
        .expect("path")
        .add_stop_line(StopLineInput {
            stop_line_key: "stop-approach",
            lane_edge: LaneEdgeReference::local("approach"),
        })
        .expect("stop")
        .add_maneuver_gate(ManeuverGateInput {
            maneuver_gate_key: "gate-later",
            maneuver_path: ManeuverPathReference::local("path-later"),
            transition_index: 0,
            stop_line: StopLineReference::local("stop-approach"),
            signal_control: SignalControlInput::None,
        })
        .expect("gate");
    let span = module.policy_source_span();
    let source = PolicyInputSource {
        primary: &span,
        contributing: &[],
    };
    let rules = [PolicyGateRuleInput {
        rule_key: "gate-later",
        gate: OwnerQualifiedReference {
            target: laneflow_compiler::ManeuverGateReference::local("gate-later"),
            owner_keys: &[],
        },
        participant_classes: None,
        interpretation: GateInterpretation::Uncontrolled,
        prohibition: GateProhibition::Always,
        evidence_keys: &[],
        source,
    }];
    module
        .add_right_of_way_policy_set(RightOfWayPolicySetInput {
            policy_set_key: "fixture-policy",
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
        .expect("policy");
}

#[test]
fn later_closed_gate_rejects_when_this_tick_reaches_it() {
    let revision = revision("runtime-fixture-policy", add_later_closed_gate);
    let mut world = install_dt(Arc::clone(&revision), 1_000);
    let route = register_named(
        &mut world,
        "runtime-fixture-policy",
        &["before", "approach", "exit"],
    );
    assert_eq!(
        spawn(&mut world, route, 0, 22_800, 8_000).unwrap_err(),
        SpawnError::StopConstraintUnsatisfiable,
        "8.2 米外的拒绝门这一拍够得到，连续刹停距离却还够"
    );
    let mut farther = install_dt(revision, 1_000);
    let route = register_named(
        &mut farther,
        "runtime-fixture-policy",
        &["before", "approach", "exit"],
    );
    spawn(&mut farther, route, 0, 10_000, 8_000).expect("21 米外的门这一拍够不着");
}

#[test]
fn downstream_lower_limit_rejects_a_speed_that_cannot_fall() {
    let revision = revision("runtime/placement-plain", |module| {
        add_edge(module, "fast", 50.0, 15.0, Some("slow"));
        add_edge(module, "slow", 20.0, 5.0, None);
    });
    let mut world = install(revision);
    let route = register_edges(&mut world, &[0, 1]);
    spawn(&mut world, route, 0, 4_500, 15_000).expect("50 米够从 15 m/s 降到 5 m/s");
    assert_eq!(
        spawn(&mut world, route, 0, 49_000, 15_000).unwrap_err(),
        SpawnError::DownstreamSpeedUnsatisfiable
    );
}

#[test]
fn a_looser_intermediate_limit_does_not_hide_a_later_drop() {
    let revision = revision("runtime/placement-plain", |module| {
        add_edge(module, "fast", 5.0, 10.0, Some("middle"));
        add_edge(module, "middle", 1.0, 8.0, Some("slow"));
        add_edge(module, "slow", 20.0, 3.0, None);
    });
    let mut world = install(revision);
    let route = register_named(
        &mut world,
        "runtime/placement-plain",
        &["fast", "middle", "slow"],
    );
    assert_eq!(
        spawn(&mut world, route, 0, 4_500, 7_000).unwrap_err(),
        SpawnError::DownstreamSpeedUnsatisfiable,
        "7 m/s 低于中间的 8 m/s，但仍须降到 1 米外的 3 m/s"
    );
}

#[test]
fn leader_and_follower_use_emergency_braking_not_comfort_gap() {
    let revision = revision("runtime/placement-plain", |module| {
        add_edge(module, "road", 200.0, 25.0, None);
    });
    let mut world = install(Arc::clone(&revision));
    let route = register_edges(&mut world, &[0]);
    let stopped = spawn(&mut world, route, 0, 20_000, 0).expect("静止车");
    spawn(&mut world, route, 0, 25_000, 0).expect("不重叠但小于期望间距仍然接受");
    assert!(world.vehicle(stopped).is_some());

    let mut moving = install(Arc::clone(&revision));
    let route = register_edges(&mut moving, &[0]);
    let leader = spawn(&mut moving, route, 0, 16_500, 0).expect("静止前车");
    assert_eq!(
        spawn(&mut moving, route, 0, 10_000, 10_000).unwrap_err(),
        SpawnError::UnsafeLeader { leader }
    );
    let mut declared = install(Arc::clone(&revision));
    let route = register_edges(&mut declared, &[0]);
    let leader = spawn(&mut declared, route, 0, 16_500, 0).expect("静止前车");
    let fast = VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 10_000, 10_000)
        .with_open_entrance()
        .with_departure(crate::VehicleDepartureState::new(0, 10_000, 10_000));
    assert_eq!(
        declared.spawn_vehicle(fast).unwrap_err(),
        SpawnError::UnsafeLeader { leader }
    );

    let mut ahead = install(revision);
    let route = register_edges(&mut ahead, &[0]);
    let follower = spawn(&mut ahead, route, 0, 20_000, 20_000).expect("移动后车");
    let before = ahead.command_cursor();
    assert_eq!(
        spawn(&mut ahead, route, 0, 26_500, 0).unwrap_err(),
        SpawnError::UnsafeFollower { follower }
    );
    assert_eq!(ahead.command_cursor(), before);
    assert_eq!(ahead.live_vehicles(), &[follower]);
}

#[test]
fn restore_keeps_a_state_fresh_spawn_rejects() {
    let revision = revision("runtime/placement-plain", |module| {
        add_edge(module, "road", 200.0, 25.0, None);
    });
    let mut fresh = install(Arc::clone(&revision));
    let route = register_edges(&mut fresh, &[0]);
    let follower = spawn(&mut fresh, route, 0, 20_000, 20_000).expect("移动后车");
    let blocked = VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 26_500, 0)
        .with_open_entrance();
    assert_eq!(
        fresh.spawn_vehicle(blocked).unwrap_err(),
        SpawnError::UnsafeFollower { follower }
    );

    let mut restored = install(revision);
    let route = register_edges(&mut restored, &[0]);
    spawn(&mut restored, route, 0, 20_000, 20_000).expect("移动后车");
    let blocked = VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 26_500, 0)
        .with_open_entrance();
    restored
        .state
        .restore_unparked_vehicle(blocked, 0, VehicleStatus::Active, None, None, false)
        .expect("恢复不套用新鲜出生的运动安全");
    assert_eq!(restored.live_vehicles().len(), 2);
}

#[test]
fn occupancy_insert_matches_live_order_when_a_completed_vehicle_remains() {
    let revision = revision("runtime/placement-plain", |module| {
        add_edge(module, "road", 200.0, 25.0, None);
    });
    let mut world = install(revision);
    let route = register_edges(&mut world, &[0]);
    let completed = spawn(&mut world, route, 0, 10_000, 0).expect("先放进去的车");
    let index = usize::try_from(completed.index()).expect("index");
    world.state.committed.vehicles[index]
        .state
        .as_mut()
        .expect("vehicle")
        .status = VehicleStatus::Completed;
    world.state.rebuild_active_order();
    world.state.derived.spawn_overlap.mark_stale();
    world
        .state
        .rebuild_occupancy_index()
        .expect("完成车退出占用");
    let moving = spawn(&mut world, route, 0, 80_000, 0).expect("后面的车");
    let before = crate::kernel::occupancy::occupancy_fingerprint(&world.state.derived.occupancy);
    assert!(
        before
            .iter()
            .any(|row| row.0 == moving.index() && row.4 == 1),
        "完成车占着 live 序号，后车的占用序号是 1，实际 {before:?}"
    );
    world
        .state
        .rebuild_occupancy_index()
        .expect("按 live 顺序重建");
    assert_eq!(
        crate::kernel::occupancy::occupancy_fingerprint(&world.state.derived.occupancy),
        before
    );
}

#[test]
fn replace_uses_the_same_follower_admission() {
    let revision = revision("runtime/placement-plain", |module| {
        add_edge(module, "road", 200.0, 25.0, None);
    });
    let mut world = install(revision);
    let route = register_edges(&mut world, &[0]);
    let follower = spawn(&mut world, route, 0, 20_000, 20_000).expect("移动后车");
    let parked_aside = spawn(&mut world, route, 0, 100_000, 0).expect("待替换的车");
    let index = usize::try_from(parked_aside.index()).expect("index");
    world.state.committed.vehicles[index]
        .state
        .as_mut()
        .expect("vehicle")
        .status = VehicleStatus::Completed;
    world.state.rebuild_active_order();
    world.state.derived.spawn_overlap.mark_stale();
    world
        .state
        .rebuild_occupancy_index()
        .expect("完成后的车退出占用索引");
    let blocked = VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 26_500, 0)
        .with_open_entrance();
    assert_eq!(
        world
            .replace_completed_vehicle(parked_aside, blocked)
            .unwrap_err(),
        ReplaceError::UnsafeFollower { follower }
    );
    assert_eq!(world.live_vehicles().len(), 2);
    assert_eq!(
        world.vehicle(follower).map(|state| state.status()),
        Some(VehicleStatus::Active)
    );
}

fn install_sized(revision: Arc<SharedNetworkRevision>, vehicles: u32, routes: u32) -> TrafficWorld {
    let origin = *revision.canonical_origin();
    TrafficWorld::install(
        Arc::clone(&revision),
        WorldConfig::new(vehicles, routes, 4_096, 256, 100),
        ExecutionConfig::new(std::num::NonZeroU32::MIN),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://placement-admission",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("source"),
        },
        742,
        crate::test_policy::selection(&revision),
    )
    .expect("install")
}

fn same_speed_gap(leader_progress: u32, follower_progress: u32) -> i64 {
    i64::from(leader_progress) - 4_500 - i64::from(follower_progress)
}

#[test]
fn same_speed_at_min_gap_is_not_accepted_then_hard_stopped() {
    let revision = revision("runtime/placement-plain", |module| {
        add_edge(module, "road", 200.0, 25.0, None);
    });
    for (leader_at, follower_at) in [(16_500, 10_000), (16_400, 10_000)] {
        assert!(
            same_speed_gap(leader_at, follower_at) <= 2_000,
            "fixture gap must be at or under min_gap"
        );
        let mut behind = install(Arc::clone(&revision));
        let route = register_edges(&mut behind, &[0]);
        let leader = spawn(&mut behind, route, 0, leader_at, 10_000).expect("前车");
        assert_eq!(
            spawn(&mut behind, route, 0, follower_at, 10_000).unwrap_err(),
            SpawnError::UnsafeLeader { leader }
        );

        let mut ahead = install(Arc::clone(&revision));
        let route = register_edges(&mut ahead, &[0]);
        let follower = spawn(&mut ahead, route, 0, follower_at, 10_000).expect("后车");
        assert_eq!(
            spawn(&mut ahead, route, 0, leader_at, 10_000).unwrap_err(),
            SpawnError::UnsafeFollower { follower }
        );
    }
}

#[test]
fn accepted_same_speed_gap_survives_one_tick_without_a_hard_stop() {
    let revision = revision("runtime/placement-plain", |module| {
        add_edge(module, "road", 200.0, 25.0, None);
    });
    let mut behind = install(Arc::clone(&revision));
    let route = register_edges(&mut behind, &[0]);
    spawn(&mut behind, route, 0, 26_500, 10_000).expect("前车");
    let follower = spawn(&mut behind, route, 0, 10_000, 10_000).expect("净距足够的后车");
    behind.step(crate::TickInput::new(100)).expect("step");
    let moved = behind.vehicle(follower).expect("follower");
    assert!(
        moved.speed_mm_s() >= 9_600,
        "第一拍速度 {} 不能被硬投影打到紧急制动以下",
        moved.speed_mm_s()
    );
    let travel = moved.progress_mm() - 10_000;
    assert!(
        (900..1_500).contains(&travel),
        "第一拍位移 {travel} mm 应接近巡航，而不是被硬房间截成 0"
    );

    let mut ahead = install(revision);
    let route = register_edges(&mut ahead, &[0]);
    let follower = spawn(&mut ahead, route, 0, 10_000, 10_000).expect("后车");
    spawn(&mut ahead, route, 0, 26_500, 10_000).expect("前方新车");
    ahead.step(crate::TickInput::new(100)).expect("step");
    let moved = ahead.vehicle(follower).expect("follower");
    assert!(moved.speed_mm_s() >= 9_600, "speed {}", moved.speed_mm_s());
    let travel = moved.progress_mm() - 10_000;
    assert!((900..1_500).contains(&travel), "后车第一拍位移 {travel} mm");
}

#[test]
fn leader_emergency_does_not_donate_solver_room() {
    let revision = revision("runtime/placement-plain", |module| {
        module
            .add_vehicle_profile(VehicleProfileInput {
                vehicle_profile_key: "soft",
                participant_class: ParticipantClassReference::local("road-user"),
                iidm: IidmVehicleProfileInput {
                    emergency_deceleration_meters_per_second_squared: 2.5,
                    ..profile()
                },
            })
            .expect("soft profile");
        add_edge(module, "road", 200.0, 25.0, None);
    });
    let mut world = install(Arc::clone(&revision));
    let soft = world
        .traffic()
        .relations()
        .vehicle_profile(VehicleProfileOrdinal::from_raw(1))
        .expect("soft profile ordinal");
    assert!((soft.emergency_decel() - 2.5).abs() < 0.01);
    let route = register_edges(&mut world, &[0]);
    let leader = world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(1), route, 0, 16_500, 10_000)
                .with_open_entrance(),
        )
        .expect("制动很弱的前车");
    assert_eq!(
        spawn(&mut world, route, 0, 10_000, 10_000).unwrap_err(),
        SpawnError::UnsafeLeader { leader },
        "前车几乎停不住，也不能因此给后车腾出求解器并不传播的空间"
    );

    let mut ahead = install(revision);
    let route = register_edges(&mut ahead, &[0]);
    let follower = spawn(&mut ahead, route, 0, 10_000, 10_000).expect("后车");
    assert_eq!(
        ahead
            .spawn_vehicle(
                VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(1),
                    route,
                    0,
                    16_500,
                    10_000,
                )
                .with_open_entrance()
            )
            .unwrap_err(),
        SpawnError::UnsafeFollower { follower }
    );
}

#[test]
fn follower_emergency_uses_its_own_brakes() {
    let revision = revision("runtime/placement-plain", |module| {
        module
            .add_vehicle_profile(VehicleProfileInput {
                vehicle_profile_key: "firm",
                participant_class: ParticipantClassReference::local("road-user"),
                iidm: IidmVehicleProfileInput {
                    emergency_deceleration_meters_per_second_squared: 12.0,
                    ..profile()
                },
            })
            .expect("firm profile");
        add_edge(module, "road", 200.0, 25.0, None);
    });
    let mut firm_world = install(Arc::clone(&revision));
    let route = register_edges(&mut firm_world, &[0]);
    spawn(&mut firm_world, route, 0, 16_545, 1_000).expect("前车");
    firm_world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(1), route, 0, 10_000, 1_000)
                .with_open_entrance(),
        )
        .expect("更强的紧急制动放得进这 45 mm 空隙");

    let mut soft_world = install(revision);
    let route = register_edges(&mut soft_world, &[0]);
    let leader = spawn(&mut soft_world, route, 0, 16_545, 1_000).expect("前车");
    assert_eq!(
        spawn(&mut soft_world, route, 0, 10_000, 1_000).unwrap_err(),
        SpawnError::UnsafeLeader { leader },
        "同一空隙用后车自己的 4 m/s²，本拍硬截断超出它的紧急制动"
    );
}

#[test]
fn follower_on_previous_edge_uses_its_own_gap() {
    let revision = revision("runtime/placement-plain", |module| {
        add_edge(module, "entry", 20.0, 25.0, Some("exit"));
        add_edge(module, "exit", 20.0, 25.0, None);
    });
    let mut world = install(revision);
    let route = register_named(&mut world, "runtime/placement-plain", &["entry", "exit"]);
    let follower = spawn(&mut world, route, 0, 15_000, 10_000).expect("上一边的后车");
    assert_eq!(
        spawn(&mut world, route, 1, 1_500, 10_000).unwrap_err(),
        SpawnError::UnsafeFollower { follower }
    );
}

#[test]
fn repeated_edge_keeps_the_farther_rear_window() {
    let revision = revision("runtime/placement-plain", |module| {
        module
            .add_vehicle_profile(VehicleProfileInput {
                vehicle_profile_key: "short",
                participant_class: ParticipantClassReference::local("road-user"),
                iidm: IidmVehicleProfileInput {
                    length_meters: 0.3,
                    ..profile()
                },
            })
            .expect("short profile");
        add_edge(module, "a", 2.5, 15.0, Some("b"));
        add_edge(module, "b", 2.5, 15.0, Some("a"));
    });
    let mut world = install(Arc::clone(&revision));
    let route = register_named(&mut world, "runtime/placement-plain", &["a", "b", "a"]);
    let follower = world
        .spawn_vehicle(
            VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(1), route, 0, 900, 10_000)
                .with_open_entrance(),
        )
        .expect("靠后那段车尾后面的短车");
    assert_eq!(
        spawn(&mut world, route, 2, 500, 0).unwrap_err(),
        SpawnError::UnsafeFollower { follower },
        "环线上同一条边的两段车身都要留下，不能只留更靠近起点的那段"
    );
}

#[test]
fn cyclic_revisit_still_sees_the_follower_on_the_earlier_pass() {
    let revision = revision("runtime/placement-plain", |module| {
        module
            .add_vehicle_profile(VehicleProfileInput {
                vehicle_profile_key: "short",
                participant_class: ParticipantClassReference::local("road-user"),
                iidm: IidmVehicleProfileInput {
                    length_meters: 0.3,
                    ..profile()
                },
            })
            .expect("short profile");
        add_edge(module, "a", 12.0, 15.0, Some("b"));
        add_edge(module, "b", 2.0, 15.0, Some("a"));
    });
    let mut world = install(Arc::clone(&revision));
    let route = register_named(&mut world, "runtime/placement-plain", &["a", "b", "a"]);
    let follower = spawn(&mut world, route, 0, 11_500, 10_000).expect("第一圈靠近边末的后车");
    assert_eq!(
        world
            .spawn_vehicle(
                VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(1), route, 2, 500, 0,)
                    .with_open_entrance()
            )
            .unwrap_err(),
        SpawnError::UnsafeFollower { follower },
        "绕回同一条物理边时，边末的后车仍在制动距离里"
    );
}

#[test]
fn follower_behind_a_spanning_rear_is_not_hidden_by_the_front_edge() {
    let revision = revision("runtime/placement-plain", |module| {
        module
            .add_vehicle_profile(VehicleProfileInput {
                vehicle_profile_key: "long",
                participant_class: ParticipantClassReference::local("road-user"),
                iidm: IidmVehicleProfileInput {
                    length_meters: 30.0,
                    ..profile()
                },
            })
            .expect("long profile");
        add_edge(module, "entry", 100.0, 10.0, Some("exit"));
        add_edge(module, "exit", 50.0, 10.0, None);
    });
    let mut world = install(revision);
    let route = register_named(&mut world, "runtime/placement-plain", &["entry", "exit"]);
    let follower = spawn(&mut world, route, 0, 69_000, 10_000).expect("后杠后的近车");
    assert_eq!(
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(1),
                route,
                1,
                1_000,
                10_000,
            ).with_open_entrance())
            .unwrap_err(),
        SpawnError::UnsafeFollower { follower },
        "30 m 车身跨进下一条边时，后杠仍在上一条边中部，紧跟的后车必须被看见"
    );
}

#[test]
fn maneuver_transition_reaches_the_upstream_follower_search() {
    let revision = revision("runtime-fixture-policy", |module| {
        add_signal_corridor(module, SignalAspect::Green);
    });
    let mut world = install(Arc::clone(&revision));
    let route = register_named(
        &mut world,
        "runtime-fixture-policy",
        &["entry", "middle", "exit"],
    );
    let (entry, middle) = {
        let edges = world.route_edges(route).expect("corridor edges");
        (edges[0], edges[1])
    };
    {
        let revision = world.revision();
        let traffic = revision.traffic();
        world
            .state
            .workspace
            .occupancy_scratch
            .ensure_maneuver_upstream(traffic)
            .expect("maneuver reverse");
        let upstream = world
            .state
            .workspace
            .occupancy_scratch
            .maneuver_upstream(middle);
        assert!(
            upstream.contains(&entry.raw()),
            "机动 transition 的反向表必须含入口边，路口内部边没有车道后继时只靠这张表"
        );
    }
    let follower = spawn(&mut world, route, 0, 5_000, 10_000).expect("入口上的后车");
    assert_eq!(
        spawn(&mut world, route, 1, 1_500, 8_000).unwrap_err(),
        SpawnError::UnsafeFollower { follower }
    );
}

#[test]
fn shorter_upstream_path_is_not_hidden_by_an_earlier_long_path() {
    let revision = revision("runtime/placement-plain", |module| {
        let long_and_join = [
            LaneEdgeReference::local("long"),
            LaneEdgeReference::local("join"),
        ];
        let join_only = [LaneEdgeReference::local("join")];
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "stem",
                length_meters: 100.0,
                speed_limit_meters_per_second: 10.0,
                successors: &long_and_join,
            })
            .expect("stem")
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "long",
                length_meters: 16.0,
                speed_limit_meters_per_second: 10.0,
                successors: &join_only,
            })
            .expect("long")
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "join",
                length_meters: 20.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .expect("join");
    });
    let mut world = install(revision);
    let follower_route = register_named(&mut world, "runtime/placement-plain", &["stem", "join"]);
    let follower = spawn(&mut world, follower_route, 0, 99_000, 10_000).expect("短路近处的后车");
    let join = register_named(&mut world, "runtime/placement-plain", &["join"]);
    assert_eq!(
        spawn(&mut world, join, 0, 5_000, 10_000).unwrap_err(),
        SpawnError::UnsafeFollower { follower },
        "绕 16 m 的长路先走到时，不能挡住 1.5 m 外的直接后车"
    );
}

#[test]
fn near_red_light_is_rejected_when_the_first_tick_would_hard_stop() {
    let revision = revision("runtime-fixture-policy", |module| {
        add_signal_corridor(module, SignalAspect::Red);
    });
    let mut world = install(Arc::clone(&revision));
    let route = register_named(
        &mut world,
        "runtime-fixture-policy",
        &["entry", "middle", "exit"],
    );
    let cursor = world.command_cursor();
    assert_eq!(
        spawn(&mut world, route, 0, 9_968, 500).unwrap_err(),
        SpawnError::StopConstraintUnsatisfiable,
        "连续刹停距离够 32 mm，但第一拍仍会被红灯打成急停"
    );
    assert_eq!(world.command_cursor(), cursor);
    assert!(world.live_vehicles().is_empty());

    let parked = spawn(&mut world, route, 0, 4_500, 0).expect("待替换的车");
    let index = usize::try_from(parked.index()).expect("index");
    world.state.committed.vehicles[index]
        .state
        .as_mut()
        .expect("vehicle")
        .status = VehicleStatus::Completed;
    world.state.rebuild_active_order();
    world.state.derived.spawn_overlap.mark_stale();
    world
        .state
        .rebuild_occupancy_index()
        .expect("完成后的车退出占用索引");
    assert_eq!(
        world
            .replace_completed_vehicle(
                parked,
                VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 9_968, 500)
                    .with_open_entrance(),
            )
            .unwrap_err(),
        ReplaceError::StopConstraintUnsatisfiable
    );
}

#[test]
fn first_tick_route_end_clamp_beyond_emergency_braking_is_rejected() {
    let revision = revision("runtime/placement-plain", |module| {
        add_edge(module, "road", 20.0, 15.0, None);
    });
    let mut world = install(revision);
    let route = register_edges(&mut world, &[0]);
    assert_eq!(
        spawn(&mut world, route, 0, 19_600, 10_000).unwrap_err(),
        SpawnError::StopConstraintUnsatisfiable,
        "路终只剩 400 mm 时，第一拍会被截成超出紧急制动的急停"
    );
}

#[test]
fn diverge_follower_on_the_other_branch_is_not_a_direct_follower() {
    let revision = revision("runtime/placement-plain", |module| {
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "stem",
                length_meters: 30.0,
                speed_limit_meters_per_second: 25.0,
                successors: &[
                    LaneEdgeReference::local("left"),
                    LaneEdgeReference::local("right"),
                ],
            })
            .expect("stem");
        add_edge(module, "left", 20.0, 25.0, None);
        add_edge(module, "right", 20.0, 25.0, None);
    });
    let mut world = install(revision);
    let right = register_named(&mut world, "runtime/placement-plain", &["stem", "right"]);
    let left = register_named(&mut world, "runtime/placement-plain", &["stem", "left"]);
    spawn(&mut world, right, 0, 10_000, 10_000).expect("走右支的后车");
    spawn(&mut world, left, 1, 5_000, 10_000).expect("左支上的车不是它的直接前车");
}

#[test]
fn unrelated_roads_do_not_rebuild_or_scan_every_vehicle() {
    const ROADS: u32 = 24;
    let revision = revision("runtime/placement-plain", |module| {
        for index in 0..ROADS {
            add_edge(module, &format!("road-{index}"), 50.0, 25.0, None);
        }
    });
    let mut world = install_sized(revision, ROADS + 4, ROADS + 4);
    let mut keys = Vec::new();
    for index in 0..ROADS {
        keys.push(format!("road-{index}"));
    }
    let key_refs: Vec<&str> = keys.iter().map(String::as_str).collect();
    crate::kernel::occupancy::reset_occupancy_rebuild_events();
    crate::kernel::placement::reset_follower_candidates();
    let started = std::time::Instant::now();
    for index in 0..ROADS {
        let route = register_named(
            &mut world,
            "runtime/placement-plain",
            &[key_refs[index as usize]],
        );
        spawn(&mut world, route, 0, 10_000, 0).expect("disjoint spawn");
    }
    let elapsed = started.elapsed();
    assert_eq!(
        crate::kernel::occupancy::occupancy_rebuild_events(),
        1,
        "连续成功摆放不能逐车全量重建占用索引"
    );
    assert_eq!(
        crate::kernel::placement::follower_candidates(),
        0,
        "互不连接的道路不能把其他路上的车算进后车候选"
    );
    assert!(
        elapsed.as_millis() < 2_000,
        "24 条互不相关道路的摆放耗时 {} ms",
        elapsed.as_millis()
    );
    let patched = world.state.derived.occupancy.record_keys();
    world.state.rebuild_occupancy_index().expect("对照重建");
    assert_eq!(
        patched,
        world.state.derived.occupancy.record_keys(),
        "增量索引必须和全量重建一致"
    );
}
