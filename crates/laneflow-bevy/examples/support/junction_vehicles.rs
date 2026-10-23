//! 车辆表现：按角色着色的车体、Entity 绑定与每帧位姿同步。
use bevy::prelude::*;
use laneflow_bevy::{LaneFlowCommittedPoseBatch, LaneFlowSession};
use laneflow_spatial::{CanonicalPoseF32, FramePlacementToken};
use laneflow_static_contract::VehicleProfileOrdinal;
use laneflow_static_network::SharedNetworkRevision;

use super::junction_debug_scene::{SpawnedVehicle, VehicleRole};

/// 车辆 box 宽度近似（米）；车型线格式只有车长。
pub(super) const VEHICLE_WIDTH_METERS: f32 = 1.8;
const VEHICLE_HEIGHT_METERS: f32 = 1.5;

#[derive(Resource)]
pub(super) struct SpawnedPlan(pub Box<[SpawnedVehicle]>);

/// 跨帧复用的提取缓冲（adapter-api §6 稳定容量合同）。
#[derive(Resource, Default)]
pub(super) struct PoseBuffer(pub LaneFlowCommittedPoseBatch);

/// 车辆表现几何：Runtime pose 是前保险杠位置，表现中心沿切向后移半个车长。
#[derive(Resource)]
pub(super) struct VehicleMetrics {
    pub half_length: f32,
}

#[derive(Component)]
pub(super) struct VehicleBox;

/// 首个车型的车长（毫米）；缺省 4.5 m。
pub(super) fn profile_length(revision: &SharedNetworkRevision) -> u32 {
    revision
        .traffic()
        .relations()
        .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
        .map_or(4_500, |profile| profile.length_mm())
}

/// 按角色着色，车体 + 深色 cabin 子实体，中心落在前保险杠后方半个车长。
pub(super) fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    session: &mut LaneFlowSession,
    plan: &[SpawnedVehicle],
) {
    let length = profile_length(&session.world().revision()) as f32 / 1_000.0;
    commands.insert_resource(VehicleMetrics {
        half_length: length / 2.0,
    });
    let vehicle_mesh = meshes.add(Cuboid::new(
        VEHICLE_WIDTH_METERS,
        VEHICLE_HEIGHT_METERS,
        length,
    ));
    let cabin_mesh = meshes.add(Cuboid::new(VEHICLE_WIDTH_METERS * 0.85, 0.55, length * 0.5));
    let cabin_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.08, 0.09, 0.12),
        ..default()
    });
    for spawned in plan {
        let body = materials.add(StandardMaterial {
            base_color: role_color(spawned.role),
            perceptual_roughness: 0.5,
            ..default()
        });
        let entity = commands
            .spawn((
                Mesh3d(vehicle_mesh.clone()),
                MeshMaterial3d(body),
                Transform::IDENTITY,
                VehicleBox,
            ))
            .id();
        commands.spawn((
            Mesh3d(cabin_mesh.clone()),
            MeshMaterial3d(cabin_material.clone()),
            Transform::from_xyz(0.0, VEHICLE_HEIGHT_METERS * 0.5 + 0.1, -0.3),
            ChildOf(entity),
        ));
        if let Err(error) = session.bind_vehicle_entity(spawned.vehicle, entity) {
            warn!("bind_vehicle_entity 失败: {error:?}");
        }
    }
}

fn role_color(role: VehicleRole) -> Color {
    match role {
        VehicleRole::Through => Color::srgb(0.15, 0.35, 0.95),
        VehicleRole::WaitingLeft => Color::srgb(0.9, 0.15, 0.12),
        VehicleRole::PermissiveLeft => Color::srgb(0.95, 0.8, 0.1),
        VehicleRole::Circuit => Color::srgb(0.15, 0.75, 0.3),
    }
}

/// 车辆表现中心在路面上的变换：沿切向从前保险杠后移半个车长，尚未抬升。
pub(super) fn center_on_road(pose: CanonicalPoseF32, half_length: f32) -> Transform {
    let position = pose.position();
    let tangent = pose.tangent();
    let up = pose.up();
    let forward = Vec3::new(tangent.x(), tangent.y(), tangent.z());
    let mut transform = Transform::from_xyz(position.x(), position.y(), position.z())
        .looking_to(forward, Vec3::new(up.x(), up.y(), up.z()));
    transform.translation -= forward * half_length;
    transform
}

pub(super) fn sync_vehicle_transforms(
    mut session: ResMut<LaneFlowSession>,
    mut buffer: ResMut<PoseBuffer>,
    metrics: Res<VehicleMetrics>,
    mut transforms: Query<&mut Transform, With<VehicleBox>>,
) {
    if session
        .extract_committed_pose_batch(FramePlacementToken::new(1), &mut buffer.0)
        .is_err()
    {
        return;
    }
    if !session.consumption_context_is_current(buffer.0.context()) {
        return;
    }
    for (vehicle, record) in buffer
        .0
        .vehicles()
        .iter()
        .zip(buffer.0.batch().records().iter())
    {
        let Some(entity) = session.vehicle_entity(*vehicle) else {
            continue;
        };
        if let Ok(mut transform) = transforms.get_mut(entity) {
            // 抬升半个车高，让车底落在路面上。
            let mut target = center_on_road(record.pose(), metrics.half_length);
            target.translation += Vec3::Y * (VEHICLE_HEIGHT_METERS * 0.5 + 0.02);
            *transform = target;
        }
    }
}
