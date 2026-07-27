use laneflow_core::{
    CoreError, CoreWorld, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData, Junction,
    JunctionRegistry, LaneEdge, LaneGraph, MAX_PORTABLE_SIGNAL_TIME_MS, ManeuverGate,
    ManeuverGateHandle, ManeuverPath, Movement, Route, SignalAspect, SignalControl,
    SignalControlInput, SignalController, SignalControllerHandle, SignalGroup, SignalGroupHandle,
    SignalGroupState, SignalPhase, SignalPhaseRef, SignalRegistry, Speed, StopLine, StopLineHandle,
    StopLineLocation, VehicleProfile, VehicleProfileRegistry, VehicleSpawnInput, VehicleStatus,
};

fn edge_length(value: f64) -> EdgeLength {
    EdgeLength::try_new(value).expect("valid edge length")
}

fn canonical_graph() -> LaneGraph {
    LaneGraph::try_new([
        LaneEdge::new(
            "entry",
            edge_length(100.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["through", "bypass"],
        ),
        LaneEdge::new(
            "through",
            edge_length(40.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
        LaneEdge::new(
            "bypass",
            edge_length(30.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
    ])
    .expect("valid graph")
}

fn canonical_junctions(graph: &LaneGraph) -> JunctionRegistry {
    JunctionRegistry::try_new(
        graph,
        [Junction::new("junction")],
        [
            Movement::new("movement-through", "junction"),
            Movement::new("movement-bypass", "junction"),
        ],
        [
            ManeuverPath::new(
                "path-through",
                "movement-through",
                "entry",
                std::iter::empty::<&str>(),
                "through",
            ),
            ManeuverPath::new(
                "path-bypass",
                "movement-bypass",
                "entry",
                std::iter::empty::<&str>(),
                "bypass",
            ),
        ],
    )
    .expect("valid topology")
}

fn single_path_junctions(graph: &LaneGraph) -> JunctionRegistry {
    JunctionRegistry::try_new(
        graph,
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
    .expect("valid single-path topology")
}

fn stop_line(id: &str, edge_id: &str) -> StopLine {
    StopLine::new(id, edge_id, StopLineLocation::EdgeEnd)
}

fn state(group_id: &str, aspect: SignalAspect) -> SignalGroupState {
    SignalGroupState::new(group_id, aspect)
}

fn phase(id: &str, duration_ms: u64, states: Vec<SignalGroupState>) -> SignalPhase {
    SignalPhase::new(id, duration_ms, states)
}

fn controller(
    id: &str,
    offset_ms: u64,
    group_ids: &[&str],
    phases: Vec<SignalPhase>,
) -> SignalController {
    SignalController::new_fixed_time(id, offset_ms, group_ids.iter().copied(), phases)
}

fn canonical_registry(graph: &LaneGraph, junctions: &JunctionRegistry) -> SignalRegistry {
    SignalRegistry::try_new(
        graph,
        junctions,
        [stop_line("stop-entry", "entry")],
        [SignalGroup::new("main")],
        [controller(
            "controller-main",
            10,
            &["main"],
            vec![
                phase("green", 30_000, vec![state("main", SignalAspect::Green)]),
                phase("yellow", 3_000, vec![state("main", SignalAspect::Yellow)]),
                phase("red", 20_000, vec![state("main", SignalAspect::Red)]),
            ],
        )],
        [
            ManeuverGate::new(
                "gate-through",
                "path-through",
                0,
                "stop-entry",
                SignalControlInput::Group("main".to_owned()),
            ),
            ManeuverGate::new(
                "gate-bypass",
                "path-bypass",
                0,
                "stop-entry",
                SignalControlInput::None,
            ),
        ],
    )
    .expect("canonical Signals must normalize")
}

fn profile_registry() -> (VehicleProfileRegistry, laneflow_core::VehicleProfileHandle) {
    let registry = VehicleProfileRegistry::try_new(
        &participant_classes().0,
        [VehicleProfile::try_new_iidm(
            "passenger-car",
            participant_classes().1,
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
        .expect("valid profile")],
    )
    .expect("valid registry");
    let handle = registry
        .profile_handle("passenger-car")
        .expect("profile handle");
    (registry, handle)
}

#[test]
fn canonical_registry_resolves_first_class_maneuver_gate_handles() {
    fn assert_traits<T: Clone + Copy + std::fmt::Debug + Eq + std::hash::Hash>() {}
    assert_traits::<StopLineHandle>();
    assert_traits::<SignalGroupHandle>();
    assert_traits::<SignalControllerHandle>();
    assert_traits::<SignalPhaseRef>();
    assert_traits::<ManeuverGateHandle>();

    let graph = canonical_graph();
    let junctions = canonical_junctions(&graph);
    let signals = canonical_registry(&graph, &junctions);
    let stop = signals.stop_line_handle("stop-entry").expect("stop line");
    let group = signals.group_handle("main").expect("group");
    let controller = signals
        .controller_handle("controller-main")
        .expect("controller");
    let through = signals
        .maneuver_gate_handle("gate-through")
        .expect("through gate");
    let bypass = signals
        .maneuver_gate_handle("gate-bypass")
        .expect("bypass gate");

    assert_eq!(signals.stop_line_external_id(stop), Some("stop-entry"));
    assert_eq!(signals.group_controller(group), Some(controller));
    assert_eq!(
        signals.controller_cycle_duration_ms(controller),
        Some(53_000)
    );
    assert_eq!(
        signals.maneuver_gate_control(through),
        Some(SignalControl::Group(group))
    );
    assert_eq!(signals.maneuver_gate_stop_line(through), Some(stop));
    assert_eq!(
        signals.maneuver_gate_path(through),
        junctions.maneuver_path_handle("path-through")
    );
    assert_eq!(
        signals.maneuver_gate_control(bypass),
        Some(SignalControl::None)
    );
    assert_eq!(
        signals.maneuver_gates().collect::<Vec<_>>(),
        [through, bypass]
    );
    assert_eq!(
        signals.maneuver_gate_external_id(through),
        Some("gate-through")
    );

    let empty = SignalRegistry::empty();
    assert_eq!(empty.maneuver_gate(through), None);
    assert_eq!(empty.maneuver_gate_control(through), None);
}

#[test]
fn signal_identity_and_controller_validation_keep_stable_first_errors() {
    let graph = canonical_graph();
    let empty = JunctionRegistry::empty();
    let duplicate_stop = SignalRegistry::try_new(
        &graph,
        &empty,
        [
            stop_line("duplicate", "entry"),
            stop_line("duplicate", "bad-edge"),
        ],
        std::iter::empty::<SignalGroup>(),
        std::iter::empty::<SignalController>(),
        std::iter::empty::<ManeuverGate>(),
    )
    .expect_err("duplicate StopLine ID must fail first");
    std::assert_matches!(
        duplicate_stop,
        CoreError::DuplicateStopLineId { stop_line_id } if stop_line_id == "duplicate"
    );

    let duplicate_group = SignalRegistry::try_new(
        &graph,
        &empty,
        std::iter::empty::<StopLine>(),
        [SignalGroup::new("same"), SignalGroup::new("same")],
        std::iter::empty::<SignalController>(),
        std::iter::empty::<ManeuverGate>(),
    )
    .expect_err("duplicate group");
    std::assert_matches!(
        duplicate_group,
        CoreError::DuplicateSignalGroupId { group_id } if group_id == "same"
    );

    for duration_ms in [0, MAX_PORTABLE_SIGNAL_TIME_MS + 1] {
        let error = SignalRegistry::try_new(
            &graph,
            &empty,
            std::iter::empty::<StopLine>(),
            [SignalGroup::new("main")],
            [controller(
                "controller",
                0,
                &["main"],
                vec![phase(
                    "phase",
                    duration_ms,
                    vec![state("main", SignalAspect::Red)],
                )],
            )],
            std::iter::empty::<ManeuverGate>(),
        )
        .expect_err("invalid phase duration");
        std::assert_matches!(
            error,
            CoreError::InvalidSignalPhaseDuration {
                duration_ms: actual,
                ..
            } if actual == duration_ms
        );
    }
}

#[test]
fn maneuver_gate_validation_uses_path_identity_and_profile_order() {
    let graph = canonical_graph();
    let junctions = canonical_junctions(&graph);
    let base_groups = || [SignalGroup::new("main")];
    let base_controllers = || {
        [controller(
            "controller",
            0,
            &["main"],
            vec![phase("phase", 100, vec![state("main", SignalAspect::Red)])],
        )]
    };

    let error = SignalRegistry::try_new(
        &graph,
        &junctions,
        [stop_line("stop", "entry")],
        base_groups(),
        base_controllers(),
        [ManeuverGate::new(
            "gate",
            "unknown",
            0,
            "stop",
            SignalControlInput::Group("main".to_owned()),
        )],
    )
    .expect_err("unknown path");
    std::assert_matches!(
        error,
        CoreError::UnknownManeuverGatePath {
            maneuver_path_id,
            ..
        } if maneuver_path_id == "unknown"
    );

    let error = SignalRegistry::try_new(
        &graph,
        &junctions,
        [stop_line("stop", "entry")],
        base_groups(),
        base_controllers(),
        [ManeuverGate::new(
            "gate",
            "path-through",
            1,
            "stop",
            SignalControlInput::Group("main".to_owned()),
        )],
    )
    .expect_err("out of range transition");
    std::assert_matches!(
        error,
        CoreError::ManeuverGateTransitionOutOfRange {
            transition_index: 1,
            ..
        }
    );

    let error = SignalRegistry::try_new(
        &graph,
        &junctions,
        [stop_line("stop", "entry")],
        base_groups(),
        base_controllers(),
        [
            ManeuverGate::new(
                "first",
                "path-through",
                0,
                "stop",
                SignalControlInput::Group("main".to_owned()),
            ),
            ManeuverGate::new(
                "duplicate",
                "path-through",
                0,
                "bad stop id",
                SignalControlInput::Group("main".to_owned()),
            ),
        ],
    )
    .expect_err("duplicate path transition before later fields");
    std::assert_matches!(
        error,
        CoreError::DuplicateManeuverGatePathTransition {
            first_maneuver_gate_id,
            duplicate_maneuver_gate_id,
            ..
        } if first_maneuver_gate_id == "first" && duplicate_maneuver_gate_id == "duplicate"
    );

    let error = SignalRegistry::try_new(
        &graph,
        &junctions,
        [stop_line("stop", "entry")],
        base_groups(),
        base_controllers(),
        [ManeuverGate::new(
            "gate",
            "path-through",
            0,
            "unknown",
            SignalControlInput::Group("main".to_owned()),
        )],
    )
    .expect_err("unknown StopLine");
    std::assert_matches!(
        error,
        CoreError::UnknownManeuverGateStopLine { stop_line_id, .. }
            if stop_line_id == "unknown"
    );

    let error = SignalRegistry::try_new(
        &graph,
        &junctions,
        [stop_line("wrong-stop", "through")],
        base_groups(),
        base_controllers(),
        [ManeuverGate::new(
            "gate",
            "path-through",
            0,
            "wrong-stop",
            SignalControlInput::Group("main".to_owned()),
        )],
    )
    .expect_err("StopLine must be on path from-edge");
    std::assert_matches!(
        error,
        CoreError::ManeuverGateStopLineMismatch {
            path_from_edge_id,
            stop_line_edge_id,
            ..
        } if path_from_edge_id == "entry" && stop_line_edge_id == "through"
    );
}

#[test]
fn duplicate_non_entry_gate_transition_precedes_profile_restriction() {
    let graph = LaneGraph::try_new([
        LaneEdge::new(
            "entry",
            edge_length(10.0),
            laneflow_core::SpeedLimit::try_new(10.0).expect("speed limit"),
            ["internal"],
        ),
        LaneEdge::new(
            "internal",
            edge_length(10.0),
            laneflow_core::SpeedLimit::try_new(10.0).expect("speed limit"),
            ["exit"],
        ),
        LaneEdge::new(
            "exit",
            edge_length(10.0),
            laneflow_core::SpeedLimit::try_new(10.0).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
    ])
    .expect("valid graph");
    let junctions = JunctionRegistry::try_new(
        &graph,
        [Junction::new("junction")],
        [Movement::new("movement", "junction")],
        [ManeuverPath::new(
            "path",
            "movement",
            "entry",
            ["internal"],
            "exit",
        )],
    )
    .expect("valid topology");
    let groups = || [SignalGroup::new("main")];
    let controllers = || {
        [controller(
            "controller",
            0,
            &["main"],
            vec![phase("phase", 100, vec![state("main", SignalAspect::Red)])],
        )]
    };

    let duplicate = SignalRegistry::try_new(
        &graph,
        &junctions,
        [stop_line("stop", "internal")],
        groups(),
        controllers(),
        [
            ManeuverGate::new(
                "first",
                "path",
                1,
                "stop",
                SignalControlInput::Group("main".to_owned()),
            ),
            ManeuverGate::new(
                "duplicate",
                "path",
                1,
                "stop",
                SignalControlInput::Group("main".to_owned()),
            ),
        ],
    )
    .expect_err("duplicate transition must precede the entry-only profile restriction");
    std::assert_matches!(
        duplicate,
        CoreError::DuplicateManeuverGatePathTransition {
            maneuver_path_id,
            transition_index: 1,
            first_maneuver_gate_id,
            duplicate_maneuver_gate_id,
        } if maneuver_path_id == "path"
            && first_maneuver_gate_id == "first"
            && duplicate_maneuver_gate_id == "duplicate"
    );

    let unsupported = SignalRegistry::try_new(
        &graph,
        &junctions,
        [stop_line("stop", "internal")],
        groups(),
        controllers(),
        [ManeuverGate::new(
            "gate",
            "path",
            1,
            "stop",
            SignalControlInput::Group("main".to_owned()),
        )],
    )
    .expect_err("a unique non-entry transition remains unsupported");
    std::assert_matches!(
        unsupported,
        CoreError::UnsupportedManeuverGateTransition {
            maneuver_gate_id,
            transition_index: 1,
        } if maneuver_gate_id == "gate"
    );
}

#[test]
fn stop_line_requires_path_and_gate_coverage_for_every_outgoing_connection() {
    let graph = canonical_graph();
    let no_bypass = JunctionRegistry::try_new(
        &graph,
        [Junction::new("junction")],
        [Movement::new("movement", "junction")],
        [ManeuverPath::new(
            "path-through",
            "movement",
            "entry",
            std::iter::empty::<&str>(),
            "through",
        )],
    )
    .expect("partial topology");
    let error = SignalRegistry::try_new(
        &graph,
        &no_bypass,
        [stop_line("stop", "entry")],
        [SignalGroup::new("main")],
        [controller(
            "controller",
            0,
            &["main"],
            vec![phase(
                "phase",
                100,
                vec![state("main", SignalAspect::Green)],
            )],
        )],
        [ManeuverGate::new(
            "gate-through",
            "path-through",
            0,
            "stop",
            SignalControlInput::Group("main".to_owned()),
        )],
    )
    .expect_err("bypass connection lacks path coverage");
    std::assert_matches!(
        error,
        CoreError::MissingManeuverPathCoverage { to_edge_id, .. }
            if to_edge_id == "bypass"
    );

    let junctions = canonical_junctions(&graph);
    let error = SignalRegistry::try_new(
        &graph,
        &junctions,
        [stop_line("stop", "entry")],
        [SignalGroup::new("main")],
        [controller(
            "controller",
            0,
            &["main"],
            vec![phase(
                "phase",
                100,
                vec![state("main", SignalAspect::Green)],
            )],
        )],
        [ManeuverGate::new(
            "gate-through",
            "path-through",
            0,
            "stop",
            SignalControlInput::Group("main".to_owned()),
        )],
    )
    .expect_err("bypass path lacks gate coverage");
    std::assert_matches!(
        error,
        CoreError::MissingManeuverGateCoverage {
            maneuver_path_id,
            ..
        } if maneuver_path_id == "path-bypass"
    );
}

#[test]
fn initial_traffic_data_rebinds_topology_signals_and_route_compilation() {
    let source_graph = canonical_graph();
    let source_junctions = canonical_junctions(&source_graph);
    let signals = canonical_registry(&source_graph, &source_junctions);
    let reordered_graph = LaneGraph::try_new([
        LaneEdge::new(
            "bypass",
            edge_length(30.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
        LaneEdge::new(
            "entry",
            edge_length(100.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["through", "bypass"],
        ),
        LaneEdge::new(
            "through",
            edge_length(40.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
    ])
    .expect("same topology in different order");
    let traffic = InitialTrafficData::try_new(
        reordered_graph,
        [Route::try_new("route", ["entry", "through"]).expect("route")],
        VehicleProfileRegistry::empty(),
        source_junctions,
        signals,
        laneflow_core::ParkingRegistry::empty(),
        laneflow_core::ParticipantClassRegistry::empty(),
        laneflow_core::CrossSectionRegistry::empty(),
        laneflow_core::AccessRegistry::empty(),
    )
    .expect("all graph-dependent domains rebind");
    let gate = traffic
        .signals()
        .maneuver_gate_handle("gate-through")
        .expect("gate");
    let path = traffic
        .junctions()
        .maneuver_path_handle("path-through")
        .expect("path");
    assert_eq!(traffic.signals().maneuver_gate_path(gate), Some(path));
    assert_eq!(
        traffic.signals().stop_line_edge(
            traffic
                .signals()
                .stop_line_handle("stop-entry")
                .expect("stop line")
        ),
        traffic.lane_graph().edge_handle("entry")
    );
}

#[test]
fn signal_registry_rebinds_foreign_topology_before_resolving_gate_edges() {
    let source_graph = LaneGraph::try_new([
        LaneEdge::new(
            "padding-a",
            edge_length(10.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
        LaneEdge::new(
            "padding-b",
            edge_length(10.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
        LaneEdge::new(
            "entry",
            edge_length(10.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["exit"],
        ),
        LaneEdge::new(
            "exit",
            edge_length(10.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
    ])
    .expect("source graph");
    let source_junctions = single_path_junctions(&source_graph);
    let target_graph = LaneGraph::try_new([LaneEdge::new(
        "stop-edge",
        edge_length(10.0),
        laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
        std::iter::empty::<&str>(),
    )])
    .expect("target graph");

    let error = SignalRegistry::try_new(
        &target_graph,
        &source_junctions,
        [stop_line("stop", "stop-edge")],
        std::iter::empty::<SignalGroup>(),
        std::iter::empty::<SignalController>(),
        [ManeuverGate::new(
            "gate",
            "path",
            0,
            "stop",
            SignalControlInput::None,
        )],
    )
    .expect_err("foreign topology must fail during target-graph rebind");

    std::assert_matches!(
        error,
        CoreError::UnknownManeuverPathEdge {
            maneuver_path_id,
            role: "entry",
            edge_id,
        } if maneuver_path_id == "path" && edge_id == "entry"
    );
}

#[test]
fn signal_registry_rejects_same_index_foreign_topology_aliases() {
    let source_graph = LaneGraph::try_new([
        LaneEdge::new(
            "entry",
            edge_length(10.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["exit"],
        ),
        LaneEdge::new(
            "exit",
            edge_length(10.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
    ])
    .expect("source graph");
    let source_junctions = single_path_junctions(&source_graph);
    let target_graph = LaneGraph::try_new([
        LaneEdge::new(
            "wrong-entry",
            edge_length(10.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["wrong-exit"],
        ),
        LaneEdge::new(
            "wrong-exit",
            edge_length(10.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
    ])
    .expect("same-index target graph");

    let error = SignalRegistry::try_new(
        &target_graph,
        &source_junctions,
        [stop_line("stop", "wrong-entry")],
        std::iter::empty::<SignalGroup>(),
        std::iter::empty::<SignalController>(),
        [ManeuverGate::new(
            "gate",
            "path",
            0,
            "stop",
            SignalControlInput::None,
        )],
    )
    .expect_err("same-index foreign handles must not alias target edges");

    std::assert_matches!(
        error,
        CoreError::UnknownManeuverPathEdge {
            maneuver_path_id,
            role: "entry",
            edge_id,
        } if maneuver_path_id == "path" && edge_id == "entry"
    );
}

#[test]
fn signal_registry_rebinds_foreign_topology_with_reordered_edges() {
    let source_graph = canonical_graph();
    let source_junctions = canonical_junctions(&source_graph);
    let reordered_graph = LaneGraph::try_new([
        LaneEdge::new(
            "bypass",
            edge_length(30.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
        LaneEdge::new(
            "entry",
            edge_length(100.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            ["through", "bypass"],
        ),
        LaneEdge::new(
            "through",
            edge_length(40.0),
            laneflow_core::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
    ])
    .expect("same topology in different order");

    let signals = canonical_registry(&reordered_graph, &source_junctions);
    let stop_line = signals.stop_line_handle("stop-entry").expect("stop line");
    let through_gate = signals
        .maneuver_gate_handle("gate-through")
        .expect("through gate");
    let through_path = source_junctions
        .maneuver_path_handle("path-through")
        .expect("through path");

    assert_eq!(
        signals.stop_line_edge(stop_line),
        reordered_graph.edge_handle("entry")
    );
    assert_eq!(signals.maneuver_gate_path(through_gate), Some(through_path));
}

#[test]
fn route_stopline_rule_and_phase_delta_remain_atomic_with_new_domains() {
    let graph = canonical_graph();
    let junctions = canonical_junctions(&graph);
    let signals = canonical_registry(&graph, &junctions);
    let error = InitialTrafficData::try_new(
        graph.clone(),
        [Route::try_new("invalid", ["entry"]).expect("route")],
        VehicleProfileRegistry::empty(),
        junctions.clone(),
        signals.clone(),
        laneflow_core::ParkingRegistry::empty(),
        laneflow_core::ParticipantClassRegistry::empty(),
        laneflow_core::CrossSectionRegistry::empty(),
        laneflow_core::AccessRegistry::empty(),
    )
    .expect_err("route cannot terminate at StopLine");
    std::assert_matches!(
        error,
        CoreError::RouteTerminatesAtStopLine { route_id, .. } if route_id == "invalid"
    );

    let short_signals = SignalRegistry::try_new(
        &graph,
        &junctions,
        [stop_line("stop", "entry")],
        [SignalGroup::new("main")],
        [controller(
            "controller",
            0,
            &["main"],
            vec![phase("short", 15, vec![state("main", SignalAspect::Red)])],
        )],
        [
            ManeuverGate::new(
                "gate-through",
                "path-through",
                0,
                "stop",
                SignalControlInput::Group("main".to_owned()),
            ),
            ManeuverGate::new(
                "gate-bypass",
                "path-bypass",
                0,
                "stop",
                SignalControlInput::None,
            ),
        ],
    )
    .expect("static Signals");
    let traffic = InitialTrafficData::try_new(
        graph.clone(),
        [Route::try_new("route", ["entry", "through"]).expect("route")],
        VehicleProfileRegistry::empty(),
        junctions.clone(),
        short_signals,
        laneflow_core::ParkingRegistry::empty(),
        laneflow_core::ParticipantClassRegistry::empty(),
        laneflow_core::CrossSectionRegistry::empty(),
        laneflow_core::AccessRegistry::empty(),
    )
    .expect("static traffic");
    let error = CoreWorld::with_traffic_data(16, traffic, Vec::new())
        .expect_err("phase shorter than fixed delta");
    std::assert_matches!(
        error,
        CoreError::SignalPhaseShorterThanFixedDelta {
            duration_ms: 15,
            fixed_delta_time_ms: 16,
            ..
        }
    );

    let (profiles, profile) = profile_registry();
    let vehicle = VehicleSpawnInput::new(
        "vehicle",
        profile,
        "route",
        0,
        EdgeProgress::ZERO,
        Speed::ZERO,
        VehicleStatus::Active,
    );
    let traffic = InitialTrafficData::try_new(
        graph,
        [Route::try_new("route", ["entry", "through"]).expect("route")],
        profiles,
        junctions,
        signals,
        laneflow_core::ParkingRegistry::empty(),
        participant_classes().0,
        laneflow_core::CrossSectionRegistry::empty(),
        laneflow_core::AccessRegistry::empty(),
    )
    .expect("valid traffic");
    let world = CoreWorld::with_traffic_data(16, traffic, vec![vehicle])
        .expect("Signals and vehicles compose");
    assert_eq!(world.vehicles().count(), 1);
}

fn participant_classes() -> (
    laneflow_core::ParticipantClassRegistry,
    laneflow_core::ParticipantClassHandle,
) {
    let classes = laneflow_core::ParticipantClassRegistry::try_new(vec![
        laneflow_core::ParticipantClass::new("motorVehicle", None),
        laneflow_core::ParticipantClass::new("car", Some("motorVehicle")),
    ])
    .expect("participant classes must be valid");
    let car = classes.class_handle("car").expect("car class must exist");
    (classes, car)
}
