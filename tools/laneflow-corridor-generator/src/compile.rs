use laneflow_compiler::{
    AccessEffect, AccessRuleInput, AccessRuleTargetInput, AuthoringLaneInput, CanonicalFrameInput,
    CanonicalPoint3F32Input, CompilationOutput, CompilationUnitBuilder, CompileLimits, Compiler,
    CorridorElementReference, FacilityBandInput, IidmVehicleProfileInput, JunctionInput,
    LaneEdgeGeometryInput, LaneEdgeInput, LaneEdgeReference, LaneGroupInput, LaneGroupReference,
    ManeuverGateInput, ManeuverPathInput, MovementInput, MovementReference, ParticipantClassInput,
    ParticipantClassReference, PortableDiffBase, PortableEmissionProvenanceV1, RoadCorridorInput,
    RoadSectionInput, RoadSectionReference, SignalAspect, SignalControlInput,
    SignalControllerInput, SignalGroupInput, SignalGroupReference, SignalGroupStateInput,
    SignalPhaseInput, SourceModuleHeader, SourceModuleHeaderInput, StaticRouteInput, StopLineInput,
    SyntheticModuleBuilder, VehicleProfileInput, emit_portable_candidate,
};
use laneflow_format::FormatLimits;
use laneflow_scenario::signalized_corridor::{
    AUTHORING_NAMESPACE, PASSENGER_CAR_PROFILE_KEY, SHUTTLE_BUS_PROFILE_KEY,
};
use sha2::{Digest, Sha256};

use crate::Error;
use crate::config::{CorridorConfig, MIN_GAP_METERS, VEHICLE_LENGTH_METERS};
use crate::generator::{Approach, CorridorBuild, CorridorElement, CrossSectionDocs};
const SOURCE_DOCUMENT_KEY: &str = "signalized-corridor.document";
const GENERATOR_BUILD_ID: &str = "laneflow-corridor-generator";
const PROVENANCE: &str = "repository:laneflow";
const COMPILER_BUILD_ID: &str = "laneflow-corridor-generator-v1";
const FRONTEND_OPTIONS_SALT: &[u8] = b"laneflow-corridor-generator-synthetic-v1";

const SIGNAL_GROUP_SUFFIXES: [&str; 4] = [
    "main-left",
    "main-through-right",
    "secondary-left",
    "secondary-through-right",
];

pub(crate) fn compile_corridor(
    config: &CorridorConfig,
    corridor: &CorridorBuild,
    cross_section: &CrossSectionDocs,
) -> Result<CompilationOutput, Error> {
    let limits = CompileLimits::p100_initial_v1();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: AUTHORING_NAMESPACE,
            source_document_key: SOURCE_DOCUMENT_KEY,
            generator_build_id: GENERATOR_BUILD_ID,
            parameters_and_inputs_digest: config_digest(config),
            frontend_options_digest: sha256_bytes(FRONTEND_OPTIONS_SALT),
            random_seed: None,
            provenance: PROVENANCE,
        },
        &limits,
    )
    .map_err(|bundle| compile_error("source header", bundle))?;
    let mut builder = SyntheticModuleBuilder::new(header, &limits)
        .map_err(|bundle| compile_error("synthetic module", bundle))?;

    add_participant_classes(&mut builder)?;
    add_vehicle_profiles(&mut builder)?;
    add_lane_edges(&mut builder, corridor)?;
    add_junctions_and_movements(&mut builder, corridor)?;
    add_maneuver_paths(&mut builder, corridor)?;
    add_stop_lines(&mut builder, corridor)?;
    add_signal_groups_and_controllers(&mut builder, config)?;
    add_maneuver_gates(&mut builder, corridor)?;
    add_static_routes(&mut builder, corridor)?;
    add_cross_section(&mut builder, cross_section)?;
    add_access_rules(&mut builder, cross_section)?;
    add_canonical_frame(&mut builder, config, corridor)?;

    let module = builder
        .finish()
        .map_err(|bundle| compile_error("finish module", bundle))?;
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(module)
        .map_err(|bundle| compile_error("compilation unit", bundle))?;
    Compiler::new()
        .compile(
            unit.build()
                .map_err(|bundle| compile_error("build unit", bundle))?,
        )
        .map_err(|bundle| compile_error("compile", bundle))
}

