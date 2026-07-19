use super::*;
use crate::{
    EdgeLength, IidmProfileSpec, LaneEdge, MovementGate, ParkingRegistry, ParkingSpace,
    ParkingSpaceGeometry, SignalAspect, SignalControlInput, SignalController, SignalGroup,
    SignalGroupState, SignalPhase, StopLine, StopLineLocation, VehicleProfile,
};

const EDGE_LENGTH_METERS: f64 = 10_000.0;
const VEHICLES_PER_EDGE: usize = 1_000;
const VEHICLE_SPACING_METERS: f64 = 8.0;
const SIGNAL_GROUPS_PER_CONTROLLER: usize = 4;
const ROUTE_HEAVY_OCCURRENCES_PER_ROUTE: usize = 1_000;
const ROUTE_HEAVY_VEHICLES_PER_ROUTE: usize = 100;

#[derive(Clone, Copy, Debug)]
enum RetainedScenario {
    VehicleHeavy,
    RouteHeavy,
    Balanced,
    SignalHeavy,
    ParkingHeavy,
    CommandSpatialHeavy,
}

impl RetainedScenario {
    const ALL: [Self; 6] = [
        Self::VehicleHeavy,
        Self::RouteHeavy,
        Self::Balanced,
        Self::SignalHeavy,
        Self::ParkingHeavy,
        Self::CommandSpatialHeavy,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::VehicleHeavy => "vehicle-heavy",
            Self::RouteHeavy => "route-heavy-10x-occurrences",
            Self::Balanced => "balanced",
            Self::SignalHeavy => "signal-heavy",
            Self::ParkingHeavy => "parking-heavy",
            Self::CommandSpatialHeavy => "command-spatial-heavy",
        }
    }

    fn build(self, scale: usize) -> CoreWorld {
        match self {
            Self::VehicleHeavy | Self::CommandSpatialHeavy => vehicle_heavy_world(scale),
            Self::RouteHeavy => route_heavy_world(scale),
            Self::Balanced => signal_world(scale, 25, 25, true),
            Self::SignalHeavy => signal_world(scale, 1, 2, false),
            Self::ParkingHeavy => parking_heavy_world(scale),
        }
    }

    const fn fills_command_speed_heap(self) -> bool {
        matches!(self, Self::CommandSpatialHeavy)
    }
}

fn profile_registry() -> (VehicleProfileRegistry, VehicleProfileHandle) {
    let profiles = VehicleProfileRegistry::try_new([VehicleProfile::try_new_iidm(
        "retained-memory-profile",
        IidmProfileSpec {
            length: 4.5,
            desired_speed: 13.9,
            min_gap: 2.0,
            time_headway: 1.5,
            max_acceleration: 1.4,
            comfortable_deceleration: 2.0,
            emergency_deceleration: 6.0,
        },
    )
    .expect("retained-memory profile must be valid")])
    .expect("retained-memory profile registry must be valid");
    let profile = profiles
        .profile_handle("retained-memory-profile")
        .expect("retained-memory profile handle must exist");
    (profiles, profile)
}

fn capped_chain_graph(edge_count: usize) -> (LaneGraph, Vec<String>) {
    let edge_ids = (0..edge_count)
        .map(|index| format!("retained-edge-{index:06}"))
        .collect::<Vec<_>>();
    let graph = LaneGraph::try_new(edge_ids.iter().enumerate().map(|(index, edge_id)| {
        LaneEdge::new(
            edge_id.clone(),
            EdgeLength::try_new(EDGE_LENGTH_METERS).expect("retained edge length must be valid"),
            edge_ids.get(index + 1).into_iter().cloned(),
        )
    }))
    .expect("retained chain graph must be valid");
    (graph, edge_ids)
}

