use laneflow_core::{
    CoreError, CoreWorld, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData, LaneEdge,
    LaneGraph, MAX_PORTABLE_SIGNAL_TIME_MS, MovementGate, MovementGateKey, Route, SignalAspect,
    SignalControl, SignalControlInput, SignalController, SignalControllerHandle, SignalGroup,
    SignalGroupHandle, SignalGroupState, SignalPhase, SignalPhaseRef, SignalRegistry, Speed,
    StopLine, StopLineHandle, StopLineLocation, VehicleProfile, VehicleProfileRegistry,
    VehicleSpawnInput, VehicleStatus,
};

fn edge_length(value: f64) -> EdgeLength {
    EdgeLength::try_new(value).expect("valid edge length")
}

fn canonical_graph() -> LaneGraph {
    LaneGraph::try_new([
        LaneEdge::new("entry", edge_length(100.0), ["through", "bypass"]),
        LaneEdge::new("through", edge_length(40.0), std::iter::empty::<&str>()),
        LaneEdge::new("bypass", edge_length(30.0), std::iter::empty::<&str>()),
    ])
    .expect("valid graph")
}

fn stop_line(id: &str, edge_id: &str) -> StopLine {
    StopLine::new(id, edge_id, StopLineLocation::EdgeEnd)
}

fn group(id: &str) -> SignalGroup {
    SignalGroup::new(id)
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

fn group_gate(from: &str, to: &str, stop_line_id: &str, group_id: &str) -> MovementGate {
    MovementGate::new(
        from,
        to,
        stop_line_id,
        SignalControlInput::Group(group_id.to_owned()),
    )
}

fn none_gate(from: &str, to: &str, stop_line_id: &str) -> MovementGate {
    MovementGate::new(from, to, stop_line_id, SignalControlInput::None)
}

fn canonical_registry(graph: &LaneGraph) -> SignalRegistry {
    SignalRegistry::try_new(
        graph,
        [stop_line("stop-entry", "entry")],
        [group("main")],
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
            group_gate("entry", "through", "stop-entry", "main"),
            none_gate("entry", "bypass", "stop-entry"),
        ],
    )
    .expect("canonical Signals must normalize")
}

