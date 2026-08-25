//! Spatial 结构化错误。

use std::{error::Error, fmt};

use laneflow_static_contract::{CanonicalFrameOrdinal, LaneEdgeOrdinal, ParkingSpaceOrdinal};

use crate::PoseRecordId;

/// 三维标准空间分量轴。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpatialAxis {
    /// X 轴。
    X,
    /// Y 轴。
    Y,
    /// Z 轴。
    Z,
}

impl fmt::Display for SpatialAxis {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::X => "X",
            Self::Y => "Y",
            Self::Z => "Z",
        })
    }
}

/// LaneFlow Spatial 权威边界的结构化错误。
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SpatialError {
    /// 标准坐标框架 ID 不满足稳定 token 语法。
    InvalidFrameId {
        /// 被拒绝的输入。
        value: String,
        /// 期望的稳定 token 模式。
        pattern: &'static str,
    },
    /// 标准空间值的某个分量不是有限数。
    NonFiniteComponent {
        /// 发生错误的 LaneFlow-owned 值类型。
        value_kind: &'static str,
        /// 非有限分量所在轴。
        axis: SpatialAxis,
        /// 被拒绝的数值。
        value: f32,
    },
    /// 标准点的某个分量超出 canonical frame 的受支持范围。
    PointComponentOutOfRange {
        /// 越界分量所在轴。
        axis: SpatialAxis,
        /// 被拒绝的数值，单位为米。
        value: f32,
        /// 允许的最小值，单位为米。
        min: f32,
        /// 允许的最大值，单位为米。
        max: f32,
    },
    /// 零向量不能成为单位方向。
    ZeroLengthDirection,
    /// 同一批 `PoseInput` 混用多个 canonical frame；失败不改 `output`。
    BatchFrameMismatch {
        /// 批次已锁定的 frame。
        expected_frame: CanonicalFrameOrdinal,
        /// 冲突记录的 frame。
        actual_frame: CanonicalFrameOrdinal,
    },
    /// 共享根 LaneEdge 序号无法采样。
    UnknownLaneEdge {
        /// 共享根边序号。
        edge: LaneEdgeOrdinal,
    },
    /// 共享根停车位序号无法采样。
    UnknownParkingSpace {
        /// 共享根停车位序号。
        space: ParkingSpaceOrdinal,
    },
    /// 共享根批次中一条记录失败。
    SharedPoseRecordFailed {
        /// 零基输入序号。
        input_index: usize,
        /// 调用方 pose 记录身份。
        record: PoseRecordId,
        /// 底层错误。
        source: Box<SpatialError>,
    },
    /// 共享根车道进度越界。
    SharedProgressOutOfRange {
        /// 共享根边序号。
        edge: LaneEdgeOrdinal,
        /// 输入进度（毫米）。
        progress_mm: u32,
        /// 边长（毫米）。
        length_mm: u32,
    },
    /// 共享根停车位姿派生失败。
    SharedParkingPoseComputation {
        /// 停车位序号。
        space: ParkingSpaceOrdinal,
        /// 派生阶段。
        operation: &'static str,
        /// 底层错误。
        source: Box<SpatialError>,
    },
}

impl fmt::Display for SpatialError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFrameId { value, pattern } => {
                write!(formatter, "标准坐标框架 ID {value:?} 不满足模式 {pattern}")
            }
            Self::NonFiniteComponent {
                value_kind,
                axis,
                value,
            } => write!(
                formatter,
                "{value_kind} 的 {axis} 分量必须是有限数，实际为 {value:?}"
            ),
            Self::PointComponentOutOfRange {
                axis,
                value,
                min,
                max,
            } => write!(
                formatter,
                "标准点的 {axis} 分量 {value:?} m 超出闭区间 [{min:?}, {max:?}] m"
            ),
            Self::ZeroLengthDirection => formatter.write_str("零向量不能归一化为单位方向"),
            Self::BatchFrameMismatch {
                expected_frame,
                actual_frame,
            } => write!(
                formatter,
                "位姿批次 frame {actual_frame:?} 与 {expected_frame:?} 不一致"
            ),
            Self::UnknownLaneEdge { edge } => {
                write!(formatter, "共享根 LaneEdge {edge} 无法采样")
            }
            Self::UnknownParkingSpace { space } => {
                write!(formatter, "共享根停车位 {space} 无法采样")
            }
            Self::SharedPoseRecordFailed {
                input_index,
                record,
                source,
            } => write!(
                formatter,
                "共享根位姿输入 {input_index}（record {record:?}）失败: {source}"
            ),
            Self::SharedProgressOutOfRange {
                edge,
                progress_mm,
                length_mm,
            } => write!(
                formatter,
                "LaneEdge {edge} 的采样进度 {progress_mm} mm 超出闭区间 [0, {length_mm}] mm"
            ),
            Self::SharedParkingPoseComputation {
                space,
                operation,
                source,
            } => write!(
                formatter,
                "停车位 {space} 的 {operation} 无法保持 canonical 位姿不变量: {source}"
            ),
        }
    }
}

impl Error for SpatialError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::SharedPoseRecordFailed { source, .. }
            | Self::SharedParkingPoseComputation { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}
