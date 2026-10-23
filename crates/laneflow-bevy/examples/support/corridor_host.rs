//! 逐拍人口链与整批表现提交。GUI/headless 安装同一份系统和资源。
use std::{fmt::Debug, time::Duration};

use bevy_app::{App, AppExit, PostUpdate};
use bevy_ecs::{
    component::Component,
    entity::Entity,
    hierarchy::ChildOf,
    message::Messages,
    resource::Resource,
    schedule::{IntoScheduleConfigs, SystemSet},
    system::Res,
    world::{Mut, World},
};
use bevy_math::Vec3;
use bevy_transform::{TransformSystems, components::Transform};
use laneflow_bevy::{
    LaneFlowCommittedPoseBatch, LaneFlowFixed, LaneFlowFixedSet, LaneFlowPlugin, LaneFlowSession,
    LaneFlowVehicleReplaceOutcome, replace_completed_vehicle,
};
use laneflow_runtime::{ReplaceError, VehicleHandle, VehicleReplaceRecord, VehicleSpawnInput};
use laneflow_scenario::signalized_corridor::{
    CorridorPopulationController, CorridorPopulationCounts, CorridorReplaceAttemptOutcome,
};
use laneflow_spatial::{CanonicalPoseF32, FramePlacementToken};
use laneflow_static_contract::{SignalAspect, SignalGroupOrdinal};

use super::corridor_scene::Scene;

#[derive(Resource)]
pub(super) struct Population(pub CorridorPopulationController);

#[cfg(test)]
#[derive(Resource, Default)]
pub(super) struct BoundaryTrace {
    pub replacements: Vec<VehicleReplaceRecord>,
    pub lifecycle_calls: u64,
    pub observe_calls: u64,
}

#[derive(Component)]
pub(super) struct VehicleProxy;

#[derive(Component)]
pub(super) struct VehicleModel;

#[derive(Component)]
pub(super) struct Placement(pub FramePlacementToken);

