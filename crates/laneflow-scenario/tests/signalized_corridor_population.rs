#[path = "support/population_policy.rs"]
mod population_policy;

use std::sync::Arc;

use laneflow_format::{FormatLimits, check_canonical_network_input};
use laneflow_runtime::{
    TickInput, TrafficWorld, VehicleHandle, VehicleReplaceBlock, VehicleSpawnInput, VehicleState,
    VehicleStatus, WorldConfig,
};
use laneflow_scenario::signalized_corridor::{
    CorridorCatalog, CorridorPopulationCapacities, CorridorPopulationConfig,
    CorridorPopulationController, CorridorPopulationError, CorridorPopulationPrepare,
    CorridorReplaceApplyError, CorridorReplaceAttemptOutcome, DEFAULT_SEED,
    DEFAULT_TARGET_VEHICLE_COUNT, MAX_TARGET_VEHICLE_COUNT, MIN_TARGET_VEHICLE_COUNT,
    PASSENGER_CAR_PROFILE_KEY, bind,
};
#[cfg(feature = "placement-fixtures")]
use laneflow_static_contract::VehicleProfileOrdinal;
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};

fn install_fixture(
    revision: std::sync::Arc<laneflow_static_network::SharedNetworkRevision>,
    config: laneflow_runtime::WorldConfig,
) -> Result<laneflow_runtime::TrafficWorld, laneflow_runtime::InstallError> {
    let selection = fixture_policy("laneflow/signalized-corridor", "protected-entry");
    install_with_policy(revision, config, selection)
}

fn fixture_policy(namespace: &str, key: &str) -> laneflow_runtime::WorldPolicySelection {
    laneflow_runtime::WorldPolicySelection::Pinned(laneflow_runtime::PolicyPin {
        policy: laneflow_static_contract::RightOfWayPolicySetId::from_untyped(
            laneflow_compiler::derive_canonical_stable_id_v1(
                laneflow_static_contract::EntityKind::RightOfWayPolicySet,
                namespace,
                key,
                &laneflow_compiler::CompileLimits::p100_initial_v1(),
            )
            .expect("explicit fixture policy identity"),
        ),
    })
}

