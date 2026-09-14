//! Full committed extraction, stable selection, and real Bevy Transform application.

use std::{
    collections::{HashMap, HashSet},
    time::Instant,
};

use bevy_ecs::entity::Entity;
use bevy_math::Vec3;
use bevy_transform::components::Transform;
use laneflow_bevy::{LaneFlowCommittedPoseBatch, LaneFlowSession};
use laneflow_runtime::{ParkingBinding, ParkingTarget, VehicleHandle, VehicleStatus};
use laneflow_spatial::FramePlacementToken;
use serde::{Deserialize, Serialize};

use crate::{Harness, IndividualId, Result, checked, invalid};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresentationSample {
    pub tick: u64,
    pub individual: usize,
    pub presentable: usize,
    pub extracted: usize,
    pub applied: usize,
    pub n_presented: usize,
    pub application_divisor: usize,
    pub waiting_members: usize,
    pub latest_waiting_decisions: usize,
    pub latest_conflict_decisions: usize,
    pub pose_ns: u64,
    pub selection_mapping_ns: u64,
    pub apply_ns: u64,
    pub validation_ns: u64,
}

#[derive(Default)]
pub struct Presentation {
    poses: LaneFlowCommittedPoseBatch,
    candidates: Vec<(IndividualId, VehicleHandle, Transform)>,
    outputs: Vec<(Entity, Transform)>,
    handles: HashSet<VehicleHandle>,
    previous: HashSet<Entity>,
    selected: HashSet<Entity>,
    bindings: HashMap<IndividualId, (VehicleHandle, Entity)>,
}

