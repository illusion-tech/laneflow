//! #173 使用仓库 campus artifacts 的最小 Bevy native reference example。

use std::{
    error::Error,
    fs, io,
    num::NonZeroU32,
    path::{Path, PathBuf},
};

use bevy::{
    color::palettes::css::{DARK_SLATE_GRAY, DEEP_SKY_BLUE, GOLD, TOMATO},
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
    window::{PrimaryWindow, Window, WindowPlugin},
};
use laneflow_bevy::{
    LaneFlowDebugCenterlines, LaneFlowDebugGizmosConfig, LaneFlowDebugGizmosPlugin,
    LaneFlowFramePlacement, LaneFlowPlugin, LaneFlowSession, LaneFlowSessionConfig,
};
use laneflow_core::{CoreWorld, EdgeProgress, Speed, VehicleSpawnInput};
use laneflow_data::{NamedArtifact, from_scenario_json_slice};
use laneflow_spatial::{
    CanonicalPoint3F32, FramePlacementToken, SpatialEdgeInput, SpatialRegistry,
};

const MANIFEST_NAME: &str = "v0.1-campus.scenario.json";
const TRAFFIC_NAME: &str = "v0.7-empty-signals-and-parking.laneflow.json";
const SPATIAL_NAME: &str = "v0.1-campus.spatial.json";
const WINDOW_TITLE: &str = "LaneFlow #173 Native Reference | G: Gizmos | F12: Screenshot";
const SCREENSHOT_PATH: &str = "laneflow-native-example.png";

fn main() -> Result<(), Box<dyn Error>> {
    let (session, centerlines) = load_campus_session()?;
    let mut debug_config = LaneFlowDebugGizmosConfig::enabled(8, 64);
    debug_config.frame_axes_length = 4.0;
    debug_config.position_marker_size = 0.8;
    debug_config.direction_marker_length = 2.5;

    App::new()
        .insert_resource(ClearColor(DARK_SLATE_GRAY.into()))
        .insert_resource(session)
        .insert_resource(centerlines)
        .insert_resource(debug_config)
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: WINDOW_TITLE.to_owned(),
                resolution: (1_280, 720).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins((LaneFlowPlugin, LaneFlowDebugGizmosPlugin))
        .add_systems(Startup, setup_scene)
        .add_systems(
            Update,
            (
                toggle_debug_gizmos,
                capture_screenshot,
                report_adapter_error,
            ),
        )
        .run();

    Ok(())
}

fn load_campus_session() -> Result<(LaneFlowSession, LaneFlowDebugCenterlines), Box<dyn Error>> {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/data");
    let manifest = read_artifact(&data_dir.join(MANIFEST_NAME))?;
    let traffic_bytes = read_artifact(&data_dir.join(TRAFFIC_NAME))?;
    let spatial_bytes = read_artifact(&data_dir.join(SPATIAL_NAME))?;

    let loaded = from_scenario_json_slice(
        &manifest,
        &[
            NamedArtifact::new(TRAFFIC_NAME, &traffic_bytes),
            NamedArtifact::new(SPATIAL_NAME, &spatial_bytes),
        ],
    )
    .map_err(|source| {
        invalid_data(format!(
            "无法加载 campus scenario 或校验引用制品（目录 {}）：{source}",
            data_dir.display()
        ))
    })?;
    let (traffic, loaded_spatial) = loaded.into_parts();
    let traffic = traffic.into_initial_traffic_data();
    let graph = traffic.lane_graph().clone();
    let frame_id = loaded_spatial.frame_id().clone();
    let centerlines = loaded_spatial
        .edges()
        .iter()
        .map(|edge| edge.points().to_vec())
        .collect();
    let spatial = SpatialRegistry::try_new(
        &graph,
        frame_id.clone(),
        loaded_spatial
            .edges()
            .iter()
            .map(|edge| SpatialEdgeInput::new(edge.edge(), edge.points())),
    )
    .map_err(|source| invalid_data(format!("无法构造 campus Spatial registry：{source}")))?;
    let profile = traffic
        .vehicle_profiles()
        .profile_handle("passenger-car")
        .ok_or_else(|| invalid_data("campus traffic 缺少 passenger-car profile"))?;
    let core = CoreWorld::with_traffic_data(
        16,
        traffic,
        vec![
            VehicleSpawnInput::active(
                "campus-main",
                profile,
                "main-route",
                0,
                EdgeProgress::try_new(1.0)?,
                Speed::try_new(2.0)?,
            ),
            VehicleSpawnInput::active(
                "campus-loop",
                profile,
                "loop-once",
                0,
                EdgeProgress::try_new(1.0)?,
                Speed::try_new(0.8)?,
            ),
        ],
    )
    .map_err(|source| invalid_data(format!("无法构造 campus Core world：{source}")))?;
    let config = LaneFlowSessionConfig::new(NonZeroU32::new(8).expect("non-zero literal"));
    let session = LaneFlowSession::with_pose_capacity(core, spatial, config, 2);

    Ok((
        session,
        LaneFlowDebugCenterlines::new(frame_id, centerlines),
    ))
}

