use std::sync::Arc;

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
use laneflow_static_contract::{LaneEdgeOrdinal, ManeuverPathOrdinal};
use laneflow_static_network::{
    SharedNetworkBuildLimits, SharedNetworkBuildOptions, SharedNetworkRevision, SpatialBuildOption,
    build_shared_network_revision,
};

/// 正式编译带等待区的降速路线，不修改已编译内部表来制造成功命中。
pub fn distant_drop_fixture() -> (Arc<SharedNetworkRevision>, Vec<LaneEdgeOrdinal>) {
    let limits = CompileLimits::p100_initial_v2();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: "runtime-fixture-policy",
            source_document_key: "admission-distant-drop.document",
            generator_build_id: "admission-distant-drop-v1",
            parameters_and_inputs_digest: [0x34; 32],
            frontend_options_digest: [0; 32],
            random_seed: Some(834),
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
                desired_speed_meters_per_second: 13.0,
                min_gap_meters: 2.0,
                time_headway_seconds: 1.5,
                max_acceleration_meters_per_second_squared: 2.0,
                comfortable_deceleration_meters_per_second_squared: 2.0,
                emergency_deceleration_meters_per_second_squared: 4.0,
            },
        })
        .unwrap();
    for (edge, next, length, speed) in [
        ("approach", Some("entry"), 1_000.0, 13.0),
        ("entry", Some("storage"), 13.0, 13.0),
        ("storage", Some("exit"), 50.0, 1.0),
        ("exit", None, 100.0, 13.0),
    ] {
        let successors: Vec<_> = next.into_iter().map(LaneEdgeReference::local).collect();
        module
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: edge,
                length_meters: length,
                speed_limit_meters_per_second: speed,
                successors: &successors,
            })
            .unwrap();
    }
    module
        .add_junction(JunctionInput {
            junction_key: "junction",
        })
        .unwrap()
        .add_movement(MovementInput {
            turn_direction: None,
            movement_key: "movement",
            junction: JunctionReference::local("junction"),
            directed_entry_approach_key: "in",
            directed_exit_approach_key: "out",
        })
        .unwrap()
        .add_maneuver_path(ManeuverPathInput {
            maneuver_path_key: "path",
            movement: MovementReference::local("movement"),
            entry_edge: LaneEdgeReference::local("entry"),
            internal_edges: &[LaneEdgeReference::local("storage")],
            exit_edge: LaneEdgeReference::local("exit"),
        })
        .unwrap();
    for (index, edge, gate) in [(0, "entry", "entry-gate"), (1, "storage", "release-gate")] {
        module
            .add_stop_line(StopLineInput {
                stop_line_key: edge,
                lane_edge: LaneEdgeReference::local(edge),
            })
            .unwrap()
            .add_maneuver_gate(ManeuverGateInput {
                maneuver_gate_key: gate,
                maneuver_path: ManeuverPathReference::local("path"),
                transition_index: index,
                stop_line: StopLineReference::local(edge),
                signal_control: SignalControlInput::None,
            })
            .unwrap();
    }
    module
        .add_waiting_zone(WaitingZoneInput {
            waiting_zone_key: "waiting",
            maneuver_path: ManeuverPathReference::local("path"),
            entry_gate: ManeuverGateReference::local("entry-gate"),
            release_gate: ManeuverGateReference::local("release-gate"),
            max_occupancy: 8,
        })
        .unwrap();
    super::test_policy::add_gate_policy(
        &mut module,
        "fixture-policy",
        &[
            ("entry-gate", GateInterpretation::Uncontrolled),
            ("release-gate", GateInterpretation::Uncontrolled),
        ],
    );
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(module.finish().unwrap()).unwrap();
    let compiled = Compiler::new().compile(unit.build().unwrap()).unwrap();
    let candidate = emit_portable_candidate(
        &compiled,
        &PortableEmissionProvenance::try_new("admission-distant-drop-v1").unwrap(),
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
    let revision = build_shared_network_revision(
        checked.canonical_network_input(),
        SharedNetworkBuildOptions::new(
            SpatialBuildOption::Omit,
            SharedNetworkBuildLimits::new(64 * 1_024 * 1_024, 16 * 1_024 * 1_024),
        ),
    )
    .unwrap();
    let mut edges = revision
        .traffic()
        .maneuvers()
        .maneuver_path(ManeuverPathOrdinal::from_raw(0))
        .unwrap()
        .edges()
        .to_vec();
    let approach = *revision
        .traffic()
        .predecessors(edges[0])
        .unwrap()
        .first()
        .unwrap();
    edges.insert(0, approach);
    (revision, edges)
}