fn install_with_policy(
    revision: Arc<laneflow_static_network::SharedNetworkRevision>,
    config: WorldConfig,
    selection: laneflow_runtime::WorldPolicySelection,
) -> Result<TrafficWorld, laneflow_runtime::InstallError> {
    let origin = *revision.canonical_origin();
    laneflow_runtime::TrafficWorld::install(
        Arc::clone(&revision),
        config,
        laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        laneflow_runtime::CommittedNetworkSource::Published {
            reference: laneflow_runtime::PublishedLfcaReference::new(
                "fixture://in-process",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("non-empty fixture key"),
        },
        0,
        selection,
    )
}

const CORRIDOR_LFCA: &[u8] = include_bytes!("../../../examples/data/v0.2-signalized-corridor.lfca");
const CORRIDOR_CATALOG: &str =
    include_str!("../../../examples/data/v0.2-signalized-corridor.catalog.toml");

fn revision() -> Arc<laneflow_static_network::SharedNetworkRevision> {
    let input = check_canonical_network_input(CORRIDOR_LFCA, FormatLimits::HARD)
        .expect("checked canonical network input");
    build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("shared network revision")
}

fn catalog() -> CorridorCatalog {
    toml::from_str(CORRIDOR_CATALOG).expect("checked-in catalog")
}

fn prepare(
    target: usize,
    seed: u64,
) -> (
    CorridorPopulationPrepare,
    Arc<laneflow_static_network::SharedNetworkRevision>,
) {
    let revision = revision();
    let bound = bind(&catalog(), revision.as_ref()).expect("bind");
    let profile = *bound
        .profiles
        .get(PASSENGER_CAR_PROFILE_KEY)
        .expect("passenger-car");
    let config = CorridorPopulationConfig::try_new(target, seed).expect("config");
    let prepared = CorridorPopulationPrepare::prepare(config, bound, revision.as_ref(), profile)
        .expect("prepare");
    (prepared, revision)
}

fn spawn_plans(
    world: &mut TrafficWorld,
    prepared: &mut CorridorPopulationPrepare,
    plans: &mut [laneflow_scenario::signalized_corridor::CorridorVehiclePlan],
) -> (Vec<VehicleHandle>, Vec<laneflow_runtime::RouteHandle>) {
    let routes = prepared
        .install_routes(world)
        .expect("install catalog routes");
    let vehicles = prepared
        .admit_initial_plans(world, &routes, plans)
        .expect("initial spawn");
    (vehicles, routes)
}

fn spawn_population(
    world: &mut TrafficWorld,
    prepared: &mut CorridorPopulationPrepare,
) -> (Vec<VehicleHandle>, Vec<laneflow_runtime::RouteHandle>) {
    prepared
        .spawn_initial_vehicles(world)
        .expect("initial population")
}

#[test]
fn config_freezes_defaults_and_closed_target_range() {
    let default = CorridorPopulationConfig::default();
    assert_eq!(default.target_vehicle_count(), DEFAULT_TARGET_VEHICLE_COUNT);
    assert_eq!(default.seed(), DEFAULT_SEED);
    assert!(CorridorPopulationConfig::try_new(MIN_TARGET_VEHICLE_COUNT, 7).is_ok());
    assert!(CorridorPopulationConfig::try_new(MAX_TARGET_VEHICLE_COUNT, 7).is_ok());
    assert!(matches!(
        CorridorPopulationConfig::try_new(MIN_TARGET_VEHICLE_COUNT - 1, 7),
        Err(CorridorPopulationError::InvalidTargetVehicleCount { .. })
    ));
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn install_routes_rejects_exhausted_cursor_without_leaving_routes() {
    let (prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world =
        install_fixture(revision, WorldConfig::new(8, 64, 1_024, 1_024, TICK_MS)).expect("install");
    let before = world.live_routes().count();
    let cursor = u64::MAX - 1;
    world.set_command_cursor_for_test(cursor);
    let error = prepared
        .install_routes(&mut world)
        .expect_err("cursor cannot cover register and rollback");
    assert!(
        matches!(
            error,
            CorridorPopulationError::BoundWorldCatalogMismatch { .. }
        ),
        "exhausted cursor is a catalog install failure, got {error:?}"
    );
    assert_eq!(world.live_routes().count(), before);
    assert_eq!(world.command_cursor(), cursor);
}

#[test]
fn install_routes_rejects_short_capacity_without_leaving_routes() {
    let (prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world =
        install_fixture(revision, WorldConfig::new(8, 1, 1_024, 1_024, TICK_MS)).expect("install");
    assert!(prepared.install_routes(&mut world).is_err());
    assert_eq!(world.live_routes().count(), 0);
}

#[test]
fn prepare_50_100_200_are_deterministic_for_seed_zero() {
    fn fingerprint(target: usize) -> Vec<(usize, u32, u32)> {
        let (prepared, _) = prepare(target, 0);
        prepared
            .initial_vehicles()
            .iter()
            .map(|plan| (plan.route_index, plan.route_edge_index, plan.progress_mm))
            .collect()
    }
    let fifty = fingerprint(50);
    let hundred = fingerprint(100);
    let two_hundred = fingerprint(200);
    assert_eq!(fifty.len(), 50);
    assert_eq!(hundred.len(), 100);
    assert_eq!(two_hundred.len(), 200);
    assert_eq!(fingerprint(50), fifty);
    assert_eq!(fifty, hundred[..50]);
    assert_eq!(hundred, two_hundred[..100]);
}

#[test]
fn bind_and_replace_does_not_despawn_then_spawn() {
    let (mut prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(MIN_TARGET_VEHICLE_COUNT).expect("fits"),
            28,
            1_024,
            1_024,
            TICK_MS,
        ),
    )
    .expect("install");
    let (vehicles, routes) = spawn_population(&mut world, &mut prepared);
    let mut controller = prepared.bind(&mut world, &vehicles, &routes).expect("bind");
    assert_eq!(controller.counts().running, MIN_TARGET_VEHICLE_COUNT);
    assert_eq!(controller.counts().pending, 0);

    let mut completed = 0;
    for _ in 0..8_000 {
        world.step(TickInput::new(TICK_MS)).expect("step");
        completed += controller.consume_world(&world).expect("consume");
        if completed > 0 {
            break;
        }
    }
    assert!(completed > 0, "corridor vehicles must be able to complete");
    assert_eq!(
        controller.counts().running + controller.counts().pending,
        MIN_TARGET_VEHICLE_COUNT
    );
    for index in 0..MIN_TARGET_VEHICLE_COUNT {
        let handle = controller.logical_vehicle(index).expect("slot");
        assert!(
            world.vehicle(handle).is_some(),
            "禁止先消失再生成：Completed 必须仍 live"
        );
    }

    let rng_before = controller.rng_state();
    let report = controller
        .apply_pending(
            world.revision().network_revision(),
            world.policy_selection(),
            |old, input| {
                CorridorReplaceAttemptOutcome::from_replace(
                    world.replace_completed_vehicle(old, input),
                )
            },
        )
        .expect("apply");
    assert_eq!(report.attempted, completed);
    assert_eq!(report.replaced + report.blocked, completed);
    assert_eq!(controller.rng_state(), rng_before);
    assert!(
        report.replaced > 0 || report.blocked > 0,
        "lifecycle boundary must attempt replace"
    );
    for index in 0..MIN_TARGET_VEHICLE_COUNT {
        let handle = controller.logical_vehicle(index).expect("slot");
        let state = world.vehicle(handle).expect("still live");
        assert!(
            state.status() == VehicleStatus::Active || state.status() == VehicleStatus::Completed
        );
    }
}

#[test]
fn blocked_retry_replays_the_same_plan() {
    let (mut prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(MIN_TARGET_VEHICLE_COUNT).expect("fits"),
            28,
            1_024,
            1_024,
            TICK_MS,
        ),
    )
    .expect("install");
    let (vehicles, routes) = spawn_population(&mut world, &mut prepared);
    let mut controller = prepared.bind(&mut world, &vehicles, &routes).expect("bind");
    for _ in 0..8_000 {
        world.step(TickInput::new(TICK_MS)).expect("step");
        if controller.consume_world(&world).expect("consume") > 0 {
            break;
        }
    }
    assert!(controller.counts().pending > 0);
    let old = (0..MIN_TARGET_VEHICLE_COUNT)
        .map(|index| controller.logical_vehicle(index).expect("slot"))
        .find(|handle| {
            world
                .vehicle(*handle)
                .is_some_and(|state| state.status() == VehicleStatus::Completed)
        })
        .expect("completed handle");
    let first = controller
        .pending_spawn_input(&world, old)
        .expect("pending input");
    let rng_before = controller.rng_state();
    let before_caps = controller.capacities();
    let pending = controller.counts().pending;
    let report = controller
        .apply_pending(
            world.revision().network_revision(),
            world.policy_selection(),
            |old, _input| {
                Ok::<_, std::convert::Infallible>(CorridorReplaceAttemptOutcome::Blocked(
                    VehicleReplaceBlock {
                        old,
                        blocker: old,
                        blocker_ahead: true,
                        bumper_gap: 0,
                    },
                ))
            },
        )
        .expect("forced blocked");
    assert_eq!(report.blocked, pending);
    assert_eq!(report.replaced, 0);
    assert_eq!(controller.rng_state(), rng_before);
    assert_eq!(controller.capacities(), before_caps);
    let second = controller
        .pending_spawn_input(&world, old)
        .expect("same pending plan");
    assert_eq!(first.route(), second.route());
    assert_eq!(first.progress_mm(), second.progress_mm());
    assert_eq!(first.initial_speed_mm_s(), second.initial_speed_mm_s());
}

#[test]
fn apply_pending_host_error_restores_fifo_front() {
    let (mut prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(MIN_TARGET_VEHICLE_COUNT).expect("fits"),
            28,
            1_024,
            1_024,
            TICK_MS,
        ),
    )
    .expect("install");
    let (vehicles, routes) = spawn_population(&mut world, &mut prepared);
    let mut controller = prepared.bind(&mut world, &vehicles, &routes).expect("bind");
    for _ in 0..8_000 {
        world.step(TickInput::new(TICK_MS)).expect("step");
        if controller.consume_world(&world).expect("consume") > 0 {
            break;
        }
    }
    let pending = controller.counts().pending;
    assert!(pending > 0);
    let mut front = None;
    let error = controller
        .apply_pending(
            world.revision().network_revision(),
            world.policy_selection(),
            |old, input| {
                front = Some((old, input));
                Err("host-fail")
            },
        )
        .expect_err("host failure");
    assert!(matches!(
        error,
        CorridorReplaceApplyError::Host("host-fail")
    ));
    let (old, first) = front.expect("callback saw FIFO front");
    assert_eq!(controller.counts().pending, pending);
    let replayed = controller
        .pending_spawn_input(&world, old)
        .expect("front restored");
    assert_eq!(first, replayed);
}

#[test]
fn take_initial_vehicles_then_bind_reaches_running() {
    let (mut prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(MIN_TARGET_VEHICLE_COUNT).expect("fits"),
            28,
            1_024,
            1_024,
            TICK_MS,
        ),
    )
    .expect("install");
    let mut plans = prepared.take_initial_vehicles();
    assert_eq!(plans.len(), MIN_TARGET_VEHICLE_COUNT);
    let (vehicles, routes) = spawn_plans(&mut world, &mut prepared, &mut plans);
    let controller = prepared
        .bind(&mut world, &vehicles, &routes)
        .expect("bind after take");
    assert_eq!(controller.counts().running, MIN_TARGET_VEHICLE_COUNT);
    assert_eq!(controller.counts().pending, 0);
}

