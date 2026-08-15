    use super::*;
    use crate::{
        CoreError, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData, LaneEdge,
        ParkingArea, ParkingSpace, ParkingSpaceGeometry, SignalRegistry, Speed, TickInput,
        VehicleParkingState, VehicleProfile, VehicleProfileHandle, VehicleProfileRegistry,
    };
    use proptest::prelude::*;

    fn traffic_data<I>(
        lane_graph: LaneGraph,
        routes: I,
    ) -> (InitialTrafficData, VehicleProfileHandle)
    where
        I: IntoIterator<Item = Route>,
    {
        let registry = VehicleProfileRegistry::try_new(
            &crate::test_support::test_participant_class_registry(),
            [VehicleProfile::try_new_iidm(
                "test-profile",
                crate::test_support::test_car_participant_class(),
                IidmProfileSpec {
                    length: 4.5,
                    desired_speed: 13.9,
                    min_gap: 2.0,
                    time_headway: 1.5,
                    max_acceleration: 1.4,
                    comfortable_deceleration: 2.0,
                    emergency_deceleration: 4.0,
                },
            )
            .expect("valid profile")],
        )
        .expect("valid profile registry");
        let profile = registry
            .profile_handle("test-profile")
            .expect("profile handle exists");
        let traffic_data = InitialTrafficData::try_new(
            lane_graph,
            routes,
            registry,
            crate::JunctionRegistry::empty(),
            crate::SignalRegistry::empty(),
            crate::ParkingRegistry::empty(),
            crate::test_support::test_participant_class_registry(),
            crate::CrossSectionRegistry::empty(),
            crate::AccessRegistry::empty(),
        )
        .expect("valid traffic data");
        (traffic_data, profile)
    }

    fn lifecycle_scale_world(vehicle_count: usize) -> CoreWorld {
        let edge_length = 10.0 * vehicle_count as f64 + 100.0;
        let lane_graph = LaneGraph::try_new([LaneEdge::new(
            "A",
            EdgeLength::try_new(edge_length).expect("scale edge length"),
            crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            Vec::<String>::new(),
        )])
        .expect("scale graph");
        let (traffic_data, profile) = traffic_data(
            lane_graph,
            [Route::try_new("R", ["A"]).expect("scale route")],
        );
        let vehicles = (0..vehicle_count)
            .map(|index| {
                VehicleSpawnInput::active(
                    format!("V{index:06}"),
                    profile,
                    "R",
                    0,
                    EdgeProgress::try_new(5.0 + 10.0 * index as f64).expect("scale progress"),
                    Speed::ZERO,
                )
            })
            .collect();
        CoreWorld::with_traffic_data(20, traffic_data, vehicles).expect("scale world")
    }

    fn completed_replace_world() -> (CoreWorld, VehicleProfileHandle, RouteHandle) {
        let lane_graph = LaneGraph::try_new([LaneEdge::new(
            "A",
            EdgeLength::try_new(10.0).expect("replace edge length"),
            crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            Vec::<String>::new(),
        )])
        .expect("replace graph");
        let (traffic_data, profile) = traffic_data(
            lane_graph,
            [Route::try_new("R", ["A"]).expect("replace route")],
        );
        let world = CoreWorld::with_traffic_data(
            1_000,
            traffic_data,
            vec![VehicleSpawnInput::completed(
                "V",
                profile,
                "R",
                0,
                EdgeProgress::try_new(10.0).expect("route end"),
            )],
        )
        .expect("replace world");
        let route = world.route_handle("R").expect("replace route handle");
        (world, profile, route)
    }

    fn parking_retained_scale_world(vehicle_count: usize) -> CoreWorld {
        let edge_length = 10.0 * vehicle_count as f64 + 100.0;
        let lane_graph = LaneGraph::try_new([LaneEdge::new(
            "A",
            EdgeLength::try_new(edge_length).expect("parking retained edge length"),
            crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            Vec::<String>::new(),
        )])
        .expect("parking retained graph");
        let parking = ParkingRegistry::try_new(
            &lane_graph,
            [],
            (0..vehicle_count).map(|index| {
                ParkingSpace::new(
                    format!("S{index:06}"),
                    None,
                    "A",
                    1.0 + 10.0 * index as f64,
                    "A",
                    2.0 + 10.0 * index as f64,
                    ParkingSpaceGeometry::new(-3.0, 0.0, 5.0, 2.4),
                )
            }),
        )
        .expect("parking retained registry");
        let (base, profile) = traffic_data(
            lane_graph,
            [Route::try_new("R", ["A"]).expect("parking retained route")],
        );
        let traffic = InitialTrafficData::try_new(
            base.lane_graph().clone(),
            base.routes().cloned(),
            base.vehicle_profiles().clone(),
            crate::JunctionRegistry::empty(),
            SignalRegistry::empty(),
            parking,
            crate::test_support::test_participant_class_registry(),
            crate::CrossSectionRegistry::empty(),
            crate::AccessRegistry::empty(),
        )
        .expect("parking retained traffic");
        let vehicles = (0..vehicle_count)
            .map(|index| {
                VehicleSpawnInput::active(
                    format!("V{index:06}"),
                    profile,
                    "R",
                    0,
                    EdgeProgress::try_new(5.0 + 10.0 * index as f64)
                        .expect("parking retained progress"),
                    Speed::ZERO,
                )
            })
            .collect();
        CoreWorld::with_traffic_data(20, traffic, vehicles).expect("parking retained world")
    }

    fn sparse_command_world(background_count: usize) -> (CoreWorld, VehicleProfileHandle) {
        let edge_length = 10.0 * background_count as f64 + 2_000.0;
        let lane_graph = LaneGraph::try_new([LaneEdge::new(
            "A",
            EdgeLength::try_new(edge_length).expect("sparse edge length"),
            crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            Vec::<String>::new(),
        )])
        .expect("sparse graph");
        let (traffic_data, profile) = traffic_data(
            lane_graph,
            [Route::try_new("R", ["A"]).expect("sparse route")],
        );
        let mut vehicles = (0..background_count)
            .map(|index| {
                VehicleSpawnInput::active(
                    format!("B{index:06}"),
                    profile,
                    "R",
                    0,
                    EdgeProgress::try_new(1_000.0 + 10.0 * index as f64)
                        .expect("background progress"),
                    Speed::ZERO,
                )
            })
            .collect::<Vec<_>>();
        vehicles.push(VehicleSpawnInput::active(
            "local",
            profile,
            "R",
            0,
            EdgeProgress::try_new(5.0).expect("local progress"),
            Speed::ZERO,
        ));
        (
            CoreWorld::with_traffic_data(20, traffic_data, vehicles).expect("sparse world"),
            profile,
        )
    }

    fn parking_runtime_world() -> (CoreWorld, VehicleProfileHandle) {
        let lane_graph = LaneGraph::try_new([LaneEdge::new(
            "A",
            EdgeLength::try_new(200.0).expect("parking edge length"),
            crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            Vec::<String>::new(),
        )])
        .expect("parking graph");
        let parking = ParkingRegistry::try_new(
            &lane_graph,
            [ParkingArea::new("lot")],
            [
                ParkingSpace::new(
                    "S0",
                    Some("lot".to_owned()),
                    "A",
                    20.0,
                    "A",
                    40.0,
                    ParkingSpaceGeometry::new(-3.0, 0.0, 4.5, 2.4),
                ),
                ParkingSpace::new(
                    "S1",
                    Some("lot".to_owned()),
                    "A",
                    60.0,
                    "A",
                    80.0,
                    ParkingSpaceGeometry::new(-3.0, 0.0, 4.5, 2.4),
                ),
            ],
        )
        .expect("parking registry");
        let (base, profile) = traffic_data(
            lane_graph,
            [Route::try_new("R", ["A"]).expect("parking route")],
        );
        let traffic = InitialTrafficData::try_new(
            base.lane_graph().clone(),
            base.routes().cloned(),
            base.vehicle_profiles().clone(),
            crate::JunctionRegistry::empty(),
            SignalRegistry::empty(),
            parking,
            crate::test_support::test_participant_class_registry(),
            crate::CrossSectionRegistry::empty(),
            crate::AccessRegistry::empty(),
        )
        .expect("parking traffic data");
        let vehicles = vec![
            VehicleSpawnInput::active("V0", profile, "R", 0, EdgeProgress::ZERO, Speed::ZERO),
            VehicleSpawnInput::active(
                "V1",
                profile,
                "R",
                0,
                EdgeProgress::try_new(120.0).expect("parking progress"),
                Speed::ZERO,
            ),
        ];
        (
            CoreWorld::with_traffic_data(20, traffic, vehicles).expect("parking world"),
            profile,
        )
    }

    fn repeated_parking_target_world() -> CoreWorld {
        let lane_graph = LaneGraph::try_new([
            LaneEdge::new(
                "A",
                EdgeLength::try_new(100.0).expect("A length"),
                crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
                ["B"],
            ),
            LaneEdge::new(
                "B",
                EdgeLength::try_new(100.0).expect("B length"),
                crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
                ["A"],
            ),
        ])
        .expect("repeated target graph");
        let parking = ParkingRegistry::try_new(
            &lane_graph,
            [],
            [ParkingSpace::new(
                "S",
                None,
                "A",
                20.0,
                "A",
                40.0,
                ParkingSpaceGeometry::new(-3.0, 0.0, 4.5, 2.4),
            )],
        )
        .expect("repeated target parking");
        let (base, _) = traffic_data(
            lane_graph,
            [Route::try_new("R", ["A", "B", "A", "B", "A"]).expect("repeated target route")],
        );
        let traffic = InitialTrafficData::try_new(
            base.lane_graph().clone(),
            base.routes().cloned(),
            base.vehicle_profiles().clone(),
            crate::JunctionRegistry::empty(),
            SignalRegistry::empty(),
            parking,
            crate::test_support::test_participant_class_registry(),
            crate::CrossSectionRegistry::empty(),
            crate::AccessRegistry::empty(),
        )
        .expect("repeated target traffic");
        CoreWorld::with_traffic_data(20, traffic, Vec::<VehicleSpawnInput>::new())
            .expect("repeated target world")
    }

    fn reserved_parking_world() -> CoreWorld {
        let (mut world, _) = parking_runtime_world();
        let vehicle = world.vehicle_handle("V0").expect("parking vehicle");
        let space = world.parking().space_handle("S0").expect("parking space");
        world
            .reserve_parking_space(vehicle, space)
            .expect("parking reservation");
        world
    }

    #[test]
    fn first_reachable_parking_target_matches_independent_route_scan_oracle() {
        let world = repeated_parking_target_world();
        let route = world.route_handle("R").expect("route");
        let space = world.parking().space_handle("S").expect("space");
        let progress_samples = [
            0.0,
            19.0,
            20.0,
            20.0 + LONGITUDINAL_CONSTRAINT_TOLERANCE_METERS / 2.0,
            20.0 + 2.0 * LONGITUDINAL_CONSTRAINT_TOLERANCE_METERS,
            99.0,
        ];

        for from_index in 0..5 {
            for from_progress in progress_samples {
                let expected = (from_index..5).find(|candidate| {
                    let is_entry_edge = candidate % 2 == 0;
                    let current_is_reachable = *candidate != from_index
                        || longitudinal_constraint_reached(20.0, from_progress);
                    is_entry_edge && current_is_reachable
                });
                let actual = world
                    .first_reachable_parking_entry(route, from_index, from_progress, space)
                    .map(|target| target.route_edge_index);

                assert_eq!(
                    actual, expected,
                    "from occurrence {from_index} at progress {from_progress}"
                );
            }
        }
    }

    #[test]
    fn unit_step_advances_post_step_time() {
        let mut world = CoreWorld::new(20).expect("valid world");

        let result = world.step(TickInput::new(20)).expect("step succeeds");

        assert_eq!(world.tick_index(), 1);
        assert_eq!(world.time_ms(), 20);
        assert_eq!(result.tick_index, 1);
        assert_eq!(result.time_ms, 20);
    }

    #[test]
    fn candidate_state_scratch_reuses_allocation_across_successful_ticks() {
        let lane_graph = LaneGraph::try_new([LaneEdge::new(
            "A",
            EdgeLength::try_new(10_000.0).expect("valid edge length"),
            crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            Vec::<String>::new(),
        )])
        .expect("valid lane graph");
        let route = Route::try_new("R1", ["A"]).expect("valid route");
        let (traffic_data, profile) = traffic_data(lane_graph, [route]);
        let vehicle =
            VehicleSpawnInput::active("V1", profile, "R1", 0, EdgeProgress::ZERO, Speed::ZERO);
        let mut world =
            CoreWorld::with_traffic_data(16, traffic_data, vec![vehicle]).expect("valid world");
        let capacity = world.candidate_state_scratch.states.capacity();
        let allocation = world.candidate_state_scratch.states.as_ptr();
        assert!(capacity >= world.vehicles.len());
        assert!(world.clone().candidate_state_scratch.states.capacity() >= capacity);

        world.step(TickInput::new(16)).expect("first step succeeds");
        assert!(world.candidate_state_scratch.states.is_empty());
        assert_eq!(world.candidate_state_scratch.states.capacity(), capacity);
        assert_eq!(world.candidate_state_scratch.states.as_ptr(), allocation);

        world
            .step(TickInput::new(16))
            .expect("second step succeeds");

        assert!(world.candidate_state_scratch.states.is_empty());
        assert_eq!(world.candidate_state_scratch.states.capacity(), capacity);
        assert_eq!(world.candidate_state_scratch.states.as_ptr(), allocation);
    }

    #[test]
    fn candidate_state_scratch_is_restored_after_advance_failure() {
        let lane_graph = LaneGraph::try_new([
            LaneEdge::new(
                "A",
                EdgeLength::try_new(f64::MAX).expect("valid edge length"),
                crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
                ["B"],
            ),
            LaneEdge::new(
                "B",
                EdgeLength::try_new(f64::MAX).expect("valid edge length"),
                crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
                Vec::<String>::new(),
            ),
        ])
        .expect("valid lane graph");
        let route = Route::try_new("R1", ["A", "B"]).expect("valid route");
        let (traffic_data, profile) = traffic_data(lane_graph, [route]);
        let vehicle = VehicleSpawnInput::active(
            "V1",
            profile,
            "R1",
            0,
            EdgeProgress::try_new(f64::MAX / 2.0).expect("valid progress"),
            Speed::try_new(f64::MAX).expect("valid speed"),
        );
        let mut world =
            CoreWorld::with_traffic_data(1_000, traffic_data, vec![vehicle]).expect("valid world");
        let before = world.clone();
        let vehicle = world.vehicle_handle("V1").expect("vehicle handle exists");
        let capacity = world.candidate_state_scratch.states.capacity();
        let allocation = world.candidate_state_scratch.states.as_ptr();
        assert!(capacity >= world.vehicles.len());

        let first_error = world
            .step(TickInput::new(1_000))
            .expect_err("overflowing route progress must fail");
        std::assert_matches!(
            first_error,
            CoreError::NonFiniteRouteTravel {
                vehicle: actual_vehicle,
                ..
            } if actual_vehicle == vehicle
        );
        assert_eq!(world, before);
        assert!(world.candidate_state_scratch.states.is_empty());
        assert_eq!(world.candidate_state_scratch.states.capacity(), capacity);
        assert_eq!(world.candidate_state_scratch.states.as_ptr(), allocation);

        let second_error = world
            .step(TickInput::new(1_000))
            .expect_err("repeated overflowing route progress must fail");
        std::assert_matches!(second_error, CoreError::NonFiniteRouteTravel { .. });

        assert_eq!(world, before);
        assert!(world.candidate_state_scratch.states.is_empty());
        assert_eq!(world.candidate_state_scratch.states.capacity(), capacity);
        assert_eq!(world.candidate_state_scratch.states.as_ptr(), allocation);
    }

    #[test]
    fn unit_delta_mismatch_keeps_world_unchanged() {
        let lane_graph = LaneGraph::try_new([LaneEdge::new(
            "A",
            EdgeLength::try_new(10.0).expect("valid edge length"),
            crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            Vec::<String>::new(),
        )])
        .expect("valid lane graph");
        let route = Route::try_new("R1", ["A"]).expect("valid route");
        let (traffic_data, profile) = traffic_data(lane_graph, [route]);
        let vehicle = VehicleSpawnInput::active(
            "V1",
            profile,
            "R1",
            0,
            EdgeProgress::try_new(1.0).expect("valid progress"),
            Speed::try_new(0.0).expect("valid speed"),
        );
        let mut world =
            CoreWorld::with_traffic_data(20, traffic_data, vec![vehicle]).expect("valid world");
        let before = world.clone();

        let error = world
            .step(TickInput::new(16))
            .expect_err("delta mismatch must fail");

        std::assert_matches!(
            error,
            CoreError::TickDeltaMismatch {
                expected_delta_time_ms: 20,
                actual_delta_time_ms: 16
            }
        );
        assert_eq!(world, before);
    }

    #[test]
    fn boundary_epsilon_is_not_treated_as_zero_remainder() {
        assert!(is_edge_boundary_remainder_zero(
            EDGE_BOUNDARY_TOLERANCE_METERS / 2.0
        ));
        assert!(!is_edge_boundary_remainder_zero(
            EDGE_BOUNDARY_TOLERANCE_METERS
        ));
    }

    #[test]
    fn tick_index_overflow_keeps_world_unchanged() {
        let mut world = CoreWorld::new(20).expect("valid world");
        world.tick_index = u64::MAX;
        let before = world.clone();

        let error = world
            .step(TickInput::new(20))
            .expect_err("tick index overflow must fail");

        std::assert_matches!(error, CoreError::TimeOverflow);
        assert_eq!(world, before);
    }

    #[test]
    fn time_ms_overflow_keeps_world_unchanged() {
        let mut world = CoreWorld::new(20).expect("valid world");
        world.time_ms = u64::MAX - 10;
        let before = world.clone();

        let error = world
            .step(TickInput::new(20))
            .expect_err("time overflow must fail");

        std::assert_matches!(error, CoreError::TimeOverflow);
        assert_eq!(world, before);
    }

    #[test]
    fn parking_activation_preserves_error_priority_and_makes_legacy_guard_unreachable() {
        let mut delta_world = reserved_parking_world();
        let before = delta_world.clone();
        std::assert_matches!(
            delta_world.step(TickInput::new(16)),
            Err(CoreError::TickDeltaMismatch { .. })
        );
        assert_eq!(delta_world, before);

        let mut tick_world = reserved_parking_world();
        tick_world.tick_index = u64::MAX;
        let before = tick_world.clone();
        std::assert_matches!(
            tick_world.step(TickInput::new(20)),
            Err(CoreError::TimeOverflow)
        );
        assert_eq!(tick_world, before);

        let mut time_world = reserved_parking_world();
        time_world.time_ms = u64::MAX - 10;
        let before = time_world.clone();
        std::assert_matches!(
            time_world.step(TickInput::new(20)),
            Err(CoreError::TimeOverflow)
        );
        assert_eq!(time_world, before);

        let mut integrity_world = reserved_parking_world();
        integrity_world
            .parking_runtime
            .corrupt_global_capacity_for_test();
        let before = integrity_world.clone();
        std::assert_matches!(
            integrity_world.step(TickInput::new(20)),
            Err(CoreError::ParkingBindingInvariantViolation {
                stage: "step_sentinel",
                ..
            })
        );
        assert_eq!(integrity_world, before);

        let mut capability_world = reserved_parking_world();
        let before_vehicle = capability_world
            .vehicle(capability_world.vehicle_handle("V0").expect("vehicle"))
            .expect("live vehicle")
            .clone();
        let result = capability_world
            .step(TickInput::new(20))
            .expect("#109 activation makes the legacy guard unreachable");
        assert_eq!(result.tick_index, 1);
        assert_eq!(capability_world.parking_snapshot().counts().reserved, 1);
        assert_eq!(
            capability_world
                .parking_snapshot()
                .vehicle_state(before_vehicle.handle),
            Some(crate::VehicleParkingState::Reserved {
                space: capability_world
                    .parking()
                    .space_handle("S0")
                    .expect("space"),
                approach: crate::ParkingApproachState::Approaching {
                    route: before_vehicle.route,
                    route_edge_index: 0,
                },
            })
        );
    }

    #[test]
    fn parking_arrival_is_one_shot_and_commit_excludes_vehicle_from_motion() {
        let (mut world, _) = parking_runtime_world();
        let vehicle = world.vehicle_handle("V0").expect("vehicle");
        let space = world.parking().space_handle("S0").expect("space");
        world
            .reserve_parking_space(vehicle, space)
            .expect("reservation");

        let mut arrival_event = None;
        for _ in 0..2_000 {
            let result = world.step(TickInput::new(20)).expect("approach step");
            let arrivals = result
                .events
                .iter()
                .filter_map(|event| match event {
                    CoreEvent::VehicleParkingArrivalReached(event) => Some(event.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert!(
                arrivals.len() <= 1,
                "one vehicle emits at most one arrival per tick"
            );
            if let Some(event) = arrivals.into_iter().next() {
                arrival_event = Some(event);
                break;
            }
        }

        let arrival = arrival_event.expect("vehicle must reach selected entry");
        assert_eq!(arrival.vehicle, vehicle);
        assert_eq!(arrival.space, space);
        assert_eq!(arrival.route_edge_index, 0);
        assert_eq!(
            world.parking_snapshot().vehicle_state(vehicle),
            Some(crate::VehicleParkingState::Reserved {
                space,
                approach: crate::ParkingApproachState::Arrived {
                    route: arrival.route,
                    route_edge_index: 0,
                },
            })
        );
        let arrived_state = world.vehicle(vehicle).expect("arrived vehicle").clone();
        assert_eq!(arrived_state.status, VehicleStatus::Active);
        assert_eq!(arrived_state.edge_progress.value(), 20.0);
        assert_eq!(arrived_state.current_speed, Speed::ZERO);

        let waiting = world.step(TickInput::new(20)).expect("waiting step");
        assert!(
            waiting
                .events
                .iter()
                .all(|event| !matches!(event, CoreEvent::VehicleParkingArrivalReached(_)))
        );
        assert_eq!(world.vehicle(vehicle), Some(&arrived_state));

        world
            .commit_parking(vehicle, space)
            .expect("commit parking");
        let parked_state = world.vehicle(vehicle).expect("parked vehicle").clone();
        assert_eq!(parked_state.status, VehicleStatus::Parked);
        assert_eq!(world.parking_snapshot().counts().occupied, 1);
        let parked_tick = world.step(TickInput::new(20)).expect("parked step");
        assert!(parked_tick.events.iter().all(|event| {
            !matches!(
                event,
                CoreEvent::VehicleParkingArrivalReached(_)
                    | CoreEvent::VehicleParkingStopProjectionApplied(_)
            )
        }));
        assert_eq!(world.vehicle(vehicle), Some(&parked_state));
    }

    #[test]
    fn dormant_route_completion_releases_before_completed_event_atomically() {
        let (mut world, _) = parking_runtime_world();
        let vehicle = world.vehicle_handle("V1").expect("vehicle");
        let space = world.parking().space_handle("S0").expect("space");
        world
            .reserve_parking_space(vehicle, space)
            .expect("dormant reservation");
        assert_eq!(
            world.parking_snapshot().vehicle_state(vehicle),
            Some(crate::VehicleParkingState::Reserved {
                space,
                approach: crate::ParkingApproachState::Dormant,
            })
        );

        let mut completion_events = None;
        for _ in 0..5_000 {
            let result = world.step(TickInput::new(20)).expect("dormant step");
            if world
                .vehicle(vehicle)
                .is_some_and(|state| state.status == VehicleStatus::Completed)
            {
                completion_events = Some(result.events);
                break;
            }
        }

        let events = completion_events.expect("dormant vehicle must complete route");
        let release_index = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    CoreEvent::ParkingReservationReleased(event)
                        if event.vehicle == vehicle
                            && event.space == space
                            && event.reason == ParkingReleaseReason::RouteCompleted
                )
            })
            .expect("completion release event");
        let completed_index = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    CoreEvent::VehicleCompletedRoute(event) if event.vehicle == vehicle
                )
            })
            .expect("completed event");
        assert_eq!(release_index + 1, completed_index);
        assert_eq!(world.parking_snapshot().counts().reserved, 0);
        assert_eq!(
            world.parking_snapshot().space_state(space),
            Some(ParkingSpaceState::Vacant)
        );
        assert_eq!(
            world.parking_snapshot().vehicle_state(vehicle),
            Some(crate::VehicleParkingState::Unbound)
        );
    }

    #[test]
    fn completion_release_candidate_is_discarded_on_later_step_failure_and_retry_replays() {
        let (mut world, _) = parking_runtime_world();
        let vehicle = world.vehicle_handle("V1").expect("vehicle");
        let space = world.parking().space_handle("S0").expect("space");
        let state = world.vehicles[vehicle.index()]
            .state
            .as_mut()
            .expect("vehicle state");
        state.edge_progress = EdgeProgress::try_new(199.9).expect("near route end");
        state.current_speed = Speed::try_new(10.0).expect("completion speed");
        world
            .reserve_parking_space(vehicle, space)
            .expect("dormant reservation");
        let mut fresh = world.clone();

        world.step_failure_after_vehicle = Some(vehicle);
        let before_failure = world.clone();
        let error = world
            .step(TickInput::new(20))
            .expect_err("injected post-advance failure");
        std::assert_matches!(
            error,
            CoreError::ParkingBindingInvariantViolation {
                stage: "test_after_vehicle_advance",
                vehicle: Some(actual_vehicle),
                space: Some(actual_space),
            } if actual_vehicle == vehicle && actual_space == space
        );
        assert_eq!(world, before_failure);
        assert_eq!(world.parking_snapshot().counts().reserved, 1);
        assert_eq!(
            world.parking_snapshot().space_state(space),
            Some(ParkingSpaceState::Reserved { vehicle })
        );

        world.step_failure_after_vehicle = None;
        let retry = world.step(TickInput::new(20)).expect("retry");
        let replay = fresh.step(TickInput::new(20)).expect("fresh replay");
        assert_eq!(retry, replay);
        assert_eq!(world, fresh);
        assert!(matches!(
            retry.events.as_slice(),
            [
                CoreEvent::ParkingReservationReleased(_),
                CoreEvent::VehicleCompletedRoute(_)
            ]
        ));
    }

    #[test]
    fn route_reference_equality_covers_live_count_but_ignores_container_capacity() {
        let mut left = RouteReferenceIndex::default();
        let mut right = RouteReferenceIndex::default();

        left.attach(VehicleHandle::new(3, 2), 17);
        assert_ne!(left, right, "live reference count is authority state");

        right.attach(VehicleHandle::new(4, 9), 23);
        left.by_update_position.reserve(64);
        assert_eq!(
            left, right,
            "derived container capacity must remain ignored"
        );
    }

    #[test]
    fn route_in_use_uses_stable_order_after_vehicle_slot_reuse() {
        let lane_graph = LaneGraph::try_new([LaneEdge::new(
            "A",
            EdgeLength::try_new(10.0).expect("valid edge length"),
            crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            Vec::<String>::new(),
        )])
        .expect("valid lane graph");
        let (traffic_data, profile) = traffic_data(
            lane_graph,
            [Route::try_new("R1", ["A"]).expect("valid route")],
        );
        let mut world =
            CoreWorld::with_traffic_data(20, traffic_data, Vec::new()).expect("valid world");
        let route = world.route_handle("R1").expect("route handle");
        let end = EdgeProgress::try_new(10.0).expect("valid end progress");
        let first = world
            .spawn_vehicle(VehicleSpawnInput::completed("first", profile, "R1", 0, end))
            .expect("first vehicle");
        let second = world
            .spawn_vehicle(VehicleSpawnInput::completed(
                "second", profile, "R1", 0, end,
            ))
            .expect("second vehicle");

        world.despawn_vehicle(first).expect("despawn first");
        let replacement = world
            .spawn_vehicle(VehicleSpawnInput::completed(
                "replacement",
                profile,
                "R1",
                0,
                end,
            ))
            .expect("replacement vehicle");

        assert_eq!(replacement.index(), first.index(), "slot must be reused");
        assert_eq!(
            world
                .vehicles()
                .map(|vehicle| vehicle.handle)
                .collect::<Vec<_>>(),
            vec![second, replacement]
        );
        let error = world.remove_route(route).expect_err("route remains in use");
        std::assert_matches!(
            error,
            CoreError::RouteInUse { vehicle, .. } if vehicle == second
        );
        world.assert_lifecycle_indices_consistent();
    }

    #[test]
    fn deterministic_tombstone_compaction_preserves_live_order_and_route_first() {
        let lane_graph = LaneGraph::try_new([LaneEdge::new(
            "A",
            EdgeLength::try_new(10.0).expect("valid edge length"),
            crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            Vec::<String>::new(),
        )])
        .expect("valid lane graph");
        let (traffic_data, profile) = traffic_data(
            lane_graph,
            [Route::try_new("R1", ["A"]).expect("valid route")],
        );
        let mut world =
            CoreWorld::with_traffic_data(20, traffic_data, Vec::new()).expect("valid world");
        let route = world.route_handle("R1").expect("route handle");
        let end = EdgeProgress::try_new(10.0).expect("valid end progress");
        let mut handles = Vec::new();
        for index in 0..130 {
            handles.push(
                world
                    .spawn_vehicle(VehicleSpawnInput::completed(
                        format!("V{index:03}"),
                        profile,
                        "R1",
                        0,
                        end,
                    ))
                    .expect("vehicle spawns"),
            );
        }
        for handle in handles.iter().take(65).copied() {
            world.despawn_vehicle(handle).expect("vehicle despawns");
        }

        assert_eq!(world.vehicle_update_order.tombstones, 0);
        assert_eq!(world.vehicle_update_order.entries.len(), 65);
        assert_eq!(
            world
                .vehicles()
                .map(|vehicle| vehicle.handle)
                .collect::<Vec<_>>(),
            handles[65..]
        );
        let error = world.remove_route(route).expect_err("route remains in use");
        std::assert_matches!(
            error,
            CoreError::RouteInUse { vehicle, .. } if vehicle == handles[65]
        );
        world.assert_lifecycle_indices_consistent();
    }

    proptest! {
        #[test]
        fn command_spatial_overlap_matches_full_scan_oracle(
            route_case in 0_usize..10,
            progress_value in 0_u8..=20,
            stopped in any::<bool>(),
        ) {
            let lane_graph = LaneGraph::try_new([
                LaneEdge::new(
                    "A",
                    EdgeLength::try_new(20.0).expect("length"),
                    crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
                    ["B", "C"],
                ),
                LaneEdge::new(
                    "B",
                    EdgeLength::try_new(20.0).expect("length"),
                    crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
                    ["D"],
                ),
                LaneEdge::new(
                    "C",
                    EdgeLength::try_new(20.0).expect("length"),
                    crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
                    ["D"],
                ),
                LaneEdge::new(
                    "D",
                    EdgeLength::try_new(20.0).expect("length"),
                    crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
                    ["A"],
                ),
            ])
            .expect("valid cyclic graph");
            let routes = [
                Route::try_new("R0", ["A", "B", "D", "A", "C", "D"]).expect("R0"),
                Route::try_new("R1", ["C", "D", "A", "B"]).expect("R1"),
            ];
            let (traffic_data, profile) = traffic_data(lane_graph, routes);
            let vehicles = vec![
                VehicleSpawnInput::active(
                    "existing-a",
                    profile,
                    "R0",
                    0,
                    EdgeProgress::try_new(2.0).expect("progress"),
                    Speed::ZERO,
                ),
                VehicleSpawnInput::stopped(
                    "existing-b",
                    profile,
                    "R0",
                    1,
                    EdgeProgress::try_new(9.0).expect("progress"),
                ),
                VehicleSpawnInput::active(
                    "existing-d",
                    profile,
                    "R0",
                    2,
                    EdgeProgress::try_new(15.0).expect("progress"),
                    Speed::ZERO,
                ),
                VehicleSpawnInput::stopped(
                    "existing-c",
                    profile,
                    "R1",
                    0,
                    EdgeProgress::try_new(11.0).expect("progress"),
                ),
            ];
            let mut world = CoreWorld::with_traffic_data(20, traffic_data, vehicles)
                .expect("oracle world");
            let cases = [
                ("R0", 0),
                ("R0", 1),
                ("R0", 2),
                ("R0", 3),
                ("R0", 4),
                ("R0", 5),
                ("R1", 0),
                ("R1", 1),
                ("R1", 2),
                ("R1", 3),
            ];
            let (route_id, route_edge_index) = cases[route_case];
            let progress = EdgeProgress::try_new(f64::from(progress_value)).expect("progress");
            let input = if stopped {
                VehicleSpawnInput::stopped(
                    "candidate",
                    profile,
                    route_id,
                    route_edge_index,
                    progress,
                )
            } else {
                VehicleSpawnInput::active(
                    "candidate",
                    profile,
                    route_id,
                    route_edge_index,
                    progress,
                    Speed::ZERO,
                )
            };
            let route = world.route_handle(route_id).expect("route handle");
            let normalized = world.normalize_vehicle_input(route, &input).expect("normalized");
            let expected = format!(
                "{:?}",
                world.validate_candidate_overlap_full_scan(route, &input.id, &normalized)
            );
            let actual = format!(
                "{:?}",
                world.validate_candidate_overlap(route, &input.id, &normalized)
            );

            prop_assert_eq!(actual, expected);
            world.assert_lifecycle_indices_consistent();
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        #[test]
        fn parking_leave_local_follower_query_matches_full_scan_oracle(
            follower_progress in 0_u8..=100,
            follower_speed in 0_u8..=20,
            stopped in any::<bool>(),
        ) {
            let (mut world, profile) = parking_runtime_world();
            for id in ["V0", "V1"] {
                let vehicle = world.vehicle_handle(id).expect("seed vehicle");
                world.despawn_vehicle(vehicle).expect("remove seed vehicle");
            }
            let space = world.parking().space_handle("S0").expect("space");
            let route = world.route_handle("R").expect("route");
            let parked = world
                .spawn_parked_vehicle(ParkedVehicleSpawnInput {
                    id: "parked".to_owned(),
                    profile,
                    route_id: "R".to_owned(),
                    route_edge_index: 0,
                    space,
                })
                .expect("parked vehicle")
                .vehicle;
            let progress = EdgeProgress::try_new(f64::from(follower_progress)).expect("progress");
            let follower_input = if stopped {
                VehicleSpawnInput::stopped("follower", profile, "R", 0, progress)
            } else {
                VehicleSpawnInput::active(
                    "follower",
                    profile,
                    "R",
                    0,
                    progress,
                    Speed::try_new(f64::from(follower_speed)).expect("speed"),
                )
            };
            world.spawn_vehicle(follower_input).expect("follower");
            let candidate = NormalizedVehicleInput {
                profile,
                route_edge_index: 0,
                edge_progress: EdgeProgress::try_new(40.0).expect("exit progress"),
                current_speed: Speed::ZERO,
                status: VehicleStatus::Active,
            };
            let expected = format!(
                "{:?}",
                world.validate_parking_leave_followers_full_scan(
                    parked,
                    space,
                    route,
                    0,
                    &candidate,
                )
            );
            let actual = format!(
                "{:?}",
                world.validate_parking_leave_followers(parked, space, route, 0, &candidate)
            );

            prop_assert_eq!(actual, expected);
            world.assert_lifecycle_indices_consistent();
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn parking_leave_follower_oracle_covers_adjacent_and_repeated_occurrences(
            follower_case in 0_usize..8,
            progress_seed in 0_u8..=99,
            follower_speed in 0_u8..=40,
            stopped in any::<bool>(),
        ) {
            let lane_graph = LaneGraph::try_new([
                LaneEdge::new(
                    "A",
                    EdgeLength::try_new(100.0).expect("length"),
                    crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
                    ["B", "C"],
                ),
                LaneEdge::new(
                    "B",
                    EdgeLength::try_new(100.0).expect("length"),
                    crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
                    ["D"],
                ),
                LaneEdge::new(
                    "C",
                    EdgeLength::try_new(100.0).expect("length"),
                    crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
                    ["D"],
                ),
                LaneEdge::new(
                    "D",
                    EdgeLength::try_new(100.0).expect("length"),
                    crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
                    ["A"],
                ),
            ])
            .expect("cyclic parking graph");
            let parking = ParkingRegistry::try_new(
                &lane_graph,
                [],
                [ParkingSpace::new(
                    "S",
                    None,
                    "A",
                    10.0,
                    "A",
                    20.0,
                    ParkingSpaceGeometry::new(-3.0, 0.0, 4.5, 2.4),
                )],
            )
            .expect("cyclic parking registry");
            let (base, profile) = traffic_data(
                lane_graph,
                [
                    Route::try_new("R0", ["A", "B", "D", "A", "C", "D"])
                        .expect("R0"),
                    Route::try_new("R1", ["C", "D", "A", "B"]).expect("R1"),
                ],
            );
            let traffic = InitialTrafficData::try_new(
                base.lane_graph().clone(),
                base.routes().cloned(),
                base.vehicle_profiles().clone(),
                crate::JunctionRegistry::empty(),
                SignalRegistry::empty(),
                parking,
                crate::test_support::test_participant_class_registry(),
                crate::CrossSectionRegistry::empty(),
                crate::AccessRegistry::empty(),
            )
            .expect("cyclic parking traffic");
            let mut world = CoreWorld::with_traffic_data(1_000, traffic, Vec::new())
                .expect("cyclic parking world");
            let space = world.parking().space_handle("S").expect("space");
            let route = world.route_handle("R0").expect("R0");
            let parked = world
                .spawn_parked_vehicle(ParkedVehicleSpawnInput {
                    id: "parked".to_owned(),
                    profile,
                    route_id: "R0".to_owned(),
                    route_edge_index: 0,
                    space,
                })
                .expect("parked vehicle")
                .vehicle;
            let cases = [
                ("R0", 0),
                ("R0", 2),
                ("R0", 3),
                ("R0", 5),
                ("R1", 0),
                ("R1", 1),
                ("R1", 2),
                ("R1", 3),
            ];
            let (route_id, route_edge_index) = cases[follower_case];
            let edge_id = world
                .lane_graph()
                .edge_external_id(
                    world.routes[world.route_handle(route_id).expect("route").index()]
                        .edge_handles[route_edge_index],
                )
                .expect("edge id");
            let progress = if matches!(edge_id, "B" | "C" | "D") {
                90.0 + f64::from(progress_seed % 10)
            } else {
                f64::from(progress_seed)
            };
            let progress = EdgeProgress::try_new(progress).expect("progress");
            let follower_input = if stopped {
                VehicleSpawnInput::stopped(
                    "follower",
                    profile,
                    route_id,
                    route_edge_index,
                    progress,
                )
            } else {
                VehicleSpawnInput::active(
                    "follower",
                    profile,
                    route_id,
                    route_edge_index,
                    progress,
                    Speed::try_new(f64::from(follower_speed)).expect("speed"),
                )
            };
            world.spawn_vehicle(follower_input).expect("follower");
            let candidate = NormalizedVehicleInput {
                profile,
                route_edge_index: 0,
                edge_progress: EdgeProgress::try_new(20.0).expect("exit progress"),
                current_speed: Speed::ZERO,
                status: VehicleStatus::Active,
            };
            let expected = format!(
                "{:?}",
                world.validate_parking_leave_followers_full_scan(
                    parked,
                    space,
                    route,
                    0,
                    &candidate,
                )
            );
            let actual = format!(
                "{:?}",
                world.validate_parking_leave_followers(parked, space, route, 0, &candidate)
            );

            prop_assert_eq!(actual, expected);
            world.assert_lifecycle_indices_consistent();
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn parking_reservation_commands_match_model_and_replay_deterministically(
            operations in prop::collection::vec(any::<u8>(), 1..=128),
        ) {
            let (mut world, _) = parking_runtime_world();
            let mut replay = world.clone();
            let vehicles = [
                world.vehicle_handle("V0").expect("V0"),
                world.vehicle_handle("V1").expect("V1"),
            ];
            let spaces = [
                world.parking().space_handle("S0").expect("S0"),
                world.parking().space_handle("S1").expect("S1"),
            ];
            let mut vehicle_spaces = [None; 2];
            let mut space_vehicles = [None; 2];

            for operation in operations {
                let vehicle_index = usize::from(operation & 1);
                let space_index = usize::from((operation >> 1) & 1);
                let vehicle = vehicles[vehicle_index];
                let space = spaces[space_index];
                if operation & 4 == 0 {
                    let actual = world.reserve_parking_space(vehicle, space);
                    let replayed = replay.reserve_parking_space(vehicle, space);
                    prop_assert_eq!(format!("{actual:?}"), format!("{replayed:?}"));
                    if vehicle_spaces[vehicle_index] == Some(space_index) {
                        prop_assert_eq!(
                            actual.expect("exact reservation retry").effect,
                            ParkingCommandEffect::AlreadySatisfied
                        );
                    } else if let Some(current_space) = vehicle_spaces[vehicle_index] {
                        assert!(matches!(
                            actual,
                            Err(CoreError::ParkingVehicleAlreadyBound {
                                current_space: actual_space,
                                ..
                            }) if actual_space == spaces[current_space]
                        ));
                    } else if let Some(current_vehicle) = space_vehicles[space_index] {
                        assert!(matches!(
                            actual,
                            Err(CoreError::ParkingSpaceUnavailable {
                                current_vehicle: actual_vehicle,
                                ..
                            }) if actual_vehicle == vehicles[current_vehicle]
                        ));
                    } else {
                        prop_assert_eq!(
                            actual.expect("vacant reservation").effect,
                            ParkingCommandEffect::Applied
                        );
                        vehicle_spaces[vehicle_index] = Some(space_index);
                        space_vehicles[space_index] = Some(vehicle_index);
                    }
                } else {
                    let actual = world.cancel_parking_reservation(vehicle, space);
                    let replayed = replay.cancel_parking_reservation(vehicle, space);
                    prop_assert_eq!(format!("{actual:?}"), format!("{replayed:?}"));
                    if vehicle_spaces[vehicle_index] == Some(space_index) {
                        prop_assert_eq!(
                            actual.expect("exact cancellation").effect,
                            ParkingCommandEffect::Applied
                        );
                        vehicle_spaces[vehicle_index] = None;
                        space_vehicles[space_index] = None;
                    } else if vehicle_spaces[vehicle_index].is_none()
                        && space_vehicles[space_index].is_none()
                    {
                        prop_assert_eq!(
                            actual.expect("vacant cancellation retry").effect,
                            ParkingCommandEffect::AlreadySatisfied
                        );
                    } else {
                        assert!(matches!(
                            actual,
                            Err(CoreError::ParkingReservationMismatch { .. })
                        ));
                    }
                }

                prop_assert_eq!(&world, &replay);
                let expected_reserved = vehicle_spaces.iter().flatten().count();
                let counts = world.parking_snapshot().counts();
                prop_assert_eq!(counts.reserved, expected_reserved);
                prop_assert_eq!(counts.vacant, spaces.len() - expected_reserved);
                prop_assert_eq!(counts.occupied, 0);
                for (space_index, space) in spaces.iter().copied().enumerate() {
                    let expected = space_vehicles[space_index].map_or(
                        ParkingSpaceState::Vacant,
                        |vehicle_index| ParkingSpaceState::Reserved {
                            vehicle: vehicles[vehicle_index],
                        },
                    );
                    prop_assert_eq!(world.parking_snapshot().space_state(space), Some(expected));
                }
                for (vehicle_index, vehicle) in vehicles.iter().copied().enumerate() {
                    match vehicle_spaces[vehicle_index] {
                        Some(space_index) => assert!(matches!(
                            world.parking_snapshot().vehicle_state(vehicle),
                            Some(VehicleParkingState::Reserved { space, .. })
                                if space == spaces[space_index]
                        )),
                        None => prop_assert_eq!(
                            world.parking_snapshot().vehicle_state(vehicle),
                            Some(VehicleParkingState::Unbound)
                        ),
                    }
                }
                world.assert_lifecycle_indices_consistent();
                replay.assert_lifecycle_indices_consistent();
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn lifecycle_order_and_route_references_match_vec_model(
            operations in prop::collection::vec(any::<u8>(), 1..=128),
        ) {
            let lane_graph = LaneGraph::try_new([LaneEdge::new(
                "A",
                EdgeLength::try_new(100.0).expect("length"),
                crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
                Vec::<String>::new(),
            )])
            .expect("graph");
            let (traffic_data, profile) = traffic_data(
                lane_graph,
                [
                    Route::try_new("R0", ["A"]).expect("R0"),
                    Route::try_new("R1", ["A"]).expect("R1"),
                ],
            );
            let mut world = CoreWorld::with_traffic_data(20, traffic_data, Vec::new())
                .expect("world");
            let routes = [
                world.route_handle("R0").expect("R0 handle"),
                world.route_handle("R1").expect("R1 handle"),
            ];
            let end = EdgeProgress::try_new(100.0).expect("end progress");
            let mut model = Vec::<(usize, VehicleHandle, usize)>::new();
            let mut last_handles = [None; 16];

            for operation in operations {
                let id_index = usize::from(operation) % last_handles.len();
                let route_index = id_index % routes.len();
                let id = format!("V{id_index:02}");
                if operation % 3 != 2 {
                    let before = world.clone();
                    let result = world.spawn_vehicle(VehicleSpawnInput::completed(
                        id.clone(),
                        profile,
                        format!("R{route_index}"),
                        0,
                        end,
                    ));
                    if let Some((_, expected, _)) =
                        model.iter().find(|(candidate, _, _)| *candidate == id_index)
                    {
                        let error = result.expect_err("duplicate model vehicle");
                        std::assert_matches!(
                            error,
                            CoreError::DuplicateVehicleId { vehicle_id } if vehicle_id == id
                        );
                        assert_eq!(world, before);
                        assert_eq!(world.vehicle_handle(&id), Some(*expected));
                    } else {
                        let handle = result.expect("model spawn");
                        last_handles[id_index] = Some(handle);
                        model.push((id_index, handle, route_index));
                    }
                } else if let Some(position) = model
                    .iter()
                    .position(|(candidate, _, _)| *candidate == id_index)
                {
                    let (_, handle, expected_route) = model.remove(position);
                    let record = world.despawn_vehicle(handle).expect("model despawn");
                    assert_eq!(record.handle, handle);
                    assert_eq!(record.external_id, id);
                    assert_eq!(record.route, routes[expected_route]);
                    assert_eq!(record.status, VehicleStatus::Completed);
                    assert_eq!(world.vehicle(handle), None);
                } else {
                    let stale = last_handles[id_index]
                        .unwrap_or_else(|| VehicleHandle::new(1_000 + id_index, 0));
                    let before = world.clone();
                    let error = world
                        .despawn_vehicle(stale)
                        .expect_err("missing model vehicle");
                    std::assert_matches!(
                        error,
                        CoreError::UnknownVehicleHandle { vehicle } if vehicle == stale
                    );
                    assert_eq!(world, before);
                }

                let expected_order = model
                    .iter()
                    .map(|(_, handle, _)| *handle)
                    .collect::<Vec<_>>();
                assert_eq!(
                    world
                        .vehicles()
                        .map(|vehicle| vehicle.handle)
                        .collect::<Vec<_>>(),
                    expected_order
                );
                for (model_id, handle, _) in &model {
                    assert_eq!(world.vehicle_handle(&format!("V{model_id:02}")), Some(*handle));
                }
                for (route_index, route) in routes.iter().copied().enumerate() {
                    if let Some((_, expected, _)) = model
                        .iter()
                        .find(|(_, _, candidate_route)| *candidate_route == route_index)
                    {
                        let error = world
                            .remove_route(route)
                            .expect_err("referenced model route");
                        std::assert_matches!(
                            error,
                            CoreError::RouteInUse { vehicle, .. } if vehicle == *expected
                        );
                    }
                }
                world.assert_lifecycle_indices_consistent();
            }
        }
    }

    #[test]
    fn lifecycle_10k_tombstone_high_water_keeps_exact_route_references() {
        let mut world = lifecycle_scale_world(10_000);
        let handles = world
            .vehicles()
            .map(|vehicle| vehicle.handle)
            .collect::<Vec<_>>();
        let initial = world.lifecycle_retained_stats();
        assert_eq!(initial.live_vehicles, 10_000);
        assert_eq!(initial.route_occurrences, 1);
        assert_eq!(initial.route_candidate_nodes, 10_000);
        assert_eq!(initial.stale_route_candidate_nodes, 0);
        assert_eq!(initial.spatial_occupants, 10_000);

        for handle in handles.iter().take(4_999).copied() {
            world
                .despawn_vehicle(handle)
                .expect("pre-threshold despawn");
        }
        let high_water = world.lifecycle_retained_stats();
        assert_eq!(high_water.live_vehicles, 5_001);
        assert_eq!(high_water.tombstones, 4_999);
        assert_eq!(high_water.route_candidate_nodes, 5_001);
        assert_eq!(high_water.stale_route_candidate_nodes, 0);
        assert_eq!(high_water.spatial_occupants, 5_001);

        world
            .despawn_vehicle(handles[4_999])
            .expect("threshold despawn");
        let compacted = world.lifecycle_retained_stats();
        assert_eq!(compacted.live_vehicles, 5_000);
        assert_eq!(compacted.tombstones, 0);
        assert_eq!(compacted.route_candidate_nodes, 5_000);
        assert_eq!(compacted.stale_route_candidate_nodes, 0);
        assert_eq!(compacted.spatial_occupants, 5_000);
        assert!(compacted.accounted_bytes >= compacted.live_vehicles);
        world.assert_lifecycle_indices_consistent();
    }

    #[test]
    fn spatial_operation_counts_depend_on_local_k_not_background_v() {
        let run = |background_count| {
            let (mut world, profile) = sparse_command_world(background_count);
            let route = world.route_handle("R").expect("route");
            let progress = EdgeProgress::try_new(5.0).expect("progress");
            let input =
                VehicleSpawnInput::active("candidate", profile, "R", 0, progress, Speed::ZERO);
            let normalized = world
                .normalize_vehicle_input(route, &input)
                .expect("normalized");
            world
                .validate_candidate_overlap(route, &input.id, &normalized)
                .expect_err("local overlap");
            let overlap_stats = world.command_spatial_index.query_stats();

            let candidate_edge = world.routes[route.index()].edge_handles[0];
            let mut spatial = std::mem::take(&mut world.command_spatial_index);
            let mut resolve_progress = |handle| {
                world
                    .vehicle(handle)
                    .expect("spatial occupant")
                    .edge_progress
                    .value()
            };
            spatial.gather_direct_follower_candidates(
                candidate_edge,
                progress.value(),
                world
                    .vehicle_profile(profile)
                    .expect("profile")
                    .iidm()
                    .length,
                world.vehicles.len(),
                &mut resolve_progress,
            );
            let direct_followers = spatial.candidates().to_vec();
            let direct_stats = spatial.query_stats();
            world.command_spatial_index = spatial;
            (
                overlap_stats,
                direct_stats,
                direct_followers,
                world.vehicle_handle("local").expect("local handle"),
            )
        };

        let small = run(128);
        let large = run(10_000);
        assert_eq!(small.0, large.0);
        assert_eq!(small.1, large.1);
        assert_eq!(small.0.edge_ranges, 2);
        assert_eq!(small.0.occupants_visited, 2);
        assert_eq!(small.1.edge_ranges, 1);
        assert_eq!(small.1.occupants_visited, 1);
        assert_eq!(small.2, vec![small.3]);
        assert_eq!(large.2, vec![large.3]);
    }

    #[test]
    fn replace_overlap_query_counts_depend_on_local_k_not_background_v() {
        let run = |background_count| {
            let (mut world, profile) = sparse_command_world(background_count);
            let route = world.route_handle("R").expect("route");
            let edge = world.routes[route.index()].edge_handles[0];
            let route_end = world
                .lane_graph
                .edge_length(edge)
                .expect("route edge")
                .value();
            let old = world
                .spawn_vehicle(VehicleSpawnInput::completed(
                    "replace-old",
                    profile,
                    "R",
                    0,
                    EdgeProgress::try_new(route_end).expect("route end"),
                ))
                .expect("completed vehicle");
            let input = VehicleReplaceInput::new(
                VehicleReplaceExternalId::Preserve,
                profile,
                route,
                0,
                EdgeProgress::try_new(5.0).expect("replacement progress"),
                Speed::ZERO,
            );

            let outcome = world
                .replace_completed_vehicle(old, &input)
                .expect("overlap is recoverable");
            assert!(matches!(outcome, VehicleReplaceOutcome::Blocked(_)));
            world.command_spatial_index.query_stats()
        };

        let small = run(128);
        let large = run(10_000);
        assert_eq!(large, small);
        assert!(large.occupants_visited <= 2);
    }

    #[test]
    fn parking_leave_follower_query_counts_depend_on_local_k_not_background_v() {
        let run = |background_count| {
            let edge_length = 10.0 * background_count as f64 + 2_000.0;
            let lane_graph = LaneGraph::try_new([LaneEdge::new(
                "A",
                EdgeLength::try_new(edge_length).expect("parking scale edge"),
                crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
                Vec::<String>::new(),
            )])
            .expect("parking scale graph");
            let parking = ParkingRegistry::try_new(
                &lane_graph,
                [],
                [ParkingSpace::new(
                    "S",
                    None,
                    "A",
                    20.0,
                    "A",
                    40.0,
                    ParkingSpaceGeometry::new(-3.0, 0.0, 4.5, 2.4),
                )],
            )
            .expect("parking scale registry");
            let (base, profile) = traffic_data(
                lane_graph,
                [Route::try_new("R", ["A"]).expect("parking scale route")],
            );
            let traffic = InitialTrafficData::try_new(
                base.lane_graph().clone(),
                base.routes().cloned(),
                base.vehicle_profiles().clone(),
                crate::JunctionRegistry::empty(),
                SignalRegistry::empty(),
                parking,
                crate::test_support::test_participant_class_registry(),
                crate::CrossSectionRegistry::empty(),
                crate::AccessRegistry::empty(),
            )
            .expect("parking scale traffic");
            let mut vehicles: Vec<_> = (0..background_count)
                .map(|index| {
                    VehicleSpawnInput::active(
                        format!("B{index:06}"),
                        profile,
                        "R",
                        0,
                        EdgeProgress::try_new(1_000.0 + 10.0 * index as f64)
                            .expect("background progress"),
                        Speed::ZERO,
                    )
                })
                .collect::<Vec<_>>();
            vehicles.push(VehicleSpawnInput::active(
                "local",
                profile,
                "R",
                0,
                EdgeProgress::try_new(35.4).expect("local progress"),
                Speed::try_new(10.0).expect("local speed"),
            ));
            let mut world =
                CoreWorld::with_traffic_data(20, traffic, vehicles).expect("parking scale world");
            let space = world.parking().space_handle("S").expect("space");
            let route = world.route_handle("R").expect("route");
            let parked = world
                .spawn_parked_vehicle(ParkedVehicleSpawnInput {
                    id: "parked".to_owned(),
                    profile,
                    route_id: "R".to_owned(),
                    route_edge_index: 0,
                    space,
                })
                .expect("parked spawn")
                .vehicle;
            let candidate = NormalizedVehicleInput {
                profile,
                route_edge_index: 0,
                edge_progress: EdgeProgress::try_new(40.0).expect("exit progress"),
                current_speed: Speed::ZERO,
                status: VehicleStatus::Active,
            };
            std::assert_matches!(
                world.validate_parking_leave_followers(parked, space, route, 0, &candidate),
                Err(CoreError::ParkingLeaveUnsafeFollower { .. })
            );
            world.command_spatial_index.query_stats()
        };

        let small = run(128);
        let large = run(10_000);
        assert_eq!(small, large);
        assert_eq!(small.edge_ranges, 1);
        assert_eq!(small.occupants_visited, 1);
    }

    #[test]
    fn parking_leave_stale_max_speed_profile_exposes_reverse_horizon_work() {
        let run = |background_count: usize| {
            let edge_length = 10.0 * background_count as f64 + 100.0;
            let exit_progress = edge_length - 10.0;
            let lane_graph = LaneGraph::try_new([
                LaneEdge::new(
                    "A",
                    EdgeLength::try_new(edge_length).expect("pathological edge length"),
                    crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
                    Vec::<String>::new(),
                ),
                LaneEdge::new(
                    "C",
                    EdgeLength::try_new(100.0).expect("fast edge length"),
                    crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
                    Vec::<String>::new(),
                ),
            ])
            .expect("pathological graph");
            let parking = ParkingRegistry::try_new(
                &lane_graph,
                [],
                [ParkingSpace::new(
                    "S",
                    None,
                    "A",
                    exit_progress - 10.0,
                    "A",
                    exit_progress,
                    ParkingSpaceGeometry::new(-3.0, 0.0, 4.5, 2.4),
                )],
            )
            .expect("pathological parking registry");
            let (base, profile) = traffic_data(
                lane_graph,
                [
                    Route::try_new("R", ["A"]).expect("pathological route"),
                    Route::try_new("fast-route", ["C"]).expect("fast route"),
                ],
            );
            let traffic = InitialTrafficData::try_new(
                base.lane_graph().clone(),
                base.routes().cloned(),
                base.vehicle_profiles().clone(),
                crate::JunctionRegistry::empty(),
                SignalRegistry::empty(),
                parking,
                crate::test_support::test_participant_class_registry(),
                crate::CrossSectionRegistry::empty(),
                crate::AccessRegistry::empty(),
            )
            .expect("pathological traffic");
            let mut vehicles: Vec<_> = (0..background_count)
                .map(|index| {
                    VehicleSpawnInput::active(
                        format!("B{index:06}"),
                        profile,
                        "R",
                        0,
                        EdgeProgress::try_new(5.0 + 10.0 * index as f64)
                            .expect("background progress"),
                        Speed::ZERO,
                    )
                })
                .collect();
            vehicles.push(VehicleSpawnInput::active(
                "fast-1",
                profile,
                "fast-route",
                0,
                EdgeProgress::try_new(50.0).expect("fast progress"),
                Speed::try_new(10_000_000.0).expect("fast speed"),
            ));
            vehicles.push(VehicleSpawnInput::active(
                "fast-2",
                profile,
                "fast-route",
                0,
                EdgeProgress::try_new(70.0).expect("second fast progress"),
                Speed::try_new(9_000_000.0).expect("second fast speed"),
            ));
            let mut world =
                CoreWorld::with_traffic_data(20, traffic, vehicles).expect("pathological world");
            let fast = world.vehicle_handle("fast-1").expect("fast vehicle");
            world
                .despawn_vehicle(fast)
                .expect("removing the fastest vehicle refreshes command max speed");
            let second_fast = world.vehicle_handle("fast-2").expect("second fast vehicle");
            world
                .despawn_vehicle(second_fast)
                .expect("same command batch reuses the exact max heap");
            let space = world.parking().space_handle("S").expect("space");
            let route = world.route_handle("R").expect("route");
            let parked = world
                .spawn_parked_vehicle(ParkedVehicleSpawnInput {
                    id: "parked".to_owned(),
                    profile,
                    route_id: "R".to_owned(),
                    route_edge_index: 0,
                    space,
                })
                .expect("parked spawn")
                .vehicle;
            let candidate = NormalizedVehicleInput {
                profile,
                route_edge_index: 0,
                edge_progress: EdgeProgress::try_new(exit_progress).expect("exit progress"),
                current_speed: Speed::ZERO,
                status: VehicleStatus::Active,
            };
            world
                .validate_parking_leave_followers(parked, space, route, 0, &candidate)
                .expect("zero-speed direct follower remains safe");
            (
                world.command_spatial_index.query_stats(),
                world.command_spatial_index.speed_heap_rebuilds(),
            )
        };

        let small = run(128);
        let large = run(10_000);
        assert_eq!(small, large);
        assert_eq!(small.0.edge_ranges, 1);
        assert_eq!(small.0.occupants_visited, 0);
        assert_eq!(small.1, 1, "one command batch builds the max heap once");
    }

    #[test]
    #[ignore = "100k retained-memory scaling is an explicit G3 validation"]
    fn lifecycle_retained_memory_10k_to_100k_is_linear() {
        let small = lifecycle_scale_world(10_000).lifecycle_retained_stats();
        let large = lifecycle_scale_world(100_000).lifecycle_retained_stats();
        assert_eq!(small.route_occurrences, large.route_occurrences);
        assert_eq!(small.tombstones, 0);
        assert_eq!(large.tombstones, 0);
        assert_eq!(small.stale_route_candidate_nodes, 0);
        assert_eq!(large.stale_route_candidate_nodes, 0);
        assert_eq!(small.spatial_occupants, 10_000);
        assert_eq!(large.spatial_occupants, 100_000);
        assert!(
            large.accounted_bytes <= small.accounted_bytes * 12,
            "retained bytes must scale <=12x: small={small:?}, large={large:?}"
        );
        eprintln!(
            "retained_memory small_bytes={} small_bytes_per_live={:.2} large_bytes={} large_bytes_per_live={:.2} ratio={:.4}",
            small.accounted_bytes,
            small.accounted_bytes as f64 / small.live_vehicles as f64,
            large.accounted_bytes,
            large.accounted_bytes as f64 / large.live_vehicles as f64,
            large.accounted_bytes as f64 / small.accounted_bytes as f64,
        );
    }

    #[test]
    #[ignore = "10k component memory is an explicit #122 research measurement"]
    fn numeric_component_memory_baseline_10k() {
        let mut world = lifecycle_scale_world(10_000);
        world
            .step(TickInput::new(world.fixed_delta_time_ms()))
            .expect("component memory warm-up step");
        let stats = world.lifecycle_retained_stats();
        assert_eq!(stats.live_vehicles, 10_000);
        assert!(stats.expanded_accounted_bytes >= stats.accounted_bytes);
        assert!(stats.complete_accounted_bytes >= stats.expanded_accounted_bytes);
        eprintln!(
            "numeric_component_memory live={} accounted_bytes={} expanded_accounted_bytes={} complete_accounted_bytes={} owned_heap_bytes={} world_inline_bytes={} lane_graph_bytes={} vehicle_profile_registry_bytes={} junction_registry_bytes={} signal_registry_bytes={} signal_runtime_state_bytes={} signal_runtime_scratch_bytes={} participant_class_registry_bytes={} cross_section_registry_bytes={} access_registry_bytes={} waiting_registry_bytes={} route_bytes={} route_maneuver_occurrence_bytes={} route_gate_occurrence_bytes={} route_waiting_zone_occurrence_bytes={} route_distance_bytes={} route_reference_bytes={} vehicle_bytes={} resolver_bytes={} free_list_bytes={} vehicle_order_bytes={} candidate_state_bytes={} parking_bytes={} parking_registry_runtime_bytes={} occupancy_scratch_bytes={} longitudinal_scratch_bytes={} command_spatial_bytes={} lane_graph_inline_size={} vehicle_profile_registry_inline_size={} junction_registry_inline_size={} signal_registry_inline_size={} signal_runtime_state_inline_size={} signal_runtime_scratch_inline_size={} vehicle_state_size={} vehicle_slot_size={}",
            stats.live_vehicles,
            stats.accounted_bytes,
            stats.expanded_accounted_bytes,
            stats.complete_accounted_bytes,
            stats.owned_heap_bytes,
            stats.world_inline_bytes,
            stats.lane_graph_bytes,
            stats.vehicle_profile_registry_bytes,
            stats.junction_registry_bytes,
            stats.signal_registry_bytes,
            stats.signal_runtime_state_bytes,
            stats.signal_runtime_scratch_bytes,
            stats.participant_class_registry_bytes,
            stats.cross_section_registry_bytes,
            stats.access_registry_bytes,
            stats.waiting_registry_bytes,
            stats.route_bytes,
            stats.route_maneuver_occurrence_bytes,
            stats.route_gate_occurrence_bytes,
            stats.route_waiting_zone_occurrence_bytes,
            stats.route_distance_bytes,
            stats.route_reference_bytes,
            stats.vehicle_bytes,
            stats.resolver_bytes,
            stats.free_list_bytes,
            stats.vehicle_order_bytes,
            stats.candidate_state_bytes,
            stats.parking_bytes,
            stats.parking_registry_runtime_bytes,
            stats.occupancy_scratch_bytes,
            stats.longitudinal_scratch_bytes,
            stats.command_spatial_bytes,
            stats.lane_graph_inline_size,
            stats.vehicle_profile_registry_inline_size,
            stats.junction_registry_inline_size,
            stats.signal_registry_inline_size,
            stats.signal_runtime_state_inline_size,
            stats.signal_runtime_scratch_inline_size,
            stats.vehicle_state_size,
            stats.vehicle_slot_size,
        );
    }

    #[test]
    fn complete_retained_accountant_sums_each_unique_owner_once() {
        assert_eq!(JunctionRegistry::empty().retained_bytes(), 0);
        let mut world = lifecycle_scale_world(128);
        world
            .step(TickInput::new(world.fixed_delta_time_ms()))
            .expect("retained accountant warm-up step");
        let stats = world.lifecycle_retained_stats();
        let expected_heap_bytes = stats.complete_components().owned_heap_bytes();

        assert_eq!(stats.owned_heap_bytes, expected_heap_bytes);
        assert_eq!(
            stats.complete_accounted_bytes,
            stats.world_inline_bytes + expected_heap_bytes
        );
        assert_eq!(stats.world_inline_bytes, std::mem::size_of::<CoreWorld>());
        assert_eq!(
            stats.lane_graph_inline_size,
            std::mem::size_of::<LaneGraph>()
        );
        assert_eq!(
            stats.vehicle_profile_registry_inline_size,
            std::mem::size_of::<VehicleProfileRegistry>()
        );
        assert_eq!(
            stats.junction_registry_inline_size,
            std::mem::size_of::<JunctionRegistry>()
        );
        assert_eq!(
            stats.signal_registry_inline_size,
            std::mem::size_of::<SignalRegistry>()
        );
        assert_eq!(
            stats.signal_runtime_state_inline_size,
            std::mem::size_of::<SignalRuntimeState>()
        );
        assert_eq!(
            stats.signal_runtime_scratch_inline_size,
            std::mem::size_of::<SignalRuntimeScratch>()
        );
        assert!(stats.lane_graph_bytes > 0);
        assert!(stats.vehicle_profile_registry_bytes > 0);
        assert!(stats.junction_registry_bytes > 0);
        assert_eq!(stats.route_maneuver_occurrence_bytes, 0);
    }

    #[test]
    #[ignore = "100k Parking retained-memory scaling is an explicit G3 validation"]
    fn parking_retained_memory_10k_to_100k_is_linear() {
        let small = parking_retained_scale_world(10_000).lifecycle_retained_stats();
        let large = parking_retained_scale_world(100_000).lifecycle_retained_stats();
        assert!(small.parking_bytes > 0);
        assert!(
            large.parking_bytes <= small.parking_bytes * 12,
            "Parking retained bytes must scale <=12x: small={small:?}, large={large:?}"
        );
        eprintln!(
            "parking_retained_memory small_bytes={} large_bytes={} ratio={:.4}",
            small.parking_bytes,
            large.parking_bytes,
            large.parking_bytes as f64 / small.parking_bytes as f64,
        );
    }

    #[test]
    fn command_spatial_membership_follows_committed_physical_edge_transition() {
        let lane_graph = LaneGraph::try_new([
            LaneEdge::new(
                "A",
                EdgeLength::try_new(10.0).expect("length"),
                crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
                ["B"],
            ),
            LaneEdge::new(
                "B",
                EdgeLength::try_new(100.0).expect("length"),
                crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
                Vec::<String>::new(),
            ),
        ])
        .expect("graph");
        let (traffic_data, profile) = traffic_data(
            lane_graph,
            [Route::try_new("R1", ["A", "B"]).expect("route")],
        );
        let vehicle = VehicleSpawnInput::active(
            "existing",
            profile,
            "R1",
            0,
            EdgeProgress::try_new(9.9).expect("progress"),
            Speed::try_new(10.0).expect("speed"),
        );
        let mut world =
            CoreWorld::with_traffic_data(20, traffic_data, vec![vehicle]).expect("world");
        let existing = world.vehicle_handle("existing").expect("handle");

        world.step(TickInput::new(20)).expect("transition step");
        let state = world.vehicle(existing).expect("state").clone();
        assert_eq!(state.route_edge_index, 1);
        let candidate = VehicleSpawnInput::active(
            "candidate",
            profile,
            "R1",
            1,
            state.edge_progress,
            Speed::ZERO,
        );
        let error = world
            .spawn_vehicle(candidate)
            .expect_err("new edge occupant must be found");

        std::assert_matches!(
            error,
            CoreError::VehiclePhysicalOverlap {
                follower_id,
                leader_id,
                ..
            } if follower_id == "candidate" && leader_id == "existing"
        );
        world.assert_lifecycle_indices_consistent();
    }

    #[test]
    fn exhausted_route_generation_retires_slot_without_reviving_stale_handle() {
        let lane_graph = LaneGraph::try_new([LaneEdge::new(
            "A",
            EdgeLength::try_new(10.0).expect("valid edge length"),
            crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            Vec::<String>::new(),
        )])
        .expect("valid lane graph");
        let (traffic_data, _) = traffic_data(
            lane_graph,
            [Route::try_new("R1", ["A"]).expect("valid route")],
        );
        let mut world =
            CoreWorld::with_traffic_data(20, traffic_data, Vec::new()).expect("valid world");
        let original = world.route_handle("R1").expect("route handle exists");
        let exhausted = RouteHandle::new(original.index(), u32::MAX);
        world.routes[original.index()].generation = u32::MAX;
        world.route_handles.insert("R1".to_owned(), exhausted);

        world
            .remove_route(exhausted)
            .expect("exhausted route slot can be removed");

        assert!(world.free_route_indices.is_empty());
        assert_eq!(world.route_external_id(exhausted), None);
        let replacement = world
            .register_route(Route::try_new("R1", ["A"]).expect("valid replacement route"))
            .expect("replacement route registers");
        assert_ne!(replacement.index(), exhausted.index());
        assert_eq!(world.route_external_id(exhausted), None);
    }

    #[test]
    fn exhausted_vehicle_generation_retires_slot_without_reviving_stale_handle() {
        let lane_graph = LaneGraph::try_new([LaneEdge::new(
            "A",
            EdgeLength::try_new(10.0).expect("valid edge length"),
            crate::graph::SpeedLimit::try_new(f64::MAX).expect("speed limit"),
            Vec::<String>::new(),
        )])
        .expect("valid lane graph");
        let (traffic_data, profile) = traffic_data(
            lane_graph,
            [Route::try_new("R1", ["A"]).expect("valid route")],
        );
        let mut world =
            CoreWorld::with_traffic_data(20, traffic_data, Vec::new()).expect("valid world");
        let original = world
            .spawn_vehicle(VehicleSpawnInput::active(
                "V1",
                profile,
                "R1",
                0,
                EdgeProgress::ZERO,
                Speed::ZERO,
            ))
            .expect("vehicle spawns");
        let exhausted = VehicleHandle::new(original.index(), u32::MAX);
        let position = world.vehicles[original.index()]
            .update_order_position
            .expect("live vehicle has update position");
        world.vehicles[original.index()].generation = u32::MAX;
        world.vehicles[original.index()]
            .state
            .as_mut()
            .expect("live vehicle state")
            .handle = exhausted;
        world.vehicle_update_order.entries[position] = Some(exhausted);
        world.vehicle_handles.insert("V1".to_owned(), exhausted);
        world.rebuild_all_route_reference_indices();
        world.rebuild_command_spatial_index();

        world
            .despawn_vehicle(exhausted)
            .expect("exhausted vehicle slot can be removed");

        assert!(world.free_vehicle_indices.is_empty());
        assert_eq!(world.vehicle_external_id(exhausted), None);
        let replacement = world
            .spawn_vehicle(VehicleSpawnInput::active(
                "V1",
                profile,
                "R1",
                0,
                EdgeProgress::ZERO,
                Speed::ZERO,
            ))
            .expect("replacement vehicle spawns");
        assert_ne!(replacement.index(), exhausted.index());
        assert_eq!(world.vehicle_external_id(exhausted), None);
    }

    #[test]
    fn replace_generation_exhaustion_retires_old_slot_and_preserves_update_position() {
        let (mut world, profile, route) = completed_replace_world();
        let original = world.vehicle_handle("V").expect("original vehicle");
        let exhausted = VehicleHandle::new(original.index(), u32::MAX);
        let position = world.vehicles[original.index()]
            .update_order_position
            .expect("live vehicle has update position");
        world.vehicles[original.index()].generation = u32::MAX;
        world.vehicles[original.index()]
            .state
            .as_mut()
            .expect("live vehicle state")
            .handle = exhausted;
        world.vehicle_update_order.entries[position] = Some(exhausted);
        world.vehicle_handles.insert("V".to_owned(), exhausted);
        world.rebuild_all_route_reference_indices();

        let input = VehicleReplaceInput::new(
            VehicleReplaceExternalId::Preserve,
            profile,
            route,
            0,
            EdgeProgress::ZERO,
            Speed::ZERO,
        );
        let VehicleReplaceOutcome::Replaced(record) = world
            .replace_completed_vehicle(exhausted, &input)
            .expect("exhausted replacement succeeds")
        else {
            panic!("replacement must succeed")
        };

        assert_ne!(record.new.index(), exhausted.index());
        assert_eq!(world.vehicle(exhausted), None);
        assert_eq!(world.vehicle_handle("V"), Some(record.new));
        assert_eq!(
            world.vehicle_update_order.entries[position],
            Some(record.new)
        );
        assert!(world.free_vehicle_indices.is_empty());
        world.assert_lifecycle_indices_consistent();
    }

    #[test]
    fn replace_rejects_corrupt_completed_parking_binding_atomically() {
        let mut world = reserved_parking_world();
        let vehicle = world.vehicle_handle("V0").expect("reserved vehicle");
        let state = world.vehicles[vehicle.index()]
            .state
            .as_mut()
            .expect("reserved state");
        state.status = VehicleStatus::Completed;
        state.current_speed = Speed::ZERO;
        state.applied_acceleration = Acceleration::ZERO;
        let profile = state.profile;
        let route = state.route;
        let input = VehicleReplaceInput::new(
            VehicleReplaceExternalId::Preserve,
            profile,
            route,
            0,
            EdgeProgress::ZERO,
            Speed::ZERO,
        );
        let before = world.clone();

        let error = world
            .replace_completed_vehicle(vehicle, &input)
            .expect_err("bound Completed vehicle is an invariant failure");
        std::assert_matches!(
            error,
            CoreError::ParkingBindingInvariantViolation {
                stage: "replace_completed_vehicle",
                vehicle: Some(actual),
                ..
            } if actual == vehicle
        );
        assert_eq!(world, before);
    }

    #[test]
    fn replace_failure_after_prepare_keeps_authority_state_unchanged() {
        let (mut world, profile, route) = completed_replace_world();
        let old = world.vehicle_handle("V").expect("old vehicle");
        let input = VehicleReplaceInput::new(
            VehicleReplaceExternalId::Preserve,
            profile,
            route,
            0,
            EdgeProgress::ZERO,
            Speed::ZERO,
        );
        world.replace_failure_after_prepare = true;
        let before = world.clone();

        let error = world
            .replace_completed_vehicle(old, &input)
            .expect_err("injected post-prepare failure");
        std::assert_matches!(
            error,
            CoreError::ParkingBindingInvariantViolation {
                stage: "test_after_vehicle_replace_prepare",
                vehicle: Some(actual),
                space: None,
            } if actual == old
        );
        assert_eq!(world, before);
    }

    #[test]
    fn repeated_replace_replays_and_retained_memory_stays_bounded() {
        let (mut world, profile, route) = completed_replace_world();
        let mut replay = world.clone();
        let input = VehicleReplaceInput::new(
            VehicleReplaceExternalId::Preserve,
            profile,
            route,
            0,
            EdgeProgress::ZERO,
            Speed::try_new(10.0).expect("replacement speed"),
        );
        let mut old = world.vehicle_handle("V").expect("old vehicle");
        let mut replay_old = replay.vehicle_handle("V").expect("replay vehicle");
        let mut warmed = None;

        for iteration in 0..10_000 {
            let outcome = world
                .replace_completed_vehicle(old, &input)
                .expect("replacement succeeds");
            let replay_outcome = replay
                .replace_completed_vehicle(replay_old, &input)
                .expect("replay replacement succeeds");
            assert_eq!(outcome, replay_outcome);
            let VehicleReplaceOutcome::Replaced(record) = outcome else {
                panic!("single-vehicle replacement cannot be blocked")
            };
            let VehicleReplaceOutcome::Replaced(replay_record) = replay_outcome else {
                panic!("replay replacement cannot be blocked")
            };
            let step = world.step(TickInput::new(1_000)).expect("completion step");
            let replay_step = replay
                .step(TickInput::new(1_000))
                .expect("replay completion step");
            assert_eq!(step, replay_step);
            assert_eq!(
                world.vehicle(record.new).expect("replacement state").status,
                VehicleStatus::Completed
            );
            old = record.new;
            replay_old = replay_record.new;

            if iteration == 0 {
                warmed = Some(world.lifecycle_retained_stats());
            }
        }

        let warmed = warmed.expect("warm retained stats");
        let retained = world.lifecycle_retained_stats();
        assert_eq!(retained.live_vehicles, 1);
        assert_eq!(retained.route_candidate_nodes, 1);
        assert_eq!(retained.stale_route_candidate_nodes, 0);
        assert_eq!(retained.tombstones, 0);
        assert_eq!(
            retained.complete_accounted_bytes,
            warmed.complete_accounted_bytes
        );
        assert_eq!(world, replay);
        world.assert_lifecycle_indices_consistent();
    }
