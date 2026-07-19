use std::collections::HashSet;

use laneflow_core::{
    CoreEvent, CoreWorld, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData, LaneEdge,
    LaneGraph, Route, Speed, TickInput, VehicleProfile, VehicleProfileRegistry, VehicleSpawnInput,
    VehicleStatus,
};
use proptest::{
    prelude::*,
    test_runner::{Config, FileFailurePersistence, TestCaseResult, TestRunner},
};

const FIXED_DELTA_TIME_MS: u64 = 100;
const VEHICLE_LENGTH: f32 = 4.0;
const PHYSICAL_GAP_TOLERANCE_METERS: f64 = 1.0e-5;
const REGRESSION_FILE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/proptest-regressions/vehicle_following_properties.txt"
);

#[derive(Clone, Debug)]
struct PlatoonCase {
    gaps_mm: Vec<u16>,
    speeds_mm_per_second: Vec<u16>,
    tick_count: u8,
}

fn platoon_cases() -> impl Strategy<Value = PlatoonCase> {
    (2usize..=24).prop_flat_map(|vehicle_count| {
        (
            prop::collection::vec(0u16..=20_000, vehicle_count - 1),
            prop::collection::vec(0u16..=30_000, vehicle_count),
            1u8..=24,
        )
            .prop_map(|(gaps_mm, speeds_mm_per_second, tick_count)| PlatoonCase {
                gaps_mm,
                speeds_mm_per_second,
                tick_count,
            })
    })
}

fn edge_length(value: f64) -> EdgeLength {
    EdgeLength::try_from(value).expect("property edge length must be valid")
}

fn progress(value: f64) -> EdgeProgress {
    EdgeProgress::try_new(value).expect("property progress must be valid")
}

fn speed(value: f64) -> Speed {
    Speed::try_from(value).expect("property speed must be valid")
}

fn build_world(case: &PlatoonCase, reverse_input: bool) -> CoreWorld {
    let mut fronts = Vec::with_capacity(case.speeds_mm_per_second.len());
    let mut front = 10.0_f64;
    fronts.push(front);
    for gap_mm in &case.gaps_mm {
        front += f64::from(VEHICLE_LENGTH) + f64::from(*gap_mm) / 1_000.0;
        fronts.push(front);
    }

    let lane_graph = LaneGraph::try_new([LaneEdge::new(
        "property-edge",
        edge_length(front + 1_000.0),
        std::iter::empty::<&str>(),
    )])
    .expect("property graph must be valid");
    let route =
        Route::try_new("property-route", ["property-edge"]).expect("property route must be valid");
    let profiles = VehicleProfileRegistry::try_new([VehicleProfile::try_new_iidm(
        "property-profile",
        IidmProfileSpec {
            length: VEHICLE_LENGTH,
            desired_speed: 30.0,
            min_gap: 2.0,
            time_headway: 1.5,
            max_acceleration: 2.0,
            comfortable_deceleration: 2.0,
            emergency_deceleration: 8.0,
        },
    )
    .expect("property profile must be valid")])
    .expect("property profile registry must be valid");
    let profile = profiles
        .profile_handle("property-profile")
        .expect("property profile handle must exist");
    let traffic_data = InitialTrafficData::try_new(lane_graph, [route], profiles)
        .expect("property traffic data must be valid");

    let mut vehicles: Vec<_> = fronts
        .into_iter()
        .zip(&case.speeds_mm_per_second)
        .enumerate()
        .map(|(index, (front, speed_mm_per_second))| {
            VehicleSpawnInput::active(
                format!("vehicle-{index:02}"),
                profile,
                "property-route",
                0,
                progress(front),
                speed(f64::from(*speed_mm_per_second) / 1_000.0),
            )
        })
        .collect();
    if reverse_input {
        vehicles.reverse();
    }

    CoreWorld::with_traffic_data(FIXED_DELTA_TIME_MS, traffic_data, vehicles)
        .expect("generated platoon must form a valid world")
}

fn assert_world_invariants(world: &CoreWorld) -> TestCaseResult {
    let mut fronts = Vec::new();
    for vehicle in world.vehicles() {
        prop_assert_eq!(vehicle.status, VehicleStatus::Active);
        prop_assert!(vehicle.current_speed.value().is_finite());
        prop_assert!(vehicle.current_speed.value() >= 0.0);
        prop_assert!(vehicle.applied_acceleration.value().is_finite());
        prop_assert!(vehicle.edge_progress.value().is_finite());
        prop_assert!(vehicle.edge_progress.value() >= 0.0);
        fronts.push(vehicle.edge_progress.value());
    }

    fronts.sort_unstable_by(f64::total_cmp);
    for pair in fronts.windows(2) {
        prop_assert!(
            pair[1] - pair[0] + PHYSICAL_GAP_TOLERANCE_METERS >= f64::from(VEHICLE_LENGTH),
            "physical overlap: follower_front={}, leader_front={}",
            pair[0],
            pair[1]
        );
    }
    Ok(())
}

fn assert_event_invariants(events: &[CoreEvent]) -> TestCaseResult {
    let mut projected_vehicles = HashSet::new();
    for event in events {
        match event {
            CoreEvent::VehicleFollowingSafetyProjectionApplied(event) => {
                prop_assert!(
                    projected_vehicles.insert(event.vehicle),
                    "vehicle emitted more than one projection event in one tick: {:?}",
                    event.vehicle
                );
            }
            other => prop_assert!(
                false,
                "unexpected event on long single-edge route: {other:?}"
            ),
        }
    }
    Ok(())
}

fn check_platoon(case: PlatoonCase) -> TestCaseResult {
    let mut first = build_world(&case, false);
    let mut reversed = build_world(&case, true);
    prop_assert_eq!(&first, &reversed);

    for _ in 0..case.tick_count {
        let first_result = first
            .step(TickInput::new(FIXED_DELTA_TIME_MS))
            .expect("valid generated platoon step must succeed");
        let reversed_result = reversed
            .step(TickInput::new(FIXED_DELTA_TIME_MS))
            .expect("permuted generated platoon step must succeed");

        prop_assert_eq!(&first_result, &reversed_result);
        prop_assert_eq!(&first, &reversed);
        assert_world_invariants(&first)?;
        assert_event_invariants(&first_result.events)?;
    }
    Ok(())
}

#[test]
fn legal_platoons_preserve_determinism_and_longitudinal_invariants() {
    let config = Config {
        cases: 128,
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(REGRESSION_FILE))),
        ..Config::default()
    };

    TestRunner::new(config)
        .run(&platoon_cases(), check_platoon)
        .expect("generated legal platoons must preserve all invariants");
}
