use super::*;
use laneflow_runtime::{
    AdmittedRouteRegisterError, AdmittedRouteRegisterInput, CandidateRouteError,
    CandidateRouteInput, CapturedRoute, CapturedSnapshot, ClearanceSpeedCurve, ClearanceSpeedPoint,
    CostModelId, CostModelKey, DynamicCostSnapshotBinding, HandoffDrivingConfig,
    ObservationExportMode, ObservationSelection, RouteHandle, RouteHandoffRequest,
    RoutingAdmissionSession, SnapshotRestoreLimits, StableRouteHandoffWindow, TickInput,
    TrafficWorld, VehicleSpawnInput, bind_observation_set, deterministic_state_digest, encode_lfrs,
    restore_lfrs,
};
use laneflow_static_contract::MAX_SPEED_MM_S;
use std::num::NonZeroU32;

fn candidate_binding(
    world: &TrafficWorld,
) -> (RoutingAdmissionSession, DynamicCostSnapshotBinding) {
    let mut session = world
        .open_observation_export(ObservationSelection::AllLaneEdges)
        .unwrap();
    let batch = world
        .export_observation(&mut session, ObservationExportMode::Full)
        .unwrap();
    let model = CostModelKey::new(CostModelId::from_bytes([1; 32]), 1);
    let cost = DynamicCostSnapshotBinding::new(
        bind_observation_set(&[&batch]).unwrap(),
        model,
        0,
        0,
        0,
        Sha256Digest::ZERO,
    )
    .unwrap();
    (world.open_routing_admission(model), cost)
}

fn admitted_command(
    snapshot: &CapturedSnapshot,
    captured: &CapturedRoute,
) -> AdmittedRouteRegisterInput {
    let origin = snapshot.origin();
    AdmittedRouteRegisterInput::new(
        origin.network_revision(),
        origin
            .static_contract_versions()
            .network_revision_derivation_version(),
        captured.edges().to_vec(),
    )
    .with_handoffs(captured.handoff_windows().to_vec())
}

pub(super) fn driving() -> HandoffDrivingConfig {
    HandoffDrivingConfig {
        curve: ClearanceSpeedCurve::try_new(vec![
            ClearanceSpeedPoint {
                clearance_mm: 0,
                speed_mm_s: 1_000,
            },
            ClearanceSpeedPoint {
                clearance_mm: 100,
                speed_mm_s: MAX_SPEED_MM_S,
            },
        ])
        .unwrap(),
        max_span_mm: NonZeroU32::new(3_000).unwrap(),
        travel_tolerance_mm: NonZeroU32::new(5).unwrap(),
    }
}

