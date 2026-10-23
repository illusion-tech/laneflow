//! 固定人口、逐拍同 Entity 回流、Runtime 灯色与同帧 HUD。
#[path = "support/corridor_camera.rs"]
mod corridor_camera;
#[path = "support/corridor_capture.rs"]
mod corridor_capture;
#[path = "support/corridor_cli.rs"]
mod corridor_cli;
#[path = "support/corridor_gui.rs"]
mod corridor_gui;
#[path = "support/corridor_host.rs"]
mod corridor_host;
#[path = "support/corridor_replay.rs"]
mod corridor_replay;
#[path = "support/corridor_scene.rs"]
mod corridor_scene;
#[cfg(test)]
#[path = "support/corridor_tests.rs"]
mod tests;

use bevy::prelude::*;
use std::error::Error;

fn main() -> Result<AppExit, Box<dyn Error>> {
    let config = match corridor_cli::parse(std::env::args().skip(1))? {
        corridor_cli::Cli::Help => {
            println!("{}", corridor_cli::HELP);
            return Ok(AppExit::Success);
        }
        corridor_cli::Cli::Run(config) => config,
    };
    let scene = corridor_scene::build(config)?;
    let capture = corridor_capture::Capture::from_env()?;
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "LaneFlow | corridor_traffic".into(),
            resolution:
                bevy::window::WindowResolution::new(1_600, 1_000).with_scale_factor_override(1.0),
            ..default()
        }),
        ..default()
    }));
    corridor_host::install(&mut app, scene)?;
    corridor_gui::install(&mut app);
    if let Some(capture) = capture {
        corridor_capture::install(&mut app, capture)?;
    }
    Ok(app.run())
}
