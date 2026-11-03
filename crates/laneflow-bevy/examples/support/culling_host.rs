//! 提取前选择、整批提交、隐藏与周期全量对照。GUI/headless 安装同一份系统和资源。
use std::{
    fmt::Debug,
    time::{Duration, Instant},
};

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
    LaneFlowCommittedPoseBatch, LaneFlowConsumptionContext, LaneFlowPlugin, LaneFlowSession,
};
use laneflow_runtime::{PoseSource, VehicleHandle};
use laneflow_spatial::{CanonicalPoseF32, FramePlacementToken};

use super::{
    culling_scene::Scene,
    culling_view::{LaneChunks, ViewVolume},
};

const AUDIT_INTERVAL_FRAMES: u64 = 60;

/// 车辆序号：`Fleet` 内的稳定下标，不是 Runtime 或 Spatial 数组下标。
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
    pub vehicles: Vec<VehicleHandle>,
    pub entities: Vec<Entity>,
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

/// 逐车可见状态是隐藏语义的权威；GUI 把它映射到 Bevy `Visibility`。
#[derive(Resource)]
pub(super) struct Culling {
    pub visible: Vec<bool>,
    pub selected: Vec<VehicleHandle>,
    pub poses: LaneFlowCommittedPoseBatch,
    pub audits: u64,
    audit_poses: LaneFlowCommittedPoseBatch,
    audit_rows: Vec<Option<usize>>,
    staged: Vec<(Entity, Transform)>,
    seen: Vec<bool>,
    frame: u64,
    selected_time: Duration,
    full_time: Option<Duration>,
}

impl Culling {
    fn new(vehicles: usize) -> Self {
        Self {
            visible: vec![false; vehicles],
            selected: Vec::with_capacity(vehicles),
            poses: LaneFlowCommittedPoseBatch::new(),
            audits: 0,
            audit_poses: LaneFlowCommittedPoseBatch::new(),
            audit_rows: vec![None; vehicles],
            staged: Vec::with_capacity(vehicles),
            seen: vec![false; vehicles],
            frame: 0,
            selected_time: Duration::ZERO,
            full_time: None,
        }
    }
}

/// 选择视域是否跟随相机；冻结只影响表现。
#[derive(Resource, Default)]
pub(super) struct ViewControl {
    pub frozen: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct FrameSummary {
    pub frame: u64,
    pub tick: u64,
    pub steps: u32,
    pub backlog: Duration,
    pub total: usize,
    pub selected: usize,
    pub output: usize,
    pub selected_time: Duration,
    pub full_time: Option<Duration>,
    pub frozen: bool,
}

impl FrameSummary {
    pub fn hud(&self) -> String {
        let full = self
            .full_time
            .map_or_else(|| "-".to_owned(), |time| time.as_micros().to_string());
        let view = if self.frozen {
            "FROZEN (F: resume)"
        } else {
            "follows camera (F: freeze)"
        };
        format!(
            "LaneFlow | culling extraction\nvehicles {}   selected {}   output {}\nselection view: {}\ntick {}   steps {}\nselected path {} us   full extract {} us\n(observation only, not performance evidence)\nWheel: zoom | LMB: pan | RMB: orbit",
            self.total,
            self.selected,
            self.output,
            view,
            self.tick,
            self.steps,
            self.selected_time.as_micros(),
            full,
        )
    }
}

#[derive(Resource, Default)]
pub(super) struct Summary {
    pub current: Option<FrameSummary>,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, SystemSet)]
pub(super) struct Present;

pub(super) fn install(app: &mut App, scene: Scene, view: ViewVolume) -> Result<(), String> {
    if app.world().contains_resource::<LaneFlowSession>() {
        return Err("宿主已安装，禁止复用另一世界的 Session".into());
    }
    let Scene {
        mut session,
        vehicles,
        lanes,
        lane_lengths_mm,
        vehicle_length,
        lane_width,
        ..
    } = scene;
    // 车身从前保险杠向后延伸；余量覆盖车长、半个车道宽和模型高度。
    let chunks = LaneChunks::build(&lanes, &lane_lengths_mm, vehicle_length + lane_width * 0.5)?;
    let token = FramePlacementToken::new(1);
    let root = app
        .world_mut()
        .spawn((Transform::IDENTITY, Placement(token)))
        .id();
    let mut entities = Vec::with_capacity(vehicles.len());
    for (slot, &vehicle) in vehicles.iter().enumerate() {
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
            .bind_vehicle_entity(vehicle, entity)
            .map_err(|error| format!("{error:?}"))?;
        entities.push(entity);
    }
    let culling = Culling::new(vehicles.len());
    app.add_plugins(LaneFlowPlugin)
        .insert_resource(session)
        .insert_resource(chunks)
        .insert_resource(view)
        .insert_resource(Fleet {
            root,
            token,
            vehicles,
            entities,
            vehicle_length,
        })
        .insert_resource(culling)
        .init_resource::<Health>()
        .init_resource::<Summary>()
        .init_resource::<ViewControl>()
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
    transform_from_axes(
        Vec3::new(p.x(), p.y(), p.z()),
        Vec3::new(t.x(), t.y(), t.z()),
        Vec3::new(u.x(), u.y(), u.z()),
    )
}

pub(super) fn transform_from_axes(
    position: Vec3,
    tangent: Vec3,
    up: Vec3,
) -> Result<Transform, &'static str> {
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
        world.resource_scope(|world, mut culling: Mut<Culling>| {
            commit(world, &mut session, &mut culling, context)
        })
    });
    if let Err(error) = result {
        fail(world, error);
    }
}