#[test]
fn all_registration_paths_preserve_repeated_windows_and_reject_invalid_inputs_atomically() {
    let revision = compile_road_editing_revision(module(0.));
    let (section, source, target) = stop_line::road_lanes(&revision);
    let turn = revision.traffic().successors(target[2]).unwrap()[0];
    let edges = [
        source[2], target[1], target[2], turn, source[0], source[1], source[2], target[1],
    ];
    let declarations = [0, 6].map(|source_index| RouteHandoffOccurrence {
        revision: revision.network_revision(),
        source_index,
        owner: BoundaryOwner::Section(section),
        left_index: 0,
        stations_mm: [24_000, 26_000],
    });
    let config = WorldConfig::try_new(WorldConfigInput {
        vehicle_capacity: 2,
        route_capacity: 3,
        route_edge_occurrence_capacity: 24,
        route_conflict_occurrence_capacity: 8,
        fixed_delta_time_ms: 100,
    })
    .unwrap();
    let mut off = install_fixture(Arc::clone(&revision), config).unwrap();
    let input = || RouteRegisterInput::new(edges.to_vec()).with_handoffs(declarations.to_vec());
    assert_eq!(
        off.register_route(input()),
        Err(RouteError::HandoffDisabled)
    );
    assert_eq!(off.command_cursor(), 0);
    let mut world = install_fixture(
        Arc::clone(&revision),
        config.with_handoff_driving(driving()),
    )
    .unwrap();
    let mut too_far = declarations[0];
    too_far.stations_mm[1] = u64::MAX / 1_000_000 + 1;
    assert_eq!(
        world.register_route(RouteRegisterInput::new(edges.to_vec()).with_handoffs(vec![too_far])),
        Err(RouteError::InvalidHandoffWindow)
    );
    assert_eq!(world.command_cursor(), 0);
    let direct = world.register_route(input()).unwrap();
    let declared = world.route_handoff_windows(direct).unwrap()[0].1.window();
    assert_eq!(declared.stations_mm, declarations[0].stations_mm);
    let straddling = laneflow_runtime::VehicleSpawnInput::new(
        conflict_profile(&revision, "car"),
        direct,
        1,
        800,
        0,
    );
    assert_eq!(
        world.spawn_vehicle(straddling),
        Err(laneflow_runtime::SpawnError::EntranceBody(
            laneflow_runtime::EntranceBodyError::HandoffStraddled
        )),
    );
    let stable: Vec<_> = world
        .route_handoff_windows(direct)
        .unwrap()
        .iter()
        .map(|&(index, window)| {
            StableRouteHandoffWindow::from_binding(&revision, index, window).unwrap()
        })
        .collect();
    let ids: Vec<_> = edges
        .iter()
        .map(|&edge| revision.identity().stable_id(edge).unwrap().into_untyped())
        .collect();
    let replay = |windows| {
        AdmittedRouteRegisterInput::new(
            revision.network_revision(),
            revision
                .canonical_origin()
                .static_contract_versions()
                .network_revision_derivation_version(),
            ids.clone(),
        )
        .with_handoffs(windows)
    };
    let mut swapped = stable.clone();
    swapped[0].right_member = swapped[0].left_member;
    assert_eq!(
        world.register_admitted_route(replay(swapped)),
        Err(AdmittedRouteRegisterError::Route(
            RouteError::InvalidHandoffWindow
        ))
    );
    assert_eq!(
        world.register_route(
            RouteRegisterInput::new(edges.to_vec()).with_handoffs(vec![declarations[0]; 2])
        ),
        Err(RouteError::InvalidHandoffWindow)
    );
    assert_eq!(world.live_routes().count(), 1);
    assert_eq!(world.command_cursor(), 1);
    let (admission, cost) = candidate_binding(&world);
    let mut wrong_occurrence = stable.clone();
    wrong_occurrence[0].source_index = 1;
    let before = deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap();
    for (windows, error) in [
        (Vec::new(), RouteError::Disconnected),
        (wrong_occurrence, RouteError::InvalidHandoffWindow),
    ] {
        assert_eq!(
            world.register_candidate_route(
                &admission,
                CandidateRouteInput::new(cost, ids.clone()).with_handoffs(windows.clone()),
            ),
            Err(CandidateRouteError::Route(error))
        );
        assert_eq!(
            world.register_admitted_route(replay(windows)),
            Err(AdmittedRouteRegisterError::Route(error))
        );
    }
    assert_eq!(world.command_cursor(), 1);
    assert_eq!(
        deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap(),
        before
    );
    let candidate = world
        .register_candidate_route(
            &admission,
            CandidateRouteInput::new(cost, ids.clone()).with_handoffs(stable.clone()),
        )
        .unwrap();
    // 快照局部 ID 回映到恢复世界，已准入命令直接取自登记成功的候选。
    let snapshot = world.capture_snapshot().unwrap();
    let captured = world
        .live_routes()
        .zip(snapshot.routes())
        .find_map(|(handle, row)| (handle == candidate).then_some(row))
        .unwrap();
    assert_eq!(captured.edges(), ids);
    assert_eq!(captured.handoff_windows(), stable);
    let restored = restore_lfrs(
        &encode_lfrs(&snapshot),
        Arc::clone(&revision),
        world.committed_source().clone(),
        world.config(),
        laneflow_runtime::ExecutionConfig::new(NonZeroU32::MIN),
        SnapshotRestoreLimits::new(4 << 20, 4096),
    )
    .unwrap();
    let restored_candidate = restored.route_handle(captured.snapshot_route_id()).unwrap();
    let mut restored_world = restored.into_world();
    let command = admitted_command(&snapshot, captured);
    let replayed = restored_world.register_admitted_route(command).unwrap();
    for (world, handle) in [
        (&world, direct),
        (&world, candidate),
        (&restored_world, restored_candidate),
        (&restored_world, replayed),
    ] {
        let windows = world.route_handoff_windows(handle).unwrap();
        assert_eq!(
            windows
                .iter()
                .map(|&(i, b)| StableRouteHandoffWindow::from_binding(&revision, i, b).unwrap())
                .collect::<Vec<_>>(),
            stable
        );
        assert_eq!(world.route_edges(handle), Some(edges.as_slice()));
    }
    world.remove_route(direct).unwrap();
    assert!(world.route_handoff_windows(direct).is_none());
}

