//! 地面、路面铺装与路面标线（停止线、斑马线、车道导向箭头）的实体生成。
use std::collections::{BTreeSet, HashMap};

use bevy::{mesh::Indices, prelude::*};
use laneflow_runtime::TrafficWorld;
use laneflow_static_contract::{EntityKind, LaneEdgeOrdinal, StopLineOrdinal, WaitingZoneOrdinal};
use laneflow_static_network::{CanonicalPoint, LanePoseNetwork, SharedNetworkRevision};

use super::{
    junction_debug_scene,
    junction_road::{RoadLayout, RoadMeshes},
};

/// 车道宽度（米），与场景合同 `examples/config/v0.1-complex-junction.toml` 的
/// `lane_width_meters` 一致。
const LANE_WIDTH_METERS: f32 = 3.5;
/// 地平面边长（米）；覆盖环路并留透视余量。
const GROUND_SIZE_METERS: f32 = 1_000.0;
/// 道路/标线/铺装的错层高度（米），避免共面 z-fight。
pub(super) const MARKING_Y: f32 = 0.06;
/// 斑马线：条纹长（沿车道方向）/条纹宽/净间隔/与停止线的净距（米）。
const CROSSWALK_LENGTH_METERS: f32 = 3.0;
const CROSSWALK_STRIPE_WIDTH_METERS: f32 = 0.45;
const CROSSWALK_STRIPE_GAP_METERS: f32 = 0.45;
const CROSSWALK_SETBACK_METERS: f32 = 0.3;
/// 车道导向箭头的施画位置：进口车道终点（停止线）前（米）。
const ARROW_SETBACK_METERS: f32 = 8.0;
/// 停止线带：沿车道方向 0.4 m、横跨整条车道宽（米）。
const STOP_LINE_BAND_METERS: f32 = 0.4;

/// 生成地面、`junction_road` 铺装 mesh 与同修订车道几何上的路面标线。
pub(super) fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    world: &TrafficWorld,
    revision: &SharedNetworkRevision,
    roads: RoadMeshes,
) {
    // 地面：大薄板，顶面在 y=0。灰绿压暗，与沥青拉开对比。
    let ground_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.27, 0.36, 0.23),
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(GROUND_SIZE_METERS, 0.2, GROUND_SIZE_METERS))),
        MeshMaterial3d(ground_material),
        Transform::from_xyz(0.0, -0.1, 0.0),
    ));
    // 沥青材质：车行道 ribbon、路口铺装、环路共用。
    let asphalt_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.16, 0.19, 0.22),
        perceptual_roughness: 0.95,
        double_sided: true,
        ..default()
    });
    // 人行道材质：混凝土浅灰。
    let sidewalk_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.45, 0.45, 0.43),
        perceptual_roughness: 0.9,
        double_sided: true,
        ..default()
    });
    let white_material = materials.add(StandardMaterial::from_color(Color::srgb(0.88, 0.9, 0.87)));
    let yellow_material = materials.add(StandardMaterial::from_color(Color::srgb(0.9, 0.64, 0.08)));
    let curb_material = materials.add(StandardMaterial::from_color(Color::srgb(0.64, 0.67, 0.64)));
    let island_material =
        materials.add(StandardMaterial::from_color(Color::srgb(0.16, 0.27, 0.13)));
    for (mesh, material) in [
        (roads.asphalt, asphalt_material),
        (roads.sidewalk, sidewalk_material),
        (roads.curb, curb_material),
        (roads.white, white_material.clone()),
        (roads.yellow, yellow_material),
        (roads.islands, island_material),
    ] {
        commands.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(material)));
    }
    let Some(lane_pose) = revision.spatial().and_then(|spatial| spatial.lane_pose()) else {
        return;
    };
    spawn_stop_lines(
        commands,
        meshes,
        revision,
        lane_pose,
        white_material.clone(),
    );
    spawn_crosswalks(
        commands,
        meshes,
        revision,
        lane_pose,
        &roads.layout,
        white_material.clone(),
    );
    spawn_lane_arrows(commands, meshes, world, revision, lane_pose, white_material);
}

pub(super) fn point_to_vec3(point: &CanonicalPoint) -> Vec3 {
    Vec3::new(point.x, point.y, point.z)
}

