use std::f64::consts::PI;

use laneflow_core::{
    CoreError, EdgeLength, InitialTrafficData, LaneEdge, LaneGraph, ParkingAnchorKind, ParkingArea,
    ParkingRegistry, ParkingSpace, ParkingSpaceGeometry, SignalRegistry, VehicleProfileRegistry,
};

const CURRENT_PARKING_ANCHOR_ENDPOINT_CLEARANCE_METERS: f64 = 1.0e-9;
const CURRENT_MIN_PARKING_LATERAL_OFFSET_ABS_EXCLUSIVE_METERS: f64 = 1.0e-9;
const CURRENT_MIN_PARKING_EXTENT_EXCLUSIVE_METERS: f64 = 1.0e-9;

fn graph(edge_ids: &[&str]) -> LaneGraph {
    LaneGraph::try_new(edge_ids.iter().map(|id| {
        LaneEdge::new(
            *id,
            EdgeLength::try_new(100.0).expect("valid edge length"),
            std::iter::empty::<&str>(),
        )
    }))
    .expect("valid graph")
}

fn geometry() -> ParkingSpaceGeometry {
    ParkingSpaceGeometry::new(-3.0, 0.0, 5.0, 2.4)
}

fn space(id: &str, area_id: Option<&str>, entry_edge: &str, exit_edge: &str) -> ParkingSpace {
    ParkingSpace::new(
        id,
        area_id.map(str::to_owned),
        entry_edge,
        12.5,
        exit_edge,
        20.0,
        geometry(),
    )
}

#[test]
fn mixed_parking_preserves_definition_and_member_order() {
    let graph = graph(&["parking-in", "parking-out"]);
    let registry = ParkingRegistry::try_new(
        &graph,
        [ParkingArea::new("lot-main"), ParkingArea::new("curb-strip")],
        [
            space("lot-02", Some("lot-main"), "parking-in", "parking-out"),
            space("standalone", None, "parking-in", "parking-in"),
            space("curb-01", Some("curb-strip"), "parking-out", "parking-out"),
            space("lot-01", Some("lot-main"), "parking-in", "parking-out"),
        ],
    )
    .expect("valid parking registry");

    assert_eq!(
        registry.areas().map(ParkingArea::id).collect::<Vec<_>>(),
        ["lot-main", "curb-strip"]
    );
    assert_eq!(
        registry.spaces().map(ParkingSpace::id).collect::<Vec<_>>(),
        ["lot-02", "standalone", "curb-01", "lot-01"]
    );

    let lot = registry.area_handle("lot-main").expect("lot handle");
    let lot_members = registry.area_spaces(lot).expect("known area");
    assert_eq!(
        lot_members
            .iter()
            .map(|handle| registry.space_external_id(*handle).expect("known space"))
            .collect::<Vec<_>>(),
        ["lot-02", "lot-01"]
    );

    let standalone = registry.space_handle("standalone").expect("space handle");
    assert_eq!(registry.space_area(standalone), Some(None));
    let entry = registry.space_entry(standalone).expect("entry");
    assert_eq!(graph.edge_external_id(entry.edge()), Some("parking-in"));
    assert_eq!(entry.progress(), 12.5);
    assert_eq!(registry.space_geometry(standalone), Some(geometry()));
}

#[test]
fn identity_and_membership_fail_before_anchor_validation() {
    let graph = graph(&["edge"]);
    let duplicate = ParkingRegistry::try_new(
        &graph,
        [],
        [
            space("duplicate", None, "missing", "edge"),
            space("duplicate", None, "edge", "edge"),
        ],
    )
    .expect_err("space identity must fail before anchors");
    std::assert_matches!(
        duplicate,
        CoreError::DuplicateParkingSpaceId { space_id } if space_id == "duplicate"
    );

    let unknown_area = ParkingRegistry::try_new(
        &graph,
        [],
        [space("space", Some("missing-area"), "missing", "edge")],
    )
    .expect_err("membership must fail before anchors");
    std::assert_matches!(
        unknown_area,
        CoreError::UnknownParkingSpaceArea { space_id, area_id }
            if space_id == "space" && area_id == "missing-area"
    );
}

