use laneflow_core::{
    CoreError, CoreEvent, CoreWorld, EdgeLength, InitialTrafficData, LaneEdge, LaneGraph,
    MovementGate, MovementGateKey, MovementGateSignalState, Route, SignalAspect,
    SignalControlInput, SignalController, SignalGroup, SignalGroupState, SignalLayerPermission,
    SignalPhase, SignalRegistry, StopLine, StopLineLocation, TickInput, VehicleProfileRegistry,
};

fn phase(id: &str, duration_ms: u64, states: &[(&str, SignalAspect)]) -> SignalPhase {
    SignalPhase::new(
        id,
        duration_ms,
        states
            .iter()
            .map(|(group, aspect)| SignalGroupState::new(*group, *aspect)),
    )
}

fn signal_world(fixed_delta_time_ms: u64, offset_ms: u64, phases: Vec<SignalPhase>) -> CoreWorld {
    let graph = LaneGraph::try_new([
        LaneEdge::new(
            "entry",
            EdgeLength::try_new(100.0).expect("valid edge"),
            ["exit", "bypass"],
        ),
        LaneEdge::new(
            "exit",
            EdgeLength::try_new(100.0).expect("valid edge"),
            Vec::<String>::new(),
        ),
        LaneEdge::new(
            "bypass",
            EdgeLength::try_new(100.0).expect("valid edge"),
            Vec::<String>::new(),
        ),
    ])
    .expect("valid graph");
    let signals = SignalRegistry::try_new(
        &graph,
        [StopLine::new("stop", "entry", StopLineLocation::EdgeEnd)],
        [SignalGroup::new("group")],
        [SignalController::new_fixed_time(
            "controller",
            offset_ms,
            ["group"],
            phases,
        )],
        [
            MovementGate::new(
                "entry",
                "exit",
                "stop",
                SignalControlInput::Group("group".to_owned()),
            ),
            MovementGate::new("entry", "bypass", "stop", SignalControlInput::None),
        ],
    )
    .expect("valid signals");
    let traffic = InitialTrafficData::try_new_with_signals(
        graph,
        [Route::try_new("route", ["entry", "exit"]).expect("valid route")],
        VehicleProfileRegistry::empty(),
        signals,
    )
    .expect("valid traffic");
    CoreWorld::with_traffic_data(fixed_delta_time_ms, traffic, Vec::new())
        .expect("valid signal-only world")
}

#[test]
fn time_zero_queries_and_post_step_events_follow_absolute_offset_timing() {
    let mut world = signal_world(
        10,
        5,
        vec![
            phase("red", 20, &[("group", SignalAspect::Red)]),
            phase("green", 30, &[("group", SignalAspect::Green)]),
        ],
    );
    let controller = world
        .signals()
        .controller_handle("controller")
        .expect("controller");
    let group = world.signals().group_handle("group").expect("group");
    let red = world
        .signals()
        .phase_ref(controller, "red")
        .expect("red phase");
    let green = world
        .signals()
        .phase_ref(controller, "green")
        .expect("green phase");
    let entry = world.edge_handle("entry").expect("entry");
    let exit = world.edge_handle("exit").expect("exit");
    let gate = MovementGateKey::new(entry, exit);
    let bypass = world.edge_handle("bypass").expect("bypass");

    let time_zero = world
        .signal_controller_state(controller)
        .expect("time-zero state");
    assert_eq!(time_zero.current_phase(), red);
    assert_eq!(time_zero.cycle_position_ms(), 5);
    assert_eq!(time_zero.phase_elapsed_ms(), 5);
    assert_eq!(time_zero.phase_remaining_ms(), 15);
    assert_eq!(
        world
            .signal_group_state(group)
            .expect("time-zero group")
            .aspect(),
        SignalAspect::Red
    );
    std::assert_matches!(
        world.movement_gate_state(gate).expect("gate").signal(),
        MovementGateSignalState::Controlled {
            group: actual_group,
            aspect: SignalAspect::Red,
            permission: SignalLayerPermission::DenyAndStop,
            ..
        } if actual_group == group
    );
    std::assert_matches!(
        world
            .movement_gate_state(MovementGateKey::new(entry, bypass))
            .expect("uncontrolled gate")
            .signal(),
        MovementGateSignalState::Uncontrolled
    );
    assert_eq!(world.signal_controller_states().count(), 1);
    assert_eq!(world.signal_group_states().count(), 1);
    assert_eq!(world.movement_gate_states().count(), 2);

    let before_boundary = world.step(TickInput::new(10)).expect("step succeeds");
    assert!(before_boundary.events.is_empty());
    assert_eq!(
        world
            .signal_controller_state(controller)
            .expect("pre-boundary state")
            .cycle_position_ms(),
        15
    );

    let crossed_boundary = world.step(TickInput::new(10)).expect("step succeeds");
    assert_eq!(crossed_boundary.tick_index, 2);
    assert_eq!(crossed_boundary.time_ms, 20);
    assert_eq!(crossed_boundary.events.len(), 2);
    std::assert_matches!(
        crossed_boundary.events[0],
        CoreEvent::SignalPhaseChanged(ref event)
            if event.tick_index == 2
                && event.controller == controller
                && event.from_phase == red
                && event.to_phase == green
    );
    std::assert_matches!(
        crossed_boundary.events[1],
        CoreEvent::SignalGroupAspectChanged(ref event)
            if event.tick_index == 2
                && event.group == group
                && event.from_aspect == SignalAspect::Red
                && event.to_aspect == SignalAspect::Green
    );

    let post_step = world
        .signal_controller_state(controller)
        .expect("post-step state");
    assert_eq!(post_step.current_phase(), green);
    assert_eq!(post_step.cycle_position_ms(), 25);
    assert_eq!(post_step.phase_elapsed_ms(), 5);
    assert_eq!(post_step.phase_remaining_ms(), 25);
    std::assert_matches!(
        world.movement_gate_state(gate).expect("gate").signal(),
        MovementGateSignalState::Controlled {
            aspect: SignalAspect::Green,
            permission: SignalLayerPermission::ProtectedAllow,
            ..
        }
    );
}

