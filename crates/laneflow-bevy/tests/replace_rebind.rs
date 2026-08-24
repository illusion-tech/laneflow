use std::{num::NonZeroU32, sync::Arc, time::Duration};

use bevy_app::App;
use bevy_time::{TimePlugin, TimeUpdateStrategy};
use bevy_transform::{TransformPlugin, components::Transform};
use laneflow_bevy::{
    LaneFlowPlugin, LaneFlowSession, LaneFlowSessionConfig, LaneFlowVehicleReplaceOutcome,
    replace_completed_vehicle,
};
use laneflow_format::{FormatLimits, check_canonical_network_input_v1};
use laneflow_runtime::{TickInput, TrafficWorld, VehicleSpawnInput, VehicleStatus, WorldConfig};
use laneflow_spatial::SpatialSession;
use laneflow_static_contract::{StaticRouteOrdinal, VehicleProfileOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
    build_shared_network_revision,
};

const FULL_SPATIAL: &[u8] = include_bytes!(
    "../../laneflow-compiler/tests/fixtures/portable-v1/lfca-v1-full-spatial/expected.lfca"
);

fn revision() -> Arc<laneflow_static_network::SharedNetworkRevision> {
    let input = check_canonical_network_input_v1(FULL_SPATIAL, FormatLimits::V1_HARD)
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

fn drive_to_completed(world: &mut TrafficWorld) -> laneflow_runtime::VehicleHandle {
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
    let vehicle = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            last_index,
            (last_length - 0.5).max(0.0),
            speed_limit,
        ))
        .expect("spawn near end");
    for _ in 0..8 {
        world.step(TickInput::new(100)).expect("step");
        if world
            .vehicle(vehicle)
            .is_some_and(|state| state.status() == VehicleStatus::Completed)
        {
            break;
        }
    }
    vehicle
}

#[test]
fn replace_reuses_bound_entity_and_keeps_transform_on_blocked() {
    let mut world =
        TrafficWorld::install(revision(), WorldConfig::new(8, 4, 1, 100)).expect("install");
    let old = drive_to_completed(&mut world);
    let route = world
        .static_route(StaticRouteOrdinal::from_raw(0))
        .expect("route");
    let spatial = SpatialSession::bind(world.revision())
        .expect("bind")
        .expect("spatial");
    let session = LaneFlowSession::new(
        world,
        Some(spatial),
        LaneFlowSessionConfig::new(NonZeroU32::new(8).expect("non-zero")),
    )
    .expect("session");

    let mut app = App::new();
    app.add_plugins((TimePlugin, TransformPlugin, LaneFlowPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        16,
    )));
    app.insert_resource(session);
    let entity = app
        .world_mut()
        .spawn(Transform::from_xyz(1.0, 2.0, 3.0))
        .id();
    app.world_mut()
        .resource_mut::<LaneFlowSession>()
        .bind_vehicle_entity(old, entity)
        .expect("bind");

    app.world_mut()
        .resource_mut::<LaneFlowSession>()
        .world_mut()
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            0.0,
            0.0,
        ))
        .expect("blocker");
    let before = *app.world().get::<Transform>(entity).expect("transform");
    let outcome = replace_completed_vehicle(
        app.world_mut(),
        old,
        VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 0.0, 0.0),
    )
    .expect("blocked is success-path for adapter");
    assert!(matches!(outcome, LaneFlowVehicleReplaceOutcome::Blocked(_)));
    assert_eq!(
        app.world()
            .resource::<LaneFlowSession>()
            .vehicle_entity(old),
        Some(entity)
    );
    assert_eq!(
        *app.world().get::<Transform>(entity).expect("transform"),
        before
    );

    let outcome = replace_completed_vehicle(
        app.world_mut(),
        old,
        VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 8.0, 0.0),
    )
    .expect("replace");
    let LaneFlowVehicleReplaceOutcome::Replaced(record) = outcome else {
        panic!("expected replaced");
    };
    assert_eq!(record.entity, Some(entity));
    assert_ne!(record.new, old);
    assert!(
        app.world()
            .resource::<LaneFlowSession>()
            .vehicle_entity(old)
            .is_none()
    );
    assert_eq!(
        app.world()
            .resource::<LaneFlowSession>()
            .vehicle_entity(record.new),
        Some(entity)
    );
    assert_eq!(
        *app.world().get::<Transform>(entity).expect("transform"),
        before,
        "presentation updates on a later outer frame"
    );
}

#[test]
fn unbound_replace_stays_unbound() {
    let mut world =
        TrafficWorld::install(revision(), WorldConfig::new(8, 4, 1, 100)).expect("install");
    let old = drive_to_completed(&mut world);
    let route = world
        .static_route(StaticRouteOrdinal::from_raw(0))
        .expect("route");
    let session = LaneFlowSession::new(
        world,
        None,
        LaneFlowSessionConfig::new(NonZeroU32::new(8).expect("non-zero")),
    )
    .expect("session");
    let mut app = App::new();
    app.add_plugins((TimePlugin, LaneFlowPlugin));
    app.insert_resource(session);
    let outcome = replace_completed_vehicle(
        app.world_mut(),
        old,
        VehicleSpawnInput::new(VehicleProfileOrdinal::from_raw(0), route, 0, 0.0, 0.0),
    )
    .expect("replace");
    let LaneFlowVehicleReplaceOutcome::Replaced(record) = outcome else {
        panic!("expected replaced");
    };
    assert_eq!(record.entity, None);
    assert!(
        app.world()
            .resource::<LaneFlowSession>()
            .vehicle_entity(record.new)
            .is_none()
    );
}
