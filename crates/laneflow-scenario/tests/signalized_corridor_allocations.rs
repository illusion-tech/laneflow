use std::{alloc::System, collections::HashSet, hint::black_box};

use laneflow_core::{
    CoreEvent, CoreWorld, EdgeProgress, InitialTrafficData, StepResult, VehicleCompletedRouteEvent,
    VehicleHandle, VehicleReplaceInput, VehicleReplaceRecord, VehicleSpawnInput,
};
use laneflow_data::from_json_slice;
use laneflow_scenario::signalized_corridor::{
    CorridorCatalog, CorridorPopulationConfig, CorridorPopulationController,
    CorridorPopulationPrepare, CorridorReplaceAttemptOutcome,
};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const TRAFFIC: &[u8] =
    include_bytes!("../../../examples/data/v0.7-signalized-corridor.laneflow.json");
const CATALOG: &str = include_str!("../../../examples/data/v0.1-signalized-corridor.catalog.toml");

fn traffic() -> InitialTrafficData {
    from_json_slice(TRAFFIC)
        .expect("checked-in corridor Traffic must load")
        .into_initial_traffic_data()
}

fn controller_with_spare() -> (CorridorPopulationController, CoreWorld, VehicleHandle) {
    let traffic = traffic();
    let profile = traffic
        .vehicle_profiles()
        .profile_handle("passenger-car")
        .expect("checked-in profile");
    let catalog = CorridorCatalog::parse(CATALOG)
        .expect("catalog parse")
        .normalize(&traffic)
        .expect("catalog normalize");
    let config = CorridorPopulationConfig::try_new(50, 11).expect("config");
    let mut prepared =
        CorridorPopulationPrepare::prepare(config, catalog.clone(), &traffic, profile)
            .expect("prepare");
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
        .expect("spare slot");
    let spare_route = &catalog.routes()[spare_slot.route_index()];
    let mut vehicles = prepared.take_initial_vehicles();
    vehicles.push(VehicleSpawnInput::active(
        "allocation-spare",
        profile,
        spare_route.id(),
        spare_slot.route_edge_index(),
        spare_slot.edge_progress(),
        laneflow_core::Speed::ZERO,
    ));
    let world = CoreWorld::with_traffic_data(20, traffic, vehicles).expect("world");
    let spare = world
        .vehicle_handle("allocation-spare")
        .expect("spare handle");
    let controller = prepared.bind(&world).expect("bind");
    (controller, world, spare)
}

