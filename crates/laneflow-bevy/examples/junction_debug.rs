//! 复杂路口 Bevy 调试示例（#285 阶段二 §4）：检入 catalog 0.1 + LFCA 的四车
//! 确定性场景 + 默认关闭的 F3 调试 overlay（mesh/文本，无 Gizmos）。
//!
//! overlay 全部只读 `LaneFlowSession` 并写独立 ECS 实体，不触碰 Runtime 状态；
//! 开/关 overlay 在相同输入下的 Runtime 摘要一致性由无窗口 smoke 对拍把关。
//! GUI 不进 CI（bevy-reference-adapter §9）。

#[path = "support/junction_debug_scene.rs"]
mod junction_debug_scene;

use std::{collections::HashMap, error::Error, fmt::Write as _};

use bevy::{mesh::Indices, prelude::*};
use laneflow_bevy::{
    LaneFlowCommittedPoseBatch, LaneFlowFixed, LaneFlowFixedSet, LaneFlowPlugin, LaneFlowSession,
};
use laneflow_runtime::{VehicleHandle, VehicleStatus, WorldGeneration};
use laneflow_spatial::{CanonicalPoseF32, FramePlacementToken};
use laneflow_static_contract::{
    ConflictZoneOrdinal, EntityKind, NetworkRevisionId, SignalAspect, SignalGroupOrdinal,
    WaitingZoneOrdinal,
};
use laneflow_static_network::{CanonicalPoint, SharedNetworkRevision};

/// 调试 ribbon 的车道宽度近似（米）；精确车道宽不在 catalog 线格式内。
const LANE_WIDTH_METERS: f32 = 3.5;
/// 车辆 box 宽度近似（米）；车型线格式只有车长。
const VEHICLE_WIDTH_METERS: f32 = 1.8;
const VEHICLE_HEIGHT_METERS: f32 = 1.5;

fn main() -> Result<(), Box<dyn Error>> {
    let scene = junction_debug_scene::build()?;
    App::new()
        .add_plugins((DefaultPlugins, LaneFlowPlugin))
        .insert_resource(scene.session)
        .insert_resource(SpawnedPlan(scene.spawned))
        .init_resource::<JunctionDebugConfig>()
        .init_resource::<PoseBuffer>()
        .init_resource::<JunctionDebugPanel>()
        .init_resource::<StaticOverlayCache>()
        .add_systems(Startup, setup_scene)
        .add_systems(
            Update,
            (
                toggle_overlay,
                sync_vehicle_transforms,
                maintain_static_overlay,
                apply_overlay_visuals.after(sync_vehicle_transforms),
            ),
        )
        .add_systems(
            LaneFlowFixed,
            observe_overlay.in_set(LaneFlowFixedSet::Observe),
        )
        .run();
    Ok(())
}

#[derive(Resource, Default)]
struct JunctionDebugConfig {
    enabled: bool,
}

#[derive(Resource)]
struct SpawnedPlan(Box<[junction_debug_scene::SpawnedVehicle]>);

/// 跨帧复用的提取缓冲（adapter-api §6 稳定容量合同）。
#[derive(Resource, Default)]
struct PoseBuffer(LaneFlowCommittedPoseBatch);

/// 每拍由 `observe_overlay` 重写的面板内容；`selected` 供高亮标记定位，
/// `signals` 是最近一拍信号组指示（group raw + aspect）。
#[derive(Resource, Default)]
struct JunctionDebugPanel {
    content: String,
    selected: Option<VehicleHandle>,
    signals: Vec<(u32, SignalAspect)>,
}

/// 车辆表现几何：Runtime pose 是前保险杠位置，表现中心沿切向后移半个车长。
#[derive(Resource)]
struct VehicleMetrics {
    half_length: f32,
}

/// 静态绘制缓存键：换根或换世代后重建；`None` 表示当前无静态标记。
#[derive(Resource, Default)]
struct StaticOverlayCache {
    key: Option<(NetworkRevisionId, WorldGeneration)>,
}

#[derive(Component)]
struct VehicleBox;

#[derive(Component)]
struct DebugStaticRoot;

#[derive(Component)]
struct SignalDotMarker {
    group: u32,
    red: Handle<StandardMaterial>,
    yellow: Handle<StandardMaterial>,
    green: Handle<StandardMaterial>,
    unknown: Handle<StandardMaterial>,
}