pub(crate) fn emit_lfca(output: &CompilationOutput) -> Result<Vec<u8>, Error> {
    let provenance = PortableEmissionProvenanceV1::try_new(COMPILER_BUILD_ID).map_err(|error| {
        Error::Validation {
            stage: "portable provenance",
            message: format!("{error:?}"),
        }
    })?;
    let candidate = emit_portable_candidate(
        output,
        &provenance,
        FormatLimits::V1_HARD,
        PortableDiffBase::Genesis,
    )
    .map_err(|error| Error::Validation {
        stage: "emit LFCA",
        message: format!("{error:?}"),
    })?;
    Ok(candidate.canonical_artifact().bytes().to_vec())
}

fn add_participant_classes(builder: &mut SyntheticModuleBuilder) -> Result<(), Error> {
    builder
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "motorVehicle",
            extends: None,
        })
        .and_then(|builder| {
            builder.add_participant_class(ParticipantClassInput {
                participant_class_key: "car",
                extends: Some(ParticipantClassReference::local("motorVehicle")),
            })
        })
        .and_then(|builder| {
            builder.add_participant_class(ParticipantClassInput {
                participant_class_key: "bus",
                extends: Some(ParticipantClassReference::local("motorVehicle")),
            })
        })
        .map_err(|bundle| compile_error("participant class", bundle))?;
    Ok(())
}

fn add_vehicle_profiles(builder: &mut SyntheticModuleBuilder) -> Result<(), Error> {
    builder
        .add_vehicle_profile(VehicleProfileInput {
            vehicle_profile_key: PASSENGER_CAR_PROFILE_KEY,
            participant_class: ParticipantClassReference::local("car"),
            iidm: IidmVehicleProfileInput {
                length_meters: VEHICLE_LENGTH_METERS,
                desired_speed_meters_per_second: 20.0,
                min_gap_meters: MIN_GAP_METERS,
                time_headway_seconds: 1.5,
                max_acceleration_meters_per_second_squared: 1.5,
                comfortable_deceleration_meters_per_second_squared: 2.0,
                emergency_deceleration_meters_per_second_squared: 6.0,
            },
        })
        .and_then(|builder| {
            builder.add_vehicle_profile(VehicleProfileInput {
                vehicle_profile_key: SHUTTLE_BUS_PROFILE_KEY,
                participant_class: ParticipantClassReference::local("bus"),
                iidm: IidmVehicleProfileInput {
                    length_meters: 12.0,
                    desired_speed_meters_per_second: 15.0,
                    min_gap_meters: 3.0,
                    time_headway_seconds: 2.0,
                    max_acceleration_meters_per_second_squared: 1.0,
                    comfortable_deceleration_meters_per_second_squared: 1.5,
                    emergency_deceleration_meters_per_second_squared: 5.0,
                },
            })
        })
        .map_err(|bundle| compile_error("vehicle profile", bundle))?;
    Ok(())
}

fn add_lane_edges(
    builder: &mut SyntheticModuleBuilder,
    corridor: &CorridorBuild,
) -> Result<(), Error> {
    for edge in &corridor.edges {
        let successors: Vec<_> = edge
            .connections
            .iter()
            .map(|id| LaneEdgeReference::local(id.as_str()))
            .collect();
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: edge.id.as_str(),
                length_meters: edge.length(),
                speed_limit_meters_per_second: edge.speed_limit,
                successors: &successors,
            })
            .map_err(|bundle| compile_error("lane edge", bundle))?;
    }
    Ok(())
}

fn add_junctions_and_movements(
    builder: &mut SyntheticModuleBuilder,
    corridor: &CorridorBuild,
) -> Result<(), Error> {
    let mut seen_junctions = Vec::new();
    let mut seen_movements = Vec::new();
    for connector in &corridor.connectors {
        let junction_id = format!("junction-{}", connector.key.junction);
        if !seen_junctions.iter().any(|id: &String| id == &junction_id) {
            builder
                .add_junction(JunctionInput {
                    junction_key: junction_id.as_str(),
                })
                .map_err(|bundle| compile_error("junction", bundle))?;
            seen_junctions.push(junction_id.clone());
        }
        if seen_movements
            .iter()
            .any(|id: &String| id == &connector.movement_id)
        {
            continue;
        }
        let entry = directed_approach_key(connector.key.approach);
        let exit = directed_approach_key(connector.key.approach.exit(connector.key.turn));
        builder
            .add_movement(MovementInput {
                movement_key: connector.movement_id.as_str(),
                junction: laneflow_compiler::JunctionReference::local(junction_id.as_str()),
                directed_entry_approach_key: entry,
                directed_exit_approach_key: exit,
            })
            .map_err(|bundle| compile_error("movement", bundle))?;
        seen_movements.push(connector.movement_id.clone());
    }
    Ok(())
}

