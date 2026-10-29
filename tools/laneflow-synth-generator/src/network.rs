use laneflow_compiler::*;
use laneflow_format::{FormatLimits, check_post_emission_bundle};

use crate::encoding::{config_bytes, digest};
use crate::{BUILD_ID, Case, Error, Identity, NAMESPACE, POLICY_KEY, compile_error, key};

pub(crate) fn compile(identity: &Identity) -> Result<Vec<u8>, Error> {
    let limits = CompileLimits::single_network_1m_v2();
    let header = SourceModuleHeader::new(
        SourceModuleHeaderInput {
            authoring_namespace_id: NAMESPACE,
            source_document_key: "lf-synth.document",
            generator_build_id: BUILD_ID,
            parameters_and_inputs_digest: digest(&config_bytes(identity)?),
            frontend_options_digest: digest(b"lf-synth-frontend-1"),
            random_seed: Some(0),
            provenance: "repository:laneflow",
        },
        &limits,
    )
    .map_err(|e| compile_error("source header", e))?;
    let mut builder = SyntheticModuleBuilder::new(header, &limits)
        .map_err(|e| compile_error("synthetic module", e))?;
    builder
        .add_participant_class(ParticipantClassInput {
            participant_class_key: "car",
            extends: None,
        })
        .map_err(|e| compile_error("participant class", e))?;
    builder
        .add_vehicle_profile(VehicleProfileInput {
            vehicle_profile_key: "car",
            participant_class: ParticipantClassReference::local("car"),
            iidm: IidmVehicleProfileInput {
                length_meters: 4.5,
                desired_speed_meters_per_second: 13.9,
                min_gap_meters: 2.0,
                time_headway_seconds: 1.5,
                max_acceleration_meters_per_second_squared: 1.4,
                comfortable_deceleration_meters_per_second_squared: 2.0,
                emergency_deceleration_meters_per_second_squared: 6.0,
            },
        })
        .map_err(|e| compile_error("vehicle profile", e))?;
    let mut gate_keys = Vec::new();
    let mut geometry = Vec::new();
    for cell in 0..identity.cells() {
        let facility = key(cell, 0, "facility");
        builder
            .add_parking_facility(ParkingFacilityInput {
                parking_facility_key: &facility,
                virtual_capacity: 0,
                virtual_entries: &[],
                virtual_exits: &[],
            })
            .map_err(|e| compile_error("parking facility", e))?;
        let groups: Vec<_> = (0..4).map(|route| key(cell, route, "group")).collect();
        for (route, group) in (0..4).zip(&groups) {
            add_route(&mut builder, cell, route, group, &facility)?;
            gate_keys.push(key(cell, route, "gate"));
            let [entry, exit] = centerlines(cell, route);
            geometry.push((key(cell, route, "entry"), entry));
            geometry.push((key(cell, route, "exit"), exit));
        }
        add_controller(&mut builder, identity, cell, &groups)?;
    }
    add_policy(&mut builder, &gate_keys)?;
    let geometry_refs: Vec<_> = geometry
        .iter()
        .map(|(name, points)| LaneEdgeGeometryInput {
            lane_edge: LaneEdgeReference::local(name),
            centerline_points: points,
        })
        .collect();
    builder
        .add_canonical_frame(CanonicalFrameInput {
            canonical_frame_key: "world",
            lane_edge_geometries: &geometry_refs,
        })
        .map_err(|e| compile_error("canonical frame", e))?;
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_synthetic_module(
        builder
            .finish()
            .map_err(|e| compile_error("finish module", e))?,
    )
    .map_err(|e| compile_error("compilation unit", e))?;
    let output = Compiler::new()
        .compile(unit.build().map_err(|e| compile_error("build unit", e))?)
        .map_err(|e| compile_error("compile", e))?;
    let candidate = emit_portable_candidate(
        &output,
        &PortableEmissionProvenance::try_new(BUILD_ID)
            .map_err(|e| compile_error("provenance", e))?,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .map_err(|e| compile_error("emit", e))?;
    check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .map_err(|e| compile_error("post-emission", e))?;
    Ok(candidate.canonical_artifact().bytes().to_vec())
}

fn add_route(
    b: &mut SyntheticModuleBuilder,
    c: u32,
    r: u32,
    group: &str,
    facility: &str,
) -> Result<(), Error> {
    let entry = key(c, r, "entry");
    let exit = key(c, r, "exit");
    let junction = key(c, r, "junction");
    let movement = key(c, r, "movement");
    let path = key(c, r, "path");
    let stop = key(c, r, "stop");
    b.add_lane_edge(LaneEdgeInput {
        lane_edge_key: &entry,
        length_meters: 5_130.0,
        speed_limit_meters_per_second: 13.9,
        successors: &[LaneEdgeReference::local(&exit)],
    })
    .map_err(|e| compile_error("entry edge", e))?;
    b.add_lane_edge(LaneEdgeInput {
        lane_edge_key: &exit,
        length_meters: 5_130.0,
        speed_limit_meters_per_second: 13.9,
        successors: &[],
    })
    .map_err(|e| compile_error("exit edge", e))?;
    b.add_junction(JunctionInput {
        junction_key: &junction,
    })
    .map_err(|e| compile_error("junction", e))?;
    b.add_movement(MovementInput {
        movement_key: &movement,
        junction: JunctionReference::local(&junction),
        directed_entry_approach_key: "entry",
        directed_exit_approach_key: "exit",
        turn_direction: None,
    })
    .map_err(|e| compile_error("movement", e))?;
    b.add_maneuver_path(ManeuverPathInput {
        maneuver_path_key: &path,
        movement: MovementReference::local(&movement),
        entry_edge: LaneEdgeReference::local(&entry),
        internal_edges: &[],
        exit_edge: LaneEdgeReference::local(&exit),
    })
    .map_err(|e| compile_error("maneuver path", e))?;
    b.add_stop_line(StopLineInput {
        stop_line_key: &stop,
        lane_edge: LaneEdgeReference::local(&entry),
    })
    .map_err(|e| compile_error("stop line", e))?;
    b.add_signal_group(SignalGroupInput {
        signal_group_key: group,
    })
    .map_err(|e| compile_error("group", e))?;
    b.add_maneuver_gate(ManeuverGateInput {
        maneuver_gate_key: &key(c, r, "gate"),
        maneuver_path: ManeuverPathReference::local(&path),
        transition_index: 0,
        stop_line: StopLineReference::local(&stop),
        signal_control: SignalControlInput::Group(SignalGroupReference::local(group)),
    })
    .map_err(|e| compile_error("gate", e))?;
    for s in 0..25 {
        let anchor = || ParkingLaneAnchorInput {
            lane_edge: LaneEdgeReference::local(&exit),
            progress_meters: 5.0 + 7.5 * f64::from(s),
        };
        b.add_parking_space(ParkingSpaceInput {
            parking_space_key: &key(c, r, &format!("space{s:02}")),
            parking_facility: Some(ParkingFacilityReference::local(facility)),
            entry: anchor(),
            exit: anchor(),
            geometry: ParkingSpaceGeometryInput {
                lateral_offset_meters: 3.0,
                heading_offset_radians: 0.0,
                length_meters: 5.0,
                width_meters: 2.4,
            },
        })
        .map_err(|e| compile_error("parking space", e))?;
    }
    Ok(())
}

fn add_controller(
    b: &mut SyntheticModuleBuilder,
    identity: &Identity,
    cell: u32,
    groups: &[String],
) -> Result<(), Error> {
    let phase_keys: Vec<_> = (0..6)
        .map(|phase| key(cell, 0, &format!("phase{phase}")))
        .collect();
    let states: Vec<Vec<_>> = (0..6)
        .map(|phase| {
            groups
                .iter()
                .enumerate()
                .map(|(route, group)| SignalGroupStateInput {
                    signal_group: SignalGroupReference::local(group),
                    aspect: match (phase, route % 2) {
                        (0, 0) | (3, 1) => SignalAspect::Green,
                        (1, 0) | (4, 1) => SignalAspect::Yellow,
                        _ => SignalAspect::Red,
                    },
                })
                .collect()
        })
        .collect();
    let phases: Vec<_> = identity
        .durations()
        .iter()
        .enumerate()
        .map(|(p, duration)| SignalPhaseInput {
            signal_phase_key: &phase_keys[p],
            duration_ms: u64::from(*duration),
            states: &states[p],
        })
        .collect();
    let refs: Vec<_> = groups
        .iter()
        .map(|g| SignalGroupReference::local(g))
        .collect();
    b.add_signal_controller(SignalControllerInput {
        signal_controller_key: &key(cell, 0, "controller"),
        offset_ms: u64::from(if identity.case == Case::W4 {
            identity.cycle_ms() - identity.fixed_delta_ms
        } else {
            (cell * 997) % identity.cycle_ms()
        }),
        signal_groups: &refs,
        phases: &phases,
    })
    .map_err(|e| compile_error("controller", e))?;
    Ok(())
}

fn add_policy(b: &mut SyntheticModuleBuilder, gates: &[String]) -> Result<(), Error> {
    let span = b.policy_source_span();
    let source = PolicyInputSource {
        primary: &span,
        contributing: &[],
    };
    let rules: Vec<_> = gates
        .iter()
        .map(|gate| PolicyGateRuleInput {
            rule_key: gate,
            gate: OwnerQualifiedReference {
                target: ManeuverGateReference::local(gate),
                owner_keys: &[],
            },
            participant_classes: None,
            interpretation: GateInterpretation::ProtectedGroup,
            prohibition: GateProhibition::None,
            evidence_keys: &[],
            source,
        })
        .collect();
    b.add_right_of_way_policy_set(RightOfWayPolicySetInput {
        policy_set_key: POLICY_KEY,
        regulation: RegulationIdentity {
            jurisdiction: "engineering",
            version: "lf-synth-v2",
            source: Some("repository:lf-synth-v2"),
        },
        evidence: &[],
        gap_profiles: &[],
        stream_rules: &[],
        gate_rules: &rules,
        source,
    })
    .map_err(|e| compile_error("protected entry policy", e))?;
    Ok(())
}

pub(crate) fn centerlines(cell: u32, route: u32) -> [Vec<CanonicalPoint3F32Input>; 2] {
    let origin_x = -16_300_000 + 1_024_000 * i64::from(cell % 32);
    let origin_z = -16_300_000 + 1_024_000 * i64::from(cell / 32) + 80_000 * i64::from(route);
    let point = |x: i64, z: i64| CanonicalPoint3F32Input {
        x: ((origin_x + x) as f64 / 1_000.0) as f32,
        y: 0.0,
        z: ((origin_z + z) as f64 / 1_000.0) as f32,
    };
    let mut entry = Vec::with_capacity(52);
    let mut exit = Vec::with_capacity(52);
    for run in 0..51 {
        let z = i64::from(run) * 1_200;
        let (start, end) = if run % 2 == 0 {
            (0, 200_000)
        } else {
            (200_000, 0)
        };
        if run < 25 {
            entry.extend([point(start, z), point(end, z)]);
        } else if run == 25 {
            entry.extend([point(start, z), point(100_000, z)]);
            exit.extend([point(100_000, z), point(end, z)]);
        } else {
            exit.extend([point(start, z), point(end, z)]);
        }
    }
    [entry, exit]
}
