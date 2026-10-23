//! 复杂路口 Bevy 调试示例（#285 阶段二 §4）：检入 catalog 0.1 + LFCA 的四车
//! 确定性场景 + 默认关闭的 F2 调试 overlay（mesh/文本，无 Gizmos）。
//!
//! overlay 全部只读 `LaneFlowSession` 并写独立 ECS 实体，不触碰 Runtime 状态；
//! 开/关 overlay 在相同输入下的 Runtime 摘要一致性由无窗口 smoke 对拍把关。
//! GUI 不进 CI（bevy-reference-adapter §9）。

#[path = "support/junction_camera.rs"]
mod junction_camera;
#[path = "support/junction_capture.rs"]
mod junction_capture;
#[path = "support/junction_cli.rs"]
mod junction_cli;
#[path = "support/junction_debug_scene.rs"]
mod junction_debug_scene;
#[path = "support/junction_overlay.rs"]
mod junction_overlay;
#[path = "support/junction_road.rs"]
mod junction_road;
#[path = "support/junction_signals.rs"]
mod junction_signals;
#[path = "support/junction_surface.rs"]
mod junction_surface;
#[path = "support/junction_vehicles.rs"]
mod junction_vehicles;

use std::error::Error;

use bevy::{asset::AssetApp, light::atmosphere::ScatteringMedium, prelude::*};
use laneflow_bevy::{LaneFlowPlugin, LaneFlowSession};

use junction_camera::CameraPresetChoice;
use junction_vehicles::{PoseBuffer, SpawnedPlan};

fn main() -> Result<AppExit, Box<dyn Error>> {
    let scene = junction_debug_scene::build()?;
    let cli = junction_cli::CliArgs::parse();
    let mut app = App::new();
    app.add_plugins(LaneFlowPlugin);
    if cli.screenshot.is_some() {
        app.add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(junction_capture::window()),
            ..default()
        }));
    } else {
        app.add_plugins(DefaultPlugins);
    }
    // ScatteringMedium 资产集合由示例自行注册（bevy_pbr 不代注册）；
    // 必须在 AssetPlugin（DefaultPlugins）之后。
    app.init_asset::<ScatteringMedium>();
    app.insert_resource(scene.session)
        .insert_resource(SpawnedPlan(scene.spawned))
        .insert_resource(CameraPresetChoice(cli.camera))
        .init_resource::<PoseBuffer>()
        .add_systems(Startup, setup_scene)
        .add_systems(
            Update,
            (
                junction_vehicles::sync_vehicle_transforms,
                junction_signals::update_traffic_lights,
            ),
        );
    junction_overlay::install(&mut app, cli.overlay);
    if let Some(path) = cli.screenshot {
        junction_capture::install(&mut app, path, cli.tick);
    }
    Ok(app.run())
}

fn setup_scene(
    mut commands: Commands,
    mut session: ResMut<LaneFlowSession>,
    plan: Res<SpawnedPlan>,
    preset: Res<CameraPresetChoice>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut scattering_mediums: ResMut<Assets<ScatteringMedium>>,
) {
    let revision = session.world().revision();
    let roads = junction_road::build(&revision);
    let layout = roads.layout;
    junction_camera::spawn(
        &mut commands,
        &mut scattering_mediums,
        preset.0,
        roads.extent * 2.0 + 28.0,
    );
    junction_surface::spawn(
        &mut commands,
        &mut meshes,
        &mut materials,
        session.world(),
        &revision,
        roads,
    );
    junction_vehicles::spawn(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut session,
        &plan.0,
    );
    junction_signals::spawn(
        &mut commands,
        &mut meshes,
        &mut materials,
        &revision,
        layout,
    );
    junction_overlay::spawn_panel(&mut commands);
}
