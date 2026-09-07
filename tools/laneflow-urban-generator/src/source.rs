//! Version 1 authoring templates. All geometry is emitted through RoadEditing.
mod junction;
mod parking;

use std::collections::BTreeMap;

use laneflow_compiler::road_editing as re;
use laneflow_compiler::{CompileLimits, GeometryAccuracyProfile, GeometryDirectionProfile};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Cell, Direction, Layout, Result, Scale, UrbanConfig};

pub const NAMESPACE: &str = "workload/lf-cn-urban/v1";
pub const COMMON_NAMESPACE: &str = "workload/lf-cn-urban/common/v1";
pub const POLICY_KEY: &str = "urban-policy";
pub const FRAME_KEY: &str = "city-frame";
pub const BUILD_ID: &str = "laneflow-urban-generator-v1";
pub const DOCUMENT_KEY: &str = "cn-urban.document";
pub const COMMON_DOCUMENT_KEY: &str = "cn-urban-common.document";

type Builder<'a> = re::RoadEditingSourceModuleBuilder<'a>;
type Point = [f64; 2];

#[derive(Clone, Debug, Serialize)]
pub struct Edge {
    pub key: String,
    pub tile: u32,
    pub cell: u32,
    pub start: Point,
    pub end: Point,
    pub successors: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Movement {
    pub key: String,
    pub cell: u32,
    pub entry: Direction,
    pub exit: Direction,
    pub turn: String,
    pub waiting: bool,
    pub control: String,
    pub edges: Vec<String>,
    #[serde(skip)]
    pub(crate) conflict_geometry: Vec<Point>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ParkingAnchor {
    pub edge: String,
    pub progress_mm: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct ParkingPool {
    pub key: String,
    pub facility: String,
    pub tile: u32,
    pub virtual_pool: bool,
    pub capacity: u32,
    pub entries: Vec<ParkingAnchor>,
    pub exits: Vec<ParkingAnchor>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalProgram {
    pub key: String,
    pub offset_ms: u64,
    pub cycle_ms: u64,
    pub phases: Vec<SignalPhase>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalPhase {
    pub key: String,
    pub duration_ms: u64,
    pub states: BTreeMap<String, String>,
}

pub struct GeneratedSource {
    pub layout: Layout,
    pub common: re::OwnedRoadEditingSourceBuffer,
    pub topology: re::OwnedRoadEditingSourceBuffer,
    pub declarations: BTreeMap<String, u64>,
    pub references: BTreeMap<String, u64>,
    pub edges: BTreeMap<String, Edge>,
    pub movements: Vec<Movement>,
    pub parking: Vec<ParkingPool>,
    pub signals: Vec<SignalProgram>,
}

pub fn generate_source(config: &UrbanConfig, scale: Scale) -> Result<GeneratedSource> {
    config.validate()?;
    let limits = CompileLimits::single_network_1m_v2();
    let config_text = toml::to_string(config)?;
    let config_digest: [u8; 32] = Sha256::digest(config_text.as_bytes()).into();
    let mut topology_digest = Sha256::new();
    topology_digest.update(config_digest);
    topology_digest.update(scale.name().as_bytes());
    let mut common = builder(
        COMMON_NAMESPACE,
        COMMON_DOCUMENT_KEY,
        Vec::new(),
        &limits,
        config_digest,
    )?;
    common.add_declaration(re::RoadEditingDeclaration::ParticipantClass(
        re::ParticipantClassInput::try_new("road-vehicle")?,
    ))?;
    for profile in &config.profiles {
        common.add_declaration(re::RoadEditingDeclaration::VehicleProfile(
            re::VehicleProfileInput::try_new(
                &profile.key,
                re::ParticipantClassReference::local("road-vehicle")?,
                profile.iidm()?,
            )?,
        ))?;
    }
    let mut counts = BTreeMap::new();
    let mut references = BTreeMap::new();
    let common = finish(common, &limits, &mut counts, &mut references, 0)?;
    let layout = Layout::new(scale);
    let mut source = builder(
        NAMESPACE,
        DOCUMENT_KEY,
        vec![COMMON_NAMESPACE.into()],
        &limits,
        topology_digest.finalize().into(),
    )?;
    source.add_declaration(re::RoadEditingDeclaration::CanonicalFrame(
        re::CanonicalFrameInput::try_new(FRAME_KEY)?,
    ))?;
    let mut edges = BTreeMap::new();
    let mut movements = Vec::new();
    let mut streams = Vec::new();
    let mut gates = Vec::new();
    let mut signals = Vec::new();
    let mut policy_references = 0;
    for cell in &layout.cells {
        for arm in Direction::ALL.into_iter().filter(|&arm| cell.has_arm(arm)) {
            for entering in [true, false] {
                add_road(
                    &mut source,
                    config,
                    &layout,
                    cell,
                    arm,
                    entering,
                    &mut edges,
                )?;
            }
        }
        policy_references += junction::add(
            &mut source,
            config,
            cell,
            &mut edges,
            &mut movements,
            &mut streams,
            &mut gates,
            &mut signals,
        )?;
    }
    policy_references += gates.len() as u64;
    junction::add_policy(&mut source, streams, gates)?;
    let parking = parking::add(&mut source, config, &layout)?;
    let topology = finish(
        source,
        &limits,
        &mut counts,
        &mut references,
        policy_references,
    )?;
    Ok(GeneratedSource {
        layout,
        common,
        topology,
        declarations: counts,
        references,
        edges,
        movements,
        parking,
        signals,
    })
}

fn builder<'a>(
    namespace: &str,
    document: &str,
    imports: Vec<String>,
    limits: &'a CompileLimits,
    input_digest: [u8; 32],
) -> Result<Builder<'a>> {
    Ok(re::RoadEditingSourceModuleBuilder::new(
        re::RoadEditingModuleHeader::try_new(
            namespace,
            document,
            imports,
            re::RoadEditingProvenance::generated(
                BUILD_ID,
                input_digest,
                Sha256::digest(b"road-editing-v1-balanced-5cm-2deg").into(),
                None,
                "repository:tools/laneflow-urban-generator",
            )?,
        )?,
        GeometryAccuracyProfile::Balanced5Cm,
        GeometryDirectionProfile::Balanced2Deg,
        limits,
    )?)
}

fn finish(
    builder: Builder<'_>,
    limits: &CompileLimits,
    counts: &mut BTreeMap<String, u64>,
    references: &mut BTreeMap<String, u64>,
    policy_references: u64,
) -> Result<re::OwnedRoadEditingSourceBuffer> {
    let model = builder.finish()?;
    for declaration in model.declarations() {
        *counts
            .entry(declaration.entity_kind().slug().to_owned())
            .or_default() += 1;
        use re::RoadEditingDeclaration as D;
        let count = match declaration {
            D::RightOfWayPolicySet(_) => policy_references as usize,
            D::RoadCorridor(v) => 2 + v.elements().len(),
            D::RoadSection(v) => 1 + v.authoring_lanes().len(),
            D::AuthoringLane(v) => 2 + usize::from(v.lane_group().is_some()),
            D::LaneEdge(v) => v.successors().len(),
            D::Junction(v) => v.approach_edges().len() + v.internal_edges().len(),
            D::Movement(_) | D::StopLine(_) | D::ConflictZone(_) | D::VehicleProfile(_) => 1,
            D::ManeuverPath(v) => 3 + v.internal_edges().len(),
            D::ManeuverGate(v) => {
                2 + usize::from(matches!(
                    v.signal_control(),
                    re::RoadEditingSignalControl::SignalGroup(_)
                ))
            }
            D::WaitingZone(_) => 3,
            D::SignalController(v) => v.signal_groups().len() + v.signal_phases().len(),
            D::SignalPhase(v) => 1 + v.states().len(),
            D::ParkingFacility(v) => v.virtual_entries().len() + v.virtual_exits().len(),
            D::ParkingSpace(v) => 2 + usize::from(v.parking_facility().is_some()),
            D::ParticipantClass(v) => usize::from(v.extends().is_some()),
            D::ParticipantStream(v) => {
                2 + v
                    .passages()
                    .iter()
                    .map(|p| {
                        1 + usize::from(matches!(p.entry(), re::PathAnchorInput::Gate(_)))
                            + usize::from(matches!(p.exit(), re::PathAnchorInput::Gate(_)))
                    })
                    .sum::<usize>()
            }
            D::CanonicalFrame(_) | D::SignalGroup(_) => 0,
            _ => {
                return Err(crate::validation(
                    "source counters",
                    "add the new template declaration to reference accounting",
                ));
            }
        };
        *references
            .entry(declaration.entity_kind().slug().to_owned())
            .or_default() += count as u64;
    }
    *counts.entry("road-alignment".into()).or_default() += model.road_alignments().len() as u64;
    *counts.entry("conflict-zone-region".into()).or_default() +=
        model.conflict_zone_regions().len() as u64;
    *references.entry("road-alignment".into()).or_default() += model.road_alignments().len() as u64;
    *references.entry("conflict-zone-region".into()).or_default() +=
        model.conflict_zone_regions().len() as u64 * 2;
    Ok(re::RoadEditingSourceWriter::new(limits).write(model)?)
}

fn point(value: Point) -> Result<re::RoadEditingPoint3> {
    Ok(re::RoadEditingPoint3::try_new(value[0], 0.0, value[1])?)
}

fn line(start: Point, end: Point) -> Result<re::RoadEditingCurveProgram> {
    Ok(re::RoadEditingCurveProgram::try_new(
        point(start)?,
        vec![re::RoadEditingCurveSegment::line(point(end)?)],
    )?)
}

fn bezier(start: Point, c1: Point, c2: Point, end: Point) -> Result<re::RoadEditingCurveProgram> {
    Ok(re::RoadEditingCurveProgram::try_new(
        point(start)?,
        vec![re::RoadEditingCurveSegment::cubic_bezier(
            point(c1)?,
            point(c2)?,
            point(end)?,
        )],
    )?)
}

fn port(cell: &Cell, arm: Direction, entering: bool, radius: f64) -> Point {
    let (cx, cz) = cell.center_meters();
    let (dx, dz) = arm.delta();
    let sign = if entering { -1.0 } else { 1.0 };
    // Right-hand traffic, with a central gap for the left-turn waiting pockets.
    [
        cx + f64::from(dx) * radius - f64::from(dz) * sign * 6.0,
        cz + f64::from(dz) * radius + f64::from(dx) * sign * 6.0,
    ]
}

fn edge_ref(key: &str) -> Result<re::LaneEdgeReference> {
    Ok(re::LaneEdgeReference::local(key)?)
}

#[allow(clippy::too_many_arguments)]
fn add_road(
    builder: &mut Builder<'_>,
    config: &UrbanConfig,
    layout: &Layout,
    cell: &Cell,
    arm: Direction,
    entering: bool,
    edges: &mut BTreeMap<String, Edge>,
) -> Result<()> {
    let key = cell.edge_key(arm, entering);
    let inner = port(cell, arm, entering, 30.0);
    let outer = port(cell, arm, entering, 125.0);
    let (start, end) = if entering {
        (outer, inner)
    } else {
        (inner, outer)
    };
    let successors = if entering {
        Vec::new()
    } else {
        layout
            .neighbour(cell, arm)
            .map(|next| vec![next.edge_key(arm.opposite(), true)])
            .unwrap_or_default()
    };
    let corridor_key = format!("{key}.road");
    let section = re::RoadSectionReference::owner_scoped(vec![corridor_key.clone()], "section")?;
    let lane = re::AuthoringLaneReference::owner_scoped(
        vec![corridor_key.clone(), "section".into()],
        "lane",
    )?;
    builder.add_alignment(re::RoadAlignmentInput::try_new(
        &key,
        re::CanonicalFrameReference::local(FRAME_KEY)?,
        line(start, end)?,
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
            vec![lane],
            re::RoadCorridorReference::local(&corridor_key)?,
        )?,
    ))?;
    builder.add_declaration(re::RoadEditingDeclaration::AuthoringLane(
        re::AuthoringLaneInput::try_new(
            "lane",
            edge_ref(&key)?,
            re::RoadEditingLaneDirection::Forward,
            re::LinearWidthProfile::try_new(3.5, 3.5)?,
            None,
            section,
        )?,
    ))?;
    let speed = if matches!(arm, Direction::West | Direction::East) {
        config.arterial_speed_mps
    } else {
        config.secondary_speed_mps
    };
    builder.add_declaration(re::RoadEditingDeclaration::LaneEdge(
        re::LaneEdgeInput::try_new(
            &key,
            speed,
            successors
                .iter()
                .map(|key| edge_ref(key))
                .collect::<Result<_>>()?,
            None,
        )?,
    ))?;
    edges.insert(
        key.clone(),
        Edge {
            key,
            tile: cell.tile,
            cell: cell.index,
            start,
            end,
            successors,
        },
    );
    Ok(())
}

pub(crate) fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
