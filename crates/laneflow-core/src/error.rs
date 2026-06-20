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
                "`fixed_delta_time_ms` 必须大于 0，实际值为 {fixed_delta_time_ms}"
            ),
            Self::TickDeltaMismatch {
                expected_delta_time_ms,
                actual_delta_time_ms,
            } => write!(
                f,
                "tick delta 不匹配：期望 {expected_delta_time_ms} ms，实际 {actual_delta_time_ms} ms"
            ),
            Self::TimeOverflow => write!(f, "tick/time 累计发生整数溢出"),
            Self::InvalidSpeed { speed } => write!(f, "speed 无效：{speed}"),
            Self::InvalidEdgeProgress { edge_progress } => {
                write!(f, "edge progress 无效：{edge_progress}")
            }
        }
    }
}

impl std::error::Error for CoreError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_messages_use_chinese_runtime_text() {
        assert_eq!(
            CoreError::InvalidFixedDeltaTime {
                fixed_delta_time_ms: 0
            }
            .to_string(),
            "`fixed_delta_time_ms` 必须大于 0，实际值为 0"
        );
        assert_eq!(
            CoreError::TickDeltaMismatch {
                expected_delta_time_ms: 20,
                actual_delta_time_ms: 16
            }
            .to_string(),
            "tick delta 不匹配：期望 20 ms，实际 16 ms"
        );
        assert_eq!(
            CoreError::TimeOverflow.to_string(),
            "tick/time 累计发生整数溢出"
        );
        assert_eq!(
            CoreError::InvalidSpeed { speed: -1.0 }.to_string(),
            "speed 无效：-1"
        );
        assert_eq!(
            CoreError::InvalidEdgeProgress {
                edge_progress: f64::NAN
            }
            .to_string(),
            "edge progress 无效：NaN"
        );
    }
}
