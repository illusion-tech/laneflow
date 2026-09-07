use laneflow_urban_generator::{
    Catalog, Direction, Layout, Scale, UrbanConfig, compare_artifacts, generate,
};

#[test]
fn connected_fixture_delivers_repeatable_installable_artifacts_and_caller_routes() {
    let config =
        UrbanConfig::parse(include_str!("../../../examples/config/cn-urban.toml")).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let a = temp.path().join("a");
    let b = temp.path().join("b");
    let first = generate(&config, Scale::Fixture, &a, None).unwrap();
    let frozen: toml::Table =
        toml::from_str(include_str!("../fixtures/v1/fixture/manifest.toml")).unwrap();
    assert_eq!(
        frozen["network_revision"].as_str(),
        Some(first.network_revision.as_str())
    );
    for (name, file) in &first.files {
        assert_eq!(
            frozen["files"][name]["sha256"].as_str(),
            Some(file.sha256.as_str()),
            "{name}"
        );
        assert_eq!(
            frozen["files"][name]["bytes"].as_integer(),
            Some(file.bytes as i64),
            "{name}"
        );
    }
    let second = generate(&config, Scale::Fixture, &b, None).unwrap();
    assert_eq!(first.network_revision, second.network_revision);
    compare_artifacts(&a, &b).unwrap();
    assert_eq!(first.lir_counts["canonical_frames"], 1);
    assert_eq!(first.lir_counts["participant_classes"], 1);
    assert_eq!(first.lir_counts["vehicle_profiles"], 3);
    assert_eq!(first.lir_counts["parking_spaces"], 40);
    assert!(first.parking_capacity_by_tile.values().all(|&n| n >= 750));
    let catalog: Catalog =
        toml::from_str(&std::fs::read_to_string(a.join("routes.toml")).unwrap()).unwrap();
    assert!(catalog.routes.iter().any(|r| r.category == "cross-tile"));
    for mode in ["ProtectedGroup", "PermissiveGroup", "Uncontrolled"] {
        assert!(catalog.movements.iter().any(|m| m.control == mode));
    }
    assert!(catalog.movements.iter().any(|m| m.waiting));
    assert!(
        catalog
            .movements
            .iter()
            .any(|m| m.entry == Direction::West && m.exit == Direction::North && m.turn == "Left")
    );
    for program in &catalog.signals {
        assert!(program.offset_ms < program.cycle_ms);
        assert!(
            program
                .phases
                .iter()
                .all(|p| p.duration_ms > 0 && p.duration_ms % 528 == 0)
        );
    }
    for target in &catalog.parking {
        for anchor in target.entries.iter().chain(&target.exits) {
            let route = catalog
                .routes
                .iter()
                .find(|r| r.key == anchor.route)
                .unwrap();
            assert_eq!(
                route.edge_keys[anchor.route_edge_index as usize],
                anchor.edge
            );
            assert_eq!(
                anchor.virtual_anchor_index.is_some(),
                target.kind == "virtual"
            );
        }
    }
}

#[test]
fn both_scales_use_the_accepted_integer_layout_and_reciprocal_ports() {
    for (scale, cells, columns) in [
        (Scale::TenThousand, 100, 5),
        (Scale::HundredThousand, 1_000, 15),
    ] {
        let layout = Layout::new(scale);
        assert_eq!(layout.cells.len(), cells);
        assert_eq!(layout.tile_columns, columns);
        for cell in &layout.cells {
            for arm in Direction::ALL {
                if let Some(other) = layout.neighbour(cell, arm) {
                    assert_eq!(
                        layout.neighbour(other, arm.opposite()).unwrap().index,
                        cell.index
                    );
                }
            }
        }
    }
}
