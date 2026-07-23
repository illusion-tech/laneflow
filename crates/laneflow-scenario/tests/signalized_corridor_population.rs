use std::collections::HashSet;

use laneflow_core::{
    CoreEvent, CoreWorld, EdgeProgress, InitialTrafficData, StepResult, VehicleCompletedRouteEvent,
    VehicleReplaceInput, VehicleReplaceRecord, VehicleSpawnInput,
};
use laneflow_data::from_json_slice;
use laneflow_scenario::signalized_corridor::{
    CorridorCatalog, CorridorPopulationConfig, CorridorPopulationCounts, CorridorPopulationError,
    CorridorPopulationPrepare, CorridorReplaceAttemptOutcome, DEFAULT_SEED,
    DEFAULT_TARGET_VEHICLE_COUNT, MAX_TARGET_VEHICLE_COUNT, MIN_TARGET_VEHICLE_COUNT,
};

const TRAFFIC: &[u8] =
    include_bytes!("../../../examples/data/v0.7-signalized-corridor.laneflow.json");
const CATALOG: &str = include_str!("../../../examples/data/v0.1-signalized-corridor.catalog.toml");

fn traffic() -> InitialTrafficData {
    from_json_slice(TRAFFIC)
        .expect("checked-in corridor Traffic must load")
        .into_initial_traffic_data()
}

fn raw_catalog() -> CorridorCatalog {
    CorridorCatalog::parse(CATALOG).expect("checked-in corridor catalog must parse")
}

fn prepare(target: usize, seed: u64) -> (CorridorPopulationPrepare, InitialTrafficData) {
    let traffic = traffic();
    let profile = traffic
        .vehicle_profiles()
        .profile_handle("passenger-car")
        .expect("checked-in profile");
    let catalog = raw_catalog()
        .normalize(&traffic)
        .expect("checked-in catalog must normalize");
    let config = CorridorPopulationConfig::try_new(target, seed).expect("test config");
    let prepared = CorridorPopulationPrepare::prepare(config, catalog, &traffic, profile)
        .expect("population prepare");
    (prepared, traffic)
}

