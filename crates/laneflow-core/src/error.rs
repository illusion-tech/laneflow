//! Core runtime 的错误类型。

use std::fmt;

/// Core runtime 暴露给调用方的错误。
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CoreError {
    /// `CoreWorld` 的固定步长必须大于 0。
    InvalidFixedDeltaTime { fixed_delta_time_ms: u64 },
    /// tick 输入的 delta 必须等于当前 world 的固定步长。
    TickDeltaMismatch {
        expected_delta_time_ms: u64,
        actual_delta_time_ms: u64,
    },
    /// tick/time 累计发生整数溢出。
    TimeOverflow,
    /// speed 必须是 finite 且大于或等于 0。
    InvalidSpeed { speed: f64 },
    /// edge progress 必须是 finite 且大于或等于 0。
    InvalidEdgeProgress { edge_progress: f64 },
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFixedDeltaTime {
                fixed_delta_time_ms,
            } => write!(
                f,
                "fixed_delta_time_ms must be greater than 0, got {fixed_delta_time_ms}"
            ),
            Self::TickDeltaMismatch {
                expected_delta_time_ms,
                actual_delta_time_ms,
            } => write!(
                f,
                "tick delta mismatch: expected {expected_delta_time_ms} ms, got {actual_delta_time_ms} ms"
            ),
            Self::TimeOverflow => write!(f, "tick/time accumulation overflowed"),
            Self::InvalidSpeed { speed } => write!(f, "invalid speed {speed}"),
            Self::InvalidEdgeProgress { edge_progress } => {
                write!(f, "invalid edge progress {edge_progress}")
            }
        }
    }
}

impl std::error::Error for CoreError {}
