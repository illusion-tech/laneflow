//! Caller-driven 车辆生命周期命令的 Bevy fixed-step 边界。

use bevy_ecs::{entity::Entity, world::World};
use laneflow_runtime::{
    ReplaceError, VehicleDespawnRecord, VehicleHandle, VehicleReplaceBlock, VehicleReplaceRecord,
    VehicleSpawnInput,
};

use crate::{LaneFlowAdapterError, LaneFlowSession};

/// 替换成功后的 Runtime identity 与可选 Bevy proxy。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaneFlowVehicleReplaceRecord {
    /// 立即 stale 的旧句柄。
    pub old: VehicleHandle,
    /// 新的 live 句柄。
    pub new: VehicleHandle,
    /// 复用的 proxy Entity；旧车未绑定时为 `None`。
    pub entity: Option<Entity>,
}

/// Adapter replacement 的成功或可恢复阻塞。
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum LaneFlowVehicleReplaceOutcome {
    /// Runtime 与 mapping 已一次提交。
    Replaced(LaneFlowVehicleReplaceRecord),
    /// 入口占用；Runtime、mapping 与 Transform 均不变。
    Blocked(VehicleReplaceBlock),
    /// 当前停车约束或前后车暂时不能接纳。世界与 mapping 不变，可稍后重试。
    Retryable(ReplaceError),
}

/// despawn 后 Runtime 事实与被原子删除的可选宿主映射。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaneFlowVehicleDespawnRecord {
    /// Runtime 侧移除事实。
    pub runtime: VehicleDespawnRecord,
    /// 被解除映射的宿主 Entity（实体本身仍存活，供宿主侧清理；未绑定为
    /// `None`）。
    pub entity: Option<Entity>,
}

/// 在 `LaneFlowFixedSet::Lifecycle` 原子替换 Completed 车辆。
///
/// 已绑定车辆复用同一 Entity 并轮换到新句柄；未绑定保持未绑定。
/// `Blocked` 与暂时不能接纳的停车约束、前车或后车结果不写入 `last_error`，以便同一
/// boundary 继续处理其他计划。降不到前方更低限速是致命错误，写入 `last_error`。
///
/// # Errors
///
/// Session 资源缺席时返回 [`LaneFlowAdapterError::MissingSessionForLifecycleCommand`]；
/// session 存在未消费的 `last_error` 时原样返回；替换车辆或其 Entity 绑定失效
/// （`UnknownVehicle` / `StaleLifecycleEntity`）、`DownstreamSpeedUnsatisfiable`、
/// `InvalidDepartureState`、`InitialSpeedExceedsDepartureBound` 与
/// 世界替换的其他致命错误记录到 `last_error` 并返回；[`ReplaceError::Blocked`] 与
/// 当前暂时不能接纳的停车约束、前车或后车错误转为可重试 outcome，不写入 `last_error`。
pub fn replace_completed_vehicle(
    world: &mut World,
    old: VehicleHandle,
    input: VehicleSpawnInput,
) -> Result<LaneFlowVehicleReplaceOutcome, LaneFlowAdapterError> {
    if !world.contains_resource::<LaneFlowSession>() {
        return Err(LaneFlowAdapterError::MissingSessionForLifecycleCommand);
    }

    world.resource_scope(
        |world, mut session: bevy_ecs::world::Mut<'_, LaneFlowSession>| {
            if let Some(error) = session.last_error.clone() {
                return Err(error);
            }

            let entity = match session.validate_replacement(old) {
                Ok(entity) => entity,
                Err(error) => return Err(record_error(&mut session, error)),
            };
            if let Some(entity) = entity
                && world.get_entity(entity).is_err()
            {
                let error = LaneFlowAdapterError::StaleLifecycleEntity {
                    vehicle: old,
                    entity,
                };
                return Err(record_error(&mut session, error));
            }

            match session.runtime_mut().replace_completed_vehicle(old, input) {
                Ok(VehicleReplaceRecord { old, new }) => {
                    session.rotate_replaced_vehicle(old, new, entity);
                    Ok(LaneFlowVehicleReplaceOutcome::Replaced(
                        LaneFlowVehicleReplaceRecord { old, new, entity },
                    ))
                }
                Err(ReplaceError::Blocked(block)) => {
                    Ok(LaneFlowVehicleReplaceOutcome::Blocked(block))
                }
                Err(source @ ReplaceError::StopConstraintUnsatisfiable)
                | Err(source @ ReplaceError::UnsafeLeader { .. })
                | Err(source @ ReplaceError::UnsafeFollower { .. }) => {
                    Ok(LaneFlowVehicleReplaceOutcome::Retryable(source))
                }
                Err(source) => {
                    let error = LaneFlowAdapterError::VehicleReplace { old, source };
                    Err(record_error(&mut session, error))
                }
            }
        },
    )
}

/// 真正移除 live vehicle，并在同一同步边界删除可选 Runtime ↔ Entity 映射。
///
/// # Errors
///
/// Session 资源缺席时返回 [`LaneFlowAdapterError::MissingSessionForLifecycleCommand`]；
/// session 存在未消费的 `last_error` 时原样返回；被映射 Entity 已消失时记录到
/// `last_error` 并返回 [`LaneFlowAdapterError::StaleLifecycleEntity`]；世界移除
/// 失败同样记录到 `last_error` 并返回。
pub fn despawn_vehicle(
    world: &mut World,
    vehicle: VehicleHandle,
) -> Result<LaneFlowVehicleDespawnRecord, LaneFlowAdapterError> {
    if !world.contains_resource::<LaneFlowSession>() {
        return Err(LaneFlowAdapterError::MissingSessionForLifecycleCommand);
    }
    world.resource_scope(
        |world, mut session: bevy_ecs::world::Mut<'_, LaneFlowSession>| {
            if let Some(error) = session.last_error.clone() {
                return Err(error);
            }
            let prepared = session.prepare_despawned_vehicle(vehicle);
            if let Some(prepared) = prepared
                && world.get_entity(prepared.entity()).is_err()
            {
                let error = LaneFlowAdapterError::StaleLifecycleEntity {
                    vehicle,
                    entity: prepared.entity(),
                };
                return Err(record_error(&mut session, error));
            }
            match session.runtime_mut().despawn_vehicle(vehicle) {
                Ok(runtime) => {
                    let entity = session.commit_despawned_vehicle(prepared);
                    Ok(LaneFlowVehicleDespawnRecord { runtime, entity })
                }
                Err(source) => {
                    let error = LaneFlowAdapterError::VehicleDespawn { vehicle, source };
                    Err(record_error(&mut session, error))
                }
            }
        },
    )
}

fn record_error(
    session: &mut LaneFlowSession,
    error: LaneFlowAdapterError,
) -> LaneFlowAdapterError {
    session.last_error = Some(error.clone());
    error
}