/// 进口停止线及待转区前端的停车线；内部 Waiting entry 准入门不再画成停车线。
fn spawn_stop_lines(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    revision: &SharedNetworkRevision,
    lane_pose: &LanePoseNetwork,
    white: Handle<StandardMaterial>,
) {
    let relations = revision.traffic().relations();
    let stop_count = usize::try_from(
        revision
            .traffic()
            .entity_counts()
            .count(EntityKind::StopLine),
    )
    .unwrap_or(0);
    // Waiting entry 是区间准入门，不额外画成路口中的第二道停车横线。
    let waiting_entries: BTreeSet<_> = (0..revision
        .traffic()
        .entity_counts()
        .count(EntityKind::WaitingZone))
        .filter_map(|raw| relations.waiting_zone(WaitingZoneOrdinal::from_raw(raw)))
        .filter_map(|zone| relations.maneuver_gate(zone.entry_gate()))
        .map(|gate| gate.stop_line().raw())
        .collect();
    let mut drawn = BTreeSet::new();
    for raw in 0..stop_count {
        let stop = StopLineOrdinal::from_raw(u32::try_from(raw).unwrap_or(0));
        let Some(view) = relations.stop_line(stop) else {
            continue;
        };
        // 只画绑定了信号门的停止线。
        let signal_gated = view.gates().iter().any(|&gate| {
            relations
                .maneuver_gate(gate)
                .and_then(|gate_view| gate_view.signal_group())
                .is_some()
        });
        if !signal_gated || waiting_entries.contains(&stop.raw()) || !drawn.insert(raw) {
            continue;
        }
        let Some(geometry) = lane_pose.lane_geometry(view.edge()) else {
            continue;
        };
        let points = geometry.points();
        let (Some(last), _) = (points.last(), points.first()) else {
            continue;
        };
        let end = point_to_vec3(last);
        let previous = points
            .get(points.len().saturating_sub(2))
            .map(point_to_vec3)
            .unwrap_or(end);
        let tangent = (end - previous).normalize_or_zero();
        if tangent.length_squared() < 0.5 {
            continue;
        }
        let band = [
            end - tangent * (STOP_LINE_BAND_METERS * 0.5),
            end + tangent * (STOP_LINE_BAND_METERS * 0.5),
        ];
        if let Some(mesh) = ribbon_mesh(&band, LANE_WIDTH_METERS, MARKING_Y, 0.0, false) {
            commands.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(white.clone())));
        }
    }
}

/// 斑马线：只在路口外部道路的进口车道（`-in` 边）的信号停止线处画，向路口一侧 2 m、
/// 横跨整个车行道（双向）的条纹带；中心投影到道路轴线上，同一进口的多条
/// 进口车道去重为一条斑马线。
fn spawn_crosswalks(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    revision: &SharedNetworkRevision,
    lane_pose: &LanePoseNetwork,
    layout: &RoadLayout,
    white: Handle<StandardMaterial>,
) {
    let mut entry_edges = BTreeSet::new();
    for (name, ordinal) in junction_debug_scene::edge_ordinals(revision) {
        if !name.contains('.') && !name.starts_with("loop-") && name.contains("-in") {
            entry_edges.insert(ordinal.raw());
        }
    }
    let relations = revision.traffic().relations();
    let stop_count = usize::try_from(
        revision
            .traffic()
            .entity_counts()
            .count(EntityKind::StopLine),
    )
    .unwrap_or(0);
    let mut placed = BTreeSet::new();
    for raw in 0..stop_count {
        let stop = StopLineOrdinal::from_raw(u32::try_from(raw).unwrap_or(0));
        let Some(view) = relations.stop_line(stop) else {
            continue;
        };
        let signal_gated = view.gates().iter().any(|&gate| {
            relations
                .maneuver_gate(gate)
                .and_then(|gate_view| gate_view.signal_group())
                .is_some()
        });
        if !signal_gated || !entry_edges.contains(&view.edge().raw()) {
            continue;
        }
        let Some(geometry) = lane_pose.lane_geometry(view.edge()) else {
            continue;
        };
        let points = geometry.points();
        let Some(last) = points.last() else {
            continue;
        };
        let end = point_to_vec3(last);
        let previous = points
            .get(points.len().saturating_sub(2))
            .map(point_to_vec3)
            .unwrap_or(end);
        let tangent = (end - previous).normalize_or_zero();
        if tangent.length_squared() < 0.5 {
            continue;
        }
        // 停止线带半宽 + 净距 + 半条斑马线长，落点在停止线靠路口一侧；
        // 横向投影回道路轴线（斑马线横跨双向整个车行道，居中于道路）。
        let lateral = Vec3::new(-tangent.z, 0.0, tangent.x);
        let mut center = end
            + tangent
                * (STOP_LINE_BAND_METERS * 0.5
                    + CROSSWALK_SETBACK_METERS
                    + CROSSWALK_LENGTH_METERS * 0.5);
        center -= lateral * lateral.dot(center);
        // 同一进口的多条进口车道共享一条斑马线：按 0.5 m 量化去重。
        let key = (
            (center.x * 2.0).round() as i32,
            (center.z * 2.0).round() as i32,
        );
        if !placed.insert(key) {
            continue;
        }
        let main_axis = tangent.x.abs() >= tangent.z.abs();
        let half_width = if main_axis {
            layout.main_half_width
        } else {
            layout.secondary_half_width
        };
        // 条纹复用 ribbon_mesh 渲染路径：每条条纹 = 沿车道方向的一段 ribbon，
        // 法向偏移即横跨位置（中心 = cursor + 半条宽）。
        let half_length = CROSSWALK_LENGTH_METERS * 0.5;
        let stripe_axis = [
            center - tangent * half_length,
            center + tangent * half_length,
        ];
        let mut cursor = -half_width + CROSSWALK_STRIPE_GAP_METERS;
        while cursor + CROSSWALK_STRIPE_WIDTH_METERS <= half_width {
            let stripe_center = cursor + CROSSWALK_STRIPE_WIDTH_METERS * 0.5;
            if let Some(mesh) = ribbon_mesh(
                &stripe_axis,
                CROSSWALK_STRIPE_WIDTH_METERS,
                MARKING_Y,
                stripe_center,
                false,
            ) {
                commands.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(white.clone())));
            }
            cursor += CROSSWALK_STRIPE_WIDTH_METERS + CROSSWALK_STRIPE_GAP_METERS;
        }
    }
}

