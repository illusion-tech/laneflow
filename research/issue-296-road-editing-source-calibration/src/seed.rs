#![allow(
    dead_code,
    reason = "closed seed DTO fields are retained for exact generator mapping, not all are needed by the first structural audit"
)]

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

mod generator;

pub use generator::{
    EncodedP100Module, GeneratorError, P100_PROFILE_COMBINATIONS, P100CompileStageDurations,
    P100ProfileCombination, TypedP100Module, build_base_modules, build_base_modules_from_seed,
    build_regularity_probe_modules, build_regularity_probe_modules_from_seed,
    compile_encoded_modules, compile_encoded_modules_with_stage_timing, encode_modules,
};

const SEED_RELATIVE_PATH: &str = "docs/reference/road-editing-source-semantic-seed-v1.json";
const SEED_SHA256: &str = "05a32c19f3fe4ab8f7ea176d996a505688f875197433ab7f83d629ef5d560ce2";
const SEED_WORKLOAD_ID: &str = "LF-COMP-GEOMETRY-P100-v1";
const WORKLOAD_RELATIVE_PATH: &str =
    "docs/reference/road-editing-source-workload-definition-v1.json";
const SERDE_JSON_CHECKSUM: &str =
    "c841b55ecdae098c80dcae9cf767f6f8a0c2cdb3416bbef72181df4d0fe73f14";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SeedAudit {
    pub module_count: u64,
    pub road_alignment_count: u64,
    pub stable_declaration_count: u64,
    pub stable_declarations: BTreeMap<&'static str, u64>,
    pub curve_program_count: u64,
    pub curve_segment_count: u64,
    pub line_segment_count: u64,
    pub cubic_bezier_segment_count: u64,
    pub cubic_control_point_count: u64,
    pub relation_occurrences: BTreeMap<&'static str, u64>,
}

#[derive(Debug)]
pub enum SeedError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Digest {
        expected: &'static str,
        actual: String,
    },
    Json {
        context: String,
        source: serde_json::Error,
    },
    Contract(String),
}

impl fmt::Display for SeedError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(formatter, "读取 {} 失败：{source}", path.display())
            }
            Self::Digest { expected, actual } => {
                write!(
                    formatter,
                    "语义种子 SHA-256 不匹配：期望 {expected}，实际 {actual}"
                )
            }
            Self::Json { context, source } => write!(formatter, "{context} 解析失败：{source}"),
            Self::Contract(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for SeedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json { source, .. } => Some(source),
            Self::Digest { .. } | Self::Contract(_) => None,
        }
    }
}

pub fn load_bound_seed(repository_root: &Path) -> Result<SeedAudit, SeedError> {
    Ok(load_bound_seed_data(repository_root)?.audit)
}

/// 已在计时区外完成 SHA/closed-DTO/结构计数校验的冻结 P100 语义种子。
pub struct LoadedP100Seed {
    data: BoundSeedData,
}

pub fn load_p100_seed(repository_root: &Path) -> Result<LoadedP100Seed, SeedError> {
    Ok(LoadedP100Seed {
        data: load_bound_seed_data(repository_root)?,
    })
}

struct BoundSeedData {
    workload: WorkloadBinding,
    documents: Vec<GeometryDocument>,
    seed_digest: [u8; 32],
    audit: SeedAudit,
}

