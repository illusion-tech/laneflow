use bevy::{
    input::mouse::{AccumulatedMouseScroll, MouseScrollPixelsPerLine},
    mesh::Indices,
    prelude::*,
    window::PrimaryWindow,
};
use laneflow_static_contract::{SignalAspect, SignalGroupOrdinal};

use super::{
    corridor_camera::Orbit,
    corridor_host::{Present, Proxies, Summary},
    corridor_scene::Geometry,
};

#[derive(Component)]
pub(super) struct Hud;

#[derive(Component)]
pub(super) struct Lamp {
    pub group: SignalGroupOrdinal,
    pub material: Handle<StandardMaterial>,
    pub aspect: Option<SignalAspect>,
}

pub(super) fn install(app: &mut App) {
    app.init_asset::<bevy::light::atmosphere::ScatteringMedium>()
        .insert_resource(ClearColor(Color::srgb(0.055, 0.08, 0.105)))
        .add_systems(Startup, setup)
        .add_systems(Update, camera_input)
        .add_systems(
            PostUpdate,
            update_display
                .after(Present)
                .before(bevy::ui::UiSystems::Prepare),
        );
}

fn setup(
    mut commands: Commands,
    geometry: Res<Geometry>,
    proxies: Res<Proxies>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let road = materials.add(Color::srgb(0.16, 0.19, 0.23));
    let white = materials.add(StandardMaterial {
        base_color: Color::srgb(0.72, 0.78, 0.81),
        unlit: true,
        ..default()
    });
    let ground = materials.add(Color::srgb(0.12, 0.20, 0.18));
    let extent = (geometry.max - geometry.min).max_element();
    let center = (geometry.min + geometry.max) * 0.5;
    commands.spawn((
        Mesh3d(
            meshes.add(
                Plane3d::default()
                    .mesh()
                    .size(extent + 200.0, extent + 200.0),
            ),
        ),
        MeshMaterial3d(ground),
        Transform::from_translation(center - Vec3::Y * 0.05),
    ));
    for points in &geometry.lanes {
        commands.spawn((
            Mesh3d(meshes.add(ribbon(points, geometry.lane_width))),
            MeshMaterial3d(road.clone()),
        ));
        for pair in points.windows(2) {
            let direction = (pair[1] - pair[0]).normalize();
            let side = direction.cross(Vec3::Y) * (geometry.lane_width * 0.5 - 0.12);
            let length = pair[0].distance(pair[1]);
            let mut offset = 0.0;
            while offset + 2.0 < length {
                let dash = (length - offset).min(4.0);
                commands.spawn((
                    Mesh3d(meshes.add(Cuboid::new(0.12, 0.03, dash))),
                    MeshMaterial3d(white.clone()),
                    Transform::from_translation(
                        pair[0] + direction * (offset + dash * 0.5) + side + Vec3::Y * 0.035,
                    )
                    .looking_to(direction, Vec3::Y),
                ));
                offset += 12.0;
            }
        }
    }
    let orbit = Orbit::new(&geometry);
    commands.spawn((Camera3d::default(), orbit.transform(), orbit));
    commands.spawn((
        DirectionalLight {
            illuminance: 15_000.0,
            ..default()
        },
        Transform::from_xyz(-100.0, 300.0, 180.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.entity(proxies.root).insert(Visibility::default());
    let body = meshes.add(Cuboid::new(1.8, 1.2, proxies.vehicle_length));
    let roof = meshes.add(Cuboid::new(1.45, 0.25, proxies.vehicle_length * 0.45));
    let nose = meshes.add(Cuboid::new(1.7, 0.1, 0.18));
    let glass = materials.add(Color::srgb(0.055, 0.12, 0.19));
    let headlight = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.95, 0.68),
        emissive: LinearRgba::rgb(1.4, 1.3, 0.8),
        ..default()
    });
    let palette: Vec<_> = [
        (0.15, 0.73, 0.97),
        (0.98, 0.48, 0.16),
        (0.90, 0.84, 0.38),
        (0.63, 0.50, 0.96),
    ]
    .into_iter()
    .map(|(r, g, b)| materials.add(Color::srgb(r, g, b)))
    .collect();
    for (slot, (&entity, &model)) in proxies.entities.iter().zip(&proxies.models).enumerate() {
        commands.entity(entity).insert(Visibility::default());
        commands.entity(model).insert((
            Mesh3d(body.clone()),
            MeshMaterial3d(palette[slot % palette.len()].clone()),
            Visibility::default(),
        ));
        commands.entity(entity).with_children(|children| {
            children.spawn((
                Mesh3d(roof.clone()),
                MeshMaterial3d(glass.clone()),
                Transform::from_xyz(0.0, 1.38, proxies.vehicle_length * 0.54),
            ));
            children.spawn((
                Mesh3d(nose.clone()),
                MeshMaterial3d(headlight.clone()),
                Transform::from_xyz(0.0, 0.9, 0.10),
            ));
        });
    }
    let housing = meshes.add(Cuboid::new(1.3, 0.35, 1.3));
    let lens = meshes.add(Sphere::new(0.52));
    let black = materials.add(Color::srgb(0.015, 0.02, 0.025));
    for lamp in &geometry.lamps {
        let position = lamp.position + lamp.tangent.cross(Vec3::Y) * (geometry.lane_width * 0.30);
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(geometry.lane_width - 0.2, 0.04, 0.4))),
            MeshMaterial3d(white.clone()),
            Transform::from_translation(lamp.position + Vec3::Y * 0.05)
                .looking_to(lamp.tangent, Vec3::Y),
        ));
        commands.spawn((
            Mesh3d(meshes.add(Cylinder::new(0.10, 3.5))),
            MeshMaterial3d(black.clone()),
            Transform::from_translation(position + Vec3::Y * 1.75),
        ));
        commands.spawn((
            Mesh3d(housing.clone()),
            MeshMaterial3d(black.clone()),
            Transform::from_translation(position + Vec3::Y * 3.4),
        ));
        let material = materials.add(lamp_material(SignalAspect::Red));
        commands.spawn((
            Mesh3d(lens.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(position + Vec3::Y * 3.85),
            Lamp {
                group: lamp.group,
                material,
                aspect: None,
            },
        ));
    }
    commands.spawn((
        Text::new(String::new()),
        Hud,
        TextFont {
            font_size: FontSize::Px(20.0),
            ..default()
        },
        TextColor(Color::srgb(0.88, 0.94, 0.98)),
        Node {
            position_type: PositionType::Absolute,
            left: px(18),
            top: px(16),
            padding: UiRect::all(px(14)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.025, 0.045, 0.065, 0.93)),
    ));
}

