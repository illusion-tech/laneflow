//! LFCA emit: compile the converted network model through the checked compiler path.
//!
//! The Traffic/Spatial intermediate model is declared as RoadEditing source, compiled
//! fail-closed by `laneflow-compiler`, then emitted as `network.lfca` (Road Editing v4).
//! Compilation IS the artifact validation; there is no separate JSON schema or runtime
//! admission step.

use std::collections::{HashMap, HashSet};

use laneflow_compiler::road_editing as re;
use laneflow_compiler::{
    CompilationOutput, CompilationUnitBuilder, CompileLimits, Compiler, GeometryAccuracyProfile,
    GeometryDirectionProfile, PortableDiffBase, PortableEmissionProvenance, SignalAspect,
    emit_portable_candidate,
};
use laneflow_format::{FormatLimits, check_post_emission_bundle};
use sha2::{Digest, Sha256};

use crate::{
    Error, Result,
    output::model::{SpatialPackage, TrafficPackage},
    source::LUST_COMMIT,
    sumo::{LUST_FRAME_ID, SUMO_ID_PREFIX},
};

/// Authoring namespace for the LuST network module.
pub const LUST_NAMESPACE: &str = "laneflow/lust-workload";
const DOCUMENT_KEY: &str = "lust-network";
const GENERATOR_BUILD_ID: &str = "laneflow-lust-converter";
const COMPILER_BUILD_ID: &str = "laneflow-lust-converter-v1";
const PROVENANCE: &str = "repository:tools/laneflow-lust-converter";
/// 冻结契约 §3.4：固定声明 `motorVehicle -> car` 两级 ParticipantClass。
const PARTICIPANT_CLASS_PARENT_KEY: &str = "motorVehicle";
const PARTICIPANT_CLASS_KEY: &str = "car";
/// SUMO default lane width; LuST net.xml does not override @width.
const SUMO_LANE_WIDTH_METERS: f64 = 3.2;

/// Entity counts of the compiled network, used by the conversion report and manifest.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TopologyCounts {
    pub lane_edges: u64,
    pub junctions: u64,
    pub movements: u64,
    pub maneuver_paths: u64,
    pub vehicle_profiles: u64,
    pub signal_controllers: u64,
    pub signal_groups: u64,
    pub stop_lines: u64,
    pub maneuver_gates: u64,
    pub parking_registry_empty: bool,
}

/// Compiled `network.lfca` bytes plus the entity counts of the source model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopologyArtifacts {
    pub network_lfca: Vec<u8>,
    pub counts: TopologyCounts,
}

