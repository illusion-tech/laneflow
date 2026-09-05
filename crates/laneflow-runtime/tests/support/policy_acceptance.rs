use super::*;

use laneflow_compiler::GateInterpretation;
use laneflow_runtime::{ConflictDecisionOutcome, ConflictNoGrantReason, TrafficTransitionKind};

fn boundary_signal_module(
    interpretation: GateInterpretation,
    stop_aspect: SignalAspect,
    speed: f64,
) -> lfre::RoadEditingSourceModule {
    conflict_road_editing_module_with_shape_and_speed(
        2,
        false,
        true,
        false,
        speed,
        ConflictPolicyFixture {
            right_turn_signal: Some(interpretation),
            signal_cycle_ms: Some([100, 300]),
            signal_stop_aspect: Some(stop_aspect),
            ..Default::default()
        },
    )
}

fn blocked_at_green(world: &mut TrafficWorld) -> VehicleHandle {
    world.step(TickInput::new(100)).unwrap();
    let [route, _] = right_turn_routes(world);
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            1,
            10_501,
            0,
        ))
        .unwrap();
    at_gate(world, route)
}

fn eligibility_clocks(world: &TrafficWorld) -> Vec<u64> {
    let bytes = encode_lfrs(&world.capture_snapshot().unwrap());
    let wire = snapshot_wire::size_prefixed_root_as_runtime_snapshot(&bytes).unwrap();
    wire.vehicles()
        .iter()
        .filter_map(|vehicle| {
            vehicle
                .conflict_eligibility()
                .map(|value| value.first_eligible_tick())
        })
        .collect()
}

fn assert_snapshot_roundtrip(world: &TrafficWorld) {
    let snapshot = world.capture_snapshot().unwrap();
    let restored = restore_lfrs(
        &encode_lfrs(&snapshot),
        world.revision(),
        world.committed_source().clone(),
        world.config(),
        SnapshotRestoreLimits::new(1_048_576, 1_024),
    )
    .unwrap()
    .into_world();
    assert_eq!(
        deterministic_state_digest(&snapshot).unwrap(),
        deterministic_state_digest(&restored.capture_snapshot().unwrap()).unwrap()
    );
    assert_eq!(
        world.committed_signal_groups().as_slice(),
        restored.committed_signal_groups().as_slice()
    );
}

#[test]
fn signal_boundary_publishes_valid_eligibility_and_preserves_continuous_waiting() {
    for interpretation in [
        GateInterpretation::ProtectedGroup,
        GateInterpretation::PermissiveGroup,
    ] {
        for stop_aspect in [SignalAspect::Red, SignalAspect::Yellow] {
            let revision = compile_road_editing_revision(boundary_signal_module(
                interpretation,
                stop_aspect,
                13.0,
            ));
            let mut world =
                install_fixture(revision, WorldConfig::new(4, 4, 64, 4, 1, 100)).unwrap();
            let subject = blocked_at_green(&mut world);
            for expected_time in [200, 300, 400, 500, 600] {
                world.step(TickInput::new(100)).unwrap();
                assert_eq!(world.time_ms(), expected_time);
                assert_eq!(
                    calibration_outcome(&world, subject),
                    if expected_time == 500 {
                        ConflictDecisionOutcome::NotEvaluated
                    } else {
                        ConflictDecisionOutcome::NoGrant(
                            ConflictNoGrantReason::DownstreamStorageBoundary,
                        )
                    }
                );
                let expected = match expected_time {
                    200 | 300 => vec![2],
                    400 | 500 => vec![],
                    600 => vec![6],
                    _ => unreachable!(),
                };
                assert_eq!(
                    eligibility_clocks(&world),
                    expected,
                    "{interpretation:?} {stop_aspect:?}"
                );
                assert_snapshot_roundtrip(&world);
            }
        }
    }
}

