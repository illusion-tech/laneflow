//! Bevy Adapter 的结构化错误。

use std::{error::Error, fmt, time::Duration};

use bevy_ecs::entity::Entity;
use laneflow_core::{CoreError, RouteHandle, VehicleHandle, VehicleStatus};
use laneflow_spatial::{FramePlacementToken, SpatialError};

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
    /// lifecycle command 在没有安装 [`crate::LaneFlowSession`] 时被调用。
    MissingSessionForLifecycleCommand,
    /// Core vehicle replacement 失败。
    CoreVehicleReplace {
        /// 调用方请求替换的旧 handle。
        old: VehicleHandle,
        /// Core 的结构化失败原因。
        source: CoreError,
    },
    /// replacement 前发现 Vehicle/Entity 部分双射已不一致。
    VehicleEntityMappingInconsistent {
        /// 正向映射中的 vehicle。
        vehicle: VehicleHandle,
        /// 正向映射中的 Entity。
        entity: Entity,
        /// 反向映射的实际 vehicle；缺失时为 `None`。
        reverse_vehicle: Option<VehicleHandle>,
    },
    /// replacement 前发现已绑定的 proxy Entity 已失效。
    StaleLifecycleEntity {
        /// 仍在映射中的 vehicle。
        vehicle: VehicleHandle,
        /// 已失效 Entity。
        entity: Entity,
    },
    /// bind/rebind 使用了当前 Core 中不存在的 vehicle handle。
    UnknownVehicleForBinding {
        /// 被拒绝的 vehicle handle。
        vehicle: VehicleHandle,
    },
    /// vehicle 已经绑定到另一个 Entity，或重复绑定同一对。
    VehicleAlreadyBound {
        /// 已绑定 vehicle。
        vehicle: VehicleHandle,
        /// 当前 Entity。
        current_entity: Entity,
        /// 请求 Entity。
        requested_entity: Entity,
    },
    /// Entity 已经绑定到另一个 vehicle。
    EntityAlreadyBound {
        /// 已绑定 Entity。
        entity: Entity,
        /// 当前 vehicle。
        current_vehicle: VehicleHandle,
        /// 请求 vehicle。
        requested_vehicle: VehicleHandle,
    },
    /// rebind 指定的 vehicle 当前没有绑定。
    VehicleNotBound {
        /// 未绑定 vehicle。
        vehicle: VehicleHandle,
    },
    /// frame root 变化时重复使用了当前 placement token。
    PlacementTokenReused {
        /// 当前 root。
        current_root: Entity,
        /// 请求 root。
        requested_root: Entity,
        /// 被重复使用的 token。
        token: FramePlacementToken,
    },
    /// 存在 vehicle/entity 映射，但尚未配置 frame placement。
    MissingFramePlacement,
    /// committed vehicle 指向了无法解析的 route。
    MissingVehicleRoute {
        /// 稳定输入序号。
        input_index: usize,
        /// vehicle handle。
        vehicle: VehicleHandle,
        /// route handle。
        route: RouteHandle,
    },
    /// committed vehicle 的 route edge index 无法解析。
    MissingVehicleRouteEdge {
        /// 稳定输入序号。
        input_index: usize,
        /// vehicle handle。
        vehicle: VehicleHandle,
        /// route-local edge index。
        route_edge_index: usize,
    },
    /// Parked vehicle 没有对应的 Occupied Parking binding。
    MissingParkedVehicleBinding {
        /// 稳定输入序号。
        input_index: usize,
        /// vehicle handle。
        vehicle: VehicleHandle,
    },
    /// 新增的 Core vehicle status 尚未定义 presentation source。
    UnsupportedVehicleStatus {
        /// 稳定输入序号。
        input_index: usize,
        /// vehicle handle。
        vehicle: VehicleHandle,
        /// 未支持状态。
        status: VehicleStatus,
    },
    /// Spatial 原子 pose batch 提取失败。
    SpatialBatch {
        /// Spatial 的结构化失败原因。
        source: SpatialError,
    },
    /// 已提取 batch 的 frame 与当前 Session frame 不同。
    PoseBatchFrameMismatch {
        /// 当前 Session frame。
        expected_frame: String,
        /// batch frame。
        actual_frame: String,
    },
    /// 已提取 batch 使用了旧 placement token。
    PoseBatchTokenMismatch {
        /// 当前 placement token。
        expected_token: FramePlacementToken,
        /// batch token。
        actual_token: FramePlacementToken,
    },
    /// frame-root Entity 已失效。
    StaleFrameRoot {
        /// 已登记 root Entity。
        root: Entity,
    },
    /// frame-root Entity 缺少 local Transform。
    FrameRootMissingTransform {
        /// root Entity。
        root: Entity,
    },
    /// frame-root Transform 包含非有限分量。
    NonFiniteFrameRootTransform {
        /// root Entity。
        root: Entity,
    },
    /// frame-root 使用了非单位缩放。
    NonUnitFrameRootScale {
        /// root Entity。
        root: Entity,
        /// 实际 XYZ scale。
        scale: [f32; 3],
    },
    /// 已映射的 proxy Entity 已失效。
    StaleMappedEntity {
        /// 稳定 batch 序号。
        input_index: usize,
        /// vehicle handle。
        vehicle: VehicleHandle,
        /// 已失效 Entity。
        entity: Entity,
    },
    /// 已映射的 proxy Entity 缺少 local Transform。
    MappedEntityMissingTransform {
        /// 稳定 batch 序号。
        input_index: usize,
        /// vehicle handle。
        vehicle: VehicleHandle,
        /// proxy Entity。
        entity: Entity,
    },
    /// 已映射的 proxy Entity 不是当前 frame-root 的直接 child。
    MappedEntityWrongParent {
        /// 稳定 batch 序号。
        input_index: usize,
        /// vehicle handle。
        vehicle: VehicleHandle,
        /// proxy Entity。
        entity: Entity,
        /// 期望 root。
        expected_root: Entity,
        /// 实际 parent；没有 parent 时为 `None`。
        actual_parent: Option<Entity>,
    },
    /// 已映射 Entity 的旧 Transform 或转换后的新 Transform 非有限。
    NonFiniteMappedTransform {
        /// 稳定 batch 序号。
        input_index: usize,
        /// vehicle handle。
        vehicle: VehicleHandle,
        /// proxy Entity。
        entity: Entity,
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
            Self::MissingSessionForLifecycleCommand => {
                formatter.write_str("vehicle lifecycle command 需要 LaneFlowSession resource")
            }
            Self::CoreVehicleReplace { old, source } => {
                write!(formatter, "Core 拒绝替换 vehicle {old:?}：{source}")
            }
            Self::VehicleEntityMappingInconsistent {
                vehicle,
                entity,
                reverse_vehicle,
            } => write!(
                formatter,
                "vehicle {vehicle:?} 正向映射到 {entity:?}，反向映射实际为 {reverse_vehicle:?}"
            ),
            Self::StaleLifecycleEntity { vehicle, entity } => write!(
                formatter,
                "replacement vehicle {vehicle:?} 映射到已失效 Entity {entity:?}"
            ),
            Self::UnknownVehicleForBinding { vehicle } => {
                write!(formatter, "无法绑定未知 Core vehicle {vehicle:?}")
            }
            Self::VehicleAlreadyBound {
                vehicle,
                current_entity,
                requested_entity,
            } => write!(
                formatter,
                "vehicle {vehicle:?} 已绑定到 {current_entity:?}，拒绝绑定 {requested_entity:?}"
            ),
            Self::EntityAlreadyBound {
                entity,
                current_vehicle,
                requested_vehicle,
            } => write!(
                formatter,
                "Entity {entity:?} 已绑定到 {current_vehicle:?}，拒绝绑定 {requested_vehicle:?}"
            ),
            Self::VehicleNotBound { vehicle } => {
                write!(formatter, "vehicle {vehicle:?} 当前没有 Entity 绑定")
            }
            Self::PlacementTokenReused {
                current_root,
                requested_root,
                token,
            } => write!(
                formatter,
                "frame root 从 {current_root:?} 切换到 {requested_root:?} 时重复使用 token {token:?}"
            ),
            Self::MissingFramePlacement => {
                formatter.write_str("存在 vehicle/entity 映射，但尚未配置 frame placement")
            }
            Self::MissingVehicleRoute {
                input_index,
                vehicle,
                route,
            } => write!(
                formatter,
                "presentation 输入 {input_index} 的 vehicle {vehicle:?} 无法解析 route {route:?}"
            ),
            Self::MissingVehicleRouteEdge {
                input_index,
                vehicle,
                route_edge_index,
            } => write!(
                formatter,
                "presentation 输入 {input_index} 的 vehicle {vehicle:?} 无法解析 route edge index {route_edge_index}"
            ),
            Self::MissingParkedVehicleBinding {
                input_index,
                vehicle,
            } => write!(
                formatter,
                "presentation 输入 {input_index} 的 Parked vehicle {vehicle:?} 缺少 Occupied Parking binding"
            ),
            Self::UnsupportedVehicleStatus {
                input_index,
                vehicle,
                status,
            } => write!(
                formatter,
                "presentation 输入 {input_index} 的 vehicle {vehicle:?} 使用未支持状态 {status:?}"
            ),
            Self::SpatialBatch { source } => {
                write!(formatter, "LaneFlow Spatial pose batch 提取失败：{source}")
            }
            Self::PoseBatchFrameMismatch {
                expected_frame,
                actual_frame,
            } => write!(
                formatter,
                "pose batch frame 不匹配：expected={expected_frame}, actual={actual_frame}"
            ),
            Self::PoseBatchTokenMismatch {
                expected_token,
                actual_token,
            } => write!(
                formatter,
                "pose batch placement token 不匹配：expected={expected_token:?}, actual={actual_token:?}"
            ),
            Self::StaleFrameRoot { root } => {
                write!(formatter, "frame-root Entity {root:?} 已失效")
            }
            Self::FrameRootMissingTransform { root } => {
                write!(formatter, "frame-root Entity {root:?} 缺少 Transform")
            }
            Self::NonFiniteFrameRootTransform { root } => {
                write!(formatter, "frame-root Entity {root:?} 的 Transform 非有限")
            }
            Self::NonUnitFrameRootScale { root, scale } => write!(
                formatter,
                "frame-root Entity {root:?} 必须使用单位缩放，actual={scale:?}"
            ),
            Self::StaleMappedEntity {
                input_index,
                vehicle,
                entity,
            } => write!(
                formatter,
                "pose record {input_index} 的 vehicle {vehicle:?} 映射到已失效 Entity {entity:?}"
            ),
            Self::MappedEntityMissingTransform {
                input_index,
                vehicle,
                entity,
            } => write!(
                formatter,
                "pose record {input_index} 的 vehicle {vehicle:?} 映射 Entity {entity:?} 缺少 Transform"
            ),
            Self::MappedEntityWrongParent {
                input_index,
                vehicle,
                entity,
                expected_root,
                actual_parent,
            } => write!(
                formatter,
                "pose record {input_index} 的 vehicle {vehicle:?} 映射 Entity {entity:?} parent 不匹配：expected={expected_root:?}, actual={actual_parent:?}"
            ),
            Self::NonFiniteMappedTransform {
                input_index,
                vehicle,
                entity,
            } => write!(
                formatter,
                "pose record {input_index} 的 vehicle {vehicle:?} 映射 Entity {entity:?} Transform 非有限"
            ),
        }
    }
}

