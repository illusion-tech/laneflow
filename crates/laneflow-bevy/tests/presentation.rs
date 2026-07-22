use std::{num::NonZeroU32, time::Duration};

use bevy_app::App;
use bevy_ecs::{entity::Entity, hierarchy::ChildOf};
use bevy_time::{TimePlugin, TimeUpdateStrategy};
use bevy_transform::{
    TransformPlugin,
    components::{GlobalTransform, Transform},
};
use laneflow_bevy::{
    LaneFlowAdapterError, LaneFlowFramePlacement, LaneFlowPlugin, LaneFlowSession,
    LaneFlowSessionConfig,
};
use laneflow_core::{
    CoreWorld, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData, LaneEdge, LaneGraph,
    ParkingRegistry, Route, SignalRegistry, Speed, VehicleHandle, VehicleProfile,
    VehicleProfileRegistry, VehicleSpawnInput,
};
use laneflow_spatial::{
    CanonicalFrameId, CanonicalPoint3F32, FramePlacementToken, SpatialEdgeInput, SpatialRegistry,
};

struct Fixture {
    session: LaneFlowSession,
    vehicles: [VehicleHandle; 2],
}

fn profile() -> VehicleProfile {
    VehicleProfile::try_new_iidm(
        "profile",
        IidmProfileSpec {
            length: 4.5,
            desired_speed: 13.9,
            min_gap: 2.0,
            time_headway: 1.5,
            max_acceleration: 1.4,
            comfortable_deceleration: 2.0,
            emergency_deceleration: 4.0,
        },
    )
    .expect("valid profile")
}

fn fixture() -> Fixture {
    let graph = LaneGraph::try_new([LaneEdge::new(
        "edge",
        EdgeLength::try_new(100.0).expect("valid edge length"),
        laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
        std::iter::empty::<&str>(),
    )])
    .expect("valid graph");
    let edge = graph.edge_handle("edge").expect("edge handle");
    let profiles = VehicleProfileRegistry::try_new([profile()]).expect("valid profiles");
    let profile = profiles.profile_handle("profile").expect("profile handle");
    let traffic = InitialTrafficData::try_new_with_signals_and_parking(
        graph.clone(),
        [Route::try_new("route", ["edge"]).expect("valid route")],
        profiles,
        SignalRegistry::empty(),
        ParkingRegistry::empty(),
    )
    .expect("valid traffic data");
    let core = CoreWorld::with_traffic_data(
        16,
        traffic,
        vec![
            VehicleSpawnInput::active(
                "vehicle-a",
                profile,
                "route",
                0,
                EdgeProgress::try_new(10.0).expect("valid progress"),
                Speed::ZERO,
            ),
            VehicleSpawnInput::active(
                "vehicle-b",
                profile,
                "route",
                0,
                EdgeProgress::try_new(50.0).expect("valid progress"),
                Speed::ZERO,
            ),
        ],
    )
    .expect("valid world");
    let vehicles = [
        core.vehicle_handle("vehicle-a").expect("vehicle-a"),
        core.vehicle_handle("vehicle-b").expect("vehicle-b"),
    ];
    let points = [
        CanonicalPoint3F32::try_new(0.0, 0.0, 0.0).expect("valid point"),
        CanonicalPoint3F32::try_new(100.0, 0.0, 0.0).expect("valid point"),
    ];
    let spatial = SpatialRegistry::try_new(
        &graph,
        CanonicalFrameId::try_new("test:presentation").expect("valid frame"),
        [SpatialEdgeInput::new(edge, &points)],
    )
    .expect("valid spatial registry");
    let config = LaneFlowSessionConfig::new(NonZeroU32::new(1).expect("non-zero"));

    Fixture {
        session: LaneFlowSession::with_pose_capacity(core, spatial, config, vehicles.len()),
        vehicles,
    }
}

fn app_with_transform_plugin() -> App {
    let mut app = App::new();
    app.add_plugins((TimePlugin, TransformPlugin, LaneFlowPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app
}

fn close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= 8.0 * f32::EPSILON,
        "actual={actual:?}, expected={expected:?}"
    );
}