#[derive(Resource)]
pub(super) struct Proxies {
    pub root: Entity,
    pub token: FramePlacementToken,
    pub entities: Vec<Entity>,
    pub models: Vec<Entity>,
    pub vehicle_length: f32,
    pub seed: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stage {
    Lifecycle,
    Observe,
    Adapter,
    Presentation,
}

#[derive(Debug)]
pub(super) struct Failure {
    pub stage: Stage,
    pub tick: u64,
    pub detail: String,
}

#[derive(Resource, Default)]
pub(super) struct Health {
    pub failure: Option<Failure>,
    pub exit_sent: bool,
}

#[derive(Resource, Default)]
pub(super) struct Presentation {
    pub poses: LaneFlowCommittedPoseBatch,
    staged: Vec<(Entity, Transform)>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct FrameSummary {
    pub frame: u64,
    pub tick: u64,
    pub time_ms: u64,
    pub seed: u64,
    pub delta: Duration,
    pub steps: u32,
    pub backlog: Duration,
    pub catch_up_limit: u32,
    pub limited: bool,
    pub counts: CorridorPopulationCounts,
    pub signals: Vec<(SignalGroupOrdinal, SignalAspect)>,
}

impl FrameSummary {
    pub fn hud(&self) -> String {
        format!(
            "LaneFlow | corridor traffic\nvehicles {}   running {}   pending {}\nsteps {}   backlog_ms {:.3}\ncatch_up_limit {}   reached {}\ntick {}   seed {}\nWheel: zoom | LMB: pan | RMB: orbit",
            self.counts.target,
            self.counts.running,
            self.counts.pending,
            self.steps,
            self.backlog.as_secs_f64() * 1_000.0,
            self.catch_up_limit,
            self.limited,
            self.tick,
            self.seed,
        )
    }
}

#[derive(Resource, Default)]
pub(super) struct Summary {
    pub frame: u64,
    pub current: Option<FrameSummary>,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, SystemSet)]
pub(super) struct Present;

pub(super) fn install(app: &mut App, scene: Scene) -> Result<(), String> {
    // Session/controller 一次成对装配，不接受第二个世界沿用原 proxy 和人口状态。
    if app.world().contains_resource::<LaneFlowSession>()
        || app.world().contains_resource::<Population>()
    {
        return Err("走廊宿主已安装，禁止跨世界复用人口资源".into());
    }
    let Scene {
        mut session,
        controller,
        geometry,
        config,
        vehicle_length,
    } = scene;
    #[cfg(test)]
    app.init_resource::<BoundaryTrace>();
    let token = FramePlacementToken::new(1);
    let root = app
        .world_mut()
        .spawn((Transform::IDENTITY, Placement(token)))
        .id();
    let mut entities = Vec::with_capacity(config.target_vehicle_count());
    let mut models = Vec::with_capacity(config.target_vehicle_count());
    for slot in 0..config.target_vehicle_count() {
        let vehicle = controller.logical_vehicle(slot).ok_or("人口槽位缺失")?;
        let entity = app
            .world_mut()
            .spawn((VehicleProxy, Transform::IDENTITY, ChildOf(root)))
            .id();
        let model = app
            .world_mut()
            .spawn((
                VehicleModel,
                Transform::from_xyz(0.0, 0.65, vehicle_length * 0.5),
                ChildOf(entity),
            ))
            .id();
        session
            .bind_vehicle_entity(vehicle, entity)
            .map_err(|error| format!("{error:?}"))?;
        entities.push(entity);
        models.push(model);
    }
    app.add_plugins(LaneFlowPlugin)
        .insert_resource(session)
        .insert_resource(Population(controller))
        .insert_resource(geometry)
        .insert_resource(Proxies {
            root,
            token,
            entities,
            models,
            vehicle_length,
            seed: config.seed(),
        })
        .init_resource::<Health>()
        .init_resource::<Presentation>()
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
            (
                lifecycle.in_set(LaneFlowFixedSet::Lifecycle),
                observe.in_set(LaneFlowFixedSet::Observe),
            ),
        )
        .configure_sets(PostUpdate, Present.before(TransformSystems::Propagate))
        .add_systems(
            PostUpdate,
            (
                collect_adapter_error,
                sync_proxies,
                summarize,
                exit_on_failure,
            )
                .chain()
                .in_set(Present),
        );
    sync_proxies(app.world_mut());
    if let Some(failure) = &app.world().resource::<Health>().failure {
        return Err(format!("启动位姿预检失败：{failure:?}"));
    }
    Ok(())
}

pub(super) fn healthy(health: Res<Health>) -> bool {
    health.failure.is_none()
}

pub(super) fn fail(world: &mut World, stage: Stage, error: impl Debug) {
    let tick = world.resource::<LaneFlowSession>().world().tick_index();
    let mut health = world.resource_mut::<Health>();
    if health.failure.is_none() {
        health.failure = Some(Failure {
            stage,
            tick,
            detail: format!("{error:?}"),
        });
    }
    world.resource_mut::<Summary>().current = None;
}

pub(super) fn lifecycle(world: &mut World) {
    if world.resource::<Health>().failure.is_some() {
        return;
    }
    #[cfg(test)]
    {
        let mut trace = world.resource_mut::<BoundaryTrace>();
        trace.replacements.clear();
        trace.lifecycle_calls += 1;
    }
    apply_with(world, |world, old, input| {
        let result =
            replace_completed_vehicle(world, old, input).map_err(|error| format!("{error:?}"))?;
        #[cfg(test)]
        if let LaneFlowVehicleReplaceOutcome::Replaced(record) = result {
            world
                .resource_mut::<BoundaryTrace>()
                .replacements
                .push(VehicleReplaceRecord {
                    old: record.old,
                    new: record.new,
                });
        }
        Ok(result)
    });
}