fn vehicle_heavy_world(vehicle_count: usize) -> CoreWorld {
    let edge_count = vehicle_count.div_ceil(VEHICLES_PER_EDGE);
    let (graph, edge_ids) = capped_chain_graph(edge_count);
    let route = Route::try_new("retained-shared-route", edge_ids)
        .expect("retained shared route must be valid");
    let (profiles, profile) = profile_registry();
    let traffic = InitialTrafficData::try_new(graph, [route], profiles)
        .expect("vehicle-heavy traffic must be valid");
    let vehicles = (0..vehicle_count)
        .map(|index| {
            let route_edge_index = index / VEHICLES_PER_EDGE;
            let local_index = index % VEHICLES_PER_EDGE;
            let speed = if index % 17 == 0 {
                Speed::try_new(13.9).expect("command-heavy speed must be valid")
            } else {
                Speed::ZERO
            };
            VehicleSpawnInput::active(
                format!("retained-vehicle-{index:06}"),
                profile,
                "retained-shared-route",
                route_edge_index,
                EdgeProgress::try_new(5.0 + VEHICLE_SPACING_METERS * local_index as f64)
                    .expect("vehicle-heavy progress must be valid"),
                speed,
            )
        })
        .collect();
    CoreWorld::with_traffic_data(16, traffic, vehicles).expect("vehicle-heavy world must be valid")
}

fn route_heavy_world(vehicle_count: usize) -> CoreWorld {
    assert_eq!(vehicle_count % ROUTE_HEAVY_VEHICLES_PER_ROUTE, 0);
    let graph = LaneGraph::try_new([LaneEdge::new(
        "retained-route-loop",
        EdgeLength::try_new(EDGE_LENGTH_METERS).expect("route-heavy edge length must be valid"),
        ["retained-route-loop"],
    )])
    .expect("route-heavy graph must be valid");
    let route_count = vehicle_count / ROUTE_HEAVY_VEHICLES_PER_ROUTE;
    let routes = (0..route_count).map(|route_index| {
        Route::try_new(
            format!("retained-route-{route_index:06}"),
            std::iter::repeat_n("retained-route-loop", ROUTE_HEAVY_OCCURRENCES_PER_ROUTE),
        )
        .expect("route-heavy route must be valid")
    });
    let (profiles, profile) = profile_registry();
    let traffic = InitialTrafficData::try_new(graph, routes, profiles)
        .expect("route-heavy traffic must be valid");
    let vehicles = (0..vehicle_count)
        .map(|index| {
            let route_index = index / ROUTE_HEAVY_VEHICLES_PER_ROUTE;
            VehicleSpawnInput::completed(
                format!("retained-route-vehicle-{index:06}"),
                profile,
                format!("retained-route-{route_index:06}"),
                ROUTE_HEAVY_OCCURRENCES_PER_ROUTE - 1,
                EdgeProgress::try_new(EDGE_LENGTH_METERS)
                    .expect("route-heavy completion progress must be valid"),
            )
        })
        .collect();
    CoreWorld::with_traffic_data(16, traffic, vehicles).expect("route-heavy world must be valid")
}

