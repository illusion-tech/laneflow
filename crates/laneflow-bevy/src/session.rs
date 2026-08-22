//! 单活动 LaneFlow Session：TrafficWorld + 可选 Spatial session。

use std::{num::NonZeroU32, time::Duration};

use bevy_ecs::resource::Resource;
use laneflow_runtime::{StepOutcome, TickInput, TrafficWorld};
use laneflow_spatial::SpatialSession;

use crate::LaneFlowAdapterError;

/// 单活动 Session 的 fixed-schedule 配置。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaneFlowSessionConfig {
    max_catch_up_steps: NonZeroU32,
}

impl LaneFlowSessionConfig {
    /// 创建显式 catch-up 上限配置。
    pub const fn new(max_catch_up_steps: NonZeroU32) -> Self {
        Self { max_catch_up_steps }
    }

    /// 返回单个 outer frame 允许的最大 step 数。
    pub const fn max_catch_up_steps(self) -> NonZeroU32 {
        self.max_catch_up_steps
    }
}

/// 最近一个 Bevy outer frame 的 LaneFlow 推进摘要。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LaneFlowFrameReport {
    frame_delta: Duration,
    steps_run: u32,
    backlog: Duration,
    catch_up_limit_reached: bool,
}

impl LaneFlowFrameReport {
    /// 返回宿主在该 outer frame 提供的 delta。
    pub const fn frame_delta(self) -> Duration {
        self.frame_delta
    }

    /// 返回该 outer frame 成功提交的 step 数。
    pub const fn steps_run(self) -> u32 {
        self.steps_run
    }

    /// 返回该 outer frame 结束后完整保留的时间 backlog。
    pub const fn backlog(self) -> Duration {
        self.backlog
    }

    /// 返回是否因为达到配置上限而仍有至少一个完整 fixed quantum 待处理。
    pub const fn catch_up_limit_reached(self) -> bool {
        self.catch_up_limit_reached
    }
}

/// 一个 Bevy `App` 中唯一活动的 LaneFlow runtime resource。
#[derive(Resource)]
pub struct LaneFlowSession {
    world: TrafficWorld,
    spatial: Option<SpatialSession>,
    config: LaneFlowSessionConfig,
    accumulator: Duration,
    frame_report: LaneFlowFrameReport,
    frame_step_results: Vec<StepOutcome>,
    pub(crate) last_error: Option<LaneFlowAdapterError>,
}

impl LaneFlowSession {
    /// 创建 Session。
    pub fn new(
        world: TrafficWorld,
        spatial: Option<SpatialSession>,
        config: LaneFlowSessionConfig,
    ) -> Self {
        Self {
            world,
            spatial,
            config,
            accumulator: Duration::ZERO,
            frame_report: LaneFlowFrameReport::default(),
            frame_step_results: Vec::new(),
            last_error: None,
        }
    }

    /// 交通世界。
    pub const fn world(&self) -> &TrafficWorld {
        &self.world
    }

    /// 可选 Spatial session。
    pub const fn spatial(&self) -> Option<&SpatialSession> {
        self.spatial.as_ref()
    }

    /// Session 配置。
    pub const fn config(&self) -> LaneFlowSessionConfig {
        self.config
    }

    /// 最近一个 outer frame 的推进摘要。
    pub const fn frame_report(&self) -> LaneFlowFrameReport {
        self.frame_report
    }

    /// 最近一个 outer frame 中按执行顺序提交的步进结果。
    pub fn frame_step_results(&self) -> &[StepOutcome] {
        &self.frame_step_results
    }

    /// 最近失败。
    pub const fn last_error(&self) -> Option<&LaneFlowAdapterError> {
        self.last_error.as_ref()
    }

    pub(crate) fn fixed_quantum(&self) -> Duration {
        Duration::from_millis(self.world.config().fixed_delta_time_ms())
    }

    pub(crate) fn begin_outer_frame(&mut self, frame_delta: Duration) -> bool {
        self.frame_step_results.clear();
        self.last_error = None;
        self.frame_report = LaneFlowFrameReport {
            frame_delta,
            steps_run: 0,
            backlog: self.accumulator,
            catch_up_limit_reached: false,
        };
        let Some(accumulator) = self.accumulator.checked_add(frame_delta) else {
            self.last_error = Some(LaneFlowAdapterError::AccumulatorOverflow {
                backlog: self.accumulator,
                frame_delta,
            });
            return false;
        };
        self.accumulator = accumulator;
        true
    }

    pub(crate) fn record_missing_time(&mut self) {
        self.frame_step_results.clear();
        self.last_error = Some(LaneFlowAdapterError::MissingTimeResource);
        self.frame_report = LaneFlowFrameReport {
            frame_delta: Duration::ZERO,
            steps_run: 0,
            backlog: self.accumulator,
            catch_up_limit_reached: false,
        };
    }

    pub(crate) fn can_step(&self) -> bool {
        self.last_error.is_none() && self.accumulator >= self.fixed_quantum()
    }

    pub(crate) fn step_world(&mut self) {
        if self.last_error.is_some() {
            return;
        }
        let delta = self.world.config().fixed_delta_time_ms();
        match self.world.step(TickInput::new(delta)) {
            Ok(result) => {
                self.accumulator = self
                    .accumulator
                    .checked_sub(self.fixed_quantum())
                    .unwrap_or(Duration::ZERO);
                self.frame_report.steps_run = self.frame_report.steps_run.saturating_add(1);
                self.frame_step_results.push(result);
            }
            Err(error) => {
                self.last_error = Some(LaneFlowAdapterError::StepFailed(error));
            }
        }
    }

    pub(crate) fn finish_outer_frame(&mut self) {
        self.frame_report.backlog = self.accumulator;
        self.frame_report.catch_up_limit_reached =
            self.last_error.is_none() && self.accumulator >= self.fixed_quantum();
    }
}
