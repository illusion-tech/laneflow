//! 复杂路口 Bevy 调试示例（#285 阶段二 §4）：检入 catalog 0.1 + LFCA 的四车
//! 确定性场景 + 默认关闭的 F2 调试 overlay（mesh/文本，无 Gizmos）。
//!
//! overlay 全部只读 `LaneFlowSession` 并写独立 ECS 实体，不触碰 Runtime 状态；
//! 开/关 overlay 在相同输入下的 Runtime 摘要一致性由无窗口 smoke 对拍把关。
//! GUI 不进 CI（bevy-reference-adapter §9）。

#[path = "support/junction_debug_scene.rs"]
mod junction_debug_scene;

use std::{collections::HashMap, error::Error, fmt::Write as _, path::PathBuf};

use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::{asset::AssetApp, camera::ScalingMode, mesh::Indices, prelude::*};
use laneflow_bevy::{
    LaneFlowCommittedPoseBatch, LaneFlowFixed, LaneFlowFixedSet, LaneFlowPlugin, LaneFlowSession,
};
use laneflow_runtime::{VehicleHandle, VehicleStatus, WorldGeneration};
use laneflow_spatial::{CanonicalPoseF32, FramePlacementToken};
use laneflow_static_contract::{
    ConflictZoneOrdinal, EntityKind, NetworkRevisionId, SignalAspect, SignalGroupOrdinal,
    StopLineOrdinal, WaitingZoneOrdinal,
};
use laneflow_static_network::{CanonicalPoint, SharedNetworkRevision};

/// 车道宽度（米），与场景合同 `examples/config/v0.1-complex-junction.toml` 的
/// `lane_width_meters` 一致。
const LANE_WIDTH_METERS: f32 = 3.5;
/// 车辆 box 宽度近似（米）；车型线格式只有车长。
const VEHICLE_WIDTH_METERS: f32 = 1.8;
const VEHICLE_HEIGHT_METERS: f32 = 1.5;
/// 地平面边长（米）；覆盖环路最外缘（±182 m）并留透视余量。
const GROUND_SIZE_METERS: f32 = 600.0;
/// 道路/标线/铺装的错层高度（米），避免共面 z-fight。
const ROAD_Y: f32 = 0.03;
const MARKING_Y: f32 = 0.06;
const PATCH_Y: f32 = 0.015;
/// 人行道顶面高度与宽度（米）。
const SIDEWALK_Y: f32 = 0.10;
const SIDEWALK_WIDTH_METERS: f32 = 3.0;
/// 斑马线：条纹长（沿车道方向）/条纹宽/净间隔/与停止线的净距（米）。
const CROSSWALK_LENGTH_METERS: f32 = 3.0;
const CROSSWALK_STRIPE_WIDTH_METERS: f32 = 0.45;
const CROSSWALK_STRIPE_GAP_METERS: f32 = 0.45;
const CROSSWALK_SETBACK_METERS: f32 = 0.3;
/// 车道导向箭头的施画位置：进口车道终点（停止线）前（米）。
const ARROW_SETBACK_METERS: f32 = 8.0;
/// 同向车道分界虚线：3 m 画 / 6 m 空（米）。
const DASH_ON_METERS: f32 = 3.0;
const DASH_OFF_METERS: f32 = 6.0;
/// 车道线线宽与对向双黄的分缝间距（米）。
const LINE_WIDTH_METERS: f32 = 0.12;
const DOUBLE_YELLOW_GAP_METERS: f32 = 0.15;
/// 停止线带：沿车道方向 0.4 m、横跨整条车道宽（米）。
const STOP_LINE_BAND_METERS: f32 = 0.4;
/// screenshot 模式的出图帧（等 Startup、渲染管线与首帧呈现稳定后落盘）。
const SCREENSHOT_FRAME: u32 = 150;
/// screenshot 模式窗口分辨率（宽, 高）。
const SCREENSHOT_RESOLUTION: (u32, u32) = (1_600, 1_000);

fn main() -> Result<(), Box<dyn Error>> {
    let scene = junction_debug_scene::build()?;
    let cli = CliArgs::parse();
    let mut app = App::new();
    app.add_plugins(LaneFlowPlugin);
    if let Some(path) = cli.screenshot.clone() {
        // screenshot 模式：固定分辨率窗口，第 SCREENSHOT_FRAME 帧落盘后自动退出。
        app.add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                resolution: SCREENSHOT_RESOLUTION.into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ScreenshotJob {
            frame: 0,
            fired: false,
            path,
        });
    } else {
        app.add_plugins(DefaultPlugins);
    }
    // ScatteringMedium 资产集合由示例自行注册（bevy_pbr 不代注册）；
    // 必须在 AssetPlugin（DefaultPlugins）之后。
    app.init_asset::<bevy::light::atmosphere::ScatteringMedium>();
    app.insert_resource(scene.session)
        .insert_resource(SpawnedPlan(scene.spawned))
        .insert_resource(CameraPresetChoice(cli.camera))
        .init_resource::<JunctionDebugConfig>()
        .init_resource::<PoseBuffer>()
        .init_resource::<JunctionDebugPanel>()
        .init_resource::<StaticOverlayCache>()
        .add_systems(Startup, setup_scene)
        .add_systems(
            Update,
            (
                toggle_overlay,
                sync_vehicle_transforms,
                update_traffic_lights,
                maintain_static_overlay,
                apply_overlay_visuals.after(sync_vehicle_transforms),
            ),
        )
        .add_systems(
            LaneFlowFixed,
            observe_overlay.in_set(LaneFlowFixedSet::Observe),
        );
    if cli.screenshot.is_some() {
        app.add_systems(Update, screenshot_at_frame);
    }
    app.run();
    Ok(())
}

/// 相机预设：`persp` 低机位透视（常规画面）；`topdown` 正交垂直向下看全路口
/// （车道几何核查用）；`close` 低位近景看路口一角（标线/停止线读型用）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CameraPreset {
    Persp,
    Topdown,
    Close,
}

impl CameraPreset {
    fn tag(self) -> &'static str {
        match self {
            Self::Persp => "persp",
            Self::Topdown => "topdown",
            Self::Close => "close",
        }
    }
}

#[derive(Resource)]
struct CameraPresetChoice(CameraPreset);

struct CliArgs {
    screenshot: Option<PathBuf>,
    camera: CameraPreset,
}

impl CliArgs {
    /// `--screenshot <路径> [--camera persp|topdown]`；路径为目录时写
    /// `<目录>/junction_debug_<preset>.png`。env 回退：
    /// `JUNCTION_DEBUG_SCREENSHOT` / `JUNCTION_DEBUG_CAMERA`。
    fn parse() -> Self {
        let mut screenshot = None;
        let mut camera = None;
        let mut tokens = std::env::args().skip(1);
        while let Some(token) = tokens.next() {
            match token.as_str() {
                "--screenshot" => screenshot = tokens.next().map(PathBuf::from),
                "--camera" => {
                    camera = match tokens.next().as_deref() {
                        Some("topdown") => Some(CameraPreset::Topdown),
                        Some("close") => Some(CameraPreset::Close),
                        _ => Some(CameraPreset::Persp),
                    };
                }
                _ => {}
            }
        }
        if screenshot.is_none() {
            screenshot = std::env::var("JUNCTION_DEBUG_SCREENSHOT")
                .ok()
                .map(PathBuf::from);
        }
        if camera.is_none() {
            camera = match std::env::var("JUNCTION_DEBUG_CAMERA").as_deref() {
                Ok("topdown") => Some(CameraPreset::Topdown),
                Ok("close") => Some(CameraPreset::Close),
                Ok(_) => Some(CameraPreset::Persp),
                Err(_) => None,
            };
        }
        let camera = camera.unwrap_or(CameraPreset::Persp);
        let screenshot = screenshot.map(|path| {
            let is_dir =
                path.is_dir() || path.to_string_lossy().ends_with(std::path::MAIN_SEPARATOR);
            if is_dir {
                path.join(format!("junction_debug_{}.png", camera.tag()))
            } else {
                path
            }
        });
        Self { screenshot, camera }
    }
}

/// screenshot 任务状态：第 `SCREENSHOT_FRAME` 帧 spawn `Screenshot`，
/// 观察 `ScreenshotCaptured` 落盘后写 `AppExit`。
#[derive(Resource)]
struct ScreenshotJob {
    frame: u32,
    fired: bool,
    path: PathBuf,
}

fn screenshot_at_frame(mut commands: Commands, mut job: ResMut<ScreenshotJob>) {
    job.frame = job.frame.saturating_add(1);
    if job.frame != SCREENSHOT_FRAME || job.fired {
        return;
    }
    job.fired = true;
    let path = job.path.clone();
    commands.spawn(Screenshot::primary_window()).observe(
        move |captured: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
            match captured.image.clone().try_into_dynamic() {
                Ok(dynamic) => match dynamic.to_rgb8().save(&path) {
                    Ok(()) => info!("screenshot saved to {}", path.display()),
                    Err(error) => error!("screenshot 保存失败: {error}"),
                },
                Err(error) => error!("screenshot 转换失败: {error:?}"),
            }
            exit.write(AppExit::Success);
        },
    );
}

#[derive(Resource, Default)]
struct JunctionDebugConfig {
    enabled: bool,
}

#[derive(Resource)]
struct SpawnedPlan(Box<[junction_debug_scene::SpawnedVehicle]>);

/// 跨帧复用的提取缓冲（adapter-api §6 稳定容量合同）。
#[derive(Resource, Default)]
struct PoseBuffer(LaneFlowCommittedPoseBatch);

/// 每拍由 `observe_overlay` 重写的面板内容；`selected` 供高亮标记定位，
/// `signals` 是最近一拍信号组指示（group raw + aspect）。
#[derive(Resource, Default)]
struct JunctionDebugPanel {
    content: String,
    selected: Option<VehicleHandle>,
    signals: Vec<(u32, SignalAspect)>,
}

