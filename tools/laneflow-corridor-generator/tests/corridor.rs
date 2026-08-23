use std::path::{Path, PathBuf};

use laneflow_corridor_generator::{CorridorConfig, generate};
use serde_json::{Value, json};

const CONFIG: &str = include_str!("../../../examples/config/v0.10-signalized-corridor.toml");

fn repository_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn default_generated() -> laneflow_corridor_generator::GeneratedScenario {
    let config = CorridorConfig::parse(CONFIG).expect("default config must parse");
    generate(&config).expect("default corridor must generate")
}

#[test]
fn default_corridor_locks_scope_counts_and_deterministic_bytes() {
    let first = default_generated();
    let second = default_generated();
    let counts = first.counts();
    assert_eq!(counts.edges, 66);
    assert_eq!(counts.routes, 28);
    assert_eq!(counts.junctions, 2);
    assert_eq!(counts.movements, 24);
    assert_eq!(counts.maneuver_paths, 32);
    assert_eq!(counts.stop_lines, 20);
    assert_eq!(counts.maneuver_gates, 32);
    assert_eq!(counts.signal_groups, 8);
    assert_eq!(counts.controllers, 2);
    assert_eq!(counts.phases, 24);
    assert_eq!(counts.portals, 6);
    assert_eq!(counts.spawn_slots, 212);
    assert_eq!(counts.facility_bands, 7);
    assert_eq!(counts.road_sections, 14);
    assert_eq!(counts.lane_groups, 6);
    assert_eq!(counts.road_corridors, 7);
    assert_eq!(counts.access_rules, 18);
    assert_eq!(first.traffic_bytes(), second.traffic_bytes());
    assert_eq!(first.spatial_bytes(), second.spatial_bytes());
    assert_eq!(first.manifest_bytes(), second.manifest_bytes());
    assert_eq!(first.catalog_bytes(), second.catalog_bytes());
}

#[test]
fn checked_in_artifacts_are_exact_generator_outputs() {
    let generated = default_generated();
    let relative = "examples/data/v0.2-signalized-corridor.catalog.toml";
    let path = repository_path(relative);
    let actual = std::fs::read(&path).expect("checked-in artifact must be readable");
    assert_eq!(
        actual,
        generated.catalog_bytes(),
        "{} is stale",
        path.display()
    );
}

