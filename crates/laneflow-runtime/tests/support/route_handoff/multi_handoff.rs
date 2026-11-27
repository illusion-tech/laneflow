use super::*;

#[test]
fn repeated_edge_pairs_require_distinct_ordered_occurrences_and_share_route_capacity() {
    let revision = compile_road_editing_revision(module(0.));
    let mut world = install_fixture(
        Arc::clone(&revision),
        WorldConfig::try_new(laneflow_runtime::WorldConfigInput {
            vehicle_capacity: 2,
            route_capacity: 4,
            route_edge_occurrence_capacity: 8,
            route_conflict_occurrence_capacity: 8,
            fixed_delta_time_ms: 100,
        })
        .expect("有效的交通配置"),
    )
    .unwrap();
    let rel = revision.traffic().relations();
    let section = [0, 1]
        .into_iter()
        .map(RoadSectionOrdinal::from_raw)
        .find(|&s| rel.section_lanes(s).unwrap().len() == 2)
        .unwrap();
    let lanes = rel.section_lanes(section).unwrap();
    let s = rel.authoring_edge_chain(lanes[0]).unwrap();
    let t = rel.authoring_edge_chain(lanes[1]).unwrap();
    let turn = revision.traffic().successors(t[2]).unwrap()[0];
    let route = RouteRegisterInput::new(vec![s[2], t[1], t[2], turn, s[0], s[1], s[2], t[1]]);
    let first = RouteHandoffOccurrence {
        revision: revision.network_revision(),
        source_index: 0,
        owner: BoundaryOwner::Section(section),
        left_index: 0,
        stations_nm: [24_000_000_000, 25_000_000_000],
    };
    let second = RouteHandoffOccurrence {
        source_index: 6,
        stations_nm: [24_000_000_000, 26_000_000_000],
        ..first
    };
    let windows = [first, second];
    let cursor = world.command_cursor();
    let bound = world.bind_route_handoffs(&route, &windows).unwrap();
    assert_eq!(
        bound.windows().iter().map(|w| w.0).collect::<Vec<_>>(),
        [0, 6]
    );
    assert!(bound.boundary_at(1).is_none());
    let profile = |key| {
        revision
            .identity()
            .stable_id(conflict_profile(&revision, key))
            .unwrap()
    };
    let trial = RouteHandoffTrial {
        profile: profile("car"),
        station_window_nm: [0, 50_000_000_000],
        rear_stations_nm: [10_000_000_000, 40_000_000_000],
        speed_m_s: 1.,
        limits: GeometryWorkLimits::STANDARD,
    };
    assert_eq!(
        world.preview_route_handoff(&bound, &trial).unwrap_err(),
        RouteHandoffTrialError::OccurrenceRequired
    );
    assert_eq!(
        world
            .preview_route_handoff_at(&bound, 1, &trial)
            .unwrap_err(),
        RouteHandoffTrialError::UnboundOccurrence
    );
    let first_result = world.preview_route_handoff_at(&bound, 0, &trial).unwrap();
    assert_eq!(first_result.source_index, 0);
    assert!(matches!(
        first_result.report.outcome,
        HandoffOutcome::OutsideWindow { .. }
    ));
    let second_result = world.preview_route_handoff_at(&bound, 6, &trial).unwrap();
    assert_eq!(second_result.source_index, 6);
    assert!(matches!(
        second_result.report.outcome,
        HandoffOutcome::Located { .. }
    ));
    // 唯一禁行的 t2 位于两次交接之间，两个窗口都须核对这条普通边。
    let denied = RouteHandoffTrial {
        profile: profile("denied"),
        ..trial
    };
    for index in [0, 6] {
        assert_eq!(
            world
                .preview_route_handoff_at(&bound, index, &denied)
                .unwrap_err(),
            RouteHandoffTrialError::AccessDenied
        );
    }

    for invalid in [&[][..], &[second, first], &[first, first]] {
        assert_eq!(
            world.bind_route_handoffs(&route, invalid).unwrap_err(),
            RouteHandoffTrialError::InvalidWindowOrder
        );
    }
    let past_end = RouteHandoffOccurrence {
        source_index: 7,
        ..second
    };
    assert_eq!(
        world
            .bind_route_handoffs(&route, &[first, past_end])
            .unwrap_err(),
        RouteHandoffTrialError::OccurrenceOutOfRange
    );
    let different_revision = RouteHandoffOccurrence {
        revision: NetworkRevisionId::from_digest(Sha256Digest::from_bytes([0; 32])),
        ..second
    };
    assert_eq!(
        world
            .bind_route_handoffs(&route, &[first, different_revision])
            .unwrap_err(),
        RouteHandoffTrialError::RevisionMismatch
    );
    let invalid_boundary = RouteHandoffOccurrence {
        left_index: 8,
        ..second
    };
    assert!(matches!(
        world.bind_route_handoffs(&route, &[first, invalid_boundary]),
        Err(RouteHandoffTrialError::Boundary(_))
    ));
    let broken_leg = RouteRegisterInput::new(vec![s[2], t[1], t[2], s[1], s[2], t[1]]);
    assert_eq!(
        world
            .bind_route_handoffs(
                &broken_leg,
                &[
                    first,
                    RouteHandoffOccurrence {
                        source_index: 4,
                        ..second
                    }
                ]
            )
            .unwrap_err(),
        RouteHandoffTrialError::Route(RouteError::Disconnected)
    );
    assert_eq!(
        world.register_route(route.clone()),
        Err(RouteError::Disconnected)
    );
    assert_eq!(world.command_cursor(), cursor);
    assert_eq!(world.live_routes().count(), 0);

    let ordinary = world
        .register_route(RouteRegisterInput::new(vec![t[2]]))
        .unwrap();
    assert_eq!(
        world.bind_route_handoffs(&route, &windows).unwrap_err(),
        RouteHandoffTrialError::Route(RouteError::EdgeOccurrenceCapacityExceeded)
    );
    world.remove_route(ordinary).unwrap();
    assert!(world.bind_route_handoffs(&route, &windows).is_ok());
}
