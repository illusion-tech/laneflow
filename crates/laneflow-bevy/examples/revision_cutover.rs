//! 运行中切换路网修订：保持车辆身份、丢弃旧批次、重建路面表现。
#[path = "support/corridor_camera.rs"]
mod corridor_camera;
#[path = "support/cutover_capture.rs"]
mod cutover_capture;
#[path = "support/cutover_cli.rs"]
mod cutover_cli;
#[path = "support/cutover_gui.rs"]
mod cutover_gui;
#[path = "support/cutover_host.rs"]
mod cutover_host;
#[path = "support/cutover_scene.rs"]
mod cutover_scene;

use bevy::prelude::*;
use std::error::Error;

fn main() -> Result<AppExit, Box<dyn Error>> {
    let (cutovers, capture) = match cutover_cli::parse(std::env::args().skip(1))? {
        cutover_cli::Cli::Help => {
            println!("{}", cutover_cli::HELP);
            return Ok(AppExit::Success);
        }
        cutover_cli::Cli::Run { cutovers, capture } => (cutovers, capture),
    };
    let scene = cutover_scene::build()?;
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "LaneFlow | revision_cutover".into(),
            resolution:
                bevy::window::WindowResolution::new(1_600, 1_000).with_scale_factor_override(1.0),
            ..default()
        }),
        ..default()
    }));
    let (min, max) = (scene.min, scene.max);
    cutover_host::install(&mut app, scene, cutovers)?;
    cutover_gui::install(&mut app, min, max);
    if let Some(capture) = capture {
        cutover_capture::install(&mut app, capture);
    }
    Ok(app.run())
}