#[test]
fn default_corridor_locks_protected_turning_geometry_routes_and_signals() {
    let generated = default_generated();
    let traffic: Value =
        serde_json::from_slice(generated.traffic_bytes()).expect("traffic JSON must parse");
    let edges = traffic["laneGraph"]["edges"]
        .as_array()
        .expect("edges must be an array");
    assert!(edges.iter().any(|edge| {
        edge["id"] == "edge-main-w2e-lane-0-road-0"
            && edge["speedLimit"].as_f64() == Some(60.0 / 3.6)
    }));
    assert!(edges.iter().any(|edge| {
        edge["id"] == "edge-side-1-n2s-lane-0-road-0"
            && edge["speedLimit"].as_f64() == Some(40.0 / 3.6)
    }));
    for (id, expected_length, expected_speed) in [
        (
            "edge-junction-1-west-straight-lane-2-to-2-internal-0",
            21.0,
            60.0 / 3.6,
        ),
        (
            "edge-junction-1-north-straight-lane-1-to-1-internal-0",
            28.0,
            40.0 / 3.6,
        ),
        (
            "edge-junction-1-west-straight-lane-1-to-0-internal-0",
            21.345_867_633_819_58,
            60.0 / 3.6,
        ),
        (
            "edge-junction-1-west-left-lane-0-to-0-internal-0",
            22.076_601_803_302_765,
            25.0 / 3.6,
        ),
        (
            "edge-junction-1-west-right-lane-2-to-1-internal-0",
            8.246_497_988_700_867,
            15.0 / 3.6,
        ),
    ] {
        let edge = edges
            .iter()
            .find(|edge| edge["id"] == id)
            .expect("protected-turning edge must exist");
        assert_eq!(edge["length"].as_f64(), Some(expected_length));
        assert_eq!(edge["speedLimit"].as_f64(), Some(expected_speed));
    }
    assert_eq!(traffic["junctions"].as_array().map(Vec::len), Some(2));
    assert_eq!(traffic["movements"].as_array().map(Vec::len), Some(24));
    assert_eq!(traffic["maneuverPaths"].as_array().map(Vec::len), Some(32));
    let stop_lines = traffic["signals"]["stopLines"]
        .as_array()
        .expect("stop lines must be an array");
    let gates = traffic["signals"]["maneuverGates"]
        .as_array()
        .expect("maneuver gates must be an array");
    assert_eq!(stop_lines.len(), 20);
    assert_eq!(gates.len(), 32);
    assert!(gates.iter().all(|gate| gate["transitionIndex"] == 0));
    assert_eq!(traffic["waitingZones"], json!([]));
    assert!(gates.iter().all(|gate| {
        stop_lines
            .iter()
            .any(|stop_line| stop_line["id"] == gate["stopLineId"])
    }));
    for junction in 1..=2 {
        for (suffix, expected) in [
            ("main-left", 2),
            ("main-through-right", 8),
            ("secondary-left", 2),
            ("secondary-through-right", 4),
        ] {
            let group_id = format!("signal-group-junction-{junction}-{suffix}");
            assert_eq!(
                gates
                    .iter()
                    .filter(|gate| gate["signalControl"]["groupId"] == group_id)
                    .count(),
                expected
            );
        }
    }
    let routes = traffic["routes"]
        .as_array()
        .expect("routes must be an array");
    let occurrences = routes
        .iter()
        .flat_map(|route| {
            route["edgeIds"]
                .as_array()
                .expect("route edge IDs must be an array")
        })
        .filter_map(Value::as_str)
        .filter(|edge_id| edge_id.ends_with("-internal-0"))
        .collect::<Vec<_>>();
    assert_eq!(routes.len(), 28);
    assert_eq!(occurrences.len(), 44);
    assert_eq!(
        occurrences
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        32
    );
    for controller in traffic["signals"]["controllers"]
        .as_array()
        .expect("controllers must be an array")
    {
        let phases = controller["phases"]
            .as_array()
            .expect("phases must be an array");
        assert_eq!(phases.len(), 12);
        assert_eq!(
            phases
                .iter()
                .map(|phase| phase["durationMs"].as_u64().unwrap())
                .sum::<u64>(),
            84_000
        );
        assert!(phases.iter().all(|phase| {
            let states = phase["states"].as_array().unwrap();
            states.len() == 4
                && states
                    .iter()
                    .filter(|state| state["aspect"] != "red")
                    .count()
                    <= 1
        }));
    }

    let spatial: Value =
        serde_json::from_slice(generated.spatial_bytes()).expect("spatial JSON must parse");
    for (id, expected_points) in [
        ("edge-junction-1-west-straight-lane-2-to-2-internal-0", 2),
        ("edge-junction-1-north-straight-lane-1-to-1-internal-0", 2),
        ("edge-junction-1-west-straight-lane-1-to-0-internal-0", 65),
        ("edge-junction-1-west-left-lane-0-to-0-internal-0", 65),
        ("edge-junction-1-west-right-lane-2-to-1-internal-0", 65),
    ] {
        let edge = spatial["edges"]
            .as_array()
            .unwrap()
            .iter()
            .find(|edge| edge["trafficEdgeId"] == id)
            .expect("spatial edge must exist");
        assert_eq!(
            edge["centerline"]["points"].as_array().map(Vec::len),
            Some(expected_points)
        );
    }
}

