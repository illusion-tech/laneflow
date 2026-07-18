//! #140 research-only edge cap benchmark against production Core control flow.

use std::{hint::black_box, time::Duration};

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use laneflow_core::{CoreEvent, CoreWorld, TickInput};

#[allow(dead_code)]
#[path = "../tests/support/vehicle_following_scenarios.rs"]
mod scenarios;

use scenarios::{
    FIXED_DELTA_TIME_MS, SCALING_VEHICLE_COUNT, STEP_COUNT, VEHICLE_COUNT, dense_platoon_world,
    dense_platoon_world_with_edge_cap, free_flow_world, free_flow_world_with_edge_cap,
    stop_and_go_world, stop_and_go_world_with_edge_cap, transition_pressure_event_count,
    transition_pressure_world,
};

#[derive(Clone, Copy, Debug)]
struct EdgeLayout {
    name: &'static str,
    cap: Option<f64>,
}

const EDGE_LAYOUTS: [EdgeLayout; 5] = [
    EdgeLayout {
        name: "single_edge",
        cap: None,
    },
    EdgeLayout {
        name: "cap_10km",
        cap: Some(10_000.0),
    },
    EdgeLayout {
        name: "cap_4km",
        cap: Some(4_000.0),
    },
    EdgeLayout {
        name: "cap_1km",
        cap: Some(1_000.0),
    },
    EdgeLayout {
        name: "cap_100m",
        cap: Some(100.0),
    },
];

#[derive(Clone, Copy, Debug)]
enum Scenario {
    FreeFlow,
    DensePlatoon,
    StopAndGo,
}

impl Scenario {
    const ALL: [Self; 3] = [Self::FreeFlow, Self::DensePlatoon, Self::StopAndGo];

