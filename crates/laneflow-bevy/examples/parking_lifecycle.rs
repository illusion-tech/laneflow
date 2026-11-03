//! 表现预留、到达、停入和离场：本批没有位姿不等于车辆已被移除。
#[path = "support/corridor_camera.rs"]
mod corridor_camera;
#[path = "support/parking_capture.rs"]
mod parking_capture;
#[path = "support/parking_cli.rs"]
mod parking_cli;
#[path = "support/parking_gui.rs"]
mod parking_gui;
#[path = "support/parking_host.rs"]
mod parking_host;
#[path = "support/parking_scene.rs"]
mod parking_scene;
#[path = "support/parking_script.rs"]
mod parking_script;

use bevy::prelude::*;
use std::error::Error;

fn main() -> Result<AppExit, Box<dyn Error>> {
    let capture = match parking_cli::parse(std::env::args().skip(1))? {
        parking_cli::Cli::Help => {
            println!("{}", parking_cli::HELP);
            return Ok(AppExit::Success);
        }
        parking_cli::Cli::Run { capture } => capture,
    };
    let scene = parking_scene::build(parking_scene::SceneConfig::new())?;
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "LaneFlow | parking_lifecycle".into(),
            resolution:
                bevy::window::WindowResolution::new(1_600, 1_000).with_scale_factor_override(1.0),
            ..default()
        }),
        ..default()
    }));
    let layout = parking_gui::layout_from_scene(&scene)?;
    parking_host::install(&mut app, scene)?;
    parking_gui::install(&mut app, layout);
    if let Some(capture) = capture {
        parking_capture::install(&mut app, capture);
    }
    Ok(app.run())
}