/// 车辆表现几何：Runtime pose 是前保险杠位置，表现中心沿切向后移半个车长。
#[derive(Resource)]
struct VehicleMetrics {
    half_length: f32,
}

/// 静态绘制缓存键：换根或换世代后重建；`None` 表示当前无静态标记。
#[derive(Resource, Default)]
struct StaticOverlayCache {
    key: Option<(NetworkRevisionId, WorldGeneration)>,
}

#[derive(Component)]
struct VehicleBox;

#[derive(Component)]
struct DebugStaticRoot;

#[derive(Component)]
struct SignalDotMarker {
    group: u32,
    red: Handle<StandardMaterial>,
    yellow: Handle<StandardMaterial>,
    green: Handle<StandardMaterial>,
    unknown: Handle<StandardMaterial>,
}

#[derive(Component)]
struct SelectedVehicleMarker;

/// 常规场景交通灯的一枚灯盘（红/黄/绿之一）；`lit` 变化时才重写独立材质。
#[derive(Component)]
struct TrafficLamp {
    group: u32,
    kind: LampKind,
    material: Handle<StandardMaterial>,
    lit: bool,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum LampKind {
    Red,
    Yellow,
    Green,
}

#[derive(Component)]
struct PanelText;

fn setup_scene(
    mut commands: Commands,
    mut session: ResMut<LaneFlowSession>,
    plan: Res<SpawnedPlan>,
    preset: Res<CameraPresetChoice>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut scattering_mediums: ResMut<Assets<bevy::light::atmosphere::ScatteringMedium>>,
) {
    // 天空大气（地球散射介质 + 相机设置）与太阳（平行光，投影，约 40° 俯角）。
    let medium = scattering_mediums.add(bevy::light::atmosphere::ScatteringMedium::earth(256, 256));
    commands.spawn(bevy::light::Atmosphere::earth(medium));
    match preset.0 {
        CameraPreset::Persp => {
            commands.spawn((
                Camera3d::default(),
                // fov 62° + ~26° 俯角：画面上方露出地平线/天空，路口核心占主体。
                Projection::Perspective(PerspectiveProjection {
                    fov: 1.0821,
                    ..default()
                }),
                bevy::pbr::AtmosphereSettings::default(),
                // 0.19 起 AmbientLight / DistanceFog 是相机组件；全局环境光用
                // GlobalAmbientLight 资源做弱补光。
                AmbientLight {
                    color: Color::srgb(0.85, 0.9, 1.0),
                    brightness: 50.0,
                    ..default()
                },
                bevy::pbr::DistanceFog {
                    color: Color::srgb(0.82, 0.88, 0.97),
                    falloff: bevy::pbr::FogFalloff::Linear {
                        start: 400.0,
                        end: 1_200.0,
                    },
                    ..default()
                },
                Transform::from_xyz(55.0, 38.0, 55.0).looking_at(Vec3::ZERO, Vec3::Y),
            ));
        }
        CameraPreset::Topdown => {
            // 正交垂直向下：覆盖含环路的全场景（±182 m），方像素便于量几何。
            commands.spawn((
                Camera3d::default(),
                Projection::Orthographic(OrthographicProjection {
                    scaling_mode: ScalingMode::Fixed {
                        width: 608.0,
                        height: 380.0,
                    },
                    ..OrthographicProjection::default_3d()
                }),
                Transform::from_xyz(0.0, 300.0, 0.0).looking_at(Vec3::ZERO, Vec3::Z),
            ));
        }
        CameraPreset::Close => {
            // 低位近景：看路口东南一角，读标线线型与停止线。
            commands.spawn((
                Camera3d::default(),
                bevy::pbr::AtmosphereSettings::default(),
                AmbientLight {
                    color: Color::srgb(0.85, 0.9, 1.0),
                    brightness: 50.0,
                    ..default()
                },
                bevy::pbr::DistanceFog {
                    color: Color::srgb(0.82, 0.88, 0.97),
                    falloff: bevy::pbr::FogFalloff::Linear {
                        start: 400.0,
                        end: 1_200.0,
                    },
                    ..default()
                },
                Transform::from_xyz(46.0, 17.0, 26.0)
                    .looking_at(Vec3::new(18.0, 0.0, 3.0), Vec3::Y),
            ));
        }
    }
    commands.spawn((
        DirectionalLight {
            illuminance: 100_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, -0.8, -0.7, 0.0)),
    ));
    commands.insert_resource(bevy::light::GlobalAmbientLight {
        color: Color::srgb(0.85, 0.9, 1.0),
        brightness: 15.0,
        ..default()
    });
    // 地面：大薄板，顶面在 y=0。灰绿压暗，与沥青拉开对比。
    let ground_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.26, 0.32, 0.22),
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(GROUND_SIZE_METERS, 0.2, GROUND_SIZE_METERS))),
        MeshMaterial3d(ground_material),
        Transform::from_xyz(0.0, -0.1, 0.0),
    ));
    // 沥青材质：车行道 ribbon、路口铺装、环路共用。
    let asphalt_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.03, 0.032, 0.038),
        perceptual_roughness: 0.95,
        double_sided: true,
        ..default()
    });
    // 人行道材质：混凝土浅灰。
    let sidewalk_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.45, 0.45, 0.43),
        perceptual_roughness: 0.9,
        double_sided: true,
        ..default()
    });
    // 道路：每条 lane edge 一条沥青 ribbon；臂道按几何边界去重画标线，
    // 内部机动边与环路不画标线（真实路口内部不画线）。
    let revision = session.world().revision();
    let white_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.88, 0.88, 0.85),
        double_sided: true,
        ..default()
    });
    let yellow_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.58, 0.4, 0.02),
        double_sided: true,
        ..default()
    });
    let mut edge_count = 0_u32;
    let mut loops = 0_u32;
    let mut arms = 0_u32;
    let mut internal = 0_u32;
    let mut bounds_min = Vec3::splat(f32::MAX);
    let mut bounds_max = Vec3::splat(f32::MIN);
    let mut road_layout = None;
    if let Some(lane_pose) = revision.spatial().and_then(|spatial| spatial.lane_pose()) {
        for index in 0..lane_pose.lane_edge_count() {
            let ordinal = laneflow_static_contract::LaneEdgeOrdinal::from_raw(index);
            let Some(geometry) = lane_pose.lane_geometry(ordinal) else {
                continue;
            };
            edge_count += 1;
            let arc = geometry.arc_length_meters();
            if arc > 300.0 {
                loops += 1;
            } else if arc >= 100.0 {
                arms += 1;
            } else {
                internal += 1;
            }
            let points: Vec<Vec3> = geometry.points().iter().map(point_to_vec3).collect();
            for point in &points {
                bounds_min = bounds_min.min(*point);
                bounds_max = bounds_max.max(*point);
            }
            if let Some(mesh) = ribbon_mesh(&points, LANE_WIDTH_METERS, ROAD_Y, 0.0, false) {
                commands.spawn((
                    Mesh3d(meshes.add(mesh)),
                    MeshMaterial3d(asphalt_material.clone()),
                ));
            }
        }
        let layout = build_road_layout(&revision, lane_pose);
        road_layout = Some(layout);
        info!(
            "道路布置: 主路半宽 {:.2} 次路半宽 {:.2} 路口 x±{:.1} z±{:.1} 臂道末端 x={:.1} z={:.1} 街角半径 NE/NW/SW/SE = {:.1}/{:.1}/{:.1}/{:.1}",
            layout.main_half_width,
            layout.secondary_half_width,
            layout.junction_half_x,
            layout.junction_half_z,
            layout.main_arm_end,
            layout.secondary_arm_end,
            layout.corner_radii[0],
            layout.corner_radii[1],
            layout.corner_radii[2],
            layout.corner_radii[3]
        );
        // 路口铺装：主/次路两条矩形带 + 四个街角四分之一圆盘，各自微错层
        // 防共面 z-fight（不用单一多边形扇形三角化——外凸半径不对称时轮廓
        // 相对原点非星形，扇面会折叠出伪影）。
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(
                layout.junction_half_x * 2.0,
                0.02,
                layout.main_half_width * 2.0,
            ))),
            MeshMaterial3d(asphalt_material.clone()),
            Transform::from_xyz(0.0, PATCH_Y - 0.010, 0.0),
        ));
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(
                layout.secondary_half_width * 2.0,
                0.02,
                layout.junction_half_z * 2.0,
            ))),
            MeshMaterial3d(asphalt_material.clone()),
            Transform::from_xyz(0.0, PATCH_Y - 0.008, 0.0),
        ));
        for (index, (sx, sz)) in [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)]
            .into_iter()
            .enumerate()
        {
            let mesh = corner_disc_mesh(
                sx * layout.secondary_half_width,
                sz * layout.main_half_width,
                layout.corner_radii[index],
                sx,
                sz,
                PATCH_Y - 0.006,
            );
            commands.spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(asphalt_material.clone()),
            ));
        }
        spawn_sidewalks(
            &mut commands,
            &mut meshes,
            &layout,
            sidewalk_material.clone(),
        );
        spawn_arm_markings(
            &mut commands,
            &mut meshes,
            &revision,
            lane_pose,
            white_material.clone(),
            yellow_material.clone(),
        );
        spawn_stop_lines(
            &mut commands,
            &mut meshes,
            &revision,
            lane_pose,
            white_material.clone(),
        );
        spawn_crosswalks(
            &mut commands,
            &mut meshes,
            &revision,
            lane_pose,
            &layout,
            white_material.clone(),
        );
        spawn_lane_arrows(
            &mut commands,
            &mut meshes,
            session.world(),
            &revision,
            lane_pose,
            white_material.clone(),
        );
        spawn_loop_furniture(
            &mut commands,
            &mut meshes,
            &revision,
            lane_pose,
            white_material.clone(),
            sidewalk_material.clone(),
        );
    }
    info!(
        "junction_debug 场景: lane_geometry {edge_count} 条（环路 {loops} / 臂道 {arms} / 内部 {internal}），bounds x[{:.1}, {:.1}] z[{:.1}, {:.1}]",
        bounds_min.x, bounds_max.x, bounds_min.z, bounds_max.z
    );
    // 车辆：按角色着色，车体 + 深色 cabin 子实体，中心落在前保险杠后方半个车长。
    let profile_length_mm = session
        .world()
        .traffic()
        .relations()
        .vehicle_profile(laneflow_static_contract::VehicleProfileOrdinal::from_raw(0))
        .map_or(4_500, |profile| profile.length_mm());
    let length = (profile_length_mm as f32) / 1_000.0;
    commands.insert_resource(VehicleMetrics {
        half_length: length / 2.0,
    });
    let vehicle_mesh = meshes.add(Cuboid::new(
        VEHICLE_WIDTH_METERS,
        VEHICLE_HEIGHT_METERS,
        length,
    ));
    let cabin_mesh = meshes.add(Cuboid::new(VEHICLE_WIDTH_METERS * 0.85, 0.55, length * 0.5));
    let cabin_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.08, 0.09, 0.12),
        ..default()
    });
    for spawned in plan.0.iter() {
        let body = materials.add(StandardMaterial {
            base_color: role_color(spawned.role),
            perceptual_roughness: 0.5,
            ..default()
        });
        let entity = commands
            .spawn((
                Mesh3d(vehicle_mesh.clone()),
                MeshMaterial3d(body),
                Transform::IDENTITY,
                VehicleBox,
            ))
            .id();
        commands.spawn((
            Mesh3d(cabin_mesh.clone()),
            MeshMaterial3d(cabin_material.clone()),
            Transform::from_xyz(0.0, VEHICLE_HEIGHT_METERS * 0.5 + 0.1, -0.3),
            ChildOf(entity),
        ));
        if let Err(error) = session.bind_vehicle_entity(spawned.vehicle, entity) {
            warn!("bind_vehicle_entity 失败: {error:?}");
        }
    }
    // 交通灯（常规场景）：每组在 stop line 旁立杆 + 灯头 + 红/黄/绿三枚灯盘，
    // 每拍按 committed_signal_groups() 点亮（亮 = 同色自发光，灭 = 深灰）。
    let pole_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.12, 0.12, 0.13),
        ..default()
    });
    let head_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.08, 0.08, 0.09),
        ..default()
    });
    let pole_mesh = meshes.add(Cuboid::new(0.22, 5.5, 0.22));
    let head_mesh = meshes.add(Cuboid::new(0.8, 1.8, 0.45));
    let lamp_mesh = meshes.add(Sphere::new(0.27).mesh().build());
    let group_count = revision
        .traffic()
        .entity_counts()
        .count(EntityKind::SignalGroup);
    for raw in 0..group_count {
        let group = SignalGroupOrdinal::from_raw(raw);
        let Some((position, direction)) = signal_anchor(&revision, group) else {
            continue;
        };
        // 灯杆立在人行道内：右侧通行，进口车道的右侧即路缘侧（有几何反例则翻转），
        // 横向推到车行道外缘以外 1.2 m，避免压在车道上。
        let right_raw = direction.cross(Vec3::Y).normalize_or_zero();
        let pole_base = match road_layout {
            Some(layout) => {
                let main_axis = direction.x.abs() >= direction.z.abs();
                let half_width = if main_axis {
                    layout.main_half_width
                } else {
                    layout.secondary_half_width
                };
                let lateral = if main_axis { position.z } else { position.x };
                let right = {
                    let axis_lateral = if main_axis { right_raw.z } else { right_raw.x };
                    if axis_lateral.signum() == lateral.signum() {
                        right_raw
                    } else {
                        -right_raw
                    }
                };
                let distance = (half_width - lateral.abs()).max(0.0) + 1.2;
                position + right * distance
            }
            None => position + right_raw * (LANE_WIDTH_METERS * 0.5 + 1.2),
        };
        commands.spawn((
            Mesh3d(pole_mesh.clone()),
            MeshMaterial3d(pole_material.clone()),
            Transform::from_translation(pole_base + Vec3::Y * 2.75),
        ));
        let head_center = pole_base + Vec3::Y * 5.1;
        commands.spawn((
            Mesh3d(head_mesh.clone()),
            MeshMaterial3d(head_material.clone()),
            Transform::from_translation(head_center).looking_to(direction, Vec3::Y),
        ));
        for (kind, lift) in [
            (LampKind::Red, 0.55_f32),
            (LampKind::Yellow, 0.0),
            (LampKind::Green, -0.55),
        ] {
            let material = materials.add(lamp_material(false, kind));
            commands.spawn((
                Mesh3d(lamp_mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::from_translation(head_center + Vec3::Y * lift - direction * 0.28),
                TrafficLamp {
                    group: raw,
                    kind,
                    material,
                    lit: false,
                },
            ));
        }
    }
    commands.spawn((
        Text::new(String::new()),
        TextFont::from_font_size(18.0),
        TextColor(Color::srgb(0.95, 0.95, 0.98)),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(12.0),
            top: Val::Px(12.0),
            padding: UiRect::all(Val::Px(8.0)),
            ..default()
        },
        // 深色半透明底衬，保证浅色天空背景下面板可读。
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
        PanelText,
    ));
}