fn add_maneuver_paths(
    builder: &mut SyntheticModuleBuilder,
    corridor: &CorridorBuild,
) -> Result<(), Error> {
    for connector in &corridor.connectors {
        let internals = [LaneEdgeReference::local(
            connector.internal_edge_id.as_str(),
        )];
        builder
            .add_maneuver_path(ManeuverPathInput {
                maneuver_path_key: connector.maneuver_path_id.as_str(),
                movement: MovementReference::local(connector.movement_id.as_str()),
                entry_edge: LaneEdgeReference::local(connector.entry_edge_id.as_str()),
                internal_edges: &internals,
                exit_edge: LaneEdgeReference::local(connector.exit_edge_id.as_str()),
            })
            .map_err(|bundle| compile_error("maneuver path", bundle))?;
    }
    Ok(())
}

fn add_stop_lines(
    builder: &mut SyntheticModuleBuilder,
    corridor: &CorridorBuild,
) -> Result<(), Error> {
    for stop_line in &corridor.stop_lines {
        builder
            .add_stop_line(StopLineInput {
                stop_line_key: stop_line.id.as_str(),
                lane_edge: LaneEdgeReference::local(stop_line.edge_id.as_str()),
            })
            .map_err(|bundle| compile_error("stop line", bundle))?;
    }
    Ok(())
}

fn add_signal_groups_and_controllers(
    builder: &mut SyntheticModuleBuilder,
    config: &CorridorConfig,
) -> Result<(), Error> {
    for junction in 1..=2 {
        let group_ids: Vec<String> = SIGNAL_GROUP_SUFFIXES
            .iter()
            .map(|suffix| format!("signal-group-junction-{junction}-{suffix}"))
            .collect();
        for group_id in &group_ids {
            builder
                .add_signal_group(SignalGroupInput {
                    signal_group_key: group_id.as_str(),
                })
                .map_err(|bundle| compile_error("signal group", bundle))?;
        }
        add_controller(builder, config, junction, &group_ids)?;
    }
    Ok(())
}

fn add_controller(
    builder: &mut SyntheticModuleBuilder,
    config: &CorridorConfig,
    junction: usize,
    group_ids: &[String],
) -> Result<(), Error> {
    let group_refs: Vec<_> = group_ids
        .iter()
        .map(|id| SignalGroupReference::local(id.as_str()))
        .collect();
    let phase_specs = [
        (
            "phase-main-left-green",
            config.signals.main_left_green_ms,
            Some(0),
            SignalAspect::Green,
        ),
        (
            "phase-main-left-yellow",
            config.signals.yellow_ms,
            Some(0),
            SignalAspect::Yellow,
        ),
        (
            "phase-after-main-left-all-red",
            config.signals.all_red_ms,
            None,
            SignalAspect::Red,
        ),
        (
            "phase-main-through-right-green",
            config.signals.main_through_right_green_ms,
            Some(1),
            SignalAspect::Green,
        ),
        (
            "phase-main-through-right-yellow",
            config.signals.yellow_ms,
            Some(1),
            SignalAspect::Yellow,
        ),
        (
            "phase-after-main-through-right-all-red",
            config.signals.all_red_ms,
            None,
            SignalAspect::Red,
        ),
        (
            "phase-secondary-left-green",
            config.signals.secondary_left_green_ms,
            Some(2),
            SignalAspect::Green,
        ),
        (
            "phase-secondary-left-yellow",
            config.signals.yellow_ms,
            Some(2),
            SignalAspect::Yellow,
        ),
        (
            "phase-after-secondary-left-all-red",
            config.signals.all_red_ms,
            None,
            SignalAspect::Red,
        ),
        (
            "phase-secondary-through-right-green",
            config.signals.secondary_through_right_green_ms,
            Some(3),
            SignalAspect::Green,
        ),
        (
            "phase-secondary-through-right-yellow",
            config.signals.yellow_ms,
            Some(3),
            SignalAspect::Yellow,
        ),
        (
            "phase-after-secondary-through-right-all-red",
            config.signals.all_red_ms,
            None,
            SignalAspect::Red,
        ),
    ];
    let mut states_by_phase = Vec::with_capacity(phase_specs.len());
    for &(_, _, active, aspect) in &phase_specs {
        let states: Vec<SignalGroupStateInput<'_>> = group_refs
            .iter()
            .enumerate()
            .map(|(index, group)| SignalGroupStateInput {
                signal_group: *group,
                aspect: if active == Some(index) {
                    aspect
                } else {
                    SignalAspect::Red
                },
            })
            .collect();
        states_by_phase.push(states);
    }
    let phases: Vec<SignalPhaseInput<'_>> = phase_specs
        .iter()
        .zip(&states_by_phase)
        .map(|(&(key, duration_ms, _, _), states)| SignalPhaseInput {
            signal_phase_key: key,
            duration_ms,
            states,
        })
        .collect();
    let controller_id = format!("signal-controller-junction-{junction}");
    builder
        .add_signal_controller(SignalControllerInput {
            signal_controller_key: controller_id.as_str(),
            offset_ms: config.signals.controller_offsets_ms[junction - 1],
            signal_groups: &group_refs,
            phases: &phases,
        })
        .map_err(|bundle| compile_error("signal controller", bundle))?;
    Ok(())
}