fn load_bound_seed_data(repository_root: &Path) -> Result<BoundSeedData, SeedError> {
    let workload = validate_workload_binding(repository_root)?;
    let path = repository_root.join(SEED_RELATIVE_PATH);
    let bytes = fs::read(&path).map_err(|source| SeedError::Io {
        path: path.clone(),
        source,
    })?;
    let seed_digest: [u8; 32] = Sha256::digest(&bytes).into();
    let actual_digest = hex_digest(&seed_digest);
    if actual_digest != SEED_SHA256 {
        return Err(SeedError::Digest {
            expected: SEED_SHA256,
            actual: actual_digest,
        });
    }

    let seed: SemanticSeed = serde_json::from_slice(&bytes).map_err(|source| SeedError::Json {
        context: "外层语义种子".into(),
        source,
    })?;
    if seed.schema != "laneflow.geometry-frontend-calibration-fixture"
        || seed.schema_version != 1
        || seed.workload_id != SEED_WORKLOAD_ID
        || seed.modules.len() != 5
    {
        return Err(SeedError::Contract(
            "外层语义种子身份或五模块形状不匹配".into(),
        ));
    }

    let mut documents = Vec::with_capacity(seed.modules.len());
    for (index, module) in seed.modules.iter().enumerate() {
        let expected_path = format!("corridor-{index:02}.geometry.json");
        if module.source_path != expected_path {
            return Err(SeedError::Contract(format!(
                "模块 {index} 的 sourcePath 应为 {expected_path}"
            )));
        }
        let document =
            serde_json::from_str::<GeometryDocument>(&module.source).map_err(|source| {
                SeedError::Json {
                    context: format!("模块 {index} 的嵌入 Geometry 文档"),
                    source,
                }
            })?;
        validate_document_header(index, &document)?;
        documents.push(document);
    }
    let audit = audit_documents(
        &documents,
        workload.generator_contract.import_edges.len() as u64,
    );
    validate_exact_audit(&audit)?;
    validate_workload_exact_counts(&audit, &workload.exact_counts)?;
    Ok(BoundSeedData {
        workload,
        documents,
        seed_digest,
        audit,
    })
}

fn validate_workload_binding(repository_root: &Path) -> Result<WorkloadBinding, SeedError> {
    let path = repository_root.join(WORKLOAD_RELATIVE_PATH);
    let bytes = fs::read(&path).map_err(|source| SeedError::Io {
        path: path.clone(),
        source,
    })?;
    let workload: WorkloadBinding =
        serde_json::from_slice(&bytes).map_err(|source| SeedError::Json {
            context: "RoadEditingSource workload definition".into(),
            source,
        })?;
    let expected_modules = ["p100.m00", "p100.m01", "p100.m02", "p100.m03", "p100.m04"];
    let expected_documents = [
        "p100.m00.lfre",
        "p100.m01.lfre",
        "p100.m02.lfre",
        "p100.m03.lfre",
        "p100.m04.lfre",
    ];
    let expected_imports = [
        ["p100.m01", "p100.m00"],
        ["p100.m02", "p100.m01"],
        ["p100.m03", "p100.m02"],
        ["p100.m04", "p100.m03"],
    ];
    if workload.schema_version != 1
        || workload.workload_id != "LF-ROAD-EDITING-P100-v1"
        || workload.semantic_seed.path != SEED_RELATIVE_PATH
        || workload.semantic_seed.sha256 != SEED_SHA256
        || workload.semantic_seed.source_workload_id != SEED_WORKLOAD_ID
        || workload.semantic_seed.test_parser.crate_name != "serde_json"
        || workload.semantic_seed.test_parser.version != "1.0.151"
        || workload.semantic_seed.test_parser.cargo_lock_checksum != SERDE_JSON_CHECKSUM
        || workload.generator_contract.id != "LF-ROAD-EDITING-P100-GENERATOR-v1"
        || workload.generator_contract.randomness != "none"
        || workload.generator_contract.module_keys != expected_modules
        || workload.generator_contract.source_document_keys != expected_documents
        || workload.generator_contract.import_edges != expected_imports
    {
        return Err(SeedError::Contract(
            "workload definition 的 seed/generator 绑定不匹配".into(),
        ));
    }
    Ok(workload)
}