#[test]
fn mapping_is_a_partial_bijection_and_rebase_requires_a_new_token() {
    let Fixture {
        mut session,
        vehicles,
    } = fixture();
    let mut app = app_with_transform_plugin();
    let root_a = app.world_mut().spawn(Transform::IDENTITY).id();
    let root_b = app.world_mut().spawn(Transform::IDENTITY).id();
    let entity_a = app.world_mut().spawn_empty().id();
    let entity_b = app.world_mut().spawn_empty().id();

    session
        .bind_vehicle_entity(vehicles[0], entity_a)
        .expect("first binding");
    assert!(matches!(
        session.bind_vehicle_entity(vehicles[0], entity_b),
        Err(LaneFlowAdapterError::VehicleAlreadyBound { .. })
    ));
    assert!(matches!(
        session.bind_vehicle_entity(vehicles[1], entity_a),
        Err(LaneFlowAdapterError::EntityAlreadyBound { .. })
    ));
    assert_eq!(session.vehicle_entities().len(), 1);
    assert_eq!(
        session.vehicle_entities().entity(vehicles[0]),
        Some(entity_a)
    );
    assert_eq!(
        session.vehicle_entities().vehicle(entity_a),
        Some(vehicles[0])
    );

    assert_eq!(
        session
            .rebind_vehicle_entity(vehicles[0], entity_b)
            .expect("unused replacement Entity"),
        entity_a
    );
    assert_eq!(
        session.vehicle_entities().entity(vehicles[0]),
        Some(entity_b)
    );
    assert_eq!(session.unbind_entity(entity_b), Some(vehicles[0]));
    assert!(session.vehicle_entities().is_empty());

    let token_a = FramePlacementToken::new(1);
    let placement_a = LaneFlowFramePlacement::new(root_a, token_a);
    session
        .set_frame_placement(placement_a)
        .expect("initial placement");
    session
        .set_frame_placement(placement_a)
        .expect("same placement is idempotent");
    assert!(matches!(
        session.set_frame_placement(LaneFlowFramePlacement::new(root_b, token_a)),
        Err(LaneFlowAdapterError::PlacementTokenReused { .. })
    ));
    session
        .set_frame_placement(LaneFlowFramePlacement::new(
            root_b,
            FramePlacementToken::new(2),
        ))
        .expect("new token permits rebase");
}

#[test]
fn post_update_applies_mapped_pose_and_skips_unbound_records() {
    let Fixture {
        mut session,
        vehicles,
    } = fixture();
    let mut app = app_with_transform_plugin();
    let root = app.world_mut().spawn(Transform::IDENTITY).id();
    let proxy = app
        .world_mut()
        .spawn((Transform::from_xyz(-1.0, -1.0, -1.0), ChildOf(root)))
        .id();
    session
        .set_frame_placement(LaneFlowFramePlacement::new(
            root,
            FramePlacementToken::new(10),
        ))
        .expect("placement");
    session
        .bind_vehicle_entity(vehicles[0], proxy)
        .expect("binding");
    app.insert_resource(session);

    app.update();

    let transform = app
        .world()
        .get::<Transform>(proxy)
        .expect("proxy Transform");
    close(transform.translation.x, 10.0);
    close(transform.translation.y, 0.0);
    close(transform.translation.z, 0.0);
    close(transform.forward().x, 1.0);
    close(transform.forward().y, 0.0);
    close(transform.forward().z, 0.0);
    close(transform.up().x, 0.0);
    close(transform.up().y, 1.0);
    close(transform.up().z, 0.0);

    let session = app.world().resource::<LaneFlowSession>();
    let report = session.presentation_report();
    assert_eq!(report.pose_records(), 2);
    assert_eq!(report.mapped_records(), 1);
    assert_eq!(report.unbound_records(), 1);
    assert_eq!(report.applied_records(), 1);
    assert!(session.pose_batch_capacity() >= 2);
    assert!(session.pose_input_capacity() >= 2);
    assert!(session.transform_staging_capacity() >= 2);
}

#[test]
fn new_root_and_token_rebase_local_pose_before_global_propagation() {
    let Fixture {
        mut session,
        vehicles,
    } = fixture();
    let mut app = app_with_transform_plugin();
    let root_a = app.world_mut().spawn(Transform::IDENTITY).id();
    let proxy = app
        .world_mut()
        .spawn((Transform::IDENTITY, ChildOf(root_a)))
        .id();
    session
        .set_frame_placement(LaneFlowFramePlacement::new(
            root_a,
            FramePlacementToken::new(20),
        ))
        .expect("initial placement");
    session
        .bind_vehicle_entity(vehicles[0], proxy)
        .expect("binding");
    app.insert_resource(session);
    app.update();

    let root_b = app
        .world_mut()
        .spawn(Transform::from_xyz(100.0, 3.0, -7.0))
        .id();
    app.world_mut().entity_mut(proxy).insert(ChildOf(root_b));
    app.world_mut()
        .resource_mut::<LaneFlowSession>()
        .set_frame_placement(LaneFlowFramePlacement::new(
            root_b,
            FramePlacementToken::new(21),
        ))
        .expect("rebased placement");

    app.update();

    let local = app
        .world()
        .get::<Transform>(proxy)
        .expect("local Transform");
    close(local.translation.x, 10.0);
    close(local.translation.y, 0.0);
    close(local.translation.z, 0.0);
    let global = app
        .world()
        .get::<GlobalTransform>(proxy)
        .expect("propagated GlobalTransform")
        .translation();
    close(global.x, 110.0);
    close(global.y, 3.0);
    close(global.z, -7.0);
}

