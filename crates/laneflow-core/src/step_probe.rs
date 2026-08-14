//! 仪器探针边界（instrumentation probe boundary）：B 类性能归因计时的正式接缝。
//!
//! 生产路径持有空操作（no-op）默认的探针：六段计时（occupancy、longitudinal
//! 总计、longitudinal proposal / projection、post-longitudinal、research-commit）
//! 收敛为本模块的探针方法。探针以 method-generic 入口承载——`CoreWorld::step`
//! 公开签名不变并始终以 [`NoOpProbe`] 运行；`ENABLED = false` 时热路径不读取
//! `Instant`，空操作实现经 monomorphization 与常量折叠整体编译消除。
//!
//! 研究态实现 [`StageTimingProbe`] 由 crate feature `instrumentation` 或 crate 内
//! 测试启用，供 Criterion 取证与未来研究 harness（#218 等）注入探针实现时恢复
//! 测量；D5 的 reduced-rate 对比（H1/C4 行全部列）随 P4 降级暂停可运行复现，
//! 生产路径探针在 P0 workload 上的五段计时复现能力完整保留。

use std::time::Duration;

/// step 六段计时的探针契约。
///
/// 所有方法提供空默认实现，实现者只覆盖关心的阶段。`ENABLED` 为关联常量：
/// 为 `false` 时生产调用点不构造任何 `Instant`，探针调用经 monomorphization
/// 与常量折叠整体编译消除，保证 no-op 默认在发布构建零成本。
#[doc(hidden)]
pub trait StepProbe {
    /// 探针是否记录；`false` 时调用点整体编译消除（不读取时钟）。
    const ENABLED: bool;

    /// occupancy / leader rebuild 阶段耗时。
    fn note_occupancy_duration(&mut self, _duration: Duration) {}

    /// longitudinal rebuild 总计耗时。
    fn note_longitudinal_duration(&mut self, _duration: Duration) {}

    /// longitudinal 内 proposal / projection 分解耗时。
    fn note_longitudinal_breakdown(&mut self, _proposal: Duration, _projection: Duration) {}

    /// post-longitudinal（advance / events / authority commit）阶段耗时。
    fn note_post_longitudinal_duration(&mut self, _duration: Duration) {}

    /// research cache commit 阶段耗时（仅研究 harness 路径调用）。
    fn note_research_commit_duration(&mut self, _duration: Duration) {}
}

/// 空操作默认探针：生产路径的零成本默认。
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default)]
pub struct NoOpProbe;

impl StepProbe for NoOpProbe {
    const ENABLED: bool = false;
}

/// 单 step 六段计时快照。
#[cfg(any(test, feature = "instrumentation"))]
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StepStageTimings {
    /// occupancy / leader rebuild 阶段耗时。
    pub occupancy: Duration,
    /// longitudinal rebuild 总计耗时。
    pub longitudinal: Duration,
    /// longitudinal 内 proposal 阶段耗时。
    pub longitudinal_proposal: Duration,
    /// longitudinal 内 projection 阶段耗时。
    pub longitudinal_projection: Duration,
    /// post-longitudinal 阶段耗时。
    pub post_longitudinal: Duration,
    /// research cache commit 阶段耗时。
    pub research_commit: Duration,
}

/// 研究态记录探针：记录最近一次 step 的六段计时。
#[cfg(any(test, feature = "instrumentation"))]
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default)]
pub struct StageTimingProbe {
    last_step: StepStageTimings,
}

#[cfg(any(test, feature = "instrumentation"))]
impl StageTimingProbe {
    /// 最近一次 step 的六段计时快照。
    pub fn last_step(&self) -> StepStageTimings {
        self.last_step
    }
}

#[cfg(any(test, feature = "instrumentation"))]
impl StepProbe for StageTimingProbe {
    const ENABLED: bool = true;

    fn note_occupancy_duration(&mut self, duration: Duration) {
        self.last_step.occupancy = duration;
    }

    fn note_longitudinal_duration(&mut self, duration: Duration) {
        self.last_step.longitudinal = duration;
    }

    fn note_longitudinal_breakdown(&mut self, proposal: Duration, projection: Duration) {
        self.last_step.longitudinal_proposal = proposal;
        self.last_step.longitudinal_projection = projection;
    }

    fn note_post_longitudinal_duration(&mut self, duration: Duration) {
        self.last_step.post_longitudinal = duration;
    }

    fn note_research_commit_duration(&mut self, duration: Duration) {
        self.last_step.research_commit = duration;
    }
}