#[test]
fn signal_boundary_journal_replays_normalized_eligibility_during_cutover() {
    let module = |speed| {
        boundary_signal_module(GateInterpretation::ProtectedGroup, SignalAspect::Red, speed)
    };
    let (base, target, diff, binding) = compile_conflict_cutover_pair(module(13.0), module(13.1));
    let mut world =
        install_fixture(Arc::clone(&base), WorldConfig::new(4, 4, 64, 4, 1, 100)).unwrap();
    blocked_at_green(&mut world);
    world.step(TickInput::new(100)).unwrap();
    assert_eq!(eligibility_clocks(&world), vec![2]);
    let descriptor = NetworkRevisionCutoverDescriptor::new(
        LfcaOriginBinding::from_canonical_origin(*base.canonical_origin()),
        LfcaOriginBinding::from_canonical_origin(*target.canonical_origin()),
        Some(binding),
        MigrationPolicyKind::CrossRevisionDirect,
        world.world_binding(),
    );
    let transaction = world
        .prepare_cross_revision_cutover(
            Arc::clone(&target),
            published_source(&target, "fixture://signal-boundary-cutover"),
            &descriptor,
            &diff,
            &CutoverPreflightLimits::new(1_048_576),
            &CutoverTransactionLimits::default(),
        )
        .unwrap();
    world.step(TickInput::new(100)).unwrap();
    world.step(TickInput::new(100)).unwrap();
    assert_eq!(world.time_ms(), 400);
    assert!(eligibility_clocks(&world).is_empty());
    let commit = transaction.commit(&mut world).unwrap();
    assert_eq!(commit.events.as_slice().len(), 1);
    assert_eq!(world.time_ms(), 400);
    assert!(eligibility_clocks(&world).is_empty());
    assert_snapshot_roundtrip(&world);
    world.step(TickInput::new(100)).unwrap();
}

#[test]
fn resource_free_gate_emits_decision_for_red_and_green() {
    for green in [false, true] {
        let revision =
            compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
                2,
                false,
                false,
                false,
                13.0,
                ConflictPolicyFixture {
                    right_turn_signal: Some(GateInterpretation::ProtectedGroup),
                    signal_cycle_ms: Some([100, 10_000]),
                    ..Default::default()
                },
            ));
        let mut world = install_fixture(revision, WorldConfig::new(4, 4, 64, 4, 1, 100)).unwrap();
        if green {
            world.step(TickInput::new(100)).unwrap();
        }
        let [route, _] = right_turn_routes(&mut world);
        let subject = at_gate(&mut world, route);
        world.step(TickInput::new(100)).unwrap();
        assert_eq!(
            calibration_outcome(&world, subject),
            if green {
                ConflictDecisionOutcome::NotRequired
            } else {
                ConflictDecisionOutcome::NotEvaluated
            }
        );
        assert_eq!(
            world.vehicle(subject).unwrap().route_edge_index(),
            u32::from(green)
        );
        assert!(world.conflict_reservation(subject).is_none());
    }
}

#[test]
fn resource_free_gate_keeps_following_resource_gate_in_the_same_tick() {
    for blocked in [false, true] {
        let revision =
            compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
                2,
                false,
                true,
                false,
                13.0,
                ConflictPolicyFixture {
                    conflict_after_release: true,
                    ..Default::default()
                },
            ));
        let mut world = install_fixture(revision, WorldConfig::new(4, 4, 64, 4, 1, 1_000)).unwrap();
        let [route, conflicting_route] = right_turn_routes(&mut world);
        if blocked {
            world
                .spawn_vehicle(VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    conflicting_route,
                    1,
                    13_000,
                    10_000,
                ))
                .unwrap();
        }
        let subject = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                calibration_gate(&world, route),
                13_000,
            ))
            .unwrap();
        world.step(TickInput::new(1_000)).unwrap();
        let decisions: Vec<_> = world
            .latest_conflict_decisions()
            .iter()
            .filter(|decision| decision.vehicle() == subject)
            .map(|decision| (decision.anchor().hop(), decision.outcome()))
            .collect();
        assert_eq!(
            decisions,
            vec![
                (0, ConflictDecisionOutcome::NotRequired),
                (
                    1,
                    if blocked {
                        ConflictDecisionOutcome::NoGrant(ConflictNoGrantReason::ConflictOccupied)
                    } else {
                        ConflictDecisionOutcome::Granted
                    }
                ),
            ]
        );
        assert_eq!(world.conflict_reservation(subject).is_some(), !blocked);
    }
}

