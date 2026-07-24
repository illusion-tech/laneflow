//! #189 使用 production artifacts 的 v0.8 signalized-corridor native example。

use std::{
    collections::HashSet,
    env,
    error::Error,
    ffi::OsString,
    fs, io,
    num::NonZeroU32,
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

use bevy::{
    ecs::world::{Mut, World},
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll},
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
    window::{Window, WindowPlugin},
};
use laneflow_bevy::{
    LaneFlowDebugCenterlines, LaneFlowDebugGizmosConfig, LaneFlowDebugGizmosPlugin, LaneFlowFixed,
    LaneFlowFixedSet, LaneFlowFramePlacement, LaneFlowPlugin, LaneFlowSession,
    LaneFlowSessionConfig, LaneFlowVehicleReplaceOutcome, replace_completed_vehicle,
};
use laneflow_core::{
    CoreWorld, EdgeProgress, SignalAspect, SignalControl, SignalGroupHandle, VehicleReplaceRecord,
};
use laneflow_data::{NamedArtifact, from_scenario_json_slice};
use laneflow_scenario::signalized_corridor::{
    CorridorCatalog, CorridorPopulationConfig, CorridorPopulationController,
    CorridorPopulationPrepare, CorridorReplaceApplyError, CorridorReplaceAttemptOutcome,
    DEFAULT_SEED, DEFAULT_TARGET_VEHICLE_COUNT,
};
use laneflow_spatial::{
    CanonicalPoint3F32, FramePlacementToken, SpatialEdgeInput, SpatialRegistry,
};
use serde::Deserialize;

const CONFIG_VERSION: &str = "0.1";
const PROFILE_ID: &str = "passenger-car";
const MAX_CATCH_UP_STEPS: u32 = 8;
const WINDOW_WIDTH: u32 = 1_600;
const WINDOW_HEIGHT: u32 = 900;
const BASE_WINDOW_TITLE: &str = "LaneFlow #189 Signalized Corridor";
const CAMERA_MAX_MOTION_PER_FRAME: f32 = 64.0;
const CAMERA_ORBIT_SENSITIVITY: f32 = 0.004;
const CAMERA_PAN_SENSITIVITY: f32 = 0.0005;
const CAMERA_ZOOM_SENSITIVITY: f32 = 0.12;
const CAMERA_MIN_PITCH_RADIANS: f32 = 0.18;
const CAMERA_MAX_PITCH_RADIANS: f32 = 1.42;
const PERFORMANCE_SAMPLE_WINDOW: Duration = Duration::from_secs(1);
const HUD_REFRESH_INTERVAL: Duration = Duration::from_millis(350);

fn main() -> Result<(), Box<dyn Error>> {
    let action = parse_args(env::args_os().skip(1))?;
    let CliAction::Run(args) = action else {
        print_help();
        return Ok(());
    };
    let bootstrap = load_corridor_runtime(&args)?;
    let vehicle_count = args.population.target_vehicle_count();
    let seed = args.population.seed();
    let screenshot_path = format!("laneflow-signalized-corridor-{vehicle_count}-seed-{seed}.png");
    let metadata = RuntimeMetadata {
        vehicle_count,
        seed,
        screenshot_path,
    };
    let mut debug_config = LaneFlowDebugGizmosConfig::enabled(256, 128);
    debug_config.enabled = false;
    debug_config.frame_axes_length = 12.0;
    debug_config.position_marker_size = 0.6;
    debug_config.direction_marker_length = 2.0;

    App::new()
        .insert_resource(ClearColor(Color::srgb(0.025, 0.035, 0.045)))
        .insert_resource(bootstrap.session)
        .insert_resource(CorridorPopulationRuntime::new(bootstrap.controller))
        .insert_resource(bootstrap.scene)
        .insert_resource(bootstrap.centerlines)
        .insert_resource(metadata)
        .insert_resource(RuntimePerformance::default())
        .insert_resource(debug_config)
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: BASE_WINDOW_TITLE.to_owned(),
                resolution: (WINDOW_WIDTH, WINDOW_HEIGHT).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins((LaneFlowPlugin, LaneFlowDebugGizmosPlugin))
        .add_systems(
            LaneFlowFixed,
            (
                (apply_population_pending, begin_lane_flow_step_measurement)
                    .chain()
                    .in_set(LaneFlowFixedSet::Lifecycle),
                (end_lane_flow_step_measurement, consume_population_step)
                    .chain()
                    .in_set(LaneFlowFixedSet::Observe),
            ),
        )
        .add_systems(Startup, setup_scene)
        .add_systems(
            Update,
            (
                update_signal_lamps,
                (sample_runtime_performance, update_runtime_hud).chain(),
                update_orbit_camera,
                toggle_debug_gizmos,
                capture_screenshot,
                report_adapter_error,
            ),
        )
        .run();

    Ok(())
}

#[derive(Clone, Debug)]
struct RunArgs {
    population: CorridorPopulationConfig,
    config_path: PathBuf,
}

#[derive(Clone, Debug)]
enum CliAction {
    Help,
    Run(RunArgs),
}

fn parse_args<I>(args: I) -> Result<CliAction, io::Error>
where
    I: IntoIterator<Item = OsString>,
{
    let mut args = args.into_iter();
    let mut vehicles = None;
    let mut seed = None;
    let mut config_path = None;
    let mut help = false;

    while let Some(argument) = args.next() {
        let flag = argument
            .to_str()
            .ok_or_else(|| invalid_input("CLI flag 必须是有效 UTF-8"))?;
        match flag {
            "-h" | "--help" => {
                if help {
                    return Err(invalid_input("重复的 --help"));
                }
                help = true;
            }
            "--vehicles" => {
                if vehicles.is_some() {
                    return Err(invalid_input("重复的 --vehicles"));
                }
                let value = required_utf8_value(&mut args, "--vehicles")?;
                vehicles = Some(value.parse::<usize>().map_err(|source| {
                    invalid_input(format!("无法解析 --vehicles {value:?}：{source}"))
                })?);
            }
            "--seed" => {
                if seed.is_some() {
                    return Err(invalid_input("重复的 --seed"));
                }
                let value = required_utf8_value(&mut args, "--seed")?;
                seed = Some(value.parse::<u64>().map_err(|source| {
                    invalid_input(format!("无法解析 --seed {value:?}：{source}"))
                })?);
            }
            "--config" => {
                if config_path.is_some() {
                    return Err(invalid_input("重复的 --config"));
                }
                config_path = Some(
                    args.next()
                        .ok_or_else(|| invalid_input("--config 缺少 path"))?
                        .into(),
                );
            }
            _ => return Err(invalid_input(format!("未知 CLI 参数 {flag:?}"))),
        }
    }

    if help {
        if vehicles.is_some() || seed.is_some() || config_path.is_some() {
            return Err(invalid_input("--help 不能与运行参数组合"));
        }
        return Ok(CliAction::Help);
    }

    let population = CorridorPopulationConfig::try_new(
        vehicles.unwrap_or(DEFAULT_TARGET_VEHICLE_COUNT),
        seed.unwrap_or(DEFAULT_SEED),
    )
    .map_err(|source| invalid_input(source.to_string()))?;
    Ok(CliAction::Run(RunArgs {
        population,
        config_path: config_path.unwrap_or_else(default_config_path),
    }))
}

