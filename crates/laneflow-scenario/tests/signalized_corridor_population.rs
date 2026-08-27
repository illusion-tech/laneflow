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
use laneflow_static_contract::VehicleProfileOrdinal;
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};

fn install_fixture(
    revision: std::sync::Arc<laneflow_static_network::SharedNetworkRevision>,
    config: laneflow_runtime::WorldConfig,
) -> Result<laneflow_runtime::TrafficWorld, laneflow_runtime::InstallError> {
    let origin = *revision.canonical_origin();
    laneflow_runtime::TrafficWorld::install(
        revision,
        config,
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
    prepared: &CorridorPopulationPrepare,
    plans: &[laneflow_scenario::signalized_corridor::CorridorVehiclePlan],
) -> (Vec<VehicleHandle>, Vec<laneflow_runtime::RouteHandle>) {
    let routes = prepared
        .install_routes(world)
        .expect("install catalog routes");
    let vehicles = plans
        .iter()
        .map(|plan| {
            world
                .spawn_vehicle(plan.spawn_input(world, &routes).expect("spawn input"))
                .expect("initial spawn")
        })
        .collect();
    (vehicles, routes)
}

fn spawn_population(
    world: &mut TrafficWorld,
    prepared: &CorridorPopulationPrepare,
) -> (Vec<VehicleHandle>, Vec<laneflow_runtime::RouteHandle>) {
    spawn_plans(world, prepared, prepared.initial_vehicles())
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
fn install_routes_rejects_short_capacity_without_leaving_routes() {
    let (prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world =
        install_fixture(revision, WorldConfig::new(8, 1, 1_024, 1, TICK_MS)).expect("install");
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
    let (prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(MIN_TARGET_VEHICLE_COUNT).expect("fits"),
            28,
            1_024,
            1,
            TICK_MS,
        ),
    )
    .expect("install");
    let (vehicles, routes) = spawn_population(&mut world, &prepared);
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
        .apply_pending(world.revision().network_revision(), |old, input| {
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
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(MIN_TARGET_VEHICLE_COUNT).expect("fits"),
            28,
            1_024,
            1,
            TICK_MS,
        ),
    )
    .expect("install");
    let (vehicles, routes) = spawn_population(&mut world, &prepared);
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
        .apply_pending(world.revision().network_revision(), |old, _input| {
            Ok::<_, std::convert::Infallible>(CorridorReplaceAttemptOutcome::Blocked(
                VehicleReplaceBlock {
                    old,
                    blocker: old,
                    blocker_ahead: true,
                    bumper_gap: 0,
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
    assert_eq!(first.progress_mm(), second.progress_mm());
    assert_eq!(first.initial_speed_mm_s(), second.initial_speed_mm_s());
}

#[test]
fn apply_pending_host_error_restores_fifo_front() {
    let (prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(MIN_TARGET_VEHICLE_COUNT).expect("fits"),
            28,
            1_024,
            1,
            TICK_MS,
        ),
    )
    .expect("install");
    let (vehicles, routes) = spawn_population(&mut world, &prepared);
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
        .apply_pending(world.revision().network_revision(), |old, input| {
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
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(MIN_TARGET_VEHICLE_COUNT).expect("fits"),
            28,
            1_024,
            1,
            TICK_MS,
        ),
    )
    .expect("install");
    let plans = prepared.take_initial_vehicles();
    assert_eq!(plans.len(), MIN_TARGET_VEHICLE_COUNT);
    let (vehicles, routes) = spawn_plans(&mut world, &prepared, &plans);
    let controller = prepared
        .bind(&mut world, &vehicles, &routes)
        .expect("bind after take");
    assert_eq!(controller.counts().running, MIN_TARGET_VEHICLE_COUNT);
    assert_eq!(controller.counts().pending, 0);
}

#[test]
fn consume_world_rejects_skipped_ticks() {
    let (prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(MIN_TARGET_VEHICLE_COUNT).expect("fits"),
            28,
            1_024,
            1,
            TICK_MS,
        ),
    )
    .expect("install");
    let (vehicles, routes) = spawn_population(&mut world, &prepared);
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

#[test]
fn consume_world_rejects_untracked_completed_vehicle() {
    let (prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(MIN_TARGET_VEHICLE_COUNT + 1).expect("fits"),
            28,
            1_024,
            1,
            TICK_MS,
        ),
    )
    .expect("install");
    let extra_route_index = prepared.initial_vehicles()[0].route_index;
    let (vehicles, routes) = spawn_population(&mut world, &prepared);
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
        "../../../crates/laneflow-compiler/tests/fixtures/portable/lfca-full-spatial/expected.lfca"
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
    install_fixture(foreign, WorldConfig::new(8, 4, 1_024, 1, 100)).expect("install")
}

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
        .spawn_vehicle(VehicleSpawnInput::new(
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
    let (prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(MIN_TARGET_VEHICLE_COUNT).expect("fits"),
            28,
            1_024,
            1,
            TICK_MS,
        ),
    )
    .expect("install");
    let (vehicles, routes) = spawn_population(&mut world, &prepared);
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
    let (prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(MIN_TARGET_VEHICLE_COUNT).expect("fits"),
            28,
            1_024,
            1,
            TICK_MS,
        ),
    )
    .expect("install");
    let (vehicles, routes) = spawn_population(&mut world, &prepared);
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
    let (prepared, revision) = prepare(MIN_TARGET_VEHICLE_COUNT, DEFAULT_SEED);
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(
            u32::try_from(MIN_TARGET_VEHICLE_COUNT).expect("fits"),
            28,
            1_024,
            1,
            TICK_MS,
        ),
    )
    .expect("install");
    let (vehicles, routes) = spawn_population(&mut world, &prepared);
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
        .apply_pending(foreign_world().revision().network_revision(), |old, _| {
            called = true;
            Ok::<_, std::convert::Infallible>(CorridorReplaceAttemptOutcome::Blocked(
                VehicleReplaceBlock {
                    old,
                    blocker: old,
                    blocker_ahead: true,
                    bumper_gap: 0,
                },
            ))
        })
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
    let (prepared, revision) = prepare(target, DEFAULT_SEED);
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(u32::try_from(target).expect("fits"), 28, 1_024, 1, TICK_MS),
    )
    .expect("install");
    let (vehicles, routes) = spawn_population(&mut world, &prepared);
    let controller = prepared.bind(&mut world, &vehicles, &routes).expect("bind");
    (world, controller)
}

fn lifecycle_tick(
    world: &mut TrafficWorld,
    controller: &mut CorridorPopulationController,
) -> usize {
    let revision = world.revision().network_revision();
    let replaced = controller
        .apply_pending(revision, |old, input| {
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
