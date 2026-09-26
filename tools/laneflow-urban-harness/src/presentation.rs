//! 宿主选择、封闭位姿提取与持续实体绑定；不改变交通推进。

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
use sha2::{Digest, Sha256};

use crate::{Harness, IndividualId, Result, checked, invalid};

/// 按稳定宿主身份排序后选择循环窗口；比例分母为当前具有句柄的 live 个体。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SelectionWindow {
    pub percent: u8,
    pub offset: usize,
    pub stride: usize,
    pub reverse: bool,
}

/// 全量验证可指定相同应用集合供配对测量；缺省保留原全量验证行为。
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", deny_unknown_fields)]
pub enum PresentationMode {
    #[default]
    FullValidation,
    FullValidationSelected {
        selection: SelectionWindow,
    },
    SelectedPresentation {
        selection: SelectionWindow,
    },
}

impl PresentationMode {
    fn selection(self) -> Option<SelectionWindow> {
        match self {
            Self::FullValidation => None,
            Self::FullValidationSelected { selection }
            | Self::SelectedPresentation { selection } => Some(selection),
        }
    }

    pub(crate) fn validate(self) -> Result<()> {
        if self
            .selection()
            .is_some_and(|selection| selection.percent > 100)
        {
            return Err(invalid("selection percent must be in 0..=100"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresentationSample {
    pub tick: u64,
    pub individual: usize,
    pub active: usize,
    pub presentable: usize,
    pub requested: usize,
    pub host_selected: usize,
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
    pub selection_ns: u64,
    pub conversion_ns: u64,
    pub presentation_ns: u64,
    pub created: usize,
    pub reused: usize,
    pub hidden: usize,
    pub shown: usize,
    pub retired_bindings: usize,
    pub mode: PresentationMode,
    pub applied_digest: String,
    pub allocation: Option<AllocationSample>,
    pub storage: PresentationStorage,
}

/// 完整表现路径（选择至应用）的进程 allocator 增量，不包含随后验证与日志。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllocationSample {
    pub allocations: usize,
    pub reallocations: usize,
    pub bytes_allocated: usize,
    pub bytes_reallocated: isize,
}

/// 宿主持有容器的实际 len/capacity，单位为元素；Hash 表容量不等于分配字节。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresentationStorage {
    pub live_order: [usize; 2],
    pub requested: [usize; 2],
    pub requested_index: [usize; 2],
    pub candidates: [usize; 2],
    pub outputs: [usize; 2],
    pub bindings: [usize; 2],
    pub visible: [usize; 2],
    pub previous_visible_scratch: [usize; 2],
    pub extracted_index: [usize; 2],
}

#[derive(Default)]
pub struct Presentation {
    mode: PresentationMode,
    sample_index: usize,
    live_order: Vec<(IndividualId, VehicleHandle)>,
    requested: Vec<VehicleHandle>,
    requested_set: HashSet<VehicleHandle>,
    poses: LaneFlowCommittedPoseBatch,
    candidates: Vec<(IndividualId, VehicleHandle, Transform)>,
    outputs: Vec<(Entity, Transform)>,
    handles: HashSet<VehicleHandle>,
    previous: HashSet<Entity>,
    selected: HashSet<Entity>,
    bindings: HashMap<IndividualId, (VehicleHandle, Entity)>,
}

impl Presentation {
    /// 创建宿主表现模式，选择配置不进入交通计划或 Runtime 配置。
    ///
    /// # Errors
    /// 选择比例超过 100 时拒绝。
    pub fn new(mode: PresentationMode) -> Result<Self> {
        mode.validate()?;
        Ok(Self {
            mode,
            ..Self::default()
        })
    }

    pub fn sample(&mut self, harness: &mut Harness<'_>) -> Result<PresentationSample> {
        self.sample_with_placement(harness, FramePlacementToken::new(1))
    }

    /// 使用宿主当前放置令牌采样；应用前再次核验令牌与消费上下文。
    ///
    /// # Errors
    /// 提取、消费资格、绑定生命周期或交通/表现验证失败时返回错误。
    pub fn sample_with_placement(
        &mut self,
        harness: &mut Harness<'_>,
        token: FramePlacementToken,
    ) -> Result<PresentationSample> {
        #[cfg(feature = "allocation")]
        let allocation_region = stats_alloc::Region::new(crate::allocation::ALLOCATOR);
        let presentation_started = Instant::now();
        let started = Instant::now();
        self.requested.clear();
        self.requested_set.clear();
        if let Some(selection) = self.mode.selection() {
            self.live_order.clear();
            self.live_order.extend(
                harness
                    .individuals
                    .iter()
                    .filter_map(|v| v.handle.map(|h| (v.id, h))),
            );
            self.live_order.sort_unstable_by_key(|(id, _)| *id);
            let live = self.live_order.len();
            if live != 0 {
                let count = live * usize::from(selection.percent) / 100;
                let offset = ((selection.offset as u128
                    + self.sample_index as u128 * selection.stride as u128)
                    % live as u128) as usize;
                for index in 0..count {
                    self.requested
                        .push(self.live_order[(offset + index) % live].1);
                }
                if selection.reverse {
                    self.requested.reverse();
                }
                self.requested_set.extend(self.requested.iter().copied());
            }
        }
        let selection_ns = elapsed(started);
        let started = Instant::now();
        {
            let mut session = harness.adapter_world()?.resource_mut::<LaneFlowSession>();
            let result = if matches!(self.mode, PresentationMode::SelectedPresentation { .. }) {
                let context = session.consumption_context();
                session.extract_selected_committed_pose_batch(
                    context,
                    &self.requested,
                    token,
                    &mut self.poses,
                )
            } else {
                session.extract_committed_pose_batch(token, &mut self.poses)
            };
            checked("committed pose extraction", result)?;
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
        let conversion_ns = elapsed(started);
        let started = Instant::now();
        // A parked or temporarily deselected proxy retains its caller identity.
        // Successful replacement/despawn is represented by a new/absent identity.
        let bindings_before = self.bindings.len();
        self.bindings
            .retain(|id, (handle, _)| harness.stable_individual(*handle) == Some(*id));
        let retired_bindings = bindings_before - self.bindings.len();
        for (id, (handle, entity)) in &self.bindings {
            if harness.stable_individual(*handle) != Some(*id)
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
        let divisor =
            if self.mode == PresentationMode::FullValidation && harness.plan.scale == "100k" {
                10
            } else {
                1
            };
        let apply_limit = self.candidates.len() / divisor;
        let world = harness.adapter_world()?;
        validate_consumption(world.resource::<LaneFlowSession>(), &self.poses, token)?;
        self.outputs.clear();
        self.selected.clear();
        let mut created = 0;
        let mut reused = 0;
        let mut shown = 0;
        for (id, handle, transform) in self
            .candidates
            .iter()
            .filter(|(_, handle, _)| {
                self.mode.selection().is_none() || self.requested_set.contains(handle)
            })
            .take(apply_limit)
        {
            let bound = world.resource::<LaneFlowSession>().vehicle_entity(*handle);
            let entity = if let Some(entity) = bound {
                if world.get_entity(entity).is_err() {
                    return Err(invalid("stale selected proxy entity"));
                }
                reused += 1;
                entity
            } else {
                created += 1;
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
                shown += 1;
                world.entity_mut(entity).insert(Transform::IDENTITY);
            }
            self.outputs.push((entity, *transform));
        }
        let mut hidden = 0;
        for entity in self.previous.difference(&self.selected) {
            if let Ok(mut entity) = world.get_entity_mut(*entity) {
                hidden += usize::from(entity.contains::<Transform>());
                entity.remove::<Transform>();
            }
        }
        let selection_mapping_ns = elapsed(started);
        let started = Instant::now();
        validate_consumption(world.resource::<LaneFlowSession>(), &self.poses, token)?;
        for (entity, transform) in &self.outputs {
            *world
                .get_mut::<Transform>(*entity)
                .ok_or_else(|| invalid("selected Transform absent"))? = *transform;
        }
        let apply_ns = elapsed(started);
        let presentation_ns = elapsed(presentation_started);
        #[cfg(feature = "allocation")]
        let allocation = {
            let stats = allocation_region.change();
            Some(AllocationSample {
                allocations: stats.allocations,
                reallocations: stats.reallocations,
                bytes_allocated: stats.bytes_allocated,
                bytes_reallocated: stats.bytes_reallocated,
            })
        };
        #[cfg(not(feature = "allocation"))]
        let allocation = None;
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
        let mut applied_digest = Sha256::new();
        applied_digest.update(b"laneflow-applied-transforms-v1");
        for (id, handle, transform) in &self.candidates {
            if !self
                .bindings
                .get(id)
                .is_some_and(|(bound, entity)| bound == handle && self.previous.contains(entity))
            {
                continue;
            }
            for value in [id.tile, id.slot, id.incarnation] {
                applied_digest.update(value.to_le_bytes());
            }
            for value in transform
                .translation
                .to_array()
                .into_iter()
                .chain(transform.rotation.to_array())
                .chain(transform.scale.to_array())
            {
                applied_digest.update(value.to_bits().to_le_bytes());
            }
        }
        let applied_digest = crate::hex(&applied_digest.finalize());
        self.handles.clear();
        for (_, handle, _) in &self.candidates {
            if !self.handles.insert(*handle) {
                return Err(invalid("duplicate extracted individual"));
            }
        }
        let mut presentable = 0;
        let mut active = 0;
        let mut expected_extracted = 0;
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
            active += usize::from(state.status() == VehicleStatus::Active);
            let expected_in_batch = expected
                && (!matches!(self.mode, PresentationMode::SelectedPresentation { .. })
                    || self.requested_set.contains(handle));
            if self.handles.contains(handle) != expected_in_batch {
                return Err(invalid(
                    "pose membership differs from committed lifecycle/binding",
                ));
            }
            presentable += usize::from(expected);
            expected_extracted += usize::from(expected_in_batch);
        }
        if expected_extracted != self.candidates.len()
            || expected_extracted != self.poses.batch().records().len()
        {
            return Err(invalid("pose/identity batch cardinalities differ"));
        }
        self.sample_index = self
            .sample_index
            .checked_add(1)
            .ok_or_else(|| invalid("presentation sample index exhausted"))?;
        Ok(PresentationSample {
            tick: harness.world().tick_index(),
            individual: harness.world().live_vehicles().len(),
            active,
            presentable,
            requested: if matches!(self.mode, PresentationMode::SelectedPresentation { .. }) {
                self.requested.len()
            } else {
                harness.world().live_vehicles().len()
            },
            host_selected: if self.mode.selection().is_some() {
                self.requested.len()
            } else {
                self.outputs.len()
            },
            extracted: self.candidates.len(),
            applied: self.outputs.len(),
            n_presented: self.candidates.len(),
            application_divisor: divisor,
            waiting_members,
            latest_waiting_decisions,
            latest_conflict_decisions,
            pose_ns,
            selection_mapping_ns,
            apply_ns,
            validation_ns: elapsed(started),
            selection_ns,
            conversion_ns,
            presentation_ns,
            created,
            reused,
            hidden,
            shown,
            retired_bindings,
            mode: self.mode,
            applied_digest,
            allocation,
            storage: PresentationStorage {
                live_order: [self.live_order.len(), self.live_order.capacity()],
                requested: [self.requested.len(), self.requested.capacity()],
                requested_index: [self.requested_set.len(), self.requested_set.capacity()],
                candidates: [self.candidates.len(), self.candidates.capacity()],
                outputs: [self.outputs.len(), self.outputs.capacity()],
                bindings: [self.bindings.len(), self.bindings.capacity()],
                visible: [self.previous.len(), self.previous.capacity()],
                previous_visible_scratch: [self.selected.len(), self.selected.capacity()],
                extracted_index: [self.handles.len(), self.handles.capacity()],
            },
        })
    }

    /// Final applied positions for a labelled, data-derived preview (not a renderer benchmark).
    pub(crate) fn preview(&self) -> Vec<serde_json::Value> {
        self.candidates.iter().filter(|(id, handle, _)| self.bindings.get(id).is_some_and(|(bound, entity)| bound == handle && self.previous.contains(entity))).map(|(id, _, transform)| {
            serde_json::json!({"individual":id,"position_m":transform.translation.to_array()})
        }).collect()
    }
}

fn validate_consumption(
    session: &LaneFlowSession,
    poses: &LaneFlowCommittedPoseBatch,
    token: FramePlacementToken,
) -> Result<()> {
    if !session.consumption_context_is_current(poses.context()) {
        return Err(invalid("stale committed pose context"));
    }
    if poses.batch().placement_token() != token {
        return Err(invalid("stale frame placement token"));
    }
    Ok(())
}

fn elapsed(started: Instant) -> u64 {
    started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Artifacts, ResolvedPlan, Window};
    use laneflow_spatial::SpatialSession;
    use laneflow_urban_generator::{Scale, UrbanConfig, generate};

    #[test]
    fn selection_exit_reentry_and_placement_rejection_preserve_identity() {
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("source");
        let config =
            UrbanConfig::parse(include_str!("../../../examples/config/cn-urban.toml")).unwrap();
        generate(&config, Scale::Fixture, &source, None).unwrap();
        let artifacts = Artifacts::load_spatial(&source).unwrap();
        let plan = ResolvedPlan::mixed(&artifacts, Window::probe(1).unwrap()).unwrap();
        let mut harness = Harness::install(
            &artifacts,
            &plan,
            laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        )
        .unwrap()
        .into_adapter(
            SpatialSession::bind(artifacts.revision().clone())
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        let selection = SelectionWindow {
            percent: 100,
            offset: 0,
            stride: 0,
            reverse: false,
        };
        let mut presentation =
            Presentation::new(PresentationMode::SelectedPresentation { selection }).unwrap();
        let before = harness.checkpoint().unwrap();
        let first = presentation
            .sample_with_placement(&mut harness, FramePlacementToken::new(41))
            .unwrap();
        assert!(first.applied > 0);
        let bindings = presentation.bindings.clone();
        let visible = presentation.previous.clone();
        assert!(
            validate_consumption(
                harness
                    .adapter_world()
                    .unwrap()
                    .resource::<LaneFlowSession>(),
                &presentation.poses,
                FramePlacementToken::new(42)
            )
            .is_err()
        );
        for entity in &visible {
            assert!(
                harness
                    .adapter_world()
                    .unwrap()
                    .get::<Transform>(*entity)
                    .is_some()
            );
        }
        presentation.mode = PresentationMode::SelectedPresentation {
            selection: SelectionWindow {
                percent: 0,
                ..selection
            },
        };
        let empty = presentation
            .sample_with_placement(&mut harness, FramePlacementToken::new(42))
            .unwrap();
        assert_eq!(empty.requested, 0);
        assert_eq!(empty.extracted, 0);
        assert_eq!(empty.applied, 0);
        assert_eq!(empty.hidden, first.applied);
        assert_eq!(presentation.bindings, bindings);
        for entity in &visible {
            assert!(
                harness
                    .adapter_world()
                    .unwrap()
                    .get::<Transform>(*entity)
                    .is_none()
            );
        }
        presentation.mode = PresentationMode::SelectedPresentation { selection };
        let restored = presentation
            .sample_with_placement(&mut harness, FramePlacementToken::new(43))
            .unwrap();
        assert_eq!(restored.created, 0);
        assert_eq!(restored.reused, first.applied);
        assert_eq!(restored.shown, first.applied);
        assert_eq!(presentation.bindings, bindings);
        assert_eq!(presentation.previous, visible);
        assert_eq!(harness.checkpoint().unwrap(), before);

        // 换成合法 headless 根后故意提取失败；零选择尚未提交，不能隐藏旧实体。
        let old_context = presentation.poses.context();
        let old_vehicles = presentation.poses.vehicles().to_vec();
        let old_records = presentation.poses.batch().records().to_vec();
        let old_transforms: Vec<_> = visible
            .iter()
            .map(|&entity| {
                (
                    entity,
                    *harness
                        .adapter_world()
                        .unwrap()
                        .get::<Transform>(entity)
                        .unwrap(),
                )
            })
            .collect();
        let committed_source = harness.world().committed_source().clone();
        let reloaded = Artifacts::load_spatial(&source).unwrap();
        harness
            .adapter_world()
            .unwrap()
            .resource_mut::<LaneFlowSession>()
            .same_revision_restore(
                reloaded.revision().clone(),
                committed_source.clone(),
                laneflow_bevy::LaneFlowTargetSpatial::Headless,
                &laneflow_runtime::CutoverPreflightLimits::new(2_147_483_648),
            )
            .unwrap();
        presentation.mode = PresentationMode::SelectedPresentation {
            selection: SelectionWindow {
                percent: 0,
                ..selection
            },
        };
        assert!(
            presentation
                .sample_with_placement(&mut harness, FramePlacementToken::new(44))
                .is_err()
        );
        assert_eq!(presentation.bindings, bindings);
        assert_eq!(presentation.previous, visible);
        assert_eq!(presentation.poses.context(), old_context);
        assert_eq!(presentation.poses.vehicles(), old_vehicles);
        assert_eq!(presentation.poses.batch().records(), old_records);
        assert_eq!(
            presentation.poses.batch().placement_token(),
            FramePlacementToken::new(43)
        );
        for (entity, transform) in &old_transforms {
            assert_eq!(
                harness.adapter_world().unwrap().get::<Transform>(*entity),
                Some(transform)
            );
        }
        let rebound = Artifacts::load_spatial(&source).unwrap();
        harness
            .adapter_world()
            .unwrap()
            .resource_mut::<LaneFlowSession>()
            .same_revision_restore(
                rebound.revision().clone(),
                committed_source,
                laneflow_bevy::LaneFlowTargetSpatial::Rebind(
                    SpatialSession::bind(rebound.revision().clone())
                        .unwrap()
                        .unwrap(),
                ),
                &laneflow_runtime::CutoverPreflightLimits::new(2_147_483_648),
            )
            .unwrap();
        presentation.mode = PresentationMode::SelectedPresentation { selection };
        let retry = presentation
            .sample_with_placement(&mut harness, FramePlacementToken::new(45))
            .unwrap();
        assert_eq!(retry.created, 0);
        assert_eq!(retry.reused, first.applied);
        assert_eq!(presentation.bindings, bindings);
        assert_eq!(presentation.previous, visible);
    }
}
