//! Spatial 结构化错误。

use std::{error::Error, fmt};

use laneflow_core::EdgeHandle;

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
    /// registry 条目数量不能由内部 `u32` 槽位数量表达。
    RegistryCapacityExceeded {
        /// 实际条目数量。
        actual: usize,
        /// 支持的最大条目数量。
        max: usize,
    },
    /// edge handle 不属于目标 lane graph 的可解析范围。
    UnknownEdgeHandle {
        /// 无法解析的 Core edge handle。
        edge: EdgeHandle,
    },
    /// 同一个 Core edge handle 被重复绑定。
    DuplicateEdgeBinding {
        /// 重复的 Core edge handle。
        edge: EdgeHandle,
    },
    /// 目标 lane graph 的 edge 缺少 Spatial 绑定。
    MissingEdgeBinding {
        /// 缺少绑定的 Core edge handle。
        edge: EdgeHandle,
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
            Self::RegistryCapacityExceeded { actual, max } => write!(
                formatter,
                "Spatial registry 条目数量 {actual} 超过 u32 槽位上限 {max}"
            ),
            Self::UnknownEdgeHandle { edge } => {
                write!(formatter, "Spatial 绑定引用未知 edge handle {edge:?}")
            }
            Self::DuplicateEdgeBinding { edge } => {
                write!(formatter, "Spatial 绑定重复引用 edge handle {edge:?}")
            }
            Self::MissingEdgeBinding { edge } => {
                write!(formatter, "lane edge {edge:?} 缺少 Spatial 绑定")
            }
        }
    }
}

impl Error for SpatialError {}
