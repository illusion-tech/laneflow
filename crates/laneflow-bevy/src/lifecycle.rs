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
}

/// despawn 后 Runtime 事实与被原子删除的可选宿主映射。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaneFlowVehicleDespawnRecord {
    pub runtime: VehicleDespawnRecord,
    pub entity: Option<Entity>,
}

/// 在 `LaneFlowFixedSet::Lifecycle` 原子替换 Completed 车辆。
///
/// 已绑定车辆复用同一 Entity 并轮换到新句柄；未绑定保持未绑定。
/// `Blocked` 不写入 `last_error`，以便同一 boundary 继续处理其他计划。
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
                Err(source) => {
                    let error = LaneFlowAdapterError::VehicleReplace { old, source };
                    Err(record_error(&mut session, error))
                }
            }
        },
    )
}

/// 真正移除 live vehicle，并在同一同步边界删除可选 Runtime ↔ Entity 映射。
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
