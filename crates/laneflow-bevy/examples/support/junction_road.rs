//! 示例道路表面：同根车道几何决定边界，表现网格不改变运行时路径。
use std::collections::BTreeMap;

use bevy::{mesh::Indices, prelude::*};
use laneflow_static_network::SharedNetworkRevision;

use super::junction_debug_scene;

const WIDTH: f32 = 3.5;
const ROAD_Y: f32 = 0.03;
const PAINT_Y: f32 = 0.045;
const WALK_Y: f32 = 0.18;
const WALK_WIDTH: f32 = 3.0;

#[derive(Clone, Copy)]
pub struct RoadLayout {
    pub main_half_width: f32,
    pub secondary_half_width: f32,
    pub junction_half_x: f32,
    pub junction_half_z: f32,
    main_arm_end: f32,
    secondary_arm_end: f32,
    corner_radius: f32,
}

pub struct RoadMeshes {
    pub asphalt: Mesh,
    pub sidewalk: Mesh,
    pub curb: Mesh,
    pub white: Mesh,
    pub yellow: Mesh,
    pub islands: Mesh,
    pub layout: RoadLayout,
    pub extent: f32,
}

#[derive(Default)]
struct Surface {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    indices: Vec<u32>,
}

impl Surface {
    fn triangle(&mut self, a: Vec3, mut b: Vec3, mut c: Vec3) {
        let mut normal = (b - a).cross(c - a);
        if normal.length_squared() < 0.000_000_01 {
            return;
        }
        if normal.y < 0.0 {
            std::mem::swap(&mut b, &mut c);
            normal = -normal;
        }
        let offset = self.positions.len() as u32;
        self.positions
            .extend([a.to_array(), b.to_array(), c.to_array()]);
        self.normals.extend([normal.normalize().to_array(); 3]);
        self.indices.extend([offset, offset + 1, offset + 2]);
    }

    fn strip(&mut self, left: &[Vec3], right: &[Vec3]) {
        assert_eq!(left.len(), right.len());
        for index in 1..left.len() {
            self.triangle(left[index - 1], right[index - 1], left[index]);
            self.triangle(right[index - 1], right[index], left[index]);
        }
    }

    fn band(&mut self, line: &[Vec3], width: f32, y: f32) {
        self.strip(
            &offset(line, -width * 0.5, y),
            &offset(line, width * 0.5, y),
        );
    }

    fn fan(&mut self, center: Vec3, outline: &[Vec3]) {
        for index in 0..outline.len() {
            self.triangle(center, outline[index], outline[(index + 1) % outline.len()]);
        }
    }

    fn finish(self) -> Mesh {
        let mut mesh = Mesh::new(
            bevy::mesh::PrimitiveTopology::TriangleList,
            bevy::asset::RenderAssetUsages::default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals);
        mesh.insert_indices(Indices::U32(self.indices));
        mesh
    }
}

fn at_height(mut point: Vec3, y: f32) -> Vec3 {
    point.y = y;
    point
}

fn lateral(line: &[Vec3], index: usize) -> Vec3 {
    let before = line[index.saturating_sub(1)];
    let after = line[(index + 1).min(line.len() - 1)];
    let direction = (after - before).normalize_or_zero();
    Vec3::new(-direction.z, 0.0, direction.x)
}

fn offset(line: &[Vec3], distance: f32, y: f32) -> Vec<Vec3> {
    line.iter()
        .enumerate()
        .map(|(index, point)| at_height(*point + lateral(line, index) * distance, y))
        .collect()
}

fn lengths(line: &[Vec3]) -> Vec<f32> {
    let mut result = vec![0.0];
    for pair in line.windows(2) {
        result.push(result.last().unwrap() + pair[0].distance(pair[1]));
    }
    result
}

fn sample(line: &[Vec3], stations: &[f32], station: f32) -> Vec3 {
    let index = stations
        .partition_point(|value| *value < station)
        .clamp(1, line.len() - 1);
    let fraction =
        ((station - stations[index - 1]) / (stations[index] - stations[index - 1])).clamp(0.0, 1.0);
    line[index - 1].lerp(line[index], fraction)
}

