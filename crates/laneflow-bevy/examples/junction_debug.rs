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
    WaitingZoneOrdinal,
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
/// 路口内部沥青铺装 patch 边长（米）；junction_radius 30 m 量级外扩。
const JUNCTION_PATCH_METERS: f32 = 64.0;
/// 道路/标线/铺装的错层高度（米），避免共面 z-fight。
const ROAD_Y: f32 = 0.03;
const MARKING_Y: f32 = 0.06;
const MARKING_ALT_Y: f32 = 0.065;
const PATCH_Y: f32 = 0.015;
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
/// （车道几何核查用）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CameraPreset {
    Persp,
    Topdown,
}

impl CameraPreset {
    fn tag(self) -> &'static str {
        match self {
            Self::Persp => "persp",
            Self::Topdown => "topdown",
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
                    brightness: 120.0,
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
        brightness: 40.0,
        ..default()
    });
    // 地面：大薄板，顶面在 y=0。深草绿压低明度，与沥青拉开对比。
    let ground_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.24, 0.38, 0.22),
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(GROUND_SIZE_METERS, 0.2, GROUND_SIZE_METERS))),
        MeshMaterial3d(ground_material),
        Transform::from_xyz(0.0, -0.1, 0.0),
    ));
    // 路口内部沥青铺装 patch，与臂道 ribbon 错层避免 z-fight。
    let asphalt_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.11, 0.11, 0.12),
        double_sided: true,
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(
            JUNCTION_PATCH_METERS,
            0.02,
            JUNCTION_PATCH_METERS,
        ))),
        MeshMaterial3d(asphalt_material.clone()),
        Transform::from_xyz(0.0, PATCH_Y - 0.01, 0.0),
    ));
    // 道路：每条 lane edge 一条沥青 ribbon + 两侧车道边白线。
    let revision = session.world().revision();
    let marking_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.9, 0.9, 0.88),
        double_sided: true,
        ..default()
    });
    let mut edge_count = 0_u32;
    let mut loops = 0_u32;
    let mut arms = 0_u32;
    let mut internal = 0_u32;
    let mut bounds_min = Vec3::splat(f32::MAX);
    let mut bounds_max = Vec3::splat(f32::MIN);
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
            // 车道边白线：两侧各一条；两侧 y 微差，使相邻车道共线白线不 z-fight。
            // lane_geometry 无对向归属信息，按设计约定全部白线。环路（弧长 > 300 m）
            // 只画沥青不画线，整圈标线视觉上太乱。
            let half = LANE_WIDTH_METERS * 0.5;
            if arc <= 300.0 {
                if let Some(mesh) = ribbon_mesh(&points, 0.12, MARKING_Y, half, false) {
                    commands.spawn((
                        Mesh3d(meshes.add(mesh)),
                        MeshMaterial3d(marking_material.clone()),
                    ));
                }
                if let Some(mesh) = ribbon_mesh(&points, 0.12, MARKING_ALT_Y, -half, false) {
                    commands.spawn((
                        Mesh3d(meshes.add(mesh)),
                        MeshMaterial3d(marking_material.clone()),
                    ));
                }
            }
        }
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
        let right = direction.cross(Vec3::Y).normalize_or_zero();
        let pole_base = position + right * (LANE_WIDTH_METERS * 0.5 + 1.2);
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