fn read_artifact(path: &Path) -> io::Result<Vec<u8>> {
    fs::read(path).map_err(|source| {
        io::Error::new(
            source.kind(),
            format!("无法读取 LaneFlow 示例制品 {}：{source}", path.display()),
        )
    })
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn setup_scene(
    mut commands: Commands<'_, '_>,
    mut meshes: ResMut<'_, Assets<Mesh>>,
    mut materials: ResMut<'_, Assets<StandardMaterial>>,
    centerlines: Res<'_, LaneFlowDebugCenterlines>,
    mut session: ResMut<'_, LaneFlowSession>,
) {
    commands.insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 350.0,
        ..default()
    });
    commands.spawn((
        DirectionalLight {
            illuminance: 12_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(12.0, 24.0, 16.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(27.0, 27.0, 36.0).looking_at(Vec3::new(2.0, 0.0, 5.0), Vec3::Y),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(64.0, 64.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.08, 0.11, 0.13),
            perceptual_roughness: 1.0,
            ..default()
        })),
        Transform::from_xyz(2.0, -0.08, 5.0),
    ));

    let root = commands
        .spawn((
            Name::new("LaneFlow campus-local frame root"),
            Transform::from_xyz(-8.0, 0.0, -5.0).with_rotation(Quat::from_rotation_y(-0.12)),
        ))
        .id();
    spawn_roads(
        &mut commands,
        &mut meshes,
        &mut materials,
        root,
        &centerlines,
    );
    spawn_vehicles(
        &mut commands,
        &mut meshes,
        &mut materials,
        root,
        &mut session,
    );
    session
        .set_frame_placement(LaneFlowFramePlacement::new(
            root,
            FramePlacementToken::new(1),
        ))
        .expect("native example frame-root satisfies the accepted placement contract");

    info!(
        "LaneFlow campus scenario loaded; press G to toggle debug Gizmos and F12 to save {SCREENSHOT_PATH}"
    );
}

fn spawn_roads(
    commands: &mut Commands<'_, '_>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    root: Entity,
    centerlines: &LaneFlowDebugCenterlines,
) {
    let segment_mesh = meshes.add(Cuboid::default());
    let segment_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.16, 0.18, 0.2),
        perceptual_roughness: 0.95,
        ..default()
    });

    for polyline in centerlines.polylines() {
        for segment in polyline.windows(2) {
            let start = canonical_point(segment[0]);
            let end = canonical_point(segment[1]);
            let delta = end - start;
            let length = delta.length();
            if length <= f32::EPSILON {
                continue;
            }

            commands.spawn((
                Name::new("Campus centerline road segment"),
                Mesh3d(segment_mesh.clone()),
                MeshMaterial3d(segment_material.clone()),
                Transform {
                    translation: (start + end) * 0.5 - Vec3::Y * 0.04,
                    rotation: Quat::from_rotation_arc(Vec3::X, delta / length),
                    scale: Vec3::new(length, 0.06, 1.4),
                },
                ChildOf(root),
            ));
        }
    }
}