fn role_color(role: junction_debug_scene::VehicleRole) -> Color {
    match role {
        junction_debug_scene::VehicleRole::Through => Color::srgb(0.15, 0.35, 0.95),
        junction_debug_scene::VehicleRole::WaitingLeft => Color::srgb(0.9, 0.15, 0.12),
        junction_debug_scene::VehicleRole::PermissiveLeft => Color::srgb(0.95, 0.8, 0.1),
        junction_debug_scene::VehicleRole::Circuit => Color::srgb(0.15, 0.75, 0.3),
    }
}

/// 信号组定位：组内首道门的停止线端点 + 该边末段切向（朝向来车方向）。
fn signal_anchor(
    revision: &SharedNetworkRevision,
    group: SignalGroupOrdinal,
) -> Option<(Vec3, Vec3)> {
    let relations = revision.traffic().relations();
    let gate = *relations.signal_group(group)?.gates().first()?;
    let stop = relations.stop_line(relations.maneuver_gate(gate)?.stop_line())?;
    let geometry = revision
        .spatial()?
        .lane_pose()?
        .lane_geometry(stop.edge())?;
    let points = geometry.points();
    let last = *points.last()?;
    let previous = points
        .get(points.len().saturating_sub(2))
        .copied()
        .unwrap_or(last);
    let direction = (point_to_vec3(&last) - point_to_vec3(&previous)).normalize_or_zero();
    let direction = if direction.length_squared() < 0.5 {
        Vec3::X
    } else {
        direction
    };
    Some((point_to_vec3(&last), direction))
}

/// 灯盘材质：亮 = 同色自发光，灭 = 深灰。
fn lamp_material(lit: bool, kind: LampKind) -> StandardMaterial {
    let color = match kind {
        LampKind::Red => Color::srgb(0.95, 0.1, 0.08),
        LampKind::Yellow => Color::srgb(0.95, 0.8, 0.1),
        LampKind::Green => Color::srgb(0.1, 0.85, 0.25),
    };
    if lit {
        StandardMaterial {
            base_color: color,
            emissive: LinearRgba::from(color) * 6.0,
            ..default()
        }
    } else {
        StandardMaterial {
            base_color: Color::srgb(0.1, 0.1, 0.1),
            ..default()
        }
    }
}

/// 每拍按 committed_signal_groups() 更新灯盘点亮状态（仅状态翻转时重写材质）。
fn update_traffic_lights(
    session: Res<LaneFlowSession>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut lamps: Query<&mut TrafficLamp>,
) {
    let signals = session.world().committed_signal_groups();
    for mut lamp in &mut lamps {
        let aspect = signals
            .as_slice()
            .iter()
            .find(|(group, _)| group.raw() == lamp.group)
            .map(|(_, aspect)| *aspect);
        let lit = matches!(
            (aspect, lamp.kind),
            (Some(SignalAspect::Red), LampKind::Red)
                | (Some(SignalAspect::Yellow), LampKind::Yellow)
                | (Some(SignalAspect::Green), LampKind::Green)
        );
        if lit != lamp.lit {
            lamp.lit = lit;
            if let Some(mut material) = materials.get_mut(&lamp.material) {
                *material = lamp_material(lit, lamp.kind);
            }
        }
    }
}

fn point_to_vec3(point: &CanonicalPoint) -> Vec3 {
    Vec3::new(point.x, point.y, point.z)
}

/// 臂道车道标线分类：对向中央双黄 / 同向车道虚线 / 外缘实线。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ArmMarkingKind {
    CenterDoubleYellow,
    LaneSeparator,
    OuterSolid,
}

