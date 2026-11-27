use super::*;
use laneflow_runtime::{
    AdmittedRouteRegisterError, AdmittedRouteRegisterInput, CandidateRouteInput,
    ClearanceSpeedCurve, ClearanceSpeedPoint, CostModelId, CostModelKey,
    DynamicCostSnapshotBinding, HandoffDrivingConfig, ObservationExportMode, ObservationSelection,
    StableRouteHandoffWindow, bind_observation_set,
};
use laneflow_static_contract::MAX_SPEED_MM_S;
use std::num::NonZeroU32;

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
    let admission = world.open_routing_admission(model);
    let candidate = world
        .register_candidate_route(
            &admission,
            CandidateRouteInput::new(cost, ids.clone()).with_handoffs(stable.clone()),
        )
        .unwrap();
    let replayed = world
        .register_admitted_route(replay(stable.clone()))
        .unwrap();
    for handle in [direct, candidate, replayed] {
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