/// 进口车道导向箭头：按穿过该车道的路线机动方向（直行/左转/右转）合并成
/// 一个白色箭头 mesh，施画在停止线前 `ARROW_SETBACK_METERS` 处。
fn spawn_lane_arrows(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    world: &TrafficWorld,
    revision: &SharedNetworkRevision,
    lane_pose: &LanePoseNetwork,
    white: Handle<StandardMaterial>,
) {
    let ordinals = junction_debug_scene::edge_ordinals(revision);
    let mut name_by_ordinal: HashMap<u32, &str> = HashMap::new();
    for (name, ordinal) in &ordinals {
        name_by_ordinal.insert(ordinal.raw(), name.as_str());
    }
    // 每条进口车道边的机动方向集合：bit0 直行、bit1 左转、bit2 右转。
    // 路线边序是 环路→进口道路→内部→出口道路→环路（环路车还会多次过路口），
    // 相切转向在入口处与直行同向，须比较本次穿越的进口/出口切向。
    let mut moves: HashMap<u32, u8> = HashMap::new();
    for route in world.live_routes() {
        let Some(edges) = world.route_edges(route) else {
            continue;
        };
        for (index, first) in edges.iter().enumerate() {
            let Some(name) = name_by_ordinal.get(&first.raw()) else {
                continue;
            };
            if !name.contains("-in") {
                continue;
            }
            let Some(second) = edges
                .iter()
                .skip(index + 1)
                .take_while(|edge| {
                    !name_by_ordinal
                        .get(&edge.raw())
                        .is_some_and(|name| name.starts_with("loop-"))
                })
                .find(|edge| {
                    name_by_ordinal
                        .get(&edge.raw())
                        .is_some_and(|name| name.contains("-out"))
                })
            else {
                continue;
            };
            let (Some(g0), Some(g1)) = (
                lane_pose.lane_geometry(*first),
                lane_pose.lane_geometry(*second),
            ) else {
                continue;
            };
            let (Some(t0), Some(t1)) = (end_tangent(g0.points()), start_tangent(g1.points()))
            else {
                continue;
            };
            // (t0 × t1).y < 0 为右转（右手系、+Y 向上）。
            let cross_y = t0.z * t1.x - t0.x * t1.z;
            let bit = if cross_y > 0.25 {
                0b010
            } else if cross_y < -0.25 {
                0b100
            } else {
                0b001
            };
            *moves.entry(first.raw()).or_default() |= bit;
        }
    }
    for (raw, bits) in moves {
        let Some(geometry) = lane_pose.lane_geometry(LaneEdgeOrdinal::from_raw(raw)) else {
            continue;
        };
        let Some(end) = geometry.points().last().map(point_to_vec3) else {
            continue;
        };
        let Some(tangent) = end_tangent(geometry.points()) else {
            continue;
        };
        // 导向箭头全部走 ribbon_mesh（已验证的渲染路径）：箭杆一条竖带，
        // 直行头为人字两斜带，转向头为横杆 + 侧向人字两斜带。
        let position = end - tangent * ARROW_SETBACK_METERS;
        let rotation = Quat::from_rotation_y(tangent.x.atan2(tangent.z));
        let mut strips: Vec<([Vec3; 2], f32)> = Vec::new();
        strips.push(([Vec3::new(0.0, 0.0, -1.3), Vec3::new(0.0, 0.0, 0.4)], 0.3));
        if bits & 0b001 != 0 {
            strips.push((
                [Vec3::new(-0.45, 0.0, 0.75), Vec3::new(0.0, 0.0, 1.5)],
                0.22,
            ));
            strips.push(([Vec3::new(0.45, 0.0, 0.75), Vec3::new(0.0, 0.0, 1.5)], 0.22));
        }
        if bits & 0b010 != 0 {
            strips.push(([Vec3::new(0.0, 0.0, 0.55), Vec3::new(1.05, 0.0, 0.55)], 0.3));
            strips.push(([Vec3::new(0.8, 0.0, 0.95), Vec3::new(1.6, 0.0, 0.55)], 0.22));
            strips.push(([Vec3::new(0.8, 0.0, 0.15), Vec3::new(1.6, 0.0, 0.55)], 0.22));
        }
        if bits & 0b100 != 0 {
            strips.push((
                [Vec3::new(0.0, 0.0, 0.55), Vec3::new(-1.05, 0.0, 0.55)],
                0.3,
            ));
            strips.push((
                [Vec3::new(-0.8, 0.0, 0.95), Vec3::new(-1.6, 0.0, 0.55)],
                0.22,
            ));
            strips.push((
                [Vec3::new(-0.8, 0.0, 0.15), Vec3::new(-1.6, 0.0, 0.55)],
                0.22,
            ));
        }
        for (points, width) in strips {
            let Some(mesh) = ribbon_mesh(&points, width, MARKING_Y, 0.0, false) else {
                continue;
            };
            commands.spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(white.clone()),
                Transform::from_translation(position).with_rotation(rotation),
            ));
        }
    }
}