#[test]
fn candidate_and_replayed_routes_drive_the_same_vehicle_through_tail_clearance() {
    let (direct, request, config) = execution::drive_at_interval(3_000, 1_000);
    let start = direct.vehicle(request.vehicle).unwrap();
    let direct_snapshot = direct.capture_snapshot().unwrap();
    let [declared] = direct_snapshot.routes() else {
        panic!("夹具只含一条路线")
    };
    let mut candidate_world = install_fixture(direct.revision(), config).unwrap();
    let (admission, cost) = candidate_binding(&candidate_world);
    let candidate = candidate_world
        .register_candidate_route(
            &admission,
            CandidateRouteInput::new(cost, declared.edges().to_vec())
                .with_handoffs(declared.handoff_windows().to_vec()),
        )
        .unwrap();

    let captured = candidate_world.capture_snapshot().unwrap();
    let [registered] = captured.routes() else {
        panic!("应已登记一条候选")
    };
    let mut replay_world = install_fixture(direct.revision(), config).unwrap();
    let replayed = replay_world
        .register_admitted_route(admitted_command(&captured, registered))
        .unwrap();

    let spawn = |world: &mut TrafficWorld, route: RouteHandle| {
        let vehicle = world
            .spawn_vehicle(VehicleSpawnInput::new(
                start.profile(),
                route,
                start.route_edge_index(),
                start.progress_mm(),
                start.speed_mm_s(),
            ))
            .unwrap();
        RouteHandoffRequest {
            vehicle,
            route,
            ..request
        }
    };
    let candidate_request = spawn(&mut candidate_world, candidate);
    let replay_request = spawn(&mut replay_world, replayed);
    let drive = |mut world: TrafficWorld, request: RouteHandoffRequest| {
        let step = TickInput::new(config.fixed_delta_time_ms());
        let mut digests = Vec::new();
        let mut front_on_target = false;
        for tick in 1..=160 {
            if tick == 1 {
                world.step_with_route_handoffs(step, &[request]).unwrap();
            } else {
                world.step(step).unwrap();
            }
            digests.push(deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap());
            match world.vehicle_route_handoffs(request.vehicle).next() {
                Some(active) => {
                    assert_eq!(active.acquired_tick, 1);
                    if active.front_handed_off {
                        assert!(
                            world.vehicle(request.vehicle).unwrap().route_edge_index()
                                > request.source_index
                        );
                        front_on_target = true;
                    }
                }
                None => {
                    assert!(front_on_target, "车尾释放应晚于前杠交接");
                    return digests;
                }
            }
        }
        panic!("30 米换道未清空");
    };
    let expected = drive(direct, request);
    assert_eq!(drive(candidate_world, candidate_request), expected);
    assert_eq!(drive(replay_world, replay_request), expected);
}
