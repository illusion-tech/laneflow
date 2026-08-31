//! Adapter 可观察错误。

use std::fmt;
use std::time::Duration;

use bevy_ecs::entity::Entity;
use laneflow_runtime::{ParkingError, ReplaceError, StepError, VehicleHandle};

/// LaneFlow Bevy Adapter 的结构化失败。
#[derive(Clone, Debug, PartialEq)]
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
    /// 生命周期命令缺少 `LaneFlowSession`。
    MissingSessionForLifecycleCommand,
    /// 车辆句柄不在当前 world。
    UnknownVehicle {
        /// 未知句柄。
        vehicle: VehicleHandle,
    },
    /// 同一车辆已绑定其他 Entity。
    DuplicateVehicleBinding {
        /// 车辆。
        vehicle: VehicleHandle,
        /// 已有 Entity。
        existing: Entity,
        /// 本次请求。
        requested: Entity,
    },
    /// 同一 Entity 已绑定其他车辆。
    DuplicateEntityBinding {
        /// Entity。
        entity: Entity,
        /// 已有车辆。
        existing: VehicleHandle,
        /// 本次请求。
        requested: VehicleHandle,
    },
    /// 已绑定 Entity 已失效。
    StaleLifecycleEntity {
        /// 车辆。
        vehicle: VehicleHandle,
        /// 失效 Entity。
        entity: Entity,
    },
    /// Runtime 原子替换致命失败。
    VehicleReplace {
        /// 旧句柄。
        old: VehicleHandle,
        /// Runtime 错误。
        source: ReplaceError,
    },
    /// Runtime 真正移除车辆失败；mapping 保持不变。
    VehicleDespawn {
        vehicle: VehicleHandle,
        source: ParkingError,
    },
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
            Self::MissingSessionForLifecycleCommand => {
                formatter.write_str("生命周期命令需要 LaneFlowSession")
            }
            Self::UnknownVehicle { vehicle } => {
                write!(formatter, "未知车辆句柄：{vehicle:?}")
            }
            Self::DuplicateVehicleBinding {
                vehicle,
                existing,
                requested,
            } => write!(
                formatter,
                "车辆 {vehicle:?} 已绑定 {existing:?}，不能再绑 {requested:?}"
            ),
            Self::DuplicateEntityBinding {
                entity,
                existing,
                requested,
            } => write!(
                formatter,
                "Entity {entity:?} 已绑定 {existing:?}，不能再绑 {requested:?}"
            ),
            Self::StaleLifecycleEntity { vehicle, entity } => {
                write!(
                    formatter,
                    "车辆 {vehicle:?} 绑定的 Entity {entity:?} 已失效"
                )
            }
            Self::VehicleReplace { old, source } => {
                write!(formatter, "车辆 {old:?} 原子替换失败：{source}")
            }
            Self::VehicleDespawn { vehicle, source } => {
                write!(formatter, "车辆 {vehicle:?} despawn 失败：{source}")
            }
        }
    }
}

impl std::error::Error for LaneFlowAdapterError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::StepFailed(error) => Some(error),
            Self::MissingTimeResource
            | Self::AccumulatorOverflow { .. }
            | Self::RevisionMismatch
            | Self::MissingSessionForLifecycleCommand
            | Self::UnknownVehicle { .. }
            | Self::DuplicateVehicleBinding { .. }
            | Self::DuplicateEntityBinding { .. }
            | Self::StaleLifecycleEntity { .. } => None,
            Self::VehicleReplace { source, .. } => Some(source),
            Self::VehicleDespawn { source, .. } => Some(source),
        }
    }
}