#[test]
fn admit_initial_plans_rejects_a_mismatched_slice_before_spawn() {
    let (mut prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(MIN_TARGET_VEHICLE_COUNT).expect("fits"),
            28,
            1_024,
            1_024,
            TICK_MS,
        ),
    )
    .expect("install");
    let mut plans = prepared.take_initial_vehicles();
    let routes = prepared
        .install_routes(&mut world)
        .expect("install catalog routes");
    let error = prepared
        .admit_initial_plans(&mut world, &routes, {
            let end = plans.len() - 1;
            &mut plans[..end]
        })
        .expect_err("truncated plan");
    assert!(matches!(
        error,
        CorridorPopulationError::InitialVehicleCount {
            expected: MIN_TARGET_VEHICLE_COUNT,
            actual,
        } if actual == MIN_TARGET_VEHICLE_COUNT - 1
    ));
    assert!(world.live_vehicles().is_empty());
    plans[0].progress_mm = plans[0].progress_mm.saturating_add(1);
    let error = prepared
        .admit_initial_plans(&mut world, &routes, &mut plans)
        .expect_err("foreign progress");
    assert!(matches!(
        error,
        CorridorPopulationError::InitialVehicleMismatch { slot_index: 0 }
    ));
    assert!(world.live_vehicles().is_empty());
}