#[derive(Component)]
struct SelectedVehicleMarker;

#[derive(Component)]
struct PanelText;

fn setup_scene(
    mut commands: Commands,
    mut session: ResMut<LaneFlowSession>,
    plan: Res<SpawnedPlan>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 330.0, 330.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        PointLight {
            intensity: 2_000_000.0,
            range: 1_200.0,
            ..default()
        },
        Transform::from_xyz(80.0, 260.0, 120.0),
    ));
    let road_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.22, 0.23, 0.25),
        double_sided: true,
        ..default()
    });
    let revision = session.world().revision();
    if let Some(lane_pose) = revision.spatial().and_then(|spatial| spatial.lane_pose()) {
        for index in 0..lane_pose.lane_edge_count() {
            let Some(geometry) =
                lane_pose.lane_geometry(laneflow_static_contract::LaneEdgeOrdinal::from_raw(index))
            else {
                continue;
            };
            let points: Vec<Vec3> = geometry.points().iter().map(point_to_vec3).collect();
            let Some(mesh) = ribbon_mesh(&points, LANE_WIDTH_METERS, 0.05, false) else {
                continue;
            };
            commands.spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(road_material.clone()),
            ));
        }
    }
    let profile_length_mm = session
        .world()
        .traffic()
        .relations()
        .vehicle_profile(laneflow_static_contract::VehicleProfileOrdinal::from_raw(0))
        .map_or(4_500, |profile| profile.length_mm());
    commands.insert_resource(VehicleMetrics {
        half_length: (profile_length_mm as f32) / 1_000.0 / 2.0,
    });
    let vehicle_mesh = meshes.add(Cuboid::new(
        VEHICLE_WIDTH_METERS,
        VEHICLE_HEIGHT_METERS,
        (profile_length_mm as f32) / 1_000.0,
    ));
    let vehicle_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.85, 0.85, 0.9),
        ..default()
    });
    for spawned in plan.0.iter() {
        let entity = commands
            .spawn((
                Mesh3d(vehicle_mesh.clone()),
                MeshMaterial3d(vehicle_material.clone()),
                Transform::IDENTITY,
                VehicleBox,
            ))
            .id();
        if let Err(error) = session.bind_vehicle_entity(spawned.vehicle, entity) {
            warn!("bind_vehicle_entity 失败: {error:?}");
        }
    }
    commands.spawn((
        Text::new(String::new()),
        TextFont::from_font_size(14.0),
        TextColor(Color::srgb(0.92, 0.92, 0.95)),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(12.0),
            top: Val::Px(12.0),
            ..default()
        },
        PanelText,
    ));
}

fn point_to_vec3(point: &CanonicalPoint) -> Vec3 {
    Vec3::new(point.x, point.y, point.z)
}

fn pose_transform(pose: CanonicalPoseF32) -> Transform {
    let position = pose.position();
    let tangent = pose.tangent();
    let up = pose.up();
    Transform::from_xyz(position.x(), position.y(), position.z()).looking_to(
        Vec3::new(tangent.x(), tangent.y(), tangent.z()),
        Vec3::new(up.x(), up.y(), up.z()),
    )
}

/// 采样点折线的三角带 ribbon；`closed` 时把首点追加到尾部闭合。
fn ribbon_mesh(points: &[Vec3], width: f32, y_lift: f32, closed: bool) -> Option<Mesh> {
    if points.len() < 2 {
        return None;
    }
    let mut line: Vec<Vec3> = points.to_vec();
    if closed {
        line.push(points[0]);
    }
    let half = width * 0.5;
    let mut positions = Vec::with_capacity(line.len() * 2);
    for (index, point) in line.iter().enumerate() {
        let previous = line[index.saturating_sub(1)];
        let next = line[(index + 1).min(line.len() - 1)];
        let direction = (next - previous).normalize_or_zero();
        let lateral = Vec3::new(-direction.z, 0.0, direction.x) * half;
        let base = *point + Vec3::Y * y_lift;
        positions.push((base - lateral).to_array());
        positions.push((base + lateral).to_array());
    }
    let mut indices = Vec::with_capacity((line.len() - 1) * 6);
    for index in 0..line.len() - 1 {
        let base = (index as u32) * 2;
        indices.extend_from_slice(&[base, base + 1, base + 2, base + 1, base + 3, base + 2]);
    }
    let mut mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_indices(Indices::U32(indices));
    mesh.compute_normals();
    Some(mesh)
}