#[test]
fn non_divisible_delta_observes_boundary_without_timer_drift() {
    let mut world = signal_world(
        10,
        0,
        vec![
            phase("red", 25, &[("group", SignalAspect::Red)]),
            phase("green", 25, &[("group", SignalAspect::Green)]),
        ],
    );
    let controller = world
        .signals()
        .controller_handle("controller")
        .expect("controller");
    let green = world
        .signals()
        .phase_ref(controller, "green")
        .expect("green");

    assert!(
        world
            .step(TickInput::new(10))
            .expect("step 1")
            .events
            .is_empty()
    );
    assert!(
        world
            .step(TickInput::new(10))
            .expect("step 2")
            .events
            .is_empty()
    );
    let result = world.step(TickInput::new(10)).expect("step 3");

    assert_eq!(result.time_ms, 30);
    assert_eq!(result.events.len(), 2);
    let state = world
        .signal_controller_state(controller)
        .expect("controller state");
    assert_eq!(state.current_phase(), green);
    assert_eq!(state.cycle_position_ms(), 30);
    assert_eq!(state.phase_elapsed_ms(), 5);
    assert_eq!(state.phase_remaining_ms(), 20);
}

#[test]
fn phase_identity_events_do_not_require_aspect_change_and_single_phase_wrap_is_silent() {
    let mut same_aspect_world = signal_world(
        10,
        0,
        vec![
            phase("first", 10, &[("group", SignalAspect::Red)]),
            phase("second", 10, &[("group", SignalAspect::Red)]),
        ],
    );
    let first_result = same_aspect_world
        .step(TickInput::new(10))
        .expect("phase boundary");
    assert_eq!(first_result.events.len(), 1);
    std::assert_matches!(first_result.events[0], CoreEvent::SignalPhaseChanged(_));

    let mut single_phase_world = signal_world(
        10,
        0,
        vec![phase("only", 10, &[("group", SignalAspect::Red)])],
    );
    let wrap = single_phase_world
        .step(TickInput::new(10))
        .expect("single phase wrap");
    assert!(wrap.events.is_empty());
}

