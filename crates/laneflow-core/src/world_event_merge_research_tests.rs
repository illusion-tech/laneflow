use std::collections::{BTreeSet, HashMap};

use super::*;
use crate::{
    EdgeLength, IidmProfileSpec, LaneEdge, MovementGate, ParkingRegistry, ParkingSpace,
    ParkingSpaceGeometry, SignalAspect, SignalControlInput, SignalController, SignalGroup,
    SignalGroupState, SignalPhase, SignalRegistry, SpeedLimit, StopLine, StopLineLocation,
    TickInput, VehicleProfile, VehicleProfileRegistry,
};

const VEHICLE_ADVANCE_PHASE: u8 = 0;
const SIGNAL_COMMIT_PHASE: u8 = 1;
const LOCAL_SEQUENCE_KIND_SHIFT: u32 = 56;
const LOCAL_SEQUENCE_OCCURRENCE_MASK: u64 = (1_u64 << LOCAL_SEQUENCE_KIND_SHIFT) - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CanonicalEventKey {
    tick_index: u64,
    phase_rank: u8,
    primary_stable_sequence: u64,
    local_sequence: u64,
    secondary_sequence: u64,
    domain_tiebreaker: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct KeyedEvent {
    key: CanonicalEventKey,
    event: CoreEvent,
}

#[derive(Clone, Debug)]
struct KeyedFailure {
    key: CanonicalEventKey,
    error: CoreError,
}

#[derive(Debug)]
struct StableSequences {
    vehicles: HashMap<VehicleHandle, u64>,
    controllers: HashMap<SignalControllerHandle, u64>,
    groups: HashMap<SignalGroupHandle, (u64, u64)>,
}

impl StableSequences {
    fn from_world(world: &CoreWorld) -> Self {
        let vehicles = world
            .vehicle_update_order
            .iter()
            .enumerate()
            .map(|(sequence, handle)| (handle, sequence_u64(sequence)))
            .collect();

        let mut controllers = HashMap::new();
        let mut groups = HashMap::new();
        for (controller_sequence, controller) in world.signals.controllers().enumerate() {
            let handle = world
                .signals
                .controller_handle(controller.id())
                .expect("normalized controller must resolve by external ID");
            let controller_sequence = sequence_u64(controller_sequence);
            assert!(
                controllers.insert(handle, controller_sequence).is_none(),
                "controller normalization sequence must be unique"
            );

            for (group_sequence, group) in world
                .signals
                .controller_groups(handle)
                .expect("normalized controller must own its groups")
                .iter()
                .copied()
                .enumerate()
            {
                assert!(
                    groups
                        .insert(group, (controller_sequence, sequence_u64(group_sequence)))
                        .is_none(),
                    "signal group must belong to one normalized controller"
                );
            }
        }

        Self {
            vehicles,
            controllers,
            groups,
        }
    }

    fn vehicle(&self, handle: VehicleHandle) -> u64 {
        self.vehicles
            .get(&handle)
            .copied()
            .expect("event vehicle must have a logical update sequence")
    }

    fn controller(&self, handle: SignalControllerHandle) -> u64 {
        self.controllers
            .get(&handle)
            .copied()
            .expect("event controller must have a normalization sequence")
    }

    fn group(&self, handle: SignalGroupHandle) -> (u64, u64) {
        self.groups
            .get(&handle)
            .copied()
            .expect("event group must have a controller-local normalization sequence")
    }
}

fn sequence_u64(sequence: usize) -> u64 {
    u64::try_from(sequence).expect("logical sequence must fit in u64")
}

fn local_sequence(kind: u8, occurrence: usize) -> u64 {
    let occurrence = sequence_u64(occurrence);
    assert!(
        occurrence <= LOCAL_SEQUENCE_OCCURRENCE_MASK,
        "research key reserves the high byte for local event kind"
    );
    (u64::from(kind) << LOCAL_SEQUENCE_KIND_SHIFT) | occurrence
}

fn vehicle_key(
    tick_index: u64,
    primary_stable_sequence: u64,
    local_sequence: u64,
    secondary_sequence: u64,
    domain_tiebreaker: u8,
) -> CanonicalEventKey {
    CanonicalEventKey {
        tick_index,
        phase_rank: VEHICLE_ADVANCE_PHASE,
        primary_stable_sequence,
        local_sequence,
        secondary_sequence,
        domain_tiebreaker,
    }
}

fn signal_key(
    tick_index: u64,
    primary_stable_sequence: u64,
    local_sequence: u64,
    secondary_sequence: u64,
    domain_tiebreaker: u8,
) -> CanonicalEventKey {
    CanonicalEventKey {
        tick_index,
        phase_rank: SIGNAL_COMMIT_PHASE,
        primary_stable_sequence,
        local_sequence,
        secondary_sequence,
        domain_tiebreaker,
    }
}

fn key_event(sequences: &StableSequences, event: CoreEvent) -> KeyedEvent {
    let key = match &event {
        CoreEvent::VehicleSpeedLimitProjectionApplied(event) => vehicle_key(
            event.tick_index,
            sequences.vehicle(event.vehicle),
            local_sequence(0, event.from_route_edge_index),
            0,
            0,
        ),
        CoreEvent::VehicleSignalStopProjectionApplied(event) => vehicle_key(
            event.tick_index,
            sequences.vehicle(event.vehicle),
            local_sequence(0, event.from_route_edge_index),
            0,
            1,
        ),
        CoreEvent::VehicleParkingStopProjectionApplied(event) => vehicle_key(
            event.tick_index,
            sequences.vehicle(event.vehicle),
            local_sequence(0, event.route_edge_index),
            0,
            2,
        ),
        CoreEvent::VehicleFollowingSafetyProjectionApplied(event) => vehicle_key(
            event.tick_index,
            sequences.vehicle(event.vehicle),
            local_sequence(1, 0),
            sequences.vehicle(event.leader),
            3,
        ),
        CoreEvent::VehicleChangedEdge(event) => vehicle_key(
            event.tick_index,
            sequences.vehicle(event.vehicle),
            local_sequence(2, event.from_route_edge_index),
            sequence_u64(event.to_route_edge_index),
            4,
        ),
        CoreEvent::ParkingReservationReleased(event) => vehicle_key(
            event.tick_index,
            sequences.vehicle(event.vehicle),
            local_sequence(3, 0),
            0,
            5,
        ),
        CoreEvent::VehicleParkingArrivalReached(event) => vehicle_key(
            event.tick_index,
            sequences.vehicle(event.vehicle),
            local_sequence(4, event.route_edge_index),
            0,
            6,
        ),
        CoreEvent::VehicleCompletedRoute(event) => vehicle_key(
            event.tick_index,
            sequences.vehicle(event.vehicle),
            local_sequence(5, event.route_edge_index),
            0,
            7,
        ),
        CoreEvent::SignalPhaseChanged(event) => signal_key(
            event.tick_index,
            sequences.controller(event.controller),
            local_sequence(0, 0),
            0,
            8,
        ),
        CoreEvent::SignalGroupAspectChanged(event) => {
            let (controller_sequence, group_sequence) = sequences.group(event.group);
            signal_key(
                event.tick_index,
                controller_sequence,
                local_sequence(1, usize::try_from(group_sequence).expect("group sequence")),
                group_sequence,
                9,
            )
        }
    };

    KeyedEvent { key, event }
}

fn simulated_bucket(key: CanonicalEventKey, bucket_count: usize, assignment_seed: u64) -> usize {
    let mixed = key.tick_index.wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ key
            .primary_stable_sequence
            .wrapping_mul(0xbf58_476d_1ce4_e5b9)
        ^ key.local_sequence.wrapping_mul(0x94d0_49bb_1331_11eb)
        ^ assignment_seed;
    usize::try_from(mixed % sequence_u64(bucket_count)).expect("bucket index must fit in usize")
}

fn merge_from_simulated_workers(
    world: &CoreWorld,
    events: &[CoreEvent],
    bucket_count: usize,
    assignment_seed: u64,
) -> Vec<CoreEvent> {
    assert!(bucket_count > 0, "worker bucket count must be non-zero");
    let sequences = StableSequences::from_world(world);
    let mut buckets = vec![Vec::new(); bucket_count];
    let mut unique_keys = BTreeSet::new();

    for event in events.iter().cloned() {
        let keyed = key_event(&sequences, event);
        assert!(
            unique_keys.insert(keyed.key),
            "canonical research keys must be unique within a step: {:?}",
            keyed.key
        );
        let bucket = simulated_bucket(keyed.key, bucket_count, assignment_seed);
        buckets[bucket].push(keyed);
    }

    for bucket in &mut buckets {
        bucket.reverse();
    }
    let mut completion_order: Vec<_> = buckets.into_iter().rev().flatten().collect();
    completion_order.sort_unstable_by_key(|keyed| keyed.key);
    completion_order
        .into_iter()
        .map(|keyed| keyed.event)
        .collect()
}

fn first_failure_from_simulated_workers(
    failures: &[KeyedFailure],
    bucket_count: usize,
    assignment_seed: u64,
) -> CoreError {
    assert!(bucket_count > 0, "worker bucket count must be non-zero");
    assert!(!failures.is_empty(), "failure merge requires a candidate");
    let mut buckets = vec![Vec::new(); bucket_count];
    let mut unique_keys = BTreeSet::new();

    for failure in failures.iter().cloned() {
        assert!(
            unique_keys.insert(failure.key),
            "canonical failure keys must be unique within a step: {:?}",
            failure.key
        );
        let bucket = simulated_bucket(failure.key, bucket_count, assignment_seed);
        buckets[bucket].push(failure);
    }

    for bucket in &mut buckets {
        bucket.reverse();
    }
    buckets
        .into_iter()
        .rev()
        .flatten()
        .min_by_key(|failure| failure.key)
        .expect("non-empty failure candidates")
        .error
}

fn assert_worker_count_invariant(world: &CoreWorld, events: &[CoreEvent]) {
    for bucket_count in [1, 2, 4, 7] {
        for assignment_seed in [0, 1, 0xa5a5_5a5a] {
            assert_eq!(
                merge_from_simulated_workers(world, events, bucket_count, assignment_seed),
                events,
                "canonical merge must reproduce serial semantics for bucket_count={bucket_count}, seed={assignment_seed}"
            );
        }
    }
}

fn edge(id: &str, length: f64, limit: f64, next: &[&str]) -> LaneEdge {
    LaneEdge::new(
        id,
        EdgeLength::try_new(length).expect("edge length"),
        SpeedLimit::try_new(limit).expect("speed limit"),
        next.iter().copied(),
    )
}

fn profile(id: &str, desired_speed: f64) -> (VehicleProfileRegistry, VehicleProfileHandle) {
    let profiles = VehicleProfileRegistry::try_new([VehicleProfile::try_new_iidm(
        id,
        IidmProfileSpec {
            length: 4.0,
            desired_speed,
            min_gap: 2.0,
            time_headway: 1.5,
            max_acceleration: 2.0,
            comfortable_deceleration: 2.0,
            emergency_deceleration: 8.0,
        },
    )
    .expect("vehicle profile")])
    .expect("vehicle profile registry");
    let handle = profiles.profile_handle(id).expect("vehicle profile handle");
    (profiles, handle)
}

fn phase(id: &str, duration_ms: u64, states: &[(&str, SignalAspect)]) -> SignalPhase {
    SignalPhase::new(
        id,
        duration_ms,
        states
            .iter()
            .map(|(group, aspect)| SignalGroupState::new(*group, *aspect)),
    )
}

fn two_vehicle_completion_world() -> CoreWorld {
    let graph = LaneGraph::try_new([
        edge("X0", 3.0, f64::MAX, &["X1"]),
        edge("X1", 3.0, f64::MAX, &[]),
        edge("Y0", 3.0, f64::MAX, &["Y1"]),
        edge("Y1", 3.0, f64::MAX, &[]),
    ])
    .expect("lane graph");
    let routes = [
        Route::try_new("route-x", ["X0", "X1"]).expect("route X"),
        Route::try_new("route-y", ["Y0", "Y1"]).expect("route Y"),
    ];
    let (profiles, profile) = profile("failure-profile", 10.0);
    let traffic = InitialTrafficData::try_new(graph, routes, profiles).expect("traffic data");
    CoreWorld::with_traffic_data(
        1_000,
        traffic,
        vec![
            VehicleSpawnInput::active(
                "vehicle-x",
                profile,
                "route-x",
                0,
                EdgeProgress::ZERO,
                Speed::try_new(10.0).expect("speed"),
            ),
            VehicleSpawnInput::active(
                "vehicle-y",
                profile,
                "route-y",
                0,
                EdgeProgress::ZERO,
                Speed::try_new(10.0).expect("speed"),
            ),
        ],
    )
    .expect("world")
}

fn parking_world(id: &str, progress: f64) -> CoreWorld {
    let graph = LaneGraph::try_new([edge("A", 200.0, f64::MAX, &[])]).expect("lane graph");
    let parking = ParkingRegistry::try_new(
        &graph,
        [],
        [ParkingSpace::new(
            "space",
            None,
            "A",
            20.0,
            "A",
            40.0,
            ParkingSpaceGeometry::new(-3.0, 0.0, 4.5, 2.4),
        )],
    )
    .expect("parking registry");
    let (profiles, profile) = profile("parking-profile", 30.0);
    let traffic = InitialTrafficData::try_new_with_signals_and_parking(
        graph,
        [Route::try_new("R", ["A"]).expect("route")],
        profiles,
        SignalRegistry::empty(),
        parking,
    )
    .expect("traffic data");
    CoreWorld::with_traffic_data(
        1_000,
        traffic,
        vec![VehicleSpawnInput::active(
            id,
            profile,
            "R",
            0,
            EdgeProgress::try_new(progress).expect("progress"),
            Speed::try_new(20.0).expect("speed"),
        )],
    )
    .expect("world")
}

#[test]
fn canonical_merge_preserves_route_transitions_and_simultaneous_completions() {
    let graph = LaneGraph::try_new([
        edge("C0", 3.0, f64::MAX, &["C1"]),
        edge("C1", 3.0, f64::MAX, &["C2"]),
        edge("C2", 3.0, f64::MAX, &[]),
        edge("A0", 3.0, f64::MAX, &["A1"]),
        edge("A1", 3.0, f64::MAX, &["A2"]),
        edge("A2", 3.0, f64::MAX, &[]),
        edge("B0", 3.0, f64::MAX, &["B1"]),
        edge("B1", 3.0, f64::MAX, &["B2"]),
        edge("B2", 3.0, f64::MAX, &[]),
    ])
    .expect("lane graph");
    let routes = [
        Route::try_new("route-c", ["C0", "C1", "C2"]).expect("route C"),
        Route::try_new("route-a", ["A0", "A1", "A2"]).expect("route A"),
        Route::try_new("route-b", ["B0", "B1", "B2"]).expect("route B"),
    ];
    let (profiles, profile) = profile("route-merge-profile", 10.0);
    let traffic = InitialTrafficData::try_new(graph, routes, profiles).expect("traffic data");
    let mut world = CoreWorld::with_traffic_data(
        1_000,
        traffic,
        vec![
            VehicleSpawnInput::active(
                "vehicle-c",
                profile,
                "route-c",
                0,
                EdgeProgress::ZERO,
                Speed::try_new(10.0).expect("speed"),
            ),
            VehicleSpawnInput::active(
                "vehicle-a",
                profile,
                "route-a",
                0,
                EdgeProgress::ZERO,
                Speed::try_new(10.0).expect("speed"),
            ),
            VehicleSpawnInput::active(
                "vehicle-b",
                profile,
                "route-b",
                0,
                EdgeProgress::ZERO,
                Speed::try_new(10.0).expect("speed"),
            ),
        ],
    )
    .expect("world");

    let result = world.step(TickInput::new(1_000)).expect("step");

    assert_eq!(result.events.len(), 9);
    for events in result.events.chunks_exact(3) {
        assert!(matches!(events[0], CoreEvent::VehicleChangedEdge(_)));
        assert!(matches!(events[1], CoreEvent::VehicleChangedEdge(_)));
        assert!(matches!(events[2], CoreEvent::VehicleCompletedRoute(_)));
    }
    assert_worker_count_invariant(&world, &result.events);
}

#[test]
fn canonical_merge_preserves_projection_before_transition() {
    let graph = LaneGraph::try_new([
        edge("A", 100.0, 20.0, &["B"]),
        edge("B", 100.0, 5.0, &["A"]),
    ])
    .expect("lane graph");
    let route = Route::try_new("R", ["A", "B", "A", "B"]).expect("route");
    let (profiles, profile) = profile("projection-profile", 30.0);
    let traffic = InitialTrafficData::try_new(graph, [route], profiles).expect("traffic data");
    let mut world = CoreWorld::with_traffic_data(
        1_000,
        traffic,
        vec![VehicleSpawnInput::active(
            "V",
            profile,
            "R",
            2,
            EdgeProgress::try_new(99.0).expect("progress"),
            Speed::try_new(20.0).expect("speed"),
        )],
    )
    .expect("world");

    let result = world.step(TickInput::new(1_000)).expect("step");

    assert!(matches!(
        result.events.as_slice(),
        [
            CoreEvent::VehicleSpeedLimitProjectionApplied(_),
            CoreEvent::VehicleChangedEdge(_)
        ]
    ));
    assert_worker_count_invariant(&world, &result.events);
}

#[test]
fn canonical_merge_preserves_following_projection_before_transition() {
    let graph = LaneGraph::try_new([
        edge("A", 5.0, f64::MAX, &["B"]),
        edge("B", 100.0, f64::MAX, &[]),
    ])
    .expect("lane graph");
    let route = Route::try_new("R", ["A", "B"]).expect("route");
    let (profiles, profile) = profile("following-profile", 20.0);
    let traffic = InitialTrafficData::try_new(graph, [route], profiles).expect("traffic data");
    let mut world = CoreWorld::with_traffic_data(
        1_000,
        traffic,
        vec![
            VehicleSpawnInput::active(
                "follower",
                profile,
                "R",
                0,
                EdgeProgress::try_new(4.0).expect("progress"),
                Speed::try_new(20.0).expect("speed"),
            ),
            VehicleSpawnInput::stopped(
                "leader",
                profile,
                "R",
                1,
                EdgeProgress::try_new(6.0).expect("progress"),
            ),
        ],
    )
    .expect("world");

    let result = world.step(TickInput::new(1_000)).expect("step");

    assert!(matches!(
        result.events.as_slice(),
        [
            CoreEvent::VehicleFollowingSafetyProjectionApplied(_),
            CoreEvent::VehicleChangedEdge(_)
        ]
    ));
    assert_worker_count_invariant(&world, &result.events);
}

#[test]
fn canonical_merge_preserves_controller_and_group_normalization_order() {
    let graph = LaneGraph::try_new([
        edge("a", 10.0, f64::MAX, &["b"]),
        edge("b", 10.0, f64::MAX, &[]),
        edge("c", 10.0, f64::MAX, &["d"]),
        edge("d", 10.0, f64::MAX, &[]),
        edge("e", 10.0, f64::MAX, &["f"]),
        edge("f", 10.0, f64::MAX, &[]),
    ])
    .expect("lane graph");
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
                        1_000,
                        &[("g1", SignalAspect::Red), ("g2", SignalAspect::Yellow)],
                    ),
                    phase(
                        "c1-new",
                        1_000,
                        &[("g1", SignalAspect::Green), ("g2", SignalAspect::Red)],
                    ),
                ],
            ),
            SignalController::new_fixed_time(
                "c2",
                0,
                ["g3"],
                [
                    phase("c2-old", 1_000, &[("g3", SignalAspect::Red)]),
                    phase("c2-new", 1_000, &[("g3", SignalAspect::Green)]),
                ],
            ),
        ],
        [
            MovementGate::new("a", "b", "sa", SignalControlInput::Group("g1".to_owned())),
            MovementGate::new("c", "d", "sc", SignalControlInput::Group("g2".to_owned())),
            MovementGate::new("e", "f", "se", SignalControlInput::Group("g3".to_owned())),
        ],
    )
    .expect("signal registry");
    let (profiles, profile) = profile("signal-projection-profile", 20.0);
    let traffic = InitialTrafficData::try_new_with_signals(
        graph,
        [Route::try_new("controlled-route", ["a", "b"]).expect("controlled route")],
        profiles,
        signals,
    )
    .expect("traffic data");
    let mut world = CoreWorld::with_traffic_data(
        1_000,
        traffic,
        vec![VehicleSpawnInput::active(
            "controlled-vehicle",
            profile,
            "controlled-route",
            0,
            EdgeProgress::try_new(5.0).expect("progress"),
            Speed::try_new(20.0).expect("speed"),
        )],
    )
    .expect("signal world");

    let result = world.step(TickInput::new(1_000)).expect("step");

    assert!(matches!(
        result.events.as_slice(),
        [
            CoreEvent::VehicleSignalStopProjectionApplied(_),
            CoreEvent::SignalPhaseChanged(_),
            CoreEvent::SignalGroupAspectChanged(_),
            CoreEvent::SignalGroupAspectChanged(_),
            CoreEvent::SignalPhaseChanged(_),
            CoreEvent::SignalGroupAspectChanged(_)
        ]
    ));
    assert_worker_count_invariant(&world, &result.events);
}

