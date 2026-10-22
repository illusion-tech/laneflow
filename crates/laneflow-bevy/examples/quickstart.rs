//! 同根 Spatial 车道、两辆已绑定车辆与完整位姿同步；关闭窗口结束。
#[path = "support/quickstart_presentation.rs"]
mod presentation;
#[path = "support/quickstart_scene.rs"]
mod quickstart_scene;

use std::error::Error;

use bevy::{mesh::Indices, prelude::*};
use laneflow_bevy::LaneFlowSession;
use laneflow_static_contract::{LaneEdgeOrdinal, VehicleProfileOrdinal};

fn main() -> Result<(), Box<dyn Error>> {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "LaneFlow | quickstart".into(),
            resolution: (1_100, 700).into(),
            ..default()
        }),
        ..default()
    }));
    app.insert_resource(ClearColor(Color::srgb(0.08, 0.12, 0.16)));
    presentation::install(&mut app, quickstart_scene::session()?)?;
    app.add_systems(Startup, setup_scene).run();
    Ok(())
}

fn setup_scene(
    mut commands: Commands,
    session: Res<LaneFlowSession>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let revision = session.world().revision();
    let lanes = revision
        .spatial()
        .and_then(|spatial| spatial.lane_pose())
        .expect("fixture lane geometry");
    let road = materials.add(Color::srgb(0.18, 0.21, 0.25));
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for index in 0..lanes.lane_edge_count() {
        let geometry = lanes
            .lane_geometry(LaneEdgeOrdinal::from_raw(index))
            .expect("fixture lane");
        let points: Vec<_> = geometry
            .points()
            .iter()
            .map(|p| Vec3::new(p.x, p.y, p.z))
            .collect();
        for point in &points {
            min = min.min(*point);
            max = max.max(*point);
        }
        commands.spawn((
            Mesh3d(meshes.add(ribbon(&points))),
            MeshMaterial3d(road.clone()),
        ));
    }
    let center = (min + max) * 0.5;
    let extent = (max - min).max_element().max(10.0);
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(center + Vec3::new(0.35, 0.85, 0.9) * extent)
            .looking_at(center, Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 12_000.0,
            ..default()
        },
        Transform::from_xyz(4.0, 8.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(extent + 12.0, extent + 12.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.25, 0.34, 0.29))),
        Transform::from_translation(center - Vec3::Y * 0.02),
    ));
    let profile = session
        .world()
        .traffic()
        .relations()
        .vehicle_profile(VehicleProfileOrdinal::from_raw(0))
        .expect("fixture profile");
    let length = profile.length_mm() as f32 / 1_000.0;
    let body = meshes.add(Cuboid::new(1.5, 0.8, length));
    let nose = meshes.add(Cuboid::new(1.4, 0.08, 0.2));
    let marker = materials.add(Color::srgb(1.0, 0.94, 0.64));
    for (index, (vehicle, _)) in session.world().committed_pose_sources().enumerate() {
        let entity = session
            .vehicle_entity(vehicle)
            .expect("every vehicle is bound");
        let color = [Color::srgb(0.95, 0.4, 0.12), Color::srgb(0.12, 0.55, 0.95)][index % 2];
        commands
            .entity(entity)
            .insert(Visibility::default())
            .with_children(|children| {
                // 代理位于前保险杠；尺寸、抬升与车头标记只放在表现子实体。
                children.spawn((
                    Mesh3d(body.clone()),
                    MeshMaterial3d(materials.add(color)),
                    Transform::from_xyz(0.0, 0.42, length * 0.5),
                ));
                children.spawn((
                    Mesh3d(nose.clone()),
                    MeshMaterial3d(marker.clone()),
                    Transform::from_xyz(0.0, 0.86, 0.15),
                ));
            });
    }
}

fn ribbon(points: &[Vec3]) -> Mesh {
    let mut positions = Vec::with_capacity(points.len() * 2);
    let mut indices = Vec::with_capacity((points.len() - 1) * 6);
    for (index, point) in points.iter().enumerate() {
        let before = points[index.saturating_sub(1)];
        let after = points[(index + 1).min(points.len() - 1)];
        let side = (after - before).normalize().cross(Vec3::Y) * 1.75;
        positions.extend([(*point - side).to_array(), (*point + side).to_array()]);
        if index + 1 < points.len() {
            let base = index as u32 * 2;
            indices.extend([base, base + 1, base + 2, base + 1, base + 3, base + 2]);
        }
    }
    Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        vec![[0.0, 1.0, 0.0]; points.len() * 2],
    )
    .with_inserted_indices(Indices::U32(indices))
}
