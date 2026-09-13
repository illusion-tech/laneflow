//! 实体灯具按进口布置；信号组绑定决定灯态，停止线不决定灯杆安装位置。
use std::collections::BTreeMap;

use bevy::{mesh::Indices, prelude::*};
use laneflow_static_contract::ManeuverPathOrdinal;
use laneflow_static_network::SharedNetworkRevision;

use super::{LampKind, TrafficLamp, junction_road::RoadLayout, lamp_material};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum Turn {
    Left,
    Through,
    Right,
}

#[derive(Clone, Copy, Debug)]
struct Head {
    group: u32,
    turn: Turn,
    lateral: f32,
    permissive: bool,
}

#[derive(Debug)]
struct Fixture {
    #[cfg(test)]
    approach: Vec3,
    direction: Vec3,
    pole: Vec3,
    heads: Vec<Head>,
}

fn fixtures(revision: &SharedNetworkRevision, layout: RoadLayout) -> Vec<Fixture> {
    let traffic = revision.traffic();
    let relations = traffic.relations();
    let pose = revision.spatial().unwrap().lane_pose().unwrap();
    let mut by_approach = BTreeMap::<(i32, i32), Fixture>::new();
    for raw in 0..traffic.maneuvers().maneuver_path_count() {
        let ordinal = ManeuverPathOrdinal::from_raw(raw);
        let path = traffic.maneuvers().maneuver_path(ordinal).unwrap();
        let incoming = pose.lane_geometry(path.edges()[0]).unwrap().points();
        let outgoing = pose
            .lane_geometry(*path.edges().last().unwrap())
            .unwrap()
            .points();
        let point = |p: &laneflow_static_network::CanonicalPoint| Vec3::new(p.x, p.y, p.z);
        let approach = point(incoming.last().unwrap());
        let direction = (approach - point(&incoming[incoming.len() - 2])).normalize();
        let exit = (point(&outgoing[1]) - point(&outgoing[0])).normalize();
        let right = direction.cross(Vec3::Y);
        let turn = if direction.cross(exit).y > 0.25 {
            Turn::Left
        } else if direction.cross(exit).y < -0.25 {
            Turn::Right
        } else {
            Turn::Through
        };
        // 待转路径的实体左转灯绑定 release；admission/entry 是区内准入约束。
        let Some(group) = path.maneuver_gates().iter().rev().find_map(|gate| {
            relations
                .maneuver_gate(*gate)
                .and_then(|view| view.signal_group())
        }) else {
            // 外环汇合通过无信号准入资源控制，不布置虚构灯具。
            continue;
        };
        let main = direction.x.abs() > 0.5;
        let longitudinal = if main {
            layout.junction_half_x
        } else {
            layout.junction_half_z
        };
        let half_width = if main {
            layout.main_half_width
        } else {
            layout.secondary_half_width
        };
        let key = (direction.x.round() as i32, direction.z.round() as i32);
        let fixture = by_approach.entry(key).or_insert_with(|| Fixture {
            #[cfg(test)]
            approach,
            direction,
            pole: direction * (longitudinal + 4.0) + right * (half_width + 1.2),
            heads: Vec::new(),
        });
        let lateral = approach.dot(right);
        if let Some(head) = fixture
            .heads
            .iter_mut()
            .find(|head| head.group == group.raw() && head.turn == turn)
        {
            head.lateral = head.lateral.max(lateral);
        } else {
            fixture.heads.push(Head {
                group: group.raw(),
                turn,
                lateral,
                // 参考场景带冲突通行段的左转为许可左转，使用圆灯而非保护左转箭头。
                permissive: turn == Turn::Left
                    && revision
                        .conflict()
                        .maneuver_path_participant_streams(ordinal)
                        .is_some_and(|streams| !streams.is_empty()),
            });
        }
    }
    let mut result: Vec<_> = by_approach.into_values().collect();
    for fixture in &mut result {
        fixture.heads.sort_by_key(|head| head.turn);
        if fixture.heads.len() > 1
            && (fixture.heads[0].lateral - fixture.heads[1].lateral).abs() < 1.0
        {
            fixture.heads[0].lateral -= 1.1;
            fixture.heads[1].lateral += 1.1;
        }
    }
    result
}