#[test]
fn admit_initial_plans_rejects_foreign_routes_before_spawn() {
    let (mut prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(MIN_TARGET_VEHICLE_COUNT).expect("fits"),
            28,
            1_024,
            1_024,
            TICK_MS,
        ),
    )
    .expect("install");
    let mut plans = prepared.take_initial_vehicles();
    let mut routes = prepared
        .install_routes(&mut world)
        .expect("install catalog routes");
    routes.swap(0, 1);
    let error = prepared
        .admit_initial_plans(&mut world, &routes, &mut plans)
        .expect_err("reordered routes");
    assert!(matches!(
        error,
        CorridorPopulationError::BoundWorldCatalogMismatch { .. }
    ));
    assert!(world.live_vehicles().is_empty());
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn admit_initial_plans_rolls_back_a_partial_batch() {
    let (mut prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(1, 28, 1_024, 1_024, TICK_MS),
    )
    .expect("install");
    let mut plans = prepared.take_initial_vehicles();
    let saved = plans
        .iter()
        .map(|plan| plan.initial_speed_mm_s)
        .collect::<Vec<_>>();
    let routes = prepared
        .install_routes(&mut world)
        .expect("install catalog routes");
    let error = prepared
        .admit_initial_plans(&mut world, &routes, &mut plans)
        .expect_err("later vehicle exceeds capacity");
    assert!(matches!(
        error,
        CorridorPopulationError::InitialSpawnRejected { .. }
    ));
    assert!(world.live_vehicles().is_empty());
    assert_eq!(
        plans
            .iter()
            .map(|plan| plan.initial_speed_mm_s)
            .collect::<Vec<_>>(),
        saved
    );
    assert_eq!(prepared.slot_initial_speed_mm_s(0), saved[0]);
    assert!(world.route_edges(routes[0]).is_some());
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn allocation_failure_does_not_lower_corridor_initial_speed() {
    let (mut prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(64, 28, 1_024, 1_024, TICK_MS),
    )
    .expect("install");
    let mut plans = prepared.take_initial_vehicles();
    let saved = plans
        .iter()
        .map(|plan| plan.initial_speed_mm_s)
        .collect::<Vec<_>>();
    let routes = prepared
        .install_routes(&mut world)
        .expect("install catalog routes");
    laneflow_runtime::set_contender_reserve_failure(true);
    let error = prepared
        .admit_initial_plans(&mut world, &routes, &mut plans)
        .expect_err("allocation failure");
    laneflow_runtime::set_contender_reserve_failure(false);
    assert!(matches!(
        error,
        CorridorPopulationError::InitialSpawnRejected { .. }
    ));
    assert!(world.live_vehicles().is_empty());
    assert_eq!(
        plans
            .iter()
            .map(|plan| plan.initial_speed_mm_s)
            .collect::<Vec<_>>(),
        saved
    );
    assert_eq!(prepared.slot_initial_speed_mm_s(0), saved[0]);
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn admit_initial_plans_restores_a_dropped_speed_when_a_later_vehicle_cannot_enter() {
    let (mut prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(1, 28, 1_024, 1_024, TICK_MS),
    )
    .expect("install");
    let mut plans = prepared.initial_vehicles().to_vec();
    let routes = prepared.install_routes(&mut world).expect("routes");
    let edge = world
        .route_edges(routes[plans[0].route_index])
        .expect("route")[plans[0].route_edge_index as usize];
    let lengths = world.traffic().lane_lengths_millimetres();
    let length = lengths[edge.index()];
    let limit = world.traffic().lane_speed_limits_millimetres_per_second()[edge.index()];
    let close = length.saturating_sub(2_000);
    assert!(
        close > plans[0].progress_mm,
        "edge end must be farther than the prepared pose"
    );
    assert_eq!(
        world.spawn_vehicle(VehicleSpawnInput::new(
            plans[0].profile,
            routes[plans[0].route_index],
            plans[0].route_edge_index,
            close,
            limit,
        )),
        Err(laneflow_runtime::SpawnError::StopConstraintUnsatisfiable),
        "this pose must be rejected until the speed drops"
    );
    plans[0].progress_mm = close;
    plans[0].initial_speed_mm_s = limit;
    prepared.set_slot_progress_mm(0, close);
    prepared.set_slot_initial_speed_mm_s(0, limit);
    let saved = plans
        .iter()
        .map(|plan| plan.initial_speed_mm_s)
        .collect::<Vec<_>>();
    let error = prepared
        .admit_initial_plans(&mut world, &routes, &mut plans)
        .expect_err("second vehicle exceeds capacity");
    assert!(matches!(
        error,
        CorridorPopulationError::InitialSpawnRejected { .. }
    ));
    assert!(world.live_vehicles().is_empty());
    assert_eq!(
        plans
            .iter()
            .map(|plan| plan.initial_speed_mm_s)
            .collect::<Vec<_>>(),
        saved
    );
    assert_eq!(prepared.slot_initial_speed_mm_s(0), saved[0]);
    assert_eq!(prepared.initial_vehicles()[0].initial_speed_mm_s, saved[0]);
}

#[test]
fn spawn_initial_vehicles_removes_routes_it_registered() {
    let (mut prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(1, 28, 1_024, 1_024, TICK_MS),
    )
    .expect("install");
    let error = prepared
        .spawn_initial_vehicles(&mut world)
        .expect_err("capacity rejects the batch");
    assert!(matches!(
        error,
        CorridorPopulationError::InitialSpawnRejected { .. }
    ));
    let _ = prepared
        .install_routes(&mut world)
        .expect("failed spawn removed the routes it registered");
}

#[test]
fn consume_world_rejects_skipped_ticks() {
    let (mut prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(MIN_TARGET_VEHICLE_COUNT).expect("fits"),
            28,
            1_024,
            1_024,
            TICK_MS,
        ),
    )
    .expect("install");
    let (vehicles, routes) = spawn_population(&mut world, &mut prepared);
    let mut controller = prepared.bind(&mut world, &vehicles, &routes).expect("bind");
    world.step(TickInput::new(TICK_MS)).expect("first step");
    world
        .step(TickInput::new(TICK_MS))
        .expect("skipped consume");
    let error = controller.consume_world(&world).expect_err("gap");
    assert!(matches!(
        error,
        CorridorPopulationError::NonMonotonicStep {
            previous: 0,
            actual: 2
        }
    ));
}

#[cfg(feature = "placement-fixtures")]
#[test]
fn consume_world_rejects_untracked_completed_vehicle() {
    let (mut prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(MIN_TARGET_VEHICLE_COUNT + 1).expect("fits"),
            28,
            1_024,
            1_024,
            TICK_MS,
        ),
    )
    .expect("install");
    let extra_route_index = prepared.initial_vehicles()[0].route_index;
    let (vehicles, routes) = spawn_population(&mut world, &mut prepared);
    let mut controller = prepared.bind(&mut world, &vehicles, &routes).expect("bind");
    let extra = spawn_near_route_end(&mut world, routes[extra_route_index]);
    world.step(TickInput::new(TICK_MS)).expect("step");
    assert_eq!(
        world.vehicle(extra).expect("extra").status(),
        VehicleStatus::Completed
    );
    let error = controller.consume_world(&world).expect_err("untracked");
    assert!(matches!(
        error,
        CorridorPopulationError::UnknownCompletionVehicle { vehicle } if vehicle == extra
    ));
}