/// 臂道标线：按臂道分组、按几何边界去重（同一边界只画一次），内部机动边
/// 与环路不画。边界横向偏移从 lane_geometry 采样点聚类推导，不硬编码车道值。
fn spawn_arm_markings(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    revision: &SharedNetworkRevision,
    lane_pose: &laneflow_static_network::LanePoseNetwork,
    white: Handle<StandardMaterial>,
    yellow: Handle<StandardMaterial>,
) {
    // (臂, 量化横向偏移 mm) -> (偏移, 边界世界折线)。同一边界由相邻两条车道
    // 各贡献一次，去重后只画一条。
    let mut boundaries: HashMap<(String, i64), (f64, Vec<Vec3>)> = HashMap::new();
    for (name, ordinal) in junction_debug_scene::edge_ordinals(revision).iter() {
        // 臂道边命名形如 e-in-i0 / s-out；内部边含 '.'；环路 loop-* 不画标线。
        if name.contains('.') || name.starts_with("loop-") {
            continue;
        }
        let arm = name.split('-').next().unwrap_or_default().to_owned();
        let Some(geometry) = lane_pose.lane_geometry(*ordinal) else {
            continue;
        };
        let points: Vec<Vec3> = geometry.points().iter().map(point_to_vec3).collect();
        let (Some(first), Some(last)) = (points.first(), points.last()) else {
            continue;
        };
        let chord = *last - *first;
        let tangent = Vec3::new(chord.x, 0.0, chord.z).normalize_or_zero();
        if tangent.length_squared() < 0.5 {
            continue;
        }
        // 定向法向与臂轴横向偏移；同臂各车道边切向共线，偏移可直接比较、去重。
        let normal = Vec3::new(-tangent.z, 0.0, tangent.x);
        let center_offset = f64::from(normal.dot(*first));
        let half = LANE_WIDTH_METERS * 0.5;
        for side in [1.0_f32, -1.0] {
            let offset = center_offset + f64::from(side * half);
            let key = (arm.clone(), (offset * 1_000.0).round() as i64);
            let shifted: Vec<Vec3> = points
                .iter()
                .map(|point| *point + normal * (side * half))
                .collect();
            boundaries.entry(key).or_insert((offset, shifted));
        }
    }
    let mut by_arm: HashMap<String, Vec<(f64, Vec<Vec3>)>> = HashMap::new();
    for ((arm, _), boundary) in boundaries {
        by_arm.entry(arm).or_default().push(boundary);
    }
    for (_arm, list) in by_arm {
        // 每侧（偏移符号）：|offset| 最小 = 对向中央双黄，最大 = 外缘实线，
        // 其余 = 同向相邻车道之间的白色虚线。
        for side in [1.0_f64, -1.0_f64] {
            let mut group: Vec<(f64, Vec<Vec3>)> = list
                .iter()
                .filter(|(offset, _)| offset.signum() == side)
                .cloned()
                .collect();
            group.sort_by(|left, right| left.0.abs().total_cmp(&right.0.abs()));
            let last_rank = group.len().saturating_sub(1);
            for (rank, (_, line)) in group.into_iter().enumerate() {
                let kind = if rank == 0 {
                    ArmMarkingKind::CenterDoubleYellow
                } else if rank == last_rank {
                    ArmMarkingKind::OuterSolid
                } else {
                    ArmMarkingKind::LaneSeparator
                };
                match kind {
                    ArmMarkingKind::CenterDoubleYellow => {
                        // 双黄：以边界为中心 ±gap/2 各一条黄实线。
                        for shift in [
                            DOUBLE_YELLOW_GAP_METERS * 0.5,
                            -DOUBLE_YELLOW_GAP_METERS * 0.5,
                        ] {
                            if let Some(mesh) =
                                ribbon_mesh(&line, LINE_WIDTH_METERS, MARKING_Y, shift, false)
                            {
                                commands.spawn((
                                    Mesh3d(meshes.add(mesh)),
                                    MeshMaterial3d(yellow.clone()),
                                ));
                            }
                        }
                    }
                    ArmMarkingKind::LaneSeparator => {
                        // 白虚线 3 m 画 / 6 m 空。
                        for mesh in dash_ribbons(
                            &line,
                            DASH_ON_METERS,
                            DASH_OFF_METERS,
                            LINE_WIDTH_METERS,
                            MARKING_Y,
                        ) {
                            commands
                                .spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(white.clone())));
                        }
                    }
                    ArmMarkingKind::OuterSolid => {
                        if let Some(mesh) =
                            ribbon_mesh(&line, LINE_WIDTH_METERS, MARKING_Y, 0.0, false)
                        {
                            commands
                                .spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(white.clone())));
                        }
                    }
                }
            }
        }
    }
}

/// 停止线：每条绑定了信号门的 stop line 在其边终点处画一条横跨车道的
/// 白色实线带（沿车道 0.4 m × 车道宽），垂直于车道切向。
fn spawn_stop_lines(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    revision: &SharedNetworkRevision,
    lane_pose: &laneflow_static_network::LanePoseNetwork,
    white: Handle<StandardMaterial>,
) {
    let relations = revision.traffic().relations();
    let stop_count = usize::try_from(
        revision
            .traffic()
            .entity_counts()
            .count(EntityKind::StopLine),
    )
    .unwrap_or(0);
    let mut drawn = std::collections::BTreeSet::new();
    for raw in 0..stop_count {
        let stop = StopLineOrdinal::from_raw(u32::try_from(raw).unwrap_or(0));
        let Some(view) = relations.stop_line(stop) else {
            continue;
        };
        // 只画绑定了信号门的停止线。
        let signal_gated = view.gates().iter().any(|&gate| {
            relations
                .maneuver_gate(gate)
                .and_then(|gate_view| gate_view.signal_group())
                .is_some()
        });
        if !signal_gated || !drawn.insert(raw) {
            continue;
        }
        let Some(geometry) = lane_pose.lane_geometry(view.edge()) else {
            continue;
        };
        let points = geometry.points();
        let (Some(last), _) = (points.last(), points.first()) else {
            continue;
        };
        let end = point_to_vec3(last);
        let previous = points
            .get(points.len().saturating_sub(2))
            .map(point_to_vec3)
            .unwrap_or(end);
        let tangent = (end - previous).normalize_or_zero();
        if tangent.length_squared() < 0.5 {
            continue;
        }
        let band = [
            end - tangent * (STOP_LINE_BAND_METERS * 0.5),
            end + tangent * (STOP_LINE_BAND_METERS * 0.5),
        ];
        if let Some(mesh) = ribbon_mesh(&band, LANE_WIDTH_METERS, MARKING_Y, 0.0, false) {
            commands.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(white.clone())));
        }
    }
}

/// 从 lane_geometry 推导的道路布置：车行道半宽、路口铺装范围、臂道末端坐标。
/// 全部由采样点推导，不硬编码场景合同数值。
#[derive(Clone, Copy)]
struct RoadLayout {
    /// 主路（东西向，e/w 臂）车行道半宽（米）。
    main_half_width: f32,
    /// 次路（南北向，n/s 臂）车行道半宽（米）。
    secondary_half_width: f32,
    /// 路口铺装沿主路/次路轴向的半范围（米），含与臂道 ribbon 的 0.5 m 搭接。
    junction_half_x: f32,
    junction_half_z: f32,
    /// 主路/次路臂道末端坐标（米），人行道直线段的终点。
    main_arm_end: f32,
    secondary_arm_end: f32,
    /// 四个街角（NE/NW/SW/SE）的铺装外凸半径（米）：以「路缘带角点」为圆心的
    /// 四分之一圆盘，半径 = 该象限内部机动路径到角点的最大距离 + 2 m 余量，
    /// 保证转向路径全部落在铺装内（人行道具此贴着铺装外缘走）。
    corner_radii: [f32; 4],
}

fn build_road_layout(
    revision: &SharedNetworkRevision,
    lane_pose: &laneflow_static_network::LanePoseNetwork,
) -> RoadLayout {
    let mut main_lateral = 0.0_f32;
    let mut secondary_lateral = 0.0_f32;
    let mut main_arm_end = 0.0_f32;
    let mut secondary_arm_end = 0.0_f32;
    let mut junction_half_x = 0.0_f32;
    let mut junction_half_z = 0.0_f32;
    for (name, ordinal) in junction_debug_scene::edge_ordinals(revision) {
        let Some(geometry) = lane_pose.lane_geometry(ordinal) else {
            continue;
        };
        if name.starts_with("loop-") {
            continue;
        }
        if name.contains('.') {
            // 内部机动边：决定路口铺装范围。
            for point in geometry.points() {
                junction_half_x = junction_half_x.max(point.x.abs());
                junction_half_z = junction_half_z.max(point.z.abs());
            }
            continue;
        }
        // 臂道边：e/w 沿 x 轴（主路），n/s 沿 z 轴（次路）。
        match name.as_bytes().first() {
            Some(b'e') | Some(b'w') => {
                for point in geometry.points() {
                    main_lateral = main_lateral.max(point.z.abs());
                    main_arm_end = main_arm_end.max(point.x.abs());
                }
            }
            Some(b'n') | Some(b's') => {
                for point in geometry.points() {
                    secondary_lateral = secondary_lateral.max(point.x.abs());
                    secondary_arm_end = secondary_arm_end.max(point.z.abs());
                }
            }
            _ => {}
        }
    }
    let main_half_width = main_lateral + LANE_WIDTH_METERS * 0.5;
    let secondary_half_width = secondary_lateral + LANE_WIDTH_METERS * 0.5;
    let junction_half_x = junction_half_x + 0.5;
    let junction_half_z = junction_half_z + 0.5;
    // 第二遍：按象限量取内部路径探入街角的深度，推出各角铺装外凸半径。
    // 注意必须遍历 lane_pose 的全部边——部分内部转向边不在任何 catalog 路线里，
    // edge_ordinals 覆盖不到；这些边按「无编制名且弧长 < 100 m」归入内部边。
    let mut corner_radii = [4.0_f32; 4];
    let radius_cap = (junction_half_x - secondary_half_width)
        .min(junction_half_z - main_half_width)
        .max(4.0);
    let named: HashMap<u32, String> = junction_debug_scene::edge_ordinals(revision)
        .into_iter()
        .map(|(name, ordinal)| (ordinal.raw(), name))
        .collect();
    for raw in 0..lane_pose.lane_edge_count() {
        let is_internal = match named.get(&raw) {
            Some(name) => name.contains('.'),
            None => lane_pose
                .lane_geometry(laneflow_static_contract::LaneEdgeOrdinal::from_raw(raw))
                .is_some_and(|geometry| geometry.arc_length_meters() < 100.0),
        };
        if !is_internal {
            continue;
        }
        let Some(geometry) =
            lane_pose.lane_geometry(laneflow_static_contract::LaneEdgeOrdinal::from_raw(raw))
        else {
            continue;
        };
        for point in geometry.points() {
            let (Some(corner_x), Some(corner_z)) = (
                (point.x.abs() > secondary_half_width).then_some(secondary_half_width),
                (point.z.abs() > main_half_width).then_some(main_half_width),
            ) else {
                continue;
            };
            let corner = match (point.x >= 0.0, point.z >= 0.0) {
                (true, true) => 0,
                (false, true) => 1,
                (false, false) => 2,
                (true, false) => 3,
            };
            let reach = (point.x.abs() - corner_x).hypot(point.z.abs() - corner_z) + 2.0;
            corner_radii[corner] = corner_radii[corner].max(reach).min(radius_cap);
        }
    }
    RoadLayout {
        main_half_width,
        secondary_half_width,
        junction_half_x,
        junction_half_z,
        main_arm_end: main_arm_end + 2.0,
        secondary_arm_end: secondary_arm_end + 2.0,
        corner_radii,
    }
}