#[test]
fn every_parking_external_id_field_uses_shared_token_validation() {
    let graph = graph(&["edge"]);
    let cases = [
        (
            vec![ParkingArea::new("bad area")],
            vec![space("space", Some("bad area"), "edge", "edge")],
            "parking.areas[].id",
        ),
        (
            Vec::new(),
            vec![space("bad space", None, "edge", "edge")],
            "parking.spaces[].id",
        ),
        (
            vec![ParkingArea::new("area")],
            vec![space("space", Some("bad area"), "edge", "edge")],
            "parking.spaces[].areaId",
        ),
        (
            Vec::new(),
            vec![space("space", None, "bad edge", "edge")],
            "parking.spaces[].entry.edgeId",
        ),
        (
            Vec::new(),
            vec![space("space", None, "edge", "bad edge")],
            "parking.spaces[].exit.edgeId",
        ),
    ];
    for (areas, spaces, expected_field) in cases {
        let error = ParkingRegistry::try_new(&graph, areas, spaces)
            .expect_err("invalid external ID must fail");
        std::assert_matches!(
            error,
            CoreError::InvalidExternalId { field, .. } if field == expected_field
        );
    }
}

#[test]
fn every_anchor_is_validated_before_any_geometry() {
    let graph = graph(&["edge"]);
    let invalid_geometry = ParkingSpace::new(
        "first",
        None,
        "edge",
        10.0,
        "edge",
        20.0,
        ParkingSpaceGeometry::new(0.0, 0.0, 5.0, 2.4),
    );
    let error = ParkingRegistry::try_new(
        &graph,
        [],
        [invalid_geometry, space("second", None, "missing", "edge")],
    )
    .expect_err("later anchor must fail before earlier geometry");
    std::assert_matches!(
        error,
        CoreError::UnknownParkingAnchorEdge {
            space_id,
            anchor: ParkingAnchorKind::Entry,
            edge_id,
        } if space_id == "second" && edge_id == "missing"
    );
}

#[test]
fn anchors_use_their_strict_endpoint_clearance() {
    let graph = graph(&["edge"]);
    let upper_exclusive = 100.0 - CURRENT_PARKING_ANCHOR_ENDPOINT_CLEARANCE_METERS;
    for progress in [
        f64::NAN,
        -0.0,
        0.0,
        CURRENT_PARKING_ANCHOR_ENDPOINT_CLEARANCE_METERS,
        upper_exclusive,
        100.0,
    ] {
        let invalid = ParkingSpace::new("space", None, "edge", progress, "edge", 50.0, geometry());
        let error = ParkingRegistry::try_new(&graph, [], [invalid])
            .expect_err("boundary progress must fail");
        std::assert_matches!(
            error,
            CoreError::ParkingAnchorProgressOutOfRange {
                anchor: ParkingAnchorKind::Entry,
                ..
            }
        );
    }

    for progress in [
        CURRENT_PARKING_ANCHOR_ENDPOINT_CLEARANCE_METERS.next_up(),
        upper_exclusive.next_down(),
    ] {
        let valid = ParkingSpace::new("space", None, "edge", progress, "edge", 50.0, geometry());
        ParkingRegistry::try_new(&graph, [], [valid])
            .expect("adjacent value inside the endpoint clearance must pass");
    }
}