fn foreign_world() -> TrafficWorld {
    const S1: &[u8] = include_bytes!(
        "../../../crates/laneflow-compiler/tests/fixtures/portable/lfca-world-policies/full-spatial.lfca"
    );
    let input = check_canonical_network_input(S1, FormatLimits::HARD).expect("s1");
    let foreign = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("foreign revision");
    install_with_policy(
        foreign,
        WorldConfig::new(8, 4, 1_024, 1_024, 100),
        fixture_policy("runtime-fixture-policy", "fixture-policy"),
    )
    .expect("install")
}

#[cfg(feature = "placement-fixtures")]
fn spawn_near_route_end(
    world: &mut TrafficWorld,
    route: laneflow_runtime::RouteHandle,
) -> VehicleHandle {
    let edges = world.route_edges(route).expect("edges").to_vec();
    let last = *edges.last().expect("route has edges");
    let last_length = world.traffic().lane_lengths_millimetres()[last.index()];
    let speed_limit = world.traffic().lane_speed_limits_millimetres_per_second()[last.index()];
    let last_index = u32::try_from(edges.len() - 1).expect("index");
    world
        .place_existing_active_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            last_index,
            last_length.saturating_sub(50),
            speed_limit,
        ))
        .expect("extra near end")
}

#[test]
fn spawn_input_rejects_foreign_revision() {
    let (prepared, _) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let world = foreign_world();
    assert!(
        prepared.initial_vehicles()[0]
            .spawn_input(&world, &[])
            .is_err(),
        "plan must fail-closed on another NetworkRevisionId"
    );
}

#[test]
fn consume_world_rejects_foreign_revision() {
    let (mut prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(MIN_TARGET_VEHICLE_COUNT).expect("fits"),
            28,
            1_024,
            1_024,
            TICK_MS,
        ),
    )
    .expect("install");
    let (vehicles, routes) = spawn_population(&mut world, &mut prepared);
    let mut controller = prepared.bind(&mut world, &vehicles, &routes).expect("bind");
    let error = controller
        .consume_world(&foreign_world())
        .expect_err("foreign consume");
    assert!(matches!(
        error,
        CorridorPopulationError::BoundWorldCatalogMismatch { .. }
    ));
}

