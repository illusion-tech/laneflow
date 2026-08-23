//! Adapter 可观察错误。

use std::fmt;
use std::time::Duration;

use laneflow_runtime::StepError;

/// LaneFlow Bevy Adapter 的结构化失败。
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LaneFlowAdapterError {
    /// outer frame 缺少 `Time` resource。
    MissingTimeResource,
    /// accumulator 溢出。
    AccumulatorOverflow {
        /// 溢出前 backlog。
        backlog: Duration,
        /// 本次 frame delta。
        frame_delta: Duration,
    },
    /// `TrafficWorld::step` 失败。
    StepFailed(StepError),
    /// `TrafficWorld` 与 `SpatialSession` 不是同一根 `Arc`。
    RevisionMismatch,
}

impl fmt::Display for LaneFlowAdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTimeResource => formatter.write_str("Bevy Time resource 缺失"),
            Self::AccumulatorOverflow {
                backlog,
                frame_delta,
            } => write!(
                formatter,
                "时间 accumulator 溢出：backlog={backlog:?} frame_delta={frame_delta:?}"
            ),
            Self::StepFailed(error) => write!(formatter, "TrafficWorld 步进失败：{error}"),
            Self::RevisionMismatch => formatter.write_str(
                "TrafficWorld 与 SpatialSession 必须绑定同一根 SharedNetworkRevision Arc",
            ),
        }
    }
}

impl std::error::Error for LaneFlowAdapterError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::StepFailed(error) => Some(error),
            Self::MissingTimeResource
            | Self::AccumulatorOverflow { .. }
            | Self::RevisionMismatch => None,
        }
    }
}
