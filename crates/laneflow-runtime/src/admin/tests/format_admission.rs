use super::*;
use crate::admin::cutover::tests::transaction_tests::{revision, source_for, world_with_vehicle};
use crate::admin::cutover_migration::tests::virtual_parking_cutover_world;
use crate::{
    CapturedParkingBinding, CapturedParkingTarget, CapturedVirtualParkingEntry,
    ParkedVehicleSpawnInput, ParkingBinding, ParkingTarget, RouteRegisterInput, TickInput,
    VehicleState, encode_lfrs,
};

fn generous_limits() -> SnapshotRestoreLimits {
    SnapshotRestoreLimits::new(16 * 1_024 * 1_024, 4 * 1_024)
}

fn conflict_world_with_route() -> (TrafficWorld, RouteHandle) {
    conflict_world_with_route_config(WorldConfig::new(8, 4, 1_024, 1_024, 1, 100))
}

fn conflict_world_with_route_config(config: WorldConfig) -> (TrafficWorld, RouteHandle) {
    let revision = revision(true);
    let origin = *revision.canonical_origin();
    let mut world = TrafficWorld::install(
        Arc::clone(&revision),
        config,
        source_for(origin, "fixture://conflict-snapshot"),
        77,
        crate::test_policy::selection(&revision),
    )
    .expect("install conflict world");
    let stream = laneflow_static_contract::ParticipantStreamOrdinal::from_raw(0);
    let path = revision
        .conflict()
        .participant_stream(stream)
        .expect("fixture stream")
        .maneuver_path();
    let route_edges = revision
        .traffic()
        .maneuvers()
        .maneuver_path(path)
        .expect("fixture path")
        .edges()
        .to_vec();
    let route = world
        .register_route(RouteRegisterInput::new(route_edges))
        .expect("conflict route");
    (world, route)
}

pub(crate) fn world_with_conflict_reservation() -> (TrafficWorld, VehicleHandle) {
    world_with_conflict_reservation_config(WorldConfig::new(8, 4, 1_024, 1_024, 1, 100))
}

fn world_with_conflict_reservation_config(config: WorldConfig) -> (TrafficWorld, VehicleHandle) {
    let (mut world, route) = conflict_world_with_route_config(config);
    let vehicle = world
        .restore_unparked_vehicle(
            VehicleSpawnInput::new(
                laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                0,
                0,
            ),
            0,
            VehicleStatus::Active,
            None,
            None,
            true,
        )
        .expect("upstream vehicle");
    install_conflict_reservation(&mut world, route, vehicle);
    (world, vehicle)
}

pub(crate) fn install_conflict_reservation(
    world: &mut TrafficWorld,
    route: RouteHandle,
    vehicle: VehicleHandle,
) {
    let (gate_range, first_occurrence) = {
        let compiled = world.compiled_route(route).expect("compiled route");
        let first_occurrence = *compiled.conflicts.first().expect("conflict occurrence");
        let gate_range = compiled.conflict_gate_ranges[first_occurrence.admission_hop as usize];
        (gate_range, first_occurrence)
    };
    {
        let state = world.committed.vehicles[vehicle.index() as usize]
            .state
            .as_mut()
            .expect("vehicle state");
        state.route_edge_index = first_occurrence.entry.route_edge_index;
        state.progress_mm = first_occurrence.entry.progress_mm;
        state.carry_um = 0;
        state.speed_mm_s = 0;
        state.waiting_membership = None;
    }
    let front_um = route_position_um(
        world,
        route,
        first_occurrence.entry.route_edge_index,
        first_occurrence.entry.progress_mm,
        0,
    )
    .expect("front position");
    let length_mm = world.vehicle_state(vehicle).expect("vehicle").length_mm;
    let tail_um = i128::try_from(front_um).expect("front fits i128")
        - i128::from(length_mm) * i128::from(MICROMETRES_PER_MILLIMETRE);
    let mut cells = Vec::new();
    let range_end = gate_range.start + gate_range.len;
    for index in gate_range.start..range_end {
        let occurrence = world
            .compiled_route(route)
            .expect("compiled route")
            .conflicts[index as usize];
        let entry_um = route_position_um(
            world,
            route,
            occurrence.entry.route_edge_index,
            occurrence.entry.progress_mm,
            0,
        )
        .expect("entry");
        let clearance_um = route_position_um(
            world,
            route,
            occurrence.clearance.route_edge_index,
            occurrence.clearance.progress_mm,
            0,
        )
        .expect("clearance");
        let cleared = tail_um >= i128::try_from(clearance_um).expect("clearance fits i128");
        cells.push(crate::kernel::conflict::RestoredConflictCell {
            address: occurrence.address(),
            occupant: front_um >= entry_um && !cleared,
            cleared,
        });
    }
    cells.sort_unstable_by_key(|cell| cell.address);
    let passage_range = crate::ConflictPassageRange::new(
        route,
        first_occurrence.maneuver_index,
        first_occurrence.admission_hop,
        gate_range.start,
        gate_range.len,
    )
    .expect("passage range");
    let mut downstream = Vec::new();
    world
        .derive_reservation_downstream_claims(passage_range, length_mm, &mut downstream)
        .expect("derive downstream physical union");
    assert!(!downstream.is_empty());
    let follower_min_gap_mm = world
        .binding
        .revision
        .traffic()
        .relations()
        .vehicle_profile(world.vehicle_state(vehicle).expect("vehicle").profile)
        .expect("vehicle profile")
        .min_gap_mm();
    let reservation = crate::kernel::conflict::ConflictWrite::new(
        &mut world.committed.conflict,
        &mut world.derived.conflict,
        &mut world.workspace.conflict,
    )
    .restore_reservation(
        vehicle,
        crate::kernel::conflict::RestoredConflictReservation {
            follower_min_gap_mm,
            acquired_tick: 0,
            passage_range,
            cells: &cells,
            downstream: &downstream,
        },
    )
    .expect("restore test reservation");
    world.committed.vehicles[vehicle.index() as usize]
        .state
        .as_mut()
        .expect("vehicle state")
        .maneuver_traversal = Some(ManeuverTraversalState {
        route,
        maneuver_occurrence_index: first_occurrence.maneuver_index,
        phase: ManeuverTraversalPhase::Clearing {
            admission_gate_hop: reservation.admission_gate_hop(),
        },
    });
    crate::kernel::conflict::ConflictWrite::new(
        &mut world.committed.conflict,
        &mut world.derived.conflict,
        &mut world.workspace.conflict,
    )
    .restore_lag_reference(
        cells[0].address,
        crate::ConflictLagReference::ActualClear(0),
    )
    .expect("tick-zero history");
    assert!(world.conflict_state_valid());
}

pub(crate) fn world_with_conflict_eligibility() -> (TrafficWorld, VehicleHandle) {
    let (mut world, route) = conflict_world_with_route();
    let locator = world
        .conflict_passage_occurrence_locator(route, 0)
        .expect("first conflict occurrence");
    let (gate_hop, gate_progress) = {
        let compiled = world.compiled_route(route).expect("compiled route");
        let hop = locator.admission_gate_hop();
        let edge = compiled.edges[hop as usize];
        (
            hop,
            world.binding.revision.traffic().lane_lengths_millimetres()[edge.index()],
        )
    };
    let vehicle = world
        .restore_unparked_vehicle(
            VehicleSpawnInput::new(
                laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                route,
                gate_hop,
                gate_progress,
                0,
            ),
            0,
            VehicleStatus::Active,
            None,
            None,
            true,
        )
        .expect("restore vehicle before conflict Gate");
    world.committed.conflict_eligibility.resize(
        usize::try_from(world.binding.config.vehicle_capacity()).expect("vehicle capacity"),
        None,
    );
    world.committed.conflict_eligibility[vehicle.index() as usize] =
        crate::ConflictEligibilityState::update(None, locator, true, 0);
    assert!(world.conflict_state_valid());
    (world, vehicle)
}

#[test]
fn restored_conflict_authority_continues_through_the_production_tick() {
    for (mut world, label) in [
        (world_with_conflict_reservation().0, "reservation"),
        (world_with_conflict_eligibility().0, "eligibility"),
    ] {
        let tick_before = world.tick_index();
        world
            .step(crate::TickInput::new(100))
            .unwrap_or_else(|error| panic!("{label} production continuation: {error:?}"));
        assert_eq!(world.tick_index(), tick_before + 1, "{label}");
        assert!(world.conflict_state_valid(), "{label}");
    }
}