#[test]
fn resource_free_decisions_follow_final_motion_after_new_or_held_reservation() {
    for speed in [6_500, 13_000] {
        let revision =
            compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
                2,
                false,
                true,
                false,
                13.0,
                ConflictPolicyFixture {
                    resource_free_release: true,
                    ..Default::default()
                },
            ));
        let mut world = install_fixture(revision, WorldConfig::new(4, 4, 64, 4, 1, 1_000)).unwrap();
        let [route, _] = right_turn_routes(&mut world);
        let subject = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                calibration_gate(&world, route),
                speed,
            ))
            .unwrap();
        world.step(TickInput::new(1_000)).unwrap();
        if speed == 6_500 {
            assert!(world.conflict_reservation(subject).is_some());
            assert_eq!(world.vehicle(subject).unwrap().route_edge_index(), 1);
            world.step(TickInput::new(1_000)).unwrap();
        }
        assert!(
            world
                .latest_conflict_decisions()
                .iter()
                .any(|decision| decision.vehicle() == subject
                    && decision.anchor().hop() == 1
                    && decision.outcome() == ConflictDecisionOutcome::NotRequired),
            "initial speed {speed}; state {:?}; decisions {:?}",
            world.vehicle(subject),
            world.latest_conflict_decisions()
        );
        assert_eq!(world.vehicle(subject).unwrap().route_edge_index(), 2);
    }
}

#[test]
fn rejected_resource_gate_does_not_report_unreached_resource_free_gate() {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                resource_free_release: true,
                ..Default::default()
            },
        ));
    let mut world = install_fixture(revision, WorldConfig::new(4, 4, 64, 4, 1, 1_000)).unwrap();
    let [route, conflicting_route] = right_turn_routes(&mut world);
    at_gate(&mut world, conflicting_route);
    let subject = world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            calibration_gate(&world, route),
            13_000,
        ))
        .unwrap();
    world.step(TickInput::new(1_000)).unwrap();
    let decisions: Vec<_> = world
        .latest_conflict_decisions()
        .iter()
        .filter(|decision| decision.vehicle() == subject)
        .map(|decision| (decision.anchor().hop(), decision.outcome()))
        .collect();
    assert_eq!(
        decisions,
        vec![(
            0,
            ConflictDecisionOutcome::NoGrant(ConflictNoGrantReason::ConflictOccupied)
        )]
    );
}

#[test]
fn waiting_capacity_denial_is_observable_without_claiming_following_conflict() {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                waiting: true,
                waiting_on_north_only: true,
                conflict_after_release: true,
                include_long_vehicle: true,
                ..Default::default()
            },
        ));
    let long = calibration_profile(&revision, "long-vehicle");
    let short = calibration_profile(&revision, "car");
    let mut world = install_fixture(revision, WorldConfig::new(4, 4, 64, 4, 1, 100)).unwrap();
    let [east, north] = right_turn_routes(&mut world);
    let leader = world
        .spawn_vehicle(VehicleSpawnInput::new(
            short,
            north,
            0,
            calibration_gate(&world, north),
            10_000,
        ))
        .unwrap();
    for _ in 0..20 {
        world.step(TickInput::new(100)).unwrap();
        if world.vehicle(leader).unwrap().route_edge_index() == 1
            && world.vehicle(leader).unwrap().progress_mm() >= 8_000
        {
            break;
        }
    }
    assert!(
        world
            .vehicle(leader)
            .unwrap()
            .waiting_membership()
            .is_some()
    );
    assert!(world.conflict_reservation(leader).is_none());
    let waiting = world
        .spawn_vehicle(VehicleSpawnInput::new(
            long,
            north,
            0,
            calibration_gate(&world, north),
            10_000,
        ))
        .unwrap();
    let ready = world
        .spawn_vehicle(VehicleSpawnInput::new(short, east, 1, 13_000, 10_000))
        .unwrap();
    for tick in 0..3 {
        world.step(TickInput::new(100)).unwrap();
        assert_eq!(
            calibration_outcome(&world, waiting),
            ConflictDecisionOutcome::NoGrant(ConflictNoGrantReason::WaitingCapacity)
        );
        assert!(world.conflict_reservation(waiting).is_none());
        if tick == 0 {
            assert_eq!(
                calibration_outcome(&world, ready),
                ConflictDecisionOutcome::Granted
            );
        }
    }
}

fn right_turn_revision(
    interpretation: GateInterpretation,
    deny: bool,
) -> Arc<SharedNetworkRevision> {
    compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
        2,
        false,
        true,
        false,
        13.0,
        ConflictPolicyFixture {
            yielding: true,
            right_turn_signal: Some(interpretation),
            deny,
            ..Default::default()
        },
    ))
}

fn right_turn_routes(world: &mut TrafficWorld) -> [RouteHandle; 2] {
    [
        ["east-entry", "east-internal", "west-exit"],
        ["north-entry", "north-internal", "south-exit"],
    ]
    .map(|edges| register_conflict_route(world, &edges))
}

