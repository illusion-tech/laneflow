//! C13 共享确定性输入脚本；截图与独立 headless 使用相同 frame 序列。
use std::time::Duration;

use bevy_app::{App, First};
use bevy_ecs::{resource::Resource, schedule::IntoScheduleConfigs, system::ResMut};
use bevy_time::{Time, TimeSystems, TimeUpdateStrategy, Virtual};
use sha2::{Digest, Sha256};

use super::{
    corridor_host::FrameSummary,
    corridor_scene::{CATALOG, LFCA, TICK_MS},
};

pub(super) const DEFAULT_CAPTURE_TICK: u64 = 12_000;
const WARMUP_FRAMES: u64 = 120;
pub(super) const DELTAS_US: [u64; 7] = [0, 500, 15_500, 128_000, 128_000, 160_000, 0];

#[derive(Resource)]
pub(super) struct ReplayClock {
    pub frame: u64,
    pub target_tick: u64,
    submitted: Duration,
}

impl ReplayClock {
    pub fn new(target_tick: u64) -> Result<Self, String> {
        if target_tick > 50_000 {
            return Err("验收截图上限为 50,000 Tick".into());
        }
        Ok(Self {
            frame: 0,
            target_tick,
            submitted: Duration::ZERO,
        })
    }

    pub fn ready(&self, summary: &FrameSummary) -> bool {
        self.frame > WARMUP_FRAMES
            && summary.tick == self.target_tick
            && summary.backlog.is_zero()
            && summary.steps == 0
    }

    fn advance(&mut self) -> Duration {
        self.frame += 1;
        if self.frame <= WARMUP_FRAMES {
            return Duration::ZERO;
        }
        let remaining = Duration::from_millis(self.target_tick * TICK_MS) - self.submitted;
        let delta = Duration::from_micros(
            DELTAS_US[((self.frame - WARMUP_FRAMES - 1) % DELTAS_US.len() as u64) as usize],
        )
        .min(remaining);
        self.submitted += delta;
        delta
    }
}

pub(super) fn install(app: &mut App, clock: ReplayClock) {
    app.insert_resource(clock)
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO))
        .add_systems(First, provide_delta.before(TimeSystems));
    app.world_mut()
        .resource_mut::<Time<Virtual>>()
        .set_max_delta(Duration::from_secs(60));
}

fn provide_delta(mut clock: ResMut<ReplayClock>, mut strategy: ResMut<TimeUpdateStrategy>) {
    *strategy = TimeUpdateStrategy::ManualDuration(clock.advance());
}

pub(super) fn evidence(summary: &FrameSummary) -> String {
    let signals = summary
        .signals
        .iter()
        .map(|(group, aspect)| format!("\"{}:{aspect:?}\"", group.raw()))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "schema = \"corridor-capture-1\"\nlfca_sha256 = \"{}\"\ncatalog_sha256 = \"{}\"\nframe = {}\ntick = {}\ntime_ms = {}\nseed = \"{}\"\nvehicles = {}\nrunning = {}\npending = {}\ndelta_ns = {}\nsteps = {}\nbacklog_ns = {}\ncatch_up_limit = {}\nlimited = {}\nwarmup_frames = {}\ndelta_script_us = {:?}\nsignals = [{}]\nhud = '''{}'''\n",
        Sha256::digest(LFCA)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
        Sha256::digest(CATALOG.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
        summary.frame,
        summary.tick,
        summary.time_ms,
        summary.seed,
        summary.counts.target,
        summary.counts.running,
        summary.counts.pending,
        summary.delta.as_nanos(),
        summary.steps,
        summary.backlog.as_nanos(),
        summary.catch_up_limit,
        summary.limited,
        WARMUP_FRAMES,
        DELTAS_US,
        signals,
        summary.hud(),
    )
}
