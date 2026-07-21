//! Bevy Adapter 的结构化错误。

use std::{error::Error, fmt, time::Duration};

use laneflow_core::CoreError;

/// LaneFlow Bevy outer-frame driver 最近一次失败。
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum LaneFlowAdapterError {
    /// `LaneFlowSession` 已安装，但宿主没有提供 Bevy `Time` resource。
    MissingTimeResource,
    /// 累加宿主 frame delta 时超出 [`Duration`] 可表达范围。
    AccumulatorOverflow {
        /// 进入本帧前保留的 backlog。
        backlog: Duration,
        /// 本帧宿主 delta。
        frame_delta: Duration,
    },
    /// LaneFlow Core fixed step 失败。
    CoreStep {
        /// 失败前的 committed tick index。
        tick_index: u64,
        /// Core 的结构化失败原因。
        source: CoreError,
    },
}

impl fmt::Display for LaneFlowAdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTimeResource => {
                formatter.write_str("LaneFlowSession 需要宿主提供 Bevy Time resource")
            }
            Self::AccumulatorOverflow {
                backlog,
                frame_delta,
            } => write!(
                formatter,
                "LaneFlow frame delta 累加溢出：backlog={backlog:?}, frame_delta={frame_delta:?}"
            ),
            Self::CoreStep { tick_index, source } => {
                write!(
                    formatter,
                    "LaneFlow Core 在 tick {tick_index} 后推进失败：{source}"
                )
            }
        }
    }
}

impl Error for LaneFlowAdapterError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CoreStep { source, .. } => Some(source),
            Self::MissingTimeResource | Self::AccumulatorOverflow { .. } => None,
        }
    }
}
