//! 分类、整批提交与同帧摘要。GUI/headless 安装同一份系统和资源。
use std::{fmt::Debug, time::Duration};

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
    LaneFlowCommittedPoseBatch, LaneFlowConsumptionContext, LaneFlowFixed, LaneFlowFixedSet,
    LaneFlowPlugin, LaneFlowSession,
};
use laneflow_runtime::{
    ParkingBinding, ParkingError, ParkingPoolCounts, ParkingTarget, PoseSource, VehicleStatus,
};
use laneflow_spatial::{CanonicalPoseF32, FramePlacementToken};
use laneflow_static_contract::ParkingFacilityOrdinal;

use super::{
    parking_scene::{Role, Scene},
    parking_script::{self as script, Phase, Rejection, Script},
};

/// `Script.actors` 内的稳定下标，不是 Runtime 或 Spatial 数组下标。
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
    pub facility: ParkingFacilityOrdinal,
    pub vehicle_length: f32,
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

/// 逐车可见状态是表现的权威；GUI 把它映射到 Bevy `Visibility`。
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
    fn new(vehicles: usize) -> Self {
        Self {
            visible: vec![false; vehicles],
            poses: LaneFlowCommittedPoseBatch::new(),
            next_visible: vec![false; vehicles],
            in_batch: vec![false; vehicles],
            staged: Vec::with_capacity(vehicles),
            frame: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ActorRow {
    pub label: &'static str,
    pub stage: &'static str,
    pub visible: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct FrameSummary {
    pub frame: u64,
    pub tick: u64,
    pub steps: u32,
    pub backlog: Duration,
    pub rows: Vec<ActorRow>,
    pub explicit: ParkingPoolCounts,
    pub virtual_pool: ParkingPoolCounts,
    pub rejection: Option<Rejection>,
}

fn pool_line(name: &str, counts: ParkingPoolCounts) -> String {
    format!(
        "{name:<8} vacant {}  reserved {}  occupied {}\n",
        counts.vacant, counts.reserved, counts.occupied
    )
}

fn rejection_line(rejection: Option<Rejection>) -> String {
    let Some(rejection) = rejection else {
        return "last rejected: none\n".into();
    };
    let reason = match rejection.error {
        ParkingError::NotArrived => "not arrived",
        ParkingError::LeavePhysicalOverlap { .. } => "physical overlap",
        ParkingError::LeaveUnsafeFollower { .. } => "unsafe follower",
        _ => "other",
    };
    format!(
        "last rejected: {} {}: {} @ tick {}\n",
        role_label(rejection.role),
        rejection.command,
        reason,
        rejection.tick
    )
}

impl FrameSummary {
    pub fn hud(&self) -> String {
        let mut text = String::from("LaneFlow | parking lifecycle\n");
        for row in &self.rows {
            let shown = if row.stage == "removed" {
                ""
            } else if row.visible {
                "visible"
            } else {
                "hidden"
            };
            text.push_str(&format!("{:<17}{:<19}{shown}\n", row.label, row.stage));
        }
        text.push_str(&pool_line("spaces", self.explicit));
        text.push_str(&pool_line("virtual", self.virtual_pool));
        text.push_str(&rejection_line(self.rejection));
        text.push_str(&format!(
            "tick {}   steps {}\nPad: green vacant | amber reserved | red occupied\nMarker: cyan garage entry | magenta garage exit\nWheel: zoom | LMB: pan | RMB: orbit",
            self.tick, self.steps
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

pub(super) fn install(app: &mut App, scene: Scene) -> Result<(), String> {
    if app.world().contains_resource::<LaneFlowSession>() {
        return Err("宿主已安装，禁止复用另一世界的 Session".into());
    }
    let Scene {
        mut session,
        route,
        cast,
        facility,
        route_occurrences,
        dwell_ticks,
        vehicle_length,
        ..
    } = scene;
    let token = FramePlacementToken::new(1);
    let root = app
        .world_mut()
        .spawn((Transform::IDENTITY, Placement(token)))
        .id();
    let mut entities = Vec::with_capacity(cast.len());
    for (slot, cast) in cast.iter().enumerate() {
        let entity = app
            .world_mut()
            .spawn((VehicleSlot(slot), Transform::IDENTITY, ChildOf(root)))
            .id();
        app.world_mut().spawn((
            VehicleModel,
            Transform::from_xyz(0.0, 0.65, vehicle_length * 0.5),
            ChildOf(entity),
        ));
        session
            .bind_vehicle_entity(cast.vehicle, entity)
            .map_err(|error| format!("{error:?}"))?;
        entities.push(entity);
    }
    let vehicles = cast.len();
    app.add_plugins(LaneFlowPlugin)
        .insert_resource(session)
        .insert_resource(Script::new(&cast, route, route_occurrences, dwell_ticks))
        .insert_resource(Fleet {
            root,
            token,
            entities,
            facility,
            vehicle_length,
        })
        .insert_resource(Presentation::new(vehicles))
        .init_resource::<Health>()
        .init_resource::<Summary>()
        .configure_sets(
            LaneFlowFixed,
            (
                LaneFlowFixedSet::Lifecycle.run_if(healthy),
                LaneFlowFixedSet::Step.run_if(healthy),
                LaneFlowFixedSet::Observe.run_if(healthy),
            ),
        )
        .add_systems(
            LaneFlowFixed,
            script::lifecycle.in_set(LaneFlowFixedSet::Lifecycle),
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

/// 可见性由 typed 状态决定，批次只提供位姿；见设计 §18.4。
pub(super) fn classify(
    status: VehicleStatus,
    binding: Option<ParkingBinding>,
    source: Option<PoseSource>,
    in_batch: bool,
) -> Result<bool, &'static str> {
    let visible = match (status, binding, source) {
        (
            VehicleStatus::Active,
            None | Some(ParkingBinding::Reserved(_)),
            Some(PoseSource::Lane { .. }),
        ) => true,
        (
            VehicleStatus::Parked,
            Some(ParkingBinding::Occupied(ParkingTarget::ExplicitSpace(bound))),
            Some(PoseSource::Parking { space }),
        ) if bound == space => true,
        (
            VehicleStatus::Parked,
            Some(ParkingBinding::Occupied(ParkingTarget::VirtualPool(_))),
            None,
        )
        | (VehicleStatus::Completed, None, None) => false,
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
    let actors = &world.resource::<Script>().actors;
    staged.clear();
    in_batch.fill(false);
    for (&vehicle, record) in poses.vehicles().iter().zip(poses.batch().records()) {
        let entity = session.vehicle_entity(vehicle).ok_or("位姿缺少绑定")?;
        let slot = world
            .get::<VehicleSlot>(entity)
            .filter(|slot| {
                fleet.entities.get(slot.0) == Some(&entity) && actors[slot.0].vehicle == vehicle
            })
            .ok_or("绑定实体不属于此宿主")?
            .0;
        if world.get::<Transform>(entity).is_none() {
            return Err("车辆 Transform 缺失".into());
        }
        staged.push((entity, transform_from_pose(record.pose())?));
        in_batch[slot] = true;
    }
    for (slot, actor) in actors.iter().enumerate() {
        next_visible[slot] = if actor.phase == Phase::Removed {
            false
        } else {
            let state = session
                .world()
                .vehicle(actor.vehicle)
                .ok_or("脚本车辆已失效")?;
            let source = session
                .world()
                .committed_pose_source(actor.vehicle)
                .map_err(|error| format!("{error:?}"))?;
            classify(
                state.status(),
                session.world().parking_binding(actor.vehicle),
                source,
                in_batch[slot],
            )
            .map_err(|error| format!("{:?}：{error}", actor.role))?
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

pub(super) fn role_label(role: Role) -> &'static str {
    match role {
        Role::Explicit => "explicit",
        Role::Virtual => "virtual",
        Role::Blocker => "blocker",
        Role::Removal => "removal",
        Role::Through => "through",
    }
}

/// 车身颜色写进 HUD，画面上的车才能对应到行。
fn row_label(role: Role) -> &'static str {
    match role {
        Role::Explicit => "explicit (blue)",
        Role::Virtual => "virtual (orange)",
        Role::Blocker => "blocker (yellow)",
        Role::Removal => "removal (violet)",
        Role::Through => "through (white)",
    }
}

fn stage(
    session: &LaneFlowSession,
    phase: Phase,
    vehicle: laneflow_runtime::VehicleHandle,
) -> &'static str {
    if phase == Phase::Removed {
        return "removed";
    }
    let world = session.world();
    let status = world.vehicle(vehicle).map(|state| state.status());
    match (status, world.parking_binding(vehicle)) {
        (Some(VehicleStatus::Completed), _) => "completed",
        (_, Some(ParkingBinding::Occupied(ParkingTarget::ExplicitSpace(_)))) => "parked (explicit)",
        (_, Some(ParkingBinding::Occupied(ParkingTarget::VirtualPool(_)))) => "parked (virtual)",
        (_, Some(binding @ ParkingBinding::Reserved(_))) => {
            if world.parking_arrived(vehicle, binding.target()) {
                "arrived"
            } else {
                "approaching"
            }
        }
        _ => "driving",
    }
}

fn summarize(world: &mut World) {
    if world.resource::<Health>().failure.is_some() {
        return;
    }
    let session = world.resource::<LaneFlowSession>();
    let presentation = world.resource::<Presentation>();
    let script = world.resource::<Script>();
    let Some(counts) = session
        .world()
        .parking_facility_counts(world.resource::<Fleet>().facility)
    else {
        fail(world, "设施不在修订内");
        return;
    };
    let report = session.frame_report();
    let rows = script
        .actors
        .iter()
        .zip(&presentation.visible)
        .map(|(actor, &visible)| ActorRow {
            label: row_label(actor.role),
            stage: stage(session, actor.phase, actor.vehicle),
            visible,
        })
        .collect();
    let summary = FrameSummary {
        frame: presentation.frame,
        tick: session.world().tick_index(),
        steps: report.steps_run(),
        backlog: report.backlog(),
        rows,
        explicit: counts.explicit,
        virtual_pool: counts.virtual_pool,
        rejection: script.last_rejection,
    };
    world.resource_mut::<Summary>().current = Some(summary);
}

pub(super) fn exit_on_failure(world: &mut World) {
    let mut health = world.resource_mut::<Health>();
    if let Some(failure) = &health.failure
        && !health.exit_sent
    {
        eprintln!(
            "parking_lifecycle 失败：tick={}, error={}",
            failure.tick, failure.detail
        );
        health.exit_sent = true;
        world
            .resource_mut::<Messages<AppExit>>()
            .write(AppExit::error());
    }
}
