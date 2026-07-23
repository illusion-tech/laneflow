//! Caller-driven vehicle lifecycle command 的 Bevy fixed-step boundary。

use bevy_ecs::{entity::Entity, world::World};
use laneflow_core::{
    VehicleHandle, VehicleReplaceBlock, VehicleReplaceInput, VehicleReplaceOutcome,
};

use crate::{LaneFlowAdapterError, LaneFlowSession};

/// replacement 成功后的 Core identity 与可选 Bevy proxy identity。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaneFlowVehicleReplaceRecord {
    /// 已立即变为 stale 的旧 Core handle。
    pub old: VehicleHandle,
    /// replacement 的新 live Core handle。
    pub new: VehicleHandle,
    /// 继续承载该 vehicle 的既有 proxy Entity；旧 vehicle 未绑定时为 `None`。
    pub entity: Option<Entity>,
}

/// Adapter replacement command 的成功或可恢复阻塞结果。
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum LaneFlowVehicleReplaceOutcome {
    /// Core 与 Adapter mapping 已一次提交。
    Replaced(LaneFlowVehicleReplaceRecord),
    /// 入口当前发生物理重叠；Core、mapping 与 Transform 均保持不变。
    Blocked(VehicleReplaceBlock),
}

/// 在 [`crate::LaneFlowFixedSet::Lifecycle`] boundary 原子替换 Completed vehicle。
///
/// 已绑定 vehicle 会复用同一个 Entity 并把映射轮换到新 handle；未绑定 vehicle
/// 只提交 Core replacement。可恢复重叠返回 [`LaneFlowVehicleReplaceOutcome::Blocked`]。
/// 其他错误会记录到 [`LaneFlowSession::last_error`]，从而停止当前 outer frame 的
/// Core step 与后续 catch-up step，并完整保留时间 backlog。
pub fn replace_completed_vehicle(
    world: &mut World,
    old: VehicleHandle,
    input: &VehicleReplaceInput,
) -> Result<LaneFlowVehicleReplaceOutcome, LaneFlowAdapterError> {
    if !world.contains_resource::<LaneFlowSession>() {
        return Err(LaneFlowAdapterError::MissingSessionForLifecycleCommand);
    }

    world.resource_scope(
        |world, mut session: bevy_ecs::world::Mut<'_, LaneFlowSession>| {
            if let Some(error) = session.last_error.clone() {
                return Err(error);
            }

            let entity = match session.vehicle_entities.validate_replacement(old) {
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

            match session.core.replace_completed_vehicle(old, input) {
                Ok(VehicleReplaceOutcome::Blocked(block)) => {
                    Ok(LaneFlowVehicleReplaceOutcome::Blocked(block))
                }
                Ok(VehicleReplaceOutcome::Replaced(record)) => {
                    session
                        .vehicle_entities
                        .rotate_replaced_vehicle(record.old, record.new, entity);
                    Ok(LaneFlowVehicleReplaceOutcome::Replaced(
                        LaneFlowVehicleReplaceRecord {
                            old: record.old,
                            new: record.new,
                            entity,
                        },
                    ))
                }
                Err(source) => {
                    let error = LaneFlowAdapterError::CoreVehicleReplace { old, source };
                    Err(record_error(&mut session, error))
                }
                Ok(_) => unreachable!(
                    "laneflow-core and laneflow-bevy are released in lockstep; every current replacement outcome is handled"
                ),
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
