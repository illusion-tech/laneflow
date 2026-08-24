use std::path::{Path, PathBuf};

use laneflow_compiler::{CanonicalIdentityFieldView, ValidatedCanonicalLir};
use laneflow_corridor_generator::{CorridorConfig, generate};
use laneflow_static_contract::FieldTag;

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

fn ascii_field<'a>(
    fields: impl IntoIterator<Item = CanonicalIdentityFieldView<'a>>,
    tag: FieldTag,
) -> String {
    let field = fields
        .into_iter()
        .find(|field| field.tag() == tag)
        .expect("identity field");
    std::str::from_utf8(field.value_bytes())
        .expect("ascii identity field")
        .to_owned()
}

fn edge_key<'a>(
    lir: &'a ValidatedCanonicalLir,
    needle: &str,
) -> laneflow_compiler::CanonicalLaneEdgeView<'a> {
    lir.lane_edges()
        .find(|edge| ascii_field(edge.identity_fields(), FieldTag::LaneEdgeKey) == needle)
        .unwrap_or_else(|| panic!("missing lane edge {needle}"))
}

fn section_key<'a>(
    lir: &'a ValidatedCanonicalLir,
    needle: &str,
) -> laneflow_compiler::CanonicalRoadSectionView<'a> {
    lir.road_sections()
        .find(|section| ascii_field(section.identity_fields(), FieldTag::SectionKey) == needle)
        .expect("section key")
}

fn corridor_key<'a>(
    lir: &'a ValidatedCanonicalLir,
    needle: &str,
) -> laneflow_compiler::CanonicalRoadCorridorView<'a> {
    lir.road_corridors()
        .find(|corridor| ascii_field(corridor.identity_fields(), FieldTag::CorridorKey) == needle)
        .expect("corridor key")
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
    assert_eq!(first.catalog_bytes(), second.catalog_bytes());
    assert_eq!(first.lfca_bytes(), second.lfca_bytes());
    let (first_lfsm, first_lfsd) = first.emit_portable_sidecars().expect("sidecars");
    let (second_lfsm, second_lfsd) = second.emit_portable_sidecars().expect("sidecars");
    assert_eq!(first_lfsm, second_lfsm);
    assert_eq!(first_lfsd, second_lfsd);
    assert!(!first_lfsm.is_empty());
    assert!(!first_lfsd.is_empty());
    let lir = first.lir();
    assert_eq!(lir.lane_edges().len(), 66);
    assert_eq!(lir.static_routes().len(), 28);
    assert_eq!(lir.junctions().len(), 2);
    assert_eq!(lir.movements().len(), 24);
    assert_eq!(lir.maneuver_paths().len(), 32);
    assert_eq!(lir.stop_lines().len(), 20);
    assert_eq!(lir.maneuver_gates().len(), 32);
    assert_eq!(lir.signal_groups().len(), 8);
    assert_eq!(lir.signal_controllers().len(), 2);
    assert_eq!(lir.signal_phases().len(), 24);
    assert_eq!(lir.waiting_zones().len(), 0);
}

#[test]
fn checked_in_artifacts_are_exact_generator_outputs() {
    let generated = default_generated();
    for (relative, bytes) in [
        (
            "examples/data/v0.2-signalized-corridor.catalog.toml",
            generated.catalog_bytes(),
        ),
        (
            "examples/data/v0.2-signalized-corridor.lfca",
            generated.lfca_bytes(),
        ),
    ] {
        let path = repository_path(relative);
        let actual = std::fs::read(&path).unwrap_or_default();
        assert_eq!(
            actual,
            bytes,
            "{} is stale; run generator generate",
            path.display()
        );
    }
}

