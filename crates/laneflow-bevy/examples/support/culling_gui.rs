use bevy::{
    camera::CameraProjection,
    input::mouse::{AccumulatedMouseScroll, MouseScrollPixelsPerLine},
    mesh::Indices,
    prelude::*,
    window::PrimaryWindow,
};

use super::{
    corridor_camera::Orbit,
    culling_cli::CameraPreset,
    culling_host::{Culling, Fleet, Present, Summary, VehicleModel, VehicleSlot, ViewControl},
    culling_scene::Scene,
    culling_view::ViewVolume,
};

/// 一格的宽度：生成器的格距减去 100 m 隔离。
const CELL_SPAN: f32 = 430.0;
const CELL_DISTANCE: f32 = 260.0;
const FOV: f32 = std::f32::consts::FRAC_PI_4;
const NEAR: f32 = 0.5;

#[derive(Component)]
pub(super) struct Hud;

#[derive(Resource)]
struct LaneRibbon(Mesh);

#[derive(Resource)]
struct Bounds {
    min: Vec3,
    max: Vec3,
    preset: CameraPreset,
}

pub(super) fn ribbon_from_scene(scene: &Scene) -> Mesh {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for points in &scene.lanes {
        let first = positions.len() as u32;
        for (index, point) in points.iter().enumerate() {
            let before = points[index.saturating_sub(1)];
            let after = points[(index + 1).min(points.len() - 1)];
            let side = (after - before).normalize().cross(Vec3::Y) * scene.lane_width * 0.5;
            positions.extend([(*point - side).to_array(), (*point + side).to_array()]);
            if index + 1 < points.len() {
                let base = first + index as u32 * 2;
                indices.extend([base, base + 1, base + 2, base + 1, base + 3, base + 2]);
            }
        }
    }
    let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(Indices::U32(indices))
}

pub(super) fn install(app: &mut App, ribbon: Mesh, bounds: (Vec3, Vec3), preset: CameraPreset) {
    app.insert_resource(ClearColor(Color::srgb(0.055, 0.08, 0.105)))
        .insert_resource(LaneRibbon(ribbon))
        .insert_resource(Bounds {
            min: bounds.0,
            max: bounds.1,
            preset,
        })
        .add_systems(Startup, setup)
        .add_systems(Update, (camera_input, update_view).chain())
        .add_systems(
            PostUpdate,
            (apply_visibility, update_hud)
                .chain()
                .after(Present)
                .before(bevy::ui::UiSystems::Prepare),
        );
}

fn grid_orbit(bounds: &Bounds) -> Orbit {
    Orbit::new(bounds.min, bounds.max)
}

/// 对准最小坐标角上的一格，任何格数下都能落在路网内。
fn cell_orbit(bounds: &Bounds) -> Orbit {
    let mut orbit = grid_orbit(bounds);
    orbit.focus = Vec3::new(
        bounds.min.x + CELL_SPAN * 0.5,
        0.0,
        bounds.min.z + CELL_SPAN * 0.5,
    );
    orbit.distance = CELL_DISTANCE;
    orbit
}

fn view_volume(transform: &Transform, window: &Window) -> ViewVolume {
    let clip_from_view = PerspectiveProjection {
        fov: FOV,
        near: NEAR,
        aspect_ratio: window.width() / window.height(),
        ..default()
    }
    .get_clip_from_view();
    ViewVolume::from_clip_from_world(clip_from_view * transform.to_matrix().inverse())
}

#[allow(clippy::too_many_arguments)]
fn setup(
    mut commands: Commands,
    bounds: Res<Bounds>,
    fleet: Res<Fleet>,
    ribbon: Res<LaneRibbon>,
    window: Query<&Window, With<PrimaryWindow>>,
    models: Query<(Entity, &ChildOf), With<VehicleModel>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut view: ResMut<ViewVolume>,
    mut control: ResMut<ViewControl>,
) {
    let extent = (bounds.max - bounds.min).max_element();
    let center = (bounds.min + bounds.max) * 0.5;
    commands.spawn((
        Mesh3d(
            meshes.add(
                Plane3d::default()
                    .mesh()
                    .size(extent + 400.0, extent + 400.0),
            ),
        ),
        MeshMaterial3d(materials.add(Color::srgb(0.12, 0.20, 0.18))),
        Transform::from_translation(center - Vec3::Y * 0.05),
    ));
    commands.spawn((
        Mesh3d(meshes.add(ribbon.0.clone())),
        MeshMaterial3d(materials.add(Color::srgb(0.20, 0.23, 0.27))),
    ));
    let orbit = match bounds.preset {
        CameraPreset::Cell => cell_orbit(&bounds),
        CameraPreset::Grid | CameraPreset::Frozen => grid_orbit(&bounds),
    };
    if bounds.preset == CameraPreset::Frozen {
        let window = window.single().expect("primary window");
        *view = view_volume(&cell_orbit(&bounds).transform(), window);
        control.frozen = true;
    }
    commands.spawn((
        Camera3d::default(),
        // 默认远平面只有 1 km，整张网格会被渲染器裁掉。
        Projection::Perspective(PerspectiveProjection {
            fov: FOV,
            near: NEAR,
            far: extent * 8.0,
            ..default()
        }),
        orbit.transform(),
        orbit,
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 15_000.0,
            ..default()
        },
        Transform::from_xyz(-100.0, 300.0, 180.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.entity(fleet.root).insert(Visibility::default());
    let length = fleet.vehicle_length;
    let body = meshes.add(Cuboid::new(1.8, 1.2, length));
    let nose = meshes.add(Cuboid::new(1.7, 0.1, 0.18));
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
    for &entity in &fleet.entities {
        // 启动时全部隐藏，首次收到位姿才显示，不闪现初始 Transform。
        commands.entity(entity).insert(Visibility::Hidden);
    }
    for (index, (model, parent)) in models.iter().enumerate() {
        commands.entity(model).insert((
            Mesh3d(body.clone()),
            MeshMaterial3d(palette[index % palette.len()].clone()),
            Visibility::default(),
        ));
        commands.entity(parent.parent()).with_children(|children| {
            children.spawn((
                Mesh3d(nose.clone()),
                MeshMaterial3d(headlight.clone()),
                Transform::from_xyz(0.0, 0.9, 0.10),
            ));
        });
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
            // 放在右下角，不遮挡对准最小坐标角的那一格。
            right: px(18),
            bottom: px(16),
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

fn update_view(
    keys: Res<ButtonInput<KeyCode>>,
    mut control: ResMut<ViewControl>,
    mut view: ResMut<ViewVolume>,
    window: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<&Transform, With<Orbit>>,
) {
    if keys.just_pressed(KeyCode::KeyF) {
        control.frozen = !control.frozen;
    }
    if control.frozen {
        return;
    }
    if let (Ok(window), Ok(transform)) = (window.single(), cameras.single()) {
        *view = view_volume(transform, window);
    }
}

fn apply_visibility(culling: Res<Culling>, mut vehicles: Query<(&VehicleSlot, &mut Visibility)>) {
    for (slot, mut visibility) in &mut vehicles {
        let target = if culling.visible[slot.0] {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        visibility.set_if_neq(target);
    }
}

pub(super) fn update_hud(summary: Res<Summary>, mut text: Query<&mut Text, With<Hud>>) {
    let Some(summary) = &summary.current else {
        return;
    };
    for mut text in &mut text {
        text.0 = summary.hud();
    }
}