fn commit(
    world: &mut World,
    session: &mut LaneFlowSession,
    culling: &mut Culling,
    context: LaneFlowConsumptionContext,
) -> Result<(), String> {
    let started = Instant::now();
    let view = world.resource::<ViewVolume>().clone();
    world.resource_mut::<LaneChunks>().refresh(&view);
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
    let chunks = world.resource::<LaneChunks>();
    culling.selected.clear();
    for &vehicle in &fleet.vehicles {
        // 只读 Runtime 来源，不用实体旧 Transform：隐藏车辆的 Transform 不再更新。
        match session
            .world()
            .committed_pose_source(vehicle)
            .map_err(|error| format!("{error:?}"))?
        {
            None => {}
            Some(PoseSource::Lane { edge, progress_mm }) => {
                if chunks
                    .is_visible(edge, progress_mm)
                    .ok_or("来源车道不在分块表内")?
                {
                    culling.selected.push(vehicle);
                }
            }
            Some(PoseSource::Parking { .. }) => {
                return Err("本例场景不应出现停车来源".into());
            }
        }
    }
    let Culling {
        selected,
        poses,
        staged,
        seen,
        visible,
        ..
    } = culling;
    session
        .extract_selected_committed_pose_batch(context, selected, token, poses)
        .map_err(|error| format!("{error:?}"))?;
    if !session.consumption_context_is_current(poses.context())
        || poses.batch().placement_token() != token
    {
        return Err("过期位姿批次".into());
    }
    staged.clear();
    seen.fill(false);
    for (&vehicle, record) in poses.vehicles().iter().zip(poses.batch().records()) {
        let entity = session.vehicle_entity(vehicle).ok_or("位姿缺少绑定")?;
        let slot = world
            .get::<VehicleSlot>(entity)
            .filter(|slot| fleet.entities.get(slot.0) == Some(&entity))
            .ok_or("绑定实体不属于此宿主")?
            .0;
        if world.get::<Transform>(entity).is_none() {
            return Err("车辆 Transform 缺失".into());
        }
        staged.push((entity, transform_from_pose(record.pose())?));
        seen[slot] = true;
    }
    // 上面全部通过后才写入；此后不再失败，保证整批原子。
    for &(entity, transform) in staged.iter() {
        *world
            .get_mut::<Transform>(entity)
            .expect("prevalidated vehicle") = transform;
    }
    visible.copy_from_slice(seen);
    culling.selected_time = started.elapsed();
    culling.frame += 1;
    if culling.frame.is_multiple_of(AUDIT_INTERVAL_FRAMES) {
        audit(world, session, culling)?;
    }
    Ok(())
}

/// 同一封闭区间内与全量提取逐值对照，结果不应用到实体。
fn audit(
    world: &World,
    session: &mut LaneFlowSession,
    culling: &mut Culling,
) -> Result<(), String> {
    let started = Instant::now();
    let token = world.resource::<Fleet>().token;
    session
        .extract_committed_pose_batch(token, &mut culling.audit_poses)
        .map_err(|error| format!("{error:?}"))?;
    culling.full_time = Some(started.elapsed());
    culling.audit_rows.fill(None);
    let slot_of = |vehicle: VehicleHandle| {
        session
            .vehicle_entity(vehicle)
            .and_then(|entity| world.get::<VehicleSlot>(entity))
            .map(|slot| slot.0)
    };
    for (row, &vehicle) in culling.audit_poses.vehicles().iter().enumerate() {
        let slot = slot_of(vehicle).ok_or("全量位姿缺少绑定")?;
        culling.audit_rows[slot] = Some(row);
    }
    let full = culling.audit_poses.batch().records();
    for (&vehicle, record) in culling
        .poses
        .vehicles()
        .iter()
        .zip(culling.poses.batch().records())
    {
        let slot = slot_of(vehicle).ok_or("位姿缺少绑定")?;
        let row = culling.audit_rows[slot].ok_or("选中车辆不在全量结果中")?;
        if full[row].pose() != record.pose() {
            return Err(format!("选择与全量位姿不一致：{vehicle:?}"));
        }
    }
    culling.audits += 1;
    Ok(())
}

fn summarize(world: &mut World) {
    if world.resource::<Health>().failure.is_some() {
        return;
    }
    let session = world.resource::<LaneFlowSession>();
    let culling = world.resource::<Culling>();
    let report = session.frame_report();
    let summary = FrameSummary {
        frame: culling.frame,
        tick: session.world().tick_index(),
        steps: report.steps_run(),
        backlog: report.backlog(),
        total: session.world().live_vehicles().len(),
        selected: culling.selected.len(),
        output: culling.poses.vehicles().len(),
        selected_time: culling.selected_time,
        full_time: culling.full_time,
        frozen: world.resource::<ViewControl>().frozen,
    };
    world.resource_mut::<Summary>().current = Some(summary);
}

pub(super) fn exit_on_failure(world: &mut World) {
    let mut health = world.resource_mut::<Health>();
    if let Some(failure) = &health.failure
        && !health.exit_sent
    {
        eprintln!(
            "culling_extraction 失败：tick={}, error={}",
            failure.tick, failure.detail
        );
        health.exit_sent = true;
        world
            .resource_mut::<Messages<AppExit>>()
            .write(AppExit::error());
    }
}