#[test]
fn geometry_fields_use_fixed_canonical_order_and_ranges() {
    let graph = graph(&["edge"]);
    let cases = [
        (
            ParkingSpaceGeometry::new(0.0, PI, 0.0, 0.0),
            "lateralOffset",
        ),
        (
            ParkingSpaceGeometry::new(3.0, PI, 0.0, 0.0),
            "headingOffsetRadians",
        ),
        (ParkingSpaceGeometry::new(3.0, -PI, 0.0, 0.0), "length"),
        (ParkingSpaceGeometry::new(3.0, -PI, 5.0, 0.0), "width"),
        (
            ParkingSpaceGeometry::new(f64::NAN, 0.0, 5.0, 2.4),
            "lateralOffset",
        ),
        (
            ParkingSpaceGeometry::new(3.0, f64::INFINITY, 5.0, 2.4),
            "headingOffsetRadians",
        ),
        (ParkingSpaceGeometry::new(3.0, 0.0, f64::NAN, 2.4), "length"),
        (
            ParkingSpaceGeometry::new(3.0, 0.0, 5.0, f64::INFINITY),
            "width",
        ),
    ];
    for (geometry, expected_field) in cases {
        let invalid = ParkingSpace::new("space", None, "edge", 10.0, "edge", 20.0, geometry);
        let error = ParkingRegistry::try_new(&graph, [], [invalid])
            .expect_err("invalid geometry must fail");
        std::assert_matches!(
            error,
            CoreError::InvalidParkingGeometryValue { field, .. } if field == expected_field
        );
    }
}

#[test]
fn parking_geometry_minimums_are_domain_specific_and_strict() {
    let graph = graph(&["edge"]);
    for (geometry, expected_field) in [
        (
            ParkingSpaceGeometry::new(
                CURRENT_MIN_PARKING_LATERAL_OFFSET_ABS_EXCLUSIVE_METERS,
                0.0,
                5.0,
                2.4,
            ),
            "lateralOffset",
        ),
        (
            ParkingSpaceGeometry::new(
                -CURRENT_MIN_PARKING_LATERAL_OFFSET_ABS_EXCLUSIVE_METERS,
                0.0,
                5.0,
                2.4,
            ),
            "lateralOffset",
        ),
        (
            ParkingSpaceGeometry::new(-0.0, 0.0, 5.0, 2.4),
            "lateralOffset",
        ),
        (
            ParkingSpaceGeometry::new(-3.0, 0.0, CURRENT_MIN_PARKING_EXTENT_EXCLUSIVE_METERS, 2.4),
            "length",
        ),
        (
            ParkingSpaceGeometry::new(-3.0, 0.0, 5.0, CURRENT_MIN_PARKING_EXTENT_EXCLUSIVE_METERS),
            "width",
        ),
        (ParkingSpaceGeometry::new(-3.0, 0.0, -0.0, 2.4), "length"),
    ] {
        let invalid = ParkingSpace::new("space", None, "edge", 10.0, "edge", 20.0, geometry);
        let error = ParkingRegistry::try_new(&graph, [], [invalid])
            .expect_err("value at its exclusive minimum must fail");
        std::assert_matches!(
            error,
            CoreError::InvalidParkingGeometryValue { field, .. } if field == expected_field
        );
    }

    for geometry in [
        ParkingSpaceGeometry::new(
            CURRENT_MIN_PARKING_LATERAL_OFFSET_ABS_EXCLUSIVE_METERS.next_up(),
            0.0,
            CURRENT_MIN_PARKING_EXTENT_EXCLUSIVE_METERS.next_up(),
            CURRENT_MIN_PARKING_EXTENT_EXCLUSIVE_METERS.next_up(),
        ),
        ParkingSpaceGeometry::new(
            (-CURRENT_MIN_PARKING_LATERAL_OFFSET_ABS_EXCLUSIVE_METERS).next_down(),
            0.0,
            5.0,
            2.4,
        ),
    ] {
        let valid = ParkingSpace::new("space", None, "edge", 10.0, "edge", 20.0, geometry);
        ParkingRegistry::try_new(&graph, [], [valid])
            .expect("value adjacent above its exclusive magnitude minimum must pass");
    }
}

#[test]
fn orphan_area_fails_after_space_geometry() {
    let graph = graph(&["edge"]);
    let invalid_geometry = ParkingSpace::new(
        "standalone",
        None,
        "edge",
        10.0,
        "edge",
        20.0,
        ParkingSpaceGeometry::new(0.0, 0.0, 5.0, 2.4),
    );
    let error = ParkingRegistry::try_new(&graph, [ParkingArea::new("orphan")], [invalid_geometry])
        .expect_err("geometry must fail before orphan detection");
    std::assert_matches!(
        error,
        CoreError::InvalidParkingGeometryValue {
            field: "lateralOffset",
            ..
        }
    );
}