fn validate_document_header(index: usize, document: &GeometryDocument) -> Result<(), SeedError> {
    let expected_namespace = format!("calibration/geometry/p100/{index:02}");
    let expected_document = format!("corridor-{index:02}.geometry.json");
    if document.geometry_version != "1"
        || document.module.namespace != expected_namespace
        || document.module.document_key != expected_document
        || !document.module.imports.is_empty()
        || document.units.distance != "meter"
        || document.units.angle != "radian"
        || document.units.speed != "meter-per-second"
        || document.units.time != "second"
    {
        return Err(SeedError::Contract(format!(
            "模块 {index} 的嵌入 Geometry header/units 不匹配"
        )));
    }
    Ok(())
}

fn audit_documents(documents: &[GeometryDocument], import_count: u64) -> SeedAudit {
    let mut stable = StableCounts::default();
    let mut curves = CurveCounts::default();
    let mut relations = RelationCounts::default();
    let mut alignments = 0_u64;

    for document in documents {
        stable.canonical_frame += document.frames.len() as u64;
        for road in &document.roads {
            alignments += 1;
            curves.observe(&road.reference_line);
            for span in &road.cross_section_spans {
                stable.road_corridor += 1;
                relations.corridor_elements += span.elements.len() as u64;
                stable.road_section += span.road_sections.len() as u64;
                stable.facility_band += span.facility_bands.len() as u64;
                for section in &span.road_sections {
                    stable.authoring_lane += section.lanes.len() as u64;
                    stable.lane_edge += section.lanes.len() as u64;
                    stable.lane_group += section.lane_groups.len() as u64;
                    relations.road_section_authoring_lanes += section.lanes.len() as u64;
                    relations.lane_edge_successors += section
                        .lanes
                        .iter()
                        .map(|lane| lane.successors.len() as u64)
                        .sum::<u64>();
                }
            }
        }
        stable.junction += document.junctions.len() as u64;
        for junction in &document.junctions {
            relations.junction_approach_edges += junction.approach_edges.len() as u64;
            relations.junction_internal_edges += junction.internal_edges.len() as u64;
            stable.lane_edge += junction.internal_edges.len() as u64;
            for edge in &junction.internal_edges {
                curves.observe(&edge.geometry);
            }
            stable.movement += junction.connections.len() as u64;
            stable.maneuver_path += junction.connections.len() as u64;
            relations.maneuver_path_internal_edges += junction
                .connections
                .iter()
                .map(|connection| connection.internal_edge_sequence.len() as u64)
                .sum::<u64>();
        }

        let overlays = &document.overlays;
        stable.signal_group += overlays.signal_groups.len() as u64;
        stable.signal_controller += overlays.signal_controllers.len() as u64;
        for controller in &overlays.signal_controllers {
            stable.signal_phase += controller.phases.len() as u64;
            relations.signal_controller_groups += controller.signal_groups.len() as u64;
            relations.signal_controller_phases += controller.phases.len() as u64;
            relations.signal_phase_states += controller
                .phases
                .iter()
                .map(|phase| phase.states.len() as u64)
                .sum::<u64>();
        }
        stable.parking_area += overlays.parking_areas.len() as u64;
        stable.parking_space += overlays.parking_spaces.len() as u64;
        stable.participant_class += overlays.participant_classes.len() as u64;
        stable.vehicle_profile += overlays.vehicle_profiles.len() as u64;
        stable.access_rule += overlays.access_rules.len() as u64;
        stable.static_route += overlays.static_routes.len() as u64;
        stable.stop_line += overlays.stop_lines.len() as u64;
        stable.maneuver_gate += overlays.maneuver_gates.len() as u64;
        stable.waiting_zone += overlays.waiting_zones.len() as u64;
        relations.access_rule_participant_classes += overlays
            .access_rules
            .iter()
            .map(|rule| rule.participant_classes.len() as u64)
            .sum::<u64>();
        relations.static_route_edges += overlays
            .static_routes
            .iter()
            .map(|route| route.edge_sequence.len() as u64)
            .sum::<u64>();
    }

    SeedAudit {
        module_count: documents.len() as u64,
        road_alignment_count: alignments,
        stable_declaration_count: stable.total(),
        stable_declarations: stable.as_map(),
        curve_program_count: curves.programs,
        curve_segment_count: curves.segments,
        line_segment_count: curves.lines,
        cubic_bezier_segment_count: curves.cubics,
        cubic_control_point_count: curves.cubics.saturating_mul(2),
        relation_occurrences: relations.as_map(import_count),
    }
}