fn required_utf8_value<I>(args: &mut I, flag: &str) -> Result<String, io::Error>
where
    I: Iterator<Item = OsString>,
{
    args.next()
        .ok_or_else(|| invalid_input(format!("{flag} 缺少 value")))?
        .into_string()
        .map_err(|_| invalid_input(format!("{flag} value 必须是有效 UTF-8")))
}

fn default_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/config/v0.8-signalized-corridor.toml")
}

fn print_help() {
    println!(
        "{BASE_WINDOW_TITLE}\n\n\
         Usage:\n  \
         cargo +1.96.0 run -p laneflow-bevy --example signalized_corridor \
         --features native-example -- [OPTIONS]\n\n\
         Options:\n  \
         --vehicles <50..=200>  logical population（default: 100）\n  \
         --seed <u64>           deterministic replay seed（default: 0）\n  \
         --config <path>        checked-in authoring/startup config\n  \
         -h, --help             print help"
    );
}

#[derive(Debug, Deserialize)]
struct StartupConfig {
    corridor_config_version: String,
    fixed_delta_ms: u64,
    geometry: StartupGeometry,
    output: StartupOutput,
}

#[derive(Debug, Deserialize)]
struct StartupGeometry {
    lane_width_meters: f32,
}

#[derive(Debug, Deserialize)]
struct StartupOutput {
    directory: String,
    traffic_artifact_ref: String,
    spatial_artifact_ref: String,
    manifest_file_name: String,
    catalog_file_name: String,
}

impl StartupConfig {
    fn parse(input: &str) -> Result<Self, io::Error> {
        let config: Self = toml::from_str(input)
            .map_err(|source| invalid_data(format!("无法解析 corridor config：{source}")))?;
        if config.corridor_config_version != CONFIG_VERSION {
            return Err(invalid_data(format!(
                "不支持 corridor_config_version {:?}；期望 {CONFIG_VERSION:?}",
                config.corridor_config_version
            )));
        }
        if config.fixed_delta_ms == 0 {
            return Err(invalid_data("fixed_delta_ms 必须大于 0"));
        }
        if !config.geometry.lane_width_meters.is_finite()
            || config.geometry.lane_width_meters <= 0.0
        {
            return Err(invalid_data("geometry.lane_width_meters 必须是有限正数"));
        }
        if config.output.directory.trim().is_empty() {
            return Err(invalid_data("output.directory 不能为空"));
        }
        for value in [
            &config.output.traffic_artifact_ref,
            &config.output.spatial_artifact_ref,
            &config.output.manifest_file_name,
            &config.output.catalog_file_name,
        ] {
            if !is_single_file_name(value) {
                return Err(invalid_data(format!(
                    "输出文件名 {value:?} 必须是单一非空 path component"
                )));
            }
        }
        Ok(config)
    }

    fn output_directory(&self, config_path: &Path) -> Result<PathBuf, io::Error> {
        let output = PathBuf::from(&self.output.directory);
        if output.is_absolute() {
            return Ok(output);
        }
        let parent = config_path.parent().ok_or_else(|| {
            invalid_input(format!("config path {} 没有 parent", config_path.display()))
        })?;
        Ok(parent.join(output))
    }
}

fn is_single_file_name(value: &str) -> bool {
    let path = Path::new(value);
    !value.is_empty()
        && path.components().count() == 1
        && matches!(path.components().next(), Some(Component::Normal(_)))
}

struct CorridorBootstrap {
    session: LaneFlowSession,
    controller: CorridorPopulationController,
    scene: CorridorScene,
    centerlines: LaneFlowDebugCenterlines,
}

fn load_corridor_runtime(args: &RunArgs) -> Result<CorridorBootstrap, Box<dyn Error>> {
    let config_text = fs::read_to_string(&args.config_path).map_err(|source| {
        io::Error::new(
            source.kind(),
            format!(
                "无法读取 corridor config {}：{source}",
                args.config_path.display()
            ),
        )
    })?;
    let config = StartupConfig::parse(&config_text)?;
    let data_dir = config.output_directory(&args.config_path)?;
    let manifest = read_artifact(&data_dir.join(&config.output.manifest_file_name))?;
    let traffic_bytes = read_artifact(&data_dir.join(&config.output.traffic_artifact_ref))?;
    let spatial_bytes = read_artifact(&data_dir.join(&config.output.spatial_artifact_ref))?;
    let catalog_text = fs::read_to_string(data_dir.join(&config.output.catalog_file_name))
        .map_err(|source| {
            io::Error::new(
                source.kind(),
                format!(
                    "无法读取 corridor catalog {}：{source}",
                    data_dir.join(&config.output.catalog_file_name).display()
                ),
            )
        })?;

    let loaded = from_scenario_json_slice(
        &manifest,
        &[
            NamedArtifact::new(&config.output.traffic_artifact_ref, &traffic_bytes),
            NamedArtifact::new(&config.output.spatial_artifact_ref, &spatial_bytes),
        ],
    )
    .map_err(|source| {
        invalid_data(format!(
            "无法加载 corridor scenario 或校验引用制品（目录 {}）：{source}",
            data_dir.display()
        ))
    })?;
    let (traffic, loaded_spatial) = loaded.into_parts();
    let traffic = traffic.into_initial_traffic_data();
    let profile = traffic
        .vehicle_profiles()
        .profile_handle(PROFILE_ID)
        .ok_or_else(|| invalid_data(format!("corridor traffic 缺少 {PROFILE_ID} profile")))?;
    let catalog = CorridorCatalog::parse(&catalog_text)
        .map_err(|source| invalid_data(format!("无法解析 corridor catalog：{source}")))?
        .normalize(&traffic)
        .map_err(|source| invalid_data(format!("无法规范化 corridor catalog：{source}")))?;
    let mut prepared =
        CorridorPopulationPrepare::prepare(args.population, catalog, &traffic, profile)
            .map_err(|source| invalid_data(format!("无法准备 corridor population：{source}")))?;

    let graph = traffic.lane_graph().clone();
    let frame_id = loaded_spatial.frame_id().clone();
    let scene_edges = loaded_spatial
        .edges()
        .iter()
        .map(|edge| SceneEdge {
            points: edge.points().iter().copied().map(canonical_point).collect(),
        })
        .collect::<Vec<_>>();
    let centerlines = LaneFlowDebugCenterlines::new(
        frame_id.clone(),
        loaded_spatial
            .edges()
            .iter()
            .map(|edge| edge.points().to_vec())
            .collect(),
    );
    let spatial = SpatialRegistry::try_new(
        &graph,
        frame_id,
        loaded_spatial
            .edges()
            .iter()
            .map(|edge| SpatialEdgeInput::new(edge.edge(), edge.points())),
    )
    .map_err(|source| invalid_data(format!("无法构造 corridor Spatial registry：{source}")))?;
    let core = CoreWorld::with_traffic_data(
        config.fixed_delta_ms,
        traffic,
        prepared.take_initial_vehicles(),
    )
    .map_err(|source| invalid_data(format!("无法构造 corridor Core world：{source}")))?;
    let controller = prepared
        .bind(&core)
        .map_err(|source| invalid_data(format!("无法绑定 corridor population：{source}")))?;
    let signal_stops = build_signal_stop_visuals(&core, &spatial)?;
    let bounds = SceneBounds::from_edges(&scene_edges)
        .ok_or_else(|| invalid_data("corridor Spatial package 没有可渲染点"))?;
    let scene = CorridorScene {
        edges: scene_edges,
        signal_stops,
        bounds,
        lane_width: config.geometry.lane_width_meters,
    };
    let session_config =
        LaneFlowSessionConfig::new(NonZeroU32::new(MAX_CATCH_UP_STEPS).expect("non-zero literal"));
    let session = LaneFlowSession::with_pose_capacity(
        core,
        spatial,
        session_config,
        args.population.target_vehicle_count(),
    );

    Ok(CorridorBootstrap {
        session,
        controller,
        scene,
        centerlines,
    })
}