#[test]
fn canonical_merge_preserves_parking_projection_and_arrival_order() {
    let mut world = parking_world("arriving", 15.0);
    let vehicle = world.vehicle_handle("arriving").expect("vehicle");
    let space = world.parking().space_handle("space").expect("space");
    world
        .reserve_parking_space(vehicle, space)
        .expect("reservation");

    let result = world.step(TickInput::new(1_000)).expect("step");

    assert!(matches!(
        result.events.as_slice(),
        [
            CoreEvent::VehicleParkingStopProjectionApplied(_),
            CoreEvent::VehicleParkingArrivalReached(_)
        ]
    ));
    assert_worker_count_invariant(&world, &result.events);
}

#[test]
fn canonical_merge_preserves_parking_release_before_completion() {
    let mut world = parking_world("completing", 199.0);
    let vehicle = world.vehicle_handle("completing").expect("vehicle");
    let space = world.parking().space_handle("space").expect("space");
    world
        .reserve_parking_space(vehicle, space)
        .expect("dormant reservation");

    let result = world.step(TickInput::new(1_000)).expect("step");

    assert!(matches!(
        result.events.as_slice(),
        [
            CoreEvent::ParkingReservationReleased(_),
            CoreEvent::VehicleCompletedRoute(_)
        ]
    ));
    assert_worker_count_invariant(&world, &result.events);
}

