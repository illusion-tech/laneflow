//! 单活动 LaneFlow Session 与 outer-frame 可观察状态。

use std::{num::NonZeroU32, time::Duration};

use bevy_ecs::resource::Resource;
use laneflow_core::{CoreWorld, StepResult};
use laneflow_spatial::{CanonicalPoseBatchScratch, SpatialRegistry};

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

    /// 返回单个 outer frame 允许的最大 Core step 数。
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

    /// 返回该 outer frame 成功提交的 Core step 数。
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
    core: CoreWorld,
    spatial: SpatialRegistry,
    pose_scratch: CanonicalPoseBatchScratch,
    config: LaneFlowSessionConfig,
    accumulator: Duration,
    frame_report: LaneFlowFrameReport,
    frame_step_results: Vec<StepResult>,
    last_error: Option<LaneFlowAdapterError>,
}

impl LaneFlowSession {
    /// 创建不预留 pose scratch 容量的 Session。
    pub fn new(core: CoreWorld, spatial: SpatialRegistry, config: LaneFlowSessionConfig) -> Self {
        Self::with_pose_capacity(core, spatial, config, 0)
    }

    /// 创建并预留 Adapter pose scratch 容量的 Session。
    pub fn with_pose_capacity(
        core: CoreWorld,
        spatial: SpatialRegistry,
        config: LaneFlowSessionConfig,
        pose_capacity: usize,
    ) -> Self {
        Self {
            core,
            spatial,
            pose_scratch: CanonicalPoseBatchScratch::with_capacity(pose_capacity),
            config,
            accumulator: Duration::ZERO,
            frame_report: LaneFlowFrameReport::default(),
            frame_step_results: Vec::new(),
            last_error: None,
        }
    }

    /// 返回 Core 权威状态的只读视图。
    pub const fn core(&self) -> &CoreWorld {
        &self.core
    }

    /// 返回 Spatial 权威注册表的只读视图。
    pub const fn spatial(&self) -> &SpatialRegistry {
        &self.spatial
    }

    /// 返回 Session 配置。
    pub const fn config(&self) -> LaneFlowSessionConfig {
        self.config
    }

    /// 返回当前完整保留的时间 backlog。
    pub const fn accumulator(&self) -> Duration {
        self.accumulator
    }

    /// 返回最近一个 outer frame 的推进摘要。
    pub const fn frame_report(&self) -> LaneFlowFrameReport {
        self.frame_report
    }

    /// 返回最近一个 outer frame 中按执行顺序提交的全部 Core step 结果。
    pub fn frame_step_results(&self) -> &[StepResult] {
        &self.frame_step_results
    }

    /// 返回最近一个 outer frame 的结构化失败；成功或尚未运行时返回 `None`。
    pub const fn last_error(&self) -> Option<&LaneFlowAdapterError> {
        self.last_error.as_ref()
    }

    /// 返回可复用 pose scratch 的当前容量。
    pub const fn pose_scratch_capacity(&self) -> usize {
        self.pose_scratch.capacity()
    }

    pub(crate) fn fixed_quantum(&self) -> Duration {
        Duration::from_millis(self.core.fixed_delta_time_ms())
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

    pub(crate) fn step_core(&mut self) {
        if self.last_error.is_some() {
            return;
        }

        let tick_index = self.core.tick_index();
        let fixed_delta_time_ms = self.core.fixed_delta_time_ms();
        match self
            .core
            .step(laneflow_core::TickInput::new(fixed_delta_time_ms))
        {
            Ok(result) => {
                self.accumulator = self
                    .accumulator
                    .checked_sub(Duration::from_millis(fixed_delta_time_ms))
                    .expect("driver only runs a Core step when one full quantum is available");
                self.frame_step_results.push(result);
            }
            Err(source) => {
                self.last_error = Some(LaneFlowAdapterError::CoreStep { tick_index, source });
            }
        }
    }

    pub(crate) fn finish_outer_frame(&mut self) {
        let steps_run = u32::try_from(self.frame_step_results.len())
            .expect("successful steps cannot exceed the configured u32 catch-up limit");
        self.frame_report.steps_run = steps_run;
        self.frame_report.backlog = self.accumulator;
        self.frame_report.catch_up_limit_reached = self.last_error.is_none()
            && steps_run == self.config.max_catch_up_steps.get()
            && self.accumulator >= self.fixed_quantum();
    }
}
