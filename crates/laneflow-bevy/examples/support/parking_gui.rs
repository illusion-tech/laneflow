use bevy::{
    camera::visibility::VisibilitySystems,
    input::mouse::{AccumulatedMouseScroll, MouseScrollPixelsPerLine},
    mesh::Indices,
    prelude::*,
    window::PrimaryWindow,
};
use laneflow_bevy::LaneFlowSession;
use laneflow_runtime::ParkingSpaceState;
use laneflow_static_contract::ParkingSpaceOrdinal;

use super::{
    corridor_camera::Orbit,
    parking_host::{
        self as host, Fleet, Present, Presentation, Summary, VehicleModel, VehicleSlot,
    },
    parking_scene::{Role, Scene},
    parking_script::Script,
};

const FOV: f32 = std::f32::consts::FRAC_PI_4;
const VACANT: Color = Color::srgb(0.20, 0.62, 0.34);
const RESERVED: Color = Color::srgb(0.90, 0.62, 0.12);
const OCCUPIED: Color = Color::srgb(0.78, 0.20, 0.18);

#[derive(Component)]
pub(super) struct Hud;

#[derive(Component)]
struct SpacePad {
    space: ParkingSpaceOrdinal,
    material: Handle<StandardMaterial>,
}

struct PadSpec {
    space: ParkingSpaceOrdinal,
    transform: Transform,
    length: f32,
    width: f32,
}

#[derive(Resource)]
struct Layout {
    ribbon: Mesh,
    pads: Vec<PadSpec>,
    entry: Transform,
    exit: Transform,
    min: Vec3,
    max: Vec3,
}

/// 启动前从场景取出只用于画面的几何；泊位位姿不是表现正本。
pub(super) struct SceneLayout(Layout);

pub(super) fn layout_from_scene(scene: &Scene) -> Result<SceneLayout, String> {
    let mut pads = Vec::with_capacity(scene.pads.len());
    for pad in &scene.pads {
        pads.push(PadSpec {
            space: pad.space,
            transform: host::transform_from_pose(pad.pose)?,
            length: pad.length,
            width: pad.width,
        });
    }
    Ok(SceneLayout(Layout {
        ribbon: ribbon_from_scene(scene),
        pads,
        entry: host::transform_from_pose(scene.garage_entry)?,
        exit: host::transform_from_pose(scene.garage_exit)?,
        min: scene.min,
        max: scene.max,
    }))
}

