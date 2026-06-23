use laneflow_core::{
    CoreError, CoreWorld, EDGE_BOUNDARY_EPSILON, EdgeLength, EdgeProgress, LaneEdge, LaneGraph,
    Route, Speed, VehicleState,
};

fn edge_length(value: f64) -> EdgeLength {
    EdgeLength::try_new(value).expect("valid edge length")
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
    route_id: &str,
    route_edge_index: usize,
    edge_progress: f64,
) -> VehicleState {
    VehicleState::active(
        id,
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
    let vehicle = active_vehicle("V1", "R1", 2, 3.0);

    let world = CoreWorld::with_traffic_data(1000, lane_graph, [route], vec![vehicle])
        .expect("valid world");

    assert_eq!(world.lane_graph().edge_length("A"), Some(edge_length(10.0)));
    assert!(world.lane_graph().can_traverse("A", "B"));
    assert_eq!(
        world.route("R1").expect("route exists").edge_ids(),
        ["A", "B", "A"]
    );
    assert_eq!(world.vehicles().len(), 1);
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
fn invalid_edge_lengths_are_rejected() {
    for invalid_length in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        -1.0,
        0.0,
        EDGE_BOUNDARY_EPSILON / 2.0,
        EDGE_BOUNDARY_EPSILON,
    ] {
        let error = EdgeLength::try_new(invalid_length).expect_err("invalid length must fail");

        std::assert_matches!(
            error,
            CoreError::InvalidLaneEdgeLength {
                edge_length,
                min_exclusive
            } if (edge_length.is_nan() && invalid_length.is_nan()
                || edge_length == invalid_length)
                && min_exclusive == EDGE_BOUNDARY_EPSILON
        );
    }
}

#[test]
fn duplicate_route_id_is_rejected() {
    let error = CoreWorld::with_traffic_data(
        1000,
        canonical_graph(),
        [
            Route::try_new("R1", ["A"]).expect("valid route"),
            Route::try_new("R1", ["B"]).expect("valid route"),
        ],
        Vec::new(),
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
fn unknown_route_edge_is_rejected() {
    let route = Route::try_new("R1", ["A", "missing"]).expect("valid route shape");
    let error = CoreWorld::with_traffic_data(1000, canonical_graph(), [route], Vec::new())
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
fn disconnected_route_edge_is_rejected() {
    let lane_graph = LaneGraph::try_new([
        LaneEdge::new("A", edge_length(10.0), std::iter::empty::<&str>()),
        LaneEdge::new("B", edge_length(5.0), std::iter::empty::<&str>()),
    ])
    .expect("valid lane graph");
    let route = Route::try_new("R1", ["A", "B"]).expect("valid route shape");

    let error = CoreWorld::with_traffic_data(1000, lane_graph, [route], Vec::new())
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
    let vehicles = vec![
        active_vehicle("V1", "R1", 0, 1.0),
        active_vehicle("V1", "R1", 0, 2.0),
    ];

    let error = CoreWorld::with_traffic_data(1000, canonical_graph(), [route], vehicles)
        .expect_err("duplicate vehicle id must fail");

    std::assert_matches!(error, CoreError::DuplicateVehicleId { vehicle_id } if vehicle_id == "V1");
}

#[test]
fn unknown_vehicle_route_is_rejected() {
    let route = Route::try_new("R1", ["A"]).expect("valid route");
    let vehicle = active_vehicle("V1", "missing", 0, 1.0);

    let error = CoreWorld::with_traffic_data(1000, canonical_graph(), [route], vec![vehicle])
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
    let vehicle = active_vehicle("V1", "R1", 1, 1.0);

    let error = CoreWorld::with_traffic_data(1000, canonical_graph(), [route], vec![vehicle])
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
    let vehicle = active_vehicle("V1", "R1", 0, 11.0);

    let error = CoreWorld::with_traffic_data(1000, canonical_graph(), [route], vec![vehicle])
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
    let result = CoreWorld::with_traffic_data(1000, canonical_graph(), [route], Vec::new());

    assert!(result.is_err());
}
