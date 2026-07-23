//! Vehicle/Entity 部分双射与 Bevy local Transform 原子提交。

use std::collections::HashMap;

use bevy_ecs::{entity::Entity, hierarchy::ChildOf, world::World};
use bevy_transform::components::Transform;
use laneflow_core::{VehicleHandle, VehicleParkingState, VehicleStatus};
use laneflow_spatial::{FramePlacementToken, PoseInputRecord};

#[cfg(feature = "debug-gizmos")]
use laneflow_spatial::CanonicalPoseBatchF32;

use crate::{LaneFlowAdapterError, LaneFlowSession};

/// 单活动 canonical frame 在 Bevy 场景中的 root 与 placement identity。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaneFlowFramePlacement {
    root: Entity,
    token: FramePlacementToken,
}

impl LaneFlowFramePlacement {
    /// 创建显式 frame-root placement。
    pub const fn new(root: Entity, token: FramePlacementToken) -> Self {
        Self { root, token }
    }

    /// 返回承载 canonical frame rigid placement 的 root Entity。
    pub const fn root(self) -> Entity {
        self.root
    }

    /// 返回本次 placement 的 opaque identity token。
    pub const fn token(self) -> FramePlacementToken {
        self.token
    }
}

/// Adapter-owned `VehicleHandle <-> Entity` 部分双射的只读视图。
#[derive(Debug, Default)]
pub struct LaneFlowVehicleEntityMap {
    by_vehicle: HashMap<VehicleHandle, Entity>,
    by_entity: HashMap<Entity, VehicleHandle>,
}

impl LaneFlowVehicleEntityMap {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            by_vehicle: HashMap::with_capacity(capacity),
            by_entity: HashMap::with_capacity(capacity),
        }
    }

    /// 返回当前绑定数量。
    pub fn len(&self) -> usize {
        self.by_vehicle.len()
    }

    /// 返回当前是否没有绑定。
    pub fn is_empty(&self) -> bool {
        self.by_vehicle.is_empty()
    }

    /// 返回稳定容量下至少可容纳的绑定数量。
    pub fn capacity(&self) -> usize {
        self.by_vehicle.capacity().min(self.by_entity.capacity())
    }

    /// 返回 vehicle 当前绑定的 proxy Entity。
    pub fn entity(&self, vehicle: VehicleHandle) -> Option<Entity> {
        self.by_vehicle.get(&vehicle).copied()
    }

    /// 返回 Entity 当前绑定的 vehicle。
    pub fn vehicle(&self, entity: Entity) -> Option<VehicleHandle> {
        self.by_entity.get(&entity).copied()
    }

    fn bind(&mut self, vehicle: VehicleHandle, entity: Entity) -> Result<(), LaneFlowAdapterError> {
        if let Some(current_entity) = self.entity(vehicle) {
            return Err(LaneFlowAdapterError::VehicleAlreadyBound {
                vehicle,
                current_entity,
                requested_entity: entity,
            });
        }
        if let Some(current_vehicle) = self.vehicle(entity) {
            return Err(LaneFlowAdapterError::EntityAlreadyBound {
                entity,
                current_vehicle,
                requested_vehicle: vehicle,
            });
        }

        self.by_vehicle.insert(vehicle, entity);
        self.by_entity.insert(entity, vehicle);
        Ok(())
    }

    fn unbind_vehicle(&mut self, vehicle: VehicleHandle) -> Option<Entity> {
        let entity = self.by_vehicle.remove(&vehicle)?;
        let reverse = self.by_entity.remove(&entity);
        debug_assert_eq!(reverse, Some(vehicle));
        Some(entity)
    }

    fn unbind_entity(&mut self, entity: Entity) -> Option<VehicleHandle> {
        let vehicle = self.by_entity.remove(&entity)?;
        let reverse = self.by_vehicle.remove(&vehicle);
        debug_assert_eq!(reverse, Some(entity));
        Some(vehicle)
    }

    fn rebind(
        &mut self,
        vehicle: VehicleHandle,
        requested_entity: Entity,
    ) -> Result<Entity, LaneFlowAdapterError> {
        let current_entity = self
            .entity(vehicle)
            .ok_or(LaneFlowAdapterError::VehicleNotBound { vehicle })?;
        if current_entity == requested_entity {
            return Err(LaneFlowAdapterError::VehicleAlreadyBound {
                vehicle,
                current_entity,
                requested_entity,
            });
        }
        if let Some(current_vehicle) = self.vehicle(requested_entity) {
            return Err(LaneFlowAdapterError::EntityAlreadyBound {
                entity: requested_entity,
                current_vehicle,
                requested_vehicle: vehicle,
            });
        }

        self.by_vehicle.insert(vehicle, requested_entity);
        self.by_entity.remove(&current_entity);
        self.by_entity.insert(requested_entity, vehicle);
        Ok(current_entity)
    }

    pub(crate) fn validate_replacement(
        &self,
        old: VehicleHandle,
    ) -> Result<Option<Entity>, LaneFlowAdapterError> {
        let Some(entity) = self.entity(old) else {
            return Ok(None);
        };
        let reverse_vehicle = self.vehicle(entity);
        if reverse_vehicle != Some(old) {
            return Err(LaneFlowAdapterError::VehicleEntityMappingInconsistent {
                vehicle: old,
                entity,
                reverse_vehicle,
            });
        }
        Ok(Some(entity))
    }

    pub(crate) fn rotate_replaced_vehicle(
        &mut self,
        old: VehicleHandle,
        new: VehicleHandle,
        entity: Option<Entity>,
    ) {
        let Some(entity) = entity else {
            return;
        };

        let removed_entity = self.by_vehicle.remove(&old);
        assert_eq!(removed_entity, Some(entity));
        let previous_new_entity = self.by_vehicle.insert(new, entity);
        assert_eq!(previous_new_entity, None);
        let previous_vehicle = self.by_entity.insert(entity, new);
        assert_eq!(previous_vehicle, Some(old));
    }
}

