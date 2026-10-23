use std::{error::Error, path::PathBuf};

use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured},
};

use super::{
    corridor_gui::{self, Hud, Lamp},
    corridor_host::{self as host, Summary},
    corridor_replay::{self as replay, ReplayClock},
    corridor_scene::Geometry,
};

#[derive(Resource)]
pub(super) struct Capture {
    path: PathBuf,
    pub tick: u64,
    fired: bool,
}

impl Capture {
    pub fn from_env() -> Result<Option<Self>, Box<dyn Error>> {
        let Some(path) = std::env::var_os("CORRIDOR_TRAFFIC_CAPTURE").map(PathBuf::from) else {
            return Ok(None);
        };
        if path.extension().is_none_or(|extension| extension != "png")
            || !path.parent().is_some_and(|parent| parent.is_dir())
        {
            return Err("CORRIDOR_TRAFFIC_CAPTURE 必须指向已有目录内的 .png 文件".into());
        }
        let tick = std::env::var("CORRIDOR_TRAFFIC_CAPTURE_TICK")
            .ok()
            .map(|value| value.parse::<u64>())
            .transpose()?
            .unwrap_or(replay::DEFAULT_CAPTURE_TICK);
        ReplayClock::new(tick)?;
        Ok(Some(Self {
            path,
            tick,
            fired: false,
        }))
    }
}

pub(super) fn install(app: &mut App, capture: Capture) -> Result<(), String> {
    let clock = ReplayClock::new(capture.tick)?;
    app.insert_resource(capture);
    replay::install(app, clock);
    app.add_systems(
        PostUpdate,
        request_capture.after(corridor_gui::update_display),
    );
    Ok(())
}

fn request_capture(world: &mut World) {
    if world.resource::<Capture>().fired || world.resource::<host::Health>().failure.is_some() {
        return;
    }
    let Some(summary) = world.resource::<Summary>().current.as_ref() else {
        return;
    };
    if !world.resource::<ReplayClock>().ready(summary) {
        return;
    }
    let summary = summary.clone();
    let mut text_query = world.query_filtered::<&Text, With<Hud>>();
    if text_query.single(world).map(|text| &text.0).ok() != Some(&summary.hud()) {
        host::fail(world, host::Stage::Presentation, "截图 HUD 与同帧摘要不符");
        host::exit_on_failure(world);
        return;
    }
    let mut lamp_query = world.query::<&Lamp>();
    let lamps: Vec<_> = lamp_query
        .iter(world)
        .map(|lamp| (lamp.group, lamp.aspect))
        .collect();
    if lamps.len() != world.resource::<Geometry>().lamps.len()
        || lamps.iter().any(|(group, aspect)| {
            *aspect
                != summary
                    .signals
                    .iter()
                    .find(|(candidate, _)| candidate == group)
                    .map(|(_, value)| *value)
        })
    {
        host::fail(
            world,
            host::Stage::Presentation,
            "截图灯具与已提交信号组不符",
        );
        host::exit_on_failure(world);
        return;
    }
    let mut capture = world.resource_mut::<Capture>();
    capture.fired = true;
    let path = capture.path.clone();
    let metadata = replay::evidence(&summary);
    println!(
        "C13 capture frame={} tick={} path={}",
        summary.frame,
        summary.tick,
        path.display()
    );
    world.spawn(Screenshot::primary_window()).observe(
        move |captured: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
            let saved = (|| -> Result<(), Box<dyn Error>> {
                let image = captured
                    .image
                    .clone()
                    .try_into_dynamic()
                    .map_err(|error| format!("{error:?}"))?;
                image.to_rgb8().save(&path)?;
                std::fs::write(path.with_extension("toml"), &metadata)?;
                Ok(())
            })();
            match saved {
                Ok(()) => {
                    println!(
                        "C13 screenshot + same-frame summary saved: {}",
                        path.display()
                    );
                    exit.write(AppExit::Success);
                }
                Err(error) => {
                    eprintln!("C13 出图失败：{error}");
                    exit.write(AppExit::error());
                }
            }
        },
    );
}
