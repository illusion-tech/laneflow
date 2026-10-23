//! 截图模式：固定物理分辨率，在目标交通 tick 落盘后自动退出。
use std::{path::PathBuf, time::Duration};

use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured},
    time::TimeUpdateStrategy,
    window::WindowResolution,
};
use laneflow_bevy::LaneFlowSession;

/// 出图前至少渲染这些帧，等待管线就绪；交通状态另由 --at-tick 指定。
const SCREENSHOT_FRAME: u32 = 150;
/// screenshot 模式窗口分辨率（宽, 高）。
const SCREENSHOT_RESOLUTION: (u32, u32) = (1_600, 1_000);

pub(super) fn window() -> Window {
    Window {
        resolution: WindowResolution::new(SCREENSHOT_RESOLUTION.0, SCREENSHOT_RESOLUTION.1)
            .with_scale_factor_override(1.0),
        ..default()
    }
}

/// 在 `DefaultPlugins` 之后调用：接管时间推进并登记出图任务。
pub(super) fn install(app: &mut App, path: PathBuf, tick: u64) {
    app.insert_resource(ScreenshotJob {
        frame: 0,
        tick,
        fired: false,
        path,
    })
    .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        128,
    )))
    .add_systems(Update, screenshot_at_frame);
}

/// screenshot 任务状态：至少预热 `SCREENSHOT_FRAME` 帧，在目标 tick spawn `Screenshot`，
/// 观察 `ScreenshotCaptured` 落盘后写 `AppExit`。
#[derive(Resource)]
struct ScreenshotJob {
    frame: u32,
    tick: u64,
    fired: bool,
    path: PathBuf,
}

fn screenshot_at_frame(
    mut commands: Commands,
    mut job: ResMut<ScreenshotJob>,
    session: Res<LaneFlowSession>,
) {
    job.frame = job.frame.saturating_add(1);
    if job.fired {
        return;
    }
    let remaining = job.tick.saturating_sub(session.world().tick_index());
    commands.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        remaining.min(8) * 16,
    )));
    if job.frame < SCREENSHOT_FRAME || remaining > 0 {
        return;
    }
    job.fired = true;
    println!(
        "junction_debug capture tick={}",
        session.world().tick_index()
    );
    commands.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    let path = job.path.clone();
    commands.spawn(Screenshot::primary_window()).observe(
        move |captured: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
            let result = match captured.image.clone().try_into_dynamic() {
                Ok(dynamic) => match dynamic.to_rgb8().save(&path) {
                    Ok(()) => {
                        info!("screenshot saved to {}", path.display());
                        AppExit::Success
                    }
                    Err(error) => {
                        error!("screenshot 保存失败: {error}");
                        AppExit::error()
                    }
                },
                Err(error) => {
                    error!("screenshot 转换失败: {error:?}");
                    AppExit::error()
                }
            };
            exit.write(result);
        },
    );
}
