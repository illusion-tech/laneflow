//! 定 Tick 出图：先等渲染管线就绪，再按固定 delta 推进到目标 Tick。
use std::{error::Error, time::Duration};

use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured},
    time::{TimeSystems, TimeUpdateStrategy},
};

use super::{
    cutover_cli::CaptureRequest,
    cutover_gui::{self, Hud},
    cutover_host::{self as host, Summary},
};

const WARMUP_FRAMES: u64 = 150;
const STEP_DELTA: Duration = Duration::from_millis(64);
const TICK: Duration = Duration::from_millis(16);

#[derive(Resource)]
struct Capture {
    request: CaptureRequest,
    frame: u64,
    fired: bool,
}

pub(super) fn install(app: &mut App, request: CaptureRequest) {
    app.insert_resource(Capture {
        request,
        frame: 0,
        fired: false,
    })
    .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO))
    .add_systems(First, provide_delta.before(TimeSystems))
    .add_systems(PostUpdate, request_capture.after(cutover_gui::update_hud));
}

fn provide_delta(
    mut capture: ResMut<Capture>,
    summary: Res<Summary>,
    mut strategy: ResMut<TimeUpdateStrategy>,
) {
    capture.frame += 1;
    let tick = summary.current.as_ref().map_or(0, |summary| summary.tick);
    let remaining = TICK * u32::try_from(capture.request.tick.saturating_sub(tick)).unwrap_or(0);
    let delta = if capture.frame <= WARMUP_FRAMES {
        Duration::ZERO
    } else {
        STEP_DELTA.min(remaining)
    };
    *strategy = TimeUpdateStrategy::ManualDuration(delta);
}

fn request_capture(world: &mut World) {
    let capture = world.resource::<Capture>();
    if capture.fired
        || capture.frame <= WARMUP_FRAMES
        || world.resource::<host::Health>().failure.is_some()
    {
        return;
    }
    let target = capture.request.tick;
    let Some(summary) = world.resource::<Summary>().current.clone() else {
        return;
    };
    if summary.tick != target || summary.steps != 0 || !summary.backlog.is_zero() {
        return;
    }
    let mut text = world.query_filtered::<&Text, With<Hud>>();
    if text.single(world).map(|text| &text.0).ok() != Some(&summary.hud()) {
        host::fail(world, "截图 HUD 与同帧摘要不符");
        host::exit_on_failure(world);
        return;
    }
    let mut capture = world.resource_mut::<Capture>();
    capture.fired = true;
    let path = capture.request.path.clone();
    println!(
        "capture frame={} tick={} path={}",
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
                Ok(())
            })();
            match saved {
                Ok(()) => {
                    println!("screenshot saved: {}", path.display());
                    exit.write(AppExit::Success);
                }
                Err(error) => {
                    eprintln!("出图失败：{error}");
                    exit.write(AppExit::error());
                }
            }
        },
    );
}