/// 人行道：四个街角各由「主路侧直线带 + 街角环带扇形 + 次路侧直线带」组成，
/// 贴着铺装外缘走（环带与铺装圆盘同心，内径即铺装半径）。路口四个口部是
/// 车行道连续区，不画人行道。
fn spawn_sidewalks(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    layout: &RoadLayout,
    material: Handle<StandardMaterial>,
) {
    let half_band = SIDEWALK_WIDTH_METERS * 0.5;
    let m = layout.main_half_width;
    let s = layout.secondary_half_width;
    for (sx, sz, radius) in [
        (1.0_f32, 1.0_f32, layout.corner_radii[0]),
        (-1.0, 1.0, layout.corner_radii[1]),
        (-1.0, -1.0, layout.corner_radii[2]),
        (1.0, -1.0, layout.corner_radii[3]),
    ] {
        // 主路侧直线带：从臂道末端到街角环带（探入 1.5 m 搭接，避免接缝露草）。
        let main_leg = [
            Vec3::new(sx * layout.main_arm_end, 0.0, sz * (m + half_band)),
            Vec3::new(sx * (s + radius + half_band), 0.0, sz * (m + half_band)),
        ];
        // 次路侧直线带同理。
        let secondary_leg = [
            Vec3::new(sx * (s + half_band), 0.0, sz * (m + radius + half_band)),
            Vec3::new(sx * (s + half_band), 0.0, sz * layout.secondary_arm_end),
        ];
        for leg in [main_leg, secondary_leg] {
            if let Some(mesh) = ribbon_mesh(&leg, SIDEWALK_WIDTH_METERS, SIDEWALK_Y, 0.0, false) {
                commands.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(material.clone())));
            }
        }
        let mesh = annulus_sector_mesh(
            sx * s,
            sz * m,
            radius,
            radius + SIDEWALK_WIDTH_METERS,
            sx,
            sz,
            SIDEWALK_Y + 0.004,
        );
        commands.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(material.clone())));
    }
}

/// 街角四分之一圆盘铺装：NE 象限生成后按 (sx, sz) 镜像；法线一律朝上。
fn corner_disc_mesh(cx: f32, cz: f32, radius: f32, sx: f32, sz: f32, y: f32) -> Mesh {
    const STEPS: u32 = 12;
    let mut positions = Vec::with_capacity(STEPS as usize + 2);
    positions.push([cx, y, cz]);
    for index in 0..=STEPS {
        let theta = (index as f32 / STEPS as f32) * std::f32::consts::FRAC_PI_2;
        let (sin, cos) = theta.sin_cos();
        positions.push([cx + sx * radius * cos, y, cz + sz * radius * sin]);
    }
    let mut indices = Vec::with_capacity(STEPS as usize * 3);
    for index in 0..STEPS {
        indices.extend_from_slice(&[0, index + 1, index + 2]);
    }
    let mut mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    );
    let count = positions.len();
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0_f32, 1.0, 0.0]; count]);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// 街角环带扇形（人行道转角）：NE 象限的 90° 环带按 (sx, sz) 镜像；
/// 法线一律朝上。
fn annulus_sector_mesh(cx: f32, cz: f32, inner: f32, outer: f32, sx: f32, sz: f32, y: f32) -> Mesh {
    const STEPS: u32 = 12;
    let mut positions = Vec::with_capacity((STEPS as usize + 1) * 2);
    for index in 0..=STEPS {
        let theta = (index as f32 / STEPS as f32) * std::f32::consts::FRAC_PI_2;
        let (sin, cos) = theta.sin_cos();
        positions.push([cx + sx * inner * cos, y, cz + sz * inner * sin]);
        positions.push([cx + sx * outer * cos, y, cz + sz * outer * sin]);
    }
    let mut indices = Vec::with_capacity(STEPS as usize * 6);
    for index in 0..STEPS {
        let base = index * 2;
        indices.extend_from_slice(&[base, base + 1, base + 2, base + 1, base + 3, base + 2]);
    }
    let mut mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    );
    let count = positions.len();
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0_f32, 1.0, 0.0]; count]);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// 斑马线：只在臂道进口车道（`-in` 边）的信号停止线处画，向路口一侧 2 m、
/// 横跨整个车行道（双向）的条纹带；中心投影到道路轴线上，同一臂口的多条
/// 进口车道去重为一条斑马线。
fn spawn_crosswalks(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    revision: &SharedNetworkRevision,
    lane_pose: &laneflow_static_network::LanePoseNetwork,
    layout: &RoadLayout,
    white: Handle<StandardMaterial>,
) {
    let mut entry_edges = std::collections::BTreeSet::new();
    for (name, ordinal) in junction_debug_scene::edge_ordinals(revision) {
        if !name.contains('.') && !name.starts_with("loop-") && name.contains("-in") {
            entry_edges.insert(ordinal.raw());
        }
    }
    let relations = revision.traffic().relations();
    let stop_count = usize::try_from(
        revision
            .traffic()
            .entity_counts()
            .count(EntityKind::StopLine),
    )
    .unwrap_or(0);
    let mut placed = std::collections::BTreeSet::new();
    for raw in 0..stop_count {
        let stop = StopLineOrdinal::from_raw(u32::try_from(raw).unwrap_or(0));
        let Some(view) = relations.stop_line(stop) else {
            continue;
        };
        let signal_gated = view.gates().iter().any(|&gate| {
            relations
                .maneuver_gate(gate)
                .and_then(|gate_view| gate_view.signal_group())
                .is_some()
        });
        if !signal_gated || !entry_edges.contains(&view.edge().raw()) {
            continue;
        }
        let Some(geometry) = lane_pose.lane_geometry(view.edge()) else {
            continue;
        };
        let points = geometry.points();
        let Some(last) = points.last() else {
            continue;
        };
        let end = point_to_vec3(last);
        let previous = points
            .get(points.len().saturating_sub(2))
            .map(point_to_vec3)
            .unwrap_or(end);
        let tangent = (end - previous).normalize_or_zero();
        if tangent.length_squared() < 0.5 {
            continue;
        }
        // 停止线带半宽 + 净距 + 半条斑马线长，落点在停止线靠路口一侧；
        // 横向投影回道路轴线（斑马线横跨双向整个车行道，居中于道路）。
        let lateral = Vec3::new(-tangent.z, 0.0, tangent.x);
        let mut center = end
            + tangent
                * (STOP_LINE_BAND_METERS * 0.5
                    + CROSSWALK_SETBACK_METERS
                    + CROSSWALK_LENGTH_METERS * 0.5);
        center -= lateral * lateral.dot(center);
        // 同一臂口的多条进口车道共享一条斑马线：按 0.5 m 量化去重。
        let key = (
            (center.x * 2.0).round() as i32,
            (center.z * 2.0).round() as i32,
        );
        if !placed.insert(key) {
            continue;
        }
        let main_axis = tangent.x.abs() >= tangent.z.abs();
        let half_width = if main_axis {
            layout.main_half_width
        } else {
            layout.secondary_half_width
        };
        // 条纹复用 ribbon_mesh 渲染路径：每条条纹 = 沿车道方向的一段 ribbon，
        // 法向偏移即横跨位置（中心 = cursor + 半条宽）。
        let half_length = CROSSWALK_LENGTH_METERS * 0.5;
        let stripe_axis = [
            center - tangent * half_length,
            center + tangent * half_length,
        ];
        let mut cursor = -half_width + CROSSWALK_STRIPE_GAP_METERS;
        while cursor + CROSSWALK_STRIPE_WIDTH_METERS <= half_width {
            let stripe_center = cursor + CROSSWALK_STRIPE_WIDTH_METERS * 0.5;
            if let Some(mesh) = ribbon_mesh(
                &stripe_axis,
                CROSSWALK_STRIPE_WIDTH_METERS,
                MARKING_Y,
                stripe_center,
                false,
            ) {
                commands.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(white.clone())));
            }
            cursor += CROSSWALK_STRIPE_WIDTH_METERS + CROSSWALK_STRIPE_GAP_METERS;
        }
    }
}