/// Compile the intermediate Traffic/Spatial model into checked `network.lfca` bytes.
pub fn compile_network_lfca(
    traffic: &TrafficPackage,
    spatial: &SpatialPackage,
) -> Result<TopologyArtifacts> {
    let limits = CompileLimits::single_network_1m_v2();
    let config_text = format!(
        "lust-converter-v1\nsource_commit={LUST_COMMIT}\nframe={LUST_FRAME_ID}\ngeometry=balanced-5cm-2deg\n"
    );
    let header = re::RoadEditingModuleHeader::try_new(
        LUST_NAMESPACE,
        DOCUMENT_KEY,
        Vec::new(),
        re::RoadEditingProvenance::generated(
            GENERATOR_BUILD_ID,
            Sha256::digest(config_text.as_bytes()).into(),
            Sha256::digest(b"lust-road-editing-v4-balanced-5cm-2deg").into(),
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

    add_profiles(&mut builder, traffic)?;
    builder.add_declaration(re::RoadEditingDeclaration::CanonicalFrame(
        re::CanonicalFrameInput::try_new(LUST_FRAME_ID)?,
    ))?;

    let curve_by_edge = spatial
        .edges
        .iter()
        .map(|edge| (edge.traffic_edge_id.as_str(), edge))
        .collect::<HashMap<_, _>>();
    // 编译器要求路口路径的两侧边界边（entry 与 exit）都声明为该路口的 approach，
    // 且 approach 边必须由 alignment → corridor → section → lane 链派生。
    let approach_edges: HashSet<&str> = traffic
        .maneuver_paths
        .iter()
        .flat_map(|path| [path.entry_edge_id.as_str(), path.exit_edge_id.as_str()])
        .collect();
    add_edges(&mut builder, traffic, &curve_by_edge, &approach_edges)?;
    add_junctions(&mut builder, traffic)?;
    add_signals(&mut builder, traffic)?;

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
    let output = Compiler::new()
        .compile(unit.build()?)
        .map_err(|bundle| Error::Validation {
            stage: "compile",
            message: diagnostics(&bundle),
        })?;
    let network_lfca = emit_lfca(&output)?;

    Ok(TopologyArtifacts {
        network_lfca,
        counts: TopologyCounts {
            lane_edges: traffic.lane_graph.edges.len() as u64,
            junctions: traffic.junctions.len() as u64,
            movements: traffic.movements.len() as u64,
            maneuver_paths: traffic.maneuver_paths.len() as u64,
            vehicle_profiles: traffic.vehicle_profiles.len() as u64,
            signal_controllers: traffic.signals.controllers.len() as u64,
            signal_groups: traffic.signals.groups.len() as u64,
            stop_lines: traffic.signals.stop_lines.len() as u64,
            maneuver_gates: traffic.signals.maneuver_gates.len() as u64,
            parking_registry_empty: true,
        },
    })
}

/// Strip the `sumo:` prefix; internal lanes (`:J_0_0`) become `int:J_0_0` so keys
/// start with an alphanumeric byte as the token grammar requires.
fn edge_key(laneflow_id: &str) -> String {
    let raw = laneflow_id
        .strip_prefix(SUMO_ID_PREFIX)
        .unwrap_or(laneflow_id);
    if let Some(rest) = raw.strip_prefix(':') {
        format!("int:{rest}")
    } else {
        raw.to_owned()
    }
}

fn bare(laneflow_id: &str) -> &str {
    laneflow_id
        .strip_prefix(SUMO_ID_PREFIX)
        .unwrap_or(laneflow_id)
}

fn edge_ref(key: &str) -> Result<re::LaneEdgeReference> {
    Ok(re::LaneEdgeReference::local(key)?)
}

fn curve(points: &[[f64; 3]]) -> Result<re::RoadEditingCurveProgram> {
    let (start, rest) = points.split_first().ok_or_else(|| {
        Error::SumoModel("spatial edge centerline must hold at least one point".to_owned())
    })?;
    let segments = rest
        .iter()
        .map(|end| Ok(re::RoadEditingCurveSegment::line(point3(*end)?)))
        .collect::<Result<Vec<_>>>()?;
    Ok(re::RoadEditingCurveProgram::try_new(
        point3(*start)?,
        segments,
    )?)
}

fn point3(value: [f64; 3]) -> Result<re::RoadEditingPoint3> {
    Ok(re::RoadEditingPoint3::try_new(
        value[0], value[1], value[2],
    )?)
}

fn add_profiles(
    builder: &mut re::RoadEditingSourceModuleBuilder<'_>,
    traffic: &TrafficPackage,
) -> Result<()> {
    builder.add_declaration(re::RoadEditingDeclaration::ParticipantClass(
        re::ParticipantClassInput::try_new(PARTICIPANT_CLASS_PARENT_KEY)?,
    ))?;
    builder.add_declaration(re::RoadEditingDeclaration::ParticipantClass(
        re::ParticipantClassInput::try_new(PARTICIPANT_CLASS_KEY)?.with_extends(
            re::ParticipantClassReference::local(PARTICIPANT_CLASS_PARENT_KEY)?,
        ),
    ))?;
    for profile in &traffic.vehicle_profiles {
        builder.add_declaration(re::RoadEditingDeclaration::VehicleProfile(
            re::VehicleProfileInput::try_new(
                bare(&profile.id),
                re::ParticipantClassReference::local(PARTICIPANT_CLASS_KEY)?,
                re::IidmVehicleProfileInput::try_new(
                    profile.length,
                    profile.desired_speed,
                    profile.min_gap,
                    profile.time_headway,
                    profile.max_acceleration,
                    profile.comfortable_deceleration,
                    profile.emergency_deceleration,
                )?,
            )?,
        ))?;
    }
    Ok(())
}

/// Junction approach edges must be RoadSection-derived per compiler rules; every other
/// edge carries explicit centerline geometry directly.
fn add_edges(
    builder: &mut re::RoadEditingSourceModuleBuilder<'_>,
    traffic: &TrafficPackage,
    curve_by_edge: &HashMap<&str, &crate::output::model::SpatialEdge>,
    approach_edges: &HashSet<&str>,
) -> Result<()> {
    let frame = re::CanonicalFrameReference::local(LUST_FRAME_ID)?;
    for edge in &traffic.lane_graph.edges {
        let key = edge_key(&edge.id);
        // 编译器规定路口穿越由 ManeuverPath 独占权威：路径链不写 LaneEdge 后继。
        let successors = Vec::new();
        let spatial = curve_by_edge.get(edge.id.as_str()).ok_or_else(|| {
            Error::SumoModel(format!("lane edge {:?} has no spatial centerline", edge.id))
        })?;
        if approach_edges.contains(edge.id.as_str()) {
            let corridor_key = format!("{key}.road");
            let section =
                re::RoadSectionReference::owner_scoped(vec![corridor_key.clone()], "section")?;
            let lane = re::AuthoringLaneReference::owner_scoped(
                vec![corridor_key.clone(), "section".to_owned()],
                "lane",
            )?;
            builder.add_alignment(re::RoadAlignmentInput::try_new(
                &key,
                frame.clone(),
                curve(&spatial.centerline.points)?,
            )?)?;
            builder.add_declaration(re::RoadEditingDeclaration::RoadCorridor(
                re::RoadCorridorInput::try_new(
                    &corridor_key,
                    re::RoadAlignmentReference::try_new(&key)?,
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
                    vec![lane.clone()],
                    re::RoadCorridorReference::local(&corridor_key)?,
                )?,
            ))?;
            builder.add_declaration(re::RoadEditingDeclaration::AuthoringLane(
                re::AuthoringLaneInput::try_new(
                    "lane",
                    edge_ref(&key)?,
                    re::RoadEditingLaneDirection::Forward,
                    re::LinearWidthProfile::try_new(SUMO_LANE_WIDTH_METERS, SUMO_LANE_WIDTH_METERS)?,
                    None,
                    section,
                )?,
            ))?;
            builder.add_declaration(re::RoadEditingDeclaration::LaneEdge(
                re::LaneEdgeInput::try_new(&key, edge.speed_limit, successors, None)?,
            ))?;
        } else {
            builder.add_declaration(re::RoadEditingDeclaration::LaneEdge(
                re::LaneEdgeInput::try_new(
                    &key,
                    edge.speed_limit,
                    successors,
                    Some(curve(&spatial.centerline.points)?),
                )?,
            ))?;
        }
    }
    Ok(())
}

/// Local movement key within its junction owner: `sumo:J:A-to-B` -> `A-to-B`.
fn movement_key(junction_key: &str, movement_id: &str) -> Result<String> {
    let with_prefix = format!("{junction_key}:");
    bare(movement_id)
        .strip_prefix(with_prefix.as_str())
        .map(str::to_owned)
        .ok_or_else(|| {
            Error::SumoModel(format!(
                "movement id {movement_id:?} does not start with its junction prefix {with_prefix:?}"
            ))
        })
}

/// Owner chain `(junction_key, movement_key, path_key)` for every ManeuverPath,
/// plus per-movement deterministic path ordinals.
struct PathKeys {
    junction_key: String,
    movement_key: String,
    path_key: String,
}

fn path_keys(traffic: &TrafficPackage) -> Result<HashMap<String, PathKeys>> {
    let junction_by_movement: HashMap<&str, &str> = traffic
        .movements
        .iter()
        .map(|movement| (movement.id.as_str(), bare(&movement.junction_id)))
        .collect();
    let mut ordinals: HashMap<&str, u64> = HashMap::new();
    let mut keys = HashMap::with_capacity(traffic.maneuver_paths.len());
    for path in &traffic.maneuver_paths {
        let junction_key = junction_by_movement
            .get(path.movement_id.as_str())
            .ok_or_else(|| {
                Error::SumoModel(format!(
                    "ManeuverPath {:?} references unknown movement {:?}",
                    path.id, path.movement_id
                ))
            })?
            .to_string();
        let movement_key = movement_key(&junction_key, &path.movement_id)?;
        let ordinal = ordinals.entry(path.movement_id.as_str()).or_insert(0);
        let path_key = format!("path-{ordinal}");
        *ordinal += 1;
        keys.insert(
            path.id.clone(),
            PathKeys {
                junction_key,
                movement_key,
                path_key,
            },
        );
    }
    Ok(keys)
}

fn path_ref(keys: &PathKeys) -> Result<re::ManeuverPathReference> {
    Ok(re::ManeuverPathReference::owner_scoped(
        vec![keys.junction_key.clone(), keys.movement_key.clone()],
        &keys.path_key,
    )?)
}

fn add_junctions(
    builder: &mut re::RoadEditingSourceModuleBuilder<'_>,
    traffic: &TrafficPackage,
) -> Result<()> {
    let path_keys = path_keys(traffic)?;
    for junction in &traffic.junctions {
        let junction_key = bare(&junction.id);
        let mut approaches: Vec<String> = Vec::new();
        let mut internals: Vec<String> = Vec::new();
        for path in traffic
            .maneuver_paths
            .iter()
            .filter(|path| path_keys[&path.id].junction_key == junction_key)
        {
            approaches.push(edge_key(&path.entry_edge_id));
            approaches.push(edge_key(&path.exit_edge_id));
            internals.extend(path.internal_edge_ids.iter().map(|id| edge_key(id)));
        }
        approaches.sort();
        approaches.dedup();
        internals.sort();
        internals.dedup();
        builder.add_declaration(re::RoadEditingDeclaration::Junction(
            re::JunctionInput::try_new(
                junction_key,
                approaches
                    .iter()
                    .map(|key| edge_ref(key))
                    .collect::<Result<Vec<_>>>()?,
                internals
                    .iter()
                    .map(|key| edge_ref(key))
                    .collect::<Result<Vec<_>>>()?,
            )?,
        ))?;
    }
    for movement in &traffic.movements {
        let junction_key = bare(&movement.junction_id);
        builder.add_declaration(re::RoadEditingDeclaration::Movement(
            re::MovementInput::try_new(
                movement_key(junction_key, &movement.id)?,
                re::JunctionReference::local(junction_key)?,
                movement.from_road_edge_id.clone(),
                movement.to_road_edge_id.clone(),
            )?,
        ))?;
    }
    for path in &traffic.maneuver_paths {
        let keys = &path_keys[&path.id];
        builder.add_declaration(re::RoadEditingDeclaration::ManeuverPath(
            re::ManeuverPathInput::try_new(
                &keys.path_key,
                re::MovementReference::owner_scoped(
                    vec![keys.junction_key.clone()],
                    &keys.movement_key,
                )?,
                edge_ref(&edge_key(&path.entry_edge_id))?,
                path.internal_edge_ids
                    .iter()
                    .map(|id| edge_ref(&edge_key(id)))
                    .collect::<Result<Vec<_>>>()?,
                edge_ref(&edge_key(&path.exit_edge_id))?,
            )?,
        ))?;
    }
    Ok(())
}

fn aspect(value: &str) -> Result<SignalAspect> {
    match value {
        "green" => Ok(SignalAspect::Green),
        "yellow" => Ok(SignalAspect::Yellow),
        "red" => Ok(SignalAspect::Red),
        other => Err(Error::SumoModel(format!(
            "unsupported signal aspect {other:?}"
        ))),
    }
}

fn add_signals(
    builder: &mut re::RoadEditingSourceModuleBuilder<'_>,
    traffic: &TrafficPackage,
) -> Result<()> {
    let signals = &traffic.signals;
    if signals.controllers.is_empty() {
        if !signals.groups.is_empty()
            || !signals.stop_lines.is_empty()
            || !signals.maneuver_gates.is_empty()
        {
            return Err(Error::SumoModel(
                "signals declare groups/stop lines/gates without any controller".to_owned(),
            ));
        }
        return Ok(());
    }
    let path_keys = path_keys(traffic)?;

    for stop_line in &signals.stop_lines {
        builder.add_declaration(re::RoadEditingDeclaration::StopLine(
            re::StopLineInput::try_new(bare(&stop_line.id), edge_ref(&edge_key(&stop_line.edge_id))?)?,
        ))?;
    }
    for group in &signals.groups {
        builder.add_declaration(re::RoadEditingDeclaration::SignalGroup(
            re::SignalGroupInput::try_new(bare(&group.id))?,
        ))?;
    }
    for controller in &signals.controllers {
        let controller_key = bare(&controller.id);
        for phase in &controller.phases {
            let phase_key = bare(&phase.id)
                .strip_prefix(with_prefix(controller_key).as_str())
                .ok_or_else(|| {
                    Error::SumoModel(format!(
                        "signal phase id {:?} does not start with its controller prefix",
                        phase.id
                    ))
                })?;
            let states = phase
                .states
                .iter()
                .map(|state| {
                    Ok(re::RoadEditingSignalPhaseState::try_new(
                        re::SignalGroupReference::local(bare(&state.group_id))?,
                        aspect(state.aspect)?,
                    )?)
                })
                .collect::<Result<Vec<_>>>()?;
            builder.add_declaration(re::RoadEditingDeclaration::SignalPhase(
                re::SignalPhaseInput::try_new(
                    phase_key,
                    phase.duration_ms,
                    states,
                    re::SignalControllerReference::local(controller_key)?,
                )?,
            ))?;
        }
        builder.add_declaration(re::RoadEditingDeclaration::SignalController(
            re::SignalControllerInput::try_new(
                controller_key,
                controller.offset_ms,
                controller
                    .group_ids
                    .iter()
                    .map(|id| Ok(re::SignalGroupReference::local(bare(id))?))
                    .collect::<Result<Vec<_>>>()?,
                controller
                    .phases
                    .iter()
                    .map(|phase| {
                        let phase_key = bare(&phase.id)
                            .strip_prefix(with_prefix(controller_key).as_str())
                            .expect("phase prefix checked above");
                        Ok(re::SignalPhaseReference::owner_scoped(
                            vec![controller_key.to_owned()],
                            phase_key,
                        )?)
                    })
                    .collect::<Result<Vec<_>>>()?,
            )?,
        ))?;
    }

    let mut gated_paths: HashSet<&str> = HashSet::new();
    for gate in &signals.maneuver_gates {
        let keys = path_keys.get(&gate.maneuver_path_id).ok_or_else(|| {
            Error::SumoModel(format!(
                "ManeuverGate {:?} references unknown ManeuverPath {:?}",
                gate.id, gate.maneuver_path_id
            ))
        })?;
        if !gated_paths.insert(gate.maneuver_path_id.as_str()) {
            return Err(Error::SumoModel(format!(
                "ManeuverPath {:?} is gated more than once",
                gate.maneuver_path_id
            )));
        }
        if gate.signal_control.kind != "group" {
            return Err(Error::SumoModel(format!(
                "ManeuverGate {:?} signal control kind {:?} is not supported",
                gate.id, gate.signal_control.kind
            )));
        }
        builder.add_declaration(re::RoadEditingDeclaration::ManeuverGate(
            re::ManeuverGateInput::try_new(
                "gate",
                path_ref(keys)?,
                gate.transition_index,
                re::StopLineReference::local(bare(&gate.stop_line_id))?,
                re::RoadEditingSignalControl::SignalGroup(re::SignalGroupReference::local(
                    bare(&gate.signal_control.group_id),
                )?),
            )?,
        ))?;
    }
    Ok(())
}

fn with_prefix(owner: &str) -> String {
    format!("{owner}:")
}

fn emit_lfca(output: &CompilationOutput) -> Result<Vec<u8>> {
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
