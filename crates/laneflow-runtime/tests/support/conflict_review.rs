use super::*;

fn revision(options: ConflictPolicyFixture) -> Arc<SharedNetworkRevision> {
    compile_road_editing_revision(conflict_road_editing_module_with_shape_and_speed(
        2, false, true, false, 13.0, options,
    ))
}

fn routes(world: &mut TrafficWorld, revision: &SharedNetworkRevision) -> [RouteHandle; 2] {
    [0, 1].map(|index| {
        let stream = revision
            .conflict()
            .participant_stream(ParticipantStreamOrdinal::from_raw(index))
            .unwrap();
        let edges = revision
            .traffic()
            .maneuvers()
            .maneuver_path(stream.maneuver_path())
            .unwrap()
            .edges()
            .to_vec();
        world
            .register_route(RouteRegisterInput::new(edges))
            .unwrap()
    })
}

fn spawn(world: &mut TrafficWorld, route: RouteHandle) -> VehicleHandle {
    let boundary = calibration_gate(world, route);
    world
        .spawn_vehicle(VehicleSpawnInput::new(
            VehicleProfileOrdinal::from_raw(0),
            route,
            0,
            boundary - 1,
            10_000,
        ))
        .unwrap()
}

#[test]
fn waiting_entry_cannot_share_conflict_coverage_in_a_compiled_network() {
    let module = conflict_road_editing_module_with_shape_and_speed(
        2,
        false,
        true,
        false,
        13.0,
        ConflictPolicyFixture {
            waiting: true,
            ..Default::default()
        },
    );
    let limits = CompileLimits::p100_initial_v2();
    let source = lfre::RoadEditingSourceWriter::new(&limits)
        .write(module)
        .unwrap();
    let input =
        lfre::RoadEditingModuleInput::try_new("runtime-conflict.lfre", source.as_bytes(), None)
            .unwrap();
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_road_editing_module(input).unwrap();
    let error = Compiler::new()
        .compile(unit.build().unwrap())
        .err()
        .expect("overlap must fail compilation");
    assert!(error.diagnostics().iter().any(|diagnostic| {
        format!("{:?}", diagnostic.payload())
            .contains("participantStream.passages.waitingZoneOverlap")
    }));
}

#[test]
fn tail_clearance_and_route_completion_commit_in_the_same_tick() {
    let revision = revision(ConflictPolicyFixture {
        clearance: Some((2, 8.5)),
        ..Default::default()
    });
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 8, 1, 100)).unwrap();
    let [route, _] = routes(&mut world, &revision);
    let vehicle = spawn(&mut world, route);
    world.step(TickInput::new(100)).unwrap();
    assert!(world.conflict_reservation(vehicle).is_some());
    let decisions = world.latest_conflict_decisions().to_vec();
    let origin = LfcaOriginBinding::from_canonical_origin(*revision.canonical_origin());
    let descriptor = NetworkRevisionCutoverDescriptor::new(
        origin,
        origin,
        None,
        MigrationPolicyKind::SameRevisionRestore,
        world.world_binding(),
    );
    let _events = world
        .cutover_same_revision(
            Arc::clone(&revision),
            world.committed_source().clone(),
            &descriptor,
            &CutoverPreflightLimits::new(1_048_576),
        )
        .unwrap();
    assert_eq!(world.latest_conflict_decisions(), decisions);
    let restored = restore_lfrs(
        &encode_lfrs(&world.capture_snapshot().unwrap()),
        revision,
        world.committed_source().clone(),
        WorldConfig::new(4, 4, 64, 8, 1, 100),
        SnapshotRestoreLimits::new(1_048_576, 1_024),
    )
    .unwrap();
    let mut restored = restored.into_world();
    for _ in 0..40 {
        world
            .step(TickInput::new(100))
            .expect("tail clearance at RouteEnd must commit");
        restored
            .step(TickInput::new(100))
            .expect("restored tail clearance must commit");
        assert_eq!(
            deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap(),
            deterministic_state_digest(&restored.capture_snapshot().unwrap()).unwrap(),
        );
    }
    assert_eq!(
        world.vehicle(vehicle).unwrap().status(),
        VehicleStatus::Completed
    );
    assert!(world.conflict_reservation(vehicle).is_none());
}

#[test]
fn clearance_target_equal_to_next_gate_is_admitted() {
    let revision = revision(ConflictPolicyFixture {
        next_gate: true,
        clearance: Some((1, 8.5)),
        ..Default::default()
    });
    let mut world =
        install_fixture(Arc::clone(&revision), WorldConfig::new(4, 4, 64, 8, 1, 100)).unwrap();
    let [route, _] = routes(&mut world, &revision);
    let vehicle = spawn(&mut world, route);
    world.step(TickInput::new(100)).unwrap();
    assert_eq!(
        calibration_outcome(&world, vehicle),
        laneflow_runtime::ConflictDecisionOutcome::Granted
    );
}
