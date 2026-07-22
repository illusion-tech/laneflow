//! #172 Bevy debug Gizmos 本机手工 smoke 场景。

use std::{num::NonZeroU32, time::Duration};

use bevy::{
    color::palettes::css::DARK_SLATE_GRAY,
    prelude::*,
    time::TimeUpdateStrategy,
    window::{Window, WindowPlugin},
};
use laneflow_bevy::{
    LaneFlowDebugCenterlines, LaneFlowDebugGizmosConfig, LaneFlowDebugGizmosPlugin,
    LaneFlowFramePlacement, LaneFlowPlugin, LaneFlowSession, LaneFlowSessionConfig,
};
use laneflow_core::{
    CoreWorld, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData, LaneEdge, LaneGraph,
    ParkingRegistry, Route, SignalRegistry, Speed, VehicleProfile, VehicleProfileRegistry,
    VehicleSpawnInput,
};
use laneflow_spatial::{
    CanonicalFrameId, CanonicalPoint3F32, FramePlacementToken, SpatialEdgeInput, SpatialRegistry,
};

const FRAME_ID: &str = "smoke:debug-gizmos";

fn main() {
    let (session, centerlines) = smoke_session();
    let mut debug_config = LaneFlowDebugGizmosConfig::enabled(16, 64);
    debug_config.frame_axes_length = 8.0;
    debug_config.position_marker_size = 2.5;
    debug_config.direction_marker_length = 7.0;

    App::new()
        .insert_resource(ClearColor(DARK_SLATE_GRAY.into()))
        .insert_resource(session)
        .insert_resource(centerlines)
        .insert_resource(debug_config)
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO))
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "LaneFlow #172 Debug Gizmos Smoke".to_owned(),
                resolution: (1_280, 720).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins((LaneFlowPlugin, LaneFlowDebugGizmosPlugin))
        .add_systems(Startup, setup_scene)
        .run();
}

fn setup_scene(mut commands: Commands<'_, '_>, mut session: ResMut<'_, LaneFlowSession>) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(38.0, 32.0, 56.0).looking_at(Vec3::new(32.0, 0.0, 0.0), Vec3::Y),
    ));

    let root = commands.spawn(Transform::IDENTITY).id();
    let vehicles: Vec<_> = session
        .core()
        .vehicles()
        .map(|vehicle| vehicle.handle)
        .collect();
    for vehicle in vehicles {
        let proxy = commands.spawn((Transform::IDENTITY, ChildOf(root))).id();
        session
            .bind_vehicle_entity(vehicle, proxy)
            .expect("smoke vehicle binding");
    }
    session
        .set_frame_placement(LaneFlowFramePlacement::new(
            root,
            FramePlacementToken::new(1),
        ))
        .expect("smoke frame placement");
}

fn smoke_session() -> (LaneFlowSession, LaneFlowDebugCenterlines) {
    let graph = LaneGraph::try_new([LaneEdge::new(
        "edge",
        EdgeLength::try_new(83.777_09).expect("valid edge length"),
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
            active_vehicle("vehicle-a", profile, 12.0),
            active_vehicle("vehicle-b", profile, 32.0),
            active_vehicle("vehicle-c", profile, 56.0),
        ],
    )
    .expect("valid world");
    let points = vec![
        point(0.0, 0.0, 0.0),
        point(24.0, 0.0, 0.0),
        point(40.0, 0.0, -8.0),
        point(56.0, 0.0, 0.0),
        point(80.0, 0.0, 0.0),
    ];
    let spatial = SpatialRegistry::try_new(
        &graph,
        CanonicalFrameId::try_new(FRAME_ID).expect("valid frame"),
        [SpatialEdgeInput::new(edge, &points)],
    )
    .expect("valid spatial registry");
    let centerlines = LaneFlowDebugCenterlines::new(
        CanonicalFrameId::try_new(FRAME_ID).expect("valid frame"),
        vec![points],
    );
    let config = LaneFlowSessionConfig::new(NonZeroU32::new(8).expect("non-zero"));

    (
        LaneFlowSession::with_pose_capacity(core, spatial, config, 3),
        centerlines,
    )
}

fn active_vehicle(
    id: &'static str,
    profile: laneflow_core::VehicleProfileHandle,
    progress: f64,
) -> VehicleSpawnInput {
    VehicleSpawnInput::active(
        id,
        profile,
        "route",
        0,
        EdgeProgress::try_new(progress).expect("valid progress"),
        Speed::ZERO,
    )
}

fn point(x: f32, y: f32, z: f32) -> CanonicalPoint3F32 {
    CanonicalPoint3F32::try_new(x, y, z).expect("valid canonical point")
}