fn register_conflict_route(world: &mut TrafficWorld, keys: &[&str]) -> RouteHandle {
    let limits = CompileLimits::p100_initial_v2();
    let revision = world.revision();
    let edges: Vec<_> = keys
        .iter()
        .map(|key| {
            let id = derive_canonical_stable_id_v1(
                EntityKind::LaneEdge,
                "city/runtime-conflict",
                key,
                &limits,
            )
            .unwrap();
            revision
                .identity()
                .ordinal(LaneEdgeId::from_untyped(id))
                .unwrap()
        })
        .collect();
    world
        .register_route(RouteRegisterInput::new(edges))
        .unwrap()
}

fn at_gate(world: &mut TrafficWorld, route: RouteHandle) -> VehicleHandle {
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            calibration_gate(world, route),
            10_000,
        ))
        .unwrap()
}

#[test]
fn formal_right_turn_red_uses_compiled_policy_and_still_yields() {
    for (interpretation, deny, priority_approach, expected) in [
        (
            GateInterpretation::CnCircularRightTurn,
            false,
            true,
            ConflictDecisionOutcome::NoGrant(ConflictNoGrantReason::LeadGap),
        ),
        (
            GateInterpretation::CnCircularRightTurn,
            false,
            false,
            ConflictDecisionOutcome::Granted,
        ),
        (
            GateInterpretation::DirectionalRightPermissive,
            false,
            false,
            ConflictDecisionOutcome::NotEvaluated,
        ),
        (
            GateInterpretation::CnCircularRightTurn,
            true,
            false,
            ConflictDecisionOutcome::NotEvaluated,
        ),
    ] {
        let revision = right_turn_revision(interpretation, deny);
        let mut world = install_fixture(revision, WorldConfig::new(4, 4, 64, 4, 1, 100)).unwrap();
        let [right_turn, priority_route] = right_turn_routes(&mut world);
        let subject = at_gate(&mut world, right_turn);
        if priority_approach {
            world
                .spawn_vehicle(VehicleSpawnInput::new(
                    VehicleProfileOrdinal::from_raw(0),
                    priority_route,
                    0,
                    calibration_gate(&world, priority_route) - 2_000,
                    10_000,
                ))
                .unwrap();
        }
        let signals = world.committed_signal_groups();
        assert_eq!(signals.as_slice().len(), 1);
        assert_eq!(signals.as_slice()[0].1, SignalAspect::Red);
        world.step(TickInput::new(100)).unwrap();
        assert_eq!(calibration_outcome(&world, subject), expected);
        let granted = expected == ConflictDecisionOutcome::Granted;
        assert_eq!(world.conflict_reservation(subject).is_some(), granted);
        assert_eq!(
            world.vehicle(subject).unwrap().route_edge_index(),
            u32::from(granted)
        );
        if !granted {
            assert!(
                world
                    .latest_transition_events()
                    .iter()
                    .all(|event| event.vehicle() != subject)
            );
        }
    }
}

fn alternating_signal_world(interpretation: GateInterpretation) -> TrafficWorld {
    let revision =
        compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            true,
            false,
            13.0,
            ConflictPolicyFixture {
                yielding: true,
                right_turn_signal: Some(interpretation),
                signal_cycle_ms: Some([100, 10_000]),
                ..Default::default()
            },
        ));
    install_fixture(revision, WorldConfig::new(4, 4, 64, 4, 1, 100)).unwrap()
}

#[test]
fn protected_green_skips_red_approach_gap_but_permissive_keeps_it() {
    for (interpretation, expected) in [
        (
            GateInterpretation::ProtectedGroup,
            ConflictDecisionOutcome::Granted,
        ),
        (
            GateInterpretation::DirectionalRightProtected,
            ConflictDecisionOutcome::Granted,
        ),
        (
            GateInterpretation::PermissiveGroup,
            ConflictDecisionOutcome::NoGrant(ConflictNoGrantReason::LeadGap),
        ),
    ] {
        let mut world = alternating_signal_world(interpretation);
        world.step(TickInput::new(100)).unwrap();
        let [subject_route, priority_route] = right_turn_routes(&mut world);
        let subject = at_gate(&mut world, subject_route);
        let target = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                priority_route,
                0,
                calibration_gate(&world, priority_route) - 2_000,
                10_000,
            ))
            .unwrap();
        world.step(TickInput::new(100)).unwrap();
        assert_eq!(
            calibration_outcome(&world, subject),
            expected,
            "{interpretation:?}"
        );
        let granted = expected == ConflictDecisionOutcome::Granted;
        assert_eq!(world.conflict_reservation(subject).is_some(), granted);
        assert_eq!(
            world.vehicle(subject).unwrap().route_edge_index(),
            u32::from(granted)
        );
        assert!(world.conflict_reservation(target).is_none());
        assert_eq!(world.vehicle(target).unwrap().route_edge_index(), 0);
    }
}

