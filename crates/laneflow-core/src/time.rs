//! fixed-step tick 的输入与输出原语。

use crate::event::CoreEvent;

/// 单次 Core step 的显式输入。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickInput {
    /// 调用方传入的固定步长，必须等于 `CoreWorld` 的配置。
    pub delta_time_ms: u64,
}

impl TickInput {
    /// 创建必填 delta 的 tick 输入。
    pub const fn new(delta_time_ms: u64) -> Self {
        Self { delta_time_ms }
    }
}

/// 单次 Core step 的可观察结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepResult {
    /// 成功 step 后的 tick index。
    pub tick_index: u64,
    /// 成功 step 后的累计 simulation time。
    pub time_ms: u64,
    /// 本次 step 产生的事件。
    pub events: Vec<CoreEvent>,
}
