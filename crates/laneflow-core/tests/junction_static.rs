use laneflow_core::{
    CoreError, EdgeLength, Junction, JunctionRegistry, LaneEdge, LaneGraph, ManeuverPath, Movement,
    SpeedLimit,
};

fn edge(id: &str, next: &[&str]) -> LaneEdge {
    LaneEdge::new(
        id,
        EdgeLength::try_new(10.0).expect("test edge length"),
        SpeedLimit::try_new(10.0).expect("test speed limit"),
        next.iter().copied(),
    )
}

fn graph() -> LaneGraph {
    LaneGraph::try_new([
        edge("entry-a", &["internal-a", "internal-b"]),
        edge("entry-b", &["internal-b"]),
        edge("internal-a", &["exit-a"]),
        edge("internal-b", &["exit-b"]),
        edge("exit-a", &[]),
        edge("exit-b", &[]),
    ])
    .expect("test graph")
}

#[test]
fn registry_preserves_normalization_and_member_order() {
    let graph = graph();
    let registry = JunctionRegistry::try_new(
        &graph,
        [Junction::new("junction-b"), Junction::new("junction-a")],
        [
            Movement::new("movement-a", "junction-a"),
            Movement::new("movement-b", "junction-b"),
        ],
        [
            ManeuverPath::new("path-b", "movement-b", "entry-b", ["internal-b"], "exit-b"),
            ManeuverPath::new("path-a", "movement-a", "entry-a", ["internal-a"], "exit-a"),
        ],
    )
    .expect("valid registry");

    let junction_ids = registry
        .junctions()
        .map(|handle| {
            registry
                .junction_external_id(handle)
                .expect("junction external ID")
        })
        .collect::<Vec<_>>();
    assert_eq!(junction_ids, ["junction-b", "junction-a"]);

    let movement_ids = registry
        .movements()
        .map(|handle| {
            registry
                .movement_external_id(handle)
                .expect("movement external ID")
        })
        .collect::<Vec<_>>();
    assert_eq!(movement_ids, ["movement-a", "movement-b"]);

    let junction_b = registry
        .junction_handle("junction-b")
        .expect("junction-b handle");
    let junction_b_movements = registry
        .junction_movements(junction_b)
        .expect("junction-b members")
        .map(|handle| {
            registry
                .movement_external_id(handle)
                .expect("member external ID")
        })
        .collect::<Vec<_>>();
    assert_eq!(junction_b_movements, ["movement-b"]);

    let path_a = registry
        .maneuver_path_handle("path-a")
        .expect("path-a handle");
    let path_a_edge_ids = registry
        .maneuver_path_edges(path_a)
        .expect("path edges")
        .iter()
        .map(|handle| graph.edge_external_id(*handle).expect("edge external ID"))
        .collect::<Vec<_>>();
    assert_eq!(path_a_edge_ids, ["entry-a", "internal-a", "exit-a"]);

    assert_eq!(
        registry.internal_edge_owner(graph.edge_handle("internal-a").expect("internal-a handle")),
        Some(junction_a_handle(&registry))
    );
}

#[test]
fn zero_internal_path_is_valid_and_rebinds_by_external_id() {
    let graph =
        LaneGraph::try_new([edge("entry", &["exit"]), edge("exit", &[])]).expect("test graph");
    let registry = JunctionRegistry::try_new(
        &graph,
        [Junction::new("junction")],
        [Movement::new("movement", "junction")],
        [ManeuverPath::new(
            "path",
            "movement",
            "entry",
            std::iter::empty::<&str>(),
            "exit",
        )],
    )
    .expect("zero-internal registry");

    let rebound_graph =
        LaneGraph::try_new([edge("exit", &[]), edge("entry", &["exit"])]).expect("rebound graph");
    let rebound = registry
        .rebind_to_lane_graph(&rebound_graph)
        .expect("rebound registry");
    let path = rebound
        .maneuver_path_handle("path")
        .expect("rebound path handle");
    let edge_ids = rebound
        .maneuver_path_edges(path)
        .expect("rebound path edges")
        .iter()
        .map(|handle| {
            rebound_graph
                .edge_external_id(*handle)
                .expect("rebound edge external ID")
        })
        .collect::<Vec<_>>();
    assert_eq!(edge_ids, ["entry", "exit"]);
}