#[test]
fn default_corridor_locks_explicit_cross_section_and_bus_lane_rules() {
    let generated = default_generated();
    let traffic: Value =
        serde_json::from_slice(generated.traffic_bytes()).expect("traffic JSON must parse");

    // 横断面数组非空且数量锁定：7 个物理 corridor 单元（主干 3 段 + 支路 4 段），
    // 每单元 2 个方向 section + 1 条中央分隔带；主干道每个 section 一个公交道组，
    // 每组 deny motorVehicle + allow bus + allow car 三条规则。
    let sections = traffic["roadSections"].as_array().expect("sections");
    let corridors = traffic["roadCorridors"].as_array().expect("corridors");
    let bands = traffic["facilityBands"].as_array().expect("bands");
    let groups = traffic["laneGroups"].as_array().expect("groups");
    let rules = traffic["accessRules"].as_array().expect("rules");
    assert_eq!(sections.len(), 14);
    assert_eq!(corridors.len(), 7);
    assert_eq!(bands.len(), 7);
    assert_eq!(groups.len(), 6);
    assert_eq!(rules.len(), 18);
    assert!(bands.iter().all(|band| band["kindId"] == "median"));
    assert!(
        sections
            .iter()
            .all(|section| section["kindId"] == "motorLane")
    );

    let section = |id: &str| {
        sections
            .iter()
            .find(|section| section["id"] == id)
            .expect("section must exist")
    };
    let lane_edges = |section: &Value| {
        section["lanes"]
            .as_array()
            .expect("lanes")
            .iter()
            .map(|lane| {
                lane["edgeIds"]
                    .as_array()
                    .expect("edge ids")
                    .iter()
                    .map(|edge_id| edge_id.as_str().expect("edge id string").to_owned())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };

    // corridor reference 方向：主干取 w2e、支路取 n2s；lane index 与 corridor
    // elements 都按 reference 系从左到右（generator 由几何派生：左 = up × tangent）。
    // w2e 自身即 reference：lane-0（中央侧）在左，lane-2（路缘侧）在右。
    assert_eq!(
        lane_edges(section("section-main-w2e-road-0")),
        [
            vec!["edge-main-w2e-lane-0-road-0"],
            vec!["edge-main-w2e-lane-1-road-0"],
            vec!["edge-main-w2e-lane-2-road-0"],
        ]
    );
    // e2w 与 reference 反向：lane index 相对其行驶方向反转，lane-2 在左。
    assert_eq!(
        lane_edges(section("section-main-e2w-road-0")),
        [
            vec!["edge-main-e2w-lane-2-road-0"],
            vec!["edge-main-e2w-lane-1-road-0"],
            vec!["edge-main-e2w-lane-0-road-0"],
        ]
    );
    // 支路 reference 为 n2s：s2n 整体在左，其 lane-1 最左。
    assert_eq!(
        lane_edges(section("section-side-1-s2n-road-0")),
        [
            vec!["edge-side-1-s2n-lane-1-road-0"],
            vec!["edge-side-1-s2n-lane-0-road-0"],
        ]
    );
    assert_eq!(
        lane_edges(section("section-side-1-n2s-road-0")),
        [
            vec!["edge-side-1-n2s-lane-0-road-0"],
            vec!["edge-side-1-n2s-lane-1-road-0"],
        ]
    );

    // corridor elements：反向 section 在左、中央分隔带居中、reference section 在右。
    // 对向 section 的 road 键与 reference 反转（segment 是方向 traversal 编号：
    // w2e road-0 最西而 e2w road-0 最东），物理上共延伸的配对是 w2e-0 ↔ e2w-4。
    let corridor = |id: &str| {
        corridors
            .iter()
            .find(|corridor| corridor["id"] == id)
            .expect("corridor must exist")
    };
    let main_corridor = corridor("corridor-main-road-0");
    assert_eq!(
        main_corridor["referenceSectionId"],
        "section-main-w2e-road-0"
    );
    assert_eq!(
        main_corridor["elements"]
            .as_array()
            .expect("elements")
            .iter()
            .map(|element| {
                element
                    .as_object()
                    .expect("element")
                    .keys()
                    .next()
                    .expect("one key")
                    .as_str()
            })
            .collect::<Vec<_>>(),
        ["sectionId", "bandId", "sectionId"]
    );
    assert_eq!(
        main_corridor["elements"][0]["sectionId"],
        "section-main-e2w-road-4"
    );
    assert_eq!(
        main_corridor["elements"][1]["bandId"],
        "band-main-median-road-0"
    );
    assert_eq!(
        main_corridor["elements"][2]["sectionId"],
        "section-main-w2e-road-0"
    );
    let side_corridor = corridor("corridor-side-1-road-0");
    assert_eq!(
        side_corridor["referenceSectionId"],
        "section-side-1-n2s-road-0"
    );
    assert_eq!(
        side_corridor["elements"][0]["sectionId"],
        "section-side-1-s2n-road-2"
    );
    assert!(corridors.iter().all(|corridor| {
        corridor["elements"]
            .as_array()
            .expect("elements")
            .iter()
            .any(|element| element.get("bandId").is_some())
    }));

    // 公交专用道：主干道每个 section 的路缘侧 lane 挂 LaneGroup——w2e 是 index 2
    // （reference 系最右），e2w 是 index 0（reference 系最右即其行驶方向路缘侧）。
    for (section_id, group_id, bus_index) in [
        ("section-main-w2e-road-0", "group-main-w2e-bus-road-0", 2),
        ("section-main-e2w-road-0", "group-main-e2w-bus-road-0", 0),
        ("section-main-w2e-road-4", "group-main-w2e-bus-road-4", 2),
        ("section-main-e2w-road-4", "group-main-e2w-bus-road-4", 0),
    ] {
        let section = section(section_id);
        let lanes = section["lanes"].as_array().expect("lanes");
        assert_eq!(lanes[bus_index]["laneGroupId"], json!(group_id));
        assert!(
            lanes
                .iter()
                .enumerate()
                .all(|(index, lane)| index == bus_index || lane.get("laneGroupId").is_none())
        );
        assert!(groups.iter().any(|group| {
            group["id"] == json!(group_id) && group["roadSectionId"] == json!(section_id)
        }));
    }

    // 每个公交道组都有 deny motorVehicle + allow bus 组合（外加演示车队的
    // allow car 豁免），target 均指向 laneGroup。
    for group in groups {
        let group_id = group["id"].as_str().expect("group id");
        let group_rules = rules
            .iter()
            .filter(|rule| rule["target"]["id"] == json!(group_id))
            .collect::<Vec<_>>();
        assert_eq!(group_rules.len(), 3);
        assert!(
            group_rules
                .iter()
                .all(|rule| rule["target"]["kind"] == "laneGroup")
        );
        assert!(group_rules.iter().any(|rule| {
            rule["effect"] == "deny" && rule["participantClassIds"] == json!(["motorVehicle"])
        }));
        assert!(group_rules.iter().any(|rule| {
            rule["effect"] == "allow" && rule["participantClassIds"] == json!(["bus"])
        }));
        assert!(group_rules.iter().any(|rule| {
            rule["effect"] == "allow" && rule["participantClassIds"] == json!(["car"])
        }));
    }

    // 参与者分类与 profile 归属：motorVehicle root + car/bus 子类。
    assert_eq!(
        traffic["participantClasses"],
        json!([
            { "id": "motorVehicle" },
            { "id": "car", "extendsId": "motorVehicle" },
            { "id": "bus", "extendsId": "motorVehicle" },
        ])
    );
    let profiles = traffic["vehicleProfiles"].as_array().expect("profiles");
    assert_eq!(profiles.len(), 2);
    assert_eq!(profiles[0]["participantClassId"], "car");
    assert_eq!(profiles[1]["participantClassId"], "bus");
}

#[test]
fn default_catalog_locks_physical_slots_lane_choices_and_weights() {
    let generated = default_generated();
    let catalog: laneflow_corridor_generator::CorridorCatalog =
        toml::from_str(std::str::from_utf8(generated.catalog_bytes()).expect("catalog is UTF-8"))
            .expect("catalog TOML must parse");
    assert_eq!(catalog.catalog_version, "0.2");
    assert_eq!(
        catalog
            .portals
            .iter()
            .map(|portal| portal.lanes.len())
            .sum::<usize>(),
        14
    );
    assert_eq!(catalog.spawn_slots.len(), 212);
    assert!(
        catalog
            .spawn_slots
            .iter()
            .all(|slot| slot.edge_id.ends_with("-road-0"))
    );
    assert_eq!(
        catalog
            .portals
            .iter()
            .map(|portal| {
                portal
                    .lanes
                    .iter()
                    .map(|lane| lane.route_choices.len())
                    .sum::<usize>()
            })
            .collect::<Vec<_>>(),
        [7, 7, 3, 4, 4, 3]
    );
    assert!(catalog.portals.iter().all(|portal| {
        portal
            .lanes
            .iter()
            .flat_map(|lane| &lane.route_choices)
            .map(|choice| choice.weight)
            .sum::<u64>()
            == 100
    }));
}

#[test]
fn config_rejects_unknown_fields_length_geometry_offsets_and_output_conflicts() {
    let unknown = CONFIG.replace(
        "fixed_delta_ms = 16",
        "fixed_delta_ms = 16\nfuture_field = true",
    );
    assert!(CorridorConfig::parse(&unknown).is_err());

    let too_long = CONFIG.replace("main_length_meters = 800.0", "main_length_meters = 1500.0");
    assert!(CorridorConfig::parse(&too_long).is_err());

    let overlap = CONFIG.replace(
        "intersection_x_meters = [-200.0, 200.0]",
        "intersection_x_meters = [-5.0, 5.0]",
    );
    assert!(CorridorConfig::parse(&overlap).is_err());

    let outside = CONFIG.replace(
        "intersection_x_meters = [-200.0, 200.0]",
        "intersection_x_meters = [-400.0, 200.0]",
    );
    assert!(CorridorConfig::parse(&outside).is_err());

    let offset = CONFIG.replace(
        "intersection_offsets_ms = [0, 42000]",
        "intersection_offsets_ms = [84000, 0]",
    );
    assert!(CorridorConfig::parse(&offset).is_err());

    let conflict = CONFIG.replace(
        "spatial_artifact_ref = \"v0.1-signalized-corridor.spatial.json\"",
        "spatial_artifact_ref = \"v0.10-signalized-corridor.laneflow.json\"",
    );
    assert!(CorridorConfig::parse(&conflict).is_err());
}

#[test]
fn configuration_must_retain_at_least_two_hundred_spawn_slots() {
    let sparse = CONFIG.replace(
        "spawn_slot_pitch_meters = 10.0",
        "spawn_slot_pitch_meters = 40.0",
    );
    let config = CorridorConfig::parse(&sparse).expect("pitch is structurally valid");
    let error = generate(&config).expect_err("insufficient catalog capacity must fail");
    assert!(error.to_string().contains("at least 200"));
}

#[test]
fn traffic_and_spatial_lengths_match_independently_for_all_66_edges() {
    let generated = default_generated();
    let traffic: Value =
        serde_json::from_slice(generated.traffic_bytes()).expect("traffic JSON must parse");
    let spatial: Value =
        serde_json::from_slice(generated.spatial_bytes()).expect("spatial JSON must parse");
    let traffic_lengths = traffic["laneGraph"]["edges"]
        .as_array()
        .expect("edges must be an array")
        .iter()
        .map(|edge| {
            (
                edge["id"].as_str().expect("traffic edge id"),
                edge["length"].as_f64().expect("traffic edge length"),
            )
        })
        .collect::<std::collections::HashMap<_, _>>();
    let spatial_edges = spatial["edges"].as_array().expect("edges must be an array");
    assert_eq!(traffic_lengths.len(), 66);
    assert_eq!(spatial_edges.len(), 66);

    let mut spatial_ids = std::collections::HashSet::new();
    for edge in spatial_edges {
        let id = edge["trafficEdgeId"].as_str().expect("spatial edge id");
        spatial_ids.insert(id);
        let points = edge["centerline"]["points"]
            .as_array()
            .expect("centerline points must be an array");
        assert!(points.len() >= 2, "edge {id} needs at least two points");
        // 从 spatial JSON 点列独立重算折线长度；Traffic length 由同一 f32 点列
        // 以 f32 hypot 求和，这里用 f64 重算，数值差在 1e-5 m 量级以内。
        let polyline_length = points
            .windows(2)
            .map(|pair| {
                let dx = pair[1][0].as_f64().expect("x") - pair[0][0].as_f64().expect("x");
                let dy = pair[1][1].as_f64().expect("y") - pair[0][1].as_f64().expect("y");
                let dz = pair[1][2].as_f64().expect("z") - pair[0][2].as_f64().expect("z");
                dx.hypot(dy).hypot(dz)
            })
            .sum::<f64>();
        let traffic_length = traffic_lengths
            .get(id)
            .copied()
            .expect("every spatial edge references a traffic edge");
        assert!(
            (polyline_length - traffic_length).abs() <= 1e-3,
            "edge {id}: spatial polyline {polyline_length} m vs traffic length {traffic_length} m"
        );
    }
    for id in traffic_lengths.keys() {
        assert!(
            spatial_ids.contains(id),
            "traffic edge {id} is missing from the spatial package"
        );
    }
}

#[test]
fn every_portal_lane_must_have_spawn_capacity() {
    let short_secondary_approaches = CONFIG
        .replace("main_length_meters = 800.0", "main_length_meters = 1800.0")
        .replace(
            "secondary_lengths_meters = [300.0, 300.0]",
            "secondary_lengths_meters = [29.0, 29.0]",
        );
    let config =
        CorridorConfig::parse(&short_secondary_approaches).expect("raw geometry remains valid");
    let error = generate(&config).expect_err("every portal lane needs at least one spawn slot");
    let message = error.to_string();
    assert!(message.contains("portal-side-1-north"));
    assert!(message.contains("at least 13 m"));
}

#[test]
fn corridor_paired_sections_share_physical_interval() {
    // 纵向共延伸不变量：corridor 的两个方向 section 必须占据同一物理分段。
    // segment 是方向 traversal 编号（w2e road-0 最西、e2w road-0 最东），若配对
    // 键不反转，两个 section 的几何中心会相距数百米而非仅一个路宽。
    let generated = default_generated();
    let traffic: Value =
        serde_json::from_slice(generated.traffic_bytes()).expect("traffic JSON must parse");
    let spatial: Value =
        serde_json::from_slice(generated.spatial_bytes()).expect("spatial JSON must parse");
    let spatial_edges = spatial["edges"].as_array().expect("spatial edges");
    let edge_midpoint = |edge_id: &str| {
        let edge = spatial_edges
            .iter()
            .find(|edge| edge["trafficEdgeId"] == edge_id)
            .expect("section lane edge must exist in spatial package");
        let points = edge["centerline"]["points"]
            .as_array()
            .expect("centerline points");
        let (first, last) = (
            points.first().expect("start point"),
            points.last().expect("end point"),
        );
        [
            (first[0].as_f64().expect("x") + last[0].as_f64().expect("x")) / 2.0,
            (first[1].as_f64().expect("y") + last[1].as_f64().expect("y")) / 2.0,
            (first[2].as_f64().expect("z") + last[2].as_f64().expect("z")) / 2.0,
        ]
    };
    let sections = traffic["roadSections"].as_array().expect("sections");
    let section_midpoint = |section_id: &str| {
        let section = sections
            .iter()
            .find(|section| section["id"] == section_id)
            .expect("corridor section must exist");
        let mut sum = [0.0_f64; 3];
        let mut count = 0_usize;
        for lane in section["lanes"].as_array().expect("lanes") {
            for edge_id in lane["edgeIds"].as_array().expect("edgeIds") {
                let midpoint = edge_midpoint(edge_id.as_str().expect("edge id"));
                for (axis, value) in sum.iter_mut().zip(midpoint) {
                    *axis += value;
                }
                count += 1;
            }
        }
        sum.map(|value| value / count as f64)
    };
    for corridor in traffic["roadCorridors"].as_array().expect("corridors") {
        let section_ids = corridor["elements"]
            .as_array()
            .expect("elements")
            .iter()
            .filter_map(|element| element["sectionId"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(section_ids.len(), 2, "corridor must pair two sections");
        let (reference, opposite) = (
            section_midpoint(section_ids[0]),
            section_midpoint(section_ids[1]),
        );
        let distance = (0..3)
            .map(|axis| (reference[axis] - opposite[axis]).powi(2))
            .sum::<f64>()
            .sqrt();
        assert!(
            distance < 20.0,
            "corridor {} sections {:?} are {distance:.1} m apart, not co-extensive",
            corridor["id"],
            section_ids
        );
    }
}