#[test]
fn failed_advance_discards_candidate_events_and_retry_replays_canonical_stream() {
    let base = two_vehicle_completion_world();
    let sequences = StableSequences::from_world(&base);
    let first_vehicle = base.vehicle_handle("vehicle-x").expect("vehicle X");
    let failure_vehicle = base.vehicle_handle("vehicle-y").expect("vehicle Y");
    let mut failure_candidates = Vec::new();

    for vehicle in [failure_vehicle, first_vehicle] {
        let mut candidate = base.clone();
        candidate.step_failure_after_vehicle = Some(vehicle);
        let before_failure = candidate.clone();
        let error = candidate
            .step(TickInput::new(1_000))
            .expect_err("injected advance failure");
        assert_eq!(candidate, before_failure);
        failure_candidates.push(KeyedFailure {
            key: vehicle_key(
                1,
                sequences.vehicle(vehicle),
                local_sequence(u8::MAX, 0),
                0,
                u8::MAX,
            ),
            error,
        });
    }

    for bucket_count in [1, 2, 4, 7] {
        for assignment_seed in [0, 1, 0xa5a5_5a5a] {
            let first_error = first_failure_from_simulated_workers(
                &failure_candidates,
                bucket_count,
                assignment_seed,
            );
            assert!(matches!(
                first_error,
                CoreError::ParkingBindingInvariantViolation {
                    stage: "test_after_vehicle_advance",
                    vehicle: Some(actual),
                    space: None,
                } if actual == first_vehicle
            ));
        }
    }

    let mut world = base.clone();
    let mut replay = world.clone();
    world.step_failure_after_vehicle = Some(failure_vehicle);
    let before_failure = world.clone();

    let error = world
        .step(TickInput::new(1_000))
        .expect_err("injected advance failure");

    assert!(matches!(
        error,
        CoreError::ParkingBindingInvariantViolation {
            stage: "test_after_vehicle_advance",
            vehicle: Some(actual),
            space: None,
        } if actual == failure_vehicle
    ));
    assert_eq!(world, before_failure);

    world.step_failure_after_vehicle = None;
    let retry = world.step(TickInput::new(1_000)).expect("retry");
    let expected = replay.step(TickInput::new(1_000)).expect("clean replay");

    assert_eq!(retry, expected);
    assert_eq!(world, replay);
    assert_worker_count_invariant(&world, &retry.events);
}
