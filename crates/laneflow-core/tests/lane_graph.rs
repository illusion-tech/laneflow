mod common;

use common::world_with_test_profile;
use laneflow_core::{
    CoreError, EdgeLength, EdgeProgress, LaneEdge, LaneGraph, NumericConversionStage, Route, Speed,
    VehicleProfileHandle, VehicleSpawnInput,
};

const MIN_EDGE_LENGTH_EXCLUSIVE_METERS: f32 = 1.0;
const MAX_EDGE_LENGTH_INCLUSIVE_METERS: f32 = 10_000.0;

fn edge_length(value: f64) -> EdgeLength {
    EdgeLength::try_from(value).expect("valid edge length")
}

fn canonical_graph() -> LaneGraph {
    LaneGraph::try_new([
        LaneEdge::new("A", edge_length(10.0), ["B"]),
        LaneEdge::new("B", edge_length(5.0), ["A"]),
    ])
    .expect("valid lane graph")
}

fn active_vehicle(
    id: &str,
    profile: VehicleProfileHandle,
    route_id: &str,
    route_edge_index: usize,
    edge_progress: f64,
) -> VehicleSpawnInput {
    VehicleSpawnInput::active(
        id,
        profile,
        route_id,
        route_edge_index,
        EdgeProgress::try_new(edge_progress).expect("valid progress"),
        Speed::try_new(1.0).expect("valid speed"),
    )
}

#[test]
fn valid_lane_graph_route_and_vehicle_can_initialize_world() {
    let lane_graph = canonical_graph();
    let route = Route::try_new("R1", ["A", "B", "A"]).expect("valid route");
    let world = world_with_test_profile(1_000, lane_graph, [route], |profile| {
        vec![active_vehicle("V1", profile, "R1", 2, 3.0)]
    })
    .expect("valid world");

    let edge_a = world.edge_handle("A").expect("edge A handle exists");
    let edge_b = world.edge_handle("B").expect("edge B handle exists");
    let route = world.route_handle("R1").expect("route handle exists");

    assert_eq!(
        world.lane_graph().edge_length(edge_a),
        Some(edge_length(10.0))
    );
    assert!(world.lane_graph().can_traverse(edge_a, edge_b));
    assert_eq!(
        world
            .route_edges(route)
            .expect("route edges exist")
            .iter()
            .map(|edge| world.edge_external_id(*edge).expect("edge id exists"))
            .collect::<Vec<_>>(),
        ["A", "B", "A"]
    );
    assert_eq!(world.vehicles().count(), 1);
}

#[test]
fn duplicate_edge_id_is_rejected() {
    let error = LaneGraph::try_new([
        LaneEdge::new("A", edge_length(10.0), std::iter::empty::<&str>()),
        LaneEdge::new("A", edge_length(5.0), std::iter::empty::<&str>()),
    ])
    .expect_err("duplicate edge id must fail");

    std::assert_matches!(error, CoreError::DuplicateLaneEdgeId { edge_id } if edge_id == "A");
}

#[test]
fn invalid_edge_external_id_is_rejected() {
    let error = LaneGraph::try_new([LaneEdge::new(
        "edge 1",
        edge_length(10.0),
        std::iter::empty::<&str>(),
    )])
    .expect_err("invalid edge external id must fail");

    std::assert_matches!(
        error,
        CoreError::InvalidExternalId { field, external_id, .. }
            if field == "laneGraph.edges[].id" && external_id == "edge 1"
    );
}

#[test]
fn unknown_next_edge_is_rejected() {
    let error = LaneGraph::try_new([LaneEdge::new("A", edge_length(10.0), ["missing"])])
        .expect_err("unknown next edge must fail");

    std::assert_matches!(
        error,
        CoreError::UnknownNextLaneEdge {
            edge_id,
            next_edge_id
        } if edge_id == "A" && next_edge_id == "missing"
    );
}

#[test]
fn unknown_connection_error_uses_input_and_connection_order() {
    let error = LaneGraph::try_new([
        LaneEdge::new(
            "z-source",
            edge_length(10.0),
            ["first-missing", "second-missing"],
        ),
        LaneEdge::new("a-source", edge_length(5.0), ["third-missing"]),
    ])
    .expect_err("the first unknown connection must fail validation");

    std::assert_matches!(
        error,
        CoreError::UnknownNextLaneEdge {
            edge_id,
            next_edge_id
        } if edge_id == "z-source" && next_edge_id == "first-missing"
    );
}

#[test]
fn invalid_connection_external_id_is_rejected_before_resolution() {
    let error = LaneGraph::try_new([LaneEdge::new("A", edge_length(10.0), ["bad target"])])
        .expect_err("invalid connection target id must fail");

    std::assert_matches!(
        error,
        CoreError::InvalidExternalId { field, external_id, .. }
            if field == "laneGraph.edges[].connections[].toEdgeId" && external_id == "bad target"
    );
}