#[test]
fn default_corridor_locks_protected_turning_geometry_routes_and_signals() {
    let generated = default_generated();
    let lir = generated.lir();
    let main = edge_key(lir, "edge-main-w2e-lane-0-road-0");
    assert!((main.speed_limit_meters_per_second() - 60.0 / 3.6).abs() < 1e-12);
    let side = edge_key(lir, "edge-side-1-n2s-lane-0-road-0");
    assert!((side.speed_limit_meters_per_second() - 40.0 / 3.6).abs() < 1e-12);
    for (id, expected_length, expected_speed) in [
        (
            "edge-junction-1-west-straight-lane-2-to-2-i0",
            21.0,
            60.0 / 3.6,
        ),
        (
            "edge-junction-1-north-straight-lane-1-to-1-i0",
            28.0,
            40.0 / 3.6,
        ),
        (
            "edge-junction-1-west-straight-lane-1-to-0-i0",
            21.345_867_633_819_58,
            60.0 / 3.6,
        ),
        (
            "edge-junction-1-west-left-lane-0-to-0-i0",
            22.076_601_803_302_765,
            25.0 / 3.6,
        ),
        (
            "edge-junction-1-west-right-lane-2-to-1-i0",
            8.246_497_988_700_867,
            15.0 / 3.6,
        ),
    ] {
        let edge = edge_key(lir, id);
        assert_eq!(edge.length_meters(), expected_length);
        assert!((edge.speed_limit_meters_per_second() - expected_speed).abs() < 1e-12);
    }
    assert!(
        lir.maneuver_gates()
            .all(|gate| gate.transition_index() == 0)
    );
    let mut internal_occurrences = 0;
    let mut unique_internals = std::collections::HashSet::new();
    for route in lir.static_routes() {
        for edge in route.edges() {
            let view = lir.lane_edge(*edge).expect("route edge");
            let key = ascii_field(view.identity_fields(), FieldTag::LaneEdgeKey);
            if key.ends_with("-i0") {
                internal_occurrences += 1;
                unique_internals.insert(key);
            }
        }
    }
    assert_eq!(lir.static_routes().len(), 28);
    assert_eq!(internal_occurrences, 44);
    assert_eq!(unique_internals.len(), 32);
    for controller in lir.signal_controllers() {
        assert_eq!(controller.phases().len(), 12);
        assert_eq!(controller.cycle_duration_ms(), 84_000);
    }
    for (id, expected_points) in [
        ("edge-junction-1-west-straight-lane-2-to-2-i0", 2),
        ("edge-junction-1-north-straight-lane-1-to-1-i0", 2),
        ("edge-junction-1-west-straight-lane-1-to-0-i0", 65),
        ("edge-junction-1-west-left-lane-0-to-0-i0", 65),
        ("edge-junction-1-west-right-lane-2-to-1-i0", 65),
    ] {
        let edge = edge_key(lir, id);
        let geometry = edge
            .spatial_geometry()
            .expect("corridor edges have spatial geometry");
        assert_eq!(geometry.points().len(), expected_points);
    }
}

