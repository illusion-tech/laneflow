//! 切换状态机、整批提交与同帧摘要。GUI/headless 安装同一份系统和资源。
use std::{collections::VecDeque, fmt::Debug, sync::Arc, time::Duration};

use bevy_app::{App, AppExit, PostUpdate};
use bevy_ecs::{
    component::Component,
    entity::Entity,
    hierarchy::ChildOf,
    message::Messages,
    resource::Resource,
    schedule::{IntoScheduleConfigs, SystemSet},
    world::{Mut, World},
};
use bevy_math::Vec3;
use bevy_transform::{TransformSystems, components::Transform};
use laneflow_bevy::{
    LaneFlowAdapterError, LaneFlowCommittedPoseBatch, LaneFlowConsumptionContext,
    LaneFlowCutoverRecord, LaneFlowFixed, LaneFlowFixedSet, LaneFlowOuterFrame,
    LaneFlowOuterFrameSet, LaneFlowPlugin, LaneFlowSession, LaneFlowTargetSpatial, despawn_vehicle,
};
use laneflow_runtime::{
    CutoverError, CutoverPreflightLimits, CutoverTransactionLimits, ParkingBinding, PoseSource,
    RouteHandle, RouteRegisterInput, VehicleHandle, VehicleSpawnInput, VehicleStatus,
};
use laneflow_spatial::{CanonicalPoseF32, FramePlacementToken, SpatialSession};
use laneflow_static_contract::NetworkRevisionId;

use super::cutover_scene::{self as scene, RevisionName, SPUR_START_MM, Scene, Target};

/// 支路车的代理实体在 `Fleet.entities` 内的下标。
pub(super) const SPUR_SLOT: usize = 4;
const PREFLIGHT: CutoverPreflightLimits = CutoverPreflightLimits::new(16 * 1_024 * 1_024);

/// `Fleet.entities` 内的稳定下标，不是 Runtime 或 Spatial 数组下标。
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct VehicleSlot(pub usize);

#[derive(Component)]
pub(super) struct VehicleModel;

#[derive(Component)]
pub(super) struct Placement(pub FramePlacementToken);

#[derive(Resource)]
pub(super) struct Fleet {
    pub root: Entity,
    pub token: FramePlacementToken,
    pub entities: Vec<Entity>,
    /// 逐槽车辆句柄；支路代理在没有支路车时为 `None`。
    pub vehicles: Vec<Option<VehicleHandle>>,
    pub spur_route: Option<RouteHandle>,
    pub vehicle_length: f32,
}