#[test]
fn duplicate_connection_target_is_rejected() {
    let error = LaneGraph::try_new([
        LaneEdge::new("A", edge_length(10.0), ["B", "B"]),
        LaneEdge::new("B", edge_length(5.0), std::iter::empty::<&str>()),
    ])
    .expect_err("duplicate connection target must fail");

    std::assert_matches!(
        error,
        CoreError::DuplicateLaneEdgeConnection {
            edge_id,
            next_edge_id
        } if edge_id == "A" && next_edge_id == "B"
    );
}

#[test]
fn terminal_self_connection_and_disconnected_component_are_valid_graph_shapes() {
    let lane_graph = LaneGraph::try_new([
        LaneEdge::new("A", edge_length(10.0), ["A"]),
        LaneEdge::new("B", edge_length(5.0), std::iter::empty::<&str>()),
        LaneEdge::new("C", edge_length(7.0), std::iter::empty::<&str>()),
    ])
    .expect("terminal, self connection, and disconnected graph component are valid");

    let edge_a = lane_graph.edge_handle("A").expect("edge A handle exists");
    let edge_b = lane_graph.edge_handle("B").expect("edge B handle exists");
    let edge_c = lane_graph.edge_handle("C").expect("edge C handle exists");

    assert!(lane_graph.can_traverse(edge_a, edge_a));
    assert!(!lane_graph.can_traverse(edge_b, edge_c));
    assert_eq!(lane_graph.edges().len(), 3);
}

#[test]
fn invalid_edge_lengths_are_rejected() {
    for invalid_length in [
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        -1.0,
        -0.0,
        0.0,
        MIN_EDGE_LENGTH_EXCLUSIVE_METERS.next_down(),
        MIN_EDGE_LENGTH_EXCLUSIVE_METERS,
        MAX_EDGE_LENGTH_INCLUSIVE_METERS.next_up(),
    ] {
        let error = EdgeLength::try_new(invalid_length).expect_err("invalid length must fail");

        std::assert_matches!(
            error,
            CoreError::InvalidLaneEdgeLength {
                edge_length,
                min_exclusive,
                max_inclusive,
            } if (edge_length.is_nan() && invalid_length.is_nan()
                || edge_length == invalid_length)
                && min_exclusive == MIN_EDGE_LENGTH_EXCLUSIVE_METERS
                && max_inclusive == MAX_EDGE_LENGTH_INCLUSIVE_METERS
        );
    }

    EdgeLength::try_new(MIN_EDGE_LENGTH_EXCLUSIVE_METERS.next_up())
        .expect("value adjacent above the exclusive minimum must pass");
    EdgeLength::try_new(MAX_EDGE_LENGTH_INCLUSIVE_METERS)
        .expect("inclusive product maximum must pass");
}

#[test]
fn raw_edge_length_checks_f64_range_before_target_conversion() {
    for invalid_length in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        -0.0,
        1.0,
        1.0_f64.next_down(),
        10_000.0_f64.next_up(),
    ] {
        std::assert_matches!(
            EdgeLength::try_from(invalid_length),
            Err(CoreError::InvalidLaneEdgeLengthInput {
                edge_length,
                stage: NumericConversionStage::RawInput,
                ..
            }) if edge_length.to_bits() == invalid_length.to_bits()
                || edge_length.is_nan() && invalid_length.is_nan()
        );
    }

    std::assert_matches!(
        EdgeLength::try_from(1.0_f64.next_up()),
        Err(CoreError::InvalidLaneEdgeLengthInput {
            edge_length,
            stage: NumericConversionStage::TargetValue,
            ..
        }) if edge_length == 1.0_f64.next_up()
    );
    assert_eq!(
        EdgeLength::try_from(f64::from(1.0_f32.next_up()))
            .expect("first representable target value must pass")
            .value(),
        1.0_f32.next_up()
    );
    assert_eq!(
        EdgeLength::try_from(10_000.0)
            .expect("inclusive raw maximum must pass")
            .value(),
        10_000.0
    );
}

#[test]
fn duplicate_route_id_is_rejected() {
    let error = world_with_test_profile(
        1_000,
        canonical_graph(),
        [
            Route::try_new("R1", ["A"]).expect("valid route"),
            Route::try_new("R1", ["B"]).expect("valid route"),
        ],
        |_| Vec::new(),
    )
    .expect_err("duplicate route id must fail");

    std::assert_matches!(error, CoreError::DuplicateRouteId { route_id } if route_id == "R1");
}

#[test]
fn empty_route_is_rejected() {
    let error =
        Route::try_new("R1", std::iter::empty::<&str>()).expect_err("empty route must fail");

    std::assert_matches!(error, CoreError::EmptyRoute { route_id } if route_id == "R1");
}

#[test]
fn invalid_route_external_id_is_rejected() {
    let error = Route::try_new("route 1", ["A"]).expect_err("invalid route id must fail");

    std::assert_matches!(
        error,
        CoreError::InvalidExternalId { field, external_id, .. }
            if field == "routes[].id" && external_id == "route 1"
    );
}