fn arrow_mesh(turn: Turn) -> Mesh {
    let outline = [
        Vec2::new(0.30, 0.08),
        Vec2::new(-0.05, 0.08),
        Vec2::new(-0.05, 0.25),
        Vec2::new(-0.36, 0.0),
        Vec2::new(-0.05, -0.25),
        Vec2::new(-0.05, -0.08),
        Vec2::new(0.30, -0.08),
    ];
    let mut positions = Vec::new();
    for index in 0..outline.len() {
        let map = |point: Vec2| {
            let point = match turn {
                Turn::Left => point,
                Turn::Right => Vec2::new(-point.x, point.y),
                Turn::Through => Vec2::new(point.y, -point.x),
            };
            Vec3::new(point.x, point.y, 0.0)
        };
        let mut a = map(outline[index]);
        let mut b = map(outline[(index + 1) % outline.len()]);
        if a.cross(b).z < 0.0 {
            std::mem::swap(&mut a, &mut b);
        }
        positions.extend([Vec3::ZERO.to_array(), a.to_array(), b.to_array()]);
    }
    let mut mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        vec![[0.0, 0.0, 1.0]; positions.len()],
    );
    mesh.insert_indices(Indices::U32((0..positions.len() as u32).collect()));
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh
}

fn tube(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: Handle<StandardMaterial>,
    start: Vec3,
    end: Vec3,
    radius: f32,
) {
    commands.spawn((
        Mesh3d(meshes.add(Cylinder::new(radius, start.distance(end)))),
        MeshMaterial3d(material),
        Transform::from_translation((start + end) * 0.5)
            .with_rotation(Quat::from_rotation_arc(Vec3::Y, (end - start).normalize())),
    ));
}