/// 路面表现数据。`version` 随每次成功切换递增，GUI 据此替换网格。
#[derive(Resource)]
pub(super) struct RoadData {
    pub revision: NetworkRevisionId,
    pub lanes: Vec<Vec<Vec3>>,
    pub version: u64,
    pub lane_width: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CutoverState {
    Idle,
    AwaitingClear,
    AwaitingRetry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RefusalReason {
    UnmappableLaneEdge,
    VehicleRevalidation,
}

impl RefusalReason {
    const fn label(self) -> &'static str {
        match self {
            Self::UnmappableLaneEdge => "unmappable lane edge",
            Self::VehicleRevalidation => "vehicle past target edge",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Refusal {
    pub reason: RefusalReason,
    pub tick: u64,
}

#[derive(Resource)]
pub(super) struct Cutover {
    r1: Target,
    r2: Target,
    pub current: RevisionName,
    pub pending: Option<RevisionName>,
    pub state: CutoverState,
    plan: VecDeque<u64>,
    pub successes: u32,
    pub events: u64,
    pub retired: u32,
    pub stale_rejected: u32,
    pub last_refusal: Option<Refusal>,
}

impl Cutover {
    pub(super) const fn target(&self, name: RevisionName) -> &Target {
        match name {
            RevisionName::R1 => &self.r1,
            RevisionName::R2 => &self.r2,
        }
    }

    /// 请求切到另一个修订；已有请求在途时忽略。
    pub(super) fn request(&mut self) -> bool {
        if self.pending.is_some() {
            return false;
        }
        self.pending = Some(self.current.other());
        true
    }
}

#[derive(Debug)]
pub(super) struct Failure {
    pub tick: u64,
    pub detail: String,
}

#[derive(Resource, Default)]
pub(super) struct Health {
    pub failure: Option<Failure>,
    exit_sent: bool,
}

/// 逐槽可见状态是表现的正本；GUI 把它映射到 Bevy `Visibility`。
#[derive(Resource)]
pub(super) struct Presentation {
    pub visible: Vec<bool>,
    pub poses: LaneFlowCommittedPoseBatch,
    next_visible: Vec<bool>,
    in_batch: Vec<bool>,
    staged: Vec<(Entity, Transform)>,
    frame: u64,
}

impl Presentation {
    fn new(slots: usize) -> Self {
        Self {
            visible: vec![false; slots],
            poses: LaneFlowCommittedPoseBatch::new(),
            next_visible: vec![false; slots],
            in_batch: vec![false; slots],
            staged: Vec::with_capacity(slots),
            frame: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct FrameSummary {
    pub frame: u64,
    pub tick: u64,
    pub steps: u32,
    pub backlog: Duration,
    pub revision: RevisionName,
    pub revision_id: NetworkRevisionId,
    pub generation: u64,
    pub state: CutoverState,
    pub target: Option<RevisionName>,
    pub successes: u32,
    pub events: u64,
    pub stale_rejected: u32,
    pub refusal: Option<Refusal>,
    pub ring_visible: usize,
    pub spur: &'static str,
}

fn state_line(state: CutoverState, target: Option<RevisionName>) -> String {
    let Some(target) = target else {
        return "cutover: idle\n".into();
    };
    let state = match state {
        CutoverState::Idle => "requested",
        CutoverState::AwaitingClear => "waiting for clear",
        CutoverState::AwaitingRetry => "waiting for retry",
    };
    format!("cutover: {state} -> {}\n", target.label())
}

fn refusal_line(refusal: Option<Refusal>) -> String {
    let Some(refusal) = refusal else {
        return "cutover refused: none\n".into();
    };
    format!(
        "cutover refused: {} @ tick {}\n",
        refusal.reason.label(),
        refusal.tick
    )
}

impl FrameSummary {
    pub fn hud(&self) -> String {
        let prefix: String = self.revision_id.as_digest().as_bytes()[..4]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let mut text = String::from("LaneFlow | revision cutover\n");
        text.push_str(&format!(
            "revision {}  {prefix}  generation {}\n",
            self.revision.label(),
            self.generation
        ));
        text.push_str(&state_line(self.state, self.target));
        text.push_str(&format!(
            "cutovers {}  events {}  stale batches rejected {}\n",
            self.successes, self.events, self.stale_rejected
        ));
        text.push_str(&refusal_line(self.refusal));
        text.push_str(&format!(
            "ring (white) {} visible\nspur (orange) {}\n",
            self.ring_visible, self.spur
        ));
        text.push_str(&format!(
            "tick {}   steps {}   backlog {} ms\nC: cutover\nWheel: zoom | LMB: pan | RMB: orbit",
            self.tick,
            self.steps,
            self.backlog.as_millis()
        ));
        text
    }
}

#[derive(Resource, Default)]
pub(super) struct Summary {
    pub current: Option<FrameSummary>,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, SystemSet)]
pub(super) struct Present;

fn healthy(health: bevy_ecs::system::Res<Health>) -> bool {
    health.failure.is_none()
}

pub(super) fn install(app: &mut App, scene: Scene, plan: Vec<u64>) -> Result<(), String> {
    if app.world().contains_resource::<LaneFlowSession>() {
        return Err("宿主已安装，禁止复用另一世界的 Session".into());
    }
    let Scene {
        mut session,
        r1,
        r2,
        ring,
        vehicle_length,
        lane_width,
        ..
    } = scene;
    let token = FramePlacementToken::new(1);
    let root = app
        .world_mut()
        .spawn((Transform::IDENTITY, Placement(token)))
        .id();
    let mut entities = Vec::with_capacity(ring.len() + 1);
    for slot in 0..=ring.len() {
        let entity = app
            .world_mut()
            .spawn((VehicleSlot(slot), Transform::IDENTITY, ChildOf(root)))
            .id();
        app.world_mut().spawn((
            VehicleModel,
            Transform::from_xyz(0.0, 0.65, vehicle_length * 0.5),
            ChildOf(entity),
        ));
        entities.push(entity);
    }
    if ring.len() != SPUR_SLOT {
        return Err("环路车数量与支路代理下标不符".into());
    }
    let mut vehicles = Vec::with_capacity(entities.len());
    for (&vehicle, &entity) in ring.iter().zip(&entities) {
        session
            .bind_vehicle_entity(vehicle, entity)
            .map_err(|error| format!("{error:?}"))?;
        vehicles.push(Some(vehicle));
    }
    vehicles.push(None);
    let slots = entities.len();
    let road = RoadData {
        revision: r1.root.canonical_origin().network_revision(),
        lanes: scene::centerlines(&r1.root)?,
        version: 0,
        lane_width,
    };
    app.add_plugins(LaneFlowPlugin)
        .insert_resource(session)
        .insert_resource(Cutover {
            r1,
            r2,
            current: RevisionName::R1,
            pending: None,
            state: CutoverState::Idle,
            plan: plan.into(),
            successes: 0,
            events: 0,
            retired: 0,
            stale_rejected: 0,
            last_refusal: None,
        })
        .insert_resource(Fleet {
            root,
            token,
            entities,
            vehicles,
            spur_route: None,
            vehicle_length,
        })
        .insert_resource(road)
        .insert_resource(Presentation::new(slots))
        .init_resource::<Health>()
        .init_resource::<Summary>()
        .configure_sets(
            LaneFlowFixed,
            (
                LaneFlowFixedSet::Step.run_if(healthy),
                LaneFlowFixedSet::Observe.run_if(healthy),
            ),
        )
        .add_systems(
            LaneFlowOuterFrame,
            administer
                .run_if(healthy)
                .in_set(LaneFlowOuterFrameSet::Administration),
        )
        .configure_sets(PostUpdate, Present.before(TransformSystems::Propagate))
        .add_systems(
            PostUpdate,
            (collect_adapter_error, present, summarize, exit_on_failure)
                .chain()
                .in_set(Present),
        );
    Ok(())
}

pub(super) fn fail(world: &mut World, error: impl Debug) {
    let tick = world.resource::<LaneFlowSession>().world().tick_index();
    let mut health = world.resource_mut::<Health>();
    if health.failure.is_none() {
        health.failure = Some(Failure {
            tick,
            detail: format!("{error:?}"),
        });
    }
    world.resource_mut::<Summary>().current = None;
}

fn collect_adapter_error(world: &mut World) {
    let error = world.resource::<LaneFlowSession>().last_error().cloned();
    if let Some(error) = error {
        fail(world, error);
    }
}

/// 每个外层帧最多尝试一次，因此零步进帧也能切换，且先于本帧步进和位姿提取。
fn administer(world: &mut World) {
    let tick = world.resource::<LaneFlowSession>().world().tick_index();
    let mut cutover = world.resource_mut::<Cutover>();
    if cutover.pending.is_none() && cutover.plan.front().is_some_and(|&at| at <= tick) {
        cutover.plan.pop_front();
        cutover.request();
    }
    let Some(target) = cutover.pending else {
        return;
    };
    let root = Arc::clone(&cutover.target(target).root);
    match SpatialSession::bind(root) {
        Ok(Some(spatial)) => attempt(world, spatial),
        Ok(None) => fail(world, "目标修订没有 Spatial component"),
        Err(error) => fail(world, error),
    }
}

/// Spatial 由调用方传入，供测试用错配的目标走同一条切换路径。
pub(super) fn attempt(world: &mut World, spatial: SpatialSession) {
    let cutover = world.resource::<Cutover>();
    let Some(name) = cutover.pending else {
        return;
    };
    let target = cutover.target(name);
    let source = match target.source() {
        Ok(source) => source,
        Err(error) => {
            fail(world, error);
            return;
        }
    };
    let (root, diff, binding) = (
        Arc::clone(&target.root),
        Arc::clone(&target.diff),
        target.binding,
    );
    let result = world
        .resource_mut::<LaneFlowSession>()
        .cross_revision_cutover(
            root,
            source,
            &diff,
            binding,
            LaneFlowTargetSpatial::Rebind(spatial),
            &PREFLIGHT,
            &CutoverTransactionLimits::default(),
        );
    let outcome = match result {
        Ok(record) => finish(world, name, record),
        Err(LaneFlowAdapterError::Cutover {
            source: CutoverError::UnmappableLaneEdge { .. },
        }) => {
            refuse(
                world,
                RefusalReason::UnmappableLaneEdge,
                CutoverState::AwaitingClear,
            );
            clear_spur(world)
        }
        Err(LaneFlowAdapterError::Cutover {
            source: CutoverError::VehicleRevalidationFailed { .. },
        }) => {
            refuse(
                world,
                RefusalReason::VehicleRevalidation,
                CutoverState::AwaitingRetry,
            );
            Ok(())
        }
        Err(error) => Err(format!("{error:?}")),
    };
    if let Err(error) = outcome {
        fail(world, error);
    }
}

fn refuse(world: &mut World, reason: RefusalReason, state: CutoverState) {
    let tick = world.resource::<LaneFlowSession>().world().tick_index();
    let mut cutover = world.resource_mut::<Cutover>();
    cutover.state = state;
    cutover.last_refusal = Some(Refusal { reason, tick });
}

/// 路线必须在车辆移除之后才能删除。代理实体保留，供下次进入 `R2` 使用。
pub(super) fn clear_spur(world: &mut World) -> Result<(), String> {
    let mut fleet = world.resource_mut::<Fleet>();
    let vehicle = fleet.vehicles[SPUR_SLOT].take();
    let route = fleet
        .spur_route
        .take()
        .ok_or("目标修订不可映射，但没有支路路线可以清除")?;
    if let Some(vehicle) = vehicle {
        despawn_vehicle(world, vehicle).map_err(|error| format!("{error:?}"))?;
    }
    world
        .resource_mut::<LaneFlowSession>()
        .world_mut()
        .remove_route(route)
        .map_err(|error| format!("{error:?}"))
}

/// 切换已提交：依次复核旧批次、交付事件、交还旧 Spatial、重建路面，进入 `R2` 时生成支路车。
fn finish(
    world: &mut World,
    name: RevisionName,
    mut record: LaneFlowCutoverRecord,
) -> Result<(), String> {
    let session = world.resource::<LaneFlowSession>();
    let presentation = world.resource::<Presentation>();
    let held_batch = presentation.frame > 0;
    if held_batch && session.consumption_context_is_current(presentation.poses.context()) {
        return Err("切换后旧批次仍被判定为当前".into());
    }
    let events = record.events().as_slice().len() as u64;
    let retired = record.retired_spatial().ok_or("切换未交还旧 Spatial")?;
    let cutover = world.resource::<Cutover>();
    if !Arc::ptr_eq(&retired.revision(), &cutover.target(cutover.current).root) {
        return Err("交还的 Spatial 不属于切换前的修订".into());
    }
    drop(retired);
    let revision = session.world().revision();
    let lanes = scene::centerlines(&revision)?;
    let mut road = world.resource_mut::<RoadData>();
    road.revision = revision.canonical_origin().network_revision();
    road.lanes = lanes;
    road.version += 1;
    let mut cutover = world.resource_mut::<Cutover>();
    cutover.current = name;
    cutover.pending = None;
    cutover.state = CutoverState::Idle;
    cutover.successes += 1;
    cutover.events += events;
    cutover.retired += 1;
    cutover.stale_rejected += u32::from(held_batch);
    if name == RevisionName::R2 {
        spawn_spur(world)?;
    }
    Ok(())
}

fn spawn_spur(world: &mut World) -> Result<(), String> {
    let proxy = world.resource::<Fleet>().entities[SPUR_SLOT];
    let mut session = world.resource_mut::<LaneFlowSession>();
    let revision = session.world().revision();
    let spur = scene::edge(&revision, "spur")?;
    let car = scene::profile(&revision)?;
    let mut runtime = session.world_mut();
    let route = runtime
        .register_route(RouteRegisterInput::new(vec![spur]))
        .map_err(|error| format!("{error:?}"))?;
    let vehicle = runtime
        .spawn_vehicle(VehicleSpawnInput::new(car, route, 0, SPUR_START_MM, 0).with_open_entrance())
        .map_err(|error| format!("{error:?}"))?;
    session
        .bind_vehicle_entity(vehicle, proxy)
        .map_err(|error| format!("{error:?}"))?;
    let mut fleet = world.resource_mut::<Fleet>();
    fleet.vehicles[SPUR_SLOT] = Some(vehicle);
    fleet.spur_route = Some(route);
    Ok(())
}

pub(super) fn transform_from_pose(pose: CanonicalPoseF32) -> Result<Transform, &'static str> {
    let p = pose.position();
    let t = pose.tangent();
    let u = pose.up();
    let position = Vec3::new(p.x(), p.y(), p.z());
    let tangent = Vec3::new(t.x(), t.y(), t.z());
    let up = Vec3::new(u.x(), u.y(), u.z());
    if !position.is_finite()
        || !tangent.is_finite()
        || !up.is_finite()
        || tangent.length_squared() <= f32::EPSILON
        || up.length_squared() <= f32::EPSILON
        || tangent.cross(up).length_squared() <= f32::EPSILON
    {
        return Err("位姿 position/tangent/up 非有限或退化");
    }
    let transform = Transform::from_translation(position).looking_to(tangent, up);
    if !transform.rotation.is_finite() {
        return Err("转换后旋转非有限");
    }
    Ok(transform)
}

/// 场景没有停车位，出现 `Parking` 来源或停车绑定都按失败处理；见设计 §19.6。
pub(super) fn classify(
    status: VehicleStatus,
    binding: Option<ParkingBinding>,
    source: Option<PoseSource>,
    in_batch: bool,
) -> Result<bool, &'static str> {
    let visible = match (status, binding, source) {
        (VehicleStatus::Active, None, Some(PoseSource::Lane { .. })) => true,
        (VehicleStatus::Completed, None, None) => false,
        _ => return Err("车辆状态、停车绑定与位姿来源的组合不合法"),
    };
    if visible != in_batch {
        return Err("位姿来源与批次是否出现不一致");
    }
    Ok(visible)
}

fn present(world: &mut World) {
    if world.resource::<Health>().failure.is_some() {
        return;
    }
    let context = world.resource::<LaneFlowSession>().consumption_context();
    present_with(world, context);
}

/// 上下文由调用方传入，供测试用过期上下文走同一条提交路径。
pub(super) fn present_with(world: &mut World, context: LaneFlowConsumptionContext) {
    let result = world.resource_scope(|world, mut session: Mut<LaneFlowSession>| {
        world.resource_scope(|world, mut presentation: Mut<Presentation>| {
            commit(world, &mut session, &mut presentation, context)
        })
    });
    if let Err(error) = result {
        fail(world, error);
    }
}

fn commit(
    world: &mut World,
    session: &mut LaneFlowSession,
    presentation: &mut Presentation,
    context: LaneFlowConsumptionContext,
) -> Result<(), String> {
    let fleet = world.resource::<Fleet>();
    let token = fleet.token;
    let root = world
        .get::<Transform>(fleet.root)
        .ok_or("root Transform 缺失")?;
    if *root != Transform::IDENTITY
        || world
            .get::<Placement>(fleet.root)
            .is_none_or(|placement| placement.0 != token)
    {
        return Err("root/placement token 失效".into());
    }
    if !session.consumption_context_is_current(context) {
        return Err("过期消费上下文".into());
    }
    let Presentation {
        visible,
        poses,
        next_visible,
        in_batch,
        staged,
        frame,
    } = presentation;
    session
        .extract_committed_pose_batch(token, poses)
        .map_err(|error| format!("{error:?}"))?;
    if !session.consumption_context_is_current(poses.context())
        || poses.batch().placement_token() != token
    {
        return Err("过期位姿批次".into());
    }
    if poses.batch().network_revision() != Some(world.resource::<RoadData>().revision) {
        return Err("批次修订与路面修订不一致".into());
    }
    staged.clear();
    in_batch.fill(false);
    for (&vehicle, record) in poses.vehicles().iter().zip(poses.batch().records()) {
        let entity = session.vehicle_entity(vehicle).ok_or("位姿缺少绑定")?;
        let slot = world
            .get::<VehicleSlot>(entity)
            .filter(|slot| {
                fleet.entities.get(slot.0) == Some(&entity)
                    && fleet.vehicles[slot.0] == Some(vehicle)
            })
            .ok_or("绑定实体不属于此宿主")?
            .0;
        if world.get::<Transform>(entity).is_none() {
            return Err("车辆 Transform 缺失".into());
        }
        staged.push((entity, transform_from_pose(record.pose())?));
        in_batch[slot] = true;
    }
    for (slot, vehicle) in fleet.vehicles.iter().enumerate() {
        next_visible[slot] = match vehicle {
            // 代理实体在没有支路车时隐藏，不参与状态表。
            None => false,
            Some(vehicle) => {
                let state = session.world().vehicle(*vehicle).ok_or("车辆已失效")?;
                let source = session
                    .world()
                    .committed_pose_source(*vehicle)
                    .map_err(|error| format!("{error:?}"))?;
                classify(
                    state.status(),
                    session.world().parking_binding(*vehicle),
                    source,
                    in_batch[slot],
                )?
            }
        };
    }
    // 上面全部通过后才写入；此后不再失败，保证整批原子。
    for &(entity, transform) in staged.iter() {
        *world
            .get_mut::<Transform>(entity)
            .expect("prevalidated vehicle") = transform;
    }
    visible.copy_from_slice(next_visible);
    *frame += 1;
    Ok(())
}

fn spur_stage(session: &LaneFlowSession, vehicle: Option<VehicleHandle>) -> &'static str {
    let status = vehicle.and_then(|vehicle| session.world().vehicle(vehicle));
    match status.map(|state| state.status()) {
        None => "none",
        Some(VehicleStatus::Completed) => "completed",
        Some(_) => "driving",
    }
}

fn summarize(world: &mut World) {
    if world.resource::<Health>().failure.is_some() {
        return;
    }
    let session = world.resource::<LaneFlowSession>();
    let presentation = world.resource::<Presentation>();
    let cutover = world.resource::<Cutover>();
    let fleet = world.resource::<Fleet>();
    let report = session.frame_report();
    let summary = FrameSummary {
        frame: presentation.frame,
        tick: session.world().tick_index(),
        steps: report.steps_run(),
        backlog: report.backlog(),
        revision: cutover.current,
        revision_id: world.resource::<RoadData>().revision,
        generation: session.world().world_generation().get(),
        state: cutover.state,
        target: cutover.pending,
        successes: cutover.successes,
        events: cutover.events,
        stale_rejected: cutover.stale_rejected,
        refusal: cutover.last_refusal,
        ring_visible: presentation.visible[..SPUR_SLOT]
            .iter()
            .filter(|&&visible| visible)
            .count(),
        spur: spur_stage(session, fleet.vehicles[SPUR_SLOT]),
    };
    world.resource_mut::<Summary>().current = Some(summary);
}

pub(super) fn exit_on_failure(world: &mut World) {
    let mut health = world.resource_mut::<Health>();
    if let Some(failure) = &health.failure
        && !health.exit_sent
    {
        eprintln!(
            "revision_cutover 失败：tick={}, error={}",
            failure.tick, failure.detail
        );
        health.exit_sent = true;
        world
            .resource_mut::<Messages<AppExit>>()
            .write(AppExit::error());
    }
}