fn read_artifact(path: &Path) -> io::Result<Vec<u8>> {
    fs::read(path).map_err(|source| {
        io::Error::new(
            source.kind(),
            format!("无法读取 LaneFlow 示例制品 {}：{source}", path.display()),
        )
    })
}

fn build_signal_stop_visuals(
    core: &CoreWorld,
    spatial: &SpatialRegistry,
) -> Result<Vec<SignalStopVisual>, Box<dyn Error>> {
    let signals = core.signals();
    let mut seen = HashSet::new();
    let mut visuals = Vec::new();
    for gate in signals.movement_gates() {
        let stop_line = signals
            .movement_gate_stop_line(gate)
            .ok_or_else(|| invalid_data("normalized MovementGate 缺少 StopLine"))?;
        if !seen.insert(stop_line) {
            continue;
        }
        let SignalControl::Group(group) = signals
            .movement_gate_control(gate)
            .ok_or_else(|| invalid_data("normalized MovementGate 缺少 signal control"))?
        else {
            continue;
        };
        let edge = signals
            .stop_line_edge(stop_line)
            .ok_or_else(|| invalid_data("normalized StopLine 缺少 edge"))?;
        let length = core
            .lane_graph()
            .edge_length(edge)
            .ok_or_else(|| invalid_data("StopLine edge handle 已 stale"))?;
        let pose = spatial
            .sample(edge, EdgeProgress::try_new(length.value())?)
            .map_err(|source| invalid_data(format!("无法采样 StopLine pose：{source}")))?;
        let tangent = canonical_direction(pose.tangent());
        visuals.push(SignalStopVisual {
            group,
            position: canonical_point(pose.position()),
            tangent,
        });
    }
    Ok(visuals)
}

#[derive(Resource)]
struct CorridorScene {
    edges: Vec<SceneEdge>,
    signal_stops: Vec<SignalStopVisual>,
    bounds: SceneBounds,
    lane_width: f32,
}

struct SceneEdge {
    points: Vec<Vec3>,
}

#[derive(Clone, Copy)]
struct SignalStopVisual {
    group: SignalGroupHandle,
    position: Vec3,
    tangent: Vec3,
}

#[derive(Clone, Copy)]
struct SceneBounds {
    min: Vec3,
    max: Vec3,
}

impl SceneBounds {
    fn from_edges(edges: &[SceneEdge]) -> Option<Self> {
        let mut points = edges.iter().flat_map(|edge| edge.points.iter()).copied();
        let first = points.next()?;
        let mut min = first;
        let mut max = first;
        for point in points {
            min = min.min(point);
            max = max.max(point);
        }
        Some(Self { min, max })
    }

    fn center(self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    fn span(self) -> f32 {
        (self.max.x - self.min.x)
            .max(self.max.z - self.min.z)
            .max(1.0)
    }
}

#[derive(Resource)]
struct CorridorPopulationRuntime {
    controller: CorridorPopulationController,
    completed: u64,
    replaced: u64,
    blocked: u64,
}

impl CorridorPopulationRuntime {
    fn new(controller: CorridorPopulationController) -> Self {
        Self {
            controller,
            completed: 0,
            replaced: 0,
            blocked: 0,
        }
    }
}

fn apply_population_pending(world: &mut World) {
    let result = world.resource_scope(
        |world, mut population: Mut<'_, CorridorPopulationRuntime>| {
            population.controller.apply_pending(|old, input| {
                replace_completed_vehicle(world, old, input).map(|outcome| match outcome {
                    LaneFlowVehicleReplaceOutcome::Replaced(record) => {
                        CorridorReplaceAttemptOutcome::Replaced(VehicleReplaceRecord {
                            old: record.old,
                            new: record.new,
                        })
                    }
                    LaneFlowVehicleReplaceOutcome::Blocked(block) => {
                        CorridorReplaceAttemptOutcome::Blocked(block)
                    }
                    _ => unreachable!(
                        "laneflow-bevy and signalized-corridor example are released in lockstep"
                    ),
                })
            })
        },
    );

    match result {
        Ok(report) => {
            let mut population = world.resource_mut::<CorridorPopulationRuntime>();
            population.replaced += report.replaced as u64;
            population.blocked += report.blocked as u64;
        }
        Err(CorridorReplaceApplyError::Host(_)) => {
            // `replace_completed_vehicle` 已把 fatal host error 写入 Session；
            // fixed driver 会停止 Step/Observe 与后续 catch-up，并保留 backlog。
        }
        Err(CorridorReplaceApplyError::Policy(error)) => {
            panic!("corridor population lifecycle invariant failed: {error}");
        }
    }
}

fn consume_population_step(
    session: Res<'_, LaneFlowSession>,
    mut population: ResMut<'_, CorridorPopulationRuntime>,
) {
    let step = session
        .frame_step_results()
        .last()
        .expect("Observe only runs after a committed Core step");
    let completed = population
        .controller
        .consume_step_result(step)
        .unwrap_or_else(|error| panic!("corridor population event invariant failed: {error}"));
    population.completed += completed as u64;
}

fn begin_lane_flow_step_measurement(mut performance: ResMut<'_, RuntimePerformance>) {
    performance.begin_step(Instant::now());
}

fn end_lane_flow_step_measurement(mut performance: ResMut<'_, RuntimePerformance>) {
    performance.end_step(Instant::now());
}

fn sample_runtime_performance(
    time: Res<'_, Time>,
    session: Res<'_, LaneFlowSession>,
    mut performance: ResMut<'_, RuntimePerformance>,
) {
    let report = session.frame_report();
    performance.sample_outer_frame(
        time.delta(),
        report.steps_run(),
        report.backlog(),
        report.catch_up_limit_reached(),
    );
}

#[derive(Resource)]
struct RuntimeMetadata {
    vehicle_count: usize,
    seed: u64,
    screenshot_path: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct RuntimePerformanceSnapshot {
    fps: f64,
    frame_time_ms: f64,
    lane_flow_step_ms_per_frame: f64,
    lane_flow_step_us_per_tick: f64,
    steps_run: u32,
    backlog: Duration,
    catch_up_limit_reached: bool,
}

#[derive(Resource, Debug, Default)]
struct RuntimePerformance {
    step_started_at: Option<Instant>,
    current_frame_step_elapsed: Duration,
    current_frame_step_count: u64,
    hud_elapsed: Duration,
    window_elapsed: Duration,
    window_frame_count: u64,
    window_step_elapsed: Duration,
    window_step_count: u64,
    snapshot: RuntimePerformanceSnapshot,
}

impl RuntimePerformance {
    fn begin_step(&mut self, started_at: Instant) {
        debug_assert!(
            self.step_started_at.is_none(),
            "a LaneFlow step measurement must end before the next step starts"
        );
        self.step_started_at = Some(started_at);
    }