fn fingerprint(inputs: &[VehicleSpawnInput]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for input in inputs {
        for byte in input
            .id
            .bytes()
            .chain(input.route_id.bytes())
            .chain((input.route_edge_index as u64).to_le_bytes())
            .chain(input.edge_progress.value().to_bits().to_le_bytes())
        {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

fn world_with_spare(
    target: usize,
    seed: u64,
) -> (
    CorridorPopulationPrepare,
    CoreWorld,
    laneflow_core::VehicleHandle,
) {
    let traffic = traffic();
    let profile = traffic
        .vehicle_profiles()
        .profile_handle("passenger-car")
        .expect("checked-in profile");
    let catalog = raw_catalog()
        .normalize(&traffic)
        .expect("checked-in catalog must normalize");
    let config = CorridorPopulationConfig::try_new(target, seed).expect("test config");
    let mut prepared =
        CorridorPopulationPrepare::prepare(config, catalog.clone(), &traffic, profile)
            .expect("population prepare");
    let occupied = prepared
        .initial_vehicles()
        .iter()
        .map(|input| {
            (
                input.route_id.as_str(),
                input.route_edge_index,
                input.edge_progress.value().to_bits(),
            )
        })
        .collect::<HashSet<_>>();
    let spare_slot = catalog
        .spawn_slots()
        .iter()
        .find(|slot| {
            let route = &catalog.routes()[slot.route_index()];
            !occupied.contains(&(
                route.id(),
                slot.route_edge_index(),
                slot.edge_progress().value().to_bits(),
            ))
        })
        .expect("230-slot catalog leaves a spare slot");
    let spare_route = &catalog.routes()[spare_slot.route_index()];
    let mut vehicles = prepared.take_initial_vehicles();
    vehicles.push(VehicleSpawnInput::active(
        "corridor-test-spare",
        profile,
        spare_route.id(),
        spare_slot.route_edge_index(),
        spare_slot.edge_progress(),
        laneflow_core::Speed::ZERO,
    ));
    let world =
        CoreWorld::with_traffic_data(20, traffic, vehicles).expect("corridor world with spare");
    let spare = world
        .vehicle_handle("corridor-test-spare")
        .expect("spare handle");
    (prepared, world, spare)
}

fn completion(world: &CoreWorld, vehicle: laneflow_core::VehicleHandle, tick: u64) -> StepResult {
    let state = world.vehicle(vehicle).expect("vehicle state");
    let route_edges = world.route_edges(state.route).expect("route edges");
    StepResult {
        tick_index: tick,
        time_ms: tick * 20,
        events: vec![CoreEvent::VehicleCompletedRoute(
            VehicleCompletedRouteEvent {
                tick_index: tick,
                vehicle,
                route: state.route,
                edge: *route_edges.last().expect("route edge"),
                route_edge_index: route_edges.len() - 1,
            },
        )],
    }
}

fn blocked_outcome(
    reference_world: &CoreWorld,
    expected_old: laneflow_core::VehicleHandle,
    input: &VehicleReplaceInput,
) -> CorridorReplaceAttemptOutcome {
    let route_id = reference_world
        .route_external_id(input.route)
        .expect("replacement route ID");
    let route_edges = reference_world
        .route_edges(input.route)
        .expect("replacement route edges");
    let last_edge = *route_edges.last().expect("replacement route edge");
    let route_end = EdgeProgress::try_new(
        reference_world
            .lane_graph()
            .edge_length(last_edge)
            .expect("replacement route edge length")
            .value(),
    )
    .expect("route-end progress");
    let host_traffic = traffic();
    let vehicles = vec![
        VehicleSpawnInput::completed(
            "host-old",
            input.profile,
            route_id,
            route_edges.len() - 1,
            route_end,
        ),
        VehicleSpawnInput::active(
            "host-blocker",
            input.profile,
            route_id,
            input.route_edge_index,
            input.edge_progress,
            laneflow_core::Speed::ZERO,
        ),
    ];
    let mut host =
        CoreWorld::with_traffic_data(20, host_traffic, vehicles).expect("blocking host world");
    let old = host.vehicle_handle("host-old").expect("host old");
    assert_eq!(old, expected_old, "deterministic batch identity");
    let outcome = host
        .replace_completed_vehicle(old, input)
        .expect("blocked host transaction");
    CorridorReplaceAttemptOutcome::from_core(outcome)
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
    assert!(matches!(
        CorridorPopulationConfig::try_new(MAX_TARGET_VEHICLE_COUNT + 1, 7),
        Err(CorridorPopulationError::InvalidTargetVehicleCount { .. })
    ));
}

#[test]
fn catalog_normalization_freezes_counts_and_ignores_raw_order() {
    let traffic = traffic();
    let raw = raw_catalog();
    let expected = raw.clone().normalize(&traffic).expect("canonical catalog");
    assert_eq!(expected.portals().len(), 6);
    assert_eq!(expected.routes().len(), 14);
    assert_eq!(expected.spawn_slots().len(), 230);

    let mut reordered = raw;
    reordered.portals.reverse();
    reordered.routes.reverse();
    reordered.spawn_slots.reverse();
    for portal in &mut reordered.portals {
        portal.entry_route_ids.reverse();
    }
    assert_eq!(
        reordered.normalize(&traffic).expect("reordered catalog"),
        expected
    );
}

#[test]
fn catalog_rejects_version_duplicates_dangling_routes_and_invalid_progress() {
    let traffic = traffic();

    let mut wrong_version = raw_catalog();
    wrong_version.catalog_version = "0.2".to_owned();
    assert!(matches!(
        wrong_version.normalize(&traffic),
        Err(CorridorPopulationError::UnsupportedCatalogVersion { .. })
    ));

    let mut duplicate_portal = raw_catalog();
    duplicate_portal
        .portals
        .push(duplicate_portal.portals[0].clone());
    assert!(matches!(
        duplicate_portal.normalize(&traffic),
        Err(CorridorPopulationError::DuplicatePortal { .. })
    ));

    let mut dangling_route = raw_catalog();
    dangling_route.spawn_slots[0].route_id = "missing-route".to_owned();
    assert!(matches!(
        dangling_route.normalize(&traffic),
        Err(CorridorPopulationError::UnknownSlotRoute { .. })
    ));

    let mut invalid_progress = raw_catalog();
    invalid_progress.spawn_slots[0].progress = f64::NAN;
    assert!(matches!(
        invalid_progress.normalize(&traffic),
        Err(CorridorPopulationError::InvalidSlotProgress { .. })
    ));
}

#[test]
fn initial_population_has_replay_golden_batches_for_50_100_and_200() {
    let mut fingerprints = Vec::new();
    for target in [50, 100, 200] {
        let (prepared, _) = prepare(target, 0);
        assert_eq!(prepared.initial_vehicles().len(), target);
        fingerprints.push(fingerprint(prepared.initial_vehicles()));
    }
    assert_eq!(
        fingerprints,
        [
            7_862_788_836_103_869_669,
            15_182_271_831_379_184_249,
            12_649_494_368_937_487_676,
        ]
    );
}

#[test]
fn bootstrap_submits_one_batch_and_binds_every_logical_identity() {
    for target in [50, 100, 200] {
        let (mut prepared, traffic) = prepare(target, 42);
        let first = prepared.initial_vehicles().to_vec();
        let (same, _) = prepare(target, 42);
        assert_eq!(same.initial_vehicles(), first);
        let vehicles = prepared.take_initial_vehicles();
        assert_eq!(vehicles.len(), target);
        assert!(prepared.initial_vehicles().is_empty());
        let world =
            CoreWorld::with_traffic_data(20, traffic, vehicles).expect("non-overlapping batch");
        assert_eq!(world.vehicles().count(), target);
        let controller = prepared.bind(&world).expect("tick-0 bind");
        assert_eq!(
            controller.counts(),
            CorridorPopulationCounts {
                running: target,
                pending: 0,
                target,
            }
        );
        for logical_index in 0..target {
            assert!(controller.logical_vehicle(logical_index).is_some());
        }
    }
}

#[test]
fn completion_plan_is_frozen_across_blocked_retry_and_success_rotates_identity() {
    let (prepared, world, spare) = world_with_spare(50, 9);
    let mut controller = prepared.bind(&world).expect("bind");
    let old = controller.logical_vehicle(0).expect("logical vehicle");
    let before_draw = controller.rng_state();
    assert_eq!(
        controller
            .consume_step_result(&completion(&world, old, 1))
            .expect("valid completion"),
        1
    );
    assert_ne!(controller.rng_state(), before_draw);
    assert_eq!(
        controller.counts(),
        CorridorPopulationCounts {
            running: 49,
            pending: 1,
            target: 50,
        }
    );

    let mut first_input = None;
    let blocked = controller
        .apply_pending::<_, ()>(|attempt_old, input| {
            assert_eq!(attempt_old, old);
            first_input = Some(input.clone());
            Ok(blocked_outcome(&world, old, input))
        })
        .expect("blocked is recoverable");
    assert_eq!(
        (blocked.attempted, blocked.blocked, blocked.replaced),
        (1, 1, 0)
    );
    let after_block = controller.rng_state();

    let replaced = controller
        .apply_pending::<_, ()>(|attempt_old, input| {
            assert_eq!(attempt_old, old);
            assert_eq!(Some(input), first_input.as_ref());
            Ok(CorridorReplaceAttemptOutcome::Replaced(
                VehicleReplaceRecord { old, new: spare },
            ))
        })
        .expect("replacement success");
    assert_eq!(
        (replaced.attempted, replaced.blocked, replaced.replaced),
        (1, 0, 1)
    );
    assert_eq!(controller.rng_state(), after_block);
    assert_eq!(controller.logical_vehicle(0), Some(spare));
    assert_eq!(
        controller.counts(),
        CorridorPopulationCounts {
            running: 50,
            pending: 0,
            target: 50,
        }
    );
}

#[test]
fn invalid_completion_batches_are_atomic_and_ordered_ticks_are_strict() {
    let (prepared, world, spare) = world_with_spare(50, 4);
    let mut controller = prepared.bind(&world).expect("bind");
    let old = controller.logical_vehicle(0).expect("logical vehicle");
    let valid = completion(&world, old, 1);
    let mut duplicate = valid.clone();
    duplicate.events.push(valid.events[0].clone());
    let before_rng = controller.rng_state();
    let before_counts = controller.counts();
    assert!(matches!(
        controller.consume_step_result(&duplicate),
        Err(CorridorPopulationError::DuplicateCompletionVehicle { .. })
    ));
    assert_eq!(controller.rng_state(), before_rng);
    assert_eq!(controller.counts(), before_counts);
    assert_eq!(controller.last_consumed_tick(), 0);

    let mut unknown = completion(&world, spare, 1);
    assert!(matches!(
        controller.consume_step_result(&unknown),
        Err(CorridorPopulationError::UnknownCompletionVehicle { .. })
    ));
    assert_eq!(controller.last_consumed_tick(), 0);

    let CoreEvent::VehicleCompletedRoute(event) = &mut unknown.events[0] else {
        unreachable!()
    };
    event.vehicle = old;
    event.edge = world.route_edges(event.route).expect("route edges")[0];
    event.route_edge_index = 0;
    assert!(matches!(
        controller.consume_step_result(&unknown),
        Err(CorridorPopulationError::CompletionEdgeOccurrenceMismatch { .. })
    ));
    assert_eq!(controller.last_consumed_tick(), 0);

    controller
        .consume_step_result(&valid)
        .expect("valid completion commits");
    assert!(matches!(
        controller.consume_step_result(&valid),
        Err(CorridorPopulationError::NonMonotonicStep { .. })
    ));
}

fn replay_frame_partition(frame_steps: &[usize]) -> (u64, laneflow_core::VehicleHandle) {
    let (prepared, world, spare) = world_with_spare(50, 77);
    let mut controller = prepared.bind(&world).expect("bind");
    let first = controller.logical_vehicle(0).expect("first identity");
    let mut current = first;
    let mut current_route = world.vehicle(first).expect("first state").route;
    let mut next = spare;
    let mut tick = 0_u64;
    for steps in frame_steps {
        for _ in 0..*steps {
            let mut planned_route = None;
            controller
                .apply_pending::<_, ()>(|attempt_old, input| {
                    planned_route = Some(input.route);
                    Ok(CorridorReplaceAttemptOutcome::Replaced(
                        VehicleReplaceRecord {
                            old: attempt_old,
                            new: next,
                        },
                    ))
                })
                .expect("lifecycle boundary");
            if let Some(route) = planned_route {
                current = next;
                next = if next == spare { first } else { spare };
                current_route = route;
            }
            tick += 1;
            controller
                .consume_step_result(&completion_for_route(&world, current, current_route, tick))
                .expect("fixed-step completion");
        }
    }
    let mut final_route = None;
    controller
        .apply_pending::<_, ()>(|attempt_old, input| {
            final_route = Some(input.route);
            Ok(CorridorReplaceAttemptOutcome::Replaced(
                VehicleReplaceRecord {
                    old: attempt_old,
                    new: next,
                },
            ))
        })
        .expect("final lifecycle boundary");
    assert!(final_route.is_some());
    assert_eq!(controller.counts().running, 50);
    (
        controller.rng_state(),
        controller.logical_vehicle(0).expect("final identity"),
    )
}

fn completion_for_route(
    world: &CoreWorld,
    vehicle: laneflow_core::VehicleHandle,
    route: laneflow_core::RouteHandle,
    tick: u64,
) -> StepResult {
    let route_edges = world.route_edges(route).expect("route edges");
    StepResult {
        tick_index: tick,
        time_ms: tick * 20,
        events: vec![CoreEvent::VehicleCompletedRoute(
            VehicleCompletedRouteEvent {
                tick_index: tick,
                vehicle,
                route,
                edge: *route_edges.last().expect("route edge"),
                route_edge_index: route_edges.len() - 1,
            },
        )],
    }
}

#[test]
fn outer_frame_chunking_does_not_change_fixed_step_replay() {
    let single_outer_frame = replay_frame_partition(&[100]);
    let one_step_frames = replay_frame_partition(&[1; 100]);
    let uneven_frames = replay_frame_partition(&[7, 3, 19, 1, 28, 4, 38]);
    assert_eq!(single_outer_frame, one_step_frames);
    assert_eq!(single_outer_frame, uneven_frames);
}

#[test]
fn same_tick_completion_order_is_golden_and_blocked_does_not_starve_later_plans() {
    let (prepared, world, spare) = world_with_spare(50, 23);
    let mut controller = prepared.bind(&world).expect("bind");
    let old_a = controller.logical_vehicle(0).expect("logical vehicle A");
    let old_b = controller.logical_vehicle(1).expect("logical vehicle B");
    let mut step = completion(&world, old_a, 1);
    step.events.extend(completion(&world, old_b, 1).events);
    assert_eq!(
        controller
            .consume_step_result(&step)
            .expect("ordered completion batch"),
        2
    );
    let after_draws = controller.rng_state();

    let mut attempts = Vec::new();
    let mut selected_routes = Vec::new();
    let report = controller
        .apply_pending::<_, ()>(|old, input| {
            attempts.push(old);
            selected_routes.push(
                world
                    .route_external_id(input.route)
                    .expect("selected route")
                    .to_owned(),
            );
            if old == old_a {
                Ok(blocked_outcome(&world, old, input))
            } else {
                Ok(CorridorReplaceAttemptOutcome::Replaced(
                    VehicleReplaceRecord { old, new: spare },
                ))
            }
        })
        .expect("mixed boundary");
    assert_eq!(attempts, [old_a, old_b]);
    assert_eq!(
        (report.attempted, report.blocked, report.replaced),
        (2, 1, 1)
    );
    assert_eq!(
        selected_routes,
        [
            "route-side-1-s2n-lane-1".to_owned(),
            "route-side-1-s2n-lane-0".to_owned(),
        ]
    );
    assert_eq!(controller.rng_state(), after_draws);

    let raw = raw_catalog();
    for (old, selected_route) in [(old_a, &selected_routes[0]), (old_b, &selected_routes[1])] {
        let old_route = world
            .route_external_id(world.vehicle(old).expect("old state").route)
            .expect("old route ID");
        let exit = raw
            .routes
            .iter()
            .find(|route| route.route_id == old_route)
            .expect("old route catalog")
            .exit_portal_id
            .as_str();
        let selected_entry = raw
            .routes
            .iter()
            .find(|route| route.route_id == *selected_route)
            .expect("selected route catalog")
            .entry_portal_id
            .as_str();
        assert_ne!(selected_entry, exit);
    }

    let retry = controller
        .apply_pending::<_, ()>(|old, input| {
            assert_eq!(old, old_a);
            assert_eq!(
                world.route_external_id(input.route).expect("retry route"),
                selected_routes[0]
            );
            Ok(blocked_outcome(&world, old, input))
        })
        .expect("blocked retry");
    assert_eq!((retry.attempted, retry.blocked), (1, 1));
    assert_eq!(controller.rng_state(), after_draws);
}