fn profile_registry() -> (VehicleProfileRegistry, laneflow_core::VehicleProfileHandle) {
    let registry = VehicleProfileRegistry::try_new([VehicleProfile::try_new_iidm(
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
    .expect("valid registry");
    let handle = registry
        .profile_handle("passenger-car")
        .expect("profile handle");
    (registry, handle)
}

#[test]
fn canonical_registry_preserves_normalization_order_and_resolves_handles() {
    fn assert_traits<T: Clone + Copy + std::fmt::Debug + Eq + std::hash::Hash>() {}
    assert_traits::<StopLineHandle>();
    assert_traits::<SignalGroupHandle>();
    assert_traits::<SignalControllerHandle>();
    assert_traits::<SignalPhaseRef>();
    assert_traits::<MovementGateKey>();

    let graph = canonical_graph();
    let signals = canonical_registry(&graph);
    assert!(!signals.is_empty());

    let stop_line = signals
        .stop_line_handle("stop-entry")
        .expect("StopLine resolver");
    assert_eq!(signals.stop_line_external_id(stop_line), Some("stop-entry"));
    assert_eq!(
        signals.stop_line_edge(stop_line),
        graph.edge_handle("entry")
    );
    assert_eq!(
        signals.stop_line_for_edge(graph.edge_handle("entry").expect("entry edge")),
        Some(stop_line)
    );

    let group = signals.group_handle("main").expect("group resolver");
    let controller = signals
        .controller_handle("controller-main")
        .expect("controller resolver");
    assert_eq!(signals.group_external_id(group), Some("main"));
    assert_eq!(signals.group_controller(group), Some(controller));
    assert_eq!(
        signals.controller_groups(controller),
        Some([group].as_slice())
    );
    assert_eq!(
        signals.controller_cycle_duration_ms(controller),
        Some(53_000)
    );

    let yellow = signals
        .phase_ref(controller, "yellow")
        .expect("phase resolver");
    assert_eq!(signals.phase_external_id(yellow), Some("yellow"));
    assert_eq!(
        signals.phase_aspects(yellow),
        Some([SignalAspect::Yellow].as_slice())
    );
    assert_eq!(signals.phase_end_offset_ms(yellow), Some(33_000));

    let through_key = MovementGateKey::new(
        graph.edge_handle("entry").expect("entry edge"),
        graph.edge_handle("through").expect("through edge"),
    );
    assert_eq!(
        signals.movement_gate_control(through_key),
        Some(SignalControl::Group(group))
    );
    assert_eq!(
        signals.movement_gate_stop_line(through_key),
        Some(stop_line)
    );

    let bypass_key = MovementGateKey::new(
        graph.edge_handle("entry").expect("entry edge"),
        graph.edge_handle("bypass").expect("bypass edge"),
    );
    assert_eq!(
        signals.movement_gate_control(bypass_key),
        Some(SignalControl::None)
    );
    assert_eq!(signals.stop_lines().count(), 1);
    assert_eq!(signals.groups().count(), 1);
    assert_eq!(signals.controllers().count(), 1);
    assert_eq!(
        signals.movement_gates().collect::<Vec<_>>(),
        [through_key, bypass_key]
    );

    assert_eq!(signals.stop_line_handle("unknown"), None);
    assert_eq!(signals.group_handle("unknown"), None);
    assert_eq!(signals.controller_handle("unknown"), None);
    assert_eq!(signals.phase_ref(controller, "unknown"), None);

    let empty = SignalRegistry::empty();
    assert_eq!(empty.stop_line(stop_line), None);
    assert_eq!(empty.group(group), None);
    assert_eq!(empty.controller(controller), None);
    assert_eq!(empty.phase(yellow), None);
    assert_eq!(empty.phase_end_offset_ms(yellow), None);
    assert_eq!(empty.movement_gate_control(through_key), None);
}

#[test]
fn identity_and_reference_errors_follow_input_order() {
    let graph = canonical_graph();

    let error = SignalRegistry::try_new(
        &graph,
        [
            stop_line("duplicate", "entry"),
            stop_line("duplicate", "bad edge"),
        ],
        std::iter::empty::<SignalGroup>(),
        std::iter::empty::<SignalController>(),
        std::iter::empty::<MovementGate>(),
    )
    .expect_err("duplicate StopLine ID must fail first");
    std::assert_matches!(error, CoreError::DuplicateStopLineId { stop_line_id } if stop_line_id == "duplicate");

    let error = SignalRegistry::try_new(
        &graph,
        [stop_line("first", "entry"), stop_line("second", "entry")],
        std::iter::empty::<SignalGroup>(),
        std::iter::empty::<SignalController>(),
        std::iter::empty::<MovementGate>(),
    )
    .expect_err("one StopLine per edge");
    std::assert_matches!(
        error,
        CoreError::DuplicateStopLineEdge {
            first_stop_line_id,
            duplicate_stop_line_id,
            ..
        } if first_stop_line_id == "first" && duplicate_stop_line_id == "second"
    );

    let error = SignalRegistry::try_new(
        &graph,
        [stop_line("missing-edge", "unknown")],
        std::iter::empty::<SignalGroup>(),
        std::iter::empty::<SignalController>(),
        std::iter::empty::<MovementGate>(),
    )
    .expect_err("unknown StopLine edge");
    std::assert_matches!(error, CoreError::UnknownStopLineEdge { edge_id, .. } if edge_id == "unknown");

    let error = SignalRegistry::try_new(
        &graph,
        std::iter::empty::<StopLine>(),
        [group("same"), group("same")],
        std::iter::empty::<SignalController>(),
        std::iter::empty::<MovementGate>(),
    )
    .expect_err("duplicate group ID");
    std::assert_matches!(error, CoreError::DuplicateSignalGroupId { group_id } if group_id == "same");

    let error = SignalRegistry::try_new(
        &graph,
        std::iter::empty::<StopLine>(),
        [group("first"), group("second")],
        [
            controller(
                "same",
                0,
                &["first"],
                vec![phase("p", 10, vec![state("first", SignalAspect::Red)])],
            ),
            controller(
                "same",
                MAX_PORTABLE_SIGNAL_TIME_MS + 1,
                &["second"],
                vec![phase("p", 10, vec![state("second", SignalAspect::Red)])],
            ),
        ],
        std::iter::empty::<MovementGate>(),
    )
    .expect_err("duplicate controller ID");
    std::assert_matches!(error, CoreError::DuplicateSignalControllerId { controller_id } if controller_id == "same");

    let error = SignalRegistry::try_new(
        &graph,
        std::iter::empty::<StopLine>(),
        [group("main")],
        [controller(
            "controller",
            0,
            &["main"],
            vec![
                phase("same", 10, vec![state("main", SignalAspect::Red)]),
                phase("same", 0, vec![state("main", SignalAspect::Green)]),
            ],
        )],
        std::iter::empty::<MovementGate>(),
    )
    .expect_err("duplicate controller-local phase ID");
    std::assert_matches!(error, CoreError::DuplicateSignalPhaseId { phase_id, .. } if phase_id == "same");
}

#[test]
fn every_static_signal_external_id_uses_the_shared_token_rule() {
    let graph = canonical_graph();
    let cases = [
        SignalRegistry::try_new(
            &graph,
            [stop_line("bad id", "entry")],
            std::iter::empty::<SignalGroup>(),
            std::iter::empty::<SignalController>(),
            std::iter::empty::<MovementGate>(),
        ),
        SignalRegistry::try_new(
            &graph,
            std::iter::empty::<StopLine>(),
            [group("bad id")],
            std::iter::empty::<SignalController>(),
            std::iter::empty::<MovementGate>(),
        ),
        SignalRegistry::try_new(
            &graph,
            std::iter::empty::<StopLine>(),
            std::iter::empty::<SignalGroup>(),
            [controller("bad id", 0, &[], vec![])],
            std::iter::empty::<MovementGate>(),
        ),
        SignalRegistry::try_new(
            &graph,
            std::iter::empty::<StopLine>(),
            [group("main")],
            [controller(
                "controller",
                0,
                &["main"],
                vec![phase("bad id", 10, vec![state("main", SignalAspect::Red)])],
            )],
            std::iter::empty::<MovementGate>(),
        ),
    ];

    for error in cases.map(|result| result.expect_err("invalid external ID must fail")) {
        std::assert_matches!(error, CoreError::InvalidExternalId { external_id, .. } if external_id == "bad id");
    }
}

#[test]
fn controller_ownership_and_cardinality_are_strict() {
    let graph = canonical_graph();
    for (controller, expected) in [
        (
            controller("empty-groups", 0, &[], vec![phase("p", 10, vec![])]),
            "groups",
        ),
        (controller("empty-phases", 0, &["main"], vec![]), "phases"),
    ] {
        let error = SignalRegistry::try_new(
            &graph,
            std::iter::empty::<StopLine>(),
            [group("main")],
            [controller],
            std::iter::empty::<MovementGate>(),
        )
        .expect_err("empty controller component must fail");
        match expected {
            "groups" => std::assert_matches!(error, CoreError::EmptySignalControllerGroups { .. }),
            "phases" => std::assert_matches!(error, CoreError::EmptySignalControllerPhases { .. }),
            _ => unreachable!(),
        }
    }

    let error = SignalRegistry::try_new(
        &graph,
        std::iter::empty::<StopLine>(),
        [group("main")],
        [controller(
            "duplicate-membership",
            0,
            &["main", "main"],
            vec![phase("p", 10, vec![state("main", SignalAspect::Red)])],
        )],
        std::iter::empty::<MovementGate>(),
    )
    .expect_err("duplicate controller group");
    std::assert_matches!(error, CoreError::DuplicateSignalControllerGroup { group_id, .. } if group_id == "main");

    let error = SignalRegistry::try_new(
        &graph,
        std::iter::empty::<StopLine>(),
        [group("main")],
        [controller(
            "unknown-membership",
            0,
            &["unknown"],
            vec![phase("p", 10, vec![state("unknown", SignalAspect::Red)])],
        )],
        std::iter::empty::<MovementGate>(),
    )
    .expect_err("controller group must exist");
    std::assert_matches!(error, CoreError::UnknownSignalControllerGroup { group_id, .. } if group_id == "unknown");

    let shared_phase = || phase("p", 10, vec![state("main", SignalAspect::Red)]);
    let error = SignalRegistry::try_new(
        &graph,
        std::iter::empty::<StopLine>(),
        [group("main")],
        [
            controller("first", 0, &["main"], vec![shared_phase()]),
            controller("second", 0, &["main"], vec![shared_phase()]),
        ],
        std::iter::empty::<MovementGate>(),
    )
    .expect_err("group must have one owner");
    std::assert_matches!(
        error,
        CoreError::SignalGroupMultipleControllers {
            first_controller_id,
            duplicate_controller_id,
            ..
        } if first_controller_id == "first" && duplicate_controller_id == "second"
    );
}

#[test]
fn phase_state_validation_reports_unknown_duplicate_then_missing() {
    let graph = canonical_graph();
    let build = |states| {
        SignalRegistry::try_new(
            &graph,
            std::iter::empty::<StopLine>(),
            [group("first"), group("second")],
            [controller(
                "controller",
                0,
                &["first", "second"],
                vec![phase("phase", 100, states)],
            )],
            std::iter::empty::<MovementGate>(),
        )
    };

    let error =
        build(vec![state("unknown", SignalAspect::Red)]).expect_err("unknown state group first");
    std::assert_matches!(error, CoreError::UnknownSignalPhaseGroup { group_id, .. } if group_id == "unknown");

    let error = build(vec![
        state("first", SignalAspect::Red),
        state("first", SignalAspect::Green),
    ])
    .expect_err("duplicate state group before missing group");
    std::assert_matches!(error, CoreError::DuplicateSignalPhaseGroup { group_id, .. } if group_id == "first");

    let error = build(vec![state("second", SignalAspect::Red)])
        .expect_err("missing follows controller group order");
    std::assert_matches!(error, CoreError::MissingSignalPhaseGroup { group_id, .. } if group_id == "first");

    let signals = SignalRegistry::try_new(
        &graph,
        [stop_line("stop", "entry")],
        [group("first"), group("second")],
        [controller(
            "controller",
            0,
            &["first", "second"],
            vec![phase(
                "phase",
                100,
                vec![
                    state("second", SignalAspect::Red),
                    state("first", SignalAspect::Green),
                ],
            )],
        )],
        [
            group_gate("entry", "through", "stop", "first"),
            group_gate("entry", "bypass", "stop", "second"),
        ],
    )
    .expect("state record order must not change normalized aspect vector");
    let controller = signals.controller_handle("controller").expect("controller");
    let phase = signals.phase_ref(controller, "phase").expect("phase");
    assert_eq!(
        signals.phase_aspects(phase),
        Some([SignalAspect::Green, SignalAspect::Red].as_slice())
    );
}

#[test]
fn portable_integer_cycle_and_offset_rules_are_enforced() {
    let graph = canonical_graph();
    for duration_ms in [0, MAX_PORTABLE_SIGNAL_TIME_MS + 1] {
        let error = SignalRegistry::try_new(
            &graph,
            std::iter::empty::<StopLine>(),
            [group("main")],
            [controller(
                "invalid-duration",
                0,
                &["main"],
                vec![phase(
                    "invalid",
                    duration_ms,
                    vec![state("main", SignalAspect::Red)],
                )],
            )],
            std::iter::empty::<MovementGate>(),
        )
        .expect_err("duration outside portable range");
        std::assert_matches!(error, CoreError::InvalidSignalPhaseDuration { duration_ms: actual, .. } if actual == duration_ms);
    }
    let error = SignalRegistry::try_new(
        &graph,
        std::iter::empty::<StopLine>(),
        [group("main")],
        [controller(
            "invalid-offset",
            MAX_PORTABLE_SIGNAL_TIME_MS + 1,
            &["main"],
            vec![phase("p", 10, vec![state("main", SignalAspect::Red)])],
        )],
        std::iter::empty::<MovementGate>(),
    )
    .expect_err("offset outside portable range");
    std::assert_matches!(error, CoreError::InvalidSignalControllerOffset { .. });

    let error = SignalRegistry::try_new(
        &graph,
        std::iter::empty::<StopLine>(),
        [group("main")],
        [controller(
            "cycle-overflow",
            0,
            &["main"],
            vec![
                phase(
                    "first",
                    MAX_PORTABLE_SIGNAL_TIME_MS,
                    vec![state("main", SignalAspect::Red)],
                ),
                phase("second", 1, vec![state("main", SignalAspect::Green)]),
            ],
        )],
        std::iter::empty::<MovementGate>(),
    )
    .expect_err("cycle checked sum must be portable");
    std::assert_matches!(error, CoreError::SignalCycleDurationOverflow { .. });

    let error = SignalRegistry::try_new(
        &graph,
        std::iter::empty::<StopLine>(),
        [group("main")],
        [controller(
            "offset-equals-cycle",
            100,
            &["main"],
            vec![phase("only", 100, vec![state("main", SignalAspect::Red)])],
        )],
        std::iter::empty::<MovementGate>(),
    )
    .expect_err("offset must be canonical");
    std::assert_matches!(
        error,
        CoreError::SignalControllerOffsetOutOfRange {
            offset_ms: 100,
            cycle_duration_ms: 100,
            ..
        }
    );

    for (duration_ms, offset_ms) in [
        (1, 0),
        (MAX_PORTABLE_SIGNAL_TIME_MS, MAX_PORTABLE_SIGNAL_TIME_MS - 1),
    ] {
        SignalRegistry::try_new(
            &graph,
            [stop_line("stop", "entry")],
            [group("main")],
            [controller(
                "boundary",
                offset_ms,
                &["main"],
                vec![phase(
                    "only",
                    duration_ms,
                    vec![state("main", SignalAspect::Green)],
                )],
            )],
            [
                group_gate("entry", "through", "stop", "main"),
                none_gate("entry", "bypass", "stop"),
            ],
        )
        .expect("portable min/max boundaries must normalize");
    }
}

#[test]
fn gate_coverage_stop_line_ownership_and_group_usage_are_global_invariants() {
    let graph = canonical_graph();
    let base_controller = || {
        controller(
            "controller",
            0,
            &["main"],
            vec![phase("p", 100, vec![state("main", SignalAspect::Green)])],
        )
    };

    let error = SignalRegistry::try_new(
        &graph,
        [stop_line("stop", "entry")],
        [group("main")],
        [base_controller()],
        [group_gate("entry", "through", "stop", "main")],
    )
    .expect_err("all outgoing connections require Gate coverage");
    std::assert_matches!(error, CoreError::MissingMovementGateCoverage { to_edge_id, .. } if to_edge_id == "bypass");

    let terminal_graph = LaneGraph::try_new([LaneEdge::new(
        "terminal",
        edge_length(10.0),
        std::iter::empty::<&str>(),
    )])
    .expect("terminal graph");
    let error = SignalRegistry::try_new(
        &terminal_graph,
        [stop_line("orphan", "terminal")],
        std::iter::empty::<SignalGroup>(),
        std::iter::empty::<SignalController>(),
        std::iter::empty::<MovementGate>(),
    )
    .expect_err("terminal StopLine is orphan");
    std::assert_matches!(error, CoreError::OrphanStopLine { stop_line_id, .. } if stop_line_id == "orphan");

    let error = SignalRegistry::try_new(
        &graph,
        std::iter::empty::<StopLine>(),
        [group("unowned")],
        std::iter::empty::<SignalController>(),
        std::iter::empty::<MovementGate>(),
    )
    .expect_err("group must have owner");
    std::assert_matches!(error, CoreError::UnownedSignalGroup { group_id } if group_id == "unowned");

    let error = SignalRegistry::try_new(
        &graph,
        [stop_line("stop", "entry")],
        [group("main"), group("unused")],
        [controller(
            "controller",
            0,
            &["main", "unused"],
            vec![phase(
                "p",
                100,
                vec![
                    state("main", SignalAspect::Green),
                    state("unused", SignalAspect::Red),
                ],
            )],
        )],
        [
            group_gate("entry", "through", "stop", "main"),
            none_gate("entry", "bypass", "stop"),
        ],
    )
    .expect_err("every group needs Gate usage");
    std::assert_matches!(error, CoreError::UnusedSignalGroup { group_id } if group_id == "unused");
}

#[test]
fn gate_references_connection_and_stop_line_must_agree() {
    let graph = canonical_graph();
    let controller = || {
        controller(
            "controller",
            0,
            &["main"],
            vec![phase("p", 100, vec![state("main", SignalAspect::Red)])],
        )
    };

    let error = SignalRegistry::try_new(
        &graph,
        [stop_line("stop", "entry")],
        [group("main")],
        [controller()],
        [group_gate("through", "entry", "stop", "main")],
    )
    .expect_err("Gate pair must be a connection");
    std::assert_matches!(error, CoreError::DisconnectedMovementGate { .. });

    let error = SignalRegistry::try_new(
        &graph,
        [stop_line("stop", "entry")],
        [group("main")],
        [controller()],
        [group_gate("unknown", "through", "stop", "main")],
    )
    .expect_err("Gate from edge must exist");
    std::assert_matches!(error, CoreError::UnknownMovementGateFromEdge { edge_id } if edge_id == "unknown");

    let error = SignalRegistry::try_new(
        &graph,
        [stop_line("stop", "entry")],
        [group("main")],
        [controller()],
        [group_gate("entry", "through", "unknown", "main")],
    )
    .expect_err("Gate StopLine must exist");
    std::assert_matches!(error, CoreError::UnknownMovementGateStopLine { stop_line_id } if stop_line_id == "unknown");

    let error = SignalRegistry::try_new(
        &graph,
        [stop_line("stop", "entry")],
        [group("main")],
        [controller()],
        [group_gate("entry", "through", "stop", "unknown")],
    )
    .expect_err("Gate SignalGroup must exist");
    std::assert_matches!(error, CoreError::UnknownMovementGateSignalGroup { group_id } if group_id == "unknown");

    let graph = LaneGraph::try_new([
        LaneEdge::new("first", edge_length(10.0), ["target"]),
        LaneEdge::new("second", edge_length(10.0), ["target"]),
        LaneEdge::new("target", edge_length(10.0), std::iter::empty::<&str>()),
    ])
    .expect("valid graph");
    let error = SignalRegistry::try_new(
        &graph,
        [stop_line("stop", "first")],
        [group("main")],
        [controller()],
        [group_gate("second", "target", "stop", "main")],
    )
    .expect_err("Gate StopLine belongs to from edge");
    std::assert_matches!(error, CoreError::MovementGateStopLineMismatch { from_edge_id, .. } if from_edge_id == "second");

    let graph = canonical_graph();
    let error = SignalRegistry::try_new(
        &graph,
        [stop_line("stop", "entry")],
        [group("main")],
        [controller()],
        [
            group_gate("entry", "through", "stop", "main"),
            group_gate("entry", "through", "bad stop line", "main"),
        ],
    )
    .expect_err("Gate pair identity must be unique");
    std::assert_matches!(error, CoreError::DuplicateMovementGate { from_edge_id, to_edge_id }
        if from_edge_id == "entry" && to_edge_id == "through");
}

#[test]
fn routes_cannot_terminate_at_stop_line_for_initial_or_runtime_registration() {
    let graph = canonical_graph();
    let signals = canonical_registry(&graph);
    let error = InitialTrafficData::try_new_with_signals(
        graph.clone(),
        [Route::try_new("invalid", ["entry"]).expect("route shape")],
        VehicleProfileRegistry::empty(),
        signals.clone(),
    )
    .expect_err("initial route cannot terminate at StopLine");
    std::assert_matches!(error, CoreError::RouteTerminatesAtStopLine { route_id, .. } if route_id == "invalid");

    let traffic = InitialTrafficData::try_new_with_signals(
        graph,
        [Route::try_new("valid", ["entry", "through"]).expect("route shape")],
        VehicleProfileRegistry::empty(),
        signals,
    )
    .expect("valid traffic data");
    let mut world = CoreWorld::with_traffic_data(16, traffic, Vec::new()).expect("valid world");
    let before = world.clone();
    let error = world
        .register_route(Route::try_new("invalid-runtime", ["entry"]).expect("route shape"))
        .expect_err("runtime registration reuses StopLine rule");
    std::assert_matches!(error, CoreError::RouteTerminatesAtStopLine { route_id, .. } if route_id == "invalid-runtime");
    assert_eq!(world, before, "failed route registration must be atomic");
}

#[test]
fn initial_traffic_data_rebinds_signals_to_its_own_lane_graph() {
    let source_graph = canonical_graph();
    let signals = canonical_registry(&source_graph);
    let reordered_graph = LaneGraph::try_new([
        LaneEdge::new("bypass", edge_length(30.0), std::iter::empty::<&str>()),
        LaneEdge::new("entry", edge_length(100.0), ["through", "bypass"]),
        LaneEdge::new("through", edge_length(40.0), std::iter::empty::<&str>()),
    ])
    .expect("same topology with different handle order");

    let traffic = InitialTrafficData::try_new_with_signals(
        reordered_graph,
        [Route::try_new("valid", ["entry", "through"]).expect("route shape")],
        VehicleProfileRegistry::empty(),
        signals,
    )
    .expect("Signals must be atomically rebound to the traffic graph");
    let stop_line = traffic
        .signals()
        .stop_line_handle("stop-entry")
        .expect("StopLine resolver");
    assert_eq!(
        traffic.signals().stop_line_edge(stop_line),
        traffic.lane_graph().edge_handle("entry")
    );

    let incompatible_graph = LaneGraph::try_new([LaneEdge::new(
        "other",
        edge_length(10.0),
        std::iter::empty::<&str>(),
    )])
    .expect("valid incompatible graph");
    let error = InitialTrafficData::try_new_with_signals(
        incompatible_graph,
        std::iter::empty::<Route>(),
        VehicleProfileRegistry::empty(),
        canonical_registry(&source_graph),
    )
    .expect_err("mismatched graph-dependent registry must fail atomically");
    std::assert_matches!(error, CoreError::UnknownStopLineEdge { edge_id, .. } if edge_id == "entry");
}

#[test]
fn world_validates_phase_delta_and_allows_vehicle_activation_after_compliance() {
    let graph = canonical_graph();
    let short_signals = SignalRegistry::try_new(
        &graph,
        [stop_line("stop", "entry")],
        [group("main")],
        [controller(
            "controller",
            0,
            &["main"],
            vec![phase("short", 15, vec![state("main", SignalAspect::Red)])],
        )],
        [
            group_gate("entry", "through", "stop", "main"),
            none_gate("entry", "bypass", "stop"),
        ],
    )
    .expect("static registry is valid without world delta");
    let traffic = InitialTrafficData::try_new_with_signals(
        graph.clone(),
        [Route::try_new("route", ["entry", "through"]).expect("route shape")],
        VehicleProfileRegistry::empty(),
        short_signals,
    )
    .expect("static traffic data");
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
    let signals = canonical_registry(&graph);
    let route = Route::try_new("route", ["entry", "through"]).expect("route shape");
    let vehicle = VehicleSpawnInput::new(
        "vehicle",
        profile,
        "route",
        0,
        EdgeProgress::try_new(0.0).expect("progress"),
        Speed::try_new(0.0).expect("speed"),
        VehicleStatus::Active,
    );
    let traffic = InitialTrafficData::try_new_with_signals(
        graph.clone(),
        [route.clone()],
        profiles.clone(),
        signals.clone(),
    )
    .expect("valid traffic data");
    let world = CoreWorld::with_traffic_data(16, traffic, vec![vehicle.clone()])
        .expect("initial vehicle activation is supported after #96 compliance");
    assert_eq!(world.vehicles().count(), 1);

    let traffic = InitialTrafficData::try_new_with_signals(graph, [route], profiles, signals)
        .expect("valid traffic data");
    let mut world =
        CoreWorld::with_traffic_data(16, traffic, Vec::new()).expect("signal-only world");
    let handle = world
        .spawn_vehicle(vehicle)
        .expect("runtime spawn is supported after #96 compliance");
    assert!(world.vehicle(handle).is_some());
}