// callback 只在 example 内可替换，供 C05/C07 验证 hostile outcome；生产始终调用 typed transaction。
pub(super) fn apply_with(
    world: &mut World,
    mut replace: impl FnMut(
        &mut World,
        VehicleHandle,
        VehicleSpawnInput,
    ) -> Result<LaneFlowVehicleReplaceOutcome, String>,
) {
    if world.resource::<Health>().failure.is_some() {
        return;
    }
    let result = world.resource_scope(|world, mut population: Mut<Population>| {
        let session = world.resource::<LaneFlowSession>();
        let revision = session.world().revision().network_revision();
        let policy = session.world().policy_selection();
        population.0.apply_pending(revision, policy, |old, input| {
            let expected = world
                .resource::<LaneFlowSession>()
                .vehicle_entity(old)
                .ok_or("回流车辆缺少 proxy")?;
            if !world.resource::<Proxies>().entities.contains(&expected)
                || world.get::<VehicleProxy>(expected).is_none()
            {
                return Err("回流 proxy 不属于此宿主".into());
            }
            let result = replace(world, old, input)?;
            convert_outcome(world, old, expected, result)
        })
    });
    if let Err(error) = result {
        fail(world, Stage::Lifecycle, error);
    }
}

fn convert_outcome(
    world: &World,
    old: VehicleHandle,
    entity: Entity,
    result: LaneFlowVehicleReplaceOutcome,
) -> Result<CorridorReplaceAttemptOutcome, String> {
    match result {
        LaneFlowVehicleReplaceOutcome::Replaced(record) => {
            let session = world.resource::<LaneFlowSession>();
            if record.old != old
                || record.entity != Some(entity)
                || session.vehicle_entity(old).is_some()
                || session.vehicle_entity(record.new) != Some(entity)
                || session.world().vehicle(record.new).is_none()
            {
                return Err(format!(
                    "已提交回流身份不一致：expected old={old:?}, entity={entity:?}, actual={record:?}"
                ));
            }
            Ok(CorridorReplaceAttemptOutcome::Replaced(
                VehicleReplaceRecord {
                    old: record.old,
                    new: record.new,
                },
            ))
        }
        LaneFlowVehicleReplaceOutcome::Blocked(block) => {
            Ok(CorridorReplaceAttemptOutcome::Blocked(block))
        }
        LaneFlowVehicleReplaceOutcome::Retryable(
            ReplaceError::StopConstraintUnsatisfiable
            | ReplaceError::UnsafeLeader { .. }
            | ReplaceError::UnsafeFollower { .. },
        ) => Ok(CorridorReplaceAttemptOutcome::Retryable),
        LaneFlowVehicleReplaceOutcome::Retryable(error) => {
            Err(format!("不可重试的 Adapter outcome：{error:?}"))
        }
        _ => Err("未知 Adapter replacement outcome".into()),
    }
}

pub(super) fn observe(world: &mut World) {
    if world.resource::<Health>().failure.is_some() {
        return;
    }
    #[cfg(test)]
    {
        world.resource_mut::<BoundaryTrace>().observe_calls += 1;
    }
    let result = world.resource_scope(|world, mut population: Mut<Population>| {
        population
            .0
            .consume_world(world.resource::<LaneFlowSession>().world())
    });
    if let Err(error) = result {
        fail(world, Stage::Observe, error);
    }
}

pub(super) fn collect_adapter_error(world: &mut World) {
    let error = world.resource::<LaneFlowSession>().last_error().cloned();
    if let Some(error) = error {
        fail(world, Stage::Adapter, error);
    }
}

pub(super) fn pose_transform(pose: CanonicalPoseF32) -> Result<Transform, &'static str> {
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

