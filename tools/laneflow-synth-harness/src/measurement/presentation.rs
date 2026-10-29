use bevy_ecs::{entity::Entity, resource::Resource, world::World};
use bevy_math::Vec3;
use bevy_transform::components::Transform;
use laneflow_bevy::{LaneFlowCommittedPoseBatch, LaneFlowSession};
use laneflow_runtime::VehicleHandle;
use laneflow_spatial::FramePlacementToken;
use serde_json::{Value, json};

use crate::{Error, failure, require};

use super::driver::Driver;

#[derive(Resource)]
pub(super) struct Presentation {
    pub selected: Vec<usize>,
    handles: Vec<VehicleHandle>,
    poses: LaneFlowCommittedPoseBatch,
    staged: Vec<(Entity, Transform)>,
    pub high_water: usize,
}

impl Presentation {
    pub fn new(selected: Vec<usize>) -> Self {
        Self {
            handles: Vec::with_capacity(selected.len()),
            staged: Vec::with_capacity(selected.len()),
            selected,
            poses: LaneFlowCommittedPoseBatch::new(),
            high_water: 0,
        }
    }

    pub fn apply(&mut self, ecs: &mut World, token: FramePlacementToken) -> Result<usize, Error> {
        self.handles.clear();
        let driver = ecs.resource::<Driver>();
        self.handles
            .extend(self.selected.iter().map(|&rank| driver.handles[rank]));
        let mut session = ecs.resource_mut::<LaneFlowSession>();
        let context = session.consumption_context();
        session
            .extract_selected_committed_pose_batch(context, &self.handles, token, &mut self.poses)
            .map_err(|e| failure("selected pose extraction", e))?;
        self.stage(ecs, token)?;
        // 独占 ECS 借用期间，预检后的 Entity/Transform 不会变化。
        for &(entity, transform) in &self.staged {
            *ecs.get_mut::<Transform>(entity)
                .expect("prevalidated Transform") = transform;
        }
        self.high_water = self.high_water.max(self.staged.len());
        Ok(self.staged.len())
    }

    fn stage(&mut self, ecs: &World, token: FramePlacementToken) -> Result<(), Error> {
        self.staged.clear();
        let session = ecs.resource::<LaneFlowSession>();
        let driver = ecs.resource::<Driver>();
        require(
            session.consumption_context_is_current(self.poses.context())
                && self.poses.batch().placement_token() == token
                && self.poses.batch().network_revision()
                    == Some(session.world().revision().network_revision())
                && self.poses.vehicles().len() == self.poses.batch().records().len(),
            || "表现批次上下文不符".into(),
        )?;
        for (index, (&handle, record)) in self
            .poses
            .vehicles()
            .iter()
            .zip(self.poses.batch().records())
            .enumerate()
        {
            require(record.record().raw() == index as u32, || {
                "pose record 顺序不符".into()
            })?;
            let rank = *driver
                .ranks
                .get(&handle)
                .ok_or_else(|| failure("pose handle", handle))?;
            let entity = session
                .vehicle_entity(handle)
                .ok_or_else(|| failure("pose binding", rank))?;
            require(
                driver.entities[rank] == Some(entity) && ecs.get::<Transform>(entity).is_some(),
                || "Entity/Transform 映射失效".into(),
            )?;
            let pose = record.pose();
            let p = pose.position();
            let t = pose.tangent();
            let u = pose.up();
            let transform = Transform::from_xyz(p.x(), p.y(), p.z()).looking_to(
                Vec3::new(t.x(), t.y(), t.z()),
                Vec3::new(u.x(), u.y(), u.z()),
            );
            require(
                transform.translation.is_finite()
                    && transform.rotation.is_finite()
                    && transform.scale.is_finite(),
                || "非有限 Transform".into(),
            )?;
            self.staged.push((entity, transform));
        }
        Ok(())
    }

    pub fn resources(&self) -> Value {
        json!({"selected_rank_capacity":self.selected.capacity(),"selected_handle_capacity":self.handles.capacity(),
            "staged_transform_capacity":self.staged.capacity(),"presented_high_water":self.high_water,
            "host_selection_bytes":self.selected.capacity()*std::mem::size_of::<usize>()+self.handles.capacity()*std::mem::size_of::<VehicleHandle>(),
            "host_staging_bytes":self.staged.capacity()*std::mem::size_of::<(Entity,Transform)>(),
            "scope":"owned_host_buffers_only_session_and_spatial_private_capacity_unexposed"})
    }
}