    fn end_step(&mut self, finished_at: Instant) {
        let started_at = self
            .step_started_at
            .take()
            .expect("LaneFlow step measurement must start in Lifecycle");
        self.current_frame_step_elapsed = self
            .current_frame_step_elapsed
            .saturating_add(finished_at.saturating_duration_since(started_at));
        self.current_frame_step_count = self.current_frame_step_count.saturating_add(1);
    }

    fn sample_outer_frame(
        &mut self,
        frame_delta: Duration,
        steps_run: u32,
        backlog: Duration,
        catch_up_limit_reached: bool,
    ) {
        // Observe does not run after a failed Step. Never carry that incomplete timing
        // across outer frames or attribute it to a later successful tick.
        self.step_started_at = None;
        self.hud_elapsed = self.hud_elapsed.saturating_add(frame_delta);
        self.window_elapsed = self.window_elapsed.saturating_add(frame_delta);
        self.window_frame_count = self.window_frame_count.saturating_add(1);
        self.window_step_elapsed = self
            .window_step_elapsed
            .saturating_add(self.current_frame_step_elapsed);
        self.window_step_count = self
            .window_step_count
            .saturating_add(self.current_frame_step_count);
        self.current_frame_step_elapsed = Duration::ZERO;
        self.current_frame_step_count = 0;

        self.snapshot.steps_run = steps_run;
        self.snapshot.backlog = backlog;
        self.snapshot.catch_up_limit_reached = catch_up_limit_reached;

        if self.window_elapsed < PERFORMANCE_SAMPLE_WINDOW {
            return;
        }

        let elapsed_seconds = self.window_elapsed.as_secs_f64();
        let frame_count = self.window_frame_count as f64;
        self.snapshot.fps = frame_count / elapsed_seconds;
        self.snapshot.frame_time_ms = elapsed_seconds * 1_000.0 / frame_count;
        self.snapshot.lane_flow_step_ms_per_frame =
            self.window_step_elapsed.as_secs_f64() * 1_000.0 / frame_count;
        self.snapshot.lane_flow_step_us_per_tick = if self.window_step_count == 0 {
            0.0
        } else {
            self.window_step_elapsed.as_secs_f64() * 1_000_000.0 / self.window_step_count as f64
        };

        self.window_elapsed = Duration::ZERO;
        self.window_frame_count = 0;
        self.window_step_elapsed = Duration::ZERO;
        self.window_step_count = 0;
    }

    fn take_hud_refresh_due(&mut self) -> bool {
        if self.hud_elapsed < HUD_REFRESH_INTERVAL {
            return false;
        }
        self.hud_elapsed = self
            .hud_elapsed
            .checked_sub(HUD_REFRESH_INTERVAL)
            .expect("HUD refresh only subtracts a completed interval");
        true
    }

    const fn snapshot(&self) -> RuntimePerformanceSnapshot {
        self.snapshot
    }
}

#[derive(Component)]
struct RuntimeHud;

#[derive(Component, Clone, Copy, Debug)]
struct CorridorOrbitCamera {
    focus: Vec3,
    yaw: f32,
    pitch: f32,
    distance: f32,
    min_distance: f32,
    max_distance: f32,
}

impl CorridorOrbitCamera {
    fn for_scene(focus: Vec3, span: f32) -> Self {
        let offset = Vec3::new(0.0, span * 0.58, span * 0.46);
        let horizontal = Vec2::new(offset.x, offset.z).length();
        Self {
            focus,
            yaw: offset.x.atan2(offset.z),
            pitch: offset.y.atan2(horizontal),
            distance: offset.length(),
            min_distance: span * 0.035,
            max_distance: span * 2.0,
        }
    }

    fn apply_to(self, transform: &mut Transform) {
        let horizontal = self.distance * self.pitch.cos();
        let offset = Vec3::new(
            horizontal * self.yaw.sin(),
            self.distance * self.pitch.sin(),
            horizontal * self.yaw.cos(),
        );
        *transform =
            Transform::from_translation(self.focus + offset).looking_at(self.focus, Vec3::Y);
    }

    fn zoom(&mut self, scroll_lines: f32) {
        self.distance = (self.distance * (-scroll_lines * CAMERA_ZOOM_SENSITIVITY).exp())
            .clamp(self.min_distance, self.max_distance);
    }

    fn orbit(&mut self, delta: Vec2) {
        self.yaw -= delta.x * CAMERA_ORBIT_SENSITIVITY;
        self.pitch = (self.pitch + delta.y * CAMERA_ORBIT_SENSITIVITY)
            .clamp(CAMERA_MIN_PITCH_RADIANS, CAMERA_MAX_PITCH_RADIANS);
    }

    fn pan(&mut self, delta: Vec2, transform: &Transform) {
        let right = Vec3::new(transform.right().x, 0.0, transform.right().z).normalize_or_zero();
        let forward =
            Vec3::new(transform.forward().x, 0.0, transform.forward().z).normalize_or_zero();
        let world_units_per_pixel = self.distance * CAMERA_PAN_SENSITIVITY;
        self.focus += (-delta.x * right + delta.y * forward) * world_units_per_pixel;
        self.focus.y = 0.0;
    }
}

#[derive(Resource)]
struct SignalLampMaterials {
    red_on: Handle<StandardMaterial>,
    red_off: Handle<StandardMaterial>,
    yellow_on: Handle<StandardMaterial>,
    yellow_off: Handle<StandardMaterial>,
    green_on: Handle<StandardMaterial>,
    green_off: Handle<StandardMaterial>,
}

impl SignalLampMaterials {
    fn material(&self, aspect: SignalAspect, active: bool) -> Handle<StandardMaterial> {
        match (aspect, active) {
            (SignalAspect::Red, true) => self.red_on.clone(),
            (SignalAspect::Red, false) => self.red_off.clone(),
            (SignalAspect::Yellow, true) => self.yellow_on.clone(),
            (SignalAspect::Yellow, false) => self.yellow_off.clone(),
            (SignalAspect::Green, true) => self.green_on.clone(),
            (SignalAspect::Green, false) => self.green_off.clone(),
        }
    }
}

#[derive(Component)]
struct SignalLamp {
    group: SignalGroupHandle,
    aspect: SignalAspect,
}

fn setup_scene(
    mut commands: Commands<'_, '_>,
    mut meshes: ResMut<'_, Assets<Mesh>>,
    mut materials: ResMut<'_, Assets<StandardMaterial>>,
    scene: Res<'_, CorridorScene>,
    metadata: Res<'_, RuntimeMetadata>,
    mut session: ResMut<'_, LaneFlowSession>,
) {
    commands.insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 140.0,
        ..default()
    });
    let center = scene.bounds.center();
    let span = scene.bounds.span();
    commands.spawn((
        DirectionalLight {
            illuminance: 18_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_translation(center + Vec3::new(-span * 0.35, span, span * 0.4))
            .looking_at(center, Vec3::Y),
    ));
    let orbit_camera = CorridorOrbitCamera::for_scene(center, span);
    let mut camera_transform = Transform::IDENTITY;
    orbit_camera.apply_to(&mut camera_transform);
    commands.spawn((
        Name::new("LaneFlow corridor orbit camera"),
        Camera3d::default(),
        camera_transform,
        orbit_camera,
    ));
    commands
        .spawn((
            Name::new("LaneFlow runtime HUD panel"),
            Node {
                position_type: PositionType::Absolute,
                top: px(14),
                left: px(14),
                max_width: px(760),
                padding: UiRect::axes(px(14), px(10)),
                border_radius: BorderRadius::all(px(7)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.018, 0.025, 0.035, 0.84)),
            GlobalZIndex(i32::MAX),
        ))
        .with_child((
            Name::new("LaneFlow runtime HUD text"),
            RuntimeHud,
            Text::new("LaneFlow signalized corridor starting..."),
            TextFont {
                font_size: FontSize::Px(18.0),
                ..default()
            },
            TextColor(Color::srgb(0.93, 0.96, 1.0)),
            TextShadow {
                offset: Vec2::new(1.0, 1.0),
                color: Color::srgba(0.0, 0.0, 0.0, 0.9),
            },
        ));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(span * 1.2, span * 1.2))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.025, 0.055, 0.035),
            perceptual_roughness: 1.0,
            ..default()
        })),
        Transform::from_translation(Vec3::new(center.x, -0.12, center.z)),
    ));

    let root = commands
        .spawn((
            Name::new("LaneFlow signalized-corridor frame root"),
            Transform::IDENTITY,
        ))
        .id();
    spawn_roads(&mut commands, &mut meshes, &mut materials, root, &scene);
    let lamp_materials =
        spawn_signal_infrastructure(&mut commands, &mut meshes, &mut materials, root, &scene);
    commands.insert_resource(lamp_materials);
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
        .expect("corridor frame root satisfies the accepted placement contract");

    info!(
        vehicles = metadata.vehicle_count,
        seed = metadata.seed,
        screenshot = metadata.screenshot_path,
        "LaneFlow signalized corridor loaded; G toggles Gizmos, F12 saves screenshot"
    );
}