#[test]
fn protected_green_waits_for_reservation_and_occupancy_but_skips_clear_lag() {
    for (interpretation, expected) in [
        (
            GateInterpretation::ProtectedGroup,
            ConflictDecisionOutcome::Granted,
        ),
        (
            GateInterpretation::DirectionalRightProtected,
            ConflictDecisionOutcome::Granted,
        ),
        (
            GateInterpretation::PermissiveGroup,
            ConflictDecisionOutcome::NoGrant(ConflictNoGrantReason::LagGap),
        ),
    ] {
        let mut world = alternating_signal_world(interpretation);
        let [subject_route, priority_route] = right_turn_routes(&mut world);
        let target = at_gate(&mut world, priority_route);
        world.step(TickInput::new(100)).unwrap();
        assert!(world.conflict_reservation(target).is_some());
        assert!(
            world.latest_transition_events().iter().all(|event| {
                !matches!(event.kind(), TrafficTransitionKind::ConflictEntered { .. })
            }),
            "the first green tick reserves the passage before physical entry"
        );
        let subject = at_gate(&mut world, subject_route);
        let mut entered = false;
        let mut rejected_while_occupied = false;
        for _ in 0..30 {
            world.step(TickInput::new(100)).unwrap();
            assert_eq!(
                calibration_outcome(&world, subject),
                ConflictDecisionOutcome::NoGrant(ConflictNoGrantReason::ConflictOccupied),
                "{interpretation:?} must preserve reservation and occupancy exclusion",
            );
            assert!(world.conflict_reservation(subject).is_none());
            rejected_while_occupied |= entered;
            entered |= world.latest_transition_events().iter().any(|event| {
                event.vehicle() == target
                    && matches!(event.kind(), TrafficTransitionKind::ConflictEntered { .. })
            });
            if world.conflict_reservation(target).is_none() {
                break;
            }
        }
        assert!(entered && rejected_while_occupied);
        assert!(world.conflict_reservation(target).is_none());
        assert!(world.latest_transition_events().iter().any(|event| {
            event.vehicle() == target
                && matches!(event.kind(), TrafficTransitionKind::ConflictCleared { .. })
        }));
        let bytes = encode_lfrs(&world.capture_snapshot().unwrap());
        let snapshot = snapshot_wire::size_prefixed_root_as_runtime_snapshot(&bytes).unwrap();
        assert!(snapshot.conflict_lag_states().iter().any(|row| {
            row.reference_kind() == snapshot_wire::ConflictLagReferenceKind::ActualClear
                && row.reference_time_ms() == world.time_ms()
        }));
        world.step(TickInput::new(100)).unwrap();
        assert_eq!(
            calibration_outcome(&world, subject),
            expected,
            "{interpretation:?}"
        );
        assert_eq!(
            world.conflict_reservation(subject).is_some(),
            expected == ConflictDecisionOutcome::Granted,
        );
    }
}

#[test]
fn protected_green_still_requires_downstream_storage() {
    for interpretation in [
        GateInterpretation::ProtectedGroup,
        GateInterpretation::DirectionalRightProtected,
    ] {
        let mut world = alternating_signal_world(interpretation);
        world.step(TickInput::new(100)).unwrap();
        let [route, _] = right_turn_routes(&mut world);
        world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                1,
                10_501,
                0,
            ))
            .unwrap();
        let subject = at_gate(&mut world, route);
        world.step(TickInput::new(100)).unwrap();
        assert_eq!(
            calibration_outcome(&world, subject),
            ConflictDecisionOutcome::NoGrant(ConflictNoGrantReason::DownstreamStorageBoundary),
            "{interpretation:?}",
        );
        assert!(world.conflict_reservation(subject).is_none());
        assert_eq!(world.vehicle(subject).unwrap().route_edge_index(), 0);
        assert!(
            world
                .latest_transition_events()
                .iter()
                .all(|event| event.vehicle() != subject)
        );
    }
}

