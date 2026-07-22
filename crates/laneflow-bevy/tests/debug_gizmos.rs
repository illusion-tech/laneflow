#![cfg(feature = "debug-gizmos")]

use std::{num::NonZeroU32, time::Duration};

use bevy_app::App;
#[cfg(feature = "debug-gizmos-smoke")]
use bevy_asset::AssetApp;
use bevy_asset::AssetPlugin;
use bevy_ecs::hierarchy::ChildOf;
use bevy_gizmos::GizmoPlugin;
#[cfg(feature = "debug-gizmos-smoke")]
use bevy_mesh::{Mesh, skinning::SkinnedMeshInverseBindposes};
use bevy_time::{TimePlugin, TimeUpdateStrategy};
use bevy_transform::{TransformPlugin, components::Transform};
use laneflow_bevy::{
    LaneFlowDebugCenterlineStatus, LaneFlowDebugCenterlines, LaneFlowDebugGizmosConfig,
    LaneFlowDebugGizmosPlugin, LaneFlowDebugGizmosReport, LaneFlowDebugGizmosStatus,
    LaneFlowDebugVehicleFilter, LaneFlowFramePlacement, LaneFlowPlugin, LaneFlowSession,
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

fn fixture() -> Fixture {
    let graph = LaneGraph::try_new([LaneEdge::new(
        "edge",
        EdgeLength::try_new(100.0).expect("valid edge length"),
        laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
        std::iter::empty::<&str>(),
    )])
    .expect("valid graph");
    let edge = graph.edge_handle("edge").expect("edge handle");
    let profiles = VehicleProfileRegistry::try_new([VehicleProfile::try_new_iidm(
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
    .expect("valid profile")])
    .expect("valid profiles");
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
    let points = [point(0.0, 0.0, 0.0), point(100.0, 0.0, 0.0)];
    let spatial = SpatialRegistry::try_new(
        &graph,
        CanonicalFrameId::try_new("test:debug-gizmos").expect("valid frame"),
        [SpatialEdgeInput::new(edge, &points)],
    )
    .expect("valid spatial registry");
    let config = LaneFlowSessionConfig::new(NonZeroU32::new(1).expect("non-zero"));

    Fixture {
        session: LaneFlowSession::with_pose_capacity(core, spatial, config, vehicles.len()),
        vehicles,
    }
}

fn point(x: f32, y: f32, z: f32) -> CanonicalPoint3F32 {
    CanonicalPoint3F32::try_new(x, y, z).expect("valid canonical point")
}

fn debug_app() -> App {
    let mut app = App::new();
    app.add_plugins((TimePlugin, TransformPlugin, AssetPlugin::default()));
    // `debug-gizmos-smoke` unifies Bevy's optional mesh integration into
    // `GizmoPlugin`; provide the assets expected by that host integration while
    // keeping the test app renderer- and window-free.
    #[cfg(feature = "debug-gizmos-smoke")]
    app.init_asset::<Mesh>()
        .init_asset::<SkinnedMeshInverseBindposes>();
    app.add_plugins((GizmoPlugin, LaneFlowPlugin, LaneFlowDebugGizmosPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app
}

fn install_bound_session(app: &mut App, mut fixture: Fixture) -> [VehicleHandle; 2] {
    let root = app
        .world_mut()
        .spawn(Transform::from_xyz(100.0, 3.0, -7.0))
        .id();
    for vehicle in fixture.vehicles {
        let proxy = app
            .world_mut()
            .spawn((Transform::IDENTITY, ChildOf(root)))
            .id();
        fixture
            .session
            .bind_vehicle_entity(vehicle, proxy)
            .expect("binding");
    }
    fixture
        .session
        .set_frame_placement(LaneFlowFramePlacement::new(
            root,
            FramePlacementToken::new(1),
        ))
        .expect("placement");
    let vehicles = fixture.vehicles;
    app.insert_resource(fixture.session);
    vehicles
}

#[test]
fn plugin_without_host_gizmo_plugin_reports_missing_dependency_without_panicking() {
    let mut app = App::new();
    app.add_plugins(LaneFlowDebugGizmosPlugin);

    assert_eq!(
        app.world().resource::<LaneFlowDebugGizmosReport>().status(),
        LaneFlowDebugGizmosStatus::MissingGizmoPlugin
    );
}

#[test]
fn explicit_budget_draws_stable_first_vehicle_and_truncates_centerline() {
    let mut app = debug_app();
    let vehicles = install_bound_session(&mut app, fixture());
    app.insert_resource(LaneFlowDebugGizmosConfig::enabled(1, 1));
    app.insert_resource(LaneFlowDebugCenterlines::new(
        CanonicalFrameId::try_new("test:debug-gizmos").expect("valid frame"),
        vec![vec![
            point(0.0, 0.0, 0.0),
            point(25.0, 0.0, 0.0),
            point(100.0, 0.0, 0.0),
        ]],
    ));

    app.update();

    let report = *app.world().resource::<LaneFlowDebugGizmosReport>();
    assert_eq!(report.status(), LaneFlowDebugGizmosStatus::Drawn);
    assert_eq!(
        report.centerline_status(),
        LaneFlowDebugCenterlineStatus::Drawn
    );
    assert!(report.frame_axes_drawn());
    assert_eq!(report.eligible_vehicle_records(), 2);
    assert_eq!(report.drawn_vehicle_records(), 1);
    assert_eq!(report.truncated_vehicle_records(), 1);
    assert_eq!(report.first_drawn_vehicle(), Some(vehicles[0]));
    assert_eq!(report.last_drawn_vehicle(), Some(vehicles[0]));
    assert_eq!(report.available_centerline_segments(), 2);
    assert_eq!(report.drawn_centerline_segments(), 1);
    assert_eq!(report.truncated_centerline_segments(), 1);
    assert_eq!(report.emitted_line_segments(), 9);

    let session = app.world().resource::<LaneFlowSession>();
    assert!(session.last_error().is_none());
    assert_eq!(session.presentation_report().applied_records(), 2);
}

#[test]
fn allow_list_filter_keeps_batch_order_and_debug_switch_does_not_change_transforms() {
    let mut app = debug_app();
    let vehicles = install_bound_session(&mut app, fixture());
    let mut config = LaneFlowDebugGizmosConfig::enabled(1, 0);
    config.vehicle_filter = LaneFlowDebugVehicleFilter::AllowList(vec![vehicles[1]]);
    app.insert_resource(config);

    app.update();

    let enabled_report = *app.world().resource::<LaneFlowDebugGizmosReport>();
    assert_eq!(enabled_report.eligible_vehicle_records(), 1);
    assert_eq!(enabled_report.first_drawn_vehicle(), Some(vehicles[1]));
    let before = app
        .world()
        .resource::<LaneFlowSession>()
        .presentation_report();

    app.world_mut()
        .resource_mut::<LaneFlowDebugGizmosConfig>()
        .enabled = false;
    app.update();

    let disabled_report = *app.world().resource::<LaneFlowDebugGizmosReport>();
    assert_eq!(
        disabled_report.status(),
        LaneFlowDebugGizmosStatus::Disabled
    );
    assert_eq!(
        app.world()
            .resource::<LaneFlowSession>()
            .presentation_report(),
        before
    );
    assert!(
        app.world()
            .resource::<LaneFlowSession>()
            .last_error()
            .is_none()
    );
}

#[test]
fn invalid_config_or_centerline_frame_never_invalidates_presentation() {
    let mut app = debug_app();
    install_bound_session(&mut app, fixture());
    let mut config = LaneFlowDebugGizmosConfig::enabled(2, 2);
    config.direction_marker_length = f32::NAN;
    app.insert_resource(config);

    app.update();

    assert_eq!(
        app.world().resource::<LaneFlowDebugGizmosReport>().status(),
        LaneFlowDebugGizmosStatus::InvalidConfig
    );
    assert_eq!(
        app.world()
            .resource::<LaneFlowSession>()
            .presentation_report()
            .applied_records(),
        2
    );
    assert!(
        app.world()
            .resource::<LaneFlowSession>()
            .last_error()
            .is_none()
    );

    app.world_mut()
        .resource_mut::<LaneFlowDebugGizmosConfig>()
        .direction_marker_length = 1.0;
    app.insert_resource(LaneFlowDebugCenterlines::new(
        CanonicalFrameId::try_new("different:frame").expect("valid frame"),
        vec![vec![point(0.0, 0.0, 0.0), point(1.0, 0.0, 0.0)]],
    ));
    app.update();

    let report = *app.world().resource::<LaneFlowDebugGizmosReport>();
    assert_eq!(report.status(), LaneFlowDebugGizmosStatus::Drawn);
    assert_eq!(
        report.centerline_status(),
        LaneFlowDebugCenterlineStatus::FrameMismatch
    );
    assert_eq!(report.drawn_centerline_segments(), 0);
    assert_eq!(
        app.world()
            .resource::<LaneFlowSession>()
            .presentation_report()
            .applied_records(),
        2
    );
}

#[test]
fn failed_current_presentation_batch_never_falls_back_to_previous_validated_batch() {
    let mut app = debug_app();
    let vehicles = install_bound_session(&mut app, fixture());
    app.insert_resource(LaneFlowDebugGizmosConfig::enabled(2, 0));

    app.update();

    assert_eq!(
        app.world().resource::<LaneFlowDebugGizmosReport>().status(),
        LaneFlowDebugGizmosStatus::Drawn
    );
    let stale = app
        .world()
        .resource::<LaneFlowSession>()
        .vehicle_entities()
        .entity(vehicles[1])
        .expect("bound entity");
    assert!(app.world_mut().despawn(stale));

    app.update();

    let debug_report = *app.world().resource::<LaneFlowDebugGizmosReport>();
    assert_eq!(
        debug_report.status(),
        LaneFlowDebugGizmosStatus::NoValidatedBatch
    );
    assert_eq!(debug_report.emitted_line_segments(), 0);
    let session = app.world().resource::<LaneFlowSession>();
    assert_eq!(session.presentation_report().applied_records(), 0);
    assert!(session.last_error().is_some());
}
