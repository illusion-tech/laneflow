//! #675 未插桩生产库的 Waiting 路线长度 A/B；不是城市产品认证。
#[path = "support/policy.rs"]
mod test_policy;

use laneflow_compiler::{
    CompilationUnitBuilder, CompileLimits, Compiler, GateInterpretation, IidmVehicleProfileInput,
    JunctionInput, JunctionReference, LaneEdgeInput, LaneEdgeReference, ManeuverGateInput,
    ManeuverGateReference, ManeuverPathInput, ManeuverPathReference, MovementInput,
    MovementReference, ParticipantClassInput, ParticipantClassReference, PortableDiffBase,
    PortableEmissionProvenance, SignalControlInput, SourceModuleHeader, SourceModuleHeaderInput,
    StopLineInput, StopLineReference, SyntheticModuleBuilder, VehicleProfileInput,
    WaitingZoneInput, emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_post_emission_bundle};
use laneflow_runtime::{
    CommittedNetworkSource, PublishedLfcaReference, RouteRegisterInput, TickInput, TrafficWorld,
    VehicleSpawnInput, WaitingDecisionOutcome, WorldConfig, deterministic_state_digest,
};
use laneflow_static_contract::{ManeuverPathOrdinal, VehicleProfileOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};
use sha2::{Digest, Sha256};
use std::{hint::black_box, sync::Arc, time::Instant};

const WARMUP: usize = 128;
const SAMPLES: usize = 4_096;