pub fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    revision: &SharedNetworkRevision,
    layout: RoadLayout,
) {
    let steel = materials.add(StandardMaterial {
        base_color: Color::srgb(0.55, 0.58, 0.59),
        metallic: 0.65,
        perceptual_roughness: 0.45,
        ..default()
    });
    let dark = materials.add(StandardMaterial::from_color(Color::srgb(
        0.025, 0.03, 0.035,
    )));
    let concrete = materials.add(StandardMaterial::from_color(Color::srgb(0.55, 0.54, 0.50)));
    let white = materials.add(StandardMaterial::from_color(Color::srgb(0.88, 0.9, 0.9)));
    let housing = meshes.add(Cuboid::new(0.92, 2.10, 0.40));
    let backing = meshes.add(Cuboid::new(1.05, 2.23, 0.10));
    let visor = meshes.add(Cuboid::new(0.79, 0.10, 0.64));
    let disc = meshes.add(Cylinder::new(0.245, 0.035));
    let left = meshes.add(arrow_mesh(Turn::Left));
    let right_arrow = meshes.add(arrow_mesh(Turn::Right));
    for fixture in fixtures(revision, layout) {
        let right = fixture.direction.cross(Vec3::Y);
        let station = fixture.pole.dot(fixture.direction);
        let first = fixture
            .heads
            .iter()
            .map(|head| head.lateral)
            .fold(f32::MAX, f32::min);
        let top = fixture.pole + Vec3::Y * 8.6;
        let beam_end = fixture.direction * station + right * (first - 1.1) + Vec3::Y * 8.6;
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(0.65, 0.45, 0.65))),
            MeshMaterial3d(concrete.clone()),
            Transform::from_translation(fixture.pole + Vec3::Y * 0.405),
        ));
        tube(
            commands,
            meshes,
            steel.clone(),
            fixture.pole + Vec3::Y * 0.5,
            top + Vec3::Y * 0.3,
            0.16,
        );
        tube(commands, meshes, steel.clone(), top, beam_end, 0.12);
        for head in fixture.heads {
            let center = fixture.direction * station + right * head.lateral + Vec3::Y * 7.25;
            let rotation = Transform::IDENTITY
                .looking_to(fixture.direction, Vec3::Y)
                .rotation;
            let local = |point: Vec3| {
                Transform::from_translation(center + rotation * point).with_rotation(rotation)
            };
            tube(
                commands,
                meshes,
                steel.clone(),
                center + Vec3::Y,
                center + Vec3::Y * 1.35,
                0.06,
            );
            commands.spawn((
                Mesh3d(backing.clone()),
                MeshMaterial3d(steel.clone()),
                local(Vec3::new(0.0, 0.0, -0.25)),
            ));
            commands.spawn((
                Mesh3d(housing.clone()),
                MeshMaterial3d(dark.clone()),
                local(Vec3::ZERO),
            ));
            for (kind, lift) in [
                (LampKind::Red, 0.62),
                (LampKind::Yellow, 0.0),
                (LampKind::Green, -0.62),
            ] {
                commands.spawn((
                    Mesh3d(visor.clone()),
                    MeshMaterial3d(dark.clone()),
                    local(Vec3::new(0.0, lift + 0.27, 0.33)),
                ));
                let material = materials.add(lamp_material(false, kind));
                let circular = head.turn == Turn::Through || head.permissive;
                let lens = if circular {
                    disc.clone()
                } else if head.turn == Turn::Left {
                    left.clone()
                } else {
                    right_arrow.clone()
                };
                let mut transform = local(Vec3::new(0.0, lift, 0.235));
                if circular {
                    transform.rotation *= Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
                }
                commands.spawn((
                    Mesh3d(lens),
                    MeshMaterial3d(material.clone()),
                    transform,
                    TrafficLamp {
                        group: head.group,
                        kind,
                        material,
                        lit: false,
                    },
                ));
            }
            if head.permissive {
                // 圆灯配固定左转方向牌：不把仍须让行的许可左转画成保护箭头灯。
                commands.spawn((
                    Mesh3d(meshes.add(Cuboid::new(0.9, 0.65, 0.08))),
                    MeshMaterial3d(dark.clone()),
                    local(Vec3::new(0.0, -1.6, 0.0)),
                ));
                commands.spawn((
                    Mesh3d(left.clone()),
                    MeshMaterial3d(white.clone()),
                    local(Vec3::new(0.0, -1.6, 0.06)),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_far_side_fixtures_cover_every_approach_and_waiting_release() {
        let scene = super::super::junction_debug_scene::build().unwrap();
        let revision = scene.session.world().revision();
        let layout = super::super::junction_road::build(&revision).layout;
        let fixtures = fixtures(&revision, layout);
        assert_eq!(fixtures.len(), 4);
        assert_eq!(
            fixtures
                .iter()
                .map(|fixture| fixture.heads.len())
                .sum::<usize>(),
            7
        );
        for fixture in &fixtures {
            assert!(fixture.approach.dot(fixture.direction) < 0.0);
            assert!(fixture.pole.dot(fixture.direction) > 30.0);
            let half_width = if fixture.direction.x.abs() > 0.5 {
                layout.main_half_width
            } else {
                layout.secondary_half_width
            };
            assert!(fixture.pole.dot(fixture.direction.cross(Vec3::Y)) > half_width);
        }
        let relations = revision.traffic().relations();
        let zone = relations
            .waiting_zone(laneflow_static_contract::WaitingZoneOrdinal::from_raw(0))
            .unwrap();
        let entry_group = relations
            .maneuver_gate(zone.entry_gate())
            .unwrap()
            .signal_group()
            .unwrap()
            .raw();
        let release_group = relations
            .maneuver_gate(zone.release_gate())
            .unwrap()
            .signal_group()
            .unwrap()
            .raw();
        assert!(
            !fixtures
                .iter()
                .flat_map(|fixture| &fixture.heads)
                .any(|head| head.group == entry_group)
        );
        let west = fixtures
            .iter()
            .find(|fixture| fixture.direction.x > 0.5)
            .unwrap();
        assert!(
            west.heads
                .iter()
                .any(|head| head.turn == Turn::Left && head.group == release_group)
        );
    }
}