/// 进口车道导向箭头：按穿过该车道的路线机动方向（直行/左转/右转）合并成
/// 一个白色箭头 mesh，施画在停止线前 `ARROW_SETBACK_METERS` 处。
fn spawn_lane_arrows(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    world: &laneflow_runtime::TrafficWorld,
    revision: &SharedNetworkRevision,
    lane_pose: &laneflow_static_network::LanePoseNetwork,
    white: Handle<StandardMaterial>,
) {
    let ordinals = junction_debug_scene::edge_ordinals(revision);
    let mut name_by_ordinal: HashMap<u32, &str> = HashMap::new();
    for (name, ordinal) in &ordinals {
        name_by_ordinal.insert(ordinal.raw(), name.as_str());
    }
    // 每条进口车道边的机动方向集合：bit0 直行、bit1 左转、bit2 右转。
    // 路线边序是 环路→进口臂→内部→出口臂→环路（环路车还会多次过路口），
    // 进口臂边可在任意位置：逐位置找 `-in` 边，用其后一条边的切向判转向。
    let mut moves: HashMap<u32, u8> = HashMap::new();
    for route in world.live_routes() {
        let Some(edges) = world.route_edges(route) else {
            continue;
        };
        for (index, first) in edges.iter().enumerate() {
            let Some(name) = name_by_ordinal.get(&first.raw()) else {
                continue;
            };
            if !name.contains("-in") {
                continue;
            }
            let Some(second) = edges.get(index + 1) else {
                continue;
            };
            let (Some(g0), Some(g1)) = (
                lane_pose.lane_geometry(*first),
                lane_pose.lane_geometry(*second),
            ) else {
                continue;
            };
            let (Some(t0), Some(t1)) = (end_tangent(g0.points()), start_tangent(g1.points()))
            else {
                continue;
            };
            // (t0 × t1).y < 0 为右转（右手系、+Y 向上）。
            let cross_y = t0.z * t1.x - t0.x * t1.z;
            let bit = if cross_y > 0.25 {
                0b010
            } else if cross_y < -0.25 {
                0b100
            } else {
                0b001
            };
            *moves.entry(first.raw()).or_default() |= bit;
        }
    }
    for (raw, bits) in moves {
        let Some(geometry) =
            lane_pose.lane_geometry(laneflow_static_contract::LaneEdgeOrdinal::from_raw(raw))
        else {
            continue;
        };
        let Some(end) = geometry.points().last().map(point_to_vec3) else {
            continue;
        };
        let Some(tangent) = end_tangent(geometry.points()) else {
            continue;
        };
        // 导向箭头全部走 ribbon_mesh（已验证的渲染路径）：箭杆一条竖带，
        // 直行头为人字两斜带，转向头为横杆 + 侧向人字两斜带。
        let position = end - tangent * ARROW_SETBACK_METERS;
        let rotation = Quat::from_rotation_y(tangent.x.atan2(tangent.z));
        let mut strips: Vec<([Vec3; 2], f32)> = Vec::new();
        strips.push(([Vec3::new(0.0, 0.0, -1.3), Vec3::new(0.0, 0.0, 0.4)], 0.3));
        if bits & 0b001 != 0 {
            strips.push((
                [Vec3::new(-0.45, 0.0, 0.75), Vec3::new(0.0, 0.0, 1.5)],
                0.22,
            ));
            strips.push(([Vec3::new(0.45, 0.0, 0.75), Vec3::new(0.0, 0.0, 1.5)], 0.22));
        }
        if bits & 0b010 != 0 {
            strips.push((
                [Vec3::new(0.0, 0.0, 0.55), Vec3::new(-1.05, 0.0, 0.55)],
                0.3,
            ));
            strips.push((
                [Vec3::new(-0.8, 0.0, 0.95), Vec3::new(-1.6, 0.0, 0.55)],
                0.22,
            ));
            strips.push((
                [Vec3::new(-0.8, 0.0, 0.15), Vec3::new(-1.6, 0.0, 0.55)],
                0.22,
            ));
        }
        if bits & 0b100 != 0 {
            strips.push(([Vec3::new(0.0, 0.0, 0.55), Vec3::new(1.05, 0.0, 0.55)], 0.3));
            strips.push(([Vec3::new(0.8, 0.0, 0.95), Vec3::new(1.6, 0.0, 0.55)], 0.22));
            strips.push(([Vec3::new(0.8, 0.0, 0.15), Vec3::new(1.6, 0.0, 0.55)], 0.22));
        }
        for (points, width) in strips {
            let Some(mesh) = ribbon_mesh(&points, width, MARKING_Y, 0.0, false) else {
                continue;
            };
            commands.spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(white.clone()),
                Transform::from_translation(position).with_rotation(rotation),
            ));
        }
    }
}

/// 折线末段/首段单位切向（XZ 平面）。
fn end_tangent(points: &[CanonicalPoint]) -> Option<Vec3> {
    let last = points.last()?;
    let previous = points.get(points.len().saturating_sub(2)).unwrap_or(last);
    let tangent = (point_to_vec3(last) - point_to_vec3(previous)).normalize_or_zero();
    (tangent.length_squared() >= 0.5).then_some(tangent)
}

fn start_tangent(points: &[CanonicalPoint]) -> Option<Vec3> {
    let first = points.first()?;
    let next = points.get(1).unwrap_or(first);
    let tangent = (point_to_vec3(next) - point_to_vec3(first)).normalize_or_zero();
    (tangent.length_squared() >= 0.5).then_some(tangent)
}

/// 环路道路化：每轨两侧白色路缘实线（读作单车道匝道），每角外轨
/// （离路口中心更远的一轨）外侧再压一条 3 m 人行道。
fn spawn_loop_furniture(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    revision: &SharedNetworkRevision,
    lane_pose: &laneflow_static_network::LanePoseNetwork,
    white: Handle<StandardMaterial>,
    sidewalk: Handle<StandardMaterial>,
) {
    let mut loops: Vec<(String, Vec<Vec3>)> = Vec::new();
    for (name, ordinal) in junction_debug_scene::edge_ordinals(revision) {
        if !name.starts_with("loop-") {
            continue;
        }
        let Some(geometry) = lane_pose.lane_geometry(ordinal) else {
            continue;
        };
        loops.push((name, geometry.points().iter().map(point_to_vec3).collect()));
    }
    let half = LANE_WIDTH_METERS * 0.5;
    for (_, points) in &loops {
        for side in [half, -half] {
            if let Some(mesh) = ribbon_mesh(points, LINE_WIDTH_METERS, MARKING_Y, side, false) {
                commands.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(white.clone())));
            }
        }
    }
    let mut by_corner: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, (name, _)) in loops.iter().enumerate() {
        if let Some(corner) = name.split('-').nth(1) {
            by_corner.entry(corner.to_owned()).or_default().push(index);
        }
    }
    for (_, indices) in by_corner {
        // 外轨：采样点离路口中心的平均平方距离更大者。
        let outer = indices
            .iter()
            .max_by(|left, right| {
                let score = |index: usize| {
                    let points = &loops[index].1;
                    points
                        .iter()
                        .map(|point| point.length_squared())
                        .sum::<f32>()
                        / points.len().max(1) as f32
                };
                score(**left).total_cmp(&score(**right))
            })
            .copied();
        let Some(outer_index) = outer else {
            continue;
        };
        let points = &loops[outer_index].1;
        if points.len() < 2 {
            continue;
        }
        // 外侧方向：中点法向两候选里离路口中心更远的一侧。
        let middle = points.len() / 2;
        let direction = (points[(middle + 1).min(points.len() - 1)]
            - points[middle.saturating_sub(1)])
        .normalize_or_zero();
        let normal = Vec3::new(-direction.z, 0.0, direction.x);
        let side = if (points[middle] + normal).length_squared()
            >= (points[middle] - normal).length_squared()
        {
            1.0
        } else {
            -1.0
        };
        let offset = side * (half + SIDEWALK_WIDTH_METERS * 0.5);
        if let Some(mesh) = ribbon_mesh(points, SIDEWALK_WIDTH_METERS, SIDEWALK_Y, offset, false) {
            commands.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(sidewalk.clone())));
        }
    }
}

/// 3 m 画 / 6 m 空的虚线折线：按弧长切窗口，每个画段出一条 ribbon。
fn dash_ribbons(points: &[Vec3], on: f32, off: f32, width: f32, y: f32) -> Vec<Mesh> {
    let mut lengths = Vec::with_capacity(points.len());
    lengths.push(0.0_f32);
    for pair in points.windows(2) {
        lengths.push(lengths.last().expect("length seed") + (pair[1] - pair[0]).length());
    }
    let total = *lengths.last().expect("sampled line has points");
    let cycle = on + off;
    let mut out = Vec::new();
    let mut start = 0.0_f32;
    while start < total {
        let end = (start + on).min(total);
        let slice = slice_polyline(points, &lengths, start, end);
        if let Some(mesh) = ribbon_mesh(&slice, width, y, 0.0, false) {
            out.push(mesh);
        }
        start += cycle;
    }
    out
}

/// 折线 [s0, s1] 弧长子段（端点插值 + 内部顶点）。
fn slice_polyline(points: &[Vec3], lengths: &[f32], s0: f32, s1: f32) -> Vec<Vec3> {
    let mut out = Vec::new();
    out.push(sample_polyline(points, lengths, s0));
    for index in 1..points.len() {
        if lengths[index] > s0 && lengths[index] < s1 {
            out.push(points[index]);
        }
    }
    out.push(sample_polyline(points, lengths, s1));
    out
}

fn sample_polyline(points: &[Vec3], lengths: &[f32], s: f32) -> Vec3 {
    for index in 1..lengths.len() {
        if s <= lengths[index] {
            let span = lengths[index] - lengths[index - 1];
            let t = if span > 0.0 {
                (s - lengths[index - 1]) / span
            } else {
                0.0
            };
            return points[index - 1].lerp(points[index], t);
        }
    }
    *points.last().expect("sampled line has points")
}

fn pose_transform(pose: CanonicalPoseF32) -> Transform {
    let position = pose.position();
    let tangent = pose.tangent();
    let up = pose.up();
    Transform::from_xyz(position.x(), position.y(), position.z()).looking_to(
        Vec3::new(tangent.x(), tangent.y(), tangent.z()),
        Vec3::new(up.x(), up.y(), up.z()),
    )
}