fn toggle_overlay(keys: Res<ButtonInput<KeyCode>>, mut config: ResMut<JunctionDebugConfig>) {
    if keys.just_pressed(KeyCode::F3) {
        config.enabled = !config.enabled;
    }
}

fn sync_vehicle_transforms(
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
            let mut target = pose_transform(record.pose());
            let tangent = record.pose().tangent();
            let forward = Vec3::new(tangent.x(), tangent.y(), tangent.z());
            // Runtime pose 是前保险杠位置；表现中心沿切向后移半个车长。
            target.translation -= forward * metrics.half_length;
            *transform = target;
        }
    }
}

/// 静态标记（Gate/Waiting/Conflict/Signal/高亮根）的构建与缓存重建。
fn maintain_static_overlay(
    mut commands: Commands,
    config: Res<JunctionDebugConfig>,
    session: Res<LaneFlowSession>,
    mut cache: ResMut<StaticOverlayCache>,
    roots: Query<Entity, With<DebugStaticRoot>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let context = session.junction_observation().context();
    let current_key = (context.network_revision(), context.world_generation());
    if !config.enabled {
        if cache.key.is_some() {
            for entity in &roots {
                commands.entity(entity).despawn();
            }
            cache.key = None;
        }
        return;
    }
    if cache.key == Some(current_key) {
        return;
    }
    for entity in &roots {
        commands.entity(entity).despawn();
    }
    cache.key = Some(current_key);

    let world = session.world();
    let revision = world.revision();
    let gate_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.95, 0.85, 0.2),
        ..default()
    });
    let waiting_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.2, 0.85, 0.9),
        double_sided: true,
        ..default()
    });
    let conflict_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.95, 0.45, 0.15),
        double_sided: true,
        ..default()
    });
    let highlight_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.3, 0.95, 0.4),
        double_sided: true,
        ..default()
    });

    let mut identity_lines: Vec<String> = Vec::new();
    let Some(spatial) = revision.spatial() else {
        identity_lines.push("no spatial component".to_owned());
        return spawn_identity_text(&mut commands, identity_lines);
    };
    let Some(lane_pose) = spatial.lane_pose() else {
        identity_lines.push("no lane pose network".to_owned());
        return spawn_identity_text(&mut commands, identity_lines);
    };

    // 静态 Gate 标记：遍历 live 路线 hop，画面按静态 Gate ordinal 去重（仅去重标记，
    // 不合并决策/预约）。
    let mut placed_gates: HashMap<u32, Vec3> = HashMap::new();
    for route in world.live_routes() {
        let Some(edges) = world.route_edges(route) else {
            continue;
        };
        for hop in 0..u32::try_from(edges.len()).unwrap_or(0) {
            let Some(observation) = world.route_gate(route, hop) else {
                continue;
            };
            let gate = observation.gate().raw();
            if placed_gates.contains_key(&gate) {
                continue;
            }
            let Some(geometry) = lane_pose.lane_geometry(observation.edge()) else {
                continue;
            };
            let Some(point) = geometry.points().last() else {
                continue;
            };
            placed_gates.insert(gate, point_to_vec3(point) + Vec3::Y * 1.2);
        }
    }
    let gate_mesh = meshes.add(Cuboid::new(0.8, 2.4, 0.8));
    for position in placed_gates.values() {
        commands.spawn((
            Mesh3d(gate_mesh.clone()),
            MeshMaterial3d(gate_material.clone()),
            Transform::from_translation(*position),
            DebugStaticRoot,
        ));
    }

    // Waiting 区间 ribbon：entry→release Gate 之间的机动路径边子段。
    let traffic = revision.traffic();
    let relations = traffic.relations();
    let maneuvers = traffic.maneuvers();
    let zone_count = traffic.entity_counts().count(EntityKind::WaitingZone);
    for raw in 0..zone_count {
        let zone = WaitingZoneOrdinal::from_raw(raw);
        let Some(zone_view) = relations.waiting_zone(zone) else {
            continue;
        };
        let Some(path) = maneuvers.maneuver_path(zone_view.path()) else {
            continue;
        };
        let Some(entry) = relations
            .maneuver_gate(zone_view.entry_gate())
            .map(|gate| gate.transition_index())
        else {
            continue;
        };
        let Some(release) = relations
            .maneuver_gate(zone_view.release_gate())
            .map(|gate| gate.transition_index())
        else {
            continue;
        };
        let edges = path.edges();
        let (Some(start), Some(end)) = (
            usize::try_from(entry)
                .ok()
                .filter(|index| *index < edges.len()),
            usize::try_from(release)
                .ok()
                .filter(|index| *index < edges.len()),
        ) else {
            continue;
        };
        // transition i 的门边界位于 edges i 与 i+1 之间：待转区间从 entry 边界后
        // 一条边开始，到 release 边界所在的前一条边为止。
        let Some(start) = start.checked_add(1).filter(|index| *index <= end) else {
            continue;
        };
        let mut points = Vec::new();
        for edge in &edges[start..=end] {
            if let Some(geometry) = lane_pose.lane_geometry(*edge) {
                points.extend(geometry.points().iter().map(point_to_vec3));
            }
        }
        if let Some(mesh) = ribbon_mesh(&points, 1.5, 0.25, false) {
            commands.spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(waiting_material.clone()),
                DebugStaticRoot,
            ));
        }
    }

    // Conflict 区域 ring_xz 描边；无区域几何时退化为带身份的文字标记。
    let conflict_count = traffic.entity_counts().count(EntityKind::ConflictZone);
    for raw in 0..conflict_count {
        let zone = ConflictZoneOrdinal::from_raw(raw);
        match spatial.conflict_zone_region(zone) {
            Some(region) => {
                let (min_y, max_y) = region.height_range();
                let base_y = (min_y + max_y) * 0.5;
                let points: Vec<Vec3> = region
                    .ring_xz()
                    .iter()
                    .map(|point| Vec3::new(point.x, base_y, point.z))
                    .collect();
                if let Some(mesh) = ribbon_mesh(&points, 0.8, 0.4, true) {
                    commands.spawn((
                        Mesh3d(meshes.add(mesh)),
                        MeshMaterial3d(conflict_material.clone()),
                        DebugStaticRoot,
                    ));
                }
            }
            None => {
                identity_lines.push(format!("conflict-zone-{raw} (no region)"));
            }
        }
    }
    if !identity_lines.is_empty() {
        spawn_identity_text(&mut commands, identity_lines);
    }

    // 信号状态点：预建红/黄/绿/未知共享材质，逐点按 committed aspect 选派句柄
    // （不改写共享 asset，避免所有点显示同一颜色）。
    let group_count = traffic.entity_counts().count(EntityKind::SignalGroup);
    let dot_mesh = meshes.add(Sphere::new(1.1).mesh().build());
    let red = materials.add(StandardMaterial::from_color(Color::srgb(0.9, 0.1, 0.1)));
    let yellow = materials.add(StandardMaterial::from_color(Color::srgb(0.95, 0.85, 0.1)));
    let green = materials.add(StandardMaterial::from_color(Color::srgb(0.1, 0.9, 0.2)));
    let unknown = materials.add(StandardMaterial::from_color(Color::srgb(0.35, 0.35, 0.38)));
    for raw in 0..group_count {
        let group = SignalGroupOrdinal::from_raw(raw);
        let Some(group_view) = relations.signal_group(group) else {
            continue;
        };
        let Some(first_gate) = group_view.gates().first().copied() else {
            continue;
        };
        let Some(gate_view) = relations.maneuver_gate(first_gate) else {
            continue;
        };
        let Some(stop) = relations.stop_line(gate_view.stop_line()) else {
            continue;
        };
        let Some(geometry) = lane_pose.lane_geometry(stop.edge()) else {
            continue;
        };
        let Some(point) = geometry.points().last() else {
            continue;
        };
        commands.spawn((
            Mesh3d(dot_mesh.clone()),
            MeshMaterial3d(red.clone()),
            Transform::from_translation(point_to_vec3(point) + Vec3::Y * 3.5),
            SignalDotMarker {
                group: raw,
                red: red.clone(),
                yellow: yellow.clone(),
                green: green.clone(),
                unknown: unknown.clone(),
            },
            DebugStaticRoot,
        ));
    }

    // 选定车辆高亮框（动态定位，见 `apply_overlay_visuals`）。
    let length = profile_length(&world.revision()) as f32 / 1_000.0;
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(2.4, 0.12, length + 1.2))),
        MeshMaterial3d(highlight_material),
        Transform::IDENTITY,
        SelectedVehicleMarker,
        DebugStaticRoot,
    ));
}

