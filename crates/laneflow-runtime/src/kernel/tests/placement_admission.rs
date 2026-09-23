use std::sync::Arc;

use laneflow_compiler::{
    CompilationUnitBuilder, CompileLimits, Compiler, GateInterpretation, IidmVehicleProfileInput,
    JunctionInput, JunctionReference, LaneEdgeInput, LaneEdgeReference, ManeuverGateInput,
    ManeuverPathInput, ManeuverPathReference, MovementInput, MovementReference,
    ParticipantClassInput, ParticipantClassReference, PortableDiffBase, PortableEmissionProvenance,
    SignalControlInput, SignalControllerInput, SignalGroupInput, SignalGroupReference,
    SignalGroupStateInput, SignalPhaseInput, SourceModuleHeader, SourceModuleHeaderInput,
    StopLineInput, StopLineReference, SyntheticModuleBuilder, VehicleProfileInput,
    derive_canonical_stable_id_v1, emit_portable_candidate,
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
    let origin = *revision.canonical_origin();
    TrafficWorld::install(
        Arc::clone(&revision),
        WorldConfig::new(8, 4, 1_024, 1_024, 100),
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
    world.spawn_vehicle(VehicleSpawnInput::new(
        VehicleProfileOrdinal::from_raw(0),
        route,
        edge,
        progress_mm,
        speed_mm_s,
    ))
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

#[test]
fn downstream_lower_limit_rejects_a_speed_that_cannot_fall() {
    let revision = revision("runtime/placement-plain", |module| {
        add_edge(module, "fast", 50.0, 15.0, Some("slow"));
        add_edge(module, "slow", 20.0, 5.0, None);
    });
    let mut world = install(revision);
    let route = register_edges(&mut world, &[0, 1]);
    spawn(&mut world, route, 0, 0, 15_000).expect("50 米够从 15 m/s 降到 5 m/s");
    assert_eq!(
        spawn(&mut world, route, 0, 49_000, 15_000).unwrap_err(),
        SpawnError::DownstreamSpeedUnsatisfiable
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
    let blocked = VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 26_500, 0);
    assert_eq!(
        fresh.spawn_vehicle(blocked).unwrap_err(),
        SpawnError::UnsafeFollower { follower }
    );

    let mut restored = install(revision);
    let route = register_edges(&mut restored, &[0]);
    spawn(&mut restored, route, 0, 20_000, 20_000).expect("移动后车");
    let blocked = VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 26_500, 0);
    restored
        .state
        .restore_unparked_vehicle(blocked, 0, VehicleStatus::Active, None, None, false)
        .expect("恢复不套用新鲜出生的运动安全");
    assert_eq!(restored.live_vehicles().len(), 2);
}

#[test]
fn replace_uses_the_same_follower_admission() {
    let revision = revision("runtime/placement-plain", |module| {
        add_edge(module, "road", 200.0, 25.0, None);
    });
    let mut world = install(revision);
    let route = register_edges(&mut world, &[0]);
    let follower = spawn(&mut world, route, 0, 20_000, 20_000).expect("移动后车");
    let parked_aside = spawn(&mut world, route, 0, 0, 0).expect("待替换的车");
    let index = usize::try_from(parked_aside.index()).expect("index");
    world.state.committed.vehicles[index]
        .state
        .as_mut()
        .expect("vehicle")
        .status = VehicleStatus::Completed;
    world.state.rebuild_active_order();
    world.state.derived.spawn_overlap.mark_stale();
    let blocked = VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 26_500, 0);
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