#[test]
fn stale_second_entity_rejects_the_whole_transform_batch() {
    let Fixture {
        mut session,
        vehicles,
    } = fixture();
    let mut app = app_with_transform_plugin();
    let root = app.world_mut().spawn(Transform::IDENTITY).id();
    let first = app
        .world_mut()
        .spawn((Transform::from_xyz(-1.0, 2.0, 3.0), ChildOf(root)))
        .id();
    let stale = app
        .world_mut()
        .spawn((Transform::from_xyz(-2.0, 4.0, 6.0), ChildOf(root)))
        .id();
    session
        .set_frame_placement(LaneFlowFramePlacement::new(
            root,
            FramePlacementToken::new(30),
        ))
        .expect("placement");
    session
        .bind_vehicle_entity(vehicles[0], first)
        .expect("first binding");
    session
        .bind_vehicle_entity(vehicles[1], stale)
        .expect("second binding");
    app.insert_resource(session);
    assert!(app.world_mut().despawn(stale));

    app.update();

    let first_transform = app
        .world()
        .get::<Transform>(first)
        .expect("first Transform");
    assert_eq!(first_transform.translation.x, -1.0);
    assert_eq!(first_transform.translation.y, 2.0);
    assert_eq!(first_transform.translation.z, 3.0);
    let session = app.world().resource::<LaneFlowSession>();
    assert!(matches!(
        session.last_error(),
        Some(LaneFlowAdapterError::StaleMappedEntity {
            input_index: 1,
            vehicle,
            entity,
        }) if *vehicle == vehicles[1] && *entity == stale
    ));
    assert_eq!(session.presentation_report().pose_records(), 2);
    assert_eq!(session.presentation_report().mapped_records(), 2);
    assert_eq!(session.presentation_report().applied_records(), 0);
}

#[test]
fn wrong_parent_is_structured_and_preserves_existing_transform() {
    let Fixture {
        mut session,
        vehicles,
    } = fixture();
    let mut app = app_with_transform_plugin();
    let expected_root = app.world_mut().spawn(Transform::IDENTITY).id();
    let actual_root = app.world_mut().spawn(Transform::IDENTITY).id();
    let proxy = app
        .world_mut()
        .spawn((Transform::from_xyz(-9.0, 8.0, 7.0), ChildOf(actual_root)))
        .id();
    session
        .set_frame_placement(LaneFlowFramePlacement::new(
            expected_root,
            FramePlacementToken::new(40),
        ))
        .expect("placement");
    session
        .bind_vehicle_entity(vehicles[0], proxy)
        .expect("binding");
    app.insert_resource(session);

    app.update();

    assert_eq!(
        app.world()
            .get::<Transform>(proxy)
            .expect("proxy Transform")
            .translation
            .x,
        -9.0
    );
    assert!(matches!(
        app.world().resource::<LaneFlowSession>().last_error(),
        Some(LaneFlowAdapterError::MappedEntityWrongParent {
            input_index: 0,
            vehicle,
            entity,
            expected_root: root,
            actual_parent: Some(parent),
        }) if *vehicle == vehicles[0]
            && *entity == proxy
            && *root == expected_root
            && *parent == actual_root
    ));
}

#[test]
fn non_unit_frame_root_is_rejected_before_any_proxy_write() {
    let Fixture {
        mut session,
        vehicles,
    } = fixture();
    let mut app = app_with_transform_plugin();
    let mut root_transform = Transform::IDENTITY;
    root_transform.scale.x = 2.0;
    let root = app.world_mut().spawn(root_transform).id();
    let proxy = app
        .world_mut()
        .spawn((Transform::from_xyz(-5.0, 0.0, 0.0), ChildOf(root)))
        .id();
    session
        .set_frame_placement(LaneFlowFramePlacement::new(
            root,
            FramePlacementToken::new(50),
        ))
        .expect("placement");
    session
        .bind_vehicle_entity(vehicles[0], proxy)
        .expect("binding");
    app.insert_resource(session);

    app.update();

    assert_eq!(
        app.world()
            .get::<Transform>(proxy)
            .expect("proxy Transform")
            .translation
            .x,
        -5.0
    );
    assert!(matches!(
        app.world().resource::<LaneFlowSession>().last_error(),
        Some(LaneFlowAdapterError::NonUnitFrameRootScale {
            root: actual_root,
            scale: [2.0, 1.0, 1.0],
        }) if *actual_root == root
    ));
}

#[test]
fn unbind_vehicle_reports_the_removed_entity() {
    let Fixture {
        mut session,
        vehicles,
    } = fixture();
    let entity = Entity::PLACEHOLDER;

    session
        .bind_vehicle_entity(vehicles[0], entity)
        .expect("binding");
    assert_eq!(session.unbind_vehicle(vehicles[0]), Some(entity));
    assert_eq!(session.unbind_vehicle(vehicles[0]), None);
}