fn signal_world(
    vehicle_count: usize,
    vehicles_per_route: usize,
    route_occurrences: usize,
    with_parking: bool,
) -> CoreWorld {
    assert_eq!(vehicle_count % vehicles_per_route, 0);
    let route_count = vehicle_count / vehicles_per_route;
    assert_eq!(route_count % SIGNAL_GROUPS_PER_CONTROLLER, 0);
    assert!(route_occurrences >= 2);

    let mut edges = Vec::with_capacity(route_count * 2);
    let mut routes = Vec::with_capacity(route_count);
    let mut stop_lines = Vec::with_capacity(route_count);
    let mut groups = Vec::with_capacity(route_count);
    let mut gates = Vec::with_capacity(route_count);
    let mut parking_spaces = Vec::with_capacity(usize::from(with_parking) * route_count);
    for route_index in 0..route_count {
        let entry = format!("retained-signal-entry-{route_index:06}");
        let exit = format!("retained-signal-exit-{route_index:06}");
        let route_id = format!("retained-signal-route-{route_index:06}");
        let stop_line = format!("retained-signal-stop-{route_index:06}");
        let group = format!("retained-signal-group-{route_index:06}");
        edges.push(LaneEdge::new(
            entry.clone(),
            EdgeLength::try_new(200.0).expect("signal entry length must be valid"),
            [exit.clone()],
        ));
        edges.push(LaneEdge::new(
            exit.clone(),
            EdgeLength::try_new(200.0).expect("signal exit length must be valid"),
            [exit.clone()],
        ));
        routes.push(
            Route::try_new(
                route_id,
                std::iter::once(entry.clone())
                    .chain(std::iter::repeat_n(exit.clone(), route_occurrences - 1)),
            )
            .expect("signal retained route must be valid"),
        );
        stop_lines.push(StopLine::new(
            stop_line.clone(),
            entry.clone(),
            StopLineLocation::EdgeEnd,
        ));
        groups.push(SignalGroup::new(group.clone()));
        gates.push(MovementGate::new(
            entry,
            exit.clone(),
            stop_line,
            SignalControlInput::Group(group),
        ));
        if with_parking {
            parking_spaces.push(ParkingSpace::new(
                format!("retained-balanced-space-{route_index:06}"),
                None,
                exit.clone(),
                20.0,
                exit,
                40.0,
                ParkingSpaceGeometry::new(-3.0, 0.0, 5.0, 2.4),
            ));
        }
    }

    let graph = LaneGraph::try_new(edges).expect("signal retained graph must be valid");
    let controllers = (0..route_count / SIGNAL_GROUPS_PER_CONTROLLER).map(|controller_index| {
        let first_group = controller_index * SIGNAL_GROUPS_PER_CONTROLLER;
        let group_ids = (first_group..first_group + SIGNAL_GROUPS_PER_CONTROLLER)
            .map(|index| format!("retained-signal-group-{index:06}"))
            .collect::<Vec<_>>();
        SignalController::new_fixed_time(
            format!("retained-signal-controller-{controller_index:06}"),
            0,
            group_ids.clone(),
            [SignalPhase::new(
                "red",
                60_000,
                group_ids
                    .into_iter()
                    .map(|group| SignalGroupState::new(group, SignalAspect::Red)),
            )],
        )
    });
    let signals = SignalRegistry::try_new(&graph, stop_lines, groups, controllers, gates)
        .expect("signal retained registry must be valid");
    let parking = ParkingRegistry::try_new(&graph, [], parking_spaces)
        .expect("balanced Parking registry must be valid");
    let (profiles, profile) = profile_registry();
    let traffic = InitialTrafficData::try_new_with_signals_and_parking(
        graph, routes, profiles, signals, parking,
    )
    .expect("signal retained traffic must be valid");
    let vehicles = (0..route_count)
        .flat_map(|route_index| {
            (0..vehicles_per_route).map(move |vehicle_index| {
                VehicleSpawnInput::active(
                    format!("retained-signal-vehicle-{route_index:06}-{vehicle_index:02}"),
                    profile,
                    format!("retained-signal-route-{route_index:06}"),
                    0,
                    EdgeProgress::try_new(20.0 + 6.5 * vehicle_index as f64)
                        .expect("signal retained progress must be valid"),
                    Speed::ZERO,
                )
            })
        })
        .collect();
    CoreWorld::with_traffic_data(16, traffic, vehicles)
        .expect("signal retained world must be valid")
}