#[test]
fn pending_spawn_input_rejects_foreign_revision() {
    let (mut prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(MIN_TARGET_VEHICLE_COUNT).expect("fits"),
            28,
            1_024,
            1_024,
            TICK_MS,
        ),
    )
    .expect("install");
    let (vehicles, routes) = spawn_population(&mut world, &mut prepared);
    let mut controller = prepared.bind(&mut world, &vehicles, &routes).expect("bind");
    for _ in 0..8_000 {
        world.step(TickInput::new(TICK_MS)).expect("step");
        if controller.consume_world(&world).expect("consume") > 0 {
            break;
        }
    }
    let old = (0..MIN_TARGET_VEHICLE_COUNT)
        .map(|index| controller.logical_vehicle(index).expect("slot"))
        .find(|handle| {
            world
                .vehicle(*handle)
                .is_some_and(|state| state.status() == VehicleStatus::Completed)
        })
        .expect("completed handle");
    let error = controller
        .pending_spawn_input(&foreign_world(), old)
        .expect_err("foreign pending");
    assert!(matches!(
        error,
        CorridorPopulationError::BoundWorldCatalogMismatch { .. }
    ));
}

#[test]
fn apply_pending_rejects_foreign_revision() {
    let (mut prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(MIN_TARGET_VEHICLE_COUNT).expect("fits"),
            28,
            1_024,
            1_024,
            TICK_MS,
        ),
    )
    .expect("install");
    let (vehicles, routes) = spawn_population(&mut world, &mut prepared);
    let mut controller = prepared.bind(&mut world, &vehicles, &routes).expect("bind");
    for _ in 0..8_000 {
        world.step(TickInput::new(TICK_MS)).expect("step");
        if controller.consume_world(&world).expect("consume") > 0 {
            break;
        }
    }
    let pending = controller.counts().pending;
    assert!(pending > 0);
    let mut called = false;
    let error = controller
        .apply_pending(
            foreign_world().revision().network_revision(),
            world.policy_selection(),
            |old, _| {
                called = true;
                Ok::<_, std::convert::Infallible>(CorridorReplaceAttemptOutcome::Blocked(
                    VehicleReplaceBlock {
                        old,
                        blocker: old,
                        blocker_ahead: true,
                        bumper_gap: 0,
                    },
                ))
            },
        )
        .expect_err("foreign apply");
    assert!(!called, "host callback must not run on revision mismatch");
    assert!(matches!(
        error,
        CorridorReplaceApplyError::Policy(
            CorridorPopulationError::BoundWorldCatalogMismatch { .. }
        )
    ));
    assert_eq!(controller.counts().pending, pending);
}

const TICK_MS: u64 = 16;
const SHORT_SOAK_REPLACED: usize = 50;
const FULL_SOAK_REPLACED: usize = 10_000;
const SHORT_SOAK_MAX_TICKS: u32 = 50_000;
const FULL_SOAK_MAX_TICKS: u32 = 5_000_000;
const REPLAY_TICKS: u32 = 12_000;

fn bound_controller(target: usize) -> (TrafficWorld, CorridorPopulationController) {
    let (mut prepared, revision) = prepare(target, DEFAULT_SEED);
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(target).expect("fits"),
            28,
            1_024,
            1_024,
            TICK_MS,
        ),
    )
    .expect("install");
    let (vehicles, routes) = spawn_population(&mut world, &mut prepared);
    let controller = prepared.bind(&mut world, &vehicles, &routes).expect("bind");
    (world, controller)
}

fn lifecycle_tick(
    world: &mut TrafficWorld,
    controller: &mut CorridorPopulationController,
) -> usize {
    let revision = world.revision().network_revision();
    let replaced = controller
        .apply_pending(revision, world.policy_selection(), |old, input| {
            CorridorReplaceAttemptOutcome::from_replace(world.replace_completed_vehicle(old, input))
        })
        .expect("apply")
        .replaced;
    world.step(TickInput::new(TICK_MS)).expect("step");
    controller.consume_world(world).expect("consume");
    replaced
}

fn soak_replacements(
    target: usize,
    replaced_goal: usize,
    max_ticks: u32,
) -> (
    CorridorPopulationCapacities,
    CorridorPopulationCapacities,
    usize,
) {
    let (mut world, mut controller) = bound_controller(target);
    let warmed = controller.capacities();
    let mut replaced = 0;
    for _ in 1..=max_ticks {
        replaced += lifecycle_tick(&mut world, &mut controller);
        let counts = controller.counts();
        assert_eq!(counts.running + counts.pending, target);
        assert_eq!(world.live_vehicles().len(), target);
        if replaced >= replaced_goal {
            break;
        }
    }
    assert!(
        replaced >= replaced_goal,
        "target {target} only reached {replaced} replacements in {max_ticks} ticks"
    );
    let after = controller.capacities();
    (warmed, after, replaced)
}

#[derive(Clone, Debug, PartialEq)]
struct HeadlessSnapshot {
    tick_index: u64,
    time_ms: u64,
    rng_at_bind: u64,
    rng_state: u64,
    replaced: usize,
    running: usize,
    pending: usize,
    last_consumed_tick: u64,
    live: Vec<VehicleHandle>,
    logical: Vec<VehicleHandle>,
    pending_fifo: Vec<VehicleHandle>,
    pending_plans: Vec<VehicleSpawnInput>,
    states: Vec<VehicleState>,
    capacities: CorridorPopulationCapacities,
}