#[test]
fn default_corridor_locks_explicit_cross_section_and_bus_lane_rules() {
    let generated = default_generated();
    let lir = generated.lir();
    assert_eq!(lir.road_sections().len(), 14);
    assert_eq!(lir.road_corridors().len(), 7);
    assert_eq!(lir.facility_bands().len(), 7);
    assert_eq!(lir.lane_groups().len(), 6);
    assert_eq!(lir.access_rules().len(), 18);
    assert!(lir.facility_bands().all(|band| band.kind_id() == "median"));
    assert!(
        lir.road_sections()
            .all(|section| section.kind_id() == "motorLane")
    );

    let lane_edges = |section_id: &str| {
        let section = section_key(lir, section_id);
        section
            .lanes()
            .iter()
            .map(|lane| {
                lir.authoring_lane(*lane)
                    .expect("authoring lane")
                    .edge_chain()
                    .iter()
                    .map(|edge| {
                        ascii_field(
                            lir.lane_edge(*edge).expect("edge").identity_fields(),
                            FieldTag::LaneEdgeKey,
                        )
                        .to_owned()
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        lane_edges("section-main-w2e-road-0"),
        [
            vec!["edge-main-w2e-lane-0-road-0".to_owned()],
            vec!["edge-main-w2e-lane-1-road-0".to_owned()],
            vec!["edge-main-w2e-lane-2-road-0".to_owned()],
        ]
    );
    assert_eq!(
        lane_edges("section-main-e2w-road-0"),
        [
            vec!["edge-main-e2w-lane-2-road-0".to_owned()],
            vec!["edge-main-e2w-lane-1-road-0".to_owned()],
            vec!["edge-main-e2w-lane-0-road-0".to_owned()],
        ]
    );

    let main_corridor = corridor_key(lir, "corridor-main-road-0");
    assert_eq!(
        ascii_field(
            lir.road_section(main_corridor.reference_section())
                .expect("reference section")
                .identity_fields(),
            FieldTag::SectionKey
        ),
        "section-main-w2e-road-0"
    );
    let elements: Vec<_> = main_corridor.elements().collect();
    assert_eq!(elements.len(), 3);

    assert_eq!(lir.participant_classes().len(), 3);
    assert_eq!(lir.vehicle_profiles().len(), 2);
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
        "lfca_file_name = \"v0.2-signalized-corridor.lfca\"",
        "lfca_file_name = \"v0.2-signalized-corridor.catalog.toml\"",
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
    let Err(error) = generate(&config) else {
        panic!("insufficient catalog capacity must fail");
    };
    assert!(error.to_string().contains("at least 200"));
}

#[test]
fn traffic_and_spatial_lengths_match_independently_for_all_66_edges() {
    let generated = default_generated();
    let lir = generated.lir();
    assert_eq!(lir.lane_edges().len(), 66);
    for edge in lir.lane_edges() {
        let geometry = edge.spatial_geometry().expect("every edge has geometry");
        let points: Vec<_> = geometry.points().collect();
        assert!(points.len() >= 2);
        let polyline = points
            .windows(2)
            .map(|pair| {
                let dx = f64::from(pair[1].x - pair[0].x);
                let dy = f64::from(pair[1].y - pair[0].y);
                let dz = f64::from(pair[1].z - pair[0].z);
                dx.hypot(dy).hypot(dz)
            })
            .sum::<f64>();
        assert!(
            (polyline - edge.length_meters()).abs() <= 1e-3,
            "edge {}: spatial polyline {polyline} m vs traffic length {}",
            ascii_field(edge.identity_fields(), FieldTag::LaneEdgeKey),
            edge.length_meters()
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
    let Err(error) = generate(&config) else {
        panic!("every portal lane needs at least one spawn slot");
    };
    let message = error.to_string();
    assert!(message.contains("portal-side-1-north"));
    assert!(message.contains("at least 13 m"));
}

#[test]
fn identity_ascii_keys_stay_below_compile_string_limit() {
    use laneflow_compiler::CompileLimits;

    let generated = default_generated();
    let limit = CompileLimits::p100_initial_v1().max_single_string_bytes();
    let mut longest = 0_u64;
    for edge in generated.lir().lane_edges() {
        let key = ascii_field(edge.identity_fields(), FieldTag::LaneEdgeKey);
        longest = longest.max(u64::try_from(key.len()).expect("key length"));
        assert!(
            u64::try_from(key.len()).expect("key length") < limit,
            "lane edge key {key:?} must stay below {limit} bytes"
        );
    }
    assert!(
        longest + 8 <= limit,
        "longest identity key is {longest} bytes; leave headroom under {limit}"
    );
}

#[test]
fn catalog_bind_spawns_few_vehicles_and_steps() {
    use std::sync::Arc;

    use laneflow_format::{FormatLimits, check_canonical_network_input_v1};
    use laneflow_runtime::{TickInput, TrafficWorld, VehicleSpawnInput, WorldConfig};
    use laneflow_scenario::signalized_corridor::{PASSENGER_CAR_PROFILE_KEY, bind};
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
        build_shared_network_revision,
    };

    let generated = default_generated();
    let catalog: laneflow_corridor_generator::CorridorCatalog =
        toml::from_str(std::str::from_utf8(generated.catalog_bytes()).expect("catalog is UTF-8"))
            .expect("catalog TOML must parse");
    let input = check_canonical_network_input_v1(generated.lfca_bytes(), FormatLimits::V1_HARD)
        .expect("checked LFCA");
    let revision = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("shared network revision");
    let bound = bind(&catalog, &revision).expect("prepare bind");
    assert_eq!(bound.network_revision, revision.network_revision());
    assert_eq!(bound.spawn_slots.len(), 212);
    assert_eq!(bound.routes.len(), 28);
    let profile = *bound
        .profiles
        .get(PASSENGER_CAR_PROFILE_KEY)
        .expect("passenger-car profile");

    let mut world = TrafficWorld::install(Arc::clone(&revision), WorldConfig::new(8, 32, 1, 16))
        .expect("install");
    assert_eq!(world.revision().network_revision(), bound.network_revision);

    for slot in bound.spawn_slots.iter().take(3) {
        let edges = world
            .traffic()
            .relations()
            .static_route_edges(slot.entry_route)
            .expect("route edges");
        let index = edges
            .iter()
            .position(|edge| *edge == slot.edge)
            .expect("slot edge is on its entry route");
        assert_eq!(index, 0, "catalog slots bind to the route entry edge");
        let route = world.static_route(slot.entry_route).expect("static route");
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                profile,
                route,
                u32::try_from(index).expect("edge index"),
                slot.progress,
                0.0,
            ))
            .expect("catalog slot must spawn");
    }
    world.step(TickInput::new(16)).expect("step");
    assert!(!world.committed_pose_sources().as_slice().is_empty());
}
