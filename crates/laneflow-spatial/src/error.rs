//! Spatial 结构化错误。

use std::{error::Error, fmt};

use laneflow_core::{EdgeHandle, ParkingSpaceHandle, VehicleHandle};

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
    /// 一条中心线不足以形成线段。
    InsufficientPolylinePoints {
        /// 对应的 Core edge handle。
        edge: EdgeHandle,
        /// 实际点数量。
        actual: usize,
        /// 最小点数量。
        min: usize,
    },
    /// 线段没有严格超过最小长度。
    DegenerateSegment {
        /// 对应的 Core edge handle。
        edge: EdgeHandle,
        /// 零基线段序号。
        segment_index: usize,
        /// runtime `f32` 线段长度，单位为米。
        length_meters: f32,
        /// 必须严格超过的下限，单位为米。
        min_exclusive_meters: f32,
    },
    /// 累计弧长溢出或因 `f32` 精度停滞而未严格递增。
    ArcLengthAccumulationFailed {
        /// 对应的 Core edge handle。
        edge: EdgeHandle,
        /// 零基线段序号。
        segment_index: usize,
        /// 加入当前线段前的累计弧长，单位为米。
        accumulated_meters: f32,
        /// 当前线段长度，单位为米。
        segment_length_meters: f32,
    },
    /// 线段太接近 canonical Y 轴，无法构造稳定的 Y-up 朝向基。
    DegenerateBasis {
        /// 对应的 Core edge handle。
        edge: EdgeHandle,
        /// 零基线段序号。
        segment_index: usize,
        /// canonical `+Y` 投影长度。
        projected_up_length: f32,
        /// 允许的最小闭区间边界。
        min_inclusive: f32,
    },
    /// Core 权威长度与量化后几何弧长不一致。
    LengthMismatch {
        /// 对应的 Core edge handle。
        edge: EdgeHandle,
        /// Core 权威长度，单位为米。
        core_length_meters: f64,
        /// runtime `f32` 几何弧长，单位为米。
        geometry_arc_length_meters: f32,
        /// 两者绝对差，单位为米。
        difference_meters: f64,
        /// 本次长度允许的总容差，单位为米。
        tolerance_meters: f64,
    },
    /// LaneGraph 声明连接的两个 edge 在 runtime 几何中不连续。
    DisconnectedEdgeJoin {
        /// 上游 edge。
        from_edge: EdgeHandle,
        /// 下游 edge。
        to_edge: EdgeHandle,
        /// 上游终点与下游起点的距离，单位为米。
        distance_meters: f32,
        /// 允许的最大距离，单位为米。
        tolerance_meters: f32,
    },
    /// 采样进度超过 Core edge 的精确有效范围。
    ProgressOutOfRange {
        /// 被采样的 edge。
        edge: EdgeHandle,
        /// 输入进度，单位为米。
        progress_meters: f64,
        /// Core edge 长度，单位为米。
        max_meters: f64,
    },
    /// 插值位置无法重新进入 canonical 点不变量。
    SamplePositionComputation {
        /// 被采样的 edge。
        edge: EdgeHandle,
        /// 零基线段序号。
        segment_index: usize,
        /// 底层 canonical 点错误。
        source: Box<SpatialError>,
    },
    /// committed batch 的 frame 与当前 registry 不一致。
    BatchFrameMismatch {
        /// 当前 Spatial registry 的 frame ID。
        registry_frame_id: String,
        /// 调用方 committed output 的 frame ID。
        output_frame_id: String,
    },
    /// ParkingSpace handle 不属于调用方提供的 Parking registry。
    UnknownParkingSpaceHandle {
        /// 无法解析的 Core ParkingSpace handle。
        space: ParkingSpaceHandle,
    },
    /// Core Parking geometry 值不能有限地表示为 canonical `f32` 输入。
    ParkingGeometryOutOfF32Range {
        /// 对应的 ParkingSpace handle。
        space: ParkingSpaceHandle,
        /// 固定的 geometry 字段名。
        field: &'static str,
        /// 被拒绝的 Core `f64` 值。
        value: f64,
    },
    /// Parking anchor pose 的派生运算无法保持 canonical 不变量。
    ParkingPoseComputation {
        /// 对应的 ParkingSpace handle。
        space: ParkingSpaceHandle,
        /// 固定的派生阶段名称。
        operation: &'static str,
        /// 底层 canonical 空间错误。
        source: Box<SpatialError>,
    },
    /// 某条稳定批量输入无法生成位姿。
    PoseRecordFailed {
        /// 零基输入序号。
        input_index: usize,
        /// 对应的稳定 vehicle handle。
        vehicle: VehicleHandle,
        /// 底层 lane 或 Parking 位姿错误。
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
            Self::InsufficientPolylinePoints { edge, actual, min } => write!(
                formatter,
                "edge {edge:?} 的中心线只有 {actual} 个点，至少需要 {min} 个"
            ),
            Self::DegenerateSegment {
                edge,
                segment_index,
                length_meters,
                min_exclusive_meters,
            } => write!(
                formatter,
                "edge {edge:?} 的线段 {segment_index} 长度 {length_meters:?} m 必须严格大于 {min_exclusive_meters:?} m"
            ),
            Self::ArcLengthAccumulationFailed {
                edge,
                segment_index,
                accumulated_meters,
                segment_length_meters,
            } => write!(
                formatter,
                "edge {edge:?} 的线段 {segment_index} 无法把 {segment_length_meters:?} m 严格累加到 {accumulated_meters:?} m"
            ),
            Self::DegenerateBasis {
                edge,
                segment_index,
                projected_up_length,
                min_inclusive,
            } => write!(
                formatter,
                "edge {edge:?} 的线段 {segment_index} projected-up 长度 {projected_up_length:?} 小于 {min_inclusive:?}"
            ),
            Self::LengthMismatch {
                edge,
                core_length_meters,
                geometry_arc_length_meters,
                difference_meters,
                tolerance_meters,
            } => write!(
                formatter,
                "edge {edge:?} 的 Core 长度 {core_length_meters:?} m 与几何弧长 {geometry_arc_length_meters:?} m 相差 {difference_meters:?} m，超过容差 {tolerance_meters:?} m"
            ),
            Self::DisconnectedEdgeJoin {
                from_edge,
                to_edge,
                distance_meters,
                tolerance_meters,
            } => write!(
                formatter,
                "edge {from_edge:?} 到 {to_edge:?} 的端点距离 {distance_meters:?} m 超过容差 {tolerance_meters:?} m"
            ),
            Self::ProgressOutOfRange {
                edge,
                progress_meters,
                max_meters,
            } => write!(
                formatter,
                "edge {edge:?} 的采样进度 {progress_meters:?} m 超出闭区间 [0, {max_meters:?}] m"
            ),
            Self::SamplePositionComputation {
                edge,
                segment_index,
                source,
            } => write!(
                formatter,
                "edge {edge:?} 的线段 {segment_index} 插值位置无效: {source}"
            ),
            Self::BatchFrameMismatch {
                registry_frame_id,
                output_frame_id,
            } => write!(
                formatter,
                "位姿批次 frame {output_frame_id:?} 与 Spatial registry frame {registry_frame_id:?} 不一致"
            ),
            Self::UnknownParkingSpaceHandle { space } => {
                write!(formatter, "位姿输入引用未知 ParkingSpace handle {space:?}")
            }
            Self::ParkingGeometryOutOfF32Range {
                space,
                field,
                value,
            } => write!(
                formatter,
                "ParkingSpace {space:?} 的 {field} 值 {value:?} 无法有限表示为 canonical f32"
            ),
            Self::ParkingPoseComputation {
                space,
                operation,
                source,
            } => write!(
                formatter,
                "ParkingSpace {space:?} 的 {operation} 无法保持 canonical 位姿不变量: {source}"
            ),
            Self::PoseRecordFailed {
                input_index,
                vehicle,
                source,
            } => write!(
                formatter,
                "位姿批量输入 {input_index}（vehicle {vehicle:?}）失败: {source}"
            ),
        }
    }
}

impl Error for SpatialError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::SamplePositionComputation { source, .. }
            | Self::ParkingPoseComputation { source, .. }
            | Self::PoseRecordFailed { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}
