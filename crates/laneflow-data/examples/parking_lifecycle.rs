use std::error::Error;

use laneflow_core::{
    CoreEvent, CoreWorld, EdgeProgress, LeaveParkingInput, ParkingApproachState,
    ParkingCommandEffect, ParkingSpaceState, Speed, TickInput, VehicleParkingState,
    VehicleSpawnInput, VehicleStatus,
};
use laneflow_data::from_json_str;

const PARKING_SIGNALS_FIXTURE: &str =
    include_str!("../../../examples/data/v0.6-parking-signals-baseline.laneflow.json");
const FIXED_DELTA_TIME_MS: u64 = 100;
const MAX_APPROACH_TICKS: usize = 1_000;

fn main() -> Result<(), Box<dyn Error>> {
    let loaded = from_json_str(PARKING_SIGNALS_FIXTURE)?;
    let traffic = loaded.into_initial_traffic_data();
    assert_eq!(traffic.signals().groups().len(), 1);
    assert_eq!(traffic.parking().areas().len(), 1);
    assert_eq!(traffic.parking().spaces().len(), 3);
    assert_eq!(
        traffic
            .parking()
            .spaces()
            .filter(|space| space.area_id().is_none())
            .count(),
        1,
        "canonical fixture must retain one standalone curbside space"
    );

    let profile = traffic
        .vehicle_profiles()
        .profile_handle("passenger-car")
        .expect("canonical profile");
    let mut world = CoreWorld::with_traffic_data(
        FIXED_DELTA_TIME_MS,
        traffic,
        vec![VehicleSpawnInput::active(
            "example-car",
            profile,
            "controlled-route",
            0,
            EdgeProgress::try_new(0.0)?,
            Speed::try_new(8.0)?,
        )],
    )?;
    let vehicle = world
        .vehicle_handle("example-car")
        .expect("example vehicle");
    let space = world
        .parking()
        .space_handle("lot-main-01")
        .expect("canonical area space");
    let route = world
        .route_handle("controlled-route")
        .expect("canonical route");

    println!("stage=loaded signals=1 areas=1 spaces=3 curbside=1");
    let reservation = world.reserve_parking_space(vehicle, space)?;
    assert_eq!(reservation.effect, ParkingCommandEffect::Applied);
    println!("stage=reserved vacant=2 reserved=1 occupied=0");

    let mut arrival_events = 0;
    let mut arrival_tick = None;
    for _ in 0..MAX_APPROACH_TICKS {
        let result = world.step(TickInput::new(FIXED_DELTA_TIME_MS))?;
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
            arrival_tick = Some(world.tick_index());
            break;
        }
    }
    let arrival_tick = arrival_tick.expect("vehicle must deterministically reach the entry");
    assert_eq!(arrival_events, 1, "arrival event must be one-shot");
    println!("stage=arrived tick={arrival_tick} arrival_events=1");

    let committed = world.commit_parking(vehicle, space)?;
    assert_eq!(committed.effect, ParkingCommandEffect::Applied);
    assert_eq!(
        world.vehicle(vehicle).expect("vehicle").status,
        VehicleStatus::Parked
    );
    assert_eq!(
        world.parking_snapshot().space_state(space),
        Some(ParkingSpaceState::Occupied { vehicle })
    );
    println!("stage=committed vacant=2 reserved=0 occupied=1 status=parked");

    let left = world.leave_parking(LeaveParkingInput {
        vehicle,
        space,
        route,
        route_edge_index: 1,
    })?;
    assert_eq!(left.effect, ParkingCommandEffect::Applied);
    let exit_progress = world
        .vehicle(vehicle)
        .expect("vehicle after leave")
        .edge_progress
        .value();
    assert_eq!(exit_progress, 4.0);
    println!("stage=left route=controlled-route occurrence=1 progress=4");

    world.step(TickInput::new(FIXED_DELTA_TIME_MS))?;
    let resumed = world.vehicle(vehicle).expect("resumed vehicle");
    assert_eq!(resumed.status, VehicleStatus::Active);
    assert!(resumed.edge_progress.value() > exit_progress);
    assert_eq!(
        world.parking_snapshot().space_state(space),
        Some(ParkingSpaceState::Vacant)
    );
    println!("stage=resumed status=active vacant=3 reserved=0 occupied=0");
    Ok(())
}