#[test]
fn initial_traffic_data_rebinds_parking_to_its_own_graph() {
    let source_graph = graph(&["entry", "exit"]);
    let parking =
        ParkingRegistry::try_new(&source_graph, [], [space("space", None, "entry", "exit")])
            .expect("source parking registry");
    let target_graph = graph(&["exit", "entry"]);
    let traffic = InitialTrafficData::try_new_with_signals_and_parking(
        target_graph,
        [],
        VehicleProfileRegistry::empty(),
        SignalRegistry::empty(),
        parking,
    )
    .expect("parking must rebind");
    let handle = traffic.parking().space_handle("space").expect("space");
    assert_eq!(
        traffic
            .parking()
            .space_entry(handle)
            .map(|anchor| anchor.edge()),
        traffic.lane_graph().edge_handle("entry")
    );
    assert_eq!(
        traffic
            .parking()
            .space_exit(handle)
            .map(|anchor| anchor.edge()),
        traffic.lane_graph().edge_handle("exit")
    );
}

#[test]
fn initial_traffic_data_rejects_parking_that_cannot_rebind() {
    let source_graph = graph(&["entry", "exit"]);
    let parking =
        ParkingRegistry::try_new(&source_graph, [], [space("space", None, "entry", "exit")])
            .expect("source parking registry");
    let target_graph = graph(&["entry"]);
    let error = InitialTrafficData::try_new_with_signals_and_parking(
        target_graph,
        [],
        VehicleProfileRegistry::empty(),
        SignalRegistry::empty(),
        parking,
    )
    .expect_err("missing exit edge must fail during final rebind");
    std::assert_matches!(
        error,
        CoreError::UnknownParkingAnchorEdge {
            space_id,
            anchor: ParkingAnchorKind::Exit,
            edge_id,
        } if space_id == "space" && edge_id == "exit"
    );
}

#[test]
fn static_queries_reject_handles_outside_registry_bounds() {
    let graph = graph(&["edge"]);
    let large = ParkingRegistry::try_new(
        &graph,
        [
            ParkingArea::new("first-area"),
            ParkingArea::new("second-area"),
        ],
        [
            space("first", Some("first-area"), "edge", "edge"),
            space("second", Some("second-area"), "edge", "edge"),
        ],
    )
    .expect("large registry");
    let foreign_area = large.area_handle("second-area").expect("area");
    let foreign_space = large.space_handle("second").expect("space");
    let small = ParkingRegistry::try_new(
        &graph,
        [ParkingArea::new("only-area")],
        [space("only", Some("only-area"), "edge", "edge")],
    )
    .expect("small registry");

    assert!(small.area(foreign_area).is_none());
    assert!(small.area_spaces(foreign_area).is_none());
    assert!(small.space(foreign_space).is_none());
    assert_eq!(small.space_area(foreign_space), None);
    assert!(small.space_entry(foreign_space).is_none());
    assert!(small.space_exit(foreign_space).is_none());
    assert!(small.space_geometry(foreign_space).is_none());
}

#[test]
fn core_world_exposes_same_immutable_parking_registry() {
    let graph = graph(&["edge"]);
    let parking = ParkingRegistry::try_new(&graph, [], [space("standalone", None, "edge", "edge")])
        .expect("parking");
    let traffic = InitialTrafficData::try_new_with_signals_and_parking(
        graph,
        [],
        VehicleProfileRegistry::empty(),
        SignalRegistry::empty(),
        parking,
    )
    .expect("traffic");
    let world =
        laneflow_core::CoreWorld::with_traffic_data(16, traffic, Vec::new()).expect("world");
    assert_eq!(world.parking().spaces().count(), 1);
    assert_eq!(
        world.parking().spaces().next().map(ParkingSpace::id),
        Some("standalone")
    );
}