impl Presentation {
    pub fn sample(&mut self, harness: &mut Harness<'_>) -> Result<PresentationSample> {
        let started = Instant::now();
        {
            let mut session = harness.adapter_world()?.resource_mut::<LaneFlowSession>();
            checked(
                "committed pose extraction",
                session.extract_committed_pose_batch(FramePlacementToken::new(1), &mut self.poses),
            )?;
        }
        let pose_ns = elapsed(started);
        let started = Instant::now();
        self.candidates.clear();
        for (&handle, record) in self
            .poses
            .vehicles()
            .iter()
            .zip(self.poses.batch().records())
        {
            let id = harness
                .stable_individual(handle)
                .ok_or_else(|| invalid("pose has no stable individual"))?;
            let pose = record.pose();
            let position = pose.position();
            let forward = pose.tangent();
            let up = pose.up();
            let transform = Transform::from_xyz(position.x(), position.y(), position.z())
                .looking_to(
                    Vec3::new(forward.x(), forward.y(), forward.z()),
                    Vec3::new(up.x(), up.y(), up.z()),
                );
            self.candidates.push((id, handle, transform));
        }
        self.candidates.sort_unstable_by_key(|(id, _, _)| *id);
        // A parked or temporarily deselected proxy retains its caller identity.
        // Successful replacement/despawn is represented by a new/absent identity.
        let live: HashMap<_, _> = harness
            .individuals
            .iter()
            .filter_map(|v| v.handle.map(|h| (v.id, h)))
            .collect();
        self.bindings.retain(|id, _| live.contains_key(id));
        for (id, (handle, entity)) in &self.bindings {
            if live.get(id) != Some(handle)
                || harness
                    .adapter_world()?
                    .resource::<LaneFlowSession>()
                    .vehicle_entity(*handle)
                    != Some(*entity)
            {
                return Err(invalid(
                    "persistent individual lost or changed its proxy binding",
                ));
            }
        }
        let divisor = if harness.plan.scale == "100k" { 10 } else { 1 };
        let applied = self.candidates.len() / divisor;
        let world = harness.adapter_world()?;
        if !world
            .resource::<LaneFlowSession>()
            .consumption_context_is_current(self.poses.context())
        {
            return Err(invalid("stale committed pose context"));
        }
        self.outputs.clear();
        self.selected.clear();
        for (id, handle, transform) in self.candidates.iter().take(applied) {
            let bound = world.resource::<LaneFlowSession>().vehicle_entity(*handle);
            let entity = if let Some(entity) = bound {
                if world.get_entity(entity).is_err() {
                    return Err(invalid("stale selected proxy entity"));
                }
                entity
            } else {
                let entity = world.spawn_empty().id();
                checked(
                    "bind selected proxy",
                    world
                        .resource_mut::<LaneFlowSession>()
                        .bind_vehicle_entity(*handle, entity),
                )?;
                entity
            };
            if !self.selected.insert(entity) {
                return Err(invalid("selected individuals share an entity"));
            }
            self.bindings.insert(*id, (*handle, entity));
            // Membership changes belong to selection/mapping, not Transform writes.
            if world.get::<Transform>(entity).is_none() {
                world.entity_mut(entity).insert(Transform::IDENTITY);
            }
            self.outputs.push((entity, *transform));
        }
        for entity in self.previous.difference(&self.selected) {
            if let Ok(mut entity) = world.get_entity_mut(*entity) {
                entity.remove::<Transform>();
            }
        }
        let selection_mapping_ns = elapsed(started);
        let started = Instant::now();
        for (entity, transform) in &self.outputs {
            *world
                .get_mut::<Transform>(*entity)
                .ok_or_else(|| invalid("selected Transform absent"))? = *transform;
        }
        let apply_ns = elapsed(started);
        let started = Instant::now();
        let (waiting_members, latest_waiting_decisions, latest_conflict_decisions) = {
            let session = world.resource::<LaneFlowSession>();
            let observation = session.junction_observation();
            if observation.context().tick_index() != session.world().tick_index() {
                return Err(invalid("junction observation is not current"));
            }
            (
                observation.waiting_zone_members().len(),
                observation.latest_waiting_decisions().len(),
                observation.latest_conflict_decisions().len(),
            )
        };
        for (entity, transform) in &self.outputs {
            if world.get::<Transform>(*entity) != Some(transform)
                || !transform.translation.is_finite()
                || !transform.rotation.is_finite()
            {
                return Err(invalid("invalid applied Transform"));
            }
        }
        for entity in self.previous.difference(&self.selected) {
            if world.get::<Transform>(*entity).is_some() {
                return Err(invalid("non-selected individual still has a Transform"));
            }
        }
        std::mem::swap(&mut self.previous, &mut self.selected);
        self.handles.clear();
        for (_, handle, _) in &self.candidates {
            if !self.handles.insert(*handle) {
                return Err(invalid("duplicate extracted individual"));
            }
        }
        let mut presentable = 0;
        for handle in harness.world().live_vehicles() {
            let state = harness
                .world()
                .vehicle(*handle)
                .ok_or_else(|| invalid("missing live vehicle"))?;
            let expected = match state.status() {
                VehicleStatus::Active => true,
                VehicleStatus::Completed => false,
                VehicleStatus::Parked => matches!(
                    harness.world().parking_binding(*handle),
                    Some(ParkingBinding::Occupied(ParkingTarget::ExplicitSpace(_)))
                ),
            };
            if self.handles.contains(handle) != expected {
                return Err(invalid(
                    "pose membership differs from committed lifecycle/binding",
                ));
            }
            presentable += usize::from(expected);
        }
        if presentable != self.candidates.len() || presentable != self.poses.batch().records().len()
        {
            return Err(invalid("pose/identity batch cardinalities differ"));
        }
        Ok(PresentationSample {
            tick: harness.world().tick_index(),
            individual: harness.world().live_vehicles().len(),
            presentable,
            extracted: self.candidates.len(),
            applied,
            n_presented: self.candidates.len(),
            application_divisor: divisor,
            waiting_members,
            latest_waiting_decisions,
            latest_conflict_decisions,
            pose_ns,
            selection_mapping_ns,
            apply_ns,
            validation_ns: elapsed(started),
        })
    }

    /// Final applied positions for a labelled, data-derived preview (not a renderer benchmark).
    pub(crate) fn preview(&self) -> Vec<serde_json::Value> {
        self.candidates.iter().take(self.outputs.len()).map(|(id, _, transform)| {
            serde_json::json!({"individual":id,"position_m":transform.translation.to_array()})
        }).collect()
    }
}

fn elapsed(started: Instant) -> u64 {
    started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
}