fn validate_workload_exact_counts(
    audit: &SeedAudit,
    expected: &WorkloadExactCounts,
) -> Result<(), SeedError> {
    let scalar_counts_match = expected.module_count == audit.module_count
        && expected.road_alignment_count == audit.road_alignment_count
        && expected.stable_declaration_count == audit.stable_declaration_count
        && expected.curve_program_count == audit.curve_program_count
        && expected.curve_segment_count == audit.curve_segment_count
        && expected.line_segment_count == audit.line_segment_count
        && expected.cubic_bezier_segment_count == audit.cubic_bezier_segment_count
        && expected.cubic_control_point_count == audit.cubic_control_point_count;
    if !scalar_counts_match
        || !map_matches(&audit.stable_declarations, &expected.stable_declarations)
        || !map_matches(&audit.relation_occurrences, &expected.relation_occurrences)
    {
        return Err(SeedError::Contract(
            "语义种子复算结果与 workload exactCounts 不匹配".into(),
        ));
    }
    Ok(())
}

fn map_matches(actual: &BTreeMap<&'static str, u64>, expected: &BTreeMap<String, u64>) -> bool {
    actual.len() == expected.len()
        && actual
            .iter()
            .all(|(name, count)| expected.get(*name) == Some(count))
}

fn validate_exact_audit(audit: &SeedAudit) -> Result<(), SeedError> {
    let exact = [
        ("moduleCount", audit.module_count, 5),
        ("roadAlignmentCount", audit.road_alignment_count, 35),
        (
            "stableDeclarationCount",
            audit.stable_declaration_count,
            1_715,
        ),
        ("curveProgramCount", audit.curve_program_count, 195),
        ("curveSegmentCount", audit.curve_segment_count, 275),
        ("lineSegmentCount", audit.line_segment_count, 175),
        (
            "cubicBezierSegmentCount",
            audit.cubic_bezier_segment_count,
            100,
        ),
        (
            "cubicControlPointCount",
            audit.cubic_control_point_count,
            200,
        ),
    ];
    for (name, actual, expected) in exact {
        if actual != expected {
            return Err(SeedError::Contract(format!(
                "{name} 应为 {expected}，实际为 {actual}"
            )));
        }
    }

    let expected_stable = BTreeMap::from([
        ("AccessRule", 90),
        ("AuthoringLane", 170),
        ("CanonicalFrame", 5),
        ("FacilityBand", 35),
        ("Junction", 10),
        ("LaneEdge", 330),
        ("LaneGroup", 30),
        ("ManeuverGate", 165),
        ("ManeuverPath", 160),
        ("Movement", 160),
        ("ParkingArea", 5),
        ("ParkingSpace", 5),
        ("ParticipantClass", 15),
        ("RoadCorridor", 35),
        ("RoadSection", 70),
        ("SignalController", 10),
        ("SignalGroup", 40),
        ("SignalPhase", 120),
        ("StaticRoute", 140),
        ("StopLine", 105),
        ("VehicleProfile", 10),
        ("WaitingZone", 5),
    ]);
    if audit.stable_declarations != expected_stable {
        return Err(SeedError::Contract(
            "稳定声明分类计数与 workload 不匹配".into(),
        ));
    }
    let expected_relations = BTreeMap::from([
        ("accessRuleParticipantClasses", 90),
        ("corridorElements", 105),
        ("imports", 4),
        ("junctionApproachEdges", 200),
        ("junctionInternalEdges", 160),
        ("laneEdgeSuccessors", 0),
        ("maneuverPathInternalEdges", 160),
        ("roadSectionAuthoringLanes", 170),
        ("signalControllerGroups", 40),
        ("signalControllerPhases", 120),
        ("signalPhaseStates", 480),
        ("staticRouteEdges", 580),
    ]);
    if audit.relation_occurrences != expected_relations {
        return Err(SeedError::Contract(
            "关系 occurrence 计数与 workload 不匹配".into(),
        ));
    }
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Default)]
struct CurveCounts {
    programs: u64,
    segments: u64,
    lines: u64,
    cubics: u64,
}