fn spawn_roads(
    commands: &mut Commands<'_, '_>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    root: Entity,
    scene: &CorridorScene,
) {
    let segment_mesh = meshes.add(Cuboid::default());
    let road_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.09, 0.105, 0.13),
        perceptual_roughness: 0.98,
        ..default()
    });
    let marking_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.78, 0.8, 0.72),
        emissive: LinearRgba::new(0.08, 0.08, 0.06, 1.0),
        perceptual_roughness: 0.85,
        ..default()
    });

    for edge in &scene.edges {
        for pair in edge.points.windows(2) {
            let start = pair[0];
            let end = pair[1];
            let delta = end - start;
            let length = delta.length();
            if length <= f32::EPSILON {
                continue;
            }
            let tangent = delta / length;
            let right = Vec3::Y.cross(tangent).normalize_or_zero();
            let midpoint = (start + end) * 0.5;
            let rotation = Quat::from_rotation_arc(Vec3::X, tangent);
            commands.spawn((
                Name::new("Corridor lane surface"),
                Mesh3d(segment_mesh.clone()),
                MeshMaterial3d(road_material.clone()),
                Transform {
                    translation: midpoint - Vec3::Y * 0.04,
                    rotation,
                    scale: Vec3::new(length, 0.08, scene.lane_width * 0.96),
                },
                ChildOf(root),
            ));
            for side in [-1.0_f32, 1.0] {
                commands.spawn((
                    Name::new("Corridor lane marking"),
                    Mesh3d(segment_mesh.clone()),
                    MeshMaterial3d(marking_material.clone()),
                    Transform {
                        translation: midpoint
                            + right * (side * scene.lane_width * 0.5)
                            + Vec3::Y * 0.015,
                        rotation,
                        scale: Vec3::new(length, 0.025, 0.055),
                    },
                    ChildOf(root),
                ));
            }
        }
    }
}

fn spawn_signal_infrastructure(
    commands: &mut Commands<'_, '_>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    root: Entity,
    scene: &CorridorScene,
) -> SignalLampMaterials {
    let cube = meshes.add(Cuboid::default());
    let stop_line_material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        emissive: LinearRgba::new(0.2, 0.2, 0.2, 1.0),
        ..default()
    });
    let pole_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.06, 0.07, 0.08),
        metallic: 0.45,
        perceptual_roughness: 0.5,
        ..default()
    });
    let lamp_materials = SignalLampMaterials {
        red_on: add_lamp_material(materials, Color::srgb(1.0, 0.035, 0.025), 7.0),
        red_off: add_lamp_material(materials, Color::srgb(0.12, 0.01, 0.01), 0.0),
        yellow_on: add_lamp_material(materials, Color::srgb(1.0, 0.62, 0.015), 7.0),
        yellow_off: add_lamp_material(materials, Color::srgb(0.12, 0.065, 0.005), 0.0),
        green_on: add_lamp_material(materials, Color::srgb(0.015, 1.0, 0.12), 7.0),
        green_off: add_lamp_material(materials, Color::srgb(0.005, 0.12, 0.02), 0.0),
    };

    for stop in &scene.signal_stops {
        let tangent = stop.tangent.normalize_or_zero();
        let right = Vec3::Y.cross(tangent).normalize_or_zero();
        commands.spawn((
            Name::new("Corridor stop line"),
            Mesh3d(cube.clone()),
            MeshMaterial3d(stop_line_material.clone()),
            Transform {
                translation: stop.position + Vec3::Y * 0.055,
                rotation: Quat::from_rotation_arc(Vec3::X, right),
                scale: Vec3::new(scene.lane_width * 0.88, 0.04, 0.16),
            },
            ChildOf(root),
        ));
        let pole_base =
            stop.position + right * (scene.lane_width * 0.43) - tangent * 0.55 + Vec3::Y * 1.75;
        commands.spawn((
            Name::new("Corridor signal pole"),
            Mesh3d(cube.clone()),
            MeshMaterial3d(pole_material.clone()),
            Transform::from_translation(pole_base - Vec3::Y * 0.8)
                .with_scale(Vec3::new(0.16, 2.2, 0.16)),
            ChildOf(root),
        ));
        for (index, aspect) in [SignalAspect::Red, SignalAspect::Yellow, SignalAspect::Green]
            .into_iter()
            .enumerate()
        {
            let offset_y = 0.64 - index as f32 * 0.64;
            commands.spawn((
                Name::new(format!("Corridor {aspect:?} signal lamp")),
                SignalLamp {
                    group: stop.group,
                    aspect,
                },
                Mesh3d(cube.clone()),
                MeshMaterial3d(lamp_materials.material(aspect, false)),
                Transform::from_translation(pole_base + Vec3::Y * offset_y)
                    .with_scale(Vec3::new(0.75, 0.52, 0.42)),
                ChildOf(root),
            ));
        }
    }

    lamp_materials
}

fn add_lamp_material(
    materials: &mut Assets<StandardMaterial>,
    color: Color,
    emissive_strength: f32,
) -> Handle<StandardMaterial> {
    materials.add(StandardMaterial {
        base_color: color,
        emissive: LinearRgba::from(color) * emissive_strength,
        perceptual_roughness: 0.35,
        ..default()
    })
}