impl Error for LaneFlowAdapterError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CoreStep { source, .. } => Some(source),
            Self::CoreVehicleReplace { source, .. } => Some(source),
            Self::SpatialBatch { source } => Some(source),
            Self::MissingTimeResource
            | Self::AccumulatorOverflow { .. }
            | Self::MissingSessionForLifecycleCommand
            | Self::VehicleEntityMappingInconsistent { .. }
            | Self::StaleLifecycleEntity { .. }
            | Self::UnknownVehicleForBinding { .. }
            | Self::VehicleAlreadyBound { .. }
            | Self::EntityAlreadyBound { .. }
            | Self::VehicleNotBound { .. }
            | Self::PlacementTokenReused { .. }
            | Self::MissingFramePlacement
            | Self::MissingVehicleRoute { .. }
            | Self::MissingVehicleRouteEdge { .. }
            | Self::MissingParkedVehicleBinding { .. }
            | Self::UnsupportedVehicleStatus { .. }
            | Self::PoseBatchFrameMismatch { .. }
            | Self::PoseBatchTokenMismatch { .. }
            | Self::StaleFrameRoot { .. }
            | Self::FrameRootMissingTransform { .. }
            | Self::NonFiniteFrameRootTransform { .. }
            | Self::NonUnitFrameRootScale { .. }
            | Self::StaleMappedEntity { .. }
            | Self::MappedEntityMissingTransform { .. }
            | Self::MappedEntityWrongParent { .. }
            | Self::NonFiniteMappedTransform { .. } => None,
        }
    }
}