/// 采样点折线的三角带 ribbon；`lateral_offset` 沿横向单位偏移（车道边线用），
/// `closed` 时把首点追加到尾部闭合。平面 ribbon 法线一律手工朝上，
/// 避免 winding 误差导致背光面全黑。
fn ribbon_mesh(
    points: &[Vec3],
    width: f32,
    y_lift: f32,
    lateral_offset: f32,
    closed: bool,
) -> Option<Mesh> {
    if points.len() < 2 {
        return None;
    }
    let mut line: Vec<Vec3> = points.to_vec();
    if closed {
        line.push(points[0]);
    }
    let half = width * 0.5;
    let mut positions = Vec::with_capacity(line.len() * 2);
    for (index, point) in line.iter().enumerate() {
        let previous = line[index.saturating_sub(1)];
        let next = line[(index + 1).min(line.len() - 1)];
        let direction = (next - previous).normalize_or_zero();
        let lateral_unit = Vec3::new(-direction.z, 0.0, direction.x);
        let base = *point + Vec3::Y * y_lift + lateral_unit * lateral_offset;
        positions.push((base - lateral_unit * half).to_array());
        positions.push((base + lateral_unit * half).to_array());
    }
    let mut indices = Vec::with_capacity((line.len() - 1) * 6);
    for index in 0..line.len() - 1 {
        let base = (index as u32) * 2;
        indices.extend_from_slice(&[base, base + 1, base + 2, base + 1, base + 3, base + 2]);
    }
    let mut mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_indices(Indices::U32(indices));
    let normals = vec![[0.0_f32, 1.0, 0.0]; line.len() * 2];
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    Some(mesh)
}

fn toggle_overlay(keys: Res<ButtonInput<KeyCode>>, mut config: ResMut<JunctionDebugConfig>) {
    if keys.just_pressed(KeyCode::F2) {
        config.enabled = !config.enabled;
        info!(
            "junction debug overlay: {}",
            if config.enabled { "on" } else { "off" }
        );
    }
}

fn sync_vehicle_transforms(
    mut session: ResMut<LaneFlowSession>,
    mut buffer: ResMut<PoseBuffer>,
    metrics: Res<VehicleMetrics>,
    mut transforms: Query<&mut Transform, With<VehicleBox>>,
) {
    if session
        .extract_committed_pose_batch(FramePlacementToken::new(1), &mut buffer.0)
        .is_err()
    {
        return;
    }
    if !session.consumption_context_is_current(buffer.0.context()) {
        return;
    }
    for (vehicle, record) in buffer
        .0
        .vehicles()
        .iter()
        .zip(buffer.0.batch().records().iter())
    {
        let Some(entity) = session.vehicle_entity(*vehicle) else {
            continue;
        };
        if let Ok(mut transform) = transforms.get_mut(entity) {
            let mut target = pose_transform(record.pose());
            let tangent = record.pose().tangent();
            let forward = Vec3::new(tangent.x(), tangent.y(), tangent.z());
            // Runtime pose 是前保险杠位置；表现中心沿切向后移半个车长、
            // 抬升半个车高，让车底落在路面上。
            target.translation -= forward * metrics.half_length;
            target.translation += Vec3::Y * (VEHICLE_HEIGHT_METERS * 0.5 + 0.02);
            *transform = target;
        }
    }
}

/// 静态标记（Gate/Waiting/Conflict/Signal/高亮根）的构建与缓存重建。
fn maintain_static_overlay(
    mut commands: Commands,
    config: Res<JunctionDebugConfig>,
    session: Res<LaneFlowSession>,
    mut cache: ResMut<StaticOverlayCache>,
    roots: Query<Entity, With<DebugStaticRoot>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let context = session.junction_observation().context();
    let current_key = (context.network_revision(), context.world_generation());
    if !config.enabled {
        if cache.key.is_some() {
            for entity in &roots {
                commands.entity(entity).despawn();
            }
            cache.key = None;
        }
        return;
    }
    if cache.key == Some(current_key) {
        return;
    }
    for entity in &roots {
        commands.entity(entity).despawn();
    }
    cache.key = Some(current_key);

    let world = session.world();
    let revision = world.revision();
    let gate_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.95, 0.85, 0.2),
        ..default()
    });
    let waiting_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.2, 0.85, 0.9),
        double_sided: true,
        ..default()
    });
    let conflict_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.95, 0.45, 0.15),
        double_sided: true,
        ..default()
    });
    let highlight_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.3, 0.95, 0.4),
        double_sided: true,
        ..default()
    });

    let mut identity_lines: Vec<String> = Vec::new();
    let Some(spatial) = revision.spatial() else {
        identity_lines.push("no spatial component".to_owned());
        return spawn_identity_text(&mut commands, identity_lines);
    };
    let Some(lane_pose) = spatial.lane_pose() else {
        identity_lines.push("no lane pose network".to_owned());
        return spawn_identity_text(&mut commands, identity_lines);
    };

    // 静态 Gate 标记：遍历 live 路线 hop，画面按静态 Gate ordinal 去重（仅去重标记，
    // 不合并决策/预约）。
    let mut placed_gates: HashMap<u32, Vec3> = HashMap::new();
    for route in world.live_routes() {
        let Some(edges) = world.route_edges(route) else {
            continue;
        };
        for hop in 0..u32::try_from(edges.len()).unwrap_or(0) {
            let Some(observation) = world.route_gate(route, hop) else {
                continue;
            };
            let gate = observation.gate().raw();
            if placed_gates.contains_key(&gate) {
                continue;
            }
            let Some(geometry) = lane_pose.lane_geometry(observation.edge()) else {
                continue;
            };
            let Some(point) = geometry.points().last() else {
                continue;
            };
            placed_gates.insert(gate, point_to_vec3(point));
        }
    }
    let gate_mesh = meshes.add(Cuboid::new(1.2, 3.5, 1.2));
    for position in placed_gates.values() {
        commands.spawn((
            Mesh3d(gate_mesh.clone()),
            MeshMaterial3d(gate_material.clone()),
            Transform::from_translation(*position + Vec3::Y * 1.75),
            DebugStaticRoot,
        ));
    }

    // Waiting 区间 ribbon：entry→release Gate 之间的机动路径边子段。
    let traffic = revision.traffic();
    let relations = traffic.relations();
    let maneuvers = traffic.maneuvers();
    let zone_count = traffic.entity_counts().count(EntityKind::WaitingZone);
    for raw in 0..zone_count {
        let zone = WaitingZoneOrdinal::from_raw(raw);
        let Some(zone_view) = relations.waiting_zone(zone) else {
            continue;
        };
        let Some(path) = maneuvers.maneuver_path(zone_view.path()) else {
            continue;
        };
        let Some(entry) = relations
            .maneuver_gate(zone_view.entry_gate())
            .map(|gate| gate.transition_index())
        else {
            continue;
        };
        let Some(release) = relations
            .maneuver_gate(zone_view.release_gate())
            .map(|gate| gate.transition_index())
        else {
            continue;
        };
        let edges = path.edges();
        let (Some(start), Some(end)) = (
            usize::try_from(entry)
                .ok()
                .filter(|index| *index < edges.len()),
            usize::try_from(release)
                .ok()
                .filter(|index| *index < edges.len()),
        ) else {
            continue;
        };
        // transition i 的门边界位于 edges i 与 i+1 之间：待转区间从 entry 边界后
        // 一条边开始，到 release 边界所在的前一条边为止。
        let Some(start) = start.checked_add(1).filter(|index| *index <= end) else {
            continue;
        };
        let mut points = Vec::new();
        for edge in &edges[start..=end] {
            if let Some(geometry) = lane_pose.lane_geometry(*edge) {
                points.extend(geometry.points().iter().map(point_to_vec3));
            }
        }
        if let Some(mesh) = ribbon_mesh(&points, 1.5, 0.25, 0.0, false) {
            commands.spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(waiting_material.clone()),
                DebugStaticRoot,
            ));
        }
    }

    // Conflict 区域 ring_xz 描边；无区域几何时退化为带身份的文字标记。
    let conflict_count = traffic.entity_counts().count(EntityKind::ConflictZone);
    for raw in 0..conflict_count {
        let zone = ConflictZoneOrdinal::from_raw(raw);
        match spatial.conflict_zone_region(zone) {
            Some(region) => {
                let (min_y, max_y) = region.height_range();
                let base_y = (min_y + max_y) * 0.5;
                let points: Vec<Vec3> = region
                    .ring_xz()
                    .iter()
                    .map(|point| Vec3::new(point.x, base_y, point.z))
                    .collect();
                if let Some(mesh) = ribbon_mesh(&points, 0.8, 0.4, 0.0, true) {
                    commands.spawn((
                        Mesh3d(meshes.add(mesh)),
                        MeshMaterial3d(conflict_material.clone()),
                        DebugStaticRoot,
                    ));
                }
            }
            None => {
                identity_lines.push(format!("conflict-zone-{raw} (no region)"));
            }
        }
    }
    if !identity_lines.is_empty() {
        spawn_identity_text(&mut commands, identity_lines);
    }

    // 信号状态点：预建红/黄/绿/未知共享材质，逐点按 committed aspect 选派句柄
    // （不改写共享 asset，避免所有点显示同一颜色）。
    let group_count = traffic.entity_counts().count(EntityKind::SignalGroup);
    let dot_mesh = meshes.add(Sphere::new(1.1).mesh().build());
    let red = materials.add(StandardMaterial::from_color(Color::srgb(0.9, 0.1, 0.1)));
    let yellow = materials.add(StandardMaterial::from_color(Color::srgb(0.95, 0.85, 0.1)));
    let green = materials.add(StandardMaterial::from_color(Color::srgb(0.1, 0.9, 0.2)));
    let unknown = materials.add(StandardMaterial::from_color(Color::srgb(0.35, 0.35, 0.38)));
    for raw in 0..group_count {
        let group = SignalGroupOrdinal::from_raw(raw);
        let Some(group_view) = relations.signal_group(group) else {
            continue;
        };
        let Some(first_gate) = group_view.gates().first().copied() else {
            continue;
        };
        let Some(gate_view) = relations.maneuver_gate(first_gate) else {
            continue;
        };
        let Some(stop) = relations.stop_line(gate_view.stop_line()) else {
            continue;
        };
        let Some(geometry) = lane_pose.lane_geometry(stop.edge()) else {
            continue;
        };
        let Some(point) = geometry.points().last() else {
            continue;
        };
        commands.spawn((
            Mesh3d(dot_mesh.clone()),
            MeshMaterial3d(red.clone()),
            Transform::from_translation(point_to_vec3(point) + Vec3::Y * 3.5),
            SignalDotMarker {
                group: raw,
                red: red.clone(),
                yellow: yellow.clone(),
                green: green.clone(),
                unknown: unknown.clone(),
            },
            DebugStaticRoot,
        ));
    }

    // 选定车辆高亮框（动态定位，见 `apply_overlay_visuals`）。
    let length = profile_length(&world.revision()) as f32 / 1_000.0;
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(2.4, 0.12, length + 1.2))),
        MeshMaterial3d(highlight_material),
        Transform::IDENTITY,
        SelectedVehicleMarker,
        DebugStaticRoot,
    ));
}