fn spawn_vehicles(
    commands: &mut Commands<'_, '_>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    root: Entity,
    session: &mut LaneFlowSession,
) {
    let body_mesh = meshes.add(Cuboid::default());
    let nose_mesh = meshes.add(Cuboid::default());
    let body_materials = [
        Color::srgb(0.9, 0.12, 0.08),
        Color::srgb(0.05, 0.42, 0.92),
        Color::srgb(0.96, 0.58, 0.04),
        Color::srgb(0.16, 0.72, 0.35),
        Color::srgb(0.62, 0.18, 0.84),
        Color::srgb(0.82, 0.82, 0.86),
    ]
    .map(|color| {
        materials.add(StandardMaterial {
            base_color: color,
            metallic: 0.18,
            perceptual_roughness: 0.5,
            ..default()
        })
    });
    let nose_material = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.88, 0.32),
        emissive: LinearRgba::new(1.2, 0.9, 0.25, 1.0),
        ..default()
    });
    let vehicles = session
        .core()
        .vehicles()
        .map(|vehicle| vehicle.handle)
        .collect::<Vec<_>>();

    for (index, vehicle) in vehicles.into_iter().enumerate() {
        let vehicle_length = session
            .core()
            .vehicle(vehicle)
            .and_then(|state| session.core().vehicle_profile(state.profile))
            .expect("corridor vehicle profile remains live")
            .iidm()
            .length as f32;
        let proxy = commands
            .spawn((
                Name::new(format!("LaneFlow corridor vehicle proxy {index:03}")),
                Transform::IDENTITY,
                ChildOf(root),
            ))
            .id();
        commands.spawn((
            Name::new("Built-in corridor vehicle body"),
            Mesh3d(body_mesh.clone()),
            MeshMaterial3d(body_materials[index % body_materials.len()].clone()),
            vehicle_body_local_transform(vehicle_length),
            ChildOf(proxy),
        ));
        commands.spawn((
            Name::new("Built-in corridor vehicle forward marker"),
            Mesh3d(nose_mesh.clone()),
            MeshMaterial3d(nose_material.clone()),
            vehicle_nose_local_transform(),
            ChildOf(proxy),
        ));
        session
            .bind_vehicle_entity(vehicle, proxy)
            .expect("corridor proxy binding is one-to-one");
    }
}

fn vehicle_body_local_transform(length: f32) -> Transform {
    Transform::from_xyz(0.0, 0.8, length * 0.5).with_scale(Vec3::new(2.35, 1.5, length))
}

fn vehicle_nose_local_transform() -> Transform {
    Transform::from_xyz(0.0, 0.9, 0.12).with_scale(Vec3::new(1.15, 0.34, 0.24))
}

