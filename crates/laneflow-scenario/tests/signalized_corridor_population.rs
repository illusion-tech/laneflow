use std::sync::Arc;

use laneflow_format::{FormatLimits, check_canonical_network_input_v1};
use laneflow_runtime::{
    ReplaceError, TickInput, TrafficWorld, VehicleHandle, VehicleStatus, WorldConfig,
};
use laneflow_scenario::signalized_corridor::{
    CorridorCatalog, CorridorPopulationConfig, CorridorPopulationError, CorridorPopulationPrepare,
    CorridorReplaceAttemptOutcome, DEFAULT_SEED, DEFAULT_TARGET_VEHICLE_COUNT,
    MAX_TARGET_VEHICLE_COUNT, MIN_TARGET_VEHICLE_COUNT, PASSENGER_CAR_PROFILE_KEY, bind,
};
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

fn spawn_population(
    world: &mut TrafficWorld,
    prepared: &CorridorPopulationPrepare,
) -> Vec<VehicleHandle> {
    prepared
        .initial_vehicles()
        .iter()
        .map(|plan| {
            world
                .spawn_vehicle(plan.spawn_input(world).expect("spawn input"))
                .expect("initial spawn")
        })
        .collect()
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
    if report.blocked > 0 {
        assert_eq!(controller.rng_state(), rng_before);
    }
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
    let _ = controller
        .apply_pending(|old, input| {
            CorridorReplaceAttemptOutcome::from_replace(world.replace_completed_vehicle(old, input))
        })
        .expect("first apply");
    if controller.counts().pending == 0 {
        return;
    }
    let second = controller
        .pending_spawn_input(&world, old)
        .unwrap_or_else(|_| first);
    if world
        .vehicle(old)
        .is_some_and(|state| state.status() == VehicleStatus::Completed)
    {
        assert_eq!(first.route(), second.route());
        assert_eq!(first.progress(), second.progress());
        assert_eq!(first.initial_speed(), second.initial_speed());
        assert_eq!(controller.rng_state(), rng_before);
    }
    let _ = ReplaceError::StaleHandle;
}
