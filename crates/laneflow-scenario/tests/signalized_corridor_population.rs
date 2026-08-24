use std::sync::Arc;

use laneflow_format::{FormatLimits, check_canonical_network_input_v1};
use laneflow_runtime::{
    TickInput, TrafficWorld, VehicleHandle, VehicleReplaceBlock, VehicleSpawnInput, VehicleStatus,
    WorldConfig,
};
use laneflow_scenario::signalized_corridor::{
    CorridorCatalog, CorridorPopulationConfig, CorridorPopulationError, CorridorPopulationPrepare,
    CorridorReplaceApplyError, CorridorReplaceAttemptOutcome, DEFAULT_SEED,
    DEFAULT_TARGET_VEHICLE_COUNT, MAX_TARGET_VEHICLE_COUNT, MIN_TARGET_VEHICLE_COUNT,
    PASSENGER_CAR_PROFILE_KEY, bind,
};
use laneflow_static_contract::{StaticRouteOrdinal, VehicleProfileOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};

const CORRIDOR_LFCA: &[u8] = include_bytes!("../../../examples/data/v0.2-signalized-corridor.lfca");
const CORRIDOR_CATALOG: &str =
    include_str!("../../../examples/data/v0.2-signalized-corridor.catalog.toml");

fn revision() -> Arc<laneflow_static_network::SharedNetworkRevision> {
    let input = check_canonical_network_input_v1(CORRIDOR_LFCA, FormatLimits::V1_HARD)
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
    plans: &[laneflow_scenario::signalized_corridor::CorridorVehiclePlan],
) -> Vec<VehicleHandle> {
    plans
        .iter()
        .map(|plan| {
            world
                .spawn_vehicle(plan.spawn_input(world).expect("spawn input"))
                .expect("initial spawn")
        })
        .collect()
}

