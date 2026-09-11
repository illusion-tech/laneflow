//! 复杂路口生成器测试：计数与字节锁定、检入制品新鲜度、catalog 校验、
//! LFCA 装载、bind + spawn/step，以及焦点路线的重复 Gate occurrence。

use std::path::{Path, PathBuf};

use laneflow_junction_generator::{JunctionCatalog, JunctionConfig, generate};
use laneflow_scenario::complex_junction::{
    FOCUS_ROUTE_IDS, MIN_SPAWN_SLOT_COUNT, PORTAL_LANE_COUNTS, VEHICLE_PROFILE_KEY,
};

const CONFIG: &str = include_str!("../../../examples/config/v0.1-complex-junction.toml");

fn repository_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn default_generated() -> laneflow_junction_generator::GeneratedScenario {
    let config = JunctionConfig::parse(CONFIG).expect("default config must parse");
    generate(&config).expect("default junction must generate")
}

fn default_catalog() -> JunctionCatalog {
    toml::from_str(
        std::str::from_utf8(default_generated().catalog_bytes()).expect("catalog is UTF-8"),
    )
    .expect("catalog TOML must parse")
}

fn install_fixture(
    revision: std::sync::Arc<laneflow_static_network::SharedNetworkRevision>,
    config: laneflow_runtime::WorldConfig,
) -> Result<laneflow_runtime::TrafficWorld, laneflow_runtime::InstallError> {
    let origin = *revision.canonical_origin();
    laneflow_runtime::TrafficWorld::install(
        revision,
        config,
        laneflow_runtime::CommittedNetworkSource::Published {
            reference: laneflow_runtime::PublishedLfcaReference::new(
                "fixture://in-process",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .expect("non-empty fixture key"),
        },
        0,
        toml::from_str::<JunctionCatalog>(include_str!(
            "../../../examples/data/v0.1-complex-junction.catalog.toml"
        ))
        .unwrap()
        .policy_selection
        .resolve()
        .unwrap(),
    )
}

#[test]
fn default_junction_locks_scope_counts_and_deterministic_bytes() {
    let first = default_generated();
    let second = default_generated();
    let counts = first.counts();
    assert_eq!(counts.edges, 31);
    assert_eq!(counts.movements, 7);
    assert_eq!(counts.maneuver_paths, 9);
    assert_eq!(counts.maneuver_gates, 11);
    assert_eq!(counts.stop_lines, 8);
    assert_eq!(counts.waiting_zones, 1);
    assert_eq!(counts.conflict_zones, 3);
    assert_eq!(counts.streams, 4);
    assert_eq!(counts.signal_groups, 4);
    assert_eq!(counts.controllers, 1);
    assert_eq!(counts.phases, 9);
    assert_eq!(counts.routes, 11);
    assert_eq!(counts.portals, 4);
    assert!(counts.spawn_slots >= MIN_SPAWN_SLOT_COUNT);
    assert_eq!(first.catalog_bytes(), second.catalog_bytes());
    assert_eq!(first.lfca_bytes(), second.lfca_bytes());
    let (first_lfsm, first_lfsd) = first.emit_portable_sidecars().expect("sidecars");
    let (second_lfsm, second_lfsd) = second.emit_portable_sidecars().expect("sidecars");
    assert_eq!(first_lfsm, second_lfsm);
    assert_eq!(first_lfsd, second_lfsd);
    assert!(!first_lfsm.is_empty());
    assert!(!first_lfsd.is_empty());
    let lir = first.lir();
    assert_eq!(lir.lane_edges().len(), 31);
    assert_eq!(lir.junctions().len(), 1);
    assert_eq!(lir.movements().len(), 7);
    assert_eq!(lir.maneuver_paths().len(), 9);
    assert_eq!(lir.stop_lines().len(), 8);
    assert_eq!(lir.maneuver_gates().len(), 11);
    assert_eq!(lir.signal_groups().len(), 4);
    assert_eq!(lir.signal_controllers().len(), 1);
    assert_eq!(lir.signal_phases().len(), 9);
    assert_eq!(lir.waiting_zones().len(), 1);
}

#[test]
fn checked_in_artifacts_are_exact_generator_outputs() {
    let generated = default_generated();
    for (relative, bytes) in [
        (
            "examples/data/v0.1-complex-junction.catalog.toml",
            generated.catalog_bytes(),
        ),
        (
            "examples/data/v0.1-complex-junction.lfca",
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
fn catalog_round_trips_validates_and_marks_two_focus_routes() {
    let catalog = default_catalog();
    laneflow_scenario::complex_junction::validate(&catalog).expect("catalog must validate");
    let encoded = toml::to_string(&catalog).expect("catalog encodes");
    let decoded: JunctionCatalog = toml::from_str(&encoded).expect("catalog re-decodes");
    assert_eq!(
        decoded, catalog,
        "TOML round trip changed catalog semantics"
    );
    assert_eq!(catalog.routes.len(), 11);
    assert_eq!(catalog.portals.len(), 4);
    for (portal, expected_lanes) in catalog.portals.iter().zip(PORTAL_LANE_COUNTS) {
        assert_eq!(portal.lanes.len(), expected_lanes, "portal {:?}", portal.id);
    }
    // 两条东→西直行车道都必须有路线经过（第二条车道承载许可左转的冲突流）。
    for entry_edge in ["e-in-i0", "e-in-i1"] {
        assert!(
            catalog
                .routes
                .iter()
                .any(|route| route.edge_ids.iter().any(|edge| edge == entry_edge)),
            "no catalog route passes through {entry_edge}"
        );
    }
    let focus: Vec<&str> = catalog
        .routes
        .iter()
        .filter(|route| route.focus)
        .map(|route| route.route_id.as_str())
        .collect();
    assert_eq!(focus, FOCUS_ROUTE_IDS);
    // 同 portal 两条环路共享出口/入口焊接点：lane 1 槽位相位错开半个 pitch，
    // 默认配置下 lane 0 首槽 6.5 m、lane 1 首槽 11.5 m。
    for portal in &catalog.portals {
        for lane in &portal.lanes {
            let first = catalog
                .spawn_slots
                .iter()
                .filter(|slot| slot.portal_id == portal.id && slot.lane_index == lane.lane_index)
                .map(|slot| slot.progress)
                .fold(f64::INFINITY, f64::min);
            let expected = 6.5 + lane.lane_index as f64 * 5.0;
            assert_eq!(
                first, expected,
                "portal {:?} lane {} first slot progress",
                portal.id, lane.lane_index
            );
        }
    }
}

#[test]
fn config_rejects_unknown_fields() {
    let unknown = format!("{CONFIG}\n[geometry.bogus]\n");
    let error = JunctionConfig::parse(&unknown).expect_err("unknown field must fail");
    assert!(error.to_string().contains("bogus"));
}

#[test]
fn config_rejects_phase_durations_not_multiple_of_tick() {
    let bad = CONFIG.replace("yellow_ms = 3008", "yellow_ms = 3009");
    assert_ne!(bad, CONFIG, "config must contain the replaced field");
    let error = JunctionConfig::parse(&bad).expect_err("non-multiple phase must fail");
    assert!(error.to_string().contains("whole multiple"));
}

#[test]
fn config_rejects_out_of_range_tick() {
    let bad = CONFIG.replace("fixed_delta_ms = 16", "fixed_delta_ms = 2");
    assert_ne!(bad, CONFIG, "config must contain the replaced field");
    let error = JunctionConfig::parse(&bad).expect_err("out-of-range tick must fail");
    assert!(error.to_string().contains("4..=1000"));
}

#[test]
fn config_rejects_pocket_offset_entering_opposing_lanes() {
    let bad = CONFIG.replace("pocket_offset_meters = 4.0", "pocket_offset_meters = 12.0");
    assert_ne!(bad, CONFIG, "config must contain the replaced field");
    let error = JunctionConfig::parse(&bad).expect_err("pocket in opposing lanes must fail");
    assert!(error.to_string().contains("opposing through lanes"));
}

#[test]
fn config_rejects_vehicle_longer_than_waiting_pocket() {
    let bad = CONFIG
        .replace("length_meters = 4.5", "length_meters = 13.0")
        .replace(
            "spawn_slot_pitch_meters = 10.0",
            "spawn_slot_pitch_meters = 30.0",
        );
    assert_ne!(bad, CONFIG, "config must contain the replaced fields");
    let error = JunctionConfig::parse(&bad).expect_err("vehicle longer than pocket must fail");
    assert!(error.to_string().contains("pocket_length_meters"));
}

#[test]
fn config_rejects_slot_pitch_below_two_vehicle_lengths() {
    let bad = CONFIG
        .replace("length_meters = 4.5", "length_meters = 6.0")
        .replace("min_gap_meters = 2.0", "min_gap_meters = 1.0")
        .replace(
            "spawn_slot_pitch_meters = 10.0",
            "spawn_slot_pitch_meters = 7.0",
        );
    assert_ne!(bad, CONFIG, "config must contain the replaced fields");
    let error = JunctionConfig::parse(&bad).expect_err("unsafe stagger must fail");
    assert!(error.to_string().contains("twice"));
}

#[test]
fn catalog_rejects_exit_portal_not_owning_final_edge() {
    use laneflow_scenario::complex_junction::CatalogError;

    let mut catalog = default_catalog();
    let route = catalog
        .routes
        .iter_mut()
        .find(|route| route.route_id == "route-w-through")
        .expect("route exists");
    route.exit_portal_id = "portal-loop-to-n".to_owned();
    let error = laneflow_scenario::complex_junction::validate(&catalog)
        .expect_err("tampered exit portal must fail");
    assert!(
        matches!(error, CatalogError::ExitPortalMismatch { .. }),
        "expected ExitPortalMismatch, got {error}"
    );
}

#[test]
fn identity_ascii_keys_stay_below_compile_string_limit() {
    use laneflow_compiler::{CanonicalIdentityFieldView, CompileLimits};
    use laneflow_static_contract::FieldTag;

    let generated = default_generated();
    let limit = CompileLimits::p100_initial_v1().max_single_string_bytes();
    let mut longest = 0_u64;
    for edge in generated.lir().lane_edges() {
        let field = edge
            .identity_fields()
            .find(|field: &CanonicalIdentityFieldView<'_>| field.tag() == FieldTag::LaneEdgeKey)
            .expect("identity field");
        let key = std::str::from_utf8(field.value_bytes()).expect("ascii identity field");
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
fn lfca_loads_with_waiting_zone_conflict_zones_and_streams() {
    use laneflow_format::{FormatLimits, check_canonical_network_input};
    use laneflow_static_contract::EntityKind;
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
        build_shared_network_revision,
    };

    let generated = default_generated();
    let input = check_canonical_network_input(generated.lfca_bytes(), FormatLimits::HARD)
        .expect("checked LFCA");
    let revision = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("shared network revision");
    let counts = revision.traffic().entity_counts();
    assert_eq!(counts.count(EntityKind::LaneEdge), 31);
    assert_eq!(counts.count(EntityKind::Junction), 1);
    assert_eq!(counts.count(EntityKind::ManeuverGate), 11);
    assert_eq!(counts.count(EntityKind::WaitingZone), 1);
    assert_eq!(counts.count(EntityKind::ConflictZone), 3);
    assert_eq!(counts.count(EntityKind::ParticipantStream), 4);
}

#[test]
fn catalog_bind_spawns_few_vehicles_and_steps() {
    use std::sync::Arc;

    use laneflow_format::{FormatLimits, check_canonical_network_input};
    use laneflow_runtime::{TickInput, VehicleSpawnInput, WorldConfig};
    use laneflow_scenario::complex_junction::bind;
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
        build_shared_network_revision,
    };

    let generated = default_generated();
    let catalog = default_catalog();
    let input = check_canonical_network_input(generated.lfca_bytes(), FormatLimits::HARD)
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
    assert_eq!(bound.spawn_slots.len(), generated.counts().spawn_slots);
    assert_eq!(bound.routes.len(), 11);
    assert_eq!(bound.focus_route_indices.len(), 2);
    let profile = *bound
        .profiles
        .get(VEHICLE_PROFILE_KEY)
        .expect("standard-car profile");

    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(8, 32, 1_024, 1_024, 1, 16),
    )
    .expect("install");
    assert_eq!(world.revision().network_revision(), bound.network_revision);
    let routes = bound
        .install_routes(&mut world)
        .expect("install catalog routes");

    for slot in bound.spawn_slots.iter().take(3) {
        let route = *routes
            .get(slot.route_index)
            .expect("catalog route must be registered");
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                profile,
                route,
                0,
                slot.progress_mm,
                0,
            ))
            .expect("catalog slot must spawn");
    }
    world.step(TickInput::new(16)).expect("step");
    assert!(!world.committed_pose_sources().as_slice().is_empty());
}

#[test]
fn bind_rejects_slots_colliding_at_millimetre_resolution() {
    use laneflow_format::{FormatLimits, check_canonical_network_input};
    use laneflow_scenario::complex_junction::{BindError, SpawnSlotCatalogEntry, bind};
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
        build_shared_network_revision,
    };

    let generated = default_generated();
    let mut catalog = default_catalog();
    let reference = catalog.spawn_slots[0].clone();
    // 与 reference 同边、米值差小于半毫米：catalog 的 f64 bit 去重放行，
    // bind 的 (edge, progress_mm) 去重必须拒绝。
    catalog.spawn_slots.push(SpawnSlotCatalogEntry {
        slot_id: "slot-millimetre-collision".to_owned(),
        portal_id: reference.portal_id.clone(),
        lane_index: reference.lane_index,
        edge_id: reference.edge_id.clone(),
        progress: reference.progress + 0.000_4,
    });
    laneflow_scenario::complex_junction::validate(&catalog).expect("edited catalog still valid");
    let input = check_canonical_network_input(generated.lfca_bytes(), FormatLimits::HARD)
        .expect("checked LFCA");
    let revision = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("shared network revision");
    let error = bind(&catalog, &revision).expect_err("millimetre collision must be rejected");
    assert!(
        matches!(error, BindError::DuplicateSlotPosition { .. }),
        "expected DuplicateSlotPosition, got {error}"
    );
}

#[test]
fn bind_rejects_focus_route_without_repeated_gate() {
    use laneflow_format::{FormatLimits, check_canonical_network_input};
    use laneflow_scenario::complex_junction::{BindError, bind};
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
        build_shared_network_revision,
    };

    let generated = default_generated();
    let mut catalog = default_catalog();
    // 替换成合法但不重复过门的边序：validate 通过，bind 必须拒绝。
    let route = catalog
        .routes
        .iter_mut()
        .find(|route| route.route_id == "route-w-through-circuit")
        .expect("focus route exists");
    route.edge_ids = ["loop-sw-i0", "w-in-i0", "w-e.i0", "e-out-i0", "loop-es-i0"]
        .iter()
        .map(|edge| (*edge).to_owned())
        .collect();
    laneflow_scenario::complex_junction::validate(&catalog).expect("edited catalog still valid");
    let input = check_canonical_network_input(generated.lfca_bytes(), FormatLimits::HARD)
        .expect("checked LFCA");
    let revision = build_shared_network_revision(
        input,
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .expect("shared network revision");
    let error = bind(&catalog, &revision).expect_err("non-repeating focus route must be rejected");
    assert!(
        matches!(error, BindError::FocusRouteNotRepeating { .. }),
        "expected FocusRouteNotRepeating, got {error}"
    );
}

#[test]
fn focus_routes_repeat_gate_occurrences() {
    use laneflow_format::{FormatLimits, check_canonical_network_input};
    use laneflow_scenario::complex_junction::bind;
    use laneflow_static_contract::{EntityKind, ManeuverPathOrdinal};
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
        build_shared_network_revision,
    };

    // catalog 层：焦点路线两次穿过同一机动路径的边序列。
    let catalog = default_catalog();
    let through = catalog
        .routes
        .iter()
        .find(|route| route.route_id == "route-w-through-circuit")
        .expect("focus route");
    let through_path = ["w-in-i0", "w-e.i0", "e-out-i0"];
    assert_eq!(
        through
            .edge_ids
            .windows(through_path.len())
            .filter(|window| *window == through_path)
            .count(),
        2,
        "route-w-through-circuit must traverse the w-e lane-0 path twice"
    );

    // 修订层：焦点路线的边序号序列里至少一条机动路径出现两次以上，
    // 该路径的机动门随之获得重复 occurrence。
    let generated = default_generated();
    let input = check_canonical_network_input(generated.lfca_bytes(), FormatLimits::HARD)
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
    let path_count = revision
        .traffic()
        .entity_counts()
        .count(EntityKind::ManeuverPath);
    for focus_index in &bound.focus_route_indices {
        let route_edges = &bound.route_exits[*focus_index].edges;
        let mut repeated_paths = 0_u32;
        for raw in 0..path_count {
            let path = revision
                .traffic()
                .maneuvers()
                .maneuver_path(ManeuverPathOrdinal::from_raw(raw))
                .expect("maneuver path ordinal in range");
            let path_edges = path.edges();
            let occurrences = route_edges
                .windows(path_edges.len())
                .filter(|window| *window == path_edges)
                .count();
            if occurrences >= 2 {
                assert!(
                    !path.maneuver_gates().is_empty(),
                    "repeated path must carry at least one gate"
                );
                repeated_paths += 1;
            }
        }
        assert!(
            repeated_paths >= 1,
            "focus route index {focus_index} must repeat at least one maneuver path"
        );
    }
}
