use laneflow_core::{
    CoreError, EdgeLength, IidmProfileSpec, InitialTrafficData, LaneEdge, LaneGraph, Route,
    VehicleProfile, VehicleProfileRegistry,
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

fn canonical_profiles() -> VehicleProfileRegistry {
    VehicleProfileRegistry::try_new([VehicleProfile::try_new_iidm(
        "passenger-car",
        IidmProfileSpec {
            length: 4.5,
            desired_speed: 13.9,
            min_gap: 2.0,
            time_headway: 1.5,
            max_acceleration: 1.5,
            comfortable_deceleration: 2.0,
            emergency_deceleration: 6.0,
        },
    )
    .expect("valid profile")])
    .expect("valid profile registry")
}

#[test]
fn valid_initial_traffic_data_preserves_input_order_and_registry() {
    let traffic_data = InitialTrafficData::try_new(
        canonical_graph(),
        [
            Route::try_new("loop", ["A", "B", "A"]).expect("valid route"),
            Route::try_new("short", ["A", "B"]).expect("valid route"),
        ],
        canonical_profiles(),
    )
    .expect("valid initial traffic data");

    assert_eq!(
        traffic_data
            .routes()
            .iter()
            .map(Route::id)
            .collect::<Vec<_>>(),
        ["loop", "short"]
    );
    assert_eq!(traffic_data.lane_graph().edges().len(), 2);
    assert_eq!(traffic_data.vehicle_profiles().len(), 1);

    let (lane_graph, routes, profiles, signals) = traffic_data.into_parts();
    assert_eq!(lane_graph.edges().len(), 2);
    assert_eq!(routes.len(), 2);
    assert_eq!(profiles.len(), 1);
    assert!(signals.is_empty());
}

#[test]
fn duplicate_initial_route_id_is_rejected_before_later_routes() {
    let error = InitialTrafficData::try_new(
        canonical_graph(),
        [
            Route::try_new("route", ["A"]).expect("valid route"),
            Route::try_new("route", ["B"]).expect("valid route"),
            Route::try_new("later", ["missing"]).expect("valid route shape"),
        ],
        VehicleProfileRegistry::empty(),
    )
    .expect_err("duplicate route id must fail first");

    std::assert_matches!(
        error,
        CoreError::DuplicateRouteId { route_id } if route_id == "route"
    );
}

#[test]
fn initial_route_unknown_edge_uses_route_and_edge_input_order() {
    let error = InitialTrafficData::try_new(
        canonical_graph(),
        [
            Route::try_new("first", ["A", "first-missing", "second-missing"])
                .expect("valid route shape"),
            Route::try_new("second", ["third-missing"]).expect("valid route shape"),
        ],
        VehicleProfileRegistry::empty(),
    )
    .expect_err("first unknown route edge must fail");

    std::assert_matches!(
        error,
        CoreError::UnknownRouteEdge { route_id, edge_id }
            if route_id == "first" && edge_id == "first-missing"
    );
}

#[test]
fn initial_route_continuity_uses_same_core_error_as_runtime_registration() {
    let graph = LaneGraph::try_new([
        LaneEdge::new("A", edge_length(10.0), std::iter::empty::<&str>()),
        LaneEdge::new("B", edge_length(5.0), std::iter::empty::<&str>()),
    ])
    .expect("valid disconnected graph");
    let error = InitialTrafficData::try_new(
        graph,
        [Route::try_new("disconnected", ["A", "B"]).expect("valid route shape")],
        VehicleProfileRegistry::empty(),
    )
    .expect_err("disconnected route must fail");

    std::assert_matches!(
        error,
        CoreError::DisconnectedRouteEdge {
            route_id,
            from_edge_id,
            to_edge_id,
        } if route_id == "disconnected" && from_edge_id == "A" && to_edge_id == "B"
    );
}