    const fn name(self) -> &'static str {
        match self {
            Self::FreeFlow => "free_flow",
            Self::DensePlatoon => "dense_platoon",
            Self::StopAndGo => "stop_and_go",
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct StepCounts {
    events: usize,
    edge_changes: usize,
}

fn build_world(vehicle_count: usize, scenario: Scenario, layout: EdgeLayout) -> CoreWorld {
    match (scenario, layout.cap) {
        (Scenario::FreeFlow, None) => free_flow_world(vehicle_count),
        (Scenario::DensePlatoon, None) => dense_platoon_world(vehicle_count),
        (Scenario::StopAndGo, None) => stop_and_go_world(vehicle_count),
        (Scenario::FreeFlow, Some(cap)) => free_flow_world_with_edge_cap(vehicle_count, cap),
        (Scenario::DensePlatoon, Some(cap)) => {
            dense_platoon_world_with_edge_cap(vehicle_count, cap)
        }
        (Scenario::StopAndGo, Some(cap)) => stop_and_go_world_with_edge_cap(vehicle_count, cap),
    }
}

fn observe_steps(world: &mut CoreWorld, step_count: usize) -> StepCounts {
    let mut counts = StepCounts::default();
    for _ in 0..step_count {
        let result = world
            .step(TickInput::new(FIXED_DELTA_TIME_MS))
            .expect("edge cap research step must succeed");
        counts.events += result.events.len();
        counts.edge_changes += result
            .events
            .iter()
            .filter(|event| matches!(event, CoreEvent::VehicleChangedEdge(_)))
            .count();
    }
    counts
}

fn benchmark_steps(world: &mut CoreWorld, step_count: usize) {
    for _ in 0..step_count {
        black_box(
            world
                .step(TickInput::new(FIXED_DELTA_TIME_MS))
                .expect("edge cap research step must succeed"),
        );
    }
}

fn topology_counts(world: &CoreWorld) -> (usize, usize, usize) {
    let edge_count = world.lane_graph().edges().len();
    let route_handles = world.routes().collect::<Vec<_>>();
    let route_occurrences = route_handles
        .iter()
        .map(|route| {
            world
                .route_edges(*route)
                .expect("registered route must retain edge occurrences")
                .len()
        })
        .sum();
    (edge_count, route_handles.len(), route_occurrences)
}

fn benchmark_step_matrix(criterion: &mut Criterion, vehicle_count: usize) {
    let group_name = if vehicle_count == VEHICLE_COUNT {
        "edge_cap_step_10k_60"
    } else {
        "edge_cap_step_100k_60_observation"
    };
    let mut group = criterion.benchmark_group(group_name);
    group.sample_size(if vehicle_count == VEHICLE_COUNT {
        20
    } else {
        10
    });
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(if vehicle_count == VEHICLE_COUNT {
        5
    } else {
        10
    }));
    group.throughput(Throughput::Elements((vehicle_count * STEP_COUNT) as u64));
    let scenarios: &[Scenario] = if vehicle_count == VEHICLE_COUNT {
        &Scenario::ALL
    } else {
        &[Scenario::DensePlatoon]
    };
    for layout in EDGE_LAYOUTS {
        for &scenario in scenarios {
            let world = build_world(vehicle_count, scenario, layout);
            let topology = topology_counts(&world);
            let observation = observe_steps(&mut world.clone(), STEP_COUNT);
            eprintln!(
                "edge_cap_observation vehicles={} layout={} scenario={} edges={} routes={} route_occurrences={} events={} edge_changes={} transitions_per_million_vehicle_steps={:.6}",
                vehicle_count,
                layout.name,
                scenario.name(),
                topology.0,
                topology.1,
                topology.2,
                observation.events,
                observation.edge_changes,
                observation.edge_changes as f64 * 1_000_000.0 / (vehicle_count * STEP_COUNT) as f64,
            );
            group.bench_with_input(
                BenchmarkId::new(
                    format!("{}/{}", layout.name, scenario.name()),
                    vehicle_count,
                ),
                &world,
                |benchmark, world| {
                    benchmark.iter_batched(
                        || world.clone(),
                        |mut world| benchmark_steps(&mut world, STEP_COUNT),
                        BatchSize::LargeInput,
                    );
                },
            );
        }
    }
    group.finish();
}

fn benchmark_construction_matrix(criterion: &mut Criterion, vehicle_count: usize) {
    let group_name = if vehicle_count == VEHICLE_COUNT {
        "edge_cap_construct_10k_dense"
    } else {
        "edge_cap_construct_100k_dense_observation"
    };
    let mut group = criterion.benchmark_group(group_name);
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(if vehicle_count == VEHICLE_COUNT {
        5
    } else {
        10
    }));
    group.throughput(Throughput::Elements(vehicle_count as u64));
    for layout in EDGE_LAYOUTS {
        group.bench_function(BenchmarkId::new(layout.name, vehicle_count), |benchmark| {
            benchmark.iter_batched(
                || (),
                |()| black_box(build_world(vehicle_count, Scenario::DensePlatoon, layout)),
                BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

fn benchmark_transition_pressure(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("edge_transition_pressure_10k_1");
    group.sample_size(20);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(5));
    group.throughput(Throughput::Elements(VEHICLE_COUNT as u64));
    for crossing_percent in [0, 1, 10, 100] {
        let world = transition_pressure_world(VEHICLE_COUNT, crossing_percent);
        let observation = observe_steps(&mut world.clone(), 1);
        assert_eq!(
            observation.edge_changes,
            transition_pressure_event_count(VEHICLE_COUNT, crossing_percent),
        );
        group.bench_with_input(
            BenchmarkId::new("crossing_percent", crossing_percent),
            &world,
            |benchmark, world| {
                benchmark.iter_batched(
                    || world.clone(),
                    |mut world| benchmark_steps(&mut world, 1),
                    BatchSize::LargeInput,
                );
            },
        );
    }
    group.finish();
}

fn benchmark_edge_cap_research(criterion: &mut Criterion) {
    benchmark_step_matrix(criterion, VEHICLE_COUNT);
    benchmark_construction_matrix(criterion, VEHICLE_COUNT);
    benchmark_transition_pressure(criterion);
    if std::env::var_os("LANEFLOW_EDGE_CAP_BENCH_100K").is_some() {
        benchmark_step_matrix(criterion, SCALING_VEHICLE_COUNT);
        benchmark_construction_matrix(criterion, SCALING_VEHICLE_COUNT);
    }
}

criterion_group!(benches, benchmark_edge_cap_research);
criterion_main!(benches);
