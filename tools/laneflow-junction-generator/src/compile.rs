//! RoadEditing 模块字节化、编译与 LFCA 发射。

use laneflow_compiler::road_editing as re;
use laneflow_compiler::{
    CompilationOutput, CompilationUnitBuilder, CompileLimits, Compiler, GeometryAccuracyProfile,
    GeometryDirectionProfile, PortableDiffBase, PortableEmissionProvenance,
    emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_post_emission_bundle};
use sha2::{Digest, Sha256};

use crate::Error;
use crate::config::JunctionConfig;
use crate::topology::{
    COMPILER_BUILD_ID, CONTROLLER_KEY, Curve, DOCUMENT_KEY, GENERATOR_BUILD_ID, JUNCTION_KEY,
    PARTICIPANT_CLASS_KEY, PROVENANCE, SIGNAL_GROUPS, Segment, TopologyBuild,
};

pub(crate) fn compile_junction(
    config: &JunctionConfig,
    topology: &TopologyBuild,
) -> Result<CompilationOutput, Error> {
    // 编译/发射用 single_network_1m_v2：p100 的 StageScratchBytes 上限按走廊级
    // 场景标定，本场景 20 条 corridor 链 + 24 段圆弧的声明规模超出其暂存上限
    // 约 1.5%。限制只门禁不改字节；StableId 按内容派生，与限制档位无关，
    // 因此 catalog pin 与 binder 侧的 p100 身份派生结果与这里完全一致。
    let limits = CompileLimits::single_network_1m_v2();
    let config_text = toml::to_string(config)?;
    let header = re::RoadEditingModuleHeader::try_new(
        laneflow_scenario::complex_junction::AUTHORING_NAMESPACE,
        DOCUMENT_KEY,
        Vec::new(),
        re::RoadEditingProvenance::generated(
            GENERATOR_BUILD_ID,
            Sha256::digest(config_text.as_bytes()).into(),
            Sha256::digest(b"road-editing-v1-balanced-5cm-2deg").into(),
            None,
            PROVENANCE,
        )?,
    )?;
    let mut builder = re::RoadEditingSourceModuleBuilder::new(
        header,
        GeometryAccuracyProfile::Balanced5Cm,
        GeometryDirectionProfile::Balanced2Deg,
        &limits,
    )?;

    builder.add_declaration(re::RoadEditingDeclaration::ParticipantClass(
        re::ParticipantClassInput::try_new(PARTICIPANT_CLASS_KEY)?,
    ))?;
    builder.add_declaration(re::RoadEditingDeclaration::VehicleProfile(
        re::VehicleProfileInput::try_new(
            laneflow_scenario::complex_junction::VEHICLE_PROFILE_KEY,
            re::ParticipantClassReference::local(PARTICIPANT_CLASS_KEY)?,
            re::IidmVehicleProfileInput::try_new(
                config.profile.length_meters,
                config.profile.desired_speed_meters_per_second,
                config.profile.min_gap_meters,
                config.profile.time_headway_seconds,
                config.profile.max_acceleration_meters_per_second_squared,
                config
                    .profile
                    .comfortable_deceleration_meters_per_second_squared,
                config
                    .profile
                    .emergency_deceleration_meters_per_second_squared,
            )?,
        )?,
    ))?;
    builder.add_declaration(re::RoadEditingDeclaration::CanonicalFrame(
        re::CanonicalFrameInput::try_new(&config.frame_id)?,
    ))?;

    add_signals(&mut builder, topology)?;
    add_edges(&mut builder, config, topology)?;
    add_junction(&mut builder, topology)?;
    add_conflicts(&mut builder, config, topology)?;
    add_streams(&mut builder, topology)?;
    add_policy(&mut builder, topology)?;

    let model = builder.finish()?;
    let buffer = re::RoadEditingSourceWriter::new(&limits).write(model)?;
    let mut unit = CompilationUnitBuilder::new(limits);
    unit.add_road_editing_module(
        re::RoadEditingModuleInput::try_new(DOCUMENT_KEY, buffer.as_bytes(), None).map_err(
            |error| Error::Validation {
                stage: "module input",
                message: format!("{error:?}"),
            },
        )?,
    )?;
    Compiler::new()
        .compile(unit.build()?)
        .map_err(|bundle| Error::Validation {
            stage: "compile",
            message: diagnostics(&bundle),
        })
}