fn add_maneuver_gates(
    builder: &mut SyntheticModuleBuilder,
    corridor: &CorridorBuild,
) -> Result<(), Error> {
    for connector in &corridor.connectors {
        builder
            .add_maneuver_gate(ManeuverGateInput {
                maneuver_gate_key: connector.maneuver_gate_id.as_str(),
                maneuver_path: laneflow_compiler::ManeuverPathReference::local(
                    connector.maneuver_path_id.as_str(),
                ),
                transition_index: 0,
                stop_line: laneflow_compiler::StopLineReference::local(
                    connector.stop_line_id.as_str(),
                ),
                signal_control: SignalControlInput::Group(SignalGroupReference::local(
                    connector.signal_group_id.as_str(),
                )),
            })
            .map_err(|bundle| compile_error("maneuver gate", bundle))?;
    }
    Ok(())
}

fn add_static_routes(
    builder: &mut SyntheticModuleBuilder,
    corridor: &CorridorBuild,
) -> Result<(), Error> {
    for route in &corridor.routes {
        let edges: Vec<_> = route
            .route
            .edge_ids
            .iter()
            .map(|id| LaneEdgeReference::local(id.as_str()))
            .collect();
        builder
            .add_static_route(StaticRouteInput {
                static_route_key: route.route.id.as_str(),
                edge_sequence: &edges,
            })
            .map_err(|bundle| compile_error("static route", bundle))?;
    }
    Ok(())
}

fn add_cross_section(
    builder: &mut SyntheticModuleBuilder,
    cross_section: &CrossSectionDocs,
) -> Result<(), Error> {
    for band in &cross_section.facility_bands {
        builder
            .add_facility_band(FacilityBandInput {
                facility_band_key: band.id.as_str(),
                kind_id: band.kind_id,
            })
            .map_err(|bundle| compile_error("facility band", bundle))?;
    }
    for group in &cross_section.lane_groups {
        builder
            .add_lane_group(LaneGroupInput {
                lane_group_key: group.id.as_str(),
                road_section: RoadSectionReference::local(group.road_section_id.as_str()),
            })
            .map_err(|bundle| compile_error("lane group", bundle))?;
    }
    for section in &cross_section.road_sections {
        let edge_chains: Vec<Vec<LaneEdgeReference<'_>>> = section
            .lanes
            .iter()
            .map(|lane| {
                lane.edge_ids
                    .iter()
                    .map(|id| LaneEdgeReference::local(id.as_str()))
                    .collect()
            })
            .collect();
        let lanes: Vec<AuthoringLaneInput<'_>> = section
            .lanes
            .iter()
            .zip(&edge_chains)
            .map(|(lane, chain)| AuthoringLaneInput {
                authoring_lane_key: lane.edge_ids[0].as_str(),
                edge_chain: chain,
                lane_group: lane.lane_group_id.as_deref().map(LaneGroupReference::local),
            })
            .collect();
        builder
            .add_road_section(RoadSectionInput {
                road_section_key: section.id.as_str(),
                kind_id: section.kind_id,
                lanes: &lanes,
            })
            .map_err(|bundle| compile_error("road section", bundle))?;
    }
    for corridor in &cross_section.road_corridors {
        let elements: Vec<CorridorElementReference<'_>> = corridor
            .elements
            .iter()
            .map(|element| match element {
                CorridorElement::Section { section_id } => CorridorElementReference::road_section(
                    RoadSectionReference::local(section_id.as_str()),
                ),
                CorridorElement::Band { band_id } => CorridorElementReference::facility_band(
                    laneflow_compiler::FacilityBandReference::local(band_id.as_str()),
                ),
            })
            .collect();
        builder
            .add_road_corridor(RoadCorridorInput {
                road_corridor_key: corridor.id.as_str(),
                reference_section: RoadSectionReference::local(
                    corridor.reference_section_id.as_str(),
                ),
                elements: &elements,
            })
            .map_err(|bundle| compile_error("road corridor", bundle))?;
    }
    Ok(())
}