impl CurveCounts {
    fn observe(&mut self, curve: &Curve) {
        self.programs += 1;
        self.segments += curve.segments.len() as u64;
        for segment in &curve.segments {
            match segment {
                CurveSegment::Line { .. } => self.lines += 1,
                CurveSegment::CubicBezier { .. } => self.cubics += 1,
            }
        }
    }
}

#[derive(Default)]
struct StableCounts {
    road_corridor: u64,
    road_section: u64,
    authoring_lane: u64,
    lane_edge: u64,
    junction: u64,
    movement: u64,
    maneuver_path: u64,
    maneuver_gate: u64,
    waiting_zone: u64,
    stop_line: u64,
    signal_group: u64,
    signal_controller: u64,
    signal_phase: u64,
    parking_area: u64,
    parking_space: u64,
    lane_group: u64,
    facility_band: u64,
    participant_class: u64,
    access_rule: u64,
    vehicle_profile: u64,
    static_route: u64,
    canonical_frame: u64,
}

impl StableCounts {
    fn total(&self) -> u64 {
        self.as_map().values().sum()
    }

    fn as_map(&self) -> BTreeMap<&'static str, u64> {
        BTreeMap::from([
            ("AccessRule", self.access_rule),
            ("AuthoringLane", self.authoring_lane),
            ("CanonicalFrame", self.canonical_frame),
            ("FacilityBand", self.facility_band),
            ("Junction", self.junction),
            ("LaneEdge", self.lane_edge),
            ("LaneGroup", self.lane_group),
            ("ManeuverGate", self.maneuver_gate),
            ("ManeuverPath", self.maneuver_path),
            ("Movement", self.movement),
            ("ParkingArea", self.parking_area),
            ("ParkingSpace", self.parking_space),
            ("ParticipantClass", self.participant_class),
            ("RoadCorridor", self.road_corridor),
            ("RoadSection", self.road_section),
            ("SignalController", self.signal_controller),
            ("SignalGroup", self.signal_group),
            ("SignalPhase", self.signal_phase),
            ("StaticRoute", self.static_route),
            ("StopLine", self.stop_line),
            ("VehicleProfile", self.vehicle_profile),
            ("WaitingZone", self.waiting_zone),
        ])
    }
}

#[derive(Default)]
struct RelationCounts {
    corridor_elements: u64,
    road_section_authoring_lanes: u64,
    lane_edge_successors: u64,
    junction_approach_edges: u64,
    junction_internal_edges: u64,
    maneuver_path_internal_edges: u64,
    signal_controller_groups: u64,
    signal_controller_phases: u64,
    signal_phase_states: u64,
    access_rule_participant_classes: u64,
    static_route_edges: u64,
}

