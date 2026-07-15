use laneflow_core::{
    CoreError, CoreEvent, CoreWorld, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData,
    LaneEdge, LaneGraph, MovementGate, Route, SignalAspect, SignalControlInput, SignalController,
    SignalGroup, SignalGroupState, SignalPhase, SignalRegistry, Speed, StopLine, StopLineLocation,
    TickInput, VehicleProfile, VehicleProfileRegistry, VehicleSpawnInput,
};

const DELTA_MS: u64 = 16;

fn phase(id: &str, first: SignalAspect, second: SignalAspect) -> SignalPhase {
    SignalPhase::new(
        id,
        32,
        [
            SignalGroupState::new("group-a", first),
            SignalGroupState::new("group-c", second),
        ],
    )
}

fn replay_world() -> CoreWorld {
    let graph = LaneGraph::try_new([
        LaneEdge::new("a", EdgeLength::try_new(10.0).unwrap(), ["b"]),
        LaneEdge::new(
            "b",
            EdgeLength::try_new(20.0).unwrap(),
            Vec::<String>::new(),
        ),
        LaneEdge::new("c", EdgeLength::try_new(10.0).unwrap(), ["d"]),
        LaneEdge::new(
            "d",
            EdgeLength::try_new(20.0).unwrap(),
            Vec::<String>::new(),
        ),
    ])
    .expect("replay graph");
    let signals = SignalRegistry::try_new(
        &graph,
        [
            StopLine::new("stop-a", "a", StopLineLocation::EdgeEnd),
            StopLine::new("stop-c", "c", StopLineLocation::EdgeEnd),
        ],
        [SignalGroup::new("group-a"), SignalGroup::new("group-c")],
        [SignalController::new_fixed_time(
            "controller",
            0,
            ["group-a", "group-c"],
            [
                phase("first", SignalAspect::Green, SignalAspect::Red),
                phase("second", SignalAspect::Red, SignalAspect::Green),
            ],
        )],
        [
            MovementGate::new(
                "a",
                "b",
                "stop-a",
                SignalControlInput::Group("group-a".to_owned()),
            ),
            MovementGate::new(
                "c",
                "d",
                "stop-c",
                SignalControlInput::Group("group-c".to_owned()),
            ),
        ],
    )
    .expect("replay signals");
    let profiles = VehicleProfileRegistry::try_new([VehicleProfile::try_new_iidm(
        "car",
        IidmProfileSpec {
            length: 4.5,
            desired_speed: 10.0,
            min_gap: 2.0,
            time_headway: 1.5,
            max_acceleration: 1.5,
            comfortable_deceleration: 2.0,
            emergency_deceleration: 6.0,
        },
    )
    .unwrap()])
    .unwrap();
    let profile = profiles.profile_handle("car").unwrap();
    let traffic = InitialTrafficData::try_new_with_signals(
        graph,
        [
            Route::try_new("route-a", ["a", "b"]).unwrap(),
            Route::try_new("route-c", ["c", "d"]).unwrap(),
        ],
        profiles,
        signals,
    )
    .unwrap();
    CoreWorld::with_traffic_data(
        DELTA_MS,
        traffic,
        vec![
            VehicleSpawnInput::active(
                "vehicle-a",
                profile,
                "route-a",
                0,
                EdgeProgress::try_new(9.9).unwrap(),
                Speed::try_new(10.0).unwrap(),
            ),
            VehicleSpawnInput::active(
                "vehicle-c",
                profile,
                "route-c",
                0,
                EdgeProgress::try_new(9.9).unwrap(),
                Speed::try_new(10.0).unwrap(),
            ),
        ],
    )
    .expect("replay world")
}

#[test]
fn identical_signal_runs_replay_exact_snapshots_and_event_order() {
    let mut left = replay_world();
    let mut right = replay_world();

    for _ in 0..80 {
        let left_result = left.step(TickInput::new(DELTA_MS)).expect("left step");
        let right_result = right.step(TickInput::new(DELTA_MS)).expect("right step");
        assert_eq!(left_result, right_result);
        assert_eq!(left, right);
    }
}

#[test]
fn failed_step_is_atomic_and_retry_matches_fresh_world_at_phase_boundary() {
    let mut retried = replay_world();
    let mut fresh = replay_world();
    assert_eq!(
        retried.step(TickInput::new(DELTA_MS)).unwrap(),
        fresh.step(TickInput::new(DELTA_MS)).unwrap()
    );
    let before_failure = retried.clone();

    let error = retried
        .step(TickInput::new(DELTA_MS - 1))
        .expect_err("wrong delta must fail");
    std::assert_matches!(
        error,
        CoreError::TickDeltaMismatch {
            expected_delta_time_ms: DELTA_MS,
            actual_delta_time_ms: 15,
        }
    );
    assert_eq!(retried, before_failure);

    let retry_result = retried.step(TickInput::new(DELTA_MS)).unwrap();
    let fresh_result = fresh.step(TickInput::new(DELTA_MS)).unwrap();
    assert_eq!(retry_result, fresh_result);
    assert_eq!(retried, fresh);

    let phase_position = retry_result
        .events
        .iter()
        .position(|event| matches!(event, CoreEvent::SignalPhaseChanged(_)))
        .expect("phase boundary event");
    let group_positions = retry_result
        .events
        .iter()
        .enumerate()
        .filter_map(|(index, event)| {
            matches!(event, CoreEvent::SignalGroupAspectChanged(_)).then_some(index)
        })
        .collect::<Vec<_>>();
    assert_eq!(group_positions.len(), 2);
    assert!(group_positions.iter().all(|index| *index > phase_position));
}