fn parking_heavy_world(vehicle_count: usize) -> CoreWorld {
    let edge_count = vehicle_count.div_ceil(VEHICLES_PER_EDGE);
    let (graph, edge_ids) = capped_chain_graph(edge_count);
    let route = Route::try_new("retained-parking-route", edge_ids.clone())
        .expect("Parking-heavy route must be valid");
    let parking = ParkingRegistry::try_new(
        &graph,
        [],
        (0..vehicle_count).map(|index| {
            let edge = &edge_ids[index / VEHICLES_PER_EDGE];
            let local_index = index % VEHICLES_PER_EDGE;
            let entry_progress = 1.0 + VEHICLE_SPACING_METERS * local_index as f64;
            ParkingSpace::new(
                format!("retained-parking-space-{index:06}"),
                None,
                edge.clone(),
                entry_progress,
                edge.clone(),
                entry_progress + 1.0,
                ParkingSpaceGeometry::new(-3.0, 0.0, 5.0, 2.4),
            )
        }),
    )
    .expect("Parking-heavy registry must be valid");
    let (profiles, profile) = profile_registry();
    let traffic = InitialTrafficData::try_new_with_signals_and_parking(
        graph,
        [route],
        profiles,
        SignalRegistry::empty(),
        parking,
    )
    .expect("Parking-heavy traffic must be valid");
    let vehicles = (0..vehicle_count)
        .map(|index| {
            let route_edge_index = index / VEHICLES_PER_EDGE;
            let local_index = index % VEHICLES_PER_EDGE;
            VehicleSpawnInput::active(
                format!("retained-parking-vehicle-{index:06}"),
                profile,
                "retained-parking-route",
                route_edge_index,
                EdgeProgress::try_new(5.0 + VEHICLE_SPACING_METERS * local_index as f64)
                    .expect("Parking-heavy progress must be valid"),
                Speed::ZERO,
            )
        })
        .collect();
    CoreWorld::with_traffic_data(16, traffic, vehicles).expect("Parking-heavy world must be valid")
}

fn assert_complete_sum(stats: LifecycleRetainedStats) {
    let heap = stats.lane_graph_bytes
        + stats.vehicle_profile_registry_bytes
        + stats.signal_registry_bytes
        + stats.signal_runtime_state_bytes
        + stats.signal_runtime_scratch_bytes
        + stats.route_bytes
        + stats.vehicle_bytes
        + stats.resolver_bytes
        + stats.free_list_bytes
        + stats.vehicle_order_bytes
        + stats.candidate_state_bytes
        + stats.parking_registry_runtime_bytes
        + stats.occupancy_scratch_bytes
        + stats.longitudinal_scratch_bytes
        + stats.command_spatial_bytes;
    assert_eq!(stats.owned_heap_bytes, heap);
    assert_eq!(
        stats.complete_accounted_bytes,
        stats.world_inline_bytes + heap
    );
}

fn print_snapshot(
    scenario: RetainedScenario,
    scale: usize,
    phase: &str,
    stats: LifecycleRetainedStats,
) {
    eprintln!(
        "retained_matrix scenario={} scale={} phase={} live={} route_occurrences={} complete_bytes={} owned_heap_bytes={} world_inline_bytes={} lane_graph_bytes={} profile_registry_bytes={} signal_registry_bytes={} signal_state_bytes={} signal_scratch_bytes={} route_bytes={} route_distance_bytes={} route_reference_bytes={} vehicle_bytes={} resolver_bytes={} free_list_bytes={} vehicle_order_bytes={} candidate_state_bytes={} parking_bytes={} occupancy_scratch_bytes={} longitudinal_scratch_bytes={} command_spatial_bytes={}",
        scenario.name(),
        scale,
        phase,
        stats.live_vehicles,
        stats.route_occurrences,
        stats.complete_accounted_bytes,
        stats.owned_heap_bytes,
        stats.world_inline_bytes,
        stats.lane_graph_bytes,
        stats.vehicle_profile_registry_bytes,
        stats.signal_registry_bytes,
        stats.signal_runtime_state_bytes,
        stats.signal_runtime_scratch_bytes,
        stats.route_bytes,
        stats.route_distance_bytes,
        stats.route_reference_bytes,
        stats.vehicle_bytes,
        stats.resolver_bytes,
        stats.free_list_bytes,
        stats.vehicle_order_bytes,
        stats.candidate_state_bytes,
        stats.parking_registry_runtime_bytes,
        stats.occupancy_scratch_bytes,
        stats.longitudinal_scratch_bytes,
        stats.command_spatial_bytes,
    );
}