fn slice(line: &[Vec3], stations: &[f32], from: f32, to: f32) -> Vec<Vec3> {
    let mut points = vec![sample(line, stations, from)];
    for (point, station) in line.iter().zip(stations) {
        if *station > from && *station < to {
            points.push(*point);
        }
    }
    points.push(sample(line, stations, to));
    points
}

fn dashed(surface: &mut Surface, line: &[Vec3], from: f32, to: f32) {
    let stations = lengths(line);
    let mut cursor = from;
    while cursor + 0.1 < to {
        let segment = slice(line, &stations, cursor, (cursor + 3.0).min(to));
        surface.band(&segment, 0.12, PAINT_Y);
        cursor += 8.0;
    }
}

/// 一个街角：从主路沿正确的内凹路缘圆弧走到次路。圆心位于街角内，
/// 与两段直线路缘相切；镜像不会改变铺装网格朝向。
fn corner(layout: RoadLayout, sx: f32, sz: f32, main_end: f32, secondary_end: f32) -> Vec<Vec3> {
    let s = layout.secondary_half_width;
    let m = layout.main_half_width;
    let r = layout.corner_radius;
    let mut line = vec![Vec3::new(sx * main_end, ROAD_Y, sz * m)];
    for index in 0..=32 {
        let angle = (index as f32 / 32.0) * std::f32::consts::FRAC_PI_2;
        line.push(Vec3::new(
            sx * (s + r - r * angle.sin()),
            ROAD_Y,
            sz * (m + r - r * angle.cos()),
        ));
    }
    line.push(Vec3::new(sx * s, ROAD_Y, sz * secondary_end));
    line
}

fn curb(surface: &mut Surface, line: &[Vec3], outside: f32) {
    let bottom: Vec<_> = line.iter().map(|point| at_height(*point, ROAD_Y)).collect();
    let top: Vec<_> = line.iter().map(|point| at_height(*point, WALK_Y)).collect();
    if outside > 0.0 {
        surface.strip(&bottom, &top);
    } else {
        surface.strip(&top, &bottom);
    }
    surface.strip(&top, &offset(line, outside * 0.16, WALK_Y));
}

fn nearest(point: Vec3, line: &[Vec3]) -> Vec3 {
    let mut result = line[0];
    let mut distance = f32::MAX;
    for pair in line.windows(2) {
        let delta = pair[1] - pair[0];
        let t = ((point - pair[0]).dot(delta) / delta.length_squared()).clamp(0.0, 1.0);
        let candidate = pair[0] + delta * t;
        if candidate.distance_squared(point) < distance {
            result = candidate;
            distance = candidate.distance_squared(point);
        }
    }
    result
}

/// 两条同向中心线共同拥有一幅路面：只描外边界，足宽处画一条分隔虚线。
/// 汇合处停止分隔线，路缘连续收窄；不把两条重叠 ribbon 画成独立道路。
fn loop_surface(
    asphalt: &mut Surface,
    white: &mut Surface,
    sidewalk: &mut Surface,
    edging: &mut Surface,
    base: &[Vec3],
    other: &[Vec3],
) {
    let stations = lengths(base);
    let total = *stations.last().unwrap();
    let divisions = total.ceil() as usize;
    let points: Vec<_> = (0..=divisions)
        .map(|index| sample(base, &stations, total * index as f32 / divisions as f32))
        .collect();
    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut middle = Vec::new();
    let mut separations = Vec::new();
    for (index, point) in points.iter().enumerate() {
        let normal = lateral(&points, index);
        let paired = nearest(*point, other);
        let separation = (paired - *point).dot(normal).max(0.0);
        left.push(at_height(*point - normal * (WIDTH * 0.5), ROAD_Y));
        right.push(at_height(
            *point + normal * (separation + WIDTH * 0.5),
            ROAD_Y,
        ));
        middle.push(at_height(*point + normal * separation * 0.5, PAINT_Y));
        separations.push(separation);
    }
    asphalt.strip(&left, &right);
    // 转弯中心侧接各臂外侧人行道，端口处保持同宽、同高、同一条路缘。
    sidewalk.strip(
        &offset(&right, 0.16, WALK_Y),
        &offset(&right, WALK_WIDTH, WALK_Y),
    );
    curb(edging, &right, 1.0);
    white.band(&offset(&left, 0.18, PAINT_Y), 0.12, PAINT_Y);
    white.band(&offset(&right, -0.18, PAINT_Y), 0.12, PAINT_Y);
    let middle_stations = lengths(&middle);
    let from = separations.iter().position(|distance| *distance >= 3.35);
    let to = separations.iter().rposition(|distance| *distance >= 3.35);
    if let (Some(from), Some(to)) = (from, to) {
        dashed(white, &middle, middle_stations[from], middle_stations[to]);
    }
    // 双车道区沿真实切向画同向箭头，汇合/分流区不补造车道或通行规则。
    for centerline in [base, other] {
        let arc = lengths(centerline);
        for station in [65.0, *arc.last().unwrap() - 65.0] {
            let center = sample(centerline, &arc, station);
            let direction = (sample(centerline, &arc, station + 1.0)
                - sample(centerline, &arc, station - 1.0))
            .normalize_or_zero();
            arrow(white, center, direction);
        }
    }
}