fn profile_length(revision: &SharedNetworkRevision) -> u32 {
    revision
        .traffic()
        .relations()
        .vehicle_profile(laneflow_static_contract::VehicleProfileOrdinal::from_raw(0))
        .map_or(4_500, |profile| profile.length_mm())
}

/// 无区域几何时以屏幕空间 UI 文字列出带身份的标记（§4：不计算几何相交补冲突事实）。
fn spawn_identity_text(commands: &mut Commands, lines: Vec<String>) {
    commands.spawn((
        Text::new(lines.join("\n")),
        TextFont::from_font_size(16.0),
        TextColor(Color::srgb(0.95, 0.5, 0.2)),
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(12.0),
            top: Val::Px(12.0),
            ..default()
        },
        DebugStaticRoot,
    ));
}

/// 每拍在 Observe 阶段消费 `junction_observation`：按车聚合决策（每拍对批次建
/// 一次索引），重写面板内容、选定车辆与信号状态。只读 Session。
fn observe_overlay(
    config: Res<JunctionDebugConfig>,
    session: Res<LaneFlowSession>,
    plan: Res<SpawnedPlan>,
    mut panel: ResMut<JunctionDebugPanel>,
) {
    if !config.enabled {
        panel.content.clear();
        panel.selected = None;
        panel.signals.clear();
        return;
    }
    let view = session.junction_observation();
    let context = view.context();
    let mut content = String::new();
    let _ = writeln!(
        content,
        "#285 junction debug | F3 overlay | tick={} gen={}",
        context.tick_index(),
        context.world_generation().get()
    );

    let waiting = view.latest_waiting_decisions();
    let conflict = view.latest_conflict_decisions();
    // 按车聚合：每拍对整个决策批次建一次可复用索引（观测合同 §3）。
    let mut by_vehicle: HashMap<VehicleHandle, Vec<String>> = HashMap::new();
    for decision in waiting {
        by_vehicle
            .entry(decision.vehicle())
            .or_default()
            .push(format!(
                "wait hop{} = {:?}",
                decision.anchor().hop(),
                decision.outcome()
            ));
    }
    for decision in conflict {
        by_vehicle
            .entry(decision.vehicle())
            .or_default()
            .push(format!(
                "conflict hop{} = {:?}",
                decision.anchor().hop(),
                decision.outcome()
            ));
    }

    // 选定车辆：只在 Active（有当前 committed pose）的车里选——本拍有决策的首辆，
    // 否则首辆；Completed 车不再停留，避免高亮/面板锁在已完成车辆上。
    let rows: Vec<_> = view.vehicles().collect();
    let active: Vec<_> = rows
        .iter()
        .filter(|row| row.state().status() == VehicleStatus::Active)
        .collect();
    let selected = active
        .iter()
        .find(|row| by_vehicle.contains_key(&row.vehicle()))
        .or_else(|| active.first())
        .map(|row| row.vehicle());
    panel.selected = selected;

    if let Some(vehicle) = selected {
        let role = plan
            .0
            .iter()
            .find(|spawned| spawned.vehicle == vehicle)
            .map_or("vehicle", |spawned| spawned.role.tag());
        let _ = writeln!(content, "selected={vehicle:?} role={role}");
        if let Some(row) = rows.iter().find(|row| row.vehicle() == vehicle) {
            let state = row.state();
            match state.maneuver_traversal() {
                Some(traversal) => {
                    let _ = writeln!(
                        content,
                        "phase={:?} occurrence={} route={:?}",
                        traversal.phase(),
                        traversal.maneuver_occurrence_index(),
                        traversal.route()
                    );
                }
                None => {
                    let _ = writeln!(content, "phase=none");
                }
            }
            match state.waiting_membership() {
                Some(membership) => {
                    let _ = writeln!(
                        content,
                        "member=zone{} seq={} release_hop={}",
                        membership.waiting_zone().raw(),
                        membership.admission_sequence(),
                        membership.release_hop()
                    );
                }
                None => {
                    let _ = writeln!(content, "member=none");
                }
            }
            match by_vehicle.get(&vehicle) {
                Some(lines) if !lines.is_empty() => {
                    for line in lines {
                        let _ = writeln!(content, "{line}");
                    }
                }
                _ => {
                    let _ = writeln!(content, "decisions=this tick none");
                }
            }
            match row.conflict_reservation() {
                Some(reservation) => {
                    let _ = writeln!(
                        content,
                        "reservation=route{:?} occurrence={} admission_hop={} acquired_tick={}",
                        reservation.route(),
                        reservation.maneuver_occurrence_index(),
                        reservation.admission_gate_hop(),
                        reservation.acquired_tick()
                    );
                }
                None => {
                    let _ = writeln!(content, "reservation=none");
                }
            }
            // 未来 Gate 列表：从当前路线边下标起的 route/hop 定位（不按静态 Gate 折叠）。
            let route = state.route();
            if let Some(edges) = session.world().route_edges(route) {
                let mut gates = Vec::new();
                let start = usize::try_from(state.route_edge_index()).unwrap_or(0);
                for (offset, _) in edges.iter().enumerate().skip(start) {
                    let Ok(hop) = u32::try_from(offset) else {
                        break;
                    };
                    if let Some(observation) = view.route_gate(route, hop) {
                        gates.push(format!("hop{}:g{}", hop, observation.gate().raw()));
                    }
                }
                if gates.len() > 8 {
                    gates.truncate(8);
                    gates.push("...".to_owned());
                }
                let _ = writeln!(content, "gates_ahead={}", gates.join(" "));
            }
        }
    } else {
        let _ = writeln!(content, "selected=none");
    }

    panel.signals = session
        .world()
        .committed_signal_groups()
        .as_slice()
        .iter()
        .map(|(group, aspect)| (group.raw(), *aspect))
        .collect();
    let signal_text: Vec<String> = panel
        .signals
        .iter()
        .map(|(group, aspect)| format!("g{group}:{aspect:?}"))
        .collect();
    let _ = writeln!(content, "signals={}", signal_text.join(" "));

    panel.content = content;
}