fn revision() -> Arc<SharedNetworkRevision> {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "city/waiting-scale",
            source_document_key: "waiting-lookup.document",
            generator_build_id: "waiting-lookup-675-v1",
            parameters_and_inputs_digest: [0x67; 32],
            frontend_options_digest: [0x75; 32],
            random_seed: Some(675),
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .unwrap();
    let mut module = SyntheticModuleBuilder::new(header, &limits).unwrap();
    module
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "road-user",
            extends: None,
        })
        .unwrap()
        .add_vehicle_profile(VehicleProfileInput {
            vehicle_profile_key: "car",
            participant_class: ParticipantClassReference::local("road-user"),
            iidm: IidmVehicleProfileInput {
                length_meters: 4.5,
                desired_speed_meters_per_second: 13.75,
                min_gap_meters: 2.0,
                time_headway_seconds: 1.4,
                max_acceleration_meters_per_second_squared: 1.8,
                comfortable_deceleration_meters_per_second_squared: 2.0,
                emergency_deceleration_meters_per_second_squared: 4.5,
            },
        })
        .unwrap();
    for (key, length, next) in [
        ("entry", 10_000.0, "storage"),
        ("storage", 8.0, "after-release"),
        ("after-release", 12.0, "exit"),
        ("exit", 12.0, "entry"),
    ] {
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: key,
                length_meters: length,
                speed_limit_meters_per_second: 13.75,
                successors: &[LaneEdgeReference::local(next)],
            })
            .unwrap();
    }
    module
        .add_junction(JunctionInput {
            junction_key: "junction",
        })
        .unwrap()
        .add_movement(MovementInput {
            movement_key: "movement",
            junction: JunctionReference::local("junction"),
            directed_entry_approach_key: "in",
            directed_exit_approach_key: "out",
            turn_direction: None,
        })
        .unwrap()
        .add_maneuver_path(ManeuverPathInput {
            maneuver_path_key: "path",
            movement: MovementReference::local("movement"),
            entry_edge: LaneEdgeReference::local("entry"),
            internal_edges: &[
                LaneEdgeReference::local("storage"),
                LaneEdgeReference::local("after-release"),
            ],
            exit_edge: LaneEdgeReference::local("exit"),
        })
        .unwrap();
    for (gate, stop, edge, transition) in [
        ("gate-entry", "stop-entry", "entry", 0),
        ("gate-release", "stop-release", "storage", 1),
    ] {
        module
            .add_stop_line(StopLineInput {
                stop_line_key: stop,
                lane_edge: LaneEdgeReference::local(edge),
            })
            .unwrap()
            .add_maneuver_gate(ManeuverGateInput {
                maneuver_gate_key: gate,
                maneuver_path: ManeuverPathReference::local("path"),
                transition_index: transition,
                stop_line: StopLineReference::local(stop),
                signal_control: SignalControlInput::None,
            })
            .unwrap();
    }
    module
        .add_waiting_zone(WaitingZoneInput {
            waiting_zone_key: "waiting",
            maneuver_path: ManeuverPathReference::local("path"),
            entry_gate: ManeuverGateReference::local("gate-entry"),
            release_gate: ManeuverGateReference::local("gate-release"),
            max_occupancy: 1,
        })
        .unwrap();
    test_policy::add_gate_policy(
        &mut module,
        "waiting-policy",
        &[
            ("gate-entry", GateInterpretation::Uncontrolled),
            ("gate-release", GateInterpretation::Uncontrolled),
        ],
    );
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(module.finish().unwrap()).unwrap();
    let output = Compiler::new().compile(unit.build().unwrap()).unwrap();
    let candidate = emit_portable_candidate(
        &output,
        &PortableEmissionProvenance::try_new("waiting-lookup-675-v1").unwrap(),
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .unwrap();
    let checked = check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .unwrap();
    build_shared_network_revision(
        checked.canonical_network_input(),
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .unwrap()
}

fn run(count: usize, samples: usize, warmup: usize) {
    let revision = revision();
    let origin = *revision.canonical_origin();
    let mut world = TrafficWorld::install(
        Arc::clone(&revision),
        WorldConfig::new(1, 1, 8_192, 1_024, 4),
        laneflow_runtime::ExecutionConfig::new(std::num::NonZeroU32::MIN),
        CommittedNetworkSource::Published {
            reference: PublishedLfcaReference::new(
                "fixture://waiting-lookup-675",
                origin.canonical_artifact_digest(),
                origin.canonical_artifact_byte_length(),
                origin.network_revision(),
            )
            .unwrap(),
        },
        675,
        test_policy::selection(&revision),
    )
    .unwrap();
    let path = revision
        .traffic()
        .maneuvers()
        .maneuver_path(ManeuverPathOrdinal::from_raw(0))
        .unwrap();
    let edges = path.edges().repeat(count);
    let entry_length = revision.traffic().lane_lengths_millimetres()[edges[0].index()];
    let route = world
        .register_route(RouteRegisterInput::new(edges))
        .unwrap();
    let input = VehicleSpawnInput::new(
        VehicleProfileOrdinal::from_raw(0),
        route,
        0,
        entry_length - 1,
        10_000,
    );
    let mut spawn_ns = Vec::with_capacity(samples);
    let mut step_ns = Vec::with_capacity(samples);
    let mut trace = Sha256::new();
    let before = format!(
        "{:x}",
        deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap()
    );
    for index in 0..warmup + samples {
        let start = Instant::now();
        let vehicle = black_box(world.spawn_vehicle(black_box(input)).unwrap());
        let spawn_elapsed = start.elapsed().as_nanos();
        let start = Instant::now();
        black_box(world.step(TickInput::new(4)).unwrap());
        let step_elapsed = start.elapsed().as_nanos();
        let state = world.vehicle(vehicle).unwrap();
        assert!(state.waiting_membership().is_some());
        assert_eq!(world.live_vehicles().len(), 1);
        assert_eq!(world.latest_waiting_decisions().len(), 1);
        assert_eq!(
            world.latest_waiting_decisions()[0].outcome(),
            WaitingDecisionOutcome::Granted
        );
        trace.update(
            format!(
                "{:?}|{:?}|{:?}",
                state,
                world.latest_waiting_decisions(),
                world.latest_transition_events()
            )
            .as_bytes(),
        );
        if index >= warmup {
            spawn_ns.push(spawn_elapsed);
            step_ns.push(step_elapsed);
        }
        world.despawn_vehicle(vehicle).unwrap();
    }
    let after = format!(
        "{:x}",
        deterministic_state_digest(&world.capture_snapshot().unwrap()).unwrap()
    );
    let trace: String = trace
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    println!(
        "waiting-lookup-ab {{\"occurrences\":{count},\"samples\":{samples},\"warmup\":{warmup},\"individual\":1,\"active\":1,\"capacity\":1,\"delta_ms\":4,\"before\":\"{before}\",\"after\":\"{after}\",\"trace\":\"{trace}\",\"spawn_ns\":{spawn_ns:?},\"step_ns\":{step_ns:?}}}"
    );
}

#[test]
fn waiting_lookup_workload_preserves_admission() {
    for count in [1, 16, 256, 1_024] {
        run(count, 4, 2);
    }
}

#[test]
#[ignore = "manual release A/B against production library; one process per round"]
fn waiting_lookup_release_ab() {
    for count in [1, 16, 256, 1_024] {
        run(count, SAMPLES, WARMUP);
    }
}