fn add_access_rules(
    builder: &mut SyntheticModuleBuilder,
    cross_section: &CrossSectionDocs,
) -> Result<(), Error> {
    for rule in &cross_section.access_rules {
        let classes: Vec<_> = rule
            .participant_class_ids
            .iter()
            .map(|id| ParticipantClassReference::local(id))
            .collect();
        builder
            .add_access_rule(AccessRuleInput {
                access_rule_key: rule.id.as_str(),
                target: AccessRuleTargetInput::LaneGroup(LaneGroupReference::local(
                    rule.target_id.as_str(),
                )),
                effect: match rule.effect {
                    "allow" => AccessEffect::Allow,
                    _ => AccessEffect::Deny,
                },
                participant_classes: &classes,
                regulation: None,
                priority: 0,
            })
            .map_err(|bundle| compile_error("access rule", bundle))?;
    }
    Ok(())
}

fn add_canonical_frame(
    builder: &mut SyntheticModuleBuilder,
    config: &CorridorConfig,
    corridor: &CorridorBuild,
) -> Result<(), Error> {
    let points: Vec<Vec<CanonicalPoint3F32Input>> = corridor
        .edges
        .iter()
        .map(|edge| {
            edge.points
                .iter()
                .map(|point| CanonicalPoint3F32Input {
                    x: point[0],
                    y: point[1],
                    z: point[2],
                })
                .collect()
        })
        .collect();
    let geometries: Vec<LaneEdgeGeometryInput<'_>> = corridor
        .edges
        .iter()
        .zip(&points)
        .map(|(edge, centerline)| LaneEdgeGeometryInput {
            lane_edge: LaneEdgeReference::local(edge.id.as_str()),
            centerline_points: centerline,
        })
        .collect();
    builder
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: config.frame_id.as_str(),
            lane_edge_geometries: &geometries,
        })
        .map_err(|bundle| compile_error("canonical frame", bundle))?;
    Ok(())
}

fn directed_approach_key(approach: Approach) -> &'static str {
    match approach {
        Approach::West => "approach-west",
        Approach::East => "approach-east",
        Approach::North => "approach-north",
        Approach::South => "approach-south",
    }
}

fn config_digest(config: &CorridorConfig) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(config.corridor_config_version.as_bytes());
    hasher.update(config.frame_id.as_bytes());
    hasher.update(config.fixed_delta_ms.to_le_bytes());
    hasher.update(config.geometry.main_length_meters.to_le_bytes());
    hasher.update(config.geometry.secondary_lengths_meters[0].to_le_bytes());
    hasher.update(config.geometry.secondary_lengths_meters[1].to_le_bytes());
    hasher.update(config.geometry.intersection_x_meters[0].to_le_bytes());
    hasher.update(config.geometry.intersection_x_meters[1].to_le_bytes());
    hasher.update(config.geometry.lane_width_meters.to_le_bytes());
    hasher.update(config.geometry.spawn_slot_pitch_meters.to_le_bytes());
    hasher.update(config.speed_limits.main_kilometers_per_hour.to_le_bytes());
    hasher.update(
        config
            .speed_limits
            .secondary_kilometers_per_hour
            .to_le_bytes(),
    );
    hasher.update(config.speed_limits.left_kilometers_per_hour.to_le_bytes());
    hasher.update(config.speed_limits.right_kilometers_per_hour.to_le_bytes());
    hasher.update(config.signals.main_left_green_ms.to_le_bytes());
    hasher.update(config.signals.main_through_right_green_ms.to_le_bytes());
    hasher.update(config.signals.secondary_left_green_ms.to_le_bytes());
    hasher.update(
        config
            .signals
            .secondary_through_right_green_ms
            .to_le_bytes(),
    );
    hasher.update(config.signals.yellow_ms.to_le_bytes());
    hasher.update(config.signals.all_red_ms.to_le_bytes());
    hasher.update(config.signals.controller_offsets_ms[0].to_le_bytes());
    hasher.update(config.signals.controller_offsets_ms[1].to_le_bytes());
    hasher.finalize().into()
}

fn sha256_bytes(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn compile_error(stage: &'static str, bundle: laneflow_compiler::DiagnosticBundle) -> Error {
    Error::Validation {
        stage,
        message: format!("{bundle:?}"),
    }
}