#[test]
fn duplicate_sequence_precedes_cross_junction_internal_owner_error() {
    let graph = graph();
    let error = JunctionRegistry::try_new(
        &graph,
        [Junction::new("junction-a"), Junction::new("junction-b")],
        [
            Movement::new("movement-a", "junction-a"),
            Movement::new("movement-b", "junction-b"),
        ],
        [
            ManeuverPath::new("path-a", "movement-a", "entry-a", ["internal-a"], "exit-a"),
            ManeuverPath::new("path-b", "movement-b", "entry-a", ["internal-a"], "exit-a"),
        ],
    )
    .expect_err("duplicate path sequence must fail");

    assert!(matches!(
        error,
        CoreError::DuplicateManeuverPathSequence {
            first_maneuver_path_id,
            first_junction_id,
            duplicate_maneuver_path_id,
            duplicate_junction_id,
        } if first_maneuver_path_id == "path-a"
            && first_junction_id == "junction-a"
            && duplicate_maneuver_path_id == "path-b"
            && duplicate_junction_id == "junction-b"
    ));
}

#[test]
fn internal_edge_cannot_cross_junction_owners() {
    let graph = LaneGraph::try_new([
        edge("entry-a", &["shared"]),
        edge("entry-b", &["shared"]),
        edge("shared", &["exit-a", "exit-b"]),
        edge("exit-a", &[]),
        edge("exit-b", &[]),
    ])
    .expect("test graph");
    let error = JunctionRegistry::try_new(
        &graph,
        [Junction::new("junction-a"), Junction::new("junction-b")],
        [
            Movement::new("movement-a", "junction-a"),
            Movement::new("movement-b", "junction-b"),
        ],
        [
            ManeuverPath::new("path-a", "movement-a", "entry-a", ["shared"], "exit-a"),
            ManeuverPath::new("path-b", "movement-b", "entry-b", ["shared"], "exit-b"),
        ],
    )
    .expect_err("cross-junction internal owner must fail");

    assert!(matches!(
        error,
        CoreError::ManeuverInternalEdgeJunctionConflict {
            edge_id,
            first_junction_id,
            duplicate_junction_id,
        } if edge_id == "shared"
            && first_junction_id == "junction-a"
            && duplicate_junction_id == "junction-b"
    ));
}

#[test]
fn internal_edge_cannot_also_be_path_entry_boundary() {
    let graph = LaneGraph::try_new([edge("A", &["I"]), edge("I", &["B"]), edge("B", &[])])
        .expect("test graph");
    let error = JunctionRegistry::try_new(
        &graph,
        [Junction::new("junction-a"), Junction::new("junction-b")],
        [
            Movement::new("movement-a", "junction-a"),
            Movement::new("movement-b", "junction-b"),
        ],
        [
            ManeuverPath::new("path-a", "movement-a", "A", ["I"], "B"),
            ManeuverPath::new("path-b", "movement-b", "I", std::iter::empty::<&str>(), "B"),
        ],
    )
    .expect_err("path entry on another path's internal edge must fail");

    assert!(matches!(
        error,
        CoreError::ManeuverPathEdgeRoleConflict {
            edge_id,
            internal_maneuver_path_id,
            boundary_maneuver_path_id,
        } if edge_id == "I"
            && internal_maneuver_path_id == "path-a"
            && boundary_maneuver_path_id == "path-b"
    ));
}

#[test]
fn junction_and_movement_cardinality_are_required() {
    let graph = graph();
    let empty_junction = JunctionRegistry::try_new(
        &graph,
        [Junction::new("junction")],
        std::iter::empty::<Movement>(),
        std::iter::empty::<ManeuverPath>(),
    )
    .expect_err("empty junction must fail");
    assert!(matches!(
        empty_junction,
        CoreError::EmptyJunction { junction_id } if junction_id == "junction"
    ));

    let empty_movement = JunctionRegistry::try_new(
        &graph,
        [Junction::new("junction")],
        [Movement::new("movement", "junction")],
        std::iter::empty::<ManeuverPath>(),
    )
    .expect_err("empty movement must fail");
    assert!(matches!(
        empty_movement,
        CoreError::EmptyMovement { movement_id } if movement_id == "movement"
    ));
}

fn junction_a_handle(registry: &JunctionRegistry) -> laneflow_core::JunctionHandle {
    registry
        .junction_handle("junction-a")
        .expect("junction-a handle")
}