/// 最近一次 outer-frame presentation 尝试的稳定计数。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LaneFlowPresentationReport {
    pose_records: usize,
    mapped_records: usize,
    unbound_records: usize,
    applied_records: usize,
}

impl LaneFlowPresentationReport {
    /// 返回 Spatial 成功提交的 pose record 数。
    pub const fn pose_records(self) -> usize {
        self.pose_records
    }

    /// 返回 pose batch 中具有 Entity 绑定的 record 数。
    pub const fn mapped_records(self) -> usize {
        self.mapped_records
    }

    /// 返回按稳定顺序允许跳过的未绑定 record 数。
    pub const fn unbound_records(self) -> usize {
        self.unbound_records
    }

    /// 返回原子提交成功写入的 local Transform 数；失败时恒为零。
    pub const fn applied_records(self) -> usize {
        self.applied_records
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct StagedTransform {
    entity: Entity,
    transform: Transform,
}

pub(crate) fn sync_lane_flow_transforms(world: &mut World) {
    if !world.contains_resource::<LaneFlowSession>() {
        return;
    }

    world.resource_scope(
        |world, mut session: bevy_ecs::change_detection::Mut<LaneFlowSession>| {
            session.sync_presentation(world);
        },
    );
}

impl LaneFlowSession {
    /// 返回 Adapter-owned Vehicle/Entity 部分双射的只读视图。
    pub const fn vehicle_entities(&self) -> &LaneFlowVehicleEntityMap {
        &self.vehicle_entities
    }

    /// 绑定 live Core vehicle 与 presentation proxy Entity。
    pub fn bind_vehicle_entity(
        &mut self,
        vehicle: VehicleHandle,
        entity: Entity,
    ) -> Result<(), LaneFlowAdapterError> {
        if self.core.vehicle(vehicle).is_none() {
            return Err(LaneFlowAdapterError::UnknownVehicleForBinding { vehicle });
        }
        self.vehicle_entities.bind(vehicle, entity)
    }

    /// 按 vehicle 解除绑定；未绑定时返回 `None`。
    pub fn unbind_vehicle(&mut self, vehicle: VehicleHandle) -> Option<Entity> {
        self.vehicle_entities.unbind_vehicle(vehicle)
    }

    /// 按 Entity 解除绑定；未绑定时返回 `None`。
    pub fn unbind_entity(&mut self, entity: Entity) -> Option<VehicleHandle> {
        self.vehicle_entities.unbind_entity(entity)
    }

    /// 原子地把已绑定 vehicle 切换到未占用的新 proxy Entity。
    pub fn rebind_vehicle_entity(
        &mut self,
        vehicle: VehicleHandle,
        entity: Entity,
    ) -> Result<Entity, LaneFlowAdapterError> {
        if self.core.vehicle(vehicle).is_none() {
            return Err(LaneFlowAdapterError::UnknownVehicleForBinding { vehicle });
        }
        self.vehicle_entities.rebind(vehicle, entity)
    }

    /// 返回当前 frame placement。
    pub const fn frame_placement(&self) -> Option<LaneFlowFramePlacement> {
        self.frame_placement
    }

    /// 设置或 rebase 单活动 frame placement。
    ///
    /// 相同 placement 可幂等设置。root 变化时必须提供与当前值不同的 token。
    pub fn set_frame_placement(
        &mut self,
        placement: LaneFlowFramePlacement,
    ) -> Result<(), LaneFlowAdapterError> {
        if let Some(current) = self.frame_placement {
            if current == placement {
                return Ok(());
            }
            if current.token == placement.token {
                return Err(LaneFlowAdapterError::PlacementTokenReused {
                    current_root: current.root,
                    requested_root: placement.root,
                    token: placement.token,
                });
            }
        }

        self.frame_placement = Some(placement);
        Ok(())
    }

    /// 清除当前 frame placement，并返回旧值。
    pub fn clear_frame_placement(&mut self) -> Option<LaneFlowFramePlacement> {
        self.frame_placement.take()
    }

    /// 返回最近一次 presentation 尝试的计数摘要。
    pub const fn presentation_report(&self) -> LaneFlowPresentationReport {
        self.presentation_report
    }

    #[cfg(feature = "debug-gizmos")]
    pub(crate) fn validated_pose_batch(&self) -> Option<&CanonicalPoseBatchF32> {
        self.pose_batch_is_validated.then_some(&self.pose_batch)
    }

    /// 返回 committed pose batch 的当前容量。
    pub const fn pose_batch_capacity(&self) -> usize {
        self.pose_batch.capacity()
    }

    /// 返回稳定 pose input buffer 的当前容量。
    pub const fn pose_input_capacity(&self) -> usize {
        self.pose_inputs.capacity()
    }

    /// 返回 Transform staging buffer 的当前容量。
    pub const fn transform_staging_capacity(&self) -> usize {
        self.transform_staging.capacity()
    }

    fn sync_presentation(&mut self, world: &mut World) {
        self.presentation_report = LaneFlowPresentationReport::default();
        self.pose_batch_is_validated = false;
        if self.last_error.is_some() || self.vehicle_entities.is_empty() {
            return;
        }

        let result = self.try_sync_presentation(world);
        match result {
            Ok(()) => self.pose_batch_is_validated = true,
            Err(error) => {
                self.presentation_report.applied_records = 0;
                self.transform_staging.clear();
                self.last_error = Some(error);
            }
        }
    }

    fn try_sync_presentation(&mut self, world: &mut World) -> Result<(), LaneFlowAdapterError> {
        let placement = self
            .frame_placement
            .ok_or(LaneFlowAdapterError::MissingFramePlacement)?;
        self.extract_presentation_batch(placement)?;
        self.validate_and_apply_presentation(world, placement)
    }

    fn extract_presentation_batch(
        &mut self,
        placement: LaneFlowFramePlacement,
    ) -> Result<(), LaneFlowAdapterError> {
        self.rebuild_pose_inputs()?;
        self.spatial
            .extract_pose_batch(
                self.core.parking(),
                placement.token,
                &self.pose_inputs,
                &mut self.pose_batch,
                &mut self.pose_scratch,
            )
            .map_err(|source| LaneFlowAdapterError::SpatialBatch { source })?;
        self.presentation_report.pose_records = self.pose_batch.len();
        Ok(())
    }

    fn rebuild_pose_inputs(&mut self) -> Result<(), LaneFlowAdapterError> {
        self.pose_inputs.clear();
        let parking = self.core.parking_snapshot();

        for (input_index, vehicle) in self.core.vehicles().enumerate() {
            match vehicle.status {
                VehicleStatus::Active | VehicleStatus::Stopped => {
                    let route_edges = self.core.route_edges(vehicle.route).ok_or(
                        LaneFlowAdapterError::MissingVehicleRoute {
                            input_index,
                            vehicle: vehicle.handle,
                            route: vehicle.route,
                        },
                    )?;
                    let edge = route_edges.get(vehicle.route_edge_index).copied().ok_or(
                        LaneFlowAdapterError::MissingVehicleRouteEdge {
                            input_index,
                            vehicle: vehicle.handle,
                            route_edge_index: vehicle.route_edge_index,
                        },
                    )?;
                    self.pose_inputs.push(PoseInputRecord::lane(
                        vehicle.handle,
                        edge,
                        vehicle.edge_progress,
                    ));
                }
                VehicleStatus::Parked => {
                    if let Some(VehicleParkingState::Occupied { space }) =
                        parking.vehicle_state(vehicle.handle)
                    {
                        self.pose_inputs
                            .push(PoseInputRecord::parking(vehicle.handle, space));
                    } else {
                        return Err(LaneFlowAdapterError::MissingParkedVehicleBinding {
                            input_index,
                            vehicle: vehicle.handle,
                        });
                    }
                }
                VehicleStatus::Completed => {}
                status => {
                    return Err(LaneFlowAdapterError::UnsupportedVehicleStatus {
                        input_index,
                        vehicle: vehicle.handle,
                        status,
                    });
                }
            }
        }

        Ok(())
    }

    fn validate_and_apply_presentation(
        &mut self,
        world: &mut World,
        placement: LaneFlowFramePlacement,
    ) -> Result<(), LaneFlowAdapterError> {
        if self.pose_batch.frame_id() != self.spatial.frame_id() {
            return Err(LaneFlowAdapterError::PoseBatchFrameMismatch {
                expected_frame: self.spatial.frame_id().as_str().to_owned(),
                actual_frame: self.pose_batch.frame_id().as_str().to_owned(),
            });
        }
        if self.pose_batch.placement_token() != placement.token {
            return Err(LaneFlowAdapterError::PoseBatchTokenMismatch {
                expected_token: placement.token,
                actual_token: self.pose_batch.placement_token(),
            });
        }

        if world.get_entity(placement.root).is_err() {
            return Err(LaneFlowAdapterError::StaleFrameRoot {
                root: placement.root,
            });
        }
        let root_transform = world.get::<Transform>(placement.root).ok_or(
            LaneFlowAdapterError::FrameRootMissingTransform {
                root: placement.root,
            },
        )?;
        if !root_transform.is_finite() {
            return Err(LaneFlowAdapterError::NonFiniteFrameRootTransform {
                root: placement.root,
            });
        }
        if root_transform.scale != Transform::IDENTITY.scale {
            return Err(LaneFlowAdapterError::NonUnitFrameRootScale {
                root: placement.root,
                scale: [
                    root_transform.scale.x,
                    root_transform.scale.y,
                    root_transform.scale.z,
                ],
            });
        }

        self.transform_staging.clear();
        self.presentation_report.mapped_records = 0;
        self.presentation_report.unbound_records = 0;

        let records = self.pose_batch.records();
        let vehicle_entities = &self.vehicle_entities;
        let staging = &mut self.transform_staging;
        for (input_index, record) in records.iter().copied().enumerate() {
            let Some(entity) = vehicle_entities.entity(record.vehicle()) else {
                self.presentation_report.unbound_records += 1;
                continue;
            };
            self.presentation_report.mapped_records += 1;

            if world.get_entity(entity).is_err() {
                return Err(LaneFlowAdapterError::StaleMappedEntity {
                    input_index,
                    vehicle: record.vehicle(),
                    entity,
                });
            }
            let current = world.get::<Transform>(entity).ok_or(
                LaneFlowAdapterError::MappedEntityMissingTransform {
                    input_index,
                    vehicle: record.vehicle(),
                    entity,
                },
            )?;
            if !current.is_finite() {
                return Err(LaneFlowAdapterError::NonFiniteMappedTransform {
                    input_index,
                    vehicle: record.vehicle(),
                    entity,
                });
            }
            let actual_parent = world.get::<ChildOf>(entity).map(ChildOf::parent);
            if actual_parent != Some(placement.root) {
                return Err(LaneFlowAdapterError::MappedEntityWrongParent {
                    input_index,
                    vehicle: record.vehicle(),
                    entity,
                    expected_root: placement.root,
                    actual_parent,
                });
            }

            let pose = record.pose();
            let position = pose.position();
            let tangent = pose.tangent();
            let up = pose.up();
            let tangent_vector =
                Transform::from_xyz(tangent.x(), tangent.y(), tangent.z()).translation;
            let up_vector = Transform::from_xyz(up.x(), up.y(), up.z()).translation;
            let transform = Transform::from_xyz(position.x(), position.y(), position.z())
                .looking_to(tangent_vector, up_vector);
            if !transform.is_finite() {
                return Err(LaneFlowAdapterError::NonFiniteMappedTransform {
                    input_index,
                    vehicle: record.vehicle(),
                    entity,
                });
            }

            staging.push(StagedTransform { entity, transform });
        }

        for staged in staging.iter().copied() {
            let mut transform = world
                .get_mut::<Transform>(staged.entity)
                .expect("validated Entity and Transform remain live during exclusive commit");
            *transform = staged.transform;
        }
        self.presentation_report.applied_records = self.presentation_report.mapped_records;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use laneflow_core::{CoreWorld, LaneGraph};
    use laneflow_spatial::{CanonicalFrameId, SpatialRegistry};

    use super::*;
    use crate::LaneFlowSessionConfig;

    #[test]
    fn rebase_between_extraction_and_apply_rejects_old_token() {
        let graph = LaneGraph::empty();
        let spatial = SpatialRegistry::try_new(
            &graph,
            CanonicalFrameId::try_new("test:token").expect("valid frame"),
            [],
        )
        .expect("empty registry");
        let core = CoreWorld::new(10).expect("valid Core world");
        let config = LaneFlowSessionConfig::new(NonZeroU32::new(1).expect("non-zero"));
        let mut session = LaneFlowSession::new(core, spatial, config);
        let mut world = World::new();
        let root = world.spawn(Transform::IDENTITY).id();
        let old = LaneFlowFramePlacement::new(root, FramePlacementToken::new(1));
        let current = LaneFlowFramePlacement::new(root, FramePlacementToken::new(2));

        session
            .extract_presentation_batch(old)
            .expect("empty batch extraction succeeds");
        session.set_frame_placement(current).expect("valid rebase");

        assert!(matches!(
            session.validate_and_apply_presentation(&mut world, current),
            Err(LaneFlowAdapterError::PoseBatchTokenMismatch {
                expected_token,
                actual_token,
            }) if expected_token == current.token() && actual_token == old.token()
        ));
        assert_eq!(session.presentation_report().applied_records(), 0);
    }
}