/// 折线末段/首段单位切向（XZ 平面）。
fn end_tangent(points: &[CanonicalPoint]) -> Option<Vec3> {
    let last = points.last()?;
    let previous = points.get(points.len().saturating_sub(2)).unwrap_or(last);
    let tangent = (point_to_vec3(last) - point_to_vec3(previous)).normalize_or_zero();
    (tangent.length_squared() >= 0.5).then_some(tangent)
}

fn start_tangent(points: &[CanonicalPoint]) -> Option<Vec3> {
    let first = points.first()?;
    let next = points.get(1).unwrap_or(first);
    let tangent = (point_to_vec3(next) - point_to_vec3(first)).normalize_or_zero();
    (tangent.length_squared() >= 0.5).then_some(tangent)
}

/// 采样点折线的三角带 ribbon；`lateral_offset` 沿横向单位偏移（车道边线用），
/// `closed` 时把首点追加到尾部闭合。平面 ribbon 法线一律手工朝上，
/// 避免 winding 误差导致背光面全黑。
pub(super) fn ribbon_mesh(
    points: &[Vec3],
    width: f32,
    y_lift: f32,
    lateral_offset: f32,
    closed: bool,
) -> Option<Mesh> {
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
        let lateral_unit = Vec3::new(-direction.z, 0.0, direction.x);
        let base = *point + Vec3::Y * y_lift + lateral_unit * lateral_offset;
        positions.push((base - lateral_unit * half).to_array());
        positions.push((base + lateral_unit * half).to_array());
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
    let normals = vec![[0.0_f32, 1.0, 0.0]; line.len() * 2];
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    Some(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn east_inbound_left_turn_arrow_points_toward_its_exit() {
        use bevy::mesh::VertexAttributeValues;

        let scene = junction_debug_scene::build().unwrap();
        let revision = scene.session.world().revision();
        let lane_pose = revision.spatial().unwrap().lane_pose().unwrap();
        let edges = junction_debug_scene::edge_ordinals(&revision);
        let points = lane_pose.lane_geometry(edges["e-in-i1"]).unwrap().points();
        let center = point_to_vec3(points.last().unwrap())
            - end_tangent(points).unwrap() * ARROW_SETBACK_METERS;
        let mut world = World::new();
        let mut meshes = Assets::<Mesh>::default();
        spawn_lane_arrows(
            &mut world.commands(),
            &mut meshes,
            scene.session.world(),
            &revision,
            lane_pose,
            Handle::default(),
        );
        world.flush();
        let mut extents = (f32::INFINITY, f32::NEG_INFINITY);
        for (mesh, transform) in world.query::<(&Mesh3d, &Transform)>().iter(&world) {
            if transform.translation.distance(center) > 0.001 {
                continue;
            }
            let Some(VertexAttributeValues::Float32x3(positions)) = meshes
                .get(&mesh.0)
                .unwrap()
                .attribute(Mesh::ATTRIBUTE_POSITION)
            else {
                panic!("arrow mesh positions");
            };
            for point in positions {
                let lateral = transform.transform_point(Vec3::from_array(*point)).z - center.z;
                extents.0 = extents.0.min(lateral);
                extents.1 = extents.1.max(lateral);
            }
        }
        // 东进口向西行驶，左转出口在南侧（+Z）；直行箭头只占中央窄带。
        assert!(extents.1 > 1.5, "missing southward turn head: {extents:?}");
        assert!(
            extents.0 > -0.8,
            "unexpected northward turn head: {extents:?}"
        );
    }
}