#[test]
fn conflict_eligibility_blocks_route_rebind_without_partial_commit() {
    let (mut world, vehicle) = world_with_conflict_eligibility();
    let state = *world.vehicle_state(vehicle).expect("eligible vehicle");
    let before = world.capture_snapshot().expect("capture before rebind");
    assert!(matches!(
        world.rebind_parking_route(
            vehicle,
            crate::RebindParkingTarget::ExplicitSpace {
                space: laneflow_static_contract::ParkingSpaceOrdinal::from_raw(0),
                new_route: state.route,
                new_current_route_occurrence: state.route_edge_index,
                new_entry_route_occurrence: state.route_edge_index,
            },
        ),
        Err(crate::ParkingError::ConflictTraversalActive)
    ));
    assert_eq!(
        world.capture_snapshot().expect("capture after rebind"),
        before,
        "eligibility-protected rebind must remain atomic"
    );
}

#[test]
fn conflict_reservation_and_tick_zero_history_round_trip() {
    let (world, _) = world_with_conflict_reservation();
    let captured = world.capture_snapshot().expect("capture Conflict state");
    assert!(
        captured
            .vehicles
            .iter()
            .any(|row| row.conflict_reservation.is_some())
    );
    assert_eq!(
        captured.conflict_lag_states[0].reference,
        crate::ConflictLagReference::ActualClear(0)
    );
    let bytes = encode_lfrs(&captured);
    let restored = restore_lfrs(
        &bytes,
        world.revision(),
        world.committed_source().clone(),
        world.config(),
        generous_limits(),
    )
    .expect("restore Conflict state");
    let reservation_vehicle_id = captured
        .vehicles
        .iter()
        .find(|row| row.conflict_reservation.is_some())
        .expect("captured reservation owner")
        .snapshot_vehicle_id;
    let restored_handle = restored
        .vehicle_handle(reservation_vehicle_id)
        .expect("restored vehicle map");
    assert!(
        restored
            .world()
            .conflict_reservation(restored_handle)
            .is_some()
    );
    let recaptured = restored.world().capture_snapshot().expect("recapture");
    assert_eq!(captured, recaptured);
    assert_eq!(
        crate::deterministic_state_digest(&captured).expect("source digest"),
        crate::deterministic_state_digest(&recaptured).expect("restored digest")
    );
}

#[test]
fn clearing_marker_is_decoded_before_conflict_aggregate_installation() {
    let (world, vehicle) = world_with_conflict_reservation();
    let state = *world.vehicle_state(vehicle).expect("Clearing vehicle");
    let expected = state.maneuver_traversal.expect("Clearing marker");
    let captured = world.capture_snapshot().expect("capture Conflict state");
    let bytes = encode_lfrs(&captured);
    let root = wire::size_prefixed_root_as_runtime_snapshot(&bytes).expect("verified LFRS");
    let row = root
        .vehicles()
        .iter()
        .find(|row| row.snapshot_vehicle_id() == u64::from(vehicle.index()) + 1)
        .expect("reservation owner row");

    let decoded = decode_waiting_authority(&world, row, VehicleStatus::Active, state.route)
        .expect("Clearing anchor decodes before reservation installation");
    assert_eq!(decoded.traversal, Some(expected));
    assert!(decoded.membership.is_none());
    assert!(world.restored_waiting_authority_valid(VehicleState {
        maneuver_traversal: decoded.traversal,
        waiting_membership: decoded.membership,
        ..state
    }));
}