fn ribbon_from_scene(scene: &Scene) -> Mesh {
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

pub(super) fn install(app: &mut App, layout: SceneLayout) {
    app.insert_resource(ClearColor(Color::srgb(0.055, 0.08, 0.105)))
        .insert_resource(layout.0)
        .add_systems(Startup, setup)
        .add_systems(Update, camera_input)
        .add_systems(
            PostUpdate,
            (update_pads, update_hud)
                .chain()
                .after(Present)
                .before(bevy::ui::UiSystems::Prepare),
        );
    install_visibility(app);
}

/// 必须先于可见性传播，否则这一帧的显示与隐藏要到下一帧才进入渲染。
fn install_visibility(app: &mut App) {
    app.add_systems(
        PostUpdate,
        apply_visibility
            .after(Present)
            .before(VisibilitySystems::VisibilityPropagate),
    );
}

fn role_color(role: Role) -> Color {
    match role {
        Role::Explicit => Color::srgb(0.15, 0.73, 0.97),
        Role::Virtual => Color::srgb(0.98, 0.48, 0.16),
        Role::Blocker => Color::srgb(0.90, 0.84, 0.38),
        Role::Removal => Color::srgb(0.63, 0.50, 0.96),
        Role::Through => Color::srgb(0.82, 0.85, 0.88),
    }
}

#[allow(clippy::too_many_arguments)]
fn setup(
    mut commands: Commands,
    layout: Res<Layout>,
    fleet: Res<Fleet>,
    script: Res<Script>,
    models: Query<(Entity, &ChildOf), With<VehicleModel>>,
    slots: Query<&VehicleSlot>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let extent = (layout.max - layout.min).max_element();
    let center = (layout.min + layout.max) * 0.5;
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
        Mesh3d(meshes.add(layout.ribbon.clone())),
        MeshMaterial3d(materials.add(Color::srgb(0.20, 0.23, 0.27))),
    ));
    for pad in &layout.pads {
        let material = materials.add(VACANT);
        // 位姿在泊位前端，车身向后延伸；矩形中心相应后移半个泊位。
        let transform = pad.transform * Transform::from_xyz(0.0, 0.02, pad.length * 0.5);
        commands.spawn((
            SpacePad {
                space: pad.space,
                material: material.clone(),
            },
            Mesh3d(meshes.add(Cuboid::new(pad.width, 0.04, pad.length))),
            MeshMaterial3d(material),
            transform,
        ));
    }
    let marker = meshes.add(Cuboid::new(1.2, 1.6, 1.2));
    for (transform, color) in [
        (layout.entry, Color::srgb(0.10, 0.85, 0.90)),
        (layout.exit, Color::srgb(0.90, 0.20, 0.80)),
    ] {
        // 虚拟池没有几何，只标出入口和出口，放在车道外侧不挡车。
        commands.spawn((
            Mesh3d(marker.clone()),
            MeshMaterial3d(materials.add(color)),
            transform * Transform::from_xyz(3.5, 0.8, 0.0),
        ));
    }
    let mut orbit = Orbit::new(layout.min, layout.max);
    orbit.distance = extent * 1.15;
    // 环路整体下移，让出左上角的 HUD。
    orbit.focus.z -= 12.0;
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: FOV,
            near: 0.5,
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
    for &entity in &fleet.entities {
        // 启动时全部隐藏，首次收到位姿才显示，不闪现初始 Transform。
        commands.entity(entity).insert(Visibility::Hidden);
    }
    for (model, parent) in &models {
        let slot = slots.get(parent.parent()).expect("车辆模型的父实体").0;
        let color = role_color(script.actors[slot].role);
        commands.entity(model).insert((
            Mesh3d(body.clone()),
            MeshMaterial3d(materials.add(color)),
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
            font_size: FontSize::Px(18.0),
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

fn apply_visibility(
    presentation: Res<Presentation>,
    mut vehicles: Query<(&VehicleSlot, &mut Visibility)>,
) {
    for (slot, mut visibility) in &mut vehicles {
        let target = if presentation.visible[slot.0] {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        visibility.set_if_neq(target);
    }
}

fn update_pads(
    session: Res<LaneFlowSession>,
    pads: Query<&SpacePad>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for pad in &pads {
        let color = match session.world().parking_space_state(pad.space) {
            Some(ParkingSpaceState::Vacant) | None => VACANT,
            Some(ParkingSpaceState::Reserved(_)) => RESERVED,
            Some(ParkingSpaceState::Occupied(_)) => OCCUPIED,
        };
        let unchanged = materials
            .get(&pad.material)
            .is_some_and(|material| material.base_color == color);
        if !unchanged && let Some(mut material) = materials.get_mut(&pad.material) {
            material.base_color = color;
        }
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bevy::{
        camera::visibility::VisibilityPlugin,
        ecs::schedule::Schedules,
        mesh::skinning::SkinnedMeshInverseBindposes,
        time::{TimePlugin, TimeUpdateStrategy},
        transform::TransformPlugin,
    };

    use super::super::parking_scene::{self, SceneConfig};
    use super::*;

    #[test]
    fn visibility_and_hud_follow_the_summary_in_the_same_frame() {
        let mut app = App::new();
        app.add_plugins((
            TimePlugin,
            TransformPlugin,
            AssetPlugin::default(),
            VisibilityPlugin,
        ))
        .init_asset::<Mesh>()
        .init_asset::<SkinnedMeshInverseBindposes>()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            16,
        )));
        let scene = parking_scene::build(SceneConfig {
            laps: 2,
            dwell_ticks: 30,
        })
        .unwrap();
        host::install(&mut app, scene).unwrap();
        install_visibility(&mut app);
        app.add_systems(
            PostUpdate,
            update_hud
                .after(Present)
                .before(bevy::ui::UiSystems::Prepare),
        );
        let hud = app.world_mut().spawn((Text::new(""), Hud)).id();
        let root = app.world().resource::<Fleet>().root;
        let entities = app.world().resource::<Fleet>().entities.clone();
        app.world_mut()
            .entity_mut(root)
            .insert(Visibility::default());
        for &entity in &entities {
            app.world_mut()
                .entity_mut(entity)
                .insert(Visibility::Hidden);
        }
        let mut changes = 0;
        let mut previous = vec![false; entities.len()];
        for _ in 0..1_500 {
            app.update();
            let world = app.world();
            assert!(world.resource::<host::Health>().failure.is_none());
            let visible = &world.resource::<Presentation>().visible;
            for (slot, &entity) in entities.iter().enumerate() {
                // 被移除的实体已销毁；其余实体本帧必须已传播到位。
                if let Some(inherited) = world.get::<InheritedVisibility>(entity) {
                    assert_eq!(inherited.get(), visible[slot]);
                }
            }
            changes += usize::from(previous != *visible);
            previous.clone_from(visible);
            let summary = world.resource::<Summary>().current.as_ref().unwrap();
            assert_eq!(world.get::<Text>(hud).unwrap().0, summary.hud());
        }
        // 至少包含首帧显示、虚拟车隐藏和重现。
        assert!(changes >= 3, "changes={changes}");
    }

    /// 上一个测试只覆盖调度器恰好选中的一种顺序；反向约束成环才说明顺序是强制的。
    #[test]
    fn apply_visibility_is_ordered_before_visibility_propagation() {
        let mut ordered = App::new();
        install_visibility(&mut ordered);
        let mut reversed = App::new();
        install_visibility(&mut reversed);
        reversed.configure_sets(
            PostUpdate,
            VisibilitySystems::VisibilityPropagate.before(apply_visibility),
        );
        let build = |app: &mut App| {
            let world = app.world_mut();
            let mut schedule = world
                .resource_mut::<Schedules>()
                .remove(PostUpdate)
                .unwrap();
            schedule.initialize(world).is_ok()
        };
        assert!(build(&mut ordered));
        assert!(!build(&mut reversed));
    }
}
