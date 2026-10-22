//! GUI 与无窗口测试共用的绑定、受控提取与整批 Transform 提交。
use bevy_app::{App, PostUpdate};
use bevy_ecs::{
    entity::Entity,
    resource::Resource,
    schedule::IntoScheduleConfigs,
    system::{Query, ResMut},
};
use bevy_math::Vec3;
use bevy_transform::{TransformSystems, components::Transform};
use laneflow_bevy::{
    LaneFlowAdapterError, LaneFlowCommittedPoseBatch, LaneFlowPlugin, LaneFlowSession,
};
use laneflow_runtime::VehicleHandle;
use laneflow_spatial::FramePlacementToken;

const PLACEMENT: FramePlacementToken = FramePlacementToken::new(1);

#[derive(Debug, PartialEq)]
pub(super) enum PresentationError {
    Extraction(LaneFlowAdapterError),
    StaleBatch,
    UnboundVehicle(VehicleHandle),
    MissingTransform {
        vehicle: VehicleHandle,
        entity: Entity,
    },
    NonFiniteTransform(VehicleHandle),
}

#[derive(Resource, Default)]
pub(super) struct Presentation {
    poses: LaneFlowCommittedPoseBatch,
    staged: Vec<(Entity, Transform)>,
    pub last_error: Option<PresentationError>,
}

pub(super) fn install(
    app: &mut App,
    mut session: LaneFlowSession,
) -> Result<(), LaneFlowAdapterError> {
    let vehicles: Vec<_> = session
        .world()
        .committed_pose_sources()
        .map(|(vehicle, _)| vehicle)
        .collect();
    for vehicle in vehicles {
        let entity = app.world_mut().spawn(Transform::IDENTITY).id();
        session.bind_vehicle_entity(vehicle, entity)?;
    }
    app.add_plugins(LaneFlowPlugin)
        .insert_resource(session)
        .init_resource::<Presentation>()
        .add_systems(PostUpdate, sync_proxies.before(TransformSystems::Propagate));
    Ok(())
}

fn sync_proxies(
    mut session: ResMut<LaneFlowSession>,
    mut presentation: ResMut<Presentation>,
    mut transforms: Query<&mut Transform>,
) {
    let Presentation {
        poses,
        staged,
        last_error,
    } = &mut *presentation;
    staged.clear();
    let result = (|| {
        session
            .extract_committed_pose_batch(PLACEMENT, poses)
            .map_err(PresentationError::Extraction)?;
        if !session.consumption_context_is_current(poses.context())
            || poses.batch().placement_token() != PLACEMENT
        {
            return Err(PresentationError::StaleBatch);
        }
        for (&vehicle, record) in poses.vehicles().iter().zip(poses.batch().records()) {
            let entity = session
                .vehicle_entity(vehicle)
                .ok_or(PresentationError::UnboundVehicle(vehicle))?;
            transforms
                .get(entity)
                .map_err(|_| PresentationError::MissingTransform { vehicle, entity })?;
            let pose = record.pose();
            let position = pose.position();
            let tangent = pose.tangent();
            let up = pose.up();
            let transform = Transform::from_xyz(position.x(), position.y(), position.z())
                .looking_to(
                    Vec3::new(tangent.x(), tangent.y(), tangent.z()),
                    Vec3::new(up.x(), up.y(), up.z()),
                );
            if !transform.translation.is_finite() || !transform.rotation.is_finite() {
                return Err(PresentationError::NonFiniteTransform(vehicle));
            }
            staged.push((entity, transform));
        }
        // Query 借用期间实体集合稳定；整批预检通过才写入，错误不留下半批变换。
        for (entity, transform) in staged.iter() {
            *transforms.get_mut(*entity).expect("prevalidated Transform") = *transform;
        }
        Ok(())
    })();
    let error = result.err();
    if error != *last_error
        && let Some(error) = &error
    {
        eprintln!("quickstart 位姿同步失败：{error:?}");
    }
    *last_error = error;
}