fn spawn_vehicles(
    commands: &mut Commands<'_, '_>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    root: Entity,
    session: &mut LaneFlowSession,
) {
    let body_mesh = meshes.add(Cuboid::default());
    let nose_mesh = meshes.add(Cuboid::new(0.8, 0.35, 0.5));
    let body_materials = [
        materials.add(StandardMaterial {
            base_color: TOMATO.into(),
            metallic: 0.15,
            perceptual_roughness: 0.55,
            ..default()
        }),
        materials.add(StandardMaterial {
            base_color: DEEP_SKY_BLUE.into(),
            metallic: 0.15,
            perceptual_roughness: 0.55,
            ..default()
        }),
    ];
    let nose_material = materials.add(StandardMaterial {
        base_color: GOLD.into(),
        emissive: LinearRgba::from(GOLD) * 0.4,
        ..default()
    });
    let vehicles: Vec<_> = session
        .core()
        .vehicles()
        .map(|vehicle| vehicle.handle)
        .collect();

    for (index, vehicle) in vehicles.into_iter().enumerate() {
        let proxy = commands
            .spawn((
                Name::new(format!("LaneFlow vehicle proxy {vehicle:?}")),
                Transform::IDENTITY,
                ChildOf(root),
            ))
            .id();
        commands.spawn((
            Name::new("Built-in vehicle body"),
            Mesh3d(body_mesh.clone()),
            MeshMaterial3d(body_materials[index % body_materials.len()].clone()),
            Transform::from_xyz(0.0, 0.65, 0.0).with_scale(Vec3::new(1.8, 1.2, 4.5)),
            ChildOf(proxy),
        ));
        commands.spawn((
            Name::new("Built-in vehicle forward marker"),
            Mesh3d(nose_mesh.clone()),
            MeshMaterial3d(nose_material.clone()),
            Transform::from_xyz(0.0, 0.75, -2.35),
            ChildOf(proxy),
        ));
        session
            .bind_vehicle_entity(vehicle, proxy)
            .expect("native example proxy binding is one-to-one");
    }
}

fn toggle_debug_gizmos(
    input: Res<'_, ButtonInput<KeyCode>>,
    mut config: ResMut<'_, LaneFlowDebugGizmosConfig>,
    mut windows: Query<'_, '_, &mut Window, With<PrimaryWindow>>,
) {
    if !input.just_pressed(KeyCode::KeyG) {
        return;
    }

    config.enabled = !config.enabled;
    if let Ok(mut window) = windows.single_mut() {
        window.title = format!(
            "{WINDOW_TITLE} | Gizmos: {}",
            if config.enabled { "ON" } else { "OFF" }
        );
    }
    info!(enabled = config.enabled, "LaneFlow debug Gizmos toggled");
}

fn capture_screenshot(mut commands: Commands<'_, '_>, input: Res<'_, ButtonInput<KeyCode>>) {
    if input.just_pressed(KeyCode::F12) {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(SCREENSHOT_PATH));
        info!(path = SCREENSHOT_PATH, "screenshot requested");
    }
}

fn report_adapter_error(
    session: Res<'_, LaneFlowSession>,
    mut previous: Local<'_, Option<String>>,
) {
    let current = session.last_error().map(|error| format!("{error:?}"));
    if current == *previous {
        return;
    }
    if let Some(error) = &current {
        error!(error, "LaneFlow Adapter frame failed");
    }
    *previous = current;
}

fn canonical_point(point: CanonicalPoint3F32) -> Vec3 {
    Vec3::new(point.x(), point.y(), point.z())
}