fn update_signal_lamps(
    session: Res<'_, LaneFlowSession>,
    lamp_materials: Res<'_, SignalLampMaterials>,
    mut lamps: Query<'_, '_, (&SignalLamp, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    for (lamp, mut material) in &mut lamps {
        let active = session
            .core()
            .signal_group_state(lamp.group)
            .is_some_and(|snapshot| snapshot.aspect() == lamp.aspect);
        let desired = lamp_materials.material(lamp.aspect, active);
        if material.0 != desired {
            material.0 = desired;
        }
    }
}

fn update_runtime_hud(
    session: Res<'_, LaneFlowSession>,
    population: Res<'_, CorridorPopulationRuntime>,
    metadata: Res<'_, RuntimeMetadata>,
    mut performance: ResMut<'_, RuntimePerformance>,
    debug: Res<'_, LaneFlowDebugGizmosConfig>,
    mut hud: Query<'_, '_, &mut Text, With<RuntimeHud>>,
) {
    if !performance.take_hud_refresh_due() {
        return;
    }
    let counts = population.controller.counts();
    let core = session.core();
    let (speed_sum, speed_count) =
        (0..metadata.vehicle_count).fold((0.0, 0_u32), |(sum, count), logical_index| {
            let speed = population
                .controller
                .logical_vehicle(logical_index)
                .and_then(|vehicle| core.vehicle(vehicle))
                .map(|vehicle| vehicle.current_speed.value());
            match speed {
                Some(speed) => (sum + speed, count + 1),
                None => (sum, count),
            }
        });
    let average_speed_kmh = if speed_count == 0 {
        0.0
    } else {
        speed_sum / f64::from(speed_count) * 3.6
    };
    let signal_status = core
        .signals()
        .controllers()
        .filter_map(|controller| {
            let handle = core.signals().controller_handle(controller.id())?;
            let state = core.signal_controller_state(handle)?;
            let phase = core
                .signals()
                .phase_external_id(state.current_phase())
                .unwrap_or("unknown");
            Some(format!(
                "{}: {phase} ({:.1}s remaining)",
                controller
                    .id()
                    .strip_prefix("controller-")
                    .unwrap_or(controller.id()),
                state.phase_remaining_ms() as f32 / 1_000.0
            ))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let performance = performance.snapshot();
    let catch_up_status = if performance.catch_up_limit_reached {
        " LIMIT"
    } else {
        ""
    };
    if let Ok(mut text) = hud.single_mut() {
        text.0 = format!(
            "LaneFlow Signalized Corridor\n\
             FPS: {:.1} ({:.2} ms/frame)\n\
             LaneFlow step: {:.3} ms/frame | {:.1} us/tick | steps {} | backlog {:.1} ms{}\n\
             Vehicles: {} total | {} running | {} entry pending | average {:.1} km/h\n\
             Seed: {}  Tick: {}\n\
             Recycled: {}  Entry retries: {}  Debug lines: {}\n\
             {}\n\
             Controls: wheel zoom | left drag pan | right drag orbit | G debug | F12 screenshot",
            performance.fps,
            performance.frame_time_ms,
            performance.lane_flow_step_ms_per_frame,
            performance.lane_flow_step_us_per_tick,
            performance.steps_run,
            performance.backlog.as_secs_f64() * 1_000.0,
            catch_up_status,
            metadata.vehicle_count,
            counts.running,
            counts.pending,
            average_speed_kmh,
            metadata.seed,
            core.tick_index(),
            population.replaced,
            population.blocked,
            if debug.enabled { "ON" } else { "OFF" },
            signal_status
        );
    }
}

fn update_orbit_camera(
    mouse_buttons: Res<'_, ButtonInput<MouseButton>>,
    mouse_motion: Res<'_, AccumulatedMouseMotion>,
    mouse_scroll: Res<'_, AccumulatedMouseScroll>,
    mut cameras: Query<'_, '_, (&mut CorridorOrbitCamera, &mut Transform)>,
) {
    for (mut orbit_camera, mut transform) in &mut cameras {
        let motion_delta = mouse_motion
            .delta
            .clamp_length_max(CAMERA_MAX_MOTION_PER_FRAME);
        if mouse_buttons.pressed(MouseButton::Right) {
            orbit_camera.orbit(motion_delta);
        } else if mouse_buttons.pressed(MouseButton::Left) {
            orbit_camera.pan(motion_delta, &transform);
        }
        if mouse_scroll.delta.y != 0.0 {
            orbit_camera.zoom(mouse_scroll.delta.y);
        }
        orbit_camera.apply_to(&mut transform);
    }
}

fn toggle_debug_gizmos(
    input: Res<'_, ButtonInput<KeyCode>>,
    mut config: ResMut<'_, LaneFlowDebugGizmosConfig>,
) {
    if input.just_pressed(KeyCode::KeyG) {
        config.enabled = !config.enabled;
        info!(enabled = config.enabled, "LaneFlow debug Gizmos toggled");
    }
}

fn capture_screenshot(
    mut commands: Commands<'_, '_>,
    input: Res<'_, ButtonInput<KeyCode>>,
    metadata: Res<'_, RuntimeMetadata>,
) {
    if input.just_pressed(KeyCode::F12) {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(metadata.screenshot_path.clone()));
        info!(
            path = metadata.screenshot_path,
            "signalized corridor screenshot requested"
        );
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

fn canonical_direction(direction: laneflow_spatial::CanonicalUnitVector3F32) -> Vec3 {
    Vec3::new(direction.x(), direction.y(), direction.z())
}

fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::time::{TimePlugin, TimeUpdateStrategy};
    use laneflow_core::{CoreEvent, StepResult, VehicleCompletedRouteEvent};

    fn run_args(vehicles: usize, seed: u64) -> RunArgs {
        RunArgs {
            population: CorridorPopulationConfig::try_new(vehicles, seed)
                .expect("valid test population"),
            config_path: default_config_path(),
        }
    }

    #[test]
    fn cli_defaults_and_strict_failures_are_stable() {
        let CliAction::Run(defaults) = parse_args([]).expect("default args") else {
            panic!("default action must run")
        };
        assert_eq!(
            defaults.population.target_vehicle_count(),
            DEFAULT_TARGET_VEHICLE_COUNT
        );
        assert_eq!(defaults.population.seed(), DEFAULT_SEED);
        assert_eq!(defaults.config_path, default_config_path());

        assert!(parse_args(["--vehicles".into(), "49".into()]).is_err());
        assert!(parse_args(["--vehicles".into(), "201".into()]).is_err());
        assert!(parse_args(["--seed".into(), "invalid".into()]).is_err());
        assert!(parse_args(["--unknown".into()]).is_err());
        assert!(
            parse_args([
                "--vehicles".into(),
                "50".into(),
                "--vehicles".into(),
                "100".into()
            ])
            .is_err()
        );
        assert!(matches!(
            parse_args(["--help".into()]).expect("help"),
            CliAction::Help
        ));
    }

    #[test]
    fn production_bootstrap_supports_50_100_and_200() {
        for vehicles in [50, 100, 200] {
            let bootstrap =
                load_corridor_runtime(&run_args(vehicles, 0)).expect("production bootstrap");
            assert_eq!(bootstrap.session.core().vehicles().count(), vehicles);
            assert_eq!(bootstrap.controller.counts().target, vehicles);
            assert_eq!(bootstrap.controller.counts().running, vehicles);
            assert_eq!(bootstrap.controller.counts().pending, 0);
            assert!(
                bootstrap
                    .session
                    .core()
                    .vehicles()
                    .all(|vehicle| vehicle.current_speed.value() > 0.0)
            );
            assert_eq!(bootstrap.scene.signal_stops.len(), 20);
            assert_eq!(bootstrap.session.core().signals().controllers().len(), 2);
            assert_eq!(
                bootstrap
                    .session
                    .core()
                    .signals()
                    .controllers()
                    .map(|controller| controller.offset_ms())
                    .collect::<Vec<_>>(),
                [0, 29_000]
            );
        }
    }

    #[test]
    fn orbit_camera_and_front_bumper_visual_contract_are_explicit() {
        let focus = Vec3::new(25.0, 0.0, -40.0);
        let mut camera = CorridorOrbitCamera::for_scene(focus, 800.0);
        let mut transform = Transform::IDENTITY;
        camera.apply_to(&mut transform);
        assert!((transform.translation.distance(focus) - camera.distance).abs() < 0.001);

        let original_distance = camera.distance;
        camera.zoom(1.0);
        assert!(camera.distance < original_distance);
        camera.orbit(Vec2::new(20.0, -10.0));
        camera.apply_to(&mut transform);
        let original_focus = camera.focus;
        camera.pan(Vec2::new(10.0, 12.0), &transform);
        assert_ne!(camera.focus, original_focus);
        assert_eq!(camera.focus.y, 0.0);

        let body = vehicle_body_local_transform(4.5);
        let nose = vehicle_nose_local_transform();
        assert!((body.translation.z - body.scale.z * 0.5).abs() < f32::EPSILON);
        assert!((nose.translation.z - nose.scale.z * 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn camera_input_maps_right_drag_to_orbit_and_left_drag_to_pan() {
        let initial_camera = CorridorOrbitCamera::for_scene(Vec3::ZERO, 800.0);
        let mut initial_transform = Transform::IDENTITY;
        initial_camera.apply_to(&mut initial_transform);
        let mut app = App::new();
        app.insert_resource(ButtonInput::<MouseButton>::default());
        app.insert_resource(AccumulatedMouseMotion::default());
        app.insert_resource(AccumulatedMouseScroll::default());
        app.add_systems(Update, update_orbit_camera);
        let camera = app
            .world_mut()
            .spawn((initial_camera, initial_transform))
            .id();

        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Right);
        app.world_mut()
            .resource_mut::<AccumulatedMouseMotion>()
            .delta = Vec2::new(24.0, -12.0);
        app.update();
        let after_orbit = *app
            .world()
            .get::<CorridorOrbitCamera>(camera)
            .expect("orbit camera");
        assert_ne!(after_orbit.yaw, initial_camera.yaw);
        assert_ne!(after_orbit.pitch, initial_camera.pitch);
        assert_eq!(after_orbit.focus, initial_camera.focus);

        {
            let mut buttons = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
            buttons.release(MouseButton::Right);
            buttons.press(MouseButton::Left);
        }
        app.world_mut()
            .resource_mut::<AccumulatedMouseMotion>()
            .delta = Vec2::new(24.0, -12.0);
        app.update();
        let after_pan = app
            .world()
            .get::<CorridorOrbitCamera>(camera)
            .expect("pan camera");
        assert_ne!(after_pan.focus, after_orbit.focus);
        assert_eq!(after_pan.yaw, after_orbit.yaw);
        assert_eq!(after_pan.pitch, after_orbit.pitch);
    }

    #[test]
    fn runtime_performance_reports_windowed_fps_and_lane_flow_step_cost() {
        let mut performance = RuntimePerformance::default();
        for _ in 0..10 {
            let started_at = Instant::now();
            performance.begin_step(started_at);
            performance.end_step(started_at + Duration::from_micros(250));
            performance.sample_outer_frame(
                Duration::from_millis(100),
                1,
                Duration::from_millis(3),
                false,
            );
        }

        let snapshot = performance.snapshot();
        assert!((snapshot.fps - 10.0).abs() < f64::EPSILON);
        assert!((snapshot.frame_time_ms - 100.0).abs() < f64::EPSILON);
        assert!((snapshot.lane_flow_step_ms_per_frame - 0.25).abs() < f64::EPSILON);
        assert!((snapshot.lane_flow_step_us_per_tick - 250.0).abs() < f64::EPSILON);
        assert_eq!(snapshot.steps_run, 1);
        assert_eq!(snapshot.backlog, Duration::from_millis(3));
        assert!(!snapshot.catch_up_limit_reached);
        assert!(performance.take_hud_refresh_due());
        assert!(performance.take_hud_refresh_due());
        assert!(!performance.take_hud_refresh_due());
    }

    #[test]
    fn runtime_performance_keeps_lane_flow_cost_zero_without_fixed_ticks() {
        let mut performance = RuntimePerformance::default();
        for _ in 0..4 {
            performance.sample_outer_frame(
                Duration::from_millis(250),
                0,
                Duration::from_millis(5),
                true,
            );
        }

        let snapshot = performance.snapshot();
        assert!((snapshot.fps - 4.0).abs() < f64::EPSILON);
        assert!((snapshot.frame_time_ms - 250.0).abs() < f64::EPSILON);
        assert_eq!(snapshot.lane_flow_step_ms_per_frame, 0.0);
        assert_eq!(snapshot.lane_flow_step_us_per_tick, 0.0);
        assert_eq!(snapshot.steps_run, 0);
        assert_eq!(snapshot.backlog, Duration::from_millis(5));
        assert!(snapshot.catch_up_limit_reached);
    }

    fn headless_app(mut bootstrap: CorridorBootstrap) -> (App, Vec<Entity>) {
        let mut app = App::new();
        app.add_plugins((TimePlugin, TransformPlugin, LaneFlowPlugin));
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
        let root = app.world_mut().spawn(Transform::IDENTITY).id();
        let vehicles = bootstrap
            .session
            .core()
            .vehicles()
            .map(|vehicle| vehicle.handle)
            .collect::<Vec<_>>();
        let mut entities = Vec::with_capacity(vehicles.len());
        for vehicle in vehicles {
            let entity = app
                .world_mut()
                .spawn((Transform::IDENTITY, ChildOf(root)))
                .id();
            bootstrap
                .session
                .bind_vehicle_entity(vehicle, entity)
                .expect("headless binding");
            entities.push(entity);
        }
        bootstrap
            .session
            .set_frame_placement(LaneFlowFramePlacement::new(
                root,
                FramePlacementToken::new(1),
            ))
            .expect("headless frame placement");
        app.insert_resource(bootstrap.session);
        app.insert_resource(CorridorPopulationRuntime::new(bootstrap.controller));
        app.insert_resource(RuntimePerformance::default());
        app.add_systems(
            LaneFlowFixed,
            (
                (apply_population_pending, begin_lane_flow_step_measurement)
                    .chain()
                    .in_set(LaneFlowFixedSet::Lifecycle),
                (end_lane_flow_step_measurement, consume_population_step)
                    .chain()
                    .in_set(LaneFlowFixedSet::Observe),
            ),
        );
        app.add_systems(Update, sample_runtime_performance);
        app.update();
        (app, entities)
    }

    fn update_with_delta(app: &mut App, delta: Duration) {
        *app.world_mut().resource_mut::<TimeUpdateStrategy>() =
            TimeUpdateStrategy::ManualDuration(delta);
        app.update();
    }

    #[test]
    fn headless_integration_keeps_all_population_sizes_stable() {
        for vehicles in [50, 100, 200] {
            let bootstrap =
                load_corridor_runtime(&run_args(vehicles, 7)).expect("production bootstrap");
            let (mut app, entities) = headless_app(bootstrap);
            for _ in 0..16 {
                update_with_delta(&mut app, Duration::from_millis(128));
            }
            let population = app.world().resource::<CorridorPopulationRuntime>();
            let counts = population.controller.counts();
            assert_eq!(counts.target, vehicles);
            assert_eq!(counts.running + counts.pending, vehicles);
            let session = app.world().resource::<LaneFlowSession>();
            assert_eq!(session.vehicle_entities().len(), vehicles);
            for (index, entity) in entities.into_iter().enumerate() {
                let vehicle = population
                    .controller
                    .logical_vehicle(index)
                    .expect("logical vehicle");
                assert_eq!(session.vehicle_entities().entity(vehicle), Some(entity));
            }
        }
    }

    #[test]
    fn recycle_rotates_handles_without_replacing_proxy_entities() {
        let bootstrap = load_corridor_runtime(&run_args(50, 11)).expect("production bootstrap");
        let (mut app, entities) = headless_app(bootstrap);
        for _ in 0..625 {
            update_with_delta(&mut app, Duration::from_millis(128));
        }
        let population = app.world().resource::<CorridorPopulationRuntime>();
        assert!(population.completed > 0);
        assert!(population.replaced > 0);
        assert_eq!(
            population.controller.counts().running + population.controller.counts().pending,
            50
        );
        let session = app.world().resource::<LaneFlowSession>();
        for (index, entity) in entities.into_iter().enumerate() {
            let vehicle = population
                .controller
                .logical_vehicle(index)
                .expect("logical vehicle");
            assert_eq!(session.vehicle_entities().entity(vehicle), Some(entity));
        }
    }

    #[test]
    fn lifecycle_host_error_preserves_session_error_and_pending_plan() {
        let bootstrap = load_corridor_runtime(&run_args(50, 13)).expect("production bootstrap");
        let (mut app, _) = headless_app(bootstrap);
        let old = app
            .world()
            .resource::<CorridorPopulationRuntime>()
            .controller
            .logical_vehicle(0)
            .expect("logical vehicle");
        let (route, edge, route_edge_index) = {
            let session = app.world().resource::<LaneFlowSession>();
            let route = session.core().vehicle(old).expect("vehicle state").route;
            let route_edges = session.core().route_edges(route).expect("route edges");
            (
                route,
                *route_edges.last().expect("completion edge"),
                route_edges.len() - 1,
            )
        };
        let step = StepResult {
            tick_index: 1,
            time_ms: 20,
            events: vec![CoreEvent::VehicleCompletedRoute(
                VehicleCompletedRouteEvent {
                    tick_index: 1,
                    vehicle: old,
                    route,
                    edge,
                    route_edge_index,
                },
            )],
        };
        app.world_mut()
            .resource_mut::<CorridorPopulationRuntime>()
            .controller
            .consume_step_result(&step)
            .expect("synthetic completion");

        apply_population_pending(app.world_mut());

        let session = app.world().resource::<LaneFlowSession>();
        assert!(matches!(
            session.last_error(),
            Some(laneflow_bevy::LaneFlowAdapterError::CoreVehicleReplace {
                old: actual,
                ..
            }) if *actual == old
        ));
        let population = app.world().resource::<CorridorPopulationRuntime>();
        assert_eq!(population.controller.counts().running, 49);
        assert_eq!(population.controller.counts().pending, 1);
        assert_eq!(population.replaced, 0);
    }

    #[test]
    fn controller_and_core_replay_ignore_outer_frame_partitioning() {
        let partitioned = load_corridor_runtime(&run_args(50, 17)).expect("partitioned bootstrap");
        let batched = load_corridor_runtime(&run_args(50, 17)).expect("batched bootstrap");
        let (mut partitioned, _) = headless_app(partitioned);
        let (mut batched, _) = headless_app(batched);

        for _ in 0..1_000 {
            update_with_delta(&mut partitioned, Duration::from_millis(64));
        }
        for _ in 0..500 {
            update_with_delta(&mut batched, Duration::from_millis(128));
        }

        let partitioned_session = partitioned.world().resource::<LaneFlowSession>();
        let batched_session = batched.world().resource::<LaneFlowSession>();
        assert_eq!(partitioned_session.core(), batched_session.core());
        let partitioned_population = partitioned.world().resource::<CorridorPopulationRuntime>();
        let batched_population = batched.world().resource::<CorridorPopulationRuntime>();
        assert_eq!(
            partitioned_population.controller.counts(),
            batched_population.controller.counts()
        );
        assert_eq!(
            partitioned_population.controller.rng_state(),
            batched_population.controller.rng_state()
        );
        assert_eq!(
            partitioned_population.controller.last_consumed_tick(),
            batched_population.controller.last_consumed_tick()
        );
        for index in 0..50 {
            assert_eq!(
                partitioned_population.controller.logical_vehicle(index),
                batched_population.controller.logical_vehicle(index)
            );
        }
    }
}
