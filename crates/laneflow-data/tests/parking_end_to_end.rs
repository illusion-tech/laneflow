use laneflow_core::{
    CoreError, CoreEvent, CoreWorld, EdgeProgress, LeaveParkingInput, ParkingApproachState,
    ParkingCommandEffect, ParkingCommandKind, ParkingSpaceState, Speed, TickInput,
    VehicleParkingState, VehicleSpawnInput, VehicleStatus,
};
use laneflow_data::from_json_str;
use serde_json::{Value, json};

const PARKING_SIGNALS_FIXTURE: &str =
    include_str!("../../../examples/data/v0.5-parking-signals-baseline.laneflow.json");
const EMPTY_FIXTURE: &str =
    include_str!("../../../examples/data/v0.5-empty-signals-and-parking.laneflow.json");

fn run_area_space_lifecycle() -> Vec<String> {
    let traffic = from_json_str(PARKING_SIGNALS_FIXTURE)
        .expect("canonical Parking + Signals fixture")
        .into_initial_traffic_data();
    assert_eq!(traffic.signals().groups().len(), 1);
    let lot = traffic
        .parking()
        .area_handle("lot-main")
        .expect("canonical area");
    assert_eq!(
        traffic
            .parking()
            .area_spaces(lot)
            .expect("area spaces")
            .len(),
        2
    );
    assert_eq!(
        traffic
            .parking()
            .spaces()
            .filter(|space| space.area_id().is_none())
            .map(|space| space.id())
            .collect::<Vec<_>>(),
        ["curbside-01"]
    );

    let profile = traffic
        .vehicle_profiles()
        .profile_handle("passenger-car")
        .expect("canonical profile");
    let mut world = CoreWorld::with_traffic_data(
        100,
        traffic,
        vec![VehicleSpawnInput::active(
            "V",
            profile,
            "controlled-route",
            0,
            EdgeProgress::try_new(0.0).expect("progress"),
            Speed::try_new(8.0).expect("speed"),
        )],
    )
    .expect("canonical world");
    let vehicle = world.vehicle_handle("V").expect("vehicle");
    let space = world
        .parking()
        .space_handle("lot-main-01")
        .expect("area space");
    let route = world.route_handle("controlled-route").expect("route");
    let mut trace = Vec::new();

    assert_eq!(
        world
            .reserve_parking_space(vehicle, space)
            .expect("reserve")
            .effect,
        ParkingCommandEffect::Applied
    );
    trace.push("reserved".to_owned());

    let mut arrival_events = 0;
    for _ in 0..1_000 {
        let result = world.step(TickInput::new(100)).expect("approach step");
        arrival_events += result
            .events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    CoreEvent::VehicleParkingArrivalReached(event)
                        if event.vehicle == vehicle && event.space == space
                )
            })
            .count();
        if matches!(
            world.parking_snapshot().vehicle_state(vehicle),
            Some(VehicleParkingState::Reserved {
                approach: ParkingApproachState::Arrived { .. },
                ..
            })
        ) {
            trace.push(format!("arrived@{}", world.tick_index()));
            break;
        }
    }
    assert_eq!(arrival_events, 1);
    assert_eq!(
        world.commit_parking(vehicle, space).expect("commit").effect,
        ParkingCommandEffect::Applied
    );
    assert_eq!(
        world.vehicle(vehicle).expect("parked").status,
        VehicleStatus::Parked
    );
    trace.push("committed".to_owned());

    let before_invalid_leave = world.clone();
    let invalid_leave = world
        .leave_parking(LeaveParkingInput {
            vehicle,
            space,
            route,
            route_edge_index: 0,
        })
        .expect_err("exit occurrence edge mismatch");
    assert!(matches!(
        invalid_leave,
        CoreError::ParkingRouteOccurrenceEdgeMismatch {
            command: ParkingCommandKind::Leave,
            route_edge_index: 0,
            ..
        }
    ));
    assert_eq!(world, before_invalid_leave, "failed leave must be atomic");

    assert_eq!(
        world
            .leave_parking(LeaveParkingInput {
                vehicle,
                space,
                route,
                route_edge_index: 1,
            })
            .expect("leave")
            .effect,
        ParkingCommandEffect::Applied
    );
    trace.push("left".to_owned());
    world.step(TickInput::new(100)).expect("resume step");
    assert_eq!(
        world.vehicle(vehicle).expect("resumed").status,
        VehicleStatus::Active
    );
    assert_eq!(
        world.parking_snapshot().space_state(space),
        Some(ParkingSpaceState::Vacant)
    );
    trace.push("resumed".to_owned());
    trace
}

#[test]
fn canonical_parking_signals_fixture_drives_deterministic_area_lifecycle_and_replay() {
    let first = run_area_space_lifecycle();
    let replay = run_area_space_lifecycle();
    assert_eq!(first, replay);
    assert_eq!(
        first.iter().map(String::as_str).collect::<Vec<_>>(),
        ["reserved", "arrived@36", "committed", "left", "resumed"]
    );
}

#[test]
fn production_loader_preserves_repeated_edge_occurrence_for_parking_approach() {
    let mut package: Value = serde_json::from_str(EMPTY_FIXTURE).expect("empty fixture JSON");
    package["parking"]["spaces"] = json!([{
        "id": "loop-space",
        "entry": { "edgeId": "loop", "progress": 2.0 },
        "exit": { "edgeId": "loop", "progress": 3.0 },
        "geometry": {
            "lateralOffset": -2.5,
            "headingOffsetRadians": 0.0,
            "length": 5.0,
            "width": 2.2
        }
    }]);
    let traffic = from_json_str(&package.to_string())
        .expect("repeated-edge Parking package")
        .into_initial_traffic_data();
    let profile = traffic
        .vehicle_profiles()
        .profile_handle("passenger-car")
        .expect("profile");
    let mut world = CoreWorld::with_traffic_data(
        100,
        traffic,
        vec![VehicleSpawnInput::active(
            "loop-car",
            profile,
            "loop-once",
            0,
            EdgeProgress::try_new(4.0).expect("progress"),
            Speed::try_new(5.0).expect("speed"),
        )],
    )
    .expect("repeated-edge world");
    let vehicle = world.vehicle_handle("loop-car").expect("vehicle");
    let space = world.parking().space_handle("loop-space").expect("space");
    world
        .reserve_parking_space(vehicle, space)
        .expect("reserve repeated occurrence");
    assert!(matches!(
        world.parking_snapshot().vehicle_state(vehicle),
        Some(VehicleParkingState::Reserved {
            approach: ParkingApproachState::Approaching {
                route_edge_index: 1,
                ..
            },
            ..
        })
    ));

    for _ in 0..200 {
        world.step(TickInput::new(100)).expect("repeated step");
        if matches!(
            world.parking_snapshot().vehicle_state(vehicle),
            Some(VehicleParkingState::Reserved {
                approach: ParkingApproachState::Arrived {
                    route_edge_index: 1,
                    ..
                },
                ..
            })
        ) {
            return;
        }
    }
    panic!("vehicle must arrive at the selected repeated-edge occurrence");
}

#[test]
fn production_loader_rejects_invalid_parking_area_reference_without_partial_world() {
    let mut package: Value =
        serde_json::from_str(PARKING_SIGNALS_FIXTURE).expect("canonical fixture JSON");
    package["parking"]["spaces"][0]["areaId"] = json!("missing-area");
    let error = from_json_str(&package.to_string()).expect_err("invalid area reference");
    assert!(
        format!("{error:?}").contains("UnknownParkingSpaceArea"),
        "structured Core validation must cross the production loader: {error:?}"
    );
}
