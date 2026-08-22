use thiserror::Error;

/// `TrafficWorld::install` 失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum InstallError {
    /// `fixed_delta_time_ms` 必须为正。
    #[error("fixed_delta_time_ms 必须为正")]
    NonPositiveDelta,
    /// 本切片只允许 1-worker 执行计划。
    #[error("本切片只允许 1-worker 执行计划")]
    WorkerCountNotOne,
    /// 某个信号 phase 的 `durationMs` 短于固定步长。
    #[error("信号 phase 时长短于 fixed_delta_time_ms")]
    PhaseShorterThanTick,
    /// 信号 controller 的 cycle 非法。
    #[error("信号 controller cycle 必须为正且含 phase")]
    InvalidSignalProgram,
}

/// `TrafficWorld::step` 失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum StepError {
    /// 调用方 delta 与 world 固定步长不一致。
    #[error("tick delta 与 fixed_delta_time_ms 不一致")]
    DeltaMismatch {
        /// world 配置的固定步长。
        expected_delta_time_ms: u64,
        /// 本次输入的步长。
        actual_delta_time_ms: u64,
    },
    /// `tick_index` 或 `time_ms` 的 checked 加法溢出。
    #[error("tick_index 或 time_ms 溢出")]
    Overflow,
}

/// 静态路线或停车位等共享根序号查找失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum LookupError {
    /// 静态路线序号越界。
    #[error("静态路线序号越界")]
    UnknownStaticRoute,
}