fn measure_scenario(scenario: RetainedScenario, scale: usize) {
    let mut world = scenario.build(scale);
    let current = world.lifecycle_retained_stats();
    assert_eq!(current.live_vehicles, scale);
    assert_complete_sum(current);
    match scenario {
        RetainedScenario::VehicleHeavy | RetainedScenario::CommandSpatialHeavy => {
            assert!(current.route_occurrences < scale);
        }
        RetainedScenario::RouteHeavy => {
            assert_eq!(current.route_occurrences, scale * 10);
        }
        RetainedScenario::Balanced => {
            assert_eq!(current.route_occurrences, scale);
            assert!(current.signal_registry_bytes > 0);
            assert!(current.parking_registry_runtime_bytes > 0);
        }
        RetainedScenario::SignalHeavy => {
            assert_eq!(current.route_occurrences, scale * 2);
            assert!(current.signal_registry_bytes > 0);
            assert!(current.signal_runtime_state_bytes > 0);
        }
        RetainedScenario::ParkingHeavy => {
            assert!(current.parking_registry_runtime_bytes > current.route_bytes);
        }
    }

    world
        .step(TickInput::new(world.fixed_delta_time_ms()))
        .expect("retained matrix warm-up step must succeed");
    if scenario.fills_command_speed_heap() {
        let active_speeds = world
            .vehicles()
            .map(|state| (state.handle, state.current_speed.value()))
            .collect::<Vec<_>>();
        let max_speed = active_speeds
            .iter()
            .map(|(_, speed)| *speed)
            .fold(0.0, f64::max);
        world
            .command_spatial_index
            .prepare_speed_removal(max_speed, active_speeds);
    }
    let mut high_water = world.lifecycle_retained_stats();
    assert_eq!(high_water.live_vehicles, scale);
    assert_complete_sum(high_water);
    assert!(high_water.complete_accounted_bytes >= current.complete_accounted_bytes);
    assert!(high_water.signal_runtime_scratch_bytes >= current.signal_runtime_scratch_bytes);

    let handles = world
        .vehicles()
        .map(|snapshot| snapshot.handle)
        .collect::<Vec<_>>();
    let cleanup_checkpoints = [1, scale / 4, scale / 2, scale.saturating_mul(3) / 4, scale];
    for (index, handle) in handles.into_iter().enumerate() {
        world
            .despawn_vehicle(handle)
            .expect("retained matrix cleanup must despawn every vehicle");
        if cleanup_checkpoints.contains(&(index + 1)) {
            let checkpoint = world.lifecycle_retained_stats();
            assert_complete_sum(checkpoint);
            if checkpoint.complete_accounted_bytes > high_water.complete_accounted_bytes {
                high_water = checkpoint;
            }
        }
    }
    let cleaned = world.lifecycle_retained_stats();
    assert_eq!(cleaned.live_vehicles, 0);
    assert_complete_sum(cleaned);
    assert!(cleaned.complete_accounted_bytes <= high_water.complete_accounted_bytes);
    if scenario.fills_command_speed_heap() {
        assert!(high_water.command_spatial_bytes > current.command_spatial_bytes);
    }

    print_snapshot(scenario, scale, "current", current);
    print_snapshot(scenario, scale, "high-water", high_water);
    print_snapshot(scenario, scale, "cleaned", cleaned);
}

fn run_matrix(scale: usize) {
    for scenario in RetainedScenario::ALL {
        measure_scenario(scenario, scale);
    }
}

#[test]
fn complete_retained_memory_matrix_smoke() {
    run_matrix(100);
}

#[test]
#[ignore = "10k complete retained-memory matrix is an explicit #127 research measurement"]
fn complete_retained_memory_matrix_10k() {
    run_matrix(10_000);
}

#[test]
#[ignore = "100k complete retained-memory matrix is an explicit #127 G3 measurement"]
fn complete_retained_memory_matrix_100k() {
    run_matrix(100_000);
}