fn spawn_population(
    world: &mut TrafficWorld,
    prepared: &CorridorPopulationPrepare,
) -> Vec<VehicleHandle> {
    spawn_plans(world, prepared.initial_vehicles())
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

#[test]
fn prepare_50_100_200_are_deterministic_for_seed_zero() {
    fn fingerprint(target: usize) -> Vec<(u32, u32, u64)> {
        let (prepared, _) = prepare(target, 0);
        prepared
            .initial_vehicles()
            .iter()
            .map(|plan| {
                (
                    plan.route.raw(),
                    plan.route_edge_index,
                    plan.progress.to_bits(),
                )
            })
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
    let (prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world = TrafficWorld::install(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(MIN_TARGET_VEHICLE_COUNT).expect("fits"),
            8,
            1,
            100,
        ),
    )
    .expect("install");
    let vehicles = spawn_population(&mut world, &prepared);
    let mut controller = prepared.bind(&world, &vehicles).expect("bind");
    assert_eq!(controller.counts().running, MIN_TARGET_VEHICLE_COUNT);
    assert_eq!(controller.counts().pending, 0);

    let mut completed = 0;
    for _ in 0..8_000 {
        world.step(TickInput::new(100)).expect("step");
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
        .apply_pending(|old, input| {
            CorridorReplaceAttemptOutcome::from_replace(world.replace_completed_vehicle(old, input))
        })
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
    let (prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world = TrafficWorld::install(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(MIN_TARGET_VEHICLE_COUNT).expect("fits"),
            8,
            1,
            100,
        ),
    )
    .expect("install");
    let vehicles = spawn_population(&mut world, &prepared);
    let mut controller = prepared.bind(&world, &vehicles).expect("bind");
    for _ in 0..8_000 {
        world.step(TickInput::new(100)).expect("step");
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
        .apply_pending(|old, _input| {
            Ok::<_, std::convert::Infallible>(CorridorReplaceAttemptOutcome::Blocked(
                VehicleReplaceBlock {
                    old,
                    blocker: old,
                    blocker_ahead: true,
                    bumper_gap: 0.0,
                },
            ))
        })
        .expect("forced blocked");
    assert_eq!(report.blocked, pending);
    assert_eq!(report.replaced, 0);
    assert_eq!(controller.rng_state(), rng_before);
    assert_eq!(controller.capacities(), before_caps);
    let second = controller
        .pending_spawn_input(&world, old)
        .expect("same pending plan");
    assert_eq!(first.route(), second.route());
    assert_eq!(first.progress(), second.progress());
    assert_eq!(first.initial_speed(), second.initial_speed());
}

#[test]
fn apply_pending_host_error_restores_fifo_front() {
    let (prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world = TrafficWorld::install(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(MIN_TARGET_VEHICLE_COUNT).expect("fits"),
            8,
            1,
            100,
        ),
    )
    .expect("install");
    let vehicles = spawn_population(&mut world, &prepared);
    let mut controller = prepared.bind(&world, &vehicles).expect("bind");
    for _ in 0..8_000 {
        world.step(TickInput::new(100)).expect("step");
        if controller.consume_world(&world).expect("consume") > 0 {
            break;
        }
    }
    let pending = controller.counts().pending;
    assert!(pending > 0);
    let mut front = None;
    let error = controller
        .apply_pending(|old, input| {
            front = Some((old, input));
            Err("host-fail")
        })
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
    let mut world = TrafficWorld::install(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(MIN_TARGET_VEHICLE_COUNT).expect("fits"),
            8,
            1,
            100,
        ),
    )
    .expect("install");
    let plans = prepared.take_initial_vehicles();
    assert_eq!(plans.len(), MIN_TARGET_VEHICLE_COUNT);
    let vehicles = spawn_plans(&mut world, &plans);
    let controller = prepared.bind(&world, &vehicles).expect("bind after take");
    assert_eq!(controller.counts().running, MIN_TARGET_VEHICLE_COUNT);
    assert_eq!(controller.counts().pending, 0);
}

#[test]
fn consume_world_rejects_skipped_ticks() {
    let (prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world = TrafficWorld::install(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(MIN_TARGET_VEHICLE_COUNT).expect("fits"),
            8,
            1,
            100,
        ),
    )
    .expect("install");
    let vehicles = spawn_population(&mut world, &prepared);
    let mut controller = prepared.bind(&world, &vehicles).expect("bind");
    world.step(TickInput::new(100)).expect("first step");
    world.step(TickInput::new(100)).expect("skipped consume");
    let error = controller.consume_world(&world).expect_err("gap");
    assert!(matches!(
        error,
        CorridorPopulationError::NonMonotonicStep {
            previous: 0,
            actual: 2
        }
    ));
}

#[test]
fn consume_world_rejects_untracked_completed_vehicle() {
    let (prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world = TrafficWorld::install(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(MIN_TARGET_VEHICLE_COUNT + 1).expect("fits"),
            8,
            1,
            100,
        ),
    )
    .expect("install");
    let vehicles = spawn_population(&mut world, &prepared);
    let mut controller = prepared.bind(&world, &vehicles).expect("bind");
    let extra = spawn_near_route_end(&mut world);
    world.step(TickInput::new(100)).expect("step");
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

fn spawn_near_route_end(world: &mut TrafficWorld) -> VehicleHandle {
    let route = world
        .static_route(StaticRouteOrdinal::from_raw(0))
        .expect("static route");
    let edges = world
        .traffic()
        .relations()
        .static_route_edges(StaticRouteOrdinal::from_raw(0))
        .expect("edges")
        .to_vec();
    let last = *edges.last().expect("route has edges");
    let last_length = world.traffic().lane_lengths_meters()[last.index()];
    let speed_limit = world.traffic().lane_speed_limits_meters_per_second()[last.index()];
    let last_index = u32::try_from(edges.len() - 1).expect("index");
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            last_index,
            (last_length - 0.5).max(0.0),
            speed_limit,
        ))
        .expect("extra near end")
}

#[test]
fn spawn_input_rejects_foreign_revision() {
    const S1: &[u8] = include_bytes!(
        "../../../crates/laneflow-compiler/tests/fixtures/portable-v1/lfca-v1-full-spatial/expected.lfca"
    );
    let (prepared, _) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let input = check_canonical_network_input_v1(S1, FormatLimits::V1_HARD).expect("s1");
    let foreign = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("foreign revision");
    let world = TrafficWorld::install(foreign, WorldConfig::new(8, 4, 1, 100)).expect("install");
    assert!(
        prepared.initial_vehicles()[0].spawn_input(&world).is_err(),
        "plan must fail-closed on another NetworkRevisionId"
    );
}