fn profile_length(revision: &SharedNetworkRevision) -> u32 {
    revision
        .traffic()
        .relations()
        .vehicle_profile(laneflow_static_contract::VehicleProfileOrdinal::from_raw(0))
        .map_or(4_500, |profile| profile.length_mm())
}

/// 无区域几何时以屏幕空间 UI 文字列出带身份的标记（§4：不计算几何相交补冲突事实）。
fn spawn_identity_text(commands: &mut Commands, lines: Vec<String>) {
    commands.spawn((
        Text::new(lines.join("\n")),
        TextFont::from_font_size(16.0),
        TextColor(Color::srgb(0.95, 0.5, 0.2)),
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(12.0),
            top: Val::Px(12.0),
            ..default()
        },
        DebugStaticRoot,
    ));
}

/// 每拍在 Observe 阶段消费 `junction_observation`：按车聚合决策（每拍对批次建
/// 一次索引），重写面板内容、选定车辆与信号状态。只读 Session。
fn observe_overlay(
    config: Res<JunctionDebugConfig>,
    session: Res<LaneFlowSession>,
    plan: Res<SpawnedPlan>,
    mut panel: ResMut<JunctionDebugPanel>,
) {
    if !config.enabled {
        panel.content.clear();
        panel.selected = None;
        panel.signals.clear();
        return;
    }
    let view = session.junction_observation();
    let context = view.context();
    let mut content = String::new();
    let _ = writeln!(
        content,
        "#285 junction debug | F2 overlay | tick={} gen={}",
        context.tick_index(),
        context.world_generation().get()
    );

    let waiting = view.latest_waiting_decisions();
    let conflict = view.latest_conflict_decisions();
    // 按车聚合：每拍对整个决策批次建一次可复用索引（观测合同 §3）。
    let mut by_vehicle: HashMap<VehicleHandle, Vec<String>> = HashMap::new();
    for decision in waiting {
        by_vehicle
            .entry(decision.vehicle())
            .or_default()
            .push(format!(
                "wait hop{} = {:?}",
                decision.anchor().hop(),
                decision.outcome()
            ));
    }
    for decision in conflict {
        by_vehicle
            .entry(decision.vehicle())
            .or_default()
            .push(format!(
                "conflict hop{} = {:?}",
                decision.anchor().hop(),
                decision.outcome()
            ));
    }

    // 选定车辆：只在 Active（有当前 committed pose）的车里选——本拍有决策的首辆，
    // 否则首辆；Completed 车不再停留，避免高亮/面板锁在已完成车辆上。
    let rows: Vec<_> = view.vehicles().collect();
    let active: Vec<_> = rows
        .iter()
        .filter(|row| row.state().status() == VehicleStatus::Active)
        .collect();
    let selected = active
        .iter()
        .find(|row| by_vehicle.contains_key(&row.vehicle()))
        .or_else(|| active.first())
        .map(|row| row.vehicle());
    panel.selected = selected;

    if let Some(vehicle) = selected {
        let role = plan
            .0
            .iter()
            .find(|spawned| spawned.vehicle == vehicle)
            .map_or("vehicle", |spawned| spawned.role.tag());
        let _ = writeln!(content, "selected={vehicle:?} role={role}");
        if let Some(row) = rows.iter().find(|row| row.vehicle() == vehicle) {
            let state = row.state();
            match state.maneuver_traversal() {
                Some(traversal) => {
                    let _ = writeln!(
                        content,
                        "phase={:?} occurrence={} route={:?}",
                        traversal.phase(),
                        traversal.maneuver_occurrence_index(),
                        traversal.route()
                    );
                }
                None => {
                    let _ = writeln!(content, "phase=none");
                }
            }
            match state.waiting_membership() {
                Some(membership) => {
                    let _ = writeln!(
                        content,
                        "member=zone{} seq={} release_hop={}",
                        membership.waiting_zone().raw(),
                        membership.admission_sequence(),
                        membership.release_hop()
                    );
                }
                None => {
                    let _ = writeln!(content, "member=none");
                }
            }
            match by_vehicle.get(&vehicle) {
                Some(lines) if !lines.is_empty() => {
                    for line in lines {
                        let _ = writeln!(content, "{line}");
                    }
                }
                _ => {
                    let _ = writeln!(content, "decisions=this tick none");
                }
            }
            match row.conflict_reservation() {
                Some(reservation) => {
                    let _ = writeln!(
                        content,
                        "reservation=route{:?} occurrence={} admission_hop={} acquired_tick={}",
                        reservation.route(),
                        reservation.maneuver_occurrence_index(),
                        reservation.admission_gate_hop(),
                        reservation.acquired_tick()
                    );
                }
                None => {
                    let _ = writeln!(content, "reservation=none");
                }
            }
            // 未来 Gate 列表：从当前路线边下标起的 route/hop 定位（不按静态 Gate 折叠）。
            let route = state.route();
            if let Some(edges) = session.world().route_edges(route) {
                let mut gates = Vec::new();
                let start = usize::try_from(state.route_edge_index()).unwrap_or(0);
                for (offset, _) in edges.iter().enumerate().skip(start) {
                    let Ok(hop) = u32::try_from(offset) else {
                        break;
                    };
                    if let Some(observation) = view.route_gate(route, hop) {
                        gates.push(format!("hop{}:g{}", hop, observation.gate().raw()));
                    }
                }
                if gates.len() > 8 {
                    gates.truncate(8);
                    gates.push("...".to_owned());
                }
                let _ = writeln!(content, "gates_ahead={}", gates.join(" "));
            }
        }
    } else {
        let _ = writeln!(content, "selected=none");
    }

    panel.signals = session
        .world()
        .committed_signal_groups()
        .as_slice()
        .iter()
        .map(|(group, aspect)| (group.raw(), *aspect))
        .collect();
    let signal_text: Vec<String> = panel
        .signals
        .iter()
        .map(|(group, aspect)| format!("g{group}:{aspect:?}"))
        .collect();
    let _ = writeln!(content, "signals={}", signal_text.join(" "));

    panel.content = content;
}

fn apply_overlay_visuals(
    config: Res<JunctionDebugConfig>,
    buffer: Res<PoseBuffer>,
    panel: Res<JunctionDebugPanel>,
    metrics: Res<VehicleMetrics>,
    mut panel_text: Query<&mut Text, With<PanelText>>,
    mut dots: Query<(&SignalDotMarker, &mut MeshMaterial3d<StandardMaterial>)>,
    mut highlight: Query<(&mut Transform, &mut Visibility), With<SelectedVehicleMarker>>,
) {
    if let Ok(mut text) = panel_text.single_mut() {
        text.0 = if config.enabled {
            panel.content.clone()
        } else {
            String::new()
        };
    }
    if !config.enabled {
        return;
    }
    // 逐点选派预建共享材质句柄；不就地改写共享 asset。
    for (marker, mut material) in &mut dots {
        let aspect = panel
            .signals
            .iter()
            .find(|(group, _)| *group == marker.group)
            .map(|(_, aspect)| *aspect);
        let handle = match aspect {
            Some(SignalAspect::Green) => marker.green.clone(),
            Some(SignalAspect::Yellow) => marker.yellow.clone(),
            Some(SignalAspect::Red) => marker.red.clone(),
            _ => marker.unknown.clone(),
        };
        *material = MeshMaterial3d(handle);
    }
    // 选中车无当前 pose（如 Completed 车不在 committed 批次里）时隐藏高亮，
    // 而不是把它挪到世界原点。
    let selected_position = panel.selected.and_then(|vehicle| {
        buffer
            .0
            .vehicles()
            .iter()
            .position(|candidate| *candidate == vehicle)
            .and_then(|index| buffer.0.batch().records().get(index))
            .map(|record| {
                let mut target = pose_transform(record.pose());
                let tangent = record.pose().tangent();
                let forward = Vec3::new(tangent.x(), tangent.y(), tangent.z());
                target.translation -= forward * metrics.half_length;
                // 高亮框贴地（不随车体抬升）。
                target.translation -= Vec3::Y * (VEHICLE_HEIGHT_METERS * 0.5 + 0.02);
                target
            })
    });
    if let Ok((mut transform, mut visibility)) = highlight.single_mut() {
        match selected_position {
            Some(target) => {
                *transform = target;
                *visibility = Visibility::Visible;
            }
            None => {
                *visibility = Visibility::Hidden;
            }
        }
    }
}