fn added_conflict_floor_world(old_vehicle: bool) -> TrafficWorld {
    let module = |include_conflict| {
        conflict_road_editing_module_with_shape_and_speed(
            2,
            false,
            include_conflict,
            false,
            13.0,
            ConflictPolicyFixture {
                yielding: true,
                ..Default::default()
            },
        )
    };
    let (base, target, diff, binding) = compile_conflict_cutover_pair(module(false), module(true));
    let mut world =
        install_fixture(Arc::clone(&base), WorldConfig::new(4, 4, 64, 4, 1, 4)).unwrap();
    if old_vehicle {
        let route =
            register_conflict_route(&mut world, &["north-entry", "north-internal", "south-exit"]);
        let old = world
            .spawn_vehicle(VehicleSpawnInput::new(
                VehicleProfileOrdinal::from_raw(0),
                route,
                1,
                10_000,
                10_000,
            ))
            .unwrap();
        world.step(TickInput::new(4)).unwrap();
        let state = world.vehicle(old).unwrap();
        assert!(
            state.progress_mm() - 4_500 > 5_501,
            "old tail cleared the future passage"
        );
        world.despawn_vehicle(old).unwrap();
    }
    let descriptor = NetworkRevisionCutoverDescriptor::new(
        LfcaOriginBinding::from_canonical_origin(*base.canonical_origin()),
        LfcaOriginBinding::from_canonical_origin(*target.canonical_origin()),
        Some(binding),
        MigrationPolicyKind::CrossRevisionDirect,
        world.world_binding(),
    );
    let transaction = world
        .prepare_cross_revision_cutover(
            Arc::clone(&target),
            published_source(&target, "fixture://policy-acceptance-floor"),
            &descriptor,
            &diff,
            &CutoverPreflightLimits::new(1_048_576),
            &CutoverTransactionLimits::default(),
        )
        .unwrap();
    if old_vehicle {
        for _ in 0..25 {
            world.step(TickInput::new(4)).unwrap();
        }
    }
    let commit = transaction.commit(&mut world).unwrap();
    assert_eq!(
        commit.events.as_slice().len(),
        1,
        "only the cutover event is published"
    );
    assert!(
        world.latest_transition_events().is_empty(),
        "no fabricated clear event"
    );
    assert_eq!(world.time_ms(), if old_vehicle { 104 } else { 0 });
    world
}

#[test]
fn new_conflict_floor_survives_restore_and_enforces_496_500_ms_from_commit() {
    for old_vehicle in [false, true] {
        let mut original = added_conflict_floor_world(old_vehicle);
        let floor = original.time_ms();
        let bytes = encode_lfrs(&original.capture_snapshot().unwrap());
        let wire = snapshot_wire::size_prefixed_root_as_runtime_snapshot(&bytes).unwrap();
        assert_eq!(
            wire.conflict_lag_states().len(),
            original.conflict_passage_cell_count()
        );
        for row in wire.conflict_lag_states() {
            assert_eq!(
                row.reference_kind(),
                snapshot_wire::ConflictLagReferenceKind::CutoverFloor
            );
            assert_eq!(row.reference_time_ms(), floor);
        }
        let mut restored = restore_lfrs(
            &bytes,
            original.revision(),
            original.committed_source().clone(),
            original.config(),
            SnapshotRestoreLimits::new(1_048_576, 1_024),
        )
        .unwrap()
        .into_world();
        for world in [&mut original, &mut restored] {
            let route =
                register_conflict_route(world, &["east-entry", "east-internal", "west-exit"]);
            let subject = at_gate(world, route);
            // The supported minimum fixed tick is 4 ms. Exact 499/500 ms is
            // covered by the shared gap primitive; this formal path reaches 496/500.
            for elapsed in (0..500).step_by(4) {
                assert_eq!(world.time_ms(), floor + elapsed);
                world.step(TickInput::new(4)).unwrap();
                assert_eq!(
                    calibration_outcome(world, subject),
                    ConflictDecisionOutcome::NoGrant(ConflictNoGrantReason::LagGap),
                    "new cell must reject through 496 ms after commit"
                );
                assert!(world.conflict_reservation(subject).is_none());
                assert!(world.latest_transition_events().is_empty());
            }
            world.step(TickInput::new(4)).unwrap();
            assert_eq!(
                calibration_outcome(world, subject),
                ConflictDecisionOutcome::Granted
            );
        }
        assert_eq!(
            deterministic_state_digest(&original.capture_snapshot().unwrap()).unwrap(),
            deterministic_state_digest(&restored.capture_snapshot().unwrap()).unwrap(),
        );
    }
}