fn snapshot(
    world: &TrafficWorld,
    controller: &CorridorPopulationController,
    rng_at_bind: u64,
    replaced: usize,
) -> HeadlessSnapshot {
    let target = controller.counts().target;
    let pending_fifo = controller.pending_vehicles();
    let pending_plans = pending_fifo
        .iter()
        .map(|old| {
            controller
                .pending_spawn_input(world, *old)
                .expect("pending plan")
        })
        .collect();
    let states = world
        .live_vehicles()
        .iter()
        .map(|handle| world.vehicle(*handle).expect("live"))
        .collect();
    HeadlessSnapshot {
        tick_index: world.tick_index(),
        time_ms: world.time_ms(),
        rng_at_bind,
        rng_state: controller.rng_state(),
        replaced,
        running: controller.counts().running,
        pending: controller.counts().pending,
        last_consumed_tick: controller.last_consumed_tick(),
        live: world.live_vehicles().to_vec(),
        logical: (0..target)
            .map(|index| controller.logical_vehicle(index).expect("slot"))
            .collect(),
        pending_fifo,
        pending_plans,
        states,
        capacities: controller.capacities(),
    }
}

fn run_chunked(target: usize, ticks: u32, chunk: u32) -> HeadlessSnapshot {
    assert_eq!(ticks % chunk, 0);
    let (mut world, mut controller) = bound_controller(target);
    let rng_at_bind = controller.rng_state();
    let mut replaced = 0;
    let frames = ticks / chunk;
    for _ in 0..frames {
        for _ in 0..chunk {
            replaced += lifecycle_tick(&mut world, &mut controller);
        }
    }
    snapshot(&world, &controller, rng_at_bind, replaced)
}

#[test]
fn soak_50_cars_keeps_retained_capacity() {
    let (warmed, after, replaced) = soak_replacements(
        MIN_TARGET_VEHICLE_COUNT,
        SHORT_SOAK_REPLACED,
        SHORT_SOAK_MAX_TICKS,
    );
    assert_eq!(after, warmed);
    assert!(replaced >= SHORT_SOAK_REPLACED);
}

#[test]
fn soak_200_cars_keeps_retained_capacity() {
    let (warmed, after, replaced) = soak_replacements(
        MAX_TARGET_VEHICLE_COUNT,
        SHORT_SOAK_REPLACED,
        SHORT_SOAK_MAX_TICKS,
    );
    assert_eq!(after, warmed);
    assert!(replaced >= SHORT_SOAK_REPLACED);
}

#[test]
#[ignore = "完整 10,000 次成功 replace；见 laneflow-scenario README"]
fn soak_50_cars_10000_replacements() {
    let (warmed, after, replaced) = soak_replacements(
        MIN_TARGET_VEHICLE_COUNT,
        FULL_SOAK_REPLACED,
        FULL_SOAK_MAX_TICKS,
    );
    assert_eq!(after, warmed);
    assert!(replaced >= FULL_SOAK_REPLACED);
}

#[test]
fn per_tick_chain_is_deterministic_across_independent_runs() {
    let one = run_chunked(MIN_TARGET_VEHICLE_COUNT, REPLAY_TICKS, 1);
    let four = run_chunked(MIN_TARGET_VEHICLE_COUNT, REPLAY_TICKS, 4);
    let eight = run_chunked(MIN_TARGET_VEHICLE_COUNT, REPLAY_TICKS, 8);
    assert_eq!(one, four);
    assert_eq!(one, eight);
    assert_eq!(one.tick_index, u64::from(REPLAY_TICKS));
    assert_eq!(one.time_ms, u64::from(REPLAY_TICKS) * TICK_MS);
    assert_eq!(one.last_consumed_tick, u64::from(REPLAY_TICKS));
    assert_ne!(
        one.rng_state, one.rng_at_bind,
        "12,000 ticks must consume recycle draws"
    );
    assert!(
        one.replaced > 0 || one.pending > 0,
        "replay window must reach completion or replacement"
    );
    assert_eq!(one.states.len(), one.live.len());
    assert_eq!(one.pending_fifo.len(), one.pending);
}

#[test]
fn grouped_steps_without_per_tick_consume_are_rejected() {
    let (mut world, mut controller) = bound_controller(MIN_TARGET_VEHICLE_COUNT);
    let _ = lifecycle_tick(&mut world, &mut controller);
    world.step(TickInput::new(TICK_MS)).expect("second step");
    world.step(TickInput::new(TICK_MS)).expect("third step");
    let error = controller
        .consume_world(&world)
        .expect_err("skipped consume");
    assert!(matches!(
        error,
        CorridorPopulationError::NonMonotonicStep {
            previous: 1,
            actual: 3
        }
    ));
}