fn camera_input(
    mut cursor: MessageReader<CursorMoved>,
    primary: Query<Entity, With<PrimaryWindow>>,
    scroll: Res<AccumulatedMouseScroll>,
    conversion: Res<MouseScrollPixelsPerLine>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut cameras: Query<(&mut Orbit, &mut Transform)>,
) {
    // 本例不锁定指针；拖拽使用窗口逻辑像素，而非设备 raw motion。
    // Windows 绝对定位设备/注入输入的 raw motion 不一定是相对像素。
    let window = primary.single().ok();
    let motion: Vec2 = cursor
        .read()
        .filter(|event| Some(event.window) == window)
        .filter_map(|event| event.delta)
        .sum();
    for (mut orbit, mut transform) in &mut cameras {
        let pan = if buttons.pressed(MouseButton::Left) {
            motion
        } else {
            Vec2::ZERO
        };
        let rotate = if buttons.pressed(MouseButton::Right) {
            motion
        } else {
            Vec2::ZERO
        };
        orbit.apply(pan, rotate, scroll.to_lines(&conversion).delta.y);
        *transform = orbit.transform();
    }
}

pub(super) fn update_display(
    summary: Res<Summary>,
    mut text: Query<&mut Text, With<Hud>>,
    mut lamps: Query<&mut Lamp>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Some(summary) = &summary.current else {
        return;
    };
    for mut text in &mut text {
        text.0 = summary.hud();
    }
    for mut lamp in &mut lamps {
        let aspect = summary
            .signals
            .iter()
            .find(|(group, _)| *group == lamp.group)
            .map(|(_, aspect)| *aspect)
            .expect("startup validated committed signal group");
        if lamp.aspect != Some(aspect) {
            *materials
                .get_mut(&lamp.material)
                .expect("example-owned lamp material") = lamp_material(aspect);
            lamp.aspect = Some(aspect);
        }
    }
}

fn lamp_material(aspect: SignalAspect) -> StandardMaterial {
    let (r, g, b) = match aspect {
        SignalAspect::Red => (1.0, 0.05, 0.02),
        SignalAspect::Yellow => (1.0, 0.65, 0.03),
        SignalAspect::Green => (0.03, 0.95, 0.32),
        _ => unreachable!("unsupported committed signal aspect: {aspect:?}"),
    };
    StandardMaterial {
        base_color: Color::srgb(r, g, b),
        emissive: LinearRgba::rgb(r * 3.0, g * 3.0, b * 3.0),
        unlit: true,
        ..default()
    }
}