pub(super) fn sync_proxies(world: &mut World) {
    if world.resource::<Health>().failure.is_some()
        || world.resource::<LaneFlowSession>().last_error().is_some()
    {
        return;
    }
    let result = world.resource_scope(|world, mut session: Mut<LaneFlowSession>| {
        world.resource_scope(
            |world, mut presentation: Mut<Presentation>| -> Result<(), String> {
                let proxies = world.resource::<Proxies>();
                let root = world
                    .get::<Transform>(proxies.root)
                    .ok_or("root Transform 缺失")?;
                if *root != Transform::IDENTITY
                    || world.get::<ChildOf>(proxies.root).is_some()
                    || world
                        .get::<Placement>(proxies.root)
                        .is_none_or(|placement| placement.0 != proxies.token)
                {
                    return Err("root/placement token 失效".into());
                }
                for (slot, (&entity, &model)) in
                    proxies.entities.iter().zip(&proxies.models).enumerate()
                {
                    let vehicle = world
                        .resource::<Population>()
                        .0
                        .logical_vehicle(slot)
                        .ok_or("logical slot 缺失")?;
                    if session.vehicle_entity(vehicle) != Some(entity)
                        || world.get::<VehicleProxy>(entity).is_none()
                        || world.get::<Transform>(entity).is_none()
                        || world
                            .get::<ChildOf>(entity)
                            .is_none_or(|parent| parent.parent() != proxies.root)
                        || world.get::<VehicleModel>(model).is_none()
                        || world
                            .get::<ChildOf>(model)
                            .is_none_or(|parent| parent.parent() != entity)
                    {
                        return Err(format!(
                            "proxy/model 绑定或父子关系失效：slot={slot}, vehicle={vehicle:?}"
                        ));
                    }
                    for target in [entity, model] {
                        let transform = world
                            .get::<Transform>(target)
                            .ok_or("模型 Transform 缺失")?;
                        if !transform.translation.is_finite()
                            || !transform.rotation.is_finite()
                            || transform.scale != Vec3::ONE
                        {
                            return Err(format!(
                                "proxy/model Transform 非有限或不是单位缩放：{target:?}"
                            ));
                        }
                    }
                }
                let Presentation { poses, staged } = &mut *presentation;
                staged.clear();
                session
                    .extract_committed_pose_batch(proxies.token, poses)
                    .map_err(|error| format!("{error:?}"))?;
                if !session.consumption_context_is_current(poses.context())
                    || poses.batch().placement_token() != proxies.token
                {
                    return Err("过期位姿批次".into());
                }
                for (&vehicle, record) in poses.vehicles().iter().zip(poses.batch().records()) {
                    let entity = session.vehicle_entity(vehicle).ok_or("位姿缺少绑定")?;
                    let transform = pose_transform(record.pose())?;
                    staged.push((entity, transform));
                }
                // exclusive system 内先验完全部目标，再提交；Completed 无 pose 时保留原变换。
                for &(entity, transform) in staged.iter() {
                    *world
                        .get_mut::<Transform>(entity)
                        .expect("prevalidated proxy") = transform;
                }
                Ok(())
            },
        )
    });
    if let Err(error) = result {
        fail(world, Stage::Presentation, error);
    }
}

pub(super) fn summarize(world: &mut World) {
    if world.resource::<Health>().failure.is_some() {
        return;
    }
    let session = world.resource::<LaneFlowSession>();
    let report = session.frame_report();
    let summary = FrameSummary {
        frame: world.resource::<Summary>().frame + 1,
        tick: session.world().tick_index(),
        time_ms: session.world().time_ms(),
        seed: world.resource::<Proxies>().seed,
        delta: report.frame_delta(),
        steps: report.steps_run(),
        backlog: report.backlog(),
        catch_up_limit: session.config().max_catch_up_steps().get(),
        limited: report.catch_up_limit_reached(),
        counts: world.resource::<Population>().0.counts(),
        signals: session
            .world()
            .committed_signal_groups()
            .as_slice()
            .to_vec(),
    };
    let mut resource = world.resource_mut::<Summary>();
    resource.frame = summary.frame;
    resource.current = Some(summary);
}

pub(super) fn exit_on_failure(world: &mut World) {
    let mut health = world.resource_mut::<Health>();
    if let Some(failure) = &health.failure
        && !health.exit_sent
    {
        eprintln!(
            "corridor_traffic 失败：stage={:?}, tick={}, error={}",
            failure.stage, failure.tick, failure.detail
        );
        health.exit_sent = true;
        world
            .resource_mut::<Messages<AppExit>>()
            .write(AppExit::error());
    }
}
