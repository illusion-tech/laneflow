use laneflow_core::{
    Acceleration, CoreError, CoreWorld, EdgeLength, EdgeProgress, IidmProfileSpec,
    InitialTrafficData, LaneEdge, LaneGraph, Route, Speed, SpeedLimit, VehicleProfile,
    VehicleProfileHandle, VehicleProfileRegistry, VehicleReplaceBlockerPosition,
    VehicleReplaceExternalId, VehicleReplaceInput, VehicleReplaceOutcome, VehicleSpawnInput,
    VehicleStatus,
};

fn progress(value: f64) -> EdgeProgress {
    EdgeProgress::try_new(value).expect("valid progress")
}

fn speed(value: f64) -> Speed {
    Speed::try_new(value).expect("valid speed")
}

fn profile(id: &str, length: f64) -> VehicleProfile {
    VehicleProfile::try_new_iidm(
        id,
        IidmProfileSpec {
            length,
            desired_speed: 9.0,
            min_gap: 2.0,
            time_headway: 1.5,
            max_acceleration: 1.4,
            comfortable_deceleration: 2.0,
            emergency_deceleration: 4.0,
        },
    )
    .expect("valid profile")
}

fn replace_world(
    vehicles: impl FnOnce(VehicleProfileHandle, VehicleProfileHandle) -> Vec<VehicleSpawnInput>,
) -> (CoreWorld, VehicleProfileHandle, VehicleProfileHandle) {
    let lane_graph = LaneGraph::try_new([
        LaneEdge::new(
            "old-edge",
            EdgeLength::try_new(100.0).expect("edge length"),
            SpeedLimit::try_new(10.0).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
        LaneEdge::new(
            "target-edge",
            EdgeLength::try_new(100.0).expect("edge length"),
            SpeedLimit::try_new(10.0).expect("speed limit"),
            ["target-next-edge"],
        ),
        LaneEdge::new(
            "target-next-edge",
            EdgeLength::try_new(100.0).expect("edge length"),
            SpeedLimit::try_new(10.0).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
        LaneEdge::new(
            "other-edge",
            EdgeLength::try_new(100.0).expect("edge length"),
            SpeedLimit::try_new(10.0).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
        LaneEdge::new(
            "unused-edge",
            EdgeLength::try_new(100.0).expect("edge length"),
            SpeedLimit::try_new(10.0).expect("speed limit"),
            std::iter::empty::<&str>(),
        ),
    ])
    .expect("valid graph");
    let profiles =
        VehicleProfileRegistry::try_new([profile("standard", 4.5), profile("compact", 3.5)])
            .expect("valid profiles");
    let standard = profiles
        .profile_handle("standard")
        .expect("standard profile");
    let compact = profiles.profile_handle("compact").expect("compact profile");
    let traffic = InitialTrafficData::try_new(
        lane_graph,
        [
            Route::try_new("old-route", ["old-edge"]).expect("old route"),
            Route::try_new("target-route", ["target-edge", "target-next-edge"])
                .expect("target route"),
            Route::try_new("other-route", ["other-edge"]).expect("other route"),
            Route::try_new("unused-route", ["unused-edge"]).expect("unused route"),
        ],
        profiles,
    )
    .expect("valid traffic");
    let world = CoreWorld::with_traffic_data(20, traffic, vehicles(standard, compact))
        .expect("valid world");
    (world, standard, compact)
}

fn preserve_input(
    world: &CoreWorld,
    profile: VehicleProfileHandle,
    progress_value: f64,
) -> VehicleReplaceInput {
    VehicleReplaceInput::new(
        VehicleReplaceExternalId::Preserve,
        profile,
        world.route_handle("target-route").expect("target route"),
        0,
        progress(progress_value),
        speed(5.0),
    )
}

#[test]
fn preserve_replaces_identity_profile_route_and_stable_order_atomically() {
    let (mut world, standard, compact) = replace_world(|standard, _| {
        vec![
            VehicleSpawnInput::active(
                "a-active",
                standard,
                "target-route",
                0,
                progress(80.0),
                Speed::ZERO,
            ),
            VehicleSpawnInput::completed("m-completed", standard, "old-route", 0, progress(100.0)),
            VehicleSpawnInput::stopped("z-stopped", standard, "other-route", 0, progress(50.0)),
        ]
    });
    let old = world.vehicle_handle("m-completed").expect("old handle");
    let before_order = world
        .vehicles()
        .map(|vehicle| vehicle.handle)
        .collect::<Vec<_>>();
    let input = preserve_input(&world, compact, 10.0);

    let outcome = world
        .replace_completed_vehicle(old, &input)
        .expect("replacement succeeds");
    let VehicleReplaceOutcome::Replaced(record) = outcome else {
        panic!("replacement must not be blocked")
    };

    assert_eq!(record.old, old);
    assert_ne!(record.new, old);
    assert_eq!(world.vehicle(old), None);
    assert_eq!(world.vehicle_handle("m-completed"), Some(record.new));
    assert_eq!(world.vehicle_external_id(record.new), Some("m-completed"));
    let replacement = world.vehicle(record.new).expect("replacement is live");
    assert_eq!(replacement.profile, compact);
    assert_eq!(replacement.route, input.route);
    assert_eq!(replacement.route_edge_index, 0);
    assert_eq!(replacement.edge_progress, progress(10.0));
    assert_eq!(replacement.current_speed, speed(5.0));
    assert_eq!(replacement.applied_acceleration, Acceleration::ZERO);
    assert_eq!(replacement.status, VehicleStatus::Active);

    let after_order = world
        .vehicles()
        .map(|vehicle| vehicle.handle)
        .collect::<Vec<_>>();
    assert_eq!(after_order, [before_order[0], record.new, before_order[2]]);

    let old_route = world.route_handle("old-route").expect("old route");
    world
        .remove_route(old_route)
        .expect("replacement detached old route reference");
    world
        .despawn_vehicle(before_order[0])
        .expect("remove earlier target-route vehicle");
    std::assert_matches!(
        world
            .remove_route(input.route)
            .expect_err("replacement owns target route reference"),
        CoreError::RouteInUse { vehicle, .. } if vehicle == record.new
    );
    assert_eq!(
        world.vehicle_profile(standard).unwrap().external_id(),
        "standard"
    );
}

#[test]
fn replace_with_new_id_and_same_id_normalization_both_work() {
    let (mut new_id_world, standard, _) = replace_world(|standard, _| {
        vec![VehicleSpawnInput::completed(
            "old-id",
            standard,
            "old-route",
            0,
            progress(100.0),
        )]
    });
    let old = new_id_world.vehicle_handle("old-id").expect("old handle");
    let route = new_id_world
        .route_handle("target-route")
        .expect("target route");
    let input = VehicleReplaceInput::new(
        VehicleReplaceExternalId::ReplaceWith("new-id".to_owned()),
        standard,
        route,
        0,
        EdgeProgress::ZERO,
        Speed::ZERO,
    );
    let VehicleReplaceOutcome::Replaced(record) = new_id_world
        .replace_completed_vehicle(old, &input)
        .expect("new ID replacement")
    else {
        panic!("replacement must succeed")
    };
    assert_eq!(new_id_world.vehicle_handle("old-id"), None);
    assert_eq!(new_id_world.vehicle_handle("new-id"), Some(record.new));

    let (mut same_id_world, standard, _) = replace_world(|standard, _| {
        vec![VehicleSpawnInput::completed(
            "same-id",
            standard,
            "old-route",
            0,
            progress(100.0),
        )]
    });
    let old = same_id_world.vehicle_handle("same-id").expect("old handle");
    let input = VehicleReplaceInput::new(
        VehicleReplaceExternalId::ReplaceWith("same-id".to_owned()),
        standard,
        same_id_world
            .route_handle("target-route")
            .expect("target route"),
        0,
        EdgeProgress::ZERO,
        Speed::ZERO,
    );
    let VehicleReplaceOutcome::Replaced(record) = same_id_world
        .replace_completed_vehicle(old, &input)
        .expect("same ID normalizes to preserve")
    else {
        panic!("replacement must succeed")
    };
    assert_eq!(same_id_world.vehicle_handle("same-id"), Some(record.new));
}

#[test]
fn overlap_block_is_typed_atomic_and_reuses_the_same_input_on_retry() {
    let (mut world, standard, _) = replace_world(|standard, _| {
        vec![
            VehicleSpawnInput::active(
                "blocker",
                standard,
                "target-route",
                0,
                progress(12.0),
                Speed::ZERO,
            ),
            VehicleSpawnInput::completed("old", standard, "old-route", 0, progress(100.0)),
        ]
    });
    let old = world.vehicle_handle("old").expect("old handle");
    let blocker = world.vehicle_handle("blocker").expect("blocker handle");
    let input = preserve_input(&world, standard, 10.0);
    let before = world.clone();

    let VehicleReplaceOutcome::Blocked(block) = world
        .replace_completed_vehicle(old, &input)
        .expect("overlap is recoverable")
    else {
        panic!("overlap must block")
    };
    assert_eq!(block.old, old);
    assert_eq!(block.blocker, blocker);
    assert_eq!(block.blocker_position, VehicleReplaceBlockerPosition::Ahead);
    assert!(block.bumper_gap < 0.0);
    assert_eq!(world, before);

    world.despawn_vehicle(blocker).expect("remove blocker");
    let VehicleReplaceOutcome::Replaced(record) = world
        .replace_completed_vehicle(old, &input)
        .expect("same borrowed input retries")
    else {
        panic!("retry must succeed")
    };
    assert_eq!(world.vehicle_handle("old"), Some(record.new));
}

#[test]
fn overlap_reports_a_blocker_behind_the_replacement() {
    let (mut world, standard, _) = replace_world(|standard, _| {
        vec![
            VehicleSpawnInput::active(
                "blocker",
                standard,
                "target-route",
                0,
                progress(8.0),
                Speed::ZERO,
            ),
            VehicleSpawnInput::completed("old", standard, "old-route", 0, progress(100.0)),
        ]
    });
    let old = world.vehicle_handle("old").expect("old handle");
    let blocker = world.vehicle_handle("blocker").expect("blocker handle");
    let input = preserve_input(&world, standard, 10.0);

    let VehicleReplaceOutcome::Blocked(block) = world
        .replace_completed_vehicle(old, &input)
        .expect("overlap is recoverable")
    else {
        panic!("overlap must block")
    };
    assert_eq!(block.blocker, blocker);
    assert_eq!(
        block.blocker_position,
        VehicleReplaceBlockerPosition::Behind
    );
}

#[test]
fn overlap_across_an_edge_boundary_is_typed_and_atomic() {
    let (mut world, standard, _) = replace_world(|standard, _| {
        vec![
            VehicleSpawnInput::active(
                "blocker",
                standard,
                "target-route",
                1,
                progress(1.0),
                Speed::ZERO,
            ),
            VehicleSpawnInput::completed("old", standard, "old-route", 0, progress(100.0)),
        ]
    });
    let old = world.vehicle_handle("old").expect("old handle");
    let blocker = world.vehicle_handle("blocker").expect("blocker handle");
    let input = preserve_input(&world, standard, 99.0);
    let before = world.clone();

    let VehicleReplaceOutcome::Blocked(block) = world
        .replace_completed_vehicle(old, &input)
        .expect("cross-boundary overlap is recoverable")
    else {
        panic!("cross-boundary overlap must block")
    };
    assert_eq!(block.blocker, blocker);
    assert_eq!(block.blocker_position, VehicleReplaceBlockerPosition::Ahead);
    assert!(block.bumper_gap < 0.0);
    assert_eq!(world, before);
}

#[test]
fn fatal_validation_failures_leave_world_unchanged() {
    let (world, standard, _) = replace_world(|standard, _| {
        vec![
            VehicleSpawnInput::active(
                "active",
                standard,
                "other-route",
                0,
                progress(20.0),
                Speed::ZERO,
            ),
            VehicleSpawnInput::completed("old", standard, "old-route", 0, progress(100.0)),
        ]
    });
    let old = world.vehicle_handle("old").expect("old handle");
    let target = world.route_handle("target-route").expect("target route");

    let assert_failure = |mut candidate_world: CoreWorld,
                          old,
                          input: VehicleReplaceInput,
                          check: fn(&CoreError) -> bool| {
        let before = candidate_world.clone();
        let error = candidate_world
            .replace_completed_vehicle(old, &input)
            .expect_err("validation must fail");
        assert!(check(&error), "unexpected error: {error:?}");
        assert_eq!(candidate_world, before);
    };

    assert_failure(
        world.clone(),
        world.vehicle_handle("active").expect("active handle"),
        VehicleReplaceInput::new(
            VehicleReplaceExternalId::Preserve,
            standard,
            target,
            0,
            EdgeProgress::ZERO,
            Speed::ZERO,
        ),
        |error| matches!(error, CoreError::VehicleReplaceStatusMismatch { .. }),
    );
    assert_failure(
        world.clone(),
        old,
        VehicleReplaceInput::new(
            VehicleReplaceExternalId::ReplaceWith("active".to_owned()),
            standard,
            target,
            0,
            EdgeProgress::ZERO,
            Speed::ZERO,
        ),
        |error| matches!(error, CoreError::DuplicateVehicleId { .. }),
    );
    assert_failure(
        world.clone(),
        old,
        VehicleReplaceInput::new(
            VehicleReplaceExternalId::ReplaceWith("bad id".to_owned()),
            standard,
            target,
            0,
            EdgeProgress::ZERO,
            Speed::ZERO,
        ),
        |error| matches!(error, CoreError::InvalidExternalId { .. }),
    );
    assert_failure(
        world.clone(),
        old,
        VehicleReplaceInput::new(
            VehicleReplaceExternalId::Preserve,
            standard,
            target,
            2,
            EdgeProgress::ZERO,
            Speed::ZERO,
        ),
        |error| matches!(error, CoreError::InvalidVehicleReplaceRouteEdgeIndex { .. }),
    );
    assert_failure(
        world.clone(),
        old,
        VehicleReplaceInput::new(
            VehicleReplaceExternalId::Preserve,
            standard,
            target,
            0,
            progress(100.1),
            Speed::ZERO,
        ),
        |error| {
            matches!(
                error,
                CoreError::VehicleReplaceEdgeProgressOutOfRange { .. }
            )
        },
    );
    assert_failure(
        world.clone(),
        old,
        VehicleReplaceInput::new(
            VehicleReplaceExternalId::Preserve,
            standard,
            target,
            0,
            EdgeProgress::ZERO,
            speed(10.1),
        ),
        |error| {
            matches!(
                error,
                CoreError::VehicleReplaceInitialSpeedExceedsLimit { .. }
            )
        },
    );

    let foreign_profiles = VehicleProfileRegistry::try_new([
        profile("foreign-a", 4.0),
        profile("foreign-b", 4.0),
        profile("foreign-c", 4.0),
    ])
    .expect("foreign profiles");
    let unknown_profile = foreign_profiles
        .profile_handle("foreign-c")
        .expect("unknown profile handle");
    assert_failure(
        world.clone(),
        old,
        VehicleReplaceInput::new(
            VehicleReplaceExternalId::Preserve,
            unknown_profile,
            target,
            0,
            EdgeProgress::ZERO,
            Speed::ZERO,
        ),
        |error| matches!(error, CoreError::UnknownVehicleProfileHandle { .. }),
    );

    let mut stale_route_world = world.clone();
    let stale_route = stale_route_world
        .route_handle("unused-route")
        .expect("unused route");
    stale_route_world
        .remove_route(stale_route)
        .expect("remove unused route");
    assert_failure(
        stale_route_world,
        old,
        VehicleReplaceInput::new(
            VehicleReplaceExternalId::Preserve,
            standard,
            stale_route,
            0,
            EdgeProgress::ZERO,
            Speed::ZERO,
        ),
        |error| matches!(error, CoreError::UnknownRouteHandle { .. }),
    );

    let mut stale_vehicle_world = world.clone();
    stale_vehicle_world
        .despawn_vehicle(old)
        .expect("despawn old");
    let input = VehicleReplaceInput::new(
        VehicleReplaceExternalId::Preserve,
        standard,
        target,
        0,
        EdgeProgress::ZERO,
        Speed::ZERO,
    );
    assert_failure(stale_vehicle_world, old, input, |error| {
        matches!(error, CoreError::UnknownVehicleHandle { .. })
    });
}
