use std::path::{Path, PathBuf};

use laneflow_corridor_generator::{CorridorConfig, generate};
use serde_json::Value;

const CONFIG: &str = include_str!("../../../examples/config/v0.8-signalized-corridor.toml");

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
    assert_eq!(counts.edges, 54);
    assert_eq!(counts.routes, 14);
    assert_eq!(counts.stop_lines, 20);
    assert_eq!(counts.movement_gates, 20);
    assert_eq!(counts.signal_groups, 4);
    assert_eq!(counts.controllers, 2);
    assert_eq!(counts.phases, 12);
    assert_eq!(counts.portals, 6);
    assert_eq!(counts.spawn_slots, 230);
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
            "examples/data/v0.7-signalized-corridor.laneflow.json",
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
            "examples/data/v0.1-signalized-corridor.catalog.toml",
            generated.catalog_bytes(),
        ),
    ] {
        let path = repository_path(relative);
        let actual = std::fs::read(&path).expect("checked-in artifact must be readable");
        assert_eq!(actual, expected, "{} is stale", path.display());
    }
}

#[test]
fn default_corridor_locks_limits_routes_and_catalog_eligibility() {
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
    assert!(
        edges
            .iter()
            .any(|edge| { edge["id"] == "edge-main-w2e-lane-0-connector-intersection-1-straight" })
    );

    let catalog: laneflow_corridor_generator::CorridorCatalog =
        toml::from_str(std::str::from_utf8(generated.catalog_bytes()).expect("catalog is UTF-8"))
            .expect("catalog TOML must parse");
    assert_eq!(catalog.spawn_slots.len(), 230);
    assert!(catalog.spawn_slots.iter().all(|slot| {
        slot.route_edge_index == 0
            || (slot.route_id.starts_with("route-main-") && slot.route_edge_index == 2)
    }));
    assert!(
        catalog
            .spawn_slots
            .iter()
            .all(|slot| !slot.edge_id.contains("connector"))
    );
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
        "intersection_offsets_ms = [0, 0]",
        "intersection_offsets_ms = [58000, 0]",
    );
    assert!(CorridorConfig::parse(&offset).is_err());

    let conflict = CONFIG.replace(
        "spatial_artifact_ref = \"v0.1-signalized-corridor.spatial.json\"",
        "spatial_artifact_ref = \"v0.7-signalized-corridor.laneflow.json\"",
    );
    assert!(CorridorConfig::parse(&conflict).is_err());
}

#[test]
fn configuration_must_retain_at_least_two_hundred_spawn_slots() {
    let sparse = CONFIG.replace(
        "spawn_slot_pitch_meters = 20.0",
        "spawn_slot_pitch_meters = 40.0",
    );
    let config = CorridorConfig::parse(&sparse).expect("pitch is structurally valid");
    let error = generate(&config).expect_err("insufficient catalog capacity must fail");
    assert!(error.to_string().contains("at least 200"));
}