fn arrow(surface: &mut Surface, center: Vec3, direction: Vec3) {
    let normal = Vec3::new(-direction.z, 0.0, direction.x);
    surface.band(
        &[center - direction * 2.0, center + direction * 2.0],
        0.18,
        PAINT_Y,
    );
    for sign in [-1.0, 1.0] {
        surface.band(
            &[
                center + direction * 0.6 + normal * sign * 0.8,
                center + direction * 2.0,
            ],
            0.18,
            PAINT_Y,
        );
    }
}

fn median(
    asphalt: &mut Surface,
    islands: &mut Surface,
    edging: &mut Surface,
    along: Vec3,
    width: f32,
    from: f32,
    to: f32,
) {
    let normal = Vec3::new(-along.z, 0.0, along.x);
    let radius = width * 0.5 - 0.18;
    if radius <= 0.0 || to - from <= radius * 2.0 {
        return;
    }
    let mut outline = Vec::new();
    for (center, sign) in [
        (along * (to - radius), 1.0),
        (along * (from + radius), -1.0),
    ] {
        for index in 0..=24 {
            let theta = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * index as f32 / 24.0;
            outline.push(at_height(
                center + (along * theta.cos() + normal * theta.sin()) * sign * radius,
                WALK_Y,
            ));
        }
    }
    let center = at_height(along * ((from + to) * 0.5), WALK_Y);
    islands.fan(center, &outline);
    let mut closed = outline.clone();
    closed.push(outline[0]);
    curb(edging, &closed, -1.0);
    // 整条道路的铺装已有连续底面，过街口不会跨到裸草地上。
    asphalt.band(&[along * (from - 4.0), along * (to + 4.0)], width, ROAD_Y);
}