#[test]
fn controller_then_group_event_order_uses_normalization_and_group_input_order() {
    let graph = LaneGraph::try_new([
        LaneEdge::new("a", EdgeLength::try_new(10.0).expect("valid edge"), ["b"]),
        LaneEdge::new(
            "b",
            EdgeLength::try_new(10.0).expect("valid edge"),
            Vec::<String>::new(),
        ),
        LaneEdge::new("c", EdgeLength::try_new(10.0).expect("valid edge"), ["d"]),
        LaneEdge::new(
            "d",
            EdgeLength::try_new(10.0).expect("valid edge"),
            Vec::<String>::new(),
        ),
        LaneEdge::new("e", EdgeLength::try_new(10.0).expect("valid edge"), ["f"]),
        LaneEdge::new(
            "f",
            EdgeLength::try_new(10.0).expect("valid edge"),
            Vec::<String>::new(),
        ),
    ])
    .expect("graph");
    let signals = SignalRegistry::try_new(
        &graph,
        [
            StopLine::new("sa", "a", StopLineLocation::EdgeEnd),
            StopLine::new("sc", "c", StopLineLocation::EdgeEnd),
            StopLine::new("se", "e", StopLineLocation::EdgeEnd),
        ],
        [
            SignalGroup::new("g1"),
            SignalGroup::new("g2"),
            SignalGroup::new("g3"),
        ],
        [
            SignalController::new_fixed_time(
                "c1",
                0,
                ["g1", "g2"],
                [
                    phase(
                        "c1-old",
                        10,
                        &[("g1", SignalAspect::Red), ("g2", SignalAspect::Yellow)],
                    ),
                    phase(
                        "c1-new",
                        10,
                        &[("g1", SignalAspect::Green), ("g2", SignalAspect::Red)],
                    ),
                ],
            ),
            SignalController::new_fixed_time(
                "c2",
                0,
                ["g3"],
                [
                    phase("c2-old", 10, &[("g3", SignalAspect::Red)]),
                    phase("c2-new", 10, &[("g3", SignalAspect::Green)]),
                ],
            ),
        ],
        [
            MovementGate::new("a", "b", "sa", SignalControlInput::Group("g1".to_owned())),
            MovementGate::new("c", "d", "sc", SignalControlInput::Group("g2".to_owned())),
            MovementGate::new("e", "f", "se", SignalControlInput::Group("g3".to_owned())),
        ],
    )
    .expect("signals");
    let traffic = InitialTrafficData::try_new_with_signals(
        graph,
        Vec::<Route>::new(),
        VehicleProfileRegistry::empty(),
        signals,
    )
    .expect("traffic");
    let mut world = CoreWorld::with_traffic_data(10, traffic, Vec::new()).expect("world");
    let c1 = world
        .signals()
        .controller_handle("c1")
        .expect("controller c1");
    let c2 = world
        .signals()
        .controller_handle("c2")
        .expect("controller c2");
    let g1 = world.signals().group_handle("g1").expect("group g1");
    let g2 = world.signals().group_handle("g2").expect("group g2");
    let g3 = world.signals().group_handle("g3").expect("group g3");

    let result = world.step(TickInput::new(10)).expect("step");
    assert_eq!(result.events.len(), 5);
    std::assert_matches!(result.events[0], CoreEvent::SignalPhaseChanged(ref event) if event.controller == c1);
    std::assert_matches!(result.events[1], CoreEvent::SignalGroupAspectChanged(ref event) if event.group == g1);
    std::assert_matches!(result.events[2], CoreEvent::SignalGroupAspectChanged(ref event) if event.group == g2);
    std::assert_matches!(result.events[3], CoreEvent::SignalPhaseChanged(ref event) if event.controller == c2);
    std::assert_matches!(result.events[4], CoreEvent::SignalGroupAspectChanged(ref event) if event.group == g3);
}

#[test]
fn failed_step_keeps_signal_snapshot_and_events_atomic() {
    let mut world = signal_world(
        10,
        0,
        vec![
            phase("red", 10, &[("group", SignalAspect::Red)]),
            phase("green", 10, &[("group", SignalAspect::Green)]),
        ],
    );
    let controller = world
        .signals()
        .controller_handle("controller")
        .expect("controller");
    let before = world.clone();
    let before_state = world
        .signal_controller_state(controller)
        .expect("controller state");

    let error = world
        .step(TickInput::new(9))
        .expect_err("delta mismatch must fail");

    std::assert_matches!(
        error,
        CoreError::TickDeltaMismatch {
            expected_delta_time_ms: 10,
            actual_delta_time_ms: 9
        }
    );
    assert_eq!(world, before);
    assert_eq!(
        world.signal_controller_state(controller),
        Some(before_state)
    );
}