#[test]
fn conflict_reservation_requires_exact_gate_range_and_crossed_side() {
    let (world, vehicle) = world_with_conflict_reservation();
    let captured = world.capture_snapshot().expect("capture Conflict state");
    let snapshot_vehicle_id = captured.vehicles[vehicle.index() as usize].snapshot_vehicle_id;

    let mut wrong_range = captured.clone();
    let passages = &mut wrong_range.vehicles[vehicle.index() as usize]
        .conflict_reservation
        .as_mut()
        .expect("reservation")
        .passages;
    if passages.len() > 1 {
        passages.pop();
    } else {
        let mut extra = passages[0];
        extra.conflict_occurrence_index += 1;
        passages.push(extra);
    }
    assert_eq!(
        restore_lfrs(
            &encode_lfrs(&wrong_range),
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::InvalidConflictAuthority {
            snapshot_vehicle_id,
        }
    );

    let mut upstream = captured;
    let owner = &mut upstream.vehicles[vehicle.index() as usize];
    owner.route_edge_index = 0;
    owner.progress_mm = 0;
    owner.carry_um = 0;
    assert_eq!(
        restore_lfrs(
            &encode_lfrs(&upstream),
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::InvalidConflictAuthority {
            snapshot_vehicle_id,
        }
    );
}

#[test]
fn conflict_downstream_union_is_rederived_from_reservation_proof() {
    let (world, _) = world_with_conflict_reservation();
    let captured = world.capture_snapshot().expect("capture Conflict state");
    let owner = captured
        .vehicles
        .iter()
        .find(|vehicle| vehicle.conflict_reservation.is_some())
        .expect("reservation owner");
    let snapshot_vehicle_id = owner.snapshot_vehicle_id;
    let mut changed = captured.clone();
    let interval = changed
        .vehicles
        .iter_mut()
        .find(|vehicle| vehicle.snapshot_vehicle_id == snapshot_vehicle_id)
        .and_then(|vehicle| vehicle.conflict_reservation.as_mut())
        .and_then(|reservation| reservation.downstream_intervals.first_mut())
        .expect("downstream interval");
    assert!(interval.end_mm - interval.start_mm > 1);
    interval.start_mm += 1;
    assert_eq!(
        restore_lfrs(
            &encode_lfrs(&changed),
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::InvalidConflictAuthority {
            snapshot_vehicle_id,
        }
    );

    let mut missing = captured;
    missing
        .vehicles
        .iter_mut()
        .find(|vehicle| vehicle.snapshot_vehicle_id == snapshot_vehicle_id)
        .and_then(|vehicle| vehicle.conflict_reservation.as_mut())
        .expect("reservation")
        .downstream_intervals
        .pop();
    assert_eq!(
        restore_lfrs(
            &encode_lfrs(&missing),
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::InvalidConflictAuthority {
            snapshot_vehicle_id,
        }
    );
}

#[test]
fn pending_conflict_authority_does_not_hide_an_invalid_endpoint_cursor() {
    let (world, vehicle) = world_with_conflict_reservation();
    let mut captured = world.capture_snapshot().expect("capture Conflict state");
    let state = world.vehicle_state(vehicle).expect("vehicle");
    let edge = world.route_edges(state.route).expect("route")
        [usize::try_from(state.route_edge_index).expect("route index")];
    let owner = &mut captured.vehicles[vehicle.index() as usize];
    owner.progress_mm = world.binding.revision.traffic().lane_lengths_millimetres()[edge.index()];
    owner.carry_um = 1;
    let snapshot_vehicle_id = owner.snapshot_vehicle_id;
    assert_eq!(
        restore_lfrs(
            &encode_lfrs(&captured),
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::Vehicle {
            snapshot_vehicle_id,
            error: crate::SpawnError::InvalidProgress,
        }
    );
}

#[test]
fn conflict_nested_tables_fit_exact_small_world_verifier_budget() {
    let config = WorldConfig::new(1, 1, 64, 64, 1, 100);
    let (world, _) = world_with_conflict_reservation_config(config);
    let captured = world.capture_snapshot().expect("capture Conflict state");
    let restored = restore_lfrs(
        &encode_lfrs(&captured),
        world.revision(),
        world.committed_source().clone(),
        config,
        generous_limits(),
    )
    .expect("nested v5 tables fit the caller-bounded verifier budget");
    assert_eq!(
        restored.world().capture_snapshot().expect("recapture"),
        captured
    );
}

#[test]
fn conflict_eligibility_preserves_tick_zero_distinct_from_none() {
    let (world, vehicle) = world_with_conflict_eligibility();
    let captured = world.capture_snapshot().expect("capture eligibility");
    let binding = captured.vehicles[vehicle.index() as usize]
        .conflict_eligibility
        .expect("saved eligibility");
    assert_eq!(binding.first_eligible_tick, 0);

    let restored = restore_lfrs(
        &encode_lfrs(&captured),
        world.revision(),
        world.committed_source().clone(),
        world.config(),
        generous_limits(),
    )
    .expect("restore eligibility");
    let recaptured = restored.world().capture_snapshot().expect("recapture");
    assert_eq!(recaptured, captured);
    assert_eq!(
        recaptured.vehicles[vehicle.index() as usize]
            .conflict_eligibility
            .expect("restored eligibility")
            .first_eligible_tick,
        0
    );

    let mut absent = captured;
    absent.vehicles[vehicle.index() as usize].conflict_eligibility = None;
    assert_ne!(
        crate::deterministic_state_digest(&absent).expect("None digest"),
        crate::deterministic_state_digest(&recaptured).expect("tick-zero digest")
    );
}

#[test]
fn conflict_eligibility_rejects_gate_policy_deny_at_restored_time() {
    const POLICIES: &[u8] = include_bytes!(
        "../../../../laneflow-compiler/tests/fixtures/portable/lfca-world-policies/expected.lfca"
    );
    let revision = laneflow_static_network::build_shared_network_revision(
        laneflow_format::check_canonical_network_input(
            POLICIES,
            laneflow_format::FormatLimits::HARD,
        )
        .expect("checked policy fixture"),
        laneflow_static_network::SharedNetworkBuildOptions::new(
            laneflow_static_network::SpatialBuildOption::Omit,
            laneflow_static_network::SharedNetworkBuildLimits::new(
                64 * 1_024 * 1_024,
                16 * 1_024 * 1_024,
            ),
        ),
    )
    .expect("shared policy fixture");
    let profile = laneflow_static_contract::VehicleProfileOrdinal::from_raw(0);
    let policy_count = revision
        .identity()
        .entity_count(laneflow_static_contract::EntityKind::RightOfWayPolicySet);
    let stream_count = revision
        .identity()
        .entity_count(laneflow_static_contract::EntityKind::ParticipantStream);
    let pin = |ordinal| {
        crate::WorldPolicySelection::Pinned(crate::PolicyPin {
            policy: revision
                .identity()
                .stable_id(laneflow_static_contract::RightOfWayPolicySetOrdinal::from_raw(ordinal))
                .expect("policy identity"),
        })
    };
    let origin = *revision.canonical_origin();
    let mut selection = None;
    for stream_raw in 0..stream_count {
        let stream = laneflow_static_contract::ParticipantStreamOrdinal::from_raw(stream_raw);
        let gate = revision
            .conflict()
            .participant_stream(stream)
            .and_then(|view| view.passages().first())
            .map(|passage| passage.admission_gate())
            .expect("stream passage Gate");
        let mut candidate = None;
        let mut deny = None;
        for policy_raw in 0..policy_count {
            let world = TrafficWorld::install(
                Arc::clone(&revision),
                WorldConfig::new(8, 4, 1_024, 1_024, 1, 100),
                source_for(origin, "fixture://eligibility-policy-selection"),
                77,
                pin(policy_raw),
            )
            .expect("install selected policy");
            match world.gate_policy_decision(gate, profile) {
                crate::GatePolicyDecision::Candidate(_) => candidate = Some(policy_raw),
                crate::GatePolicyDecision::DenyAndStop => deny = Some(policy_raw),
            }
        }
        if let (Some(candidate), Some(deny)) = (candidate, deny) {
            selection = Some((stream, candidate, deny));
            break;
        }
    }
    let (stream, candidate_policy, deny_policy) =
        selection.expect("fixture has Candidate/Deny policy pair for one stream");
    let mut world = TrafficWorld::install(
        Arc::clone(&revision),
        WorldConfig::new(8, 4, 1_024, 1_024, 1, 100),
        source_for(origin, "fixture://eligibility-policy-selection"),
        77,
        pin(candidate_policy),
    )
    .expect("install Candidate policy");
    let path = revision
        .conflict()
        .participant_stream(stream)
        .expect("selected stream")
        .maneuver_path();
    let route = world
        .register_route(RouteRegisterInput::new(
            revision
                .traffic()
                .maneuvers()
                .maneuver_path(path)
                .expect("selected path")
                .edges()
                .to_vec(),
        ))
        .expect("selected route");
    let locator = world
        .conflict_passage_occurrence_locator(route, 0)
        .expect("selected conflict occurrence");
    let gate_hop = locator.admission_gate_hop();
    let gate_progress = {
        let edge = world.compiled_route(route).expect("compiled route").edges[gate_hop as usize];
        revision.traffic().lane_lengths_millimetres()[edge.index()]
    };
    let vehicle = world
        .restore_unparked_vehicle(
            VehicleSpawnInput::new(profile, route, gate_hop, gate_progress, 0),
            0,
            VehicleStatus::Active,
            None,
            None,
            true,
        )
        .expect("restore Candidate vehicle");
    world.committed.conflict_eligibility.resize(
        usize::try_from(world.binding.config.vehicle_capacity()).expect("vehicle capacity"),
        None,
    );
    world.committed.conflict_eligibility[vehicle.index() as usize] =
        crate::ConflictEligibilityState::update(None, locator, true, 0);
    assert!(world.conflict_state_valid());

    let mut captured = world
        .capture_snapshot()
        .expect("capture Candidate eligibility");
    let snapshot_vehicle_id = captured.vehicles[vehicle.index() as usize].snapshot_vehicle_id;
    captured.policy_selection = pin(deny_policy);

    assert_eq!(
        restore_lfrs(
            &encode_lfrs(&captured),
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::InvalidConflictAuthority {
            snapshot_vehicle_id,
        }
    );
}

#[test]
fn dangling_and_wrong_occurrence_conflict_locators_fail_closed() {
    let (world, _) = world_with_conflict_reservation();
    let captured = world.capture_snapshot().expect("capture Conflict state");
    let snapshot_vehicle_id = captured.vehicles[0].snapshot_vehicle_id;

    let mut dangling = captured.clone();
    dangling.vehicles[0]
        .conflict_reservation
        .as_mut()
        .expect("reservation")
        .passages[0]
        .passage
        .participant_stream = StableId128::from_bytes([0xff; 16]);
    assert_eq!(
        restore_lfrs(
            &encode_lfrs(&dangling),
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::InvalidConflictAuthority {
            snapshot_vehicle_id,
        }
    );

    let mut wrong_occurrence = captured;
    wrong_occurrence.vehicles[0]
        .conflict_reservation
        .as_mut()
        .expect("reservation")
        .passages[0]
        .entry_route_edge_index += 1;
    assert_eq!(
        restore_lfrs(
            &encode_lfrs(&wrong_occurrence),
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::InvalidConflictAuthority {
            snapshot_vehicle_id,
        }
    );
}

#[test]
fn same_revision_cutover_preserves_conflict_authority_and_history() {
    let (mut world, _) = world_with_conflict_reservation();
    let before = world.capture_snapshot().expect("capture before cutover");
    let origin = *world.revision().canonical_origin();
    let descriptor = crate::NetworkRevisionCutoverDescriptor::new(
        crate::LfcaOriginBinding::from_canonical_origin(origin),
        crate::LfcaOriginBinding::from_canonical_origin(origin),
        None,
        crate::MigrationPolicyKind::SameRevisionRestore,
        world.world_binding(),
    );
    let target = world.revision();
    let _events = world
        .cutover_same_revision(
            Arc::clone(&target),
            source_for(origin, "fixture://same-revision-conflict"),
            &descriptor,
            &crate::CutoverPreflightLimits::new(1_048_576),
        )
        .expect("same-revision Conflict cutover");
    let after = world.capture_snapshot().expect("capture after cutover");
    assert_eq!(
        after.vehicles[0].conflict_reservation,
        before.vehicles[0].conflict_reservation
    );
    assert_eq!(
        after.vehicles[0].maneuver_traversal,
        before.vehicles[0].maneuver_traversal
    );
    assert_eq!(after.conflict_lag_states, before.conflict_lag_states);
    assert!(world.conflict_state_valid());
}

#[test]
fn same_revision_cutover_preserves_conflict_eligibility() {
    let (mut world, _) = world_with_conflict_eligibility();
    let before = world.capture_snapshot().expect("capture before cutover");
    let origin = *world.revision().canonical_origin();
    let descriptor = crate::NetworkRevisionCutoverDescriptor::new(
        crate::LfcaOriginBinding::from_canonical_origin(origin),
        crate::LfcaOriginBinding::from_canonical_origin(origin),
        None,
        crate::MigrationPolicyKind::SameRevisionRestore,
        world.world_binding(),
    );
    let target = world.revision();
    let _events = world
        .cutover_same_revision(
            Arc::clone(&target),
            source_for(origin, "fixture://same-revision-eligibility"),
            &descriptor,
            &crate::CutoverPreflightLimits::new(1_048_576),
        )
        .expect("same-revision eligibility cutover");
    let after = world.capture_snapshot().expect("capture after cutover");
    assert_eq!(
        after.vehicles[0].conflict_eligibility,
        before.vehicles[0].conflict_eligibility
    );
    assert!(world.conflict_state_valid());
}

#[test]
fn duplicate_and_future_conflict_history_fail_closed() {
    let (world, _) = world_with_conflict_reservation();
    let captured = world.capture_snapshot().expect("capture Conflict state");
    let mut duplicate = captured.clone();
    duplicate
        .conflict_lag_states
        .push(duplicate.conflict_lag_states[0]);
    assert_eq!(
        restore_lfrs(
            &encode_lfrs(&duplicate),
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::InvalidConflictHistory
    );
    let mut future = captured;
    future.conflict_lag_states[0].reference = crate::ConflictLagReference::CutoverFloor(1);
    assert_eq!(
        restore_lfrs(
            &encode_lfrs(&future),
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::InvalidConflictHistory
    );
}

fn captured_parking_for(
    world: &TrafficWorld,
    vehicle: VehicleHandle,
) -> Option<CapturedParkingBinding> {
    let revision = world.revision();
    let identity = revision.identity();
    let stable_target = |target: ParkingTarget| match target {
        ParkingTarget::ExplicitSpace(space) => CapturedParkingTarget::ExplicitSpace(
            *identity.stable_id(space).expect("space").as_untyped(),
        ),
        ParkingTarget::VirtualPool(facility) => CapturedParkingTarget::VirtualPool(
            *identity.stable_id(facility).expect("facility").as_untyped(),
        ),
    };
    world.parking_binding(vehicle).map(|binding| match binding {
        ParkingBinding::Occupied(target) => CapturedParkingBinding::Occupied {
            target: stable_target(target),
        },
        ParkingBinding::Reserved(reservation) => {
            let virtual_entry = match reservation.target() {
                ParkingTarget::ExplicitSpace(_) => None,
                ParkingTarget::VirtualPool(_) => {
                    let (edge, progress_mm) = world
                        .reservation_anchor(reservation)
                        .expect("reserved virtual anchor");
                    Some(CapturedVirtualParkingEntry {
                        lane_edge: *identity.stable_id(edge).expect("edge").as_untyped(),
                        progress_mm,
                    })
                }
            };
            CapturedParkingBinding::Reserved {
                target: stable_target(reservation.target()),
                entry_route_occurrence: reservation.entry_route_occurrence(),
                virtual_entry,
            }
        }
    })
}

#[test]
fn save_load_restores_exact_logical_state_and_local_id_maps() {
    let (mut original, route, _) = world_with_vehicle(true);
    original.step(TickInput::new(100)).expect("step");
    let profile = laneflow_static_contract::VehicleProfileOrdinal::from_raw(0);
    let _second = original
        .spawn_parked_vehicle(
            ParkedVehicleSpawnInput::new(profile, route, 0, 10_000),
            ParkingTarget::ExplicitSpace(laneflow_static_contract::ParkingSpaceOrdinal::from_raw(
                0,
            )),
        )
        .expect("park second")
        .vehicle;
    let snapshot = original.capture_snapshot().expect("capture");
    assert_eq!(snapshot.vehicles[0].status, VehicleStatus::Active);
    assert_eq!(snapshot.vehicles[1].status, VehicleStatus::Parked);
    let bytes = encode_lfrs(&snapshot);

    let restored = restore_lfrs(
        &bytes,
        original.revision(),
        original.committed_source().clone(),
        original.config(),
        generous_limits(),
    )
    .expect("restore");
    let world = restored.world();
    assert_eq!(world.world_id(), snapshot.world_id);
    assert_eq!(world.world_generation(), crate::WorldGeneration::INITIAL);
    assert_eq!(world.tick_index(), snapshot.tick);
    assert_eq!(world.time_ms(), snapshot.time_ms);
    assert_eq!(world.command_cursor(), snapshot.command_cursor);
    assert_eq!(world.config(), snapshot.config);
    assert_eq!(
        world.observation_state_sequence(),
        ObservationStateSequence::INITIAL
    );
    assert_eq!(world.committed_source(), snapshot.source());

    let identity = world.binding.revision.identity();
    for captured in &snapshot.routes {
        let handle = restored
            .route_handle(captured.snapshot_route_id)
            .expect("route map");
        let stable_edges = world
            .route_edges(handle)
            .expect("restored route")
            .iter()
            .map(|edge| *identity.stable_id(*edge).expect("edge").as_untyped())
            .collect::<Vec<_>>();
        assert_eq!(stable_edges, captured.edges);
    }
    for captured in &snapshot.vehicles {
        let handle = restored
            .vehicle_handle(captured.snapshot_vehicle_id)
            .expect("vehicle map");
        let state = world.vehicle(handle).expect("restored vehicle");
        assert_eq!(
            state.route(),
            restored
                .route_handle(captured.snapshot_route_id)
                .expect("route map")
        );
        assert_eq!(state.route_edge_index(), captured.route_edge_index);
        assert_eq!(state.progress_mm(), captured.progress_mm);
        assert_eq!(state.carry_um(), captured.carry_um);
        assert_eq!(state.speed_mm_s(), captured.speed_mm_s);
        assert_eq!(state.status(), captured.status);
        assert_eq!(
            *identity
                .stable_id(state.profile())
                .expect("profile")
                .as_untyped(),
            captured.profile
        );
        assert_eq!(
            *identity
                .stable_id(state.class())
                .expect("class")
                .as_untyped(),
            captured.class
        );
        assert_eq!(captured_parking_for(world, handle), captured.parking);
    }
    let mapped_live_order = snapshot
        .live_order
        .iter()
        .map(|id| restored.vehicle_handle(*id).expect("live map"))
        .collect::<Vec<_>>();
    assert_eq!(world.live_vehicles(), mapped_live_order);
    let mut world = restored.into_world();
    world
        .step(TickInput::new(100))
        .expect("restored world steps");
}

#[test]
fn exhausted_command_cursor_restores_parked_and_reserved_without_new_commands() {
    let (mut world, _, _) = virtual_parking_cutover_world();
    world.committed.command_cursor = u64::MAX;
    let captured = world.capture_snapshot().expect("capture");
    assert!(captured.vehicles.iter().any(|vehicle| matches!(
        vehicle.parking,
        Some(CapturedParkingBinding::Occupied { .. })
    )));
    assert!(captured.vehicles.iter().any(|vehicle| matches!(
        vehicle.parking,
        Some(CapturedParkingBinding::Reserved { .. })
    )));
    let restored = restore_lfrs(
        &encode_lfrs(&captured),
        world.revision(),
        world.committed_source().clone(),
        world.config(),
        generous_limits(),
    )
    .expect("restore does not consume commands");
    assert_eq!(restored.world().command_cursor(), u64::MAX);
    assert_eq!(
        crate::deterministic_state_digest(
            &restored
                .world()
                .capture_snapshot()
                .expect("capture restored")
        )
        .expect("restored digest"),
        crate::deterministic_state_digest(&captured).expect("source digest")
    );
}

#[test]
fn framing_and_wire_limits_fail_before_flatbuffers_lowering() {
    let (world, _, _) = world_with_vehicle(true);
    let bytes = encode_lfrs(&world.capture_snapshot().expect("capture"));
    assert_eq!(
        restore_lfrs(
            &bytes,
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            SnapshotRestoreLimits::new(1, 4 * 1_024),
        )
        .unwrap_err(),
        limit_error(
            SnapshotLimitDimension::WireBytes,
            1,
            u64::try_from(bytes.len()).expect("length")
        )
    );
    assert_eq!(
        restore_lfrs(
            &bytes[..8],
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::TruncatedFraming
    );
    let mut wrong_prefix = bytes.clone();
    wrong_prefix[0] ^= 1;
    assert!(matches!(
        restore_lfrs(
            &wrong_prefix,
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            generous_limits(),
        ),
        Err(SnapshotRestoreError::SizePrefixMismatch { .. })
    ));
    let mut wrong_identifier = bytes;
    wrong_identifier[8..12].copy_from_slice(b"NOPE");
    assert_eq!(
        restore_lfrs(
            &wrong_identifier,
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::FileIdentifierMismatch
    );

    let valid = encode_lfrs(&world.capture_snapshot().expect("capture"));
    let asset_key_len = {
        let root = wire::size_prefixed_root_as_runtime_snapshot(&valid).expect("valid LFRS");
        u64::try_from(
            root.source_published()
                .expect("published")
                .asset_key()
                .len(),
        )
        .expect("asset key length")
    };
    assert_eq!(
        restore_lfrs(
            &valid,
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            SnapshotRestoreLimits::new(u64::try_from(valid.len()).expect("wire length"), 0),
        )
        .unwrap_err(),
        limit_error(SnapshotLimitDimension::AssetKeyBytes, 0, asset_key_len)
    );

    let mut structurally_invalid = encode_lfrs(&world.capture_snapshot().expect("capture"));
    structurally_invalid[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        restore_lfrs(
            &structurally_invalid,
            world.revision(),
            world.committed_source().clone(),
            world.config(),
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::InvalidFlatbuffer
    );
}

#[test]
fn clock_capacity_and_duplicate_ids_fail_closed() {
    let (world, _, _) = world_with_vehicle(true);
    let revision = world.revision();
    let source = world.committed_source().clone();
    let config = world.config();

    let mut invalid_clock = world.capture_snapshot().expect("capture");
    invalid_clock.time_ms = 1;
    assert_eq!(
        restore_lfrs(
            &encode_lfrs(&invalid_clock),
            Arc::clone(&revision),
            source.clone(),
            config,
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::InvalidClock
    );

    let smaller = WorldConfig::new(
        config.vehicle_capacity() - 1,
        config.route_capacity(),
        config.route_edge_occurrence_capacity(),
        config.route_conflict_occurrence_capacity(),
        config.worker_count(),
        config.fixed_delta_time_ms(),
    );
    assert!(matches!(
        restore_lfrs(
            &encode_lfrs(&world.capture_snapshot().expect("capture")),
            Arc::clone(&revision),
            source.clone(),
            smaller,
            generous_limits(),
        ),
        Err(SnapshotRestoreError::TargetCapacitySmaller {
            dimension: SnapshotLimitDimension::Vehicles,
            ..
        })
    ));

    let mut duplicate_route = world.capture_snapshot().expect("capture");
    duplicate_route
        .routes
        .push(duplicate_route.routes[0].clone());
    assert!(matches!(
        restore_lfrs(
            &encode_lfrs(&duplicate_route),
            Arc::clone(&revision),
            source.clone(),
            config,
            generous_limits(),
        ),
        Err(SnapshotRestoreError::DuplicateRouteId { .. })
    ));

    let mut duplicate_vehicle = world.capture_snapshot().expect("capture");
    duplicate_vehicle
        .vehicles
        .push(duplicate_vehicle.vehicles[0].clone());
    duplicate_vehicle
        .live_order
        .push(duplicate_vehicle.live_order[0]);
    assert!(matches!(
        restore_lfrs(
            &encode_lfrs(&duplicate_vehicle),
            revision,
            source,
            config,
            generous_limits(),
        ),
        Err(SnapshotRestoreError::DuplicateVehicleId { .. })
    ));
}

#[test]
fn config_axes_and_republished_source_follow_restore_contract() {
    let (world, _, _) = world_with_vehicle(true);
    let revision = world.revision();
    let source = world.committed_source().clone();
    let config = world.config();
    let bytes = encode_lfrs(&world.capture_snapshot().expect("capture"));

    let different_dt = WorldConfig::new(
        config.vehicle_capacity(),
        config.route_capacity(),
        config.route_edge_occurrence_capacity(),
        config.route_conflict_occurrence_capacity(),
        config.worker_count(),
        config.fixed_delta_time_ms() + 1,
    );
    assert_eq!(
        restore_lfrs(
            &bytes,
            Arc::clone(&revision),
            source.clone(),
            different_dt,
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::FixedDeltaTimeMismatch {
            snapshot: config.fixed_delta_time_ms(),
            target: config.fixed_delta_time_ms() + 1,
        }
    );

    let smaller_routes = WorldConfig::new(
        config.vehicle_capacity(),
        config.route_capacity() - 1,
        config.route_edge_occurrence_capacity(),
        config.route_conflict_occurrence_capacity(),
        config.worker_count(),
        config.fixed_delta_time_ms(),
    );
    assert_eq!(
        restore_lfrs(
            &bytes,
            Arc::clone(&revision),
            source.clone(),
            smaller_routes,
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::TargetCapacitySmaller {
            dimension: SnapshotLimitDimension::Routes,
            snapshot: u64::from(config.route_capacity()),
            target: u64::from(config.route_capacity() - 1),
        }
    );

    let smaller_occurrences = WorldConfig::new(
        config.vehicle_capacity(),
        config.route_capacity(),
        config.route_edge_occurrence_capacity() - 1,
        config.route_conflict_occurrence_capacity(),
        config.worker_count(),
        config.fixed_delta_time_ms(),
    );
    assert_eq!(
        restore_lfrs(
            &bytes,
            Arc::clone(&revision),
            source.clone(),
            smaller_occurrences,
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::TargetCapacitySmaller {
            dimension: SnapshotLimitDimension::RouteEdgeOccurrences,
            snapshot: config.route_edge_occurrence_capacity(),
            target: config.route_edge_occurrence_capacity() - 1,
        }
    );

    let larger = WorldConfig::new(
        config.vehicle_capacity() + 1,
        config.route_capacity() + 1,
        config.route_edge_occurrence_capacity() + 1,
        config.route_conflict_occurrence_capacity() + 1,
        config.worker_count(),
        config.fixed_delta_time_ms(),
    );
    let restored = restore_lfrs(
        &bytes,
        Arc::clone(&revision),
        source.clone(),
        larger,
        generous_limits(),
    )
    .expect("semantic capacities may grow");
    assert_eq!(restored.world().config(), larger);
    drop(restored);

    let mut saved_with_other_worker = world.capture_snapshot().expect("capture");
    saved_with_other_worker.config = WorldConfig::new(
        config.vehicle_capacity(),
        config.route_capacity(),
        config.route_edge_occurrence_capacity(),
        config.route_conflict_occurrence_capacity(),
        99,
        config.fixed_delta_time_ms(),
    );
    let restored = restore_lfrs(
        &encode_lfrs(&saved_with_other_worker),
        Arc::clone(&revision),
        source.clone(),
        config,
        generous_limits(),
    )
    .expect("saved worker plan is ignored and rebuilt from target config");
    assert_eq!(
        restored.world().config().worker_count(),
        config.worker_count()
    );
    drop(restored);

    let republished = CommittedNetworkSource::Published {
        reference: crate::PublishedLfcaReference::new(
            "asset://same-revision-republished",
            laneflow_static_contract::Sha256Digest::from_bytes([0xa5; 32]),
            laneflow_static_contract::ExactByteLength::new(777),
            revision.network_revision(),
        )
        .expect("republished source"),
    };
    assert_ne!(republished, source);
    let restored = restore_lfrs(
        &bytes,
        revision,
        republished.clone(),
        config,
        generous_limits(),
    )
    .expect("same semantic revision permits republished exact bytes");
    assert_eq!(restored.world().committed_source(), &republished);
}

#[test]
fn occurrence_capacity_max_and_max_plus_one_fail_atomically() {
    let (world, _, _) = world_with_vehicle(true);
    let revision = world.revision();
    let source = world.committed_source().clone();
    let mut at_max = world.capture_snapshot().expect("capture");
    let occurrence_count = at_max
        .routes
        .iter()
        .map(|route| u64::try_from(route.edges.len()).expect("edge count"))
        .sum::<u64>();
    at_max.config = WorldConfig::new(
        at_max.config.vehicle_capacity(),
        at_max.config.route_capacity(),
        occurrence_count,
        at_max.config.route_conflict_occurrence_capacity(),
        at_max.config.worker_count(),
        at_max.config.fixed_delta_time_ms(),
    );
    let exact_config = at_max.config;
    let restored = restore_lfrs(
        &encode_lfrs(&at_max),
        Arc::clone(&revision),
        source.clone(),
        exact_config,
        generous_limits(),
    )
    .expect("occurrence total exactly at max");
    assert_eq!(
        restored
            .world()
            .capture_snapshot()
            .expect("capture")
            .routes()
            .iter()
            .map(|route| u64::try_from(route.edges().len()).expect("edge count"))
            .sum::<u64>(),
        occurrence_count
    );

    let mut max_plus_one = at_max;
    let extra_edge = max_plus_one.routes[0].edges[0];
    max_plus_one.routes[0].edges.push(extra_edge);
    assert_eq!(
        restore_lfrs(
            &encode_lfrs(&max_plus_one),
            revision,
            source,
            exact_config,
            generous_limits(),
        )
        .unwrap_err(),
        limit_error(
            SnapshotLimitDimension::RouteEdgeOccurrences,
            occurrence_count,
            occurrence_count + 1,
        )
    );
}

#[test]
fn parking_and_live_order_invariants_fail_closed() {
    let (mut world, route, _) = world_with_vehicle(true);
    let _second = world
        .spawn_parked_vehicle(
            ParkedVehicleSpawnInput::new(
                laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                10_000,
            ),
            ParkingTarget::ExplicitSpace(laneflow_static_contract::ParkingSpaceOrdinal::from_raw(
                0,
            )),
        )
        .expect("parked second");
    let revision = world.revision();
    let source = world.committed_source().clone();
    let config = world.config();

    let mut parking_mismatch = world.capture_snapshot().expect("capture");
    parking_mismatch.vehicles[1].parking = None;
    assert!(matches!(
        restore_lfrs(
            &encode_lfrs(&parking_mismatch),
            Arc::clone(&revision),
            source.clone(),
            config,
            generous_limits(),
        ),
        Err(SnapshotRestoreError::ParkingStatusMismatch { .. })
    ));

    let mut duplicate_live = world.capture_snapshot().expect("capture");
    duplicate_live.live_order[1] = duplicate_live.live_order[0];
    assert!(matches!(
        restore_lfrs(
            &encode_lfrs(&duplicate_live),
            revision,
            source,
            config,
            generous_limits(),
        ),
        Err(SnapshotRestoreError::DuplicateLiveOrderVehicle { .. })
    ));
}

#[test]
fn virtual_parking_corruption_capacity_and_duplicate_resources_fail_closed() {
    let (world, reserved, _occupied) = virtual_parking_cutover_world();
    let revision = world.revision();
    let source = world.committed_source().clone();
    let config = world.config();
    let before = world.capture_snapshot().expect("base virtual snapshot");
    let reserved_id = before
        .vehicles
        .iter()
        .find(|vehicle| vehicle.status == VehicleStatus::Active)
        .expect("reserved row")
        .snapshot_vehicle_id;
    assert_eq!(
        world.vehicle(reserved).expect("reserved").status(),
        VehicleStatus::Active
    );

    let mut missing_target = before.clone();
    let row = missing_target
        .vehicles
        .iter_mut()
        .find(|vehicle| vehicle.snapshot_vehicle_id == reserved_id)
        .expect("reserved row");
    let Some(CapturedParkingBinding::Reserved { target, .. }) = row.parking.as_mut() else {
        panic!("reserved binding")
    };
    *target = CapturedParkingTarget::VirtualPool(StableId128::from_bytes([0xab; 16]));
    assert_eq!(
        restore_lfrs(
            &encode_lfrs(&missing_target),
            Arc::clone(&revision),
            source.clone(),
            config,
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::UnknownParkingFacility {
            snapshot_vehicle_id: reserved_id
        }
    );

    let facility = laneflow_static_contract::ParkingFacilityOrdinal::from_raw(0);
    let facility_stable = *world
        .binding
        .revision
        .identity()
        .stable_id(facility)
        .expect("facility stable id")
        .as_untyped();
    let mut wrong_kind = before.clone();
    let row = wrong_kind
        .vehicles
        .iter_mut()
        .find(|vehicle| vehicle.snapshot_vehicle_id == reserved_id)
        .expect("reserved row");
    let Some(CapturedParkingBinding::Reserved { target, .. }) = row.parking.as_mut() else {
        panic!("reserved binding")
    };
    *target = CapturedParkingTarget::ExplicitSpace(facility_stable);
    assert_eq!(
        restore_lfrs(
            &encode_lfrs(&wrong_kind),
            Arc::clone(&revision),
            source.clone(),
            config,
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::UnknownParkingSpace {
            snapshot_vehicle_id: reserved_id
        }
    );

    let mut moved_anchor = before.clone();
    let row = moved_anchor
        .vehicles
        .iter_mut()
        .find(|vehicle| vehicle.snapshot_vehicle_id == reserved_id)
        .expect("reserved row");
    let Some(CapturedParkingBinding::Reserved {
        virtual_entry: Some(entry),
        ..
    }) = row.parking.as_mut()
    else {
        panic!("virtual entry")
    };
    entry.progress_mm += 1;
    assert_eq!(
        restore_lfrs(
            &encode_lfrs(&moved_anchor),
            Arc::clone(&revision),
            source.clone(),
            config,
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::UnknownVirtualParkingEntry {
            snapshot_vehicle_id: reserved_id
        }
    );

    let mut over_capacity = before.clone();
    let template = over_capacity
        .vehicles
        .iter()
        .find(|vehicle| vehicle.status == VehicleStatus::Parked)
        .expect("occupied row")
        .clone();
    for snapshot_vehicle_id in [3, 4] {
        let mut duplicate = template.clone();
        duplicate.snapshot_vehicle_id = snapshot_vehicle_id;
        over_capacity.vehicles.push(duplicate);
        over_capacity.live_order.push(snapshot_vehicle_id);
    }
    assert!(matches!(
        restore_lfrs(
            &encode_lfrs(&over_capacity),
            Arc::clone(&revision),
            source.clone(),
            config,
            generous_limits(),
        ),
        Err(SnapshotRestoreError::Parking {
            error: ParkingError::VirtualCapacityExhausted,
            ..
        })
    ));
    assert_eq!(world.capture_snapshot().expect("zero publish"), before);

    let (mut explicit_world, route, _) = world_with_vehicle(true);
    explicit_world
        .spawn_parked_vehicle(
            ParkedVehicleSpawnInput::new(
                laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                10_000,
            ),
            ParkingTarget::ExplicitSpace(laneflow_static_contract::ParkingSpaceOrdinal::from_raw(
                0,
            )),
        )
        .expect("explicit parked");
    let explicit_revision = explicit_world.revision();
    let explicit_source = explicit_world.committed_source().clone();
    let explicit_config = explicit_world.config();
    let mut duplicate_resource = explicit_world.capture_snapshot().expect("explicit capture");
    let mut duplicate = duplicate_resource
        .vehicles
        .iter()
        .find(|vehicle| vehicle.status == VehicleStatus::Parked)
        .expect("parked row")
        .clone();
    duplicate.snapshot_vehicle_id = 3;
    duplicate_resource.vehicles.push(duplicate);
    duplicate_resource.live_order.push(3);
    assert!(matches!(
        restore_lfrs(
            &encode_lfrs(&duplicate_resource),
            explicit_revision,
            explicit_source,
            explicit_config,
            generous_limits(),
        ),
        Err(SnapshotRestoreError::Parking {
            error: ParkingError::TargetBoundByOther,
            ..
        })
    ));
}

#[test]
fn dangling_references_and_live_order_gaps_fail_closed() {
    let (world, _route, _first) = world_with_vehicle(true);
    let revision = world.revision();
    let source = world.committed_source().clone();
    let config = world.config();

    // 悬空路线引用：车辆指向不存在的局部路线 ID（合同 §5「悬空引用」）。
    let mut dangling_route = world.capture_snapshot().expect("capture");
    dangling_route.vehicles[0].snapshot_route_id = dangling_route.routes[0].snapshot_route_id + 99;
    assert!(matches!(
        restore_lfrs(
            &encode_lfrs(&dangling_route),
            Arc::clone(&revision),
            source.clone(),
            config,
            generous_limits(),
        ),
        Err(SnapshotRestoreError::UnknownRouteReference { .. })
    ));

    // live 序含未知车辆：非零但不指向任何快照车辆。
    let mut unknown_live = world.capture_snapshot().expect("capture");
    unknown_live.live_order[0] = 99;
    assert!(matches!(
        restore_lfrs(
            &encode_lfrs(&unknown_live),
            Arc::clone(&revision),
            source.clone(),
            config,
            generous_limits(),
        ),
        Err(SnapshotRestoreError::UnknownLiveOrderVehicle { .. })
    ));

    // live 序缺项：长度小于活跃车辆数（合同 §5「精确排列」）。
    let mut incomplete_live = world.capture_snapshot().expect("capture");
    incomplete_live.live_order.pop();
    assert!(matches!(
        restore_lfrs(
            &encode_lfrs(&incomplete_live),
            Arc::clone(&revision),
            source,
            config,
            generous_limits(),
        ),
        Err(SnapshotRestoreError::IncompleteLiveOrder)
    ));
}

#[test]
fn unknown_parking_space_and_participant_class_fail_closed() {
    let (mut world, route, _) = world_with_vehicle(true);
    world
        .spawn_parked_vehicle(
            ParkedVehicleSpawnInput::new(
                laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                route,
                0,
                10_000,
            ),
            ParkingTarget::ExplicitSpace(laneflow_static_contract::ParkingSpaceOrdinal::from_raw(
                0,
            )),
        )
        .expect("parked second");
    let revision = world.revision();
    let source = world.committed_source().clone();
    let config = world.config();

    // 未知停车位稳定标识：绑定一致（Parked + Some）但 ID 不解析。
    let mut unknown_space = world.capture_snapshot().expect("capture");
    unknown_space.vehicles[1].parking = Some(CapturedParkingBinding::Occupied {
        target: CapturedParkingTarget::ExplicitSpace(
            laneflow_static_contract::StableId128::from_bytes([0xAB; 16]),
        ),
    });
    assert!(matches!(
        restore_lfrs(
            &encode_lfrs(&unknown_space),
            Arc::clone(&revision),
            source.clone(),
            config,
            generous_limits(),
        ),
        Err(SnapshotRestoreError::UnknownParkingSpace { .. })
    ));

    // 未知参与者类别稳定标识：profile 可解析、class 不解析。
    let mut unknown_class = world.capture_snapshot().expect("capture");
    unknown_class.vehicles[0].class = laneflow_static_contract::StableId128::from_bytes([0xCD; 16]);
    assert!(matches!(
        restore_lfrs(
            &encode_lfrs(&unknown_class),
            revision,
            source,
            config,
            generous_limits(),
        ),
        Err(SnapshotRestoreError::UnknownParticipantClass { .. })
    ));
}

#[test]
fn vehicle_identity_value_and_overlap_invariants_fail_closed() {
    let (world, _, _) = world_with_vehicle(true);
    let revision = world.revision();
    let source = world.committed_source().clone();
    let config = world.config();

    let mut unknown_profile = world.capture_snapshot().expect("capture");
    unknown_profile.vehicles[0].profile = StableId128::from_bytes([0xff; 16]);
    assert!(matches!(
        restore_lfrs(
            &encode_lfrs(&unknown_profile),
            Arc::clone(&revision),
            source.clone(),
            config,
            generous_limits(),
        ),
        Err(SnapshotRestoreError::UnknownVehicleProfile { .. })
    ));

    let mut invalid_carry = world.capture_snapshot().expect("capture");
    invalid_carry.vehicles[0].carry_um = 1_000;
    assert!(matches!(
        restore_lfrs(
            &encode_lfrs(&invalid_carry),
            Arc::clone(&revision),
            source.clone(),
            config,
            generous_limits(),
        ),
        Err(SnapshotRestoreError::CarryOutOfRange { .. })
    ));

    let mut invalid_completed = world.capture_snapshot().expect("capture");
    invalid_completed.vehicles[0].status = VehicleStatus::Completed;
    assert!(matches!(
        restore_lfrs(
            &encode_lfrs(&invalid_completed),
            Arc::clone(&revision),
            source.clone(),
            config,
            generous_limits(),
        ),
        Err(SnapshotRestoreError::InvalidCompletedState { .. })
    ));

    let mut overlap = world.capture_snapshot().expect("capture");
    let mut duplicate = overlap.vehicles[0].clone();
    duplicate.snapshot_vehicle_id = 2;
    overlap.vehicles.push(duplicate);
    overlap.live_order.push(2);
    assert!(matches!(
        restore_lfrs(
            &encode_lfrs(&overlap),
            revision,
            source,
            config,
            generous_limits(),
        ),
        Err(SnapshotRestoreError::Vehicle {
            error: SpawnError::Overlap,
            ..
        })
    ));
}

#[test]
fn closed_versions_bindings_and_enums_reject_unknown_values() {
    let (world, _, _) = world_with_vehicle(true);
    let revision = world.revision();
    let source = world.committed_source().clone();
    let config = world.config();
    let valid = encode_lfrs(&world.capture_snapshot().expect("capture"));

    let mut prior_format = valid.clone();
    let format_offset = {
        let root =
            wire::size_prefixed_root_as_runtime_snapshot(&prior_format).expect("verified LFRS");
        table_field_offset(root._tab, wire::RuntimeSnapshot::VT_FORMAT_VERSION)
    };
    prior_format[format_offset..format_offset + 4].copy_from_slice(&4_u32.to_le_bytes());
    assert_eq!(
        restore_lfrs(
            &prior_format,
            Arc::clone(&revision),
            source.clone(),
            config,
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::UnsupportedFormatVersion { actual: 4 }
    );

    let mut unknown_format = valid.clone();
    unknown_format[format_offset..format_offset + 4].copy_from_slice(&6_u32.to_le_bytes());
    assert_eq!(
        restore_lfrs(
            &unknown_format,
            Arc::clone(&revision),
            source.clone(),
            config,
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::UnsupportedFormatVersion { actual: 6 }
    );

    let mut prior_runtime = valid.clone();
    let runtime_offset = {
        let root =
            wire::size_prefixed_root_as_runtime_snapshot(&prior_runtime).expect("verified LFRS");
        table_field_offset(root._tab, wire::RuntimeSnapshot::VT_RUNTIME_STATE_VERSION)
    };
    prior_runtime[runtime_offset..runtime_offset + 2].copy_from_slice(&4_u16.to_le_bytes());
    assert_eq!(
        restore_lfrs(
            &prior_runtime,
            Arc::clone(&revision),
            source.clone(),
            config,
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::UnsupportedRuntimeStateVersion { actual: 4 }
    );

    let mut unknown_runtime = valid.clone();
    unknown_runtime[runtime_offset..runtime_offset + 2].copy_from_slice(&6_u16.to_le_bytes());
    assert_eq!(
        restore_lfrs(
            &unknown_runtime,
            Arc::clone(&revision),
            source.clone(),
            config,
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::UnsupportedRuntimeStateVersion { actual: 6 }
    );

    let mut unknown_fields = valid.clone();
    append_zero_root_vtable_field(&mut unknown_fields);
    assert_eq!(
        restore_lfrs(
            &unknown_fields,
            Arc::clone(&revision),
            source.clone(),
            config,
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::UnknownTableFields {
            table: "RuntimeSnapshot",
            supported: ROOT_V5_FIELDS,
            actual: ROOT_V5_FIELDS + 4,
        }
    );

    let mut missing_published = valid.clone();
    clear_root_table_field(
        &mut missing_published,
        wire::RuntimeSnapshot::VT_SOURCE_PUBLISHED,
    );
    assert_eq!(
        restore_lfrs(
            &missing_published,
            Arc::clone(&revision),
            source.clone(),
            config,
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::MissingField {
            field: "source_published",
        }
    );

    let mut missing_required_routes = valid.clone();
    clear_root_table_field(
        &mut missing_required_routes,
        wire::RuntimeSnapshot::VT_ROUTES,
    );
    assert_eq!(
        restore_lfrs(
            &missing_required_routes,
            Arc::clone(&revision),
            source.clone(),
            config,
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::InvalidFlatbuffer
    );

    let mut unknown_source = valid.clone();
    let source_kind_offset = {
        let root =
            wire::size_prefixed_root_as_runtime_snapshot(&unknown_source).expect("verified LFRS");
        table_field_offset(root._tab, wire::RuntimeSnapshot::VT_SOURCE_KIND)
    };
    unknown_source[source_kind_offset] = 0xff;
    assert_eq!(
        restore_lfrs(
            &unknown_source,
            Arc::clone(&revision),
            source.clone(),
            config,
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::UnsupportedSourceKind { actual: 0xff }
    );

    let mut unknown_status = valid.clone();
    let status_offset = {
        let root =
            wire::size_prefixed_root_as_runtime_snapshot(&unknown_status).expect("verified LFRS");
        table_field_offset(
            root.vehicles().get(0)._tab,
            wire::SnapshotVehicle::VT_STATUS,
        )
    };
    unknown_status[status_offset] = 0xff;
    assert!(matches!(
        restore_lfrs(
            &unknown_status,
            Arc::clone(&revision),
            source.clone(),
            config,
            generous_limits(),
        ),
        Err(SnapshotRestoreError::InvalidVehicleStatus { actual: 0xff, .. })
    ));

    let (parking_world, _, _) = virtual_parking_cutover_world();
    let parking_revision = parking_world.revision();
    let parking_source = parking_world.committed_source().clone();
    let parking_config = parking_world.config();
    let parking_valid = encode_lfrs(&parking_world.capture_snapshot().expect("parking capture"));
    let mut unknown_parking_state = parking_valid.clone();
    let (parking_state_offset, parking_vehicle_id) = {
        let root = wire::size_prefixed_root_as_runtime_snapshot(&unknown_parking_state)
            .expect("verified parking LFRS");
        let vehicle = root.vehicles().get(0);
        let binding = vehicle.parking().expect("parking binding");
        (
            table_field_offset(binding._tab, wire::ParkingBinding::VT_STATE),
            vehicle.snapshot_vehicle_id(),
        )
    };
    unknown_parking_state[parking_state_offset] = 0xff;
    assert_eq!(
        restore_lfrs(
            &unknown_parking_state,
            Arc::clone(&parking_revision),
            parking_source.clone(),
            parking_config,
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::InvalidParkingBindingState {
            snapshot_vehicle_id: parking_vehicle_id,
            actual: 0xff,
        }
    );

    let mut unknown_parking_kind = parking_valid;
    let parking_kind_offset = {
        let root = wire::size_prefixed_root_as_runtime_snapshot(&unknown_parking_kind)
            .expect("verified parking LFRS");
        let binding = root.vehicles().get(0).parking().expect("parking binding");
        table_field_offset(binding._tab, wire::ParkingBinding::VT_TARGET_KIND)
    };
    unknown_parking_kind[parking_kind_offset] = 0xff;
    assert_eq!(
        restore_lfrs(
            &unknown_parking_kind,
            parking_revision,
            parking_source,
            parking_config,
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::InvalidParkingTargetKind {
            snapshot_vehicle_id: parking_vehicle_id,
            actual: 0xff,
        }
    );

    let mut wrong_revision = valid.clone();
    let revision_offset = {
        let root =
            wire::size_prefixed_root_as_runtime_snapshot(&wrong_revision).expect("verified LFRS");
        table_field_offset(root._tab, wire::RuntimeSnapshot::VT_NETWORK_REVISION)
    };
    wrong_revision[revision_offset] ^= 1;
    assert_eq!(
        restore_lfrs(
            &wrong_revision,
            Arc::clone(&revision),
            source.clone(),
            config,
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::NetworkRevisionMismatch
    );

    let mut wrong_contract = valid.clone();
    let contract_offset = {
        let root =
            wire::size_prefixed_root_as_runtime_snapshot(&wrong_contract).expect("verified LFRS");
        table_field_offset(
            root._tab,
            wire::RuntimeSnapshot::VT_STATIC_CONTRACT_VERSIONS,
        )
    };
    wrong_contract[contract_offset] ^= 1;
    assert_eq!(
        restore_lfrs(
            &wrong_contract,
            Arc::clone(&revision),
            source.clone(),
            config,
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::StaticContractVersionsMismatch
    );

    let mut wrong_source_revision = valid.clone();
    let source_revision_offset = {
        let root = wire::size_prefixed_root_as_runtime_snapshot(&wrong_source_revision)
            .expect("verified LFRS");
        table_field_offset(
            root.source_published().expect("published")._tab,
            wire::PublishedSourceBinding::VT_NETWORK_REVISION,
        )
    };
    wrong_source_revision[source_revision_offset] ^= 1;
    assert_eq!(
        restore_lfrs(
            &wrong_source_revision,
            revision,
            source,
            config,
            generous_limits(),
        )
        .unwrap_err(),
        SnapshotRestoreError::SnapshotSourceRevisionMismatch
    );

    // 事件游标随切片 C 事件批次通道成为真实轴：非零值恢复为世界状态。
    let mut event_cursor = world.capture_snapshot().expect("capture");
    event_cursor.event_cursor = 7;
    let restored = restore_lfrs(
        &encode_lfrs(&event_cursor),
        world.revision(),
        world.committed_source().clone(),
        config,
        generous_limits(),
    )
    .unwrap()
    .into_world();
    assert_eq!(restored.event_cursor(), 7);
}

fn table_field_offset(
    table: laneflow_runtime_snapshot_wire::runtime::Table<'_>,
    field: laneflow_runtime_snapshot_wire::runtime::VOffsetT,
) -> usize {
    let relative = usize::from(table.vtable().get(field));
    assert_ne!(relative, 0, "fixture field must be present");
    table.loc() + relative
}

fn root_vtable_start(bytes: &[u8]) -> usize {
    let root = wire::size_prefixed_root_as_runtime_snapshot(bytes).expect("verified LFRS");
    let table = root._tab.loc();
    let backwards = i32::from_le_bytes(
        bytes[table..table + 4]
            .try_into()
            .expect("root table vtable offset"),
    );
    assert!(backwards > 0);
    table - usize::try_from(backwards).expect("positive vtable offset")
}

fn clear_root_table_field(
    bytes: &mut [u8],
    field: laneflow_runtime_snapshot_wire::runtime::VOffsetT,
) {
    let vtable = root_vtable_start(bytes);
    let entry = vtable + usize::from(field);
    bytes[entry..entry + 2].copy_from_slice(&0_u16.to_le_bytes());
}

fn append_zero_root_vtable_field(bytes: &mut Vec<u8>) {
    let (vtable, table, backwards) = {
        let root = wire::size_prefixed_root_as_runtime_snapshot(bytes).expect("verified LFRS");
        let table = root._tab.loc();
        let backwards = i32::from_le_bytes(
            bytes[table..table + 4]
                .try_into()
                .expect("root table vtable offset"),
        );
        (root_vtable_start(bytes), table, backwards)
    };
    let current_bytes = u16::from_le_bytes(
        bytes[vtable..vtable + 2]
            .try_into()
            .expect("vtable byte length"),
    );
    let extended_bytes = current_bytes
        .checked_add(8)
        .expect("four extra fields preserve root table alignment");
    let extra = vtable + usize::from(current_bytes);
    bytes.splice(extra..extra, [0_u8; 8]);
    let declared = u32::from_le_bytes(bytes[..4].try_into().expect("size prefix"));
    bytes[..4].copy_from_slice(
        &declared
            .checked_add(8)
            .expect("extended size prefix")
            .to_le_bytes(),
    );
    let root_offset = u32::from_le_bytes(bytes[4..8].try_into().expect("root offset"));
    bytes[4..8].copy_from_slice(
        &root_offset
            .checked_add(8)
            .expect("shifted root offset")
            .to_le_bytes(),
    );
    bytes[table + 8..table + 12].copy_from_slice(
        &backwards
            .checked_add(8)
            .expect("shifted vtable offset")
            .to_le_bytes(),
    );
    bytes[vtable..vtable + 2].copy_from_slice(&extended_bytes.to_le_bytes());
}

#[test]
fn restored_routes_still_use_common_admitted_compiler() {
    let (mut world, route, _) = world_with_vehicle(true);
    let second = world
        .register_route(RouteRegisterInput::new(
            world.route_edges(route).expect("route").to_vec(),
        ))
        .expect("second route");
    let snapshot = world.capture_snapshot().expect("capture");
    let restored = restore_lfrs(
        &encode_lfrs(&snapshot),
        world.revision(),
        world.committed_source().clone(),
        world.config(),
        generous_limits(),
    )
    .expect("restore");
    assert!(restored.route_handle(1).is_some());
    assert!(restored.route_handle(2).is_some());
    assert!(restored.route_handle(3).is_none());
    assert!(world.route_edges(second).is_some());
}