pub fn build(revision: &SharedNetworkRevision) -> RoadMeshes {
    let lane_pose = revision
        .spatial()
        .and_then(|spatial| spatial.lane_pose())
        .expect("junction spatial geometry");
    let mut edges = BTreeMap::new();
    let mut layout = RoadLayout {
        main_half_width: 0.0,
        secondary_half_width: 0.0,
        junction_half_x: 0.0,
        junction_half_z: 0.0,
        main_arm_end: 0.0,
        secondary_arm_end: 0.0,
        corner_radius: 0.0,
    };
    let mut extent = 0.0_f32;
    for (name, ordinal) in junction_debug_scene::edge_ordinals(revision) {
        let geometry = lane_pose
            .lane_geometry(ordinal)
            .expect("catalog lane geometry");
        let points: Vec<_> = geometry
            .points()
            .iter()
            .map(|point| Vec3::new(point.x, point.y, point.z))
            .collect();
        for point in &points {
            extent = extent.max(point.x.abs()).max(point.z.abs());
            if name.contains('.') {
                layout.junction_half_x = layout.junction_half_x.max(point.x.abs());
                layout.junction_half_z = layout.junction_half_z.max(point.z.abs());
            } else if !name.starts_with("loop-") {
                if name.starts_with('e') || name.starts_with('w') {
                    layout.main_half_width =
                        layout.main_half_width.max(point.z.abs() + WIDTH * 0.5);
                    layout.main_arm_end = layout.main_arm_end.max(point.x.abs());
                } else {
                    layout.secondary_half_width =
                        layout.secondary_half_width.max(point.x.abs() + WIDTH * 0.5);
                    layout.secondary_arm_end = layout.secondary_arm_end.max(point.z.abs());
                }
            }
        }
        edges.insert(name, points);
    }
    layout.junction_half_x += 0.5;
    layout.junction_half_z += 0.5;
    layout.corner_radius = (layout.junction_half_x - layout.secondary_half_width)
        .min(layout.junction_half_z - layout.main_half_width)
        - 0.5;
    let mut asphalt = Surface::default();
    let mut white = Surface::default();
    let mut yellow = Surface::default();
    let mut sidewalk = Surface::default();
    let mut edging = Surface::default();
    let mut islands = Surface::default();
    let mut outline = Vec::new();
    for (index, (sx, sz)) in [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)]
        .into_iter()
        .enumerate()
    {
        let mut section = corner(
            layout,
            sx,
            sz,
            layout.junction_half_x,
            layout.junction_half_z,
        );
        if index % 2 == 1 {
            section.reverse();
        }
        outline.extend(section);
        let edge = corner(
            layout,
            sx,
            sz,
            layout.main_arm_end,
            layout.secondary_arm_end,
        );
        let outside = -sx * sz;
        sidewalk.strip(
            &offset(&edge, outside * 0.16, WALK_Y),
            &offset(&edge, outside * WALK_WIDTH, WALK_Y),
        );
        curb(&mut edging, &edge, outside);
    }
    asphalt.fan(Vec3::Y * ROAD_Y, &outline);
    // 同一臂以世界横坐标/纵坐标区分边界，不随车辆行驶方向翻转去重键。
    let mut boundaries = BTreeMap::new();
    for (name, points) in &edges {
        if name.contains('.') || name.starts_with("loop-") {
            continue;
        }
        let main = name.starts_with('e') || name.starts_with('w');
        let arm = name.as_bytes()[0];
        asphalt.band(points, WIDTH, ROAD_Y);
        for side in [-1.0, 1.0] {
            let line = offset(points, side * WIDTH * 0.5, PAINT_Y);
            let coordinate = if main { line[0].z } else { line[0].x };
            boundaries
                .entry((arm, (coordinate * 1_000.0).round() as i32))
                .or_insert(line);
        }
    }
    for ((arm, coordinate), line) in &boundaries {
        let main = *arm == b'e' || *arm == b'w';
        let half_width = if main {
            layout.main_half_width
        } else {
            layout.secondary_half_width
        };
        let offset = (*coordinate as f32 / 1_000.0).abs();
        let inside = half_width - if main { WIDTH * 2.0 } else { WIDTH };
        if (offset - inside).abs() < 0.1 {
            yellow.band(line, 0.12, PAINT_Y);
        } else if (offset - half_width).abs() < 0.1 {
            white.band(line, 0.12, PAINT_Y);
        } else {
            dashed(&mut white, line, 0.0, *lengths(line).last().unwrap());
        }
    }
    for (along, half_width, from, to, lanes) in [
        (
            Vec3::X,
            layout.main_half_width,
            layout.junction_half_x,
            layout.main_arm_end,
            2.0,
        ),
        (
            -Vec3::X,
            layout.main_half_width,
            layout.junction_half_x,
            layout.main_arm_end,
            2.0,
        ),
        (
            Vec3::Z,
            layout.secondary_half_width,
            layout.junction_half_z,
            layout.secondary_arm_end,
            1.0,
        ),
        (
            -Vec3::Z,
            layout.secondary_half_width,
            layout.junction_half_z,
            layout.secondary_arm_end,
            1.0,
        ),
    ] {
        median(
            &mut asphalt,
            &mut islands,
            &mut edging,
            along,
            (half_width - WIDTH * lanes) * 2.0,
            from + 4.0,
            to - 4.0,
        );
    }
    for portal in ["ne", "es", "sw", "wn"] {
        loop_surface(
            &mut asphalt,
            &mut white,
            &mut sidewalk,
            &mut edging,
            &edges[&format!("loop-{portal}-i0")],
            &edges[&format!("loop-{portal}-i1")],
        );
    }
    RoadMeshes {
        asphalt: asphalt.finish(),
        sidewalk: sidewalk.finish(),
        curb: edging.finish(),
        white: white.finish(),
        yellow: yellow.finish(),
        islands: islands.finish(),
        layout,
        extent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertices(mesh: &Mesh) -> &[[f32; 3]] {
        match mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() {
            bevy::mesh::VertexAttributeValues::Float32x3(points) => points,
            _ => panic!("position format"),
        }
    }

    #[test]
    fn road_surface_covers_turning_vehicle_footprints_and_marks_both_directions() {
        let scene = junction_debug_scene::build().expect("reference scene");
        let revision = scene.session.world().revision();
        let road = build(&revision);
        let asphalt = vertices(&road.asphalt);
        let islands = vertices(&road.islands);
        let contains = |positions: &[[f32; 3]], point: Vec3| {
            positions.chunks_exact(3).any(|triangle| {
                let a = Vec3::from_array(triangle[0]);
                let b = Vec3::from_array(triangle[1]);
                let c = Vec3::from_array(triangle[2]);
                let sides = [
                    (b - a).cross(point - a).y,
                    (c - b).cross(point - b).y,
                    (a - c).cross(point - c).y,
                ];
                sides.iter().all(|side| *side >= -0.001) || sides.iter().all(|side| *side <= 0.001)
            })
        };
        let profile = scene
            .session
            .world()
            .traffic()
            .relations()
            .vehicle_profile(laneflow_static_contract::VehicleProfileOrdinal::from_raw(0))
            .expect("reference vehicle profile");
        let vehicle_length = profile.length_mm() as f32 / 1_000.0;
        let lane_pose = revision.spatial().unwrap().lane_pose().unwrap();
        for (name, ordinal) in junction_debug_scene::edge_ordinals(&revision) {
            if !name.contains('.') {
                continue;
            }
            let line: Vec<_> = lane_pose
                .lane_geometry(ordinal)
                .unwrap()
                .points()
                .iter()
                .map(|p| Vec3::new(p.x, p.y, p.z))
                .collect();
            for (index, point) in line.iter().enumerate() {
                let normal = lateral(&line, index);
                let forward = Vec3::new(normal.z, 0.0, -normal.x);
                // 与实际模型一致：规范位姿是前保险杠，车身向后延伸一整车长。
                for along in [0.0, -vehicle_length * 0.5, -vehicle_length] {
                    for side in [
                        -super::super::VEHICLE_WIDTH_METERS * 0.5,
                        0.0,
                        super::super::VEHICLE_WIDTH_METERS * 0.5,
                    ] {
                        let edge = *point + normal * side + forward * along;
                        assert!(
                            contains(asphalt, edge) && !contains(islands, edge),
                            "{name}: vehicle footprint outside road or on island at {edge:?}"
                        );
                    }
                }
            }
        }
        let paint = vertices(&road.white);
        for sx in [-1.0, 1.0] {
            for sz in [-1.0, 1.0] {
                assert!(
                    paint
                        .iter()
                        .any(|p| (50.0..120.0).contains(&(p[0] * sx))
                            && (p[2] - sz * 6.0).abs() < 0.15),
                    "both directions of both main-road arms need lane dividers"
                );
            }
        }
    }

    #[test]
    fn mirrored_street_corners_keep_upward_faces_and_tangent_curbs() {
        let layout = RoadLayout {
            main_half_width: 9.5,
            secondary_half_width: 7.75,
            junction_half_x: 30.5,
            junction_half_z: 30.5,
            main_arm_end: 150.0,
            secondary_arm_end: 150.0,
            corner_radius: 20.5,
        };
        for (sx, sz) in [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)] {
            let line = corner(layout, sx, sz, 150.0, 150.0);
            let first = (line[2] - line[1]).normalize();
            let last = (line[line.len() - 2] - line[line.len() - 3]).normalize();
            assert!(first.dot(Vec3::X * -sx) > 0.999);
            assert!(last.dot(Vec3::Z * sz) > 0.999);
            let mut surface = Surface::default();
            surface.fan(Vec3::Y * ROAD_Y, &line);
            assert!(surface.normals.iter().all(|normal| normal[1] > 0.99));
            let outside = -sx * sz;
            let mut edging = Surface::default();
            curb(&mut edging, &line, outside);
            let first_face_normal = Vec3::from_array(edging.normals[0]);
            assert!(first_face_normal.dot(lateral(&line, 0) * -outside) > 0.99);
        }
    }
}
