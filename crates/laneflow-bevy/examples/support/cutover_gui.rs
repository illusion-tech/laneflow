use bevy::{
    camera::visibility::{NoFrustumCulling, VisibilitySystems},
    input::mouse::{AccumulatedMouseScroll, MouseScrollPixelsPerLine},
    mesh::Indices,
    prelude::*,
    window::PrimaryWindow,
};

use super::{
    corridor_camera::Orbit,
    cutover_host::{
        Cutover, Fleet, Present, Presentation, RoadData, SPUR_SLOT, Summary, VehicleModel,
        VehicleSlot,
    },
};

const FOV: f32 = std::f32::consts::FRAC_PI_4;

#[derive(Component)]
pub(super) struct Hud;

/// 路面实体记录已构建的版本，版本落后时替换网格。
#[derive(Component)]
struct RoadSurface {
    version: u64,
}

/// 两个修订路面的并集，只用于初始视角。
#[derive(Resource)]
struct Bounds {
    min: Vec3,
    max: Vec3,
}

fn ribbon(road: &RoadData) -> Mesh {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for points in &road.lanes {
        let first = positions.len() as u32;
        for (index, point) in points.iter().enumerate() {
            let before = points[index.saturating_sub(1)];
            let after = points[(index + 1).min(points.len() - 1)];
            let side = (after - before).normalize().cross(Vec3::Y) * road.lane_width * 0.5;
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

pub(super) fn install(app: &mut App, min: Vec3, max: Vec3) {
    app.insert_resource(ClearColor(Color::srgb(0.055, 0.08, 0.105)))
        .insert_resource(Bounds { min, max })
        .add_systems(Startup, setup)
        .add_systems(Update, (camera_input, request_on_key))
        .add_systems(
            PostUpdate,
            (update_road, update_hud)
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

fn slot_color(slot: usize) -> Color {
    if slot == SPUR_SLOT {
        Color::srgb(0.98, 0.48, 0.16)
    } else {
        Color::srgb(0.82, 0.85, 0.88)
    }
}

#[allow(clippy::too_many_arguments)]
fn setup(
    mut commands: Commands,
    bounds: Res<Bounds>,
    road: Res<RoadData>,
    fleet: Res<Fleet>,
    models: Query<(Entity, &ChildOf), With<VehicleModel>>,
    slots: Query<&VehicleSlot>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
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
    // 切换后路面范围会变，旧包围盒不能决定是否剔除。
    commands.spawn((
        RoadSurface {
            version: road.version,
        },
        Mesh3d(meshes.add(ribbon(&road))),
        MeshMaterial3d(materials.add(Color::srgb(0.20, 0.23, 0.27))),
        NoFrustumCulling,
    ));
    let mut orbit = Orbit::new(bounds.min, bounds.max);
    orbit.distance = extent * 1.15;
    // 路面整体右移，让出左上角的 HUD。
    orbit.focus.x -= 28.0;
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
        commands.entity(model).insert((
            Mesh3d(body.clone()),
            MeshMaterial3d(materials.add(slot_color(slot))),
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

/// 请求在下一帧的行政阶段执行；请求在途时按键被忽略。
fn request_on_key(keys: Res<ButtonInput<KeyCode>>, mut cutover: ResMut<Cutover>) {
    if keys.just_pressed(KeyCode::KeyC) {
        cutover.request();
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

fn update_road(
    road: Res<RoadData>,
    mut surfaces: Query<(&mut RoadSurface, &mut Mesh3d)>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    for (mut surface, mut mesh) in &mut surfaces {
        if surface.version != road.version {
            mesh.0 = meshes.add(ribbon(&road));
            surface.version = road.version;
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

    use super::super::{cutover_host as host, cutover_scene as scene};
    use super::*;

    #[test]
    fn visibility_road_and_hud_follow_the_summary_in_the_same_frame() {
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
        host::install(&mut app, scene::build().unwrap(), vec![60, 400, 800, 1_200]).unwrap();
        install_visibility(&mut app);
        app.add_systems(
            PostUpdate,
            (update_road, update_hud)
                .chain()
                .after(Present)
                .before(bevy::ui::UiSystems::Prepare),
        );
        let hud = app.world_mut().spawn((Text::new(""), Hud)).id();
        let surface = app
            .world_mut()
            .spawn((RoadSurface { version: 0 }, Mesh3d(Handle::default())))
            .id();
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
        let mut previous = false;
        for _ in 0..1_500 {
            app.update();
            let world = app.world();
            assert!(world.resource::<host::Health>().failure.is_none());
            let visible = &world.resource::<Presentation>().visible;
            for (slot, &entity) in entities.iter().enumerate() {
                let inherited = world.get::<InheritedVisibility>(entity).unwrap();
                assert_eq!(inherited.get(), visible[slot]);
            }
            changes += usize::from(previous != visible[SPUR_SLOT]);
            previous = visible[SPUR_SLOT];
            let road = world.resource::<RoadData>();
            assert_eq!(
                world.get::<RoadSurface>(surface).unwrap().version,
                road.version
            );
            let summary = world.resource::<Summary>().current.as_ref().unwrap();
            assert_eq!(summary.revision_id, road.revision);
            assert_eq!(world.get::<Text>(hud).unwrap().0, summary.hud());
        }
        // 两次进入 `R2` 和两次回切，每次都改变代理实体的可见性。
        assert_eq!(changes, 4);
        assert_eq!(app.world().resource::<Cutover>().successes, 4);
    }

    #[test]
    fn key_press_requests_a_cutover_for_the_next_frame() {
        let mut app = App::new();
        app.add_plugins(TimePlugin)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO))
            .init_resource::<ButtonInput<KeyCode>>()
            .add_systems(Update, request_on_key);
        host::install(&mut app, scene::build().unwrap(), Vec::new()).unwrap();
        app.update();
        assert_eq!(app.world().resource::<Cutover>().pending, None);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyC);
        app.update();
        let target = Some(scene::RevisionName::R2);
        assert_eq!(app.world().resource::<Cutover>().pending, target);
        assert_eq!(app.world().resource::<Cutover>().successes, 0);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
        app.update();
        let cutover = app.world().resource::<Cutover>();
        assert_eq!(cutover.current, scene::RevisionName::R2);
        assert_eq!(cutover.pending, None);
        assert_eq!(cutover.successes, 1);
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