#[test]
fn invalid_route_edge_external_id_is_rejected() {
    let error = Route::try_new("R1", ["bad edge"]).expect_err("invalid route edge id must fail");

    std::assert_matches!(
        error,
        CoreError::InvalidExternalId { field, external_id, .. }
            if field == "routes[].edgeIds[]" && external_id == "bad edge"
    );
}

#[test]
fn unknown_route_edge_is_rejected() {
    let route = Route::try_new("R1", ["A", "missing"]).expect("valid route shape");
    let error = world_with_test_profile(1_000, canonical_graph(), [route], |_| Vec::new())
        .expect_err("unknown route edge must fail");

    std::assert_matches!(
        error,
        CoreError::UnknownRouteEdge {
            route_id,
            edge_id
        } if route_id == "R1" && edge_id == "missing"
    );
}

#[test]
fn unknown_route_edge_error_uses_registration_and_edge_sequence_order() {
    let first = Route::try_new("z-first", ["A", "first-missing", "second-missing"])
        .expect("valid route shape");
    let second = Route::try_new("a-second", ["A", "third-missing"]).expect("valid route shape");

    let error = world_with_test_profile(1_000, canonical_graph(), [first, second], |_| Vec::new())
        .expect_err("the first unknown route edge must fail validation");

    std::assert_matches!(
        error,
        CoreError::UnknownRouteEdge { route_id, edge_id }
            if route_id == "z-first" && edge_id == "first-missing"
    );
}

#[test]
fn disconnected_route_edge_is_rejected() {
    let lane_graph = LaneGraph::try_new([
        LaneEdge::new("A", edge_length(10.0), std::iter::empty::<&str>()),
        LaneEdge::new("B", edge_length(5.0), std::iter::empty::<&str>()),
    ])
    .expect("valid lane graph");
    let route = Route::try_new("R1", ["A", "B"]).expect("valid route shape");

    let error = world_with_test_profile(1_000, lane_graph, [route], |_| Vec::new())
        .expect_err("disconnected route must fail");

    std::assert_matches!(
        error,
        CoreError::DisconnectedRouteEdge {
            route_id,
            from_edge_id,
            to_edge_id
        } if route_id == "R1" && from_edge_id == "A" && to_edge_id == "B"
    );
}

#[test]
fn duplicate_vehicle_id_is_rejected() {
    let route = Route::try_new("R1", ["A"]).expect("valid route");
    let error = world_with_test_profile(1_000, canonical_graph(), [route], |profile| {
        vec![
            active_vehicle("V1", profile, "R1", 0, 1.0),
            active_vehicle("V1", profile, "R1", 0, 2.0),
        ]
    })
    .expect_err("duplicate vehicle id must fail");

    std::assert_matches!(error, CoreError::DuplicateVehicleId { vehicle_id } if vehicle_id == "V1");
}

#[test]
fn unknown_vehicle_route_is_rejected() {
    let route = Route::try_new("R1", ["A"]).expect("valid route");
    let error = world_with_test_profile(1_000, canonical_graph(), [route], |profile| {
        vec![active_vehicle("V1", profile, "missing", 0, 1.0)]
    })
    .expect_err("unknown vehicle route must fail");

    std::assert_matches!(
        error,
        CoreError::UnknownVehicleRoute {
            vehicle_id,
            route_id
        } if vehicle_id == "V1" && route_id == "missing"
    );
}

#[test]
fn vehicle_route_edge_index_out_of_range_is_rejected() {
    let route = Route::try_new("R1", ["A"]).expect("valid route");
    let error = world_with_test_profile(1_000, canonical_graph(), [route], |profile| {
        vec![active_vehicle("V1", profile, "R1", 1, 1.0)]
    })
    .expect_err("invalid vehicle route edge index must fail");

    std::assert_matches!(
        error,
        CoreError::InvalidVehicleRouteEdgeIndex {
            vehicle_id,
            route_id,
            route_edge_index: 1,
            route_edge_count: 1
        } if vehicle_id == "V1" && route_id == "R1"
    );
}

#[test]
fn vehicle_edge_progress_above_edge_length_is_rejected() {
    let route = Route::try_new("R1", ["A"]).expect("valid route");
    let error = world_with_test_profile(1_000, canonical_graph(), [route], |profile| {
        vec![active_vehicle("V1", profile, "R1", 0, 11.0)]
    })
    .expect_err("edge progress above edge length must fail");

    std::assert_matches!(
        error,
        CoreError::VehicleEdgeProgressOutOfRange {
            vehicle_id,
            edge_id,
            edge_progress: 11.0,
            edge_length: 10.0
        } if vehicle_id == "V1" && edge_id == "A"
    );
}

#[test]
fn validation_failure_does_not_return_partial_world() {
    let route = Route::try_new("R1", ["A", "missing"]).expect("valid route shape");
    let result = world_with_test_profile(1_000, canonical_graph(), [route], |_| Vec::new());

    assert!(result.is_err());
}