fn ribbon(points: &[Vec3], width: f32) -> Mesh {
    let mut positions = Vec::with_capacity(points.len() * 2);
    let mut indices = Vec::with_capacity((points.len() - 1) * 6);
    for (index, point) in points.iter().enumerate() {
        let before = points[index.saturating_sub(1)];
        let after = points[(index + 1).min(points.len() - 1)];
        let side = (after - before).normalize().cross(Vec3::Y) * width * 0.5;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{corridor_host, corridor_scene};
    use std::time::Duration;

    #[test]
    fn c12_real_text_and_independent_lamp_materials_follow_the_same_summary() {
        let mut app = App::new();
        app.add_plugins((bevy::time::TimePlugin, bevy::transform::TransformPlugin));
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            Duration::from_millis(128),
        ));
        corridor_host::install(
            &mut app,
            corridor_scene::build(
                laneflow_scenario::signalized_corridor::CorridorPopulationConfig::default(),
            )
            .unwrap(),
        )
        .unwrap();
        app.init_resource::<Assets<StandardMaterial>>();
        let hud = app.world_mut().spawn((Text::new(""), Hud)).id();
        let geometry = app.world().resource::<Geometry>().clone();
        for binding in &geometry.lamps {
            let material = app
                .world_mut()
                .resource_mut::<Assets<StandardMaterial>>()
                .add(lamp_material(SignalAspect::Red));
            app.world_mut().spawn(Lamp {
                group: binding.group,
                material,
                aspect: None,
            });
        }
        app.add_systems(PostUpdate, update_display.after(Present));
        let mut first = Vec::new();
        for frame in 0..90 {
            app.update();
            let summary = app
                .world()
                .resource::<Summary>()
                .current
                .as_ref()
                .unwrap()
                .clone();
            assert_eq!(app.world().get::<Text>(hud).unwrap().0, summary.hud());
            let mut lamps = app.world_mut().query::<&Lamp>();
            let values: Vec<_> = lamps
                .iter(app.world())
                .map(|lamp| (lamp.group, lamp.aspect, lamp.material.clone()))
                .collect();
            assert_eq!(values.len(), geometry.lamps.len());
            assert_eq!(
                values
                    .iter()
                    .map(|(_, _, material)| material.id())
                    .collect::<std::collections::HashSet<_>>()
                    .len(),
                values.len()
            );
            for (group, aspect, material) in &values {
                assert_eq!(
                    *aspect,
                    summary
                        .signals
                        .iter()
                        .find(|(candidate, _)| candidate == group)
                        .map(|(_, aspect)| *aspect)
                );
                let actual = app
                    .world()
                    .resource::<Assets<StandardMaterial>>()
                    .get(material)
                    .unwrap();
                let expected = lamp_material(aspect.unwrap());
                assert_eq!(actual.base_color, expected.base_color);
                assert_eq!(actual.emissive, expected.emissive);
            }
            let aspects: Vec<_> = values.iter().map(|(_, aspect, _)| *aspect).collect();
            if frame == 0 {
                first = aspects.clone();
            }
            if frame == 89 {
                assert_ne!(first, aspects, "test must cross a real phase boundary");
            }
        }
    }

    #[test]
    fn c13_camera_system_consumes_right_drag_left_drag_and_wheel_resources() {
        let geometry = corridor_scene::build(
            laneflow_scenario::signalized_corridor::CorridorPopulationConfig::default(),
        )
        .unwrap()
        .geometry;
        let mut app = App::new();
        app.add_message::<CursorMoved>()
            .init_resource::<AccumulatedMouseScroll>()
            .init_resource::<MouseScrollPixelsPerLine>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_systems(Update, camera_input);
        let window = app.world_mut().spawn(PrimaryWindow).id();
        let orbit = Orbit::new(&geometry);
        let initial = orbit.transform();
        let camera = app.world_mut().spawn((initial, orbit)).id();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Right);
        app.world_mut().write_message(CursorMoved {
            window,
            position: Vec2::new(160.0, 140.0),
            delta: Some(Vec2::new(60.0, 40.0)),
        });
        app.update();
        let rotated = *app.world().get::<Transform>(camera).unwrap();
        assert_ne!(rotated.rotation, initial.rotation);
        let focus = app.world().get::<Orbit>(camera).unwrap().focus;
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Right);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.world_mut().write_message(CursorMoved {
            window,
            position: Vec2::new(220.0, 180.0),
            delta: Some(Vec2::new(60.0, 40.0)),
        });
        app.update();
        assert_ne!(app.world().get::<Orbit>(camera).unwrap().focus, focus);
        let focus = app.world().get::<Orbit>(camera).unwrap().focus;
        let foreign = app.world_mut().spawn_empty().id();
        app.world_mut().write_message(CursorMoved {
            window: foreign,
            position: Vec2::splat(10_000.0),
            delta: Some(Vec2::splat(10_000.0)),
        });
        app.update();
        assert_eq!(app.world().get::<Orbit>(camera).unwrap().focus, focus);
        let distance = app.world().get::<Orbit>(camera).unwrap().distance;
        app.world_mut()
            .resource_mut::<AccumulatedMouseScroll>()
            .delta
            .y = 2.0;
        app.update();
        assert!(app.world().get::<Orbit>(camera).unwrap().distance < distance);
    }
}