impl RelationCounts {
    fn as_map(&self, import_count: u64) -> BTreeMap<&'static str, u64> {
        BTreeMap::from([
            (
                "accessRuleParticipantClasses",
                self.access_rule_participant_classes,
            ),
            ("corridorElements", self.corridor_elements),
            ("imports", import_count),
            ("junctionApproachEdges", self.junction_approach_edges),
            ("junctionInternalEdges", self.junction_internal_edges),
            ("laneEdgeSuccessors", self.lane_edge_successors),
            (
                "maneuverPathInternalEdges",
                self.maneuver_path_internal_edges,
            ),
            (
                "roadSectionAuthoringLanes",
                self.road_section_authoring_lanes,
            ),
            ("signalControllerGroups", self.signal_controller_groups),
            ("signalControllerPhases", self.signal_controller_phases),
            ("signalPhaseStates", self.signal_phase_states),
            ("staticRouteEdges", self.static_route_edges),
        ])
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkloadBinding {
    schema_version: u32,
    workload_id: String,
    semantic_seed: WorkloadSeedBinding,
    generator_contract: GeneratorContractBinding,
    exact_counts: WorkloadExactCounts,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkloadSeedBinding {
    path: String,
    sha256: String,
    source_workload_id: String,
    test_parser: TestParserBinding,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TestParserBinding {
    #[serde(rename = "crate")]
    crate_name: String,
    version: String,
    cargo_lock_checksum: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeneratorContractBinding {
    id: String,
    randomness: String,
    module_keys: Vec<String>,
    source_document_keys: Vec<String>,
    import_edges: Vec<[String; 2]>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkloadExactCounts {
    module_count: u64,
    road_alignment_count: u64,
    stable_declaration_count: u64,
    stable_declarations: BTreeMap<String, u64>,
    curve_program_count: u64,
    curve_segment_count: u64,
    line_segment_count: u64,
    cubic_bezier_segment_count: u64,
    cubic_control_point_count: u64,
    relation_occurrences: BTreeMap<String, u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SemanticSeed {
    schema: String,
    schema_version: u32,
    workload_id: String,
    modules: Vec<SeedModule>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SeedModule {
    source_path: String,
    source: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GeometryDocument {
    geometry_version: String,
    module: ModuleRecord,
    units: Units,
    frames: Vec<Frame>,
    roads: Vec<Road>,
    junctions: Vec<Junction>,
    overlays: Overlays,
}

#[derive(Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum Provenance {
    Direct {
        description: String,
    },
    Generated {
        generator_build_id: String,
        parameters_and_inputs_digest: String,
        frontend_options_digest: String,
        random_seed: Option<String>,
        description: String,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ModuleRecord {
    namespace: String,
    document_key: String,
    imports: Vec<String>,
    provenance: Provenance,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Units {
    distance: String,
    angle: String,
    speed: String,
    time: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Frame {
    frame_key: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Curve {
    start: [f64; 3],
    segments: Vec<CurveSegment>,
}

#[derive(Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum CurveSegment {
    Line {
        end: [f64; 3],
    },
    CubicBezier {
        control1: [f64; 3],
        control2: [f64; 3],
        end: [f64; 3],
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Road {
    road_key: String,
    frame: String,
    reference_line: Curve,
    cross_section_spans: Vec<CrossSectionSpan>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum EndStation {
    Finite(f64),
    AlignmentEnd(String),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CrossSectionSpan {
    span_key: String,
    corridor_key: String,
    start_station_meters: f64,
    end_station_meters: EndStation,
    reference_section_key: String,
    reference_lane_key: String,
    elements: Vec<CorridorElement>,
    road_sections: Vec<RoadSection>,
    facility_bands: Vec<FacilityBand>,
}

#[derive(Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum CorridorElement {
    RoadSection { section_key: String },
    FacilityBand { facility_band_key: String },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RoadSection {
    section_key: String,
    kind_id: String,
    lanes: Vec<Lane>,
    lane_groups: Vec<LaneGroup>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Lane {
    lane_key: String,
    lane_edge_key: String,
    direction: LaneDirection,
    width_meters: f64,
    speed_limit_meters_per_second: f64,
    lane_group_key: Option<String>,
    successors: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum LaneDirection {
    Forward,
    Backward,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LaneGroup {
    lane_group_key: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FacilityBand {
    facility_band_key: String,
    kind_id: String,
    width_meters: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Junction {
    junction_key: String,
    approach_edges: Vec<String>,
    internal_edges: Vec<InternalEdge>,
    connections: Vec<Connection>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InternalEdge {
    lane_edge_key: String,
    speed_limit_meters_per_second: f64,
    geometry: Curve,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Connection {
    movement_key: String,
    directed_entry_approach_key: String,
    directed_exit_approach_key: String,
    maneuver_path_key: String,
    entry_edge: String,
    internal_edge_sequence: Vec<String>,
    exit_edge: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Overlays {
    signal_groups: Vec<SignalGroup>,
    signal_controllers: Vec<SignalController>,
    parking_areas: Vec<ParkingArea>,
    parking_spaces: Vec<ParkingSpace>,
    participant_classes: Vec<ParticipantClass>,
    vehicle_profiles: Vec<VehicleProfile>,
    access_rules: Vec<AccessRule>,
    static_routes: Vec<StaticRoute>,
    stop_lines: Vec<StopLine>,
    maneuver_gates: Vec<ManeuverGate>,
    waiting_zones: Vec<WaitingZone>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SignalGroup {
    signal_group_key: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SignalController {
    signal_controller_key: String,
    offset_seconds: f64,
    signal_groups: Vec<String>,
    phases: Vec<SignalPhase>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SignalPhase {
    signal_phase_key: String,
    duration_seconds: f64,
    states: Vec<SignalState>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SignalState {
    signal_group: String,
    aspect: SignalAspect,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum SignalAspect {
    Red,
    Yellow,
    Green,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ParkingArea {
    parking_area_key: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ParkingAnchor {
    lane_edge: String,
    progress_meters: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ParkingGeometry {
    lateral_offset_meters: f64,
    heading_offset_radians: f64,
    length_meters: f64,
    width_meters: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ParkingSpace {
    parking_space_key: String,
    parking_area: Option<String>,
    entry: ParkingAnchor,
    exit: ParkingAnchor,
    geometry: ParkingGeometry,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ParticipantClass {
    participant_class_key: String,
    extends: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Iidm {
    length_meters: f64,
    desired_speed_meters_per_second: f64,
    min_gap_meters: f64,
    time_headway_seconds: f64,
    max_acceleration_meters_per_second_squared: f64,
    comfortable_deceleration_meters_per_second_squared: f64,
    emergency_deceleration_meters_per_second_squared: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VehicleProfile {
    vehicle_profile_key: String,
    participant_class: String,
    iidm: Iidm,
}

#[derive(Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum AccessTarget {
    LaneEdge { lane_edge: String },
    LaneGroup { lane_group: String },
    RoadSection { road_section: String },
    ManeuverPath { maneuver_path: String },
    FacilityBand { facility_band: String },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Regulation {
    jurisdiction: String,
    version: String,
    source: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AccessRule {
    access_rule_key: String,
    target: AccessTarget,
    effect: AccessEffect,
    participant_classes: Vec<String>,
    regulation: Option<Regulation>,
    priority: i32,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum AccessEffect {
    Allow,
    Deny,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StaticRoute {
    static_route_key: String,
    edge_sequence: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StopLine {
    stop_line_key: String,
    lane_edge: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManeuverGate {
    maneuver_gate_key: String,
    maneuver_path: String,
    transition_index: u32,
    stop_line: String,
    signal_control: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WaitingZone {
    waiting_zone_key: String,
    maneuver_path: String,
    entry_gate: String,
    release_gate: String,
    max_occupancy: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("research crate is two levels below repository root")
            .to_path_buf()
    }

    #[test]
    fn bound_seed_parses_with_closed_dtos_and_matches_exact_counts() {
        let audit = load_bound_seed(&repository_root()).unwrap();
        assert_eq!(audit.module_count, 5);
        assert_eq!(audit.stable_declaration_count, 1_715);
        assert_eq!(audit.curve_segment_count, 275);
    }

    #[test]
    fn duplicate_and_unknown_fields_fail_closed() {
        let duplicate = r#"{"frameKey":"frame","frameKey":"other"}"#;
        assert!(serde_json::from_str::<Frame>(duplicate).is_err());
        let unknown = r#"{"frameKey":"frame","legacy":true}"#;
        assert!(serde_json::from_str::<Frame>(unknown).is_err());
    }
}
