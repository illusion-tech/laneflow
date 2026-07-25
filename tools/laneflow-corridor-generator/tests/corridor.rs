use std::path::{Path, PathBuf};

use laneflow_corridor_generator::{CorridorConfig, generate};
use serde_json::Value;

const CONFIG: &str = include_str!("../../../examples/config/v0.9-signalized-corridor.toml");

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
    assert_eq!(first.traffic_bytes(), second.traffic_bytes());
    assert_eq!(first.spatial_bytes(), second.spatial_bytes());
    assert_eq!(first.manifest_bytes(), second.manifest_bytes());
    assert_eq!(first.catalog_bytes(), second.catalog_bytes());
}

#[test]
fn checked_in_artifacts_are_exact_generator_outputs() {
    let generated = default_generated();
    for (relative, expected) in [
        (
            "examples/data/v0.8-signalized-corridor.laneflow.json",
            generated.traffic_bytes(),
        ),
        (
            "examples/data/v0.1-signalized-corridor.spatial.json",
            generated.spatial_bytes(),
        ),
        (
            "examples/data/v0.1-signalized-corridor.scenario.json",
            generated.manifest_bytes(),
        ),
        (
            "examples/data/v0.2-signalized-corridor.catalog.toml",
            generated.catalog_bytes(),
        ),
    ] {
        let path = repository_path(relative);
        let actual = std::fs::read(&path).expect("checked-in artifact must be readable");
        assert_eq!(actual, expected, "{} is stale", path.display());
    }
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
        "spatial_artifact_ref = \"v0.8-signalized-corridor.laneflow.json\"",
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