fn apply_overlay_visuals(
    config: Res<JunctionDebugConfig>,
    buffer: Res<PoseBuffer>,
    panel: Res<JunctionDebugPanel>,
    metrics: Res<VehicleMetrics>,
    mut panel_text: Query<&mut Text, With<PanelText>>,
    mut dots: Query<(&SignalDotMarker, &mut MeshMaterial3d<StandardMaterial>)>,
    mut highlight: Query<(&mut Transform, &mut Visibility), With<SelectedVehicleMarker>>,
) {
    if let Ok(mut text) = panel_text.single_mut() {
        text.0 = if config.enabled {
            panel.content.clone()
        } else {
            String::new()
        };
    }
    if !config.enabled {
        return;
    }
    // 逐点选派预建共享材质句柄；不就地改写共享 asset。
    for (marker, mut material) in &mut dots {
        let aspect = panel
            .signals
            .iter()
            .find(|(group, _)| *group == marker.group)
            .map(|(_, aspect)| *aspect);
        let handle = match aspect {
            Some(SignalAspect::Green) => marker.green.clone(),
            Some(SignalAspect::Yellow) => marker.yellow.clone(),
            Some(SignalAspect::Red) => marker.red.clone(),
            _ => marker.unknown.clone(),
        };
        *material = MeshMaterial3d(handle);
    }
    // 选中车无当前 pose（如 Completed 车不在 committed 批次里）时隐藏高亮，
    // 而不是把它挪到世界原点。
    let selected_position = panel.selected.and_then(|vehicle| {
        buffer
            .0
            .vehicles()
            .iter()
            .position(|candidate| *candidate == vehicle)
            .and_then(|index| buffer.0.batch().records().get(index))
            .map(|record| {
                let mut target = pose_transform(record.pose());
                let tangent = record.pose().tangent();
                let forward = Vec3::new(tangent.x(), tangent.y(), tangent.z());
                target.translation -= forward * metrics.half_length;
                target
            })
    });
    if let Ok((mut transform, mut visibility)) = highlight.single_mut() {
        match selected_position {
            Some(target) => {
                *transform = target;
                *visibility = Visibility::Visible;
            }
            None => {
                *visibility = Visibility::Hidden;
            }
        }
    }
}