pub(crate) fn emit_lfca(output: &CompilationOutput) -> Result<Vec<u8>, Error> {
    let provenance = PortableEmissionProvenance::try_new(COMPILER_BUILD_ID).map_err(|error| {
        Error::Validation {
            stage: "portable provenance",
            message: format!("{error:?}"),
        }
    })?;
    let candidate = emit_portable_candidate(
        output,
        &provenance,
        FormatLimits::HARD,
        PortableDiffBase::Genesis,
    )
    .map_err(|error| Error::Validation {
        stage: "emit LFCA",
        message: format!("{error:?}"),
    })?;
    check_post_emission_bundle(
        candidate.canonical_artifact().bytes(),
        candidate.source_map().bytes(),
        candidate.semantic_diff().bytes(),
        candidate.expected_semantic_diff_base(),
        FormatLimits::HARD,
    )
    .map_err(|error| Error::Validation {
        stage: "post-emission",
        message: format!("{error:?}"),
    })?;
    Ok(candidate.canonical_artifact().bytes().to_vec())
}

fn point(value: crate::topology::Point) -> Result<re::RoadEditingPoint3, Error> {
    Ok(re::RoadEditingPoint3::try_new(value[0], 0.0, value[1])?)
}

fn curve(curve: &Curve) -> Result<re::RoadEditingCurveProgram, Error> {
    let segments = curve
        .segments
        .iter()
        .map(|segment| {
            Ok(match *segment {
                Segment::Line { end } => re::RoadEditingCurveSegment::line(point(end)?),
                Segment::Bezier { c1, c2, end } => {
                    re::RoadEditingCurveSegment::cubic_bezier(point(c1)?, point(c2)?, point(end)?)
                }
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    Ok(re::RoadEditingCurveProgram::try_new(
        point(curve.start)?,
        segments,
    )?)
}

fn edge_ref(key: &str) -> Result<re::LaneEdgeReference, Error> {
    Ok(re::LaneEdgeReference::local(key)?)
}

fn path_ref(topology: &TopologyBuild, path: usize) -> Result<re::ManeuverPathReference, Error> {
    let path_build = &topology.paths[path];
    let movement_key = &topology.movements[path_build.movement].key;
    Ok(re::ManeuverPathReference::owner_scoped(
        vec![JUNCTION_KEY.to_owned(), movement_key.clone()],
        &path_build.key,
    )?)
}

fn gate_ref(
    topology: &TopologyBuild,
    path: usize,
    gate: &str,
) -> Result<re::ManeuverGateReference, Error> {
    let path_build = &topology.paths[path];
    let movement_key = &topology.movements[path_build.movement].key;
    Ok(re::ManeuverGateReference::owner_scoped(
        vec![
            JUNCTION_KEY.to_owned(),
            movement_key.clone(),
            path_build.key.clone(),
        ],
        gate,
    )?)
}

fn add_signals(
    builder: &mut re::RoadEditingSourceModuleBuilder<'_>,
    topology: &TopologyBuild,
) -> Result<(), Error> {
    for group in SIGNAL_GROUPS {
        builder.add_declaration(re::RoadEditingDeclaration::SignalGroup(
            re::SignalGroupInput::try_new(group)?,
        ))?;
    }
    for phase in &topology.phases {
        let states = SIGNAL_GROUPS
            .iter()
            .zip(phase.aspects)
            .map(|(group, aspect)| {
                Ok(re::RoadEditingSignalPhaseState::try_new(
                    re::SignalGroupReference::local(*group)?,
                    aspect,
                )?)
            })
            .collect::<Result<Vec<_>, Error>>()?;
        builder.add_declaration(re::RoadEditingDeclaration::SignalPhase(
            re::SignalPhaseInput::try_new(
                &phase.key,
                phase.duration_ms,
                states,
                re::SignalControllerReference::local(CONTROLLER_KEY)?,
            )?,
        ))?;
    }
    builder.add_declaration(re::RoadEditingDeclaration::SignalController(
        re::SignalControllerInput::try_new(
            CONTROLLER_KEY,
            0,
            SIGNAL_GROUPS
                .iter()
                .map(|group| re::SignalGroupReference::local(*group))
                .collect::<Result<Vec<_>, _>>()?,
            topology
                .phases
                .iter()
                .map(|phase| {
                    re::SignalPhaseReference::owner_scoped(
                        vec![CONTROLLER_KEY.to_owned()],
                        &phase.key,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?,
        )?,
    ))?;
    Ok(())
}

fn add_edges(
    builder: &mut re::RoadEditingSourceModuleBuilder<'_>,
    config: &JunctionConfig,
    topology: &TopologyBuild,
) -> Result<(), Error> {
    for edge in &topology.edges {
        let successors = edge
            .successors
            .iter()
            .map(|key| edge_ref(key))
            .collect::<Result<Vec<_>, _>>()?;
        if edge.section_derived {
            // 编译器要求 junction approach 边由 RoadSection 派生：为每条道路边编制
            // alignment → corridor → section → lane 链，几何由 alignment 承载。
            let corridor_key = format!("{}.road", edge.key);
            let section =
                re::RoadSectionReference::owner_scoped(vec![corridor_key.clone()], "section")?;
            let lane = re::AuthoringLaneReference::owner_scoped(
                vec![corridor_key.clone(), "section".to_owned()],
                "lane",
            )?;
            builder.add_alignment(re::RoadAlignmentInput::try_new(
                &edge.key,
                re::CanonicalFrameReference::local(&config.frame_id)?,
                curve(&edge.curve)?,
            )?)?;
            builder.add_declaration(re::RoadEditingDeclaration::RoadCorridor(
                re::RoadCorridorInput::try_new(
                    &corridor_key,
                    re::RoadAlignmentReference::try_new(&edge.key)?,
                    0.0,
                    re::RoadEditingStationEnd::AlignmentEnd,
                    section.clone(),
                    lane.clone(),
                    vec![re::RoadEditingCorridorElement::RoadSection(section.clone())],
                )?,
            ))?;
            builder.add_declaration(re::RoadEditingDeclaration::RoadSection(
                re::RoadSectionInput::try_new(
                    "section",
                    "motorLane",
                    vec![lane],
                    re::RoadCorridorReference::local(&corridor_key)?,
                )?,
            ))?;
            builder.add_declaration(re::RoadEditingDeclaration::AuthoringLane(
                re::AuthoringLaneInput::try_new(
                    "lane",
                    edge_ref(&edge.key)?,
                    re::RoadEditingLaneDirection::Forward,
                    re::LinearWidthProfile::try_new(
                        config.geometry.lane_width_meters,
                        config.geometry.lane_width_meters,
                    )?,
                    None,
                    section,
                )?,
            ))?;
            builder.add_declaration(re::RoadEditingDeclaration::LaneEdge(
                re::LaneEdgeInput::try_new(&edge.key, edge.speed, successors, None)?,
            ))?;
        } else {
            builder.add_declaration(re::RoadEditingDeclaration::LaneEdge(
                re::LaneEdgeInput::try_new(
                    &edge.key,
                    edge.speed,
                    successors,
                    Some(curve(&edge.curve)?),
                )?,
            ))?;
        }
    }
    Ok(())
}

fn add_junction(
    builder: &mut re::RoadEditingSourceModuleBuilder<'_>,
    topology: &TopologyBuild,
) -> Result<(), Error> {
    for movement in &topology.movements {
        builder.add_declaration(re::RoadEditingDeclaration::Movement(
            re::MovementInput::try_new(
                &movement.key,
                re::JunctionReference::local(JUNCTION_KEY)?,
                movement.entry.key(),
                movement.exit.key(),
            )?
            .with_turn_direction(movement.turn),
        ))?;
    }
    for path in &topology.paths {
        let movement_key = &topology.movements[path.movement].key;
        builder.add_declaration(re::RoadEditingDeclaration::ManeuverPath(
            re::ManeuverPathInput::try_new(
                &path.key,
                re::MovementReference::owner_scoped(vec![JUNCTION_KEY.to_owned()], movement_key)?,
                edge_ref(path.edges.first().expect("path entry edge"))?,
                path.edges[1..path.edges.len() - 1]
                    .iter()
                    .map(|key| edge_ref(key))
                    .collect::<Result<Vec<_>, _>>()?,
                edge_ref(path.edges.last().expect("path exit edge"))?,
            )?,
        ))?;
    }
    for stop_line in &topology.stop_lines {
        builder.add_declaration(re::RoadEditingDeclaration::StopLine(
            re::StopLineInput::try_new(&stop_line.key, edge_ref(&stop_line.edge)?)?,
        ))?;
    }
    for gate in &topology.gates {
        builder.add_declaration(re::RoadEditingDeclaration::ManeuverGate(
            re::ManeuverGateInput::try_new(
                gate.key,
                path_ref(topology, gate.path)?,
                gate.transition,
                re::StopLineReference::local(&gate.stop_key)?,
                re::RoadEditingSignalControl::SignalGroup(re::SignalGroupReference::local(
                    gate.group,
                )?),
            )?,
        ))?;
    }
    let waiting_path = topology.waiting_zone_path;
    builder.add_declaration(re::RoadEditingDeclaration::WaitingZone(
        re::WaitingZoneInput::try_new(
            "waiting",
            path_ref(topology, waiting_path)?,
            gate_ref(topology, waiting_path, "waiting-entry")?,
            gate_ref(topology, waiting_path, "release")?,
            1,
        )?,
    ))?;
    builder.add_declaration(re::RoadEditingDeclaration::Junction(
        re::JunctionInput::try_new(
            JUNCTION_KEY,
            topology
                .junction_approaches
                .iter()
                .map(|key| edge_ref(key))
                .collect::<Result<Vec<_>, _>>()?,
            topology
                .junction_internals
                .iter()
                .map(|key| edge_ref(key))
                .collect::<Result<Vec<_>, _>>()?,
        )?,
    ))?;
    Ok(())
}

fn add_conflicts(
    builder: &mut re::RoadEditingSourceModuleBuilder<'_>,
    config: &JunctionConfig,
    topology: &TopologyBuild,
) -> Result<(), Error> {
    for zone in &topology.zones {
        let reference =
            re::ConflictZoneReference::owner_scoped(vec![JUNCTION_KEY.to_owned()], &zone.key)?;
        builder.add_declaration(re::RoadEditingDeclaration::ConflictZone(
            re::ConflictZoneInput::try_new(&zone.key, re::JunctionReference::local(JUNCTION_KEY)?)?,
        ))?;
        builder.add_conflict_zone_region(re::ConflictZoneRegionInput::try_new(
            reference,
            re::CanonicalFrameReference::local(&config.frame_id)?,
            -1.0,
            1.0,
            [[-2.0, -2.0], [2.0, -2.0], [2.0, 2.0], [-2.0, 2.0]]
                .into_iter()
                .map(|p| {
                    Ok(re::RoadEditingPoint2::try_new(
                        zone.center[0] + p[0],
                        zone.center[1] + p[1],
                    )?)
                })
                .collect::<Result<Vec<_>, Error>>()?,
        )?)?;
    }
    Ok(())
}

fn add_streams(
    builder: &mut re::RoadEditingSourceModuleBuilder<'_>,
    topology: &TopologyBuild,
) -> Result<(), Error> {
    for stream in &topology.streams {
        let path_build = &topology.paths[stream.path];
        let waiting = topology.movements[path_build.movement].waiting;
        let passages = stream
            .zones
            .iter()
            .map(|&zone| {
                Ok(re::ConflictPassageInput::new(
                    re::ConflictZoneReference::owner_scoped(
                        vec![JUNCTION_KEY.to_owned()],
                        &topology.zones[zone].key,
                    )?,
                    re::PathAnchorInput::gate(gate_ref(
                        topology,
                        stream.path,
                        if waiting { "release" } else { "admission" },
                    )?),
                    re::PathAnchorInput::edge_boundary((path_build.edges.len() - 1) as u32),
                ))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        builder.add_declaration(re::RoadEditingDeclaration::ParticipantStream(
            re::ParticipantStreamInput::try_new(
                &stream.key,
                re::JunctionReference::local(JUNCTION_KEY)?,
                path_ref(topology, stream.path)?,
                passages,
            )?,
        ))?;
    }
    Ok(())
}

/// 工程示例策略，明确声明版本与依据，不冒充现实法域的法规全集。
fn add_policy(
    builder: &mut re::RoadEditingSourceModuleBuilder<'_>,
    topology: &TopologyBuild,
) -> Result<(), Error> {
    const EVIDENCE: &str = "junction-v1";
    const GAP: &str = "permissive-gap";
    let gates = topology
        .gates
        .iter()
        .map(|gate| {
            let path_build = &topology.paths[gate.path];
            let movement_key = &topology.movements[path_build.movement].key;
            let interpretation = if path_build.permissive {
                laneflow_compiler::GateInterpretation::PermissiveGroup
            } else {
                laneflow_compiler::GateInterpretation::ProtectedGroup
            };
            Ok(re::PolicyGateRuleInput::try_new(
                format!("{}.{}.{}", movement_key, path_build.key, gate.key),
                gate_ref(topology, gate.path, gate.key)?,
                None,
                interpretation,
                laneflow_compiler::GateProhibition::None,
                vec![EVIDENCE.to_owned()],
            )?)
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let streams = topology
        .streams
        .iter()
        .map(|stream| {
            Ok(re::PolicyStreamRuleInput::try_new(
                &stream.key,
                re::ParticipantStreamReference::owner_scoped(
                    vec![JUNCTION_KEY.to_owned()],
                    &stream.key,
                )?,
                None,
                stream.priority,
                stream
                    .yield_to
                    .iter()
                    .map(|key| {
                        Ok(re::ParticipantStreamReference::owner_scoped(
                            vec![JUNCTION_KEY.to_owned()],
                            key,
                        )?)
                    })
                    .collect::<Result<Vec<_>, Error>>()?,
                stream.gap.then(|| GAP.to_owned()),
                vec![EVIDENCE.to_owned()],
            )?)
        })
        .collect::<Result<Vec<_>, Error>>()?;
    builder.add_declaration(re::RoadEditingDeclaration::RightOfWayPolicySet(
        re::RightOfWayPolicySetInput::try_new(
            laneflow_scenario::complex_junction::POLICY_KEY,
            re::RegulationIdentity::try_new("engineering", "complex-junction-1")?,
            vec![re::PolicyEvidenceInput::try_new(
                EVIDENCE,
                PROVENANCE,
                Some("工程验证场景模板，非现实法域认证".to_owned()),
            )?],
            vec![re::PolicyGapProfileInput::try_new(
                GAP,
                "junction-conservative-v1",
                5_000,
                2_000,
                500,
            )?],
            streams,
            gates,
        )?,
    ))?;
    Ok(())
}

fn diagnostics(bundle: &laneflow_compiler::DiagnosticBundle) -> String {
    bundle
        .diagnostics()
        .iter()
        .map(|diagnostic| {
            format!(
                "{} {:?}: {:?}",
                diagnostic.code().as_str(),
                diagnostic.stable_key(),
                diagnostic.payload()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

impl From<laneflow_compiler::DiagnosticBundle> for Error {
    fn from(value: laneflow_compiler::DiagnosticBundle) -> Self {
        Self::Validation {
            stage: "road-editing source",
            message: diagnostics(&value),
        }
    }
}
