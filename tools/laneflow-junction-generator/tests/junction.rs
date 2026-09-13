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
    assert_eq!(counts.edges, 39);
    assert_eq!(counts.movements, 11);
    assert_eq!(counts.maneuver_paths, 13);
    assert_eq!(counts.maneuver_gates, 15);
    assert_eq!(counts.stop_lines, 12);
    assert_eq!(counts.waiting_zones, 1);
    assert_eq!(counts.conflict_zones, 5);
    assert_eq!(counts.streams, 8);
    assert_eq!(counts.signal_groups, 5);
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
    assert_eq!(lir.lane_edges().len(), 39);
    assert_eq!(lir.junctions().len(), 3);
    assert_eq!(lir.movements().len(), 11);
    assert_eq!(lir.maneuver_paths().len(), 13);
    assert_eq!(lir.stop_lines().len(), 12);
    assert_eq!(lir.maneuver_gates().len(), 15);
    assert_eq!(lir.signal_groups().len(), 5);
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
fn waiting_capacity_follows_storage_length_vehicle_length_and_gap() {
    for (storage, length, gap, capacity) in [
        (12.0, 4.5, 2.0, 2),
        (10.0, 4.5, 2.0, 1),
        // 同为 10 m 储车段，较短车型恢复两车容量，并保留合法车辆包络。
        (10.0, 4.0, 2.0, 2),
        (12.0, 4.5, 4.0, 1),
        (11.0, 4.5, 2.0, 2),
        (10.999, 4.5, 2.0, 1),
    ] {
        let mut config = JunctionConfig::parse(CONFIG).unwrap();
        config.geometry.pocket_length_meters = storage;
        config.profile.length_meters = length;
        config.profile.min_gap_meters = gap;
        let generated = generate(&config).unwrap_or_else(|error| {
            panic!("waiting configuration {storage}/{length}/{gap}: {error}")
        });
        let lir = generated.lir();
        let zone = lir.waiting_zones().next().unwrap();
        assert_eq!(zone.max_occupancy(), capacity, "{storage}/{length}/{gap}");
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
    // 同 portal 两条环路在分叉/汇合段几何收敛：lane 1 槽位按半 pitch 相位
    // 起错（6.5 + 5 = 11.5 起），靠近焊接点的不安全候选被贪心过滤丢弃，
    // 因此只锁定相位网格（≡ 11.5 mod pitch），不锁定具体首个存活槽位。
    for portal in &catalog.portals {
        for lane in &portal.lanes {
            let first = catalog
                .spawn_slots
                .iter()
                .filter(|slot| slot.portal_id == portal.id && slot.lane_index == lane.lane_index)
                .map(|slot| slot.progress)
                .fold(f64::INFINITY, f64::min);
            let base = 6.5 + lane.lane_index as f64 * 5.0;
            assert!(
                first >= base && (first - base) % 10.0 == 0.0,
                "portal {:?} lane {} first slot progress {first} is off the phase grid {base}",
                portal.id,
                lane.lane_index
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
fn config_rejects_shallow_pocket_offset() {
    let bad = CONFIG.replace("pocket_offset_meters = 4.0", "pocket_offset_meters = 0.5");
    assert_ne!(bad, CONFIG, "config must contain the replaced field");
    let error = JunctionConfig::parse(&bad).expect_err("shallow pocket must fail");
    assert!(error.to_string().contains("approach through lane"));
}

#[test]
fn config_rejects_lane_width_below_vehicle_envelope() {
    let bad = CONFIG.replace("lane_width_meters = 3.5", "lane_width_meters = 1.0");
    assert_ne!(bad, CONFIG, "config must contain the replaced field");
    let error = JunctionConfig::parse(&bad).expect_err("sub-envelope lane width must fail");
    assert!(error.to_string().contains("vehicle width envelope"));
}

#[test]
fn generate_rejects_non_finite_slot_candidates() {
    let bad = CONFIG
        .replace("arm_length_meters = 150.0", "arm_length_meters = 1e308")
        .replace(
            "spawn_slot_pitch_meters = 10.0",
            "spawn_slot_pitch_meters = 1e308",
        );
    assert_ne!(bad, CONFIG, "config must contain the replaced fields");
    let config = JunctionConfig::parse(&bad).expect("raw config remains valid");
    let Err(error) = generate(&config) else {
        panic!("non-finite slot candidates must fail");
    };
    assert!(matches!(
        error,
        laneflow_junction_generator::Error::Config(_)
    ));
}

#[test]
fn generate_rejects_waiting_storage_without_room_for_connection() {
    let bad = CONFIG
        .replace("pocket_length_meters = 12.0", "pocket_length_meters = 19.0")
        .replace("curve_control_meters = 12.0", "curve_control_meters = 10.0");
    assert_ne!(bad, CONFIG, "config must contain the replaced fields");
    let config = JunctionConfig::parse(&bad).expect("raw config remains valid");
    let Err(error) = generate(&config) else {
        panic!("storage must leave a connection before waiting entry");
    };
    assert!(error.to_string().contains("approach connection"));
}

#[test]
fn generate_rejects_overlapping_protected_envelopes() {
    // 中心线不相交，但转弯车尾扫入相邻直行车道的车辆包络仍必须拒绝。
    let bad = CONFIG.replace("lane_width_meters = 3.5", "lane_width_meters = 2.1");
    assert_ne!(bad, CONFIG, "config must contain the replaced fields");
    let config = JunctionConfig::parse(&bad).expect("raw config remains valid");
    let Err(error) = generate(&config) else {
        panic!("overlapping protected envelopes must fail");
    };
    assert!(error.to_string().contains("vehicle envelopes"));
}

#[test]
fn generate_rejects_output_aliasing_config() {
    for directory_override in [".", "missing/.."] {
        let directory = std::env::temp_dir().join(format!(
            "laneflow-junction-alias-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).expect("create temp dir");
        let config_path = directory.join("junction.toml");
        let aliased = CONFIG
            .replace(
                "directory = \"../data\"",
                &format!("directory = \"{directory_override}\""),
            )
            .replace(
                "catalog_file_name = \"v0.1-complex-junction.catalog.toml\"",
                "catalog_file_name = \"junction.toml\"",
            );
        assert_ne!(aliased, CONFIG, "config must contain the replaced fields");
        std::fs::write(&config_path, aliased).expect("write temp config");
        let error = laneflow_junction_generator::generate_files(&config_path)
            .expect_err("aliased output must fail");
        assert!(
            error.to_string().contains("overwrite the source config"),
            "directory override {directory_override:?}: {error}"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }
}

#[test]
fn generate_rejects_output_alias_through_symlink_parent() {
    let root = std::env::temp_dir().join(format!(
        "laneflow-junction-symlink-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let source = root.join("source");
    let child = source.join("child");
    std::fs::create_dir_all(&child).expect("create temp dirs");
    #[cfg(windows)]
    let link_result = std::os::windows::fs::symlink_dir(&child, root.join("link"));
    #[cfg(unix)]
    let link_result = std::os::unix::fs::symlink(&child, root.join("link"));
    if link_result.is_err() {
        // 无符号链接权限的环境（部分 Windows 开发机）跳过本用例。
        let _ = std::fs::remove_dir_all(&root);
        return;
    }
    // config 在 source/ 下；输出目录 "../link/.." 经符号链接解析回 source/。
    let config_path = source.join("junction.toml");
    let aliased = CONFIG
        .replace("directory = \"../data\"", "directory = \"../link/..\"")
        .replace(
            "catalog_file_name = \"v0.1-complex-junction.catalog.toml\"",
            "catalog_file_name = \"junction.toml\"",
        );
    assert_ne!(aliased, CONFIG, "config must contain the replaced fields");
    std::fs::write(&config_path, aliased).expect("write temp config");
    let error = laneflow_junction_generator::generate_files(&config_path)
        .expect_err("symlink-parent alias must fail");
    assert!(error.to_string().contains("overwrite the source config"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn generate_rejects_output_files_aliased_through_symlink() {
    let root = std::env::temp_dir().join(format!(
        "laneflow-junction-out-alias-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let out = root.join("out");
    std::fs::create_dir_all(&out).expect("create temp dirs");
    // b.out 是指向尚不存在的 a.out 的悬空符号链接：两个输出落到同一文件。
    #[cfg(windows)]
    let link_result = std::os::windows::fs::symlink_file("a.out", out.join("b.out"));
    #[cfg(unix)]
    let link_result = std::os::unix::fs::symlink("a.out", out.join("b.out"));
    if link_result.is_err() {
        // 无符号链接权限的环境跳过本用例。
        let _ = std::fs::remove_dir_all(&root);
        return;
    }
    let config_path = root.join("junction.toml");
    let aliased = CONFIG
        .replace("directory = \"../data\"", "directory = \"out\"")
        .replace(
            "catalog_file_name = \"v0.1-complex-junction.catalog.toml\"",
            "catalog_file_name = \"a.out\"",
        )
        .replace(
            "lfca_file_name = \"v0.1-complex-junction.lfca\"",
            "lfca_file_name = \"b.out\"",
        );
    assert_ne!(aliased, CONFIG, "config must contain the replaced fields");
    std::fs::write(&config_path, aliased).expect("write temp config");
    let error = laneflow_junction_generator::generate_files(&config_path)
        .expect_err("aliased outputs must fail");
    assert!(error.to_string().contains("resolve to the same file"));
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn generate_rejects_output_alias_differing_only_by_case() {
    let root = std::env::temp_dir().join(format!(
        "laneflow-junction-case-alias-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("create temp dir");
    let config_path = root.join("junction.toml");
    let aliased = CONFIG
        .replace("directory = \"../data\"", "directory = \".\"")
        .replace(
            "catalog_file_name = \"v0.1-complex-junction.catalog.toml\"",
            "catalog_file_name = \"JUNCTION.TOML\"",
        );
    assert_ne!(aliased, CONFIG, "config must contain the replaced fields");
    std::fs::write(&config_path, aliased).expect("write temp config");
    let error = laneflow_junction_generator::generate_files(&config_path)
        .expect_err("case-only alias must fail");
    assert!(error.to_string().contains("overwrite the source config"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn generate_rejects_excessive_slot_count() {
    let bad = CONFIG
        .replace("length_meters = 4.5", "length_meters = 0.1")
        .replace("min_gap_meters = 2.0", "min_gap_meters = 0.1")
        .replace(
            "spawn_slot_pitch_meters = 10.0",
            "spawn_slot_pitch_meters = 0.2",
        );
    assert_ne!(bad, CONFIG, "config must contain the replaced fields");
    let config = JunctionConfig::parse(&bad).expect("raw config remains valid");
    let Err(error) = generate(&config) else {
        panic!("excessive slot count must fail");
    };
    assert!(error.to_string().contains("spawn"));
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
    assert_eq!(counts.count(EntityKind::LaneEdge), 39);
    assert_eq!(counts.count(EntityKind::Junction), 3);
    assert_eq!(counts.count(EntityKind::ManeuverGate), 15);
    assert_eq!(counts.count(EntityKind::WaitingZone), 1);
    assert_eq!(counts.count(EntityKind::ConflictZone), 5);
    assert_eq!(counts.count(EntityKind::ParticipantStream), 8);
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
fn waiting_pocket_holds_two_cars_and_admits_the_third_after_release() {
    use laneflow_format::{FormatLimits, check_canonical_network_input};
    use laneflow_runtime::{TickInput, VehicleSpawnInput, WaitingDecisionOutcome, WorldConfig};
    use laneflow_scenario::complex_junction::bind;
    use laneflow_static_contract::{SignalAspect, WaitingZoneOrdinal};
    use laneflow_static_network::{
        SharedNetworkBuildLimits, SharedNetworkBuildOptions, SpatialBuildOption,
        build_shared_network_revision,
    };
    use std::sync::Arc;

    let generated = default_generated();
    let catalog: JunctionCatalog =
        toml::from_str(std::str::from_utf8(generated.catalog_bytes()).unwrap()).unwrap();
    let revision = build_shared_network_revision(
        check_canonical_network_input(generated.lfca_bytes(), FormatLimits::HARD).unwrap(),
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::RetainAvailable,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .unwrap();
    let bound = bind(&catalog, &revision).unwrap();
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::new(8, 32, 1_024, 1_024, 1, 16),
    )
    .unwrap();
    let routes = bound.install_routes(&mut world).unwrap();
    let route_index = catalog
        .routes
        .iter()
        .position(|r| r.route_id == "route-w-left-waiting")
        .unwrap();
    let profile = bound.profiles[VEHICLE_PROFILE_KEY];
    let lead_in_edge = world.route_edges(routes[route_index]).unwrap()[2];
    let lead_in_length = revision.traffic().lane_lengths_millimetres()[lead_in_edge.raw() as usize];
    // 三辆车从同一进口上游依次驶入，不直接 spawn 在待转区内绕过准入。
    let vehicles: Vec<_> = [115_000, 105_000, 95_000]
        .into_iter()
        .map(|progress| {
            world
                .spawn_vehicle(VehicleSpawnInput::new(
                    profile,
                    routes[route_index],
                    1,
                    progress,
                    0,
                ))
                .unwrap()
        })
        .collect();
    let zone_id = WaitingZoneOrdinal::from_raw(0);
    let relations = revision.traffic().relations();
    let zone = relations.waiting_zone(zone_id).unwrap();
    let release_group = relations
        .maneuver_gate(zone.release_gate())
        .unwrap()
        .signal_group()
        .unwrap();
    assert_eq!(zone.max_occupancy(), 2);
    let mut held_two = false;
    let mut stopped_full_ticks = 0;
    let mut admitted_third = false;
    for _ in 0..2_750 {
        world.step(TickInput::new(16)).unwrap();
        assert!(world.waiting_zone(zone_id).unwrap().occupancy() <= 2);
        let signals = world.committed_signal_groups();
        let release = signals
            .as_slice()
            .iter()
            .find(|(id, _)| *id == release_group)
            .unwrap()
            .1;
        let front = world.vehicle(vehicles[0]).unwrap();
        let second = world.vehicle(vehicles[1]).unwrap();
        let third = world.vehicle(vehicles[2]).unwrap();
        if release == SignalAspect::Red
            && front.route_edge_index() == 3
            && second.route_edge_index() == 3
        {
            // 两辆车最终整车停进 12 m 储车段，至少留 2 m 车间距。
            if front.speed_mm_s() == 0 && second.speed_mm_s() == 0 {
                assert!(front.progress_mm() <= 12_000);
                assert!(second.progress_mm() >= second.length_mm());
                assert!(front.progress_mm() >= second.progress_mm() + front.length_mm() + 2_000);
                assert_eq!(world.waiting_zone(zone_id).unwrap().occupancy(), 2);
                assert_eq!(front.waiting_membership().unwrap().waiting_zone(), zone_id);
                assert_eq!(second.waiting_membership().unwrap().waiting_zone(), zone_id);
                assert!(third.waiting_membership().is_none());
                assert!(third.route_edge_index() < 3);
                held_two = true;
                if third.speed_mm_s() == 0 {
                    // 跟驰已在 entry 前挡住第三辆，无需产生 Capacity 决策。
                    // 按连接段剩余距离加第二辆车尾位置，检查跨边净距。
                    assert_eq!(third.route_edge_index(), 2);
                    assert!(third.progress_mm() < lead_in_length);
                    let gap = lead_in_length - third.progress_mm() + second.progress_mm()
                        - second.length_mm();
                    assert!(gap >= 2_000);
                    stopped_full_ticks += 1;
                }
            }
        }
        for decision in world.latest_waiting_decisions() {
            if decision.vehicle() == vehicles[2]
                && decision.outcome() == WaitingDecisionOutcome::Granted
            {
                assert!(stopped_full_ticks >= 100);
                assert_eq!(release, SignalAspect::Green);
                assert!(front.waiting_membership().is_none());
                assert_eq!(third.waiting_membership().unwrap().waiting_zone(), zone_id);
                admitted_third = true;
            }
        }
    }
    assert!(
        held_two,
        "two full cars must fit while the left-turn light is red"
    );
    assert!(
        stopped_full_ticks >= 100,
        "the third car must remain stopped outside a full pocket"
    );
    assert!(
        admitted_third,
        "the third car must enter when release frees space"
    );
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

/// 同角两条环路中心线采样与端点检查的共享准备。
struct LoopGeometry {
    /// (loop key) -> (first, last, points)。
    loops: std::collections::BTreeMap<
        String,
        (
            laneflow_static_network::CanonicalPoint,
            laneflow_static_network::CanonicalPoint,
            Vec<laneflow_static_network::CanonicalPoint>,
        ),
    >,
}

fn loop_geometry() -> LoopGeometry {
    use laneflow_format::{FormatLimits, check_canonical_network_input};
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
    let lane_pose = revision
        .spatial()
        .and_then(|spatial| spatial.lane_pose())
        .expect("lane pose network");
    let mut loops = std::collections::BTreeMap::new();
    let route_edges: std::collections::BTreeMap<_, _> = catalog
        .routes
        .iter()
        .flat_map(|route| {
            route
                .edge_ids
                .iter()
                .cloned()
                .zip(bound.routes[&route.route_id].iter().copied())
        })
        .collect();
    for (name, ordinal) in bound.edges.iter() {
        if !name.starts_with("loop-") || name.ends_with(".merge") || name.ends_with(".admission") {
            continue;
        }
        let geometry = lane_pose
            .lane_geometry(*ordinal)
            .unwrap_or_else(|| panic!("loop edge {name} must carry geometry"));
        let mut points: Vec<_> = geometry.points().to_vec();
        for suffix in [".admission", ".merge"] {
            let Some(taper) = route_edges.get(&format!("{name}{suffix}")) else {
                continue;
            };
            let taper = lane_pose
                .lane_geometry(*taper)
                .expect("merge taper geometry");
            assert_eq!(
                points.last(),
                taper.points().first(),
                "split preserves exact weld"
            );
            points.extend_from_slice(&taper.points()[1..]);
        }
        loops.insert(
            name.clone(),
            (
                *points.first().expect("loop has points"),
                *points.last().expect("loop has points"),
                points,
            ),
        );
    }
    assert_eq!(loops.len(), 8, "四个 portal 各两条环路");
    LoopGeometry { loops }
}

/// 折线弧长（米，f64 重算；采样点 y 恒为 0）。
fn arc_lengths(points: &[laneflow_static_network::CanonicalPoint]) -> Vec<f64> {
    let mut lengths = Vec::with_capacity(points.len());
    lengths.push(0.0_f64);
    for pair in points.windows(2) {
        let dx = f64::from(pair[1].x) - f64::from(pair[0].x);
        let dz = f64::from(pair[1].z) - f64::from(pair[0].z);
        lengths.push(lengths.last().expect("length seed") + (dx * dx + dz * dz).sqrt());
    }
    lengths
}

fn point_distance(
    a: laneflow_static_network::CanonicalPoint,
    b: laneflow_static_network::CanonicalPoint,
) -> f64 {
    let dx = f64::from(a.x) - f64::from(b.x);
    let dz = f64::from(a.z) - f64::from(b.z);
    (dx * dx + dz * dz).sqrt()
}

/// 线段对真交（不含端点相触）；共点端的焊接接触不算交叉。
fn segments_cross(
    a0: laneflow_static_network::CanonicalPoint,
    a1: laneflow_static_network::CanonicalPoint,
    b0: laneflow_static_network::CanonicalPoint,
    b1: laneflow_static_network::CanonicalPoint,
) -> bool {
    fn cross(o: (f64, f64), p: (f64, f64), q: (f64, f64)) -> f64 {
        (p.0 - o.0) * (q.1 - o.1) - (p.1 - o.1) * (q.0 - o.0)
    }
    let a = (
        (f64::from(a0.x), f64::from(a0.z)),
        (f64::from(a1.x), f64::from(a1.z)),
    );
    let b = (
        (f64::from(b0.x), f64::from(b0.z)),
        (f64::from(b1.x), f64::from(b1.z)),
    );
    let d1 = cross(b.0, b.1, a.0);
    let d2 = cross(b.0, b.1, a.1);
    let d3 = cross(a.0, a.1, b.0);
    let d4 = cross(a.0, a.1, b.1);
    ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0))
}

#[test]
fn loop_pairs_stay_separated_outside_taper_and_never_cross() {
    // 回归 #285 环路几何缺陷：同角两条环路必须全段横向分离（平行偏移一个
    // 车道宽），且任何位置不相交。单车道臂强制 2↔1 汇合/分流的共点端
    // 45 m 直线锥形区内允许单调收敛，区外
    // 最小间距必须 ≥ 3.4 m（平行偏移实测恰为车道宽 3.5 m，留采样余量）。
    const TAPER_METERS: f64 = 45.0;
    const SEPARATION_FLOOR: f64 = 3.4;

    let prepared = loop_geometry();
    for portal in ["es", "wn", "ne", "sw"] {
        let inner = prepared
            .loops
            .get(&format!("loop-{portal}-i0"))
            .unwrap_or_else(|| panic!("loop-{portal}-i0"));
        let wide = prepared
            .loops
            .get(&format!("loop-{portal}-i1"))
            .unwrap_or_else(|| panic!("loop-{portal}-i1"));
        let shared_start = point_distance(inner.0, wide.0) == 0.0;
        let shared_end = point_distance(inner.1, wide.1) == 0.0;
        assert!(
            shared_start != shared_end,
            "loop-{portal} 恰有一端共点（2↔1 汇合/分流）"
        );
        let inner_lengths = arc_lengths(&inner.2);
        let wide_lengths = arc_lengths(&wide.2);
        let mut minimum = f64::MAX;
        for (index, point) in inner.2.iter().enumerate() {
            let head = inner_lengths[index];
            let tail = inner_lengths[inner.2.len() - 1] - head;
            if (shared_start && head < TAPER_METERS) || (shared_end && tail < TAPER_METERS) {
                continue;
            }
            for (other_index, other) in wide.2.iter().enumerate() {
                let other_head = wide_lengths[other_index];
                let other_tail = wide_lengths[wide.2.len() - 1] - other_head;
                if (shared_start && other_head < TAPER_METERS)
                    || (shared_end && other_tail < TAPER_METERS)
                {
                    continue;
                }
                minimum = minimum.min(point_distance(*point, *other));
            }
        }
        assert!(
            minimum >= SEPARATION_FLOOR,
            "loop-{portal} 锥形区外最小间距 {minimum:.3} m 必须 ≥ {SEPARATION_FLOOR} m"
        );
        for a in inner.2.windows(2) {
            for b in wide.2.windows(2) {
                assert!(
                    !segments_cross(a[0], a[1], b[0], b[1]),
                    "loop-{portal} 两环中心线不得交叉"
                );
            }
        }
    }
}

#[test]
fn smaller_loop_radius_keeps_straight_tapers_compilable() {
    let mut config = JunctionConfig::parse(CONFIG).unwrap();
    config.geometry.loop_corner_radius_meters = 15.0;
    let generated = generate(&config).expect("15 m radius must preserve taper tangents");
    assert_eq!(generated.counts().edges, 39);
}

#[test]
fn compact_loop_configuration_returns_error_without_panicking() {
    let mut config = JunctionConfig::parse(CONFIG).unwrap();
    config.geometry.arm_length_meters = 10.0;
    config.geometry.junction_radius_meters = 3.0;
    config.geometry.lane_width_meters = 2.0;
    config.geometry.center_offset_meters = 4.0;
    config.geometry.pocket_length_meters = 1.0;
    config.geometry.pocket_offset_meters = 2.0;
    config.geometry.curve_control_meters = 1.0;
    config.geometry.loop_corner_radius_meters = 1.0;
    assert!(generate(&config).is_err());
}

#[test]
fn loop_lane_tapers_finish_before_the_first_bend() {
    let prepared = loop_geometry();
    for portal in ["ne", "es", "sw", "wn"] {
        let base = &prepared.loops[&format!("loop-{portal}-i0")];
        let other = &prepared.loops[&format!("loop-{portal}-i1")];
        let shared_start = base.0 == other.0;
        let mut points = base.2.clone();
        let mut second = other.2.clone();
        if !shared_start {
            points.reverse();
            second.reverse();
        }
        let origin = points[0];
        let dx = f64::from(points[1].x - origin.x);
        let dz = f64::from(points[1].z - origin.z);
        let length = (dx * dx + dz * dz).sqrt();
        let tangent = [dx / length, dz / length];
        let project = |point: laneflow_static_network::CanonicalPoint| {
            let x = f64::from(point.x - origin.x);
            let z = f64::from(point.z - origin.z);
            (
                x * tangent[0] + z * tangent[1],
                (x * tangent[1] - z * tangent[0]).abs(),
            )
        };
        let first_bend = points
            .iter()
            .copied()
            .find(|point| project(*point).1 > 0.01)
            .expect("loop turns");
        assert!(
            project(first_bend).0 >= 45.0,
            "{portal}: taper must occupy a straight"
        );
        let taper: Vec<_> = second
            .iter()
            .copied()
            .take_while(|point| project(*point).0 <= 45.01)
            .collect();
        assert!(
            taper.len() >= 20,
            "{portal}: smooth taper must survive emission"
        );
        let mut previous = 0.0;
        for point in &taper {
            let (_, lateral) = project(*point);
            assert!(
                lateral + 0.001 >= previous && lateral <= 3.501,
                "{portal}: monotone lane transition"
            );
            previous = lateral;
        }
        assert!(
            previous > 3.49,
            "{portal}: full lane separation before turning"
        );
        for pair in taper.windows(2) {
            let a = project(pair[0]);
            let b = project(pair[1]);
            assert!(
                (b.1 - a.1).abs() <= (b.0 - a.0).abs() * 0.13 + 0.001,
                "{portal}: taper steering angle must stay gentle"
            );
        }
    }
}

#[test]
fn loop_endpoints_weld_to_arm_lane_ports() {
    // 每条环路端点逐位等于其臂道车道端口（LaneEdge 焊接约束）：
    // 出发端 = 本环出口车道外端，到达端 = 到达入口车道外端；同角两条环路在
    // 分离侧端点不共点（间距恰为一个车道宽 3.5 m），共点侧端点重合。
    let prepared = loop_geometry();
    let config = JunctionConfig::parse(CONFIG).expect("default config must parse");
    let geometry = &config.geometry;
    let lane_width = geometry.lane_width_meters;

    // 与生成器 `topology::port` 同式的右侧通行端口（米制 f64）。
    fn arm_delta(arm: &str) -> [f64; 2] {
        match arm {
            "e" => [1.0, 0.0],
            "w" => [-1.0, 0.0],
            "n" => [0.0, -1.0],
            "s" => [0.0, 1.0],
            _ => panic!("unknown arm {arm}"),
        }
    }
    fn lane_offsets(config: &JunctionConfig, arm: &str) -> Vec<f64> {
        let center = config.geometry.center_offset_meters;
        let half = config.geometry.lane_width_meters / 2.0;
        if arm == "e" || arm == "w" {
            vec![center - half, center + half]
        } else {
            vec![center]
        }
    }
    fn port(config: &JunctionConfig, arm: &str, entering: bool, offset: f64) -> [f64; 2] {
        let [dx, dz] = arm_delta(arm);
        let sign = if entering { -1.0 } else { 1.0 };
        let radius = config.geometry.arm_length_meters;
        [
            dx * radius - dz * sign * offset,
            dz * radius + dx * sign * offset,
        ]
    }
    fn canonical(x: f64, z: f64) -> laneflow_static_network::CanonicalPoint {
        laneflow_static_network::CanonicalPoint {
            x: x as f32,
            y: 0.0,
            z: z as f32,
        }
    }

    // (loop key, 出口臂, 出口车道, 入口臂, 入口车道)
    let welds: [(&str, &str, usize, &str, usize); 8] = [
        ("loop-es-i0", "e", 0, "s", 0),
        ("loop-es-i1", "e", 1, "s", 0),
        ("loop-wn-i0", "w", 0, "n", 0),
        ("loop-wn-i1", "w", 1, "n", 0),
        ("loop-sw-i0", "s", 0, "w", 0),
        ("loop-sw-i1", "s", 0, "w", 1),
        ("loop-ne-i0", "n", 0, "e", 0),
        ("loop-ne-i1", "n", 0, "e", 1),
    ];
    for (key, from, from_lane, to, to_lane) in welds {
        let (first, last, _) = prepared.loops.get(key).unwrap_or_else(|| panic!("{key}"));
        let from_offset = lane_offsets(&config, from)[from_lane];
        let to_offset = lane_offsets(&config, to)[to_lane];
        let expected_start = port(&config, from, false, from_offset);
        let expected_end = port(&config, to, true, to_offset);
        assert_eq!(
            *first,
            canonical(expected_start[0], expected_start[1]),
            "{key} 出发端必须逐位焊接 {from} 出口车道 {from_lane} 外端"
        );
        assert_eq!(
            *last,
            canonical(expected_end[0], expected_end[1]),
            "{key} 到达端必须逐位焊接 {to} 入口车道 {to_lane} 外端"
        );
    }
    for portal in ["es", "wn"] {
        let first0 = prepared.loops[&format!("loop-{portal}-i0")].0;
        let first1 = prepared.loops[&format!("loop-{portal}-i1")].0;
        assert_eq!(
            point_distance(first0, first1),
            lane_width,
            "loop-{portal} 分离侧（出发）端点间距必须恰为一个车道宽（2↔1 汇合的对端）"
        );
    }
    for portal in ["sw", "ne"] {
        let last0 = prepared.loops[&format!("loop-{portal}-i0")].1;
        let last1 = prepared.loops[&format!("loop-{portal}-i1")].1;
        assert_eq!(
            point_distance(last0, last1),
            lane_width,
            "loop-{portal} 分离侧（到达）端点间距必须恰为一个车道宽（2↔1 分流的对端）"
        );
    }
}
