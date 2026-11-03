//! 只提取并表现视野内车辆：提取前保守选择、隐藏时保留身份、周期全量对照。
#[path = "support/corridor_camera.rs"]
mod corridor_camera;
#[path = "support/culling_capture.rs"]
mod culling_capture;
#[path = "support/culling_cli.rs"]
mod culling_cli;
#[path = "support/culling_gui.rs"]
mod culling_gui;
#[path = "support/culling_host.rs"]
mod culling_host;
#[path = "support/culling_scene.rs"]
mod culling_scene;
#[path = "support/culling_view.rs"]
mod culling_view;

use bevy::prelude::*;
use std::error::Error;

fn main() -> Result<AppExit, Box<dyn Error>> {
    let config = match culling_cli::parse(std::env::args().skip(1))? {
        culling_cli::Cli::Help => {
            println!("{}", culling_cli::HELP);
            return Ok(AppExit::Success);
        }
        culling_cli::Cli::Run(config) => config,
    };
    let scene = culling_scene::build(culling_scene::SceneConfig::new(config.cells))?;
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "LaneFlow | culling_extraction".into(),
            resolution:
                bevy::window::WindowResolution::new(1_600, 1_000).with_scale_factor_override(1.0),
            ..default()
        }),
        ..default()
    }));
    let ribbon = culling_gui::ribbon_from_scene(&scene);
    let bounds = (scene.min, scene.max);
    culling_host::install(&mut app, scene, culling_view::ViewVolume::ALL)?;
    culling_gui::install(&mut app, ribbon, bounds, config.camera);
    if let Some(capture) = config.capture {
        culling_capture::install(&mut app, capture);
    }
    Ok(app.run())
}