fn event_for(
    world: &CoreWorld,
    vehicle: VehicleHandle,
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

fn blocking_outcome(
    reference_world: &CoreWorld,
    expected_old: VehicleHandle,
    input: &VehicleReplaceInput,
) -> CorridorReplaceAttemptOutcome {
    let route_id = reference_world
        .route_external_id(input.route)
        .expect("route ID");
    let route_edges = reference_world
        .route_edges(input.route)
        .expect("route edges");
    let last_edge = *route_edges.last().expect("route edge");
    let route_end = EdgeProgress::try_new(
        reference_world
            .lane_graph()
            .edge_length(last_edge)
            .expect("edge length")
            .value(),
    )
    .expect("route end");
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
        CoreWorld::with_traffic_data(20, traffic(), vehicles).expect("blocking host world");
    let old = host.vehicle_handle("host-old").expect("host old");
    assert_eq!(old, expected_old);
    CorridorReplaceAttemptOutcome::from_core(
        host.replace_completed_vehicle(old, input)
            .expect("blocked transaction"),
    )
}

fn measure<T>(operation: impl FnOnce() -> T) -> (T, Stats) {
    let region = Region::new(GLOBAL);
    let output = operation();
    black_box(&output);
    let stats = black_box(region.change());
    (output, stats)
}

fn assert_zero_allocation(label: &str, stats: Stats) {
    assert_eq!(stats.allocations, 0, "{label}: allocations");
    assert_eq!(stats.reallocations, 0, "{label}: reallocations");
    assert_eq!(stats.bytes_allocated, 0, "{label}: allocated bytes");
    assert_eq!(stats.bytes_reallocated, 0, "{label}: reallocated bytes");
}

#[test]
fn steady_lifecycle_is_allocation_free_and_retained_capacity_is_bounded() {
    let (mut controller, world, spare) = controller_with_spare();
    let empty = StepResult {
        tick_index: 1,
        time_ms: 20,
        events: Vec::new(),
    };
    let (empty_result, empty_stats) = measure(|| controller.consume_step_result(&empty));
    assert_eq!(empty_result.expect("empty step"), 0);
    assert_zero_allocation("empty completion scan", empty_stats);

    let old = controller.logical_vehicle(0).expect("logical vehicle");
    let old_route = world.vehicle(old).expect("old state").route;
    let completion = event_for(&world, old, old_route, 2);
    let (completion_result, completion_stats) =
        measure(|| controller.consume_step_result(&completion));
    assert_eq!(completion_result.expect("completion"), 1);
    assert_zero_allocation("completion consume", completion_stats);

    let mut frozen_input = None;
    let warm_report = controller
        .apply_pending::<_, ()>(|attempt_old, input| {
            frozen_input = Some(input.clone());
            Ok(blocking_outcome(&world, attempt_old, input))
        })
        .expect("warm blocked retry");
    assert_eq!(warm_report.blocked, 1);
    let blocked = blocking_outcome(
        &world,
        old,
        frozen_input.as_ref().expect("frozen replacement input"),
    );
    let (blocked_result, blocked_stats) =
        measure(|| controller.apply_pending::<_, ()>(|_, _| Ok(blocked)));
    assert_eq!(blocked_result.expect("blocked retry").blocked, 1);
    assert_zero_allocation("blocked retry", blocked_stats);

    let (replace_result, replace_stats) = measure(|| {
        controller.apply_pending::<_, ()>(|attempt_old, _| {
            Ok(CorridorReplaceAttemptOutcome::Replaced(
                VehicleReplaceRecord {
                    old: attempt_old,
                    new: spare,
                },
            ))
        })
    });
    assert_eq!(replace_result.expect("replacement").replaced, 1);
    assert_zero_allocation("successful identity rotation", replace_stats);

    let (mut controller, world, spare) = controller_with_spare();
    let first = controller.logical_vehicle(0).expect("first identity");
    let baseline_capacities = controller.capacities();
    let mut current = first;
    let mut current_route = world.vehicle(first).expect("first state").route;
    let mut next = spare;
    let mut step = event_for(&world, current, current_route, 1);
    for tick in 1..=10_000 {
        step.tick_index = tick;
        step.time_ms = tick * 20;
        let CoreEvent::VehicleCompletedRoute(event) = &mut step.events[0] else {
            unreachable!()
        };
        event.tick_index = tick;
        event.vehicle = current;
        event.route = current_route;
        let route_edges = world.route_edges(current_route).expect("route edges");
        event.edge = *route_edges.last().expect("route edge");
        event.route_edge_index = route_edges.len() - 1;
        controller
            .consume_step_result(&step)
            .expect("ordered completion");
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
            .expect("identity rotation");
        current = next;
        next = if next == spare { first } else { spare };
        current_route = planned_route.expect("planned route");
    }
    let final_capacities = controller.capacities();
    assert_eq!(final_capacities.slots, baseline_capacities.slots);
    assert!(final_capacities.vehicle_slots <= baseline_capacities.vehicle_slots);
    assert_eq!(final_capacities.pending, baseline_capacities.pending);
    assert_eq!(
        final_capacities.completion_slots,
        baseline_capacities.completion_slots
    );
    assert_eq!(
        final_capacities.completion_seen,
        baseline_capacities.completion_seen
    );
    assert_eq!(controller.counts().running, 50);
    assert_eq!(controller.counts().pending, 0);
}