#[test]
fn catalog_binding_rejects_unknown_policy_and_not_required_on_gated_root() {
    use laneflow_scenario::signalized_corridor::{BindError, CatalogPolicySelection};
    let revision = revision();
    let mut catalog = catalog();
    catalog.policy_selection = CatalogPolicySelection::NotRequired {};
    assert_eq!(
        bind(&catalog, &revision).unwrap_err(),
        BindError::PolicyRequired
    );
    let policy = laneflow_static_contract::RightOfWayPolicySetId::from_untyped(
        laneflow_static_contract::StableId128::ZERO,
    );
    catalog.policy_selection = CatalogPolicySelection::Pinned {
        policy: policy.to_string(),
    };
    assert_eq!(
        bind(&catalog, &revision).unwrap_err(),
        BindError::UnknownPolicy(policy)
    );
}

#[test]
fn population_rejects_other_policy_on_same_root_without_mutating_lifecycle() {
    let (revision, catalog, selections) = population_policy::fixture(catalog());
    for (selected, other) in [
        (selections[0], selections[1]),
        (selections[0], selections[2]),
        (selections[2], selections[0]),
    ] {
        let prepare_selected = |selection| {
            let mut catalog = catalog.clone();
            population_policy::select(&mut catalog, selection);
            let bound = bind(&catalog, &revision).unwrap();
            let profile = bound.profiles[PASSENGER_CAR_PROFILE_KEY];
            CorridorPopulationPrepare::prepare(
                CorridorPopulationConfig::try_new(MIN_TARGET_VEHICLE_COUNT, 0).unwrap(),
                bound,
                &revision,
                profile,
            )
            .unwrap()
        };
        let config = WorldConfig::new(50, 28, 1_024, 1_024, TICK_MS);
        let mut world = install_with_policy(Arc::clone(&revision), config, selected).unwrap();
        let mut foreign = install_with_policy(Arc::clone(&revision), config, other).unwrap();
        assert!(Arc::ptr_eq(&world.revision(), &foreign.revision()));
        assert_ne!(world.policy_selection(), foreign.policy_selection());
        let mut prepared = prepare_selected(selected);
        let mut foreign_prepared = prepare_selected(other);
        let (vehicles, routes) = spawn_population(&mut world, &mut prepared);
        let (foreign_vehicles, foreign_routes) =
            spawn_population(&mut foreign, &mut foreign_prepared);
        // 两个世界的局部句柄碰巧完全一致，修订和句柄校验不足以发现错配。
        assert_eq!(vehicles, foreign_vehicles);
        assert_eq!(routes, foreign_routes);
        assert!(matches!(
            prepared.initial_vehicles()[0].spawn_input(&foreign, &foreign_routes),
            Err(CorridorPopulationError::BoundWorldCatalogMismatch { .. })
        ));
        assert!(prepared.install_routes(&mut foreign).is_err());
        assert!(matches!(
            prepare_selected(selected).bind(&mut foreign, &foreign_vehicles, &foreign_routes),
            Err(CorridorPopulationError::BoundWorldCatalogMismatch { .. })
        ));
        let mut controller = prepared.bind(&mut world, &vehicles, &routes).unwrap();
        let rng = controller.rng_state();
        world.step(TickInput::new(TICK_MS)).unwrap();
        foreign.step(TickInput::new(TICK_MS)).unwrap();
        let before = snapshot(&world, &controller, rng, 0);
        assert!(matches!(
            controller.consume_world(&foreign),
            Err(CorridorPopulationError::BoundWorldCatalogMismatch { .. })
        ));
        assert_eq!(snapshot(&world, &controller, rng, 0), before);
        controller.consume_world(&world).unwrap();
        for _ in 0..8_000 {
            world.step(TickInput::new(TICK_MS)).unwrap();
            if controller.consume_world(&world).unwrap() > 0 {
                break;
            }
        }
        let before = snapshot(&world, &controller, rng, 0);
        let old = *before
            .pending_fifo
            .first()
            .expect("fixture reaches completion");
        assert!(matches!(
            controller.pending_spawn_input(&foreign, old),
            Err(CorridorPopulationError::BoundWorldCatalogMismatch { .. })
        ));
        let foreign_digest =
            laneflow_runtime::deterministic_state_digest(&foreign.capture_snapshot().unwrap())
                .unwrap();
        let mut called = false;
        let error = controller
            .apply_pending(
                foreign.revision().network_revision(),
                foreign.policy_selection(),
                |old, input| {
                    called = true;
                    CorridorReplaceAttemptOutcome::from_replace(
                        foreign.replace_completed_vehicle(old, input),
                    )
                },
            )
            .expect_err("foreign policy must fail before callback");
        assert!(!called);
        assert!(matches!(
            error,
            CorridorReplaceApplyError::Policy(
                CorridorPopulationError::BoundWorldCatalogMismatch { .. }
            )
        ));
        assert_eq!(snapshot(&world, &controller, rng, 0), before);
        assert_eq!(
            laneflow_runtime::deterministic_state_digest(&foreign.capture_snapshot().unwrap())
                .unwrap(),
            foreign_digest
        );
        // 拒绝不污染队列、PRNG、句柄、容量或消费拍号，原世界可继续替换与步进。
        assert!(lifecycle_tick(&mut world, &mut controller) > 0);
        assert_eq!(
            controller.counts().running + controller.counts().pending,
            MIN_TARGET_VEHICLE_COUNT
        );
    }
}
