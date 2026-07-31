//! `LF-COMP-CORRIDOR-v1` 的绑定夹具模板、八阶段管线与摘要已知向量。
//!
//! 原始 JSON 只在模板准备期读取。模板保留有类型局部序号、引用、标量和规范几何，
//! 不保留夹具 ID 或路径；规模运行只复制模板并执行确定性阶段降阶。

use crate::identity::{
    ABSENT_LOCAL_INDEX, IDENTITY_MAGIC, IdentityContract, IdentityField, IdentityFieldValue,
    STABLE_ID_DOMAIN, SemanticRecord, encode_canonical_identity, encode_semantic_record_stream,
};
use crate::stage::{
    HirStageRecord, IdentityAggregateCounts, MirLirStageRecord, SourceSpanRecord, StageBreakdown,
    StageContract, StageGenerationError, StageShape, TypedAstStageRecord,
};
use crate::{
    GeneratorContract, GraphProfileId, SequenceKind, TrustedContract, expand_module_graph,
    permute_in_place,
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path};

pub const CORRIDOR_WORKLOAD_ID: &str = "LF-COMP-CORRIDOR-v1";
pub const CORRIDOR_KNOWN_VECTOR_SCHEMA: &str =
    "laneflow.compiler-calibration-corridor-summary-known-vectors";
#[cfg(test)]
const CORRIDOR_KNOWN_VECTOR_BYTE_LENGTH: usize = 9_101;
#[cfg(test)]
const CORRIDOR_KNOWN_VECTOR_SHA256: &str =
    "b32b42230f5f8c3894336b8666da4bb77f425f1daf0875b227928c2dc315d0d3";

const ENTITY_KIND_ABSENT: u16 = 0;
const SHARED_CONSTANT_ENTITY_KIND: u16 = 0x00ff;
const SHORT_UNIQUE_PROFILE_ID: &str = "short-unique-v1";
const SIGNALIZED_TRAFFIC_ROLE: &str = "traffic-signalized-corridor";
const SIGNALIZED_SPATIAL_ROLE: &str = "spatial-signalized-corridor";
const PARKING_TRAFFIC_ROLE: &str = "traffic-parking-signals-baseline";

const EXPECTED_STAGE_INPUTS: [(&str, u64); 6] = [
    ("sourceDeclarationCount", 357),
    ("identityFieldOccurrenceCount", 1_018),
    ("profiledKeyOccurrenceCount", 409),
    ("sourceReferenceCount", 2_369),
    ("sourceRelationCount", 627),
    ("sourceGeometryCount", 1_398),
];

const EXPECTED_PER_UNIT_COUNTS: [(&str, u64); 33] = [
    ("LaneEdge", 69),
    ("edgeConnection", 66),
    ("Junction", 3),
    ("Movement", 26),
    ("ManeuverPath", 34),
    ("StaticRoute", 30),
    ("routeOccurrence", 120),
    ("VehicleProfile", 3),
    ("ParticipantClass", 4),
    ("RoadCorridor", 8),
    ("RoadSection", 15),
    ("AuthoringLane", 35),
    ("laneCoverageOccurrence", 36),
    ("LaneGroup", 6),
    ("FacilityBand", 7),
    ("AccessRule", 18),
    ("accessRelation", 18),
    ("StopLine", 21),
    ("ManeuverGate", 34),
    ("SignalGroup", 9),
    ("SignalController", 3),
    ("SignalPhase", 27),
    ("signalPhaseStateOccurrence", 99),
    ("ParkingArea", 1),
    ("ParkingSpace", 3),
    ("CanonicalFrame", 1),
    ("canonicalGeometryPoint", 1_398),
    ("ownerRelation", 186),
    ("signalGroupRelation", 33),
    ("gateOccurrence", 34),
    ("parkingSpaceAnchors", 3),
    ("junctionInternalEdgeRole", 32),
    ("semanticOutputRecord", 2_382),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CorridorContract {
    template_files: Vec<BoundTemplateFile>,
    expected_stage_inputs: BTreeMap<String, u64>,
    expected_per_unit_counts: BTreeMap<String, u64>,
}

impl CorridorContract {
    pub fn from_manifest(manifest: &Value) -> Result<Self, CorridorError> {
        let workloads = required_array(manifest, "workloads")?;
        let workload = workloads
            .iter()
            .find(|candidate| {
                candidate.get("id").and_then(Value::as_str) == Some(CORRIDOR_WORKLOAD_ID)
            })
            .ok_or_else(|| CorridorError::Missing("workloads[LF-COMP-CORRIDOR-v1]".to_owned()))?;
        require_bool(workload, "scalable", true)?;
        require_string_array(
            workload,
            "graphProfiles",
            &["wide-star-v1", "deep-chain-v1", "shared-fanin-dag-v1"],
        )?;
        require_string_array(workload, "stringProfiles", &[SHORT_UNIQUE_PROFILE_ID])?;

        let stage_inputs = required_object(workload, "perUnitStageInputs")?;
        for (field, expected) in EXPECTED_STAGE_INPUTS {
            require_u64(stage_inputs, field, expected)?;
        }
        let per_unit_counts = required_object(workload, "perUnitCounts")?;
        for (field, expected) in EXPECTED_PER_UNIT_COUNTS {
            require_u64(per_unit_counts, field, expected)?;
        }

        let template_files = required_array(workload, "templateFiles")?
            .iter()
            .map(BoundTemplateFile::parse)
            .collect::<Result<Vec<_>, _>>()?;
        let roles = template_files
            .iter()
            .map(|file| file.role.as_str())
            .collect::<Vec<_>>();
        if roles
            != [
                SIGNALIZED_TRAFFIC_ROLE,
                SIGNALIZED_SPATIAL_ROLE,
                PARKING_TRAFFIC_ROLE,
            ]
        {
            return Err(CorridorError::Mismatch {
                path: "workloads[LF-COMP-CORRIDOR-v1].templateFiles.roles".to_owned(),
                expected: format!(
                    "{SIGNALIZED_TRAFFIC_ROLE}, {SIGNALIZED_SPATIAL_ROLE}, {PARKING_TRAFFIC_ROLE}"
                ),
                actual: roles.join(", "),
            });
        }

        Ok(Self {
            template_files,
            expected_stage_inputs: EXPECTED_STAGE_INPUTS
                .into_iter()
                .map(|(field, value)| (field.to_owned(), value))
                .collect(),
            expected_per_unit_counts: EXPECTED_PER_UNIT_COUNTS
                .into_iter()
                .map(|(field, value)| (field.to_owned(), value))
                .collect(),
        })
    }

    pub(crate) fn load_template(
        &self,
        repository_root: &Path,
    ) -> Result<CorridorTemplate, CorridorError> {
        let mut documents = BTreeMap::<String, Value>::new();
        for binding in &self.template_files {
            let bytes = read_bound_file(repository_root, binding)?;
            let value =
                serde_json::from_slice::<Value>(&bytes).map_err(|source| CorridorError::Json {
                    path: binding.path.clone(),
                    source,
                })?;
            require_string(&value, "formatVersion", &binding.format_version)?;
            if documents.insert(binding.role.clone(), value).is_some() {
                return Err(CorridorError::DuplicateRole(binding.role.clone()));
            }
        }
        let signalized = documents
            .remove(SIGNALIZED_TRAFFIC_ROLE)
            .ok_or_else(|| CorridorError::Missing(SIGNALIZED_TRAFFIC_ROLE.to_owned()))?;
        let spatial = documents
            .remove(SIGNALIZED_SPATIAL_ROLE)
            .ok_or_else(|| CorridorError::Missing(SIGNALIZED_SPATIAL_ROLE.to_owned()))?;
        let parking = documents
            .remove(PARKING_TRAFFIC_ROLE)
            .ok_or_else(|| CorridorError::Missing(PARKING_TRAFFIC_ROLE.to_owned()))?;
        let template = build_raw_template(&signalized, &spatial, &parking)?;
        self.validate_template(&template)?;
        Ok(template)
    }

    pub(crate) fn validate_template(
        &self,
        template: &CorridorTemplate,
    ) -> Result<(), CorridorError> {
        let actual_stage_inputs = template.stage_input_counts();
        for (field, expected) in &self.expected_stage_inputs {
            let actual = actual_stage_inputs
                .get(field)
                .copied()
                .ok_or_else(|| CorridorError::Missing(field.clone()))?;
            if actual != *expected {
                return Err(CorridorError::Mismatch {
                    path: format!("perUnitStageInputs.{field}"),
                    expected: expected.to_string(),
                    actual: actual.to_string(),
                });
            }
        }
        let actual_counts = template.domain_counts();
        for (field, expected) in &self.expected_per_unit_counts {
            let actual = actual_counts.get(field).copied().unwrap_or(0);
            if actual != *expected {
                return Err(CorridorError::Mismatch {
                    path: format!("perUnitCounts.{field}"),
                    expected: expected.to_string(),
                    actual: actual.to_string(),
                });
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BoundTemplateFile {
    role: String,
    path: String,
    format_version: String,
    byte_length: u64,
    sha256: String,
}

impl BoundTemplateFile {
    fn parse(value: &Value) -> Result<Self, CorridorError> {
        Ok(Self {
            role: required_string(value, "role")?.to_owned(),
            path: required_string(value, "path")?.to_owned(),
            format_version: required_string(value, "formatVersion")?.to_owned(),
            byte_length: required_u64(value, "byteLength")?,
            sha256: required_string(value, "sha256")?.to_owned(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct EntityRef {
    pub(crate) kind: u16,
    pub(crate) local: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TemplateEntity {
    pub(crate) reference: EntityRef,
    pub(crate) identity_references: BTreeMap<u16, EntityRef>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TemplateRelation {
    Owner {
        child: EntityRef,
        parent: EntityRef,
    },
    EdgeConnection {
        source: EntityRef,
        target: EntityRef,
    },
    RouteOccurrence {
        route: EntityRef,
        index: u32,
        edge: EntityRef,
    },
    Access {
        rule: EntityRef,
        participant: EntityRef,
        target: EntityRef,
        decision: u8,
    },
    SignalGroup {
        group: EntityRef,
        gate: EntityRef,
    },
    PhaseState {
        phase: EntityRef,
        group: EntityRef,
        state: u8,
    },
    Gate {
        path: EntityRef,
        transition_index: u32,
        gate: EntityRef,
        stop_line: EntityRef,
        edge: EntityRef,
        edge_position_bits: u32,
    },
    WaitingZone {
        path: EntityRef,
        entry_transition_index: u32,
        release_transition_index: u32,
        zone: EntityRef,
        before_gate: EntityRef,
        after_gate: EntityRef,
        capacity: u32,
    },
    Parking {
        space: EntityRef,
        entry_edge: EntityRef,
        entry_high_bits: u32,
        entry_residual_bits: u32,
        exit_edge: EntityRef,
        exit_high_bits: u32,
        exit_residual_bits: u32,
    },
    LaneCoverage {
        lane: EntityRef,
        index: u32,
        edge: EntityRef,
    },
    JunctionInternalEdge {
        junction: EntityRef,
        edge: EntityRef,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TemplateGeometryRule {
    Fixed,
    JunctionGridV1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TemplateGeometry {
    pub(crate) edge: EntityRef,
    pub(crate) frame: EntityRef,
    pub(crate) point_index: u32,
    pub(crate) x_bits: u32,
    pub(crate) y_bits: u32,
    pub(crate) z_bits: u32,
    pub(crate) coordinate_rule: TemplateGeometryRule,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CorridorTemplate {
    pub(crate) entities: Vec<TemplateEntity>,
    pub(crate) relations: Vec<TemplateRelation>,
    pub(crate) geometry: Vec<TemplateGeometry>,
}

impl CorridorTemplate {
    fn entity_counts(&self) -> BTreeMap<u16, u64> {
        let mut counts = BTreeMap::new();
        for entity in &self.entities {
            *counts.entry(entity.reference.kind).or_default() += 1;
        }
        counts
    }

    pub(crate) fn stage_input_counts(&self) -> BTreeMap<String, u64> {
        let identity_field_occurrences = self
            .entities
            .iter()
            .map(|entity| identity_field_count(entity.reference.kind))
            .sum::<u64>();
        let profiled_key_occurrences = self
            .entities
            .iter()
            .map(|entity| profiled_key_field_count(entity.reference.kind))
            .sum::<u64>();
        let identity_references = self
            .entities
            .iter()
            .map(|entity| {
                u64::try_from(entity.identity_references.len())
                    .expect("identity reference count must fit u64")
            })
            .sum::<u64>();
        let payload_references = self
            .relations
            .iter()
            .map(TemplateRelation::stable_reference_count)
            .sum::<u64>()
            + u64::try_from(self.geometry.len()).expect("geometry count must fit u64");
        BTreeMap::from([
            (
                "sourceDeclarationCount".to_owned(),
                u64::try_from(self.entities.len()).expect("entity count must fit u64"),
            ),
            (
                "identityFieldOccurrenceCount".to_owned(),
                identity_field_occurrences,
            ),
            (
                "profiledKeyOccurrenceCount".to_owned(),
                profiled_key_occurrences,
            ),
            (
                "sourceReferenceCount".to_owned(),
                identity_references + payload_references,
            ),
            (
                "sourceRelationCount".to_owned(),
                u64::try_from(self.relations.len()).expect("relation count must fit u64"),
            ),
            (
                "sourceGeometryCount".to_owned(),
                u64::try_from(self.geometry.len()).expect("geometry count must fit u64"),
            ),
        ])
    }

    pub(crate) fn domain_counts(&self) -> BTreeMap<String, u64> {
        let entity_counts = self.entity_counts();
        let mut counts = BTreeMap::new();
        for (kind, name) in ENTITY_KIND_NAMES.iter().enumerate().skip(1) {
            counts.insert(
                (*name).to_owned(),
                entity_counts
                    .get(&u16::try_from(kind).expect("entity kind must fit u16"))
                    .copied()
                    .unwrap_or(0),
            );
        }
        for relation in &self.relations {
            *counts.entry(relation.count_name().to_owned()).or_default() += 1;
        }
        counts.insert(
            "canonicalGeometryPoint".to_owned(),
            u64::try_from(self.geometry.len()).expect("geometry count must fit u64"),
        );
        counts.insert(
            "semanticOutputRecord".to_owned(),
            u64::try_from(self.entities.len() + self.relations.len() + self.geometry.len())
                .expect("semantic record count must fit u64"),
        );
        counts
    }
}

impl TemplateRelation {
    pub(crate) fn count_name(&self) -> &'static str {
        match self {
            Self::Owner { .. } => "ownerRelation",
            Self::EdgeConnection { .. } => "edgeConnection",
            Self::RouteOccurrence { .. } => "routeOccurrence",
            Self::Access { .. } => "accessRelation",
            Self::SignalGroup { .. } => "signalGroupRelation",
            Self::PhaseState { .. } => "signalPhaseStateOccurrence",
            Self::Gate { .. } => "gateOccurrence",
            Self::WaitingZone { .. } => "waitingZoneOccurrence",
            Self::Parking { .. } => "parkingSpaceAnchors",
            Self::LaneCoverage { .. } => "laneCoverageOccurrence",
            Self::JunctionInternalEdge { .. } => "junctionInternalEdgeRole",
        }
    }

    pub(crate) fn stable_reference_count(&self) -> u64 {
        match self {
            Self::Owner { .. }
            | Self::EdgeConnection { .. }
            | Self::RouteOccurrence { .. }
            | Self::SignalGroup { .. }
            | Self::PhaseState { .. }
            | Self::LaneCoverage { .. }
            | Self::JunctionInternalEdge { .. } => 1,
            Self::Access { .. } => 2,
            Self::Gate { .. } | Self::Parking { .. } => 3,
            Self::WaitingZone { .. } => 3,
        }
    }

    pub(crate) fn append_stable_references(&self, references: &mut Vec<EntityRef>) {
        match self {
            Self::Owner { parent, .. } => references.push(*parent),
            Self::EdgeConnection { target, .. } => references.push(*target),
            Self::RouteOccurrence { edge, .. } => references.push(*edge),
            Self::Access {
                participant,
                target,
                ..
            } => references.extend([*participant, *target]),
            Self::SignalGroup { gate, .. } => references.push(*gate),
            Self::PhaseState { group, .. } => references.push(*group),
            Self::Gate {
                gate,
                stop_line,
                edge,
                ..
            } => references.extend([*gate, *stop_line, *edge]),
            Self::WaitingZone {
                zone,
                before_gate,
                after_gate,
                ..
            } => references.extend([*zone, *before_gate, *after_gate]),
            Self::Parking {
                space,
                entry_edge,
                exit_edge,
                ..
            } => references.extend([*space, *entry_edge, *exit_edge]),
            Self::LaneCoverage { edge, .. } | Self::JunctionInternalEdge { edge, .. } => {
                references.push(*edge);
            }
        }
    }
}

const ENTITY_KIND_NAMES: [&str; 23] = [
    "",
    "RoadCorridor",
    "RoadSection",
    "AuthoringLane",
    "LaneEdge",
    "Junction",
    "Movement",
    "ManeuverPath",
    "ManeuverGate",
    "WaitingZone",
    "StopLine",
    "SignalGroup",
    "SignalController",
    "SignalPhase",
    "ParkingArea",
    "ParkingSpace",
    "LaneGroup",
    "FacilityBand",
    "ParticipantClass",
    "AccessRule",
    "VehicleProfile",
    "StaticRoute",
    "CanonicalFrame",
];

pub(crate) fn identity_field_count(kind: u16) -> u64 {
    match kind {
        2 | 3 | 8 | 9 | 13 | 16 | 17 => 3,
        6 | 7 => 5,
        1 | 4 | 5 | 10 | 11 | 12 | 14 | 15 | 18 | 19 | 20 | 21 | 22 => 2,
        _ => 0,
    }
}

pub(crate) fn profiled_key_field_count(kind: u16) -> u64 {
    match kind {
        6 => 3,
        1..=22 => 1,
        _ => 0,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CorridorError {
    #[error("走廊工作负载缺少字段或路径：{0}")]
    Missing(String),
    #[error("走廊工作负载字段 {path} 不匹配：期望 {expected}，实际 {actual}")]
    Mismatch {
        path: String,
        expected: String,
        actual: String,
    },
    #[error("走廊工作负载模板角色重复：{0}")]
    DuplicateRole(String),
    #[error("走廊工作负载模板路径越出仓库或不是普通相对路径：{0}")]
    InvalidPath(String),
    #[error("无法读取走廊工作负载模板 {path}：{source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("无法解析走廊工作负载模板 {path}：{source}")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("走廊工作负载引用无法解析：{0}")]
    UnknownReference(String),
    #[error("走廊工作负载存在重复声明或所有者：{0}")]
    DuplicateReference(String),
    #[error("走廊工作负载数值不是有限 f32 可表示值：{0}")]
    InvalidNumber(String),
    #[error("走廊工作负载契约不完整：{0}")]
    Contract(String),
    #[error(transparent)]
    Generator(#[from] crate::GeneratorError),
    #[error(transparent)]
    Stage(#[from] StageGenerationError),
}

fn read_bound_file(
    repository_root: &Path,
    binding: &BoundTemplateFile,
) -> Result<Vec<u8>, CorridorError> {
    let relative = Path::new(&binding.path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(CorridorError::InvalidPath(binding.path.clone()));
    }
    let path = repository_root.join(relative);
    let bytes = fs::read(&path).map_err(|source| CorridorError::Io {
        path: binding.path.clone(),
        source,
    })?;
    let actual_length = u64::try_from(bytes.len()).expect("file length must fit u64");
    if actual_length != binding.byte_length {
        return Err(CorridorError::Mismatch {
            path: format!("{}.byteLength", binding.path),
            expected: binding.byte_length.to_string(),
            actual: actual_length.to_string(),
        });
    }
    let actual_sha = lower_hex(&Sha256::digest(&bytes));
    if actual_sha != binding.sha256 {
        return Err(CorridorError::Mismatch {
            path: format!("{}.sha256", binding.path),
            expected: binding.sha256.clone(),
            actual: actual_sha,
        });
    }
    Ok(bytes)
}

#[derive(Default)]
struct DocumentRefs {
    by_kind: BTreeMap<u16, BTreeMap<String, EntityRef>>,
    lanes: Vec<Vec<EntityRef>>,
    phases: Vec<Vec<EntityRef>>,
}

impl DocumentRefs {
    fn named(&self, kind: u16, id: &str, context: &str) -> Result<EntityRef, CorridorError> {
        self.by_kind
            .get(&kind)
            .and_then(|values| values.get(id))
            .copied()
            .ok_or_else(|| {
                CorridorError::UnknownReference(format!("{context}: kind={kind} id={id}"))
            })
    }
}

fn build_raw_template(
    signalized: &Value,
    spatial: &Value,
    parking: &Value,
) -> Result<CorridorTemplate, CorridorError> {
    build_projected_template(&[signalized, parking], Some((0, spatial)), Some(1))
}

pub(crate) fn build_current_fixture_raw_template(
    traffic: &Value,
    spatial: Option<&Value>,
) -> Result<CorridorTemplate, CorridorError> {
    build_projected_template(&[traffic], spatial.map(|document| (0, document)), None)
}

fn build_projected_template(
    traffic_documents: &[&Value],
    spatial_binding: Option<(usize, &Value)>,
    synthetic_parking_document: Option<usize>,
) -> Result<CorridorTemplate, CorridorError> {
    let mut document_refs = (0..traffic_documents.len())
        .map(|_| DocumentRefs::default())
        .collect::<Vec<_>>();
    let mut entities = Vec::new();
    let mut next_local = [0_u32; 23];

    for kind in 1_u16..=21 {
        for (document_index, document) in traffic_documents.iter().enumerate() {
            register_document_entities(
                document,
                document_index,
                kind,
                &mut next_local,
                &mut document_refs,
                &mut entities,
            )?;
        }
    }
    let canonical_frame = if spatial_binding.is_some() || synthetic_parking_document.is_some() {
        let frame = EntityRef { kind: 22, local: 0 };
        entities.push(TemplateEntity {
            reference: frame,
            identity_references: BTreeMap::new(),
        });
        next_local[22] = 1;
        Some(frame)
    } else {
        None
    };

    let mut entity_positions = entities
        .iter()
        .enumerate()
        .map(|(index, entity)| (entity.reference, index))
        .collect::<BTreeMap<_, _>>();
    if entity_positions.len() != entities.len() {
        return Err(CorridorError::DuplicateReference(
            "entity kind/local tuple".to_owned(),
        ));
    }

    let mut road_owner = BTreeMap::<EntityRef, EntityRef>::new();
    for (document_index, document) in traffic_documents.iter().enumerate() {
        let refs = &document_refs[document_index];
        for corridor in array_at(document, &["roadCorridors"])? {
            let corridor_ref =
                refs.named(1, required_string(corridor, "id")?, "roadCorridors[].id")?;
            for element in array_at(corridor, &["elements"])? {
                let child =
                    if let Some(section_id) = element.get("sectionId").and_then(Value::as_str) {
                        refs.named(2, section_id, "roadCorridors[].elements[].sectionId")?
                    } else if let Some(band_id) = element.get("bandId").and_then(Value::as_str) {
                        refs.named(17, band_id, "roadCorridors[].elements[].bandId")?
                    } else {
                        return Err(CorridorError::Missing(
                            "roadCorridors[].elements[].sectionId|bandId".to_owned(),
                        ));
                    };
                if road_owner.insert(child, corridor_ref).is_some() {
                    return Err(CorridorError::DuplicateReference(format!(
                        "RoadCorridor owner for kind={} local={}",
                        child.kind, child.local
                    )));
                }
            }
        }
    }
    let expected_owned =
        usize::try_from(next_local[2] + next_local[17]).expect("owned entity count must fit usize");
    if road_owner.len() != expected_owned {
        return Err(CorridorError::Mismatch {
            path: "RoadCorridor complete owner tree".to_owned(),
            expected: expected_owned.to_string(),
            actual: road_owner.len().to_string(),
        });
    }

    let mut owner_relations = Vec::new();
    for (document_index, document) in traffic_documents.iter().enumerate() {
        let refs = &document_refs[document_index];
        let sections = array_at(document, &["roadSections"])?;
        for (section_index, section) in sections.iter().enumerate() {
            let section_ref =
                refs.named(2, required_string(section, "id")?, "roadSections[].id")?;
            for lane_ref in refs
                .lanes
                .get(section_index)
                .ok_or_else(|| CorridorError::Missing("roadSections[].lanes".to_owned()))?
            {
                set_identity_reference(
                    &mut entities,
                    &entity_positions,
                    *lane_ref,
                    32,
                    section_ref,
                )?;
                owner_relations.push(TemplateRelation::Owner {
                    child: *lane_ref,
                    parent: section_ref,
                });
            }
        }
    }
    for (document_index, document) in traffic_documents.iter().enumerate() {
        let refs = &document_refs[document_index];
        for movement in array_at(document, &["movements"])? {
            let child = refs.named(6, required_string(movement, "id")?, "movements[].id")?;
            let parent = refs.named(
                5,
                required_string(movement, "junctionId")?,
                "movements[].junctionId",
            )?;
            set_identity_reference(&mut entities, &entity_positions, child, 34, parent)?;
            owner_relations.push(TemplateRelation::Owner { child, parent });
        }
    }
    for (document_index, document) in traffic_documents.iter().enumerate() {
        let refs = &document_refs[document_index];
        for path in array_at(document, &["maneuverPaths"])? {
            let child = refs.named(7, required_string(path, "id")?, "maneuverPaths[].id")?;
            let movement = refs.named(
                6,
                required_string(path, "movementId")?,
                "maneuverPaths[].movementId",
            )?;
            let entry = refs.named(
                4,
                required_string(path, "entryEdgeId")?,
                "maneuverPaths[].entryEdgeId",
            )?;
            let exit = refs.named(
                4,
                required_string(path, "exitEdgeId")?,
                "maneuverPaths[].exitEdgeId",
            )?;
            set_identity_reference(&mut entities, &entity_positions, child, 11, movement)?;
            set_identity_reference(&mut entities, &entity_positions, child, 12, entry)?;
            set_identity_reference(&mut entities, &entity_positions, child, 13, exit)?;
            owner_relations.push(TemplateRelation::Owner {
                child,
                parent: movement,
            });
        }
    }
    for (document_index, document) in traffic_documents.iter().enumerate() {
        let refs = &document_refs[document_index];
        for gate in array_at(document, &["signals", "maneuverGates"])? {
            let child = refs.named(
                8,
                required_string(gate, "id")?,
                "signals.maneuverGates[].id",
            )?;
            let parent = refs.named(
                7,
                required_string(gate, "maneuverPathId")?,
                "signals.maneuverGates[].maneuverPathId",
            )?;
            set_identity_reference(&mut entities, &entity_positions, child, 14, parent)?;
            owner_relations.push(TemplateRelation::Owner { child, parent });
        }
    }
    for (document_index, document) in traffic_documents.iter().enumerate() {
        let refs = &document_refs[document_index];
        for zone in array_at(document, &["waitingZones"])? {
            let child = refs.named(9, required_string(zone, "id")?, "waitingZones[].id")?;
            let parent = refs.named(
                7,
                required_string(zone, "maneuverPathId")?,
                "waitingZones[].maneuverPathId",
            )?;
            set_identity_reference(&mut entities, &entity_positions, child, 14, parent)?;
            owner_relations.push(TemplateRelation::Owner { child, parent });
        }
    }
    for (document_index, document) in traffic_documents.iter().enumerate() {
        let refs = &document_refs[document_index];
        let controllers = array_at(document, &["signals", "controllers"])?;
        for (controller_index, controller) in controllers.iter().enumerate() {
            let parent = refs.named(
                12,
                required_string(controller, "id")?,
                "signals.controllers[].id",
            )?;
            for child in refs
                .phases
                .get(controller_index)
                .ok_or_else(|| CorridorError::Missing("signals.controllers[].phases".to_owned()))?
            {
                set_identity_reference(&mut entities, &entity_positions, *child, 20, parent)?;
                owner_relations.push(TemplateRelation::Owner {
                    child: *child,
                    parent,
                });
            }
        }
    }
    for (document_index, document) in traffic_documents.iter().enumerate() {
        let refs = &document_refs[document_index];
        for space in array_at(document, &["parking", "spaces"])? {
            let child = refs.named(15, required_string(space, "id")?, "parking.spaces[].id")?;
            if let Some(area_id) = space.get("areaId").and_then(Value::as_str) {
                let parent = refs.named(14, area_id, "parking.spaces[].areaId")?;
                owner_relations.push(TemplateRelation::Owner { child, parent });
            }
        }
    }
    for (document_index, document) in traffic_documents.iter().enumerate() {
        let refs = &document_refs[document_index];
        for group in array_at(document, &["laneGroups"])? {
            let child = refs.named(16, required_string(group, "id")?, "laneGroups[].id")?;
            let parent = refs.named(
                2,
                required_string(group, "roadSectionId")?,
                "laneGroups[].roadSectionId",
            )?;
            set_identity_reference(&mut entities, &entity_positions, child, 32, parent)?;
            owner_relations.push(TemplateRelation::Owner { child, parent });
        }
    }
    for (child, parent) in &road_owner {
        let tag = match child.kind {
            2 | 17 => 33,
            _ => {
                return Err(CorridorError::Mismatch {
                    path: "RoadCorridor owner child kind".to_owned(),
                    expected: "RoadSection or FacilityBand".to_owned(),
                    actual: child.kind.to_string(),
                });
            }
        };
        set_identity_reference(&mut entities, &entity_positions, *child, tag, *parent)?;
        owner_relations.push(TemplateRelation::Owner {
            child: *child,
            parent: *parent,
        });
    }

    validate_identity_reference_completeness(&entities)?;

    let mut relations = owner_relations;
    append_edge_connections(traffic_documents, &document_refs, &mut relations)?;
    append_route_occurrences(traffic_documents, &document_refs, &mut relations)?;
    append_access_relations(traffic_documents, &document_refs, &mut relations)?;
    append_signal_relations(traffic_documents, &document_refs, &mut relations)?;
    append_gate_and_waiting_occurrences(traffic_documents, &document_refs, &mut relations)?;
    append_parking_anchors(traffic_documents, &document_refs, &mut relations)?;
    append_lane_coverage(traffic_documents, &document_refs, &mut relations)?;
    append_junction_internal_edges(traffic_documents, &document_refs, &mut relations)?;

    let geometry = build_projected_geometry(
        traffic_documents,
        spatial_binding,
        synthetic_parking_document,
        &document_refs,
        canonical_frame,
    )?;
    entity_positions.clear();
    Ok(CorridorTemplate {
        entities,
        relations,
        geometry,
    })
}

fn register_document_entities(
    document: &Value,
    document_index: usize,
    kind: u16,
    next_local: &mut [u32; 23],
    document_refs: &mut [DocumentRefs],
    entities: &mut Vec<TemplateEntity>,
) -> Result<(), CorridorError> {
    if kind == 3 {
        let mut section_lanes = Vec::new();
        for section in array_at(document, &["roadSections"])? {
            let mut lanes = Vec::new();
            for _lane in array_at(section, &["lanes"])? {
                let reference = register_entity(kind, next_local, entities)?;
                lanes.push(reference);
            }
            section_lanes.push(lanes);
        }
        document_refs[document_index].lanes = section_lanes;
        return Ok(());
    }
    if kind == 13 {
        let mut controller_phases = Vec::new();
        for controller in array_at(document, &["signals", "controllers"])? {
            let mut phases = Vec::new();
            for _phase in array_at(controller, &["phases"])? {
                // SignalPhase IDs are controller-local in the source format and may repeat
                // across controllers. The controller nesting and phase array position are the
                // complete identity source; no document-global ID map is valid here.
                let reference = register_entity(kind, next_local, entities)?;
                phases.push(reference);
            }
            controller_phases.push(phases);
        }
        document_refs[document_index].phases = controller_phases;
        return Ok(());
    }

    let path: &[&str] = match kind {
        1 => &["roadCorridors"],
        2 => &["roadSections"],
        4 => &["laneGraph", "edges"],
        5 => &["junctions"],
        6 => &["movements"],
        7 => &["maneuverPaths"],
        8 => &["signals", "maneuverGates"],
        9 => &["waitingZones"],
        10 => &["signals", "stopLines"],
        11 => &["signals", "groups"],
        12 => &["signals", "controllers"],
        14 => &["parking", "areas"],
        15 => &["parking", "spaces"],
        16 => &["laneGroups"],
        17 => &["facilityBands"],
        18 => &["participantClasses"],
        19 => &["accessRules"],
        20 => &["vehicleProfiles"],
        21 => &["routes"],
        _ => {
            return Err(CorridorError::Mismatch {
                path: "entity kind".to_owned(),
                expected: "1..=21".to_owned(),
                actual: kind.to_string(),
            });
        }
    };
    for value in array_at(document, path)? {
        register_named_entity(
            kind,
            value,
            &format!("{}.id", path.join(".")),
            next_local,
            entities,
            &mut document_refs[document_index],
        )?;
    }
    Ok(())
}

fn register_entity(
    kind: u16,
    next_local: &mut [u32; 23],
    entities: &mut Vec<TemplateEntity>,
) -> Result<EntityRef, CorridorError> {
    let index = usize::from(kind);
    let local = next_local[index];
    next_local[index] = local
        .checked_add(1)
        .ok_or_else(|| CorridorError::Mismatch {
            path: "entity local ordinal".to_owned(),
            expected: "u32".to_owned(),
            actual: "overflow".to_owned(),
        })?;
    let reference = EntityRef { kind, local };
    entities.push(TemplateEntity {
        reference,
        identity_references: BTreeMap::new(),
    });
    Ok(reference)
}

fn register_named_entity(
    kind: u16,
    value: &Value,
    path: &str,
    next_local: &mut [u32; 23],
    entities: &mut Vec<TemplateEntity>,
    document_refs: &mut DocumentRefs,
) -> Result<EntityRef, CorridorError> {
    let id = required_string(value, "id")?;
    let reference = register_entity(kind, next_local, entities)?;
    let previous = document_refs
        .by_kind
        .entry(kind)
        .or_default()
        .insert(id.to_owned(), reference);
    if previous.is_some() {
        return Err(CorridorError::DuplicateReference(format!("{path}: {id}")));
    }
    Ok(reference)
}

fn set_identity_reference(
    entities: &mut [TemplateEntity],
    positions: &BTreeMap<EntityRef, usize>,
    source: EntityRef,
    tag: u16,
    target: EntityRef,
) -> Result<(), CorridorError> {
    let index = positions
        .get(&source)
        .copied()
        .ok_or_else(|| CorridorError::UnknownReference(format!("entity {source:?}")))?;
    if entities[index]
        .identity_references
        .insert(tag, target)
        .is_some()
    {
        return Err(CorridorError::DuplicateReference(format!(
            "identity tag {tag} for {source:?}"
        )));
    }
    Ok(())
}

fn validate_identity_reference_completeness(
    entities: &[TemplateEntity],
) -> Result<(), CorridorError> {
    for entity in entities {
        let required = match entity.reference.kind {
            2 => &[33][..],
            3 => &[32][..],
            6 => &[34][..],
            7 => &[11, 12, 13][..],
            8 | 9 => &[14][..],
            13 => &[20][..],
            16 => &[32][..],
            17 => &[33][..],
            _ => &[][..],
        };
        let actual = entity
            .identity_references
            .keys()
            .copied()
            .collect::<Vec<_>>();
        if actual != required {
            return Err(CorridorError::Mismatch {
                path: format!("identity references for {:?}", entity.reference),
                expected: format!("{required:?}"),
                actual: format!("{actual:?}"),
            });
        }
    }
    Ok(())
}

fn append_edge_connections(
    documents: &[&Value],
    document_refs: &[DocumentRefs],
    relations: &mut Vec<TemplateRelation>,
) -> Result<(), CorridorError> {
    for (document_index, document) in documents.iter().enumerate() {
        let refs = &document_refs[document_index];
        for edge in array_at(document, &["laneGraph", "edges"])? {
            let source = refs.named(4, required_string(edge, "id")?, "laneGraph.edges[].id")?;
            for connection in array_at(edge, &["connections"])? {
                let target = refs.named(
                    4,
                    required_string(connection, "toEdgeId")?,
                    "laneGraph.edges[].connections[].toEdgeId",
                )?;
                relations.push(TemplateRelation::EdgeConnection { source, target });
            }
        }
    }
    Ok(())
}

fn append_route_occurrences(
    documents: &[&Value],
    document_refs: &[DocumentRefs],
    relations: &mut Vec<TemplateRelation>,
) -> Result<(), CorridorError> {
    for (document_index, document) in documents.iter().enumerate() {
        let refs = &document_refs[document_index];
        for route in array_at(document, &["routes"])? {
            let route_ref = refs.named(21, required_string(route, "id")?, "routes[].id")?;
            for (index, edge_id) in array_at(route, &["edgeIds"])?.iter().enumerate() {
                let edge = refs.named(
                    4,
                    edge_id
                        .as_str()
                        .ok_or_else(|| CorridorError::Missing("routes[].edgeIds[]".to_owned()))?,
                    "routes[].edgeIds[]",
                )?;
                relations.push(TemplateRelation::RouteOccurrence {
                    route: route_ref,
                    index: u32::try_from(index).expect("route occurrence index must fit u32"),
                    edge,
                });
            }
        }
    }
    Ok(())
}

fn append_access_relations(
    documents: &[&Value],
    document_refs: &[DocumentRefs],
    relations: &mut Vec<TemplateRelation>,
) -> Result<(), CorridorError> {
    for (document_index, document) in documents.iter().enumerate() {
        let refs = &document_refs[document_index];
        for rule in array_at(document, &["accessRules"])? {
            let rule_ref = refs.named(19, required_string(rule, "id")?, "accessRules[].id")?;
            let target = required_object(rule, "target")?;
            let target_kind_name = required_string(target, "kind")?;
            let target_kind = match target_kind_name {
                "roadSection" => 2,
                "laneEdge" => 4,
                "maneuverPath" => 7,
                "laneGroup" => 16,
                "facilityBand" => 17,
                _ => {
                    return Err(CorridorError::Mismatch {
                        path: "accessRules[].target.kind".to_owned(),
                        expected: "roadSection|laneEdge|maneuverPath|laneGroup|facilityBand"
                            .to_owned(),
                        actual: target_kind_name.to_owned(),
                    });
                }
            };
            let target_ref = refs.named(
                target_kind,
                required_string(target, "id")?,
                "accessRules[].target.id",
            )?;
            let decision = match required_string(rule, "effect")? {
                "deny" => 0,
                "allow" => 1,
                actual => {
                    return Err(CorridorError::Mismatch {
                        path: "accessRules[].effect".to_owned(),
                        expected: "deny|allow".to_owned(),
                        actual: actual.to_owned(),
                    });
                }
            };
            for participant_id in array_at(rule, &["participantClassIds"])? {
                let participant = refs.named(
                    18,
                    participant_id.as_str().ok_or_else(|| {
                        CorridorError::Missing("accessRules[].participantClassIds[]".to_owned())
                    })?,
                    "accessRules[].participantClassIds[]",
                )?;
                relations.push(TemplateRelation::Access {
                    rule: rule_ref,
                    participant,
                    target: target_ref,
                    decision,
                });
            }
        }
    }
    Ok(())
}

fn append_signal_relations(
    documents: &[&Value],
    document_refs: &[DocumentRefs],
    relations: &mut Vec<TemplateRelation>,
) -> Result<(), CorridorError> {
    for (document_index, document) in documents.iter().enumerate() {
        let refs = &document_refs[document_index];
        for gate in array_at(document, &["signals", "maneuverGates"])? {
            let control = required_object(gate, "signalControl")?;
            match required_string(control, "kind")? {
                "group" => {
                    let group = refs.named(
                        11,
                        required_string(control, "groupId")?,
                        "signals.maneuverGates[].signalControl.groupId",
                    )?;
                    let gate_ref = refs.named(
                        8,
                        required_string(gate, "id")?,
                        "signals.maneuverGates[].id",
                    )?;
                    relations.push(TemplateRelation::SignalGroup {
                        group,
                        gate: gate_ref,
                    });
                }
                "none" => {}
                actual => {
                    return Err(CorridorError::Mismatch {
                        path: "signals.maneuverGates[].signalControl.kind".to_owned(),
                        expected: "group|none".to_owned(),
                        actual: actual.to_owned(),
                    });
                }
            }
        }

        let controllers = array_at(document, &["signals", "controllers"])?;
        for (controller_index, controller) in controllers.iter().enumerate() {
            let phases = array_at(controller, &["phases"])?;
            for (phase_index, phase) in phases.iter().enumerate() {
                let phase_ref = *refs
                    .phases
                    .get(controller_index)
                    .and_then(|values| values.get(phase_index))
                    .ok_or_else(|| {
                        CorridorError::Missing("signals.controllers[].phases[]".to_owned())
                    })?;
                for state in array_at(phase, &["states"])? {
                    let group = refs.named(
                        11,
                        required_string(state, "groupId")?,
                        "signals.controllers[].phases[].states[].groupId",
                    )?;
                    let state_code = match required_string(state, "aspect")? {
                        "red" => 0,
                        "yellow" => 1,
                        "green" => 2,
                        actual => {
                            return Err(CorridorError::Mismatch {
                                path: "signals.controllers[].phases[].states[].aspect".to_owned(),
                                expected: "red|yellow|green".to_owned(),
                                actual: actual.to_owned(),
                            });
                        }
                    };
                    relations.push(TemplateRelation::PhaseState {
                        phase: phase_ref,
                        group,
                        state: state_code,
                    });
                }
            }
        }
    }
    Ok(())
}

fn append_gate_and_waiting_occurrences(
    documents: &[&Value],
    document_refs: &[DocumentRefs],
    relations: &mut Vec<TemplateRelation>,
) -> Result<(), CorridorError> {
    for (document_index, document) in documents.iter().enumerate() {
        let refs = &document_refs[document_index];
        let paths = array_at(document, &["maneuverPaths"])?;
        let gates = array_at(document, &["signals", "maneuverGates"])?;
        for gate in gates {
            let path_id = required_string(gate, "maneuverPathId")?;
            let path_value = find_named(paths, path_id, "maneuverPaths")?;
            let path = refs.named(7, path_id, "signals.maneuverGates[].maneuverPathId")?;
            let transition_index = required_u32(gate, "transitionIndex")?;
            let edge_id = ordered_path_edge_id(path_value, transition_index)?;
            let edge = refs.named(4, edge_id, "gate transition LaneEdge")?;
            let gate_ref = refs.named(
                8,
                required_string(gate, "id")?,
                "signals.maneuverGates[].id",
            )?;
            let stop_line = refs.named(
                10,
                required_string(gate, "stopLineId")?,
                "signals.maneuverGates[].stopLineId",
            )?;
            relations.push(TemplateRelation::Gate {
                path,
                transition_index,
                gate: gate_ref,
                stop_line,
                edge,
                edge_position_bits: 1.0_f32.to_bits(),
            });
        }

        for zone in array_at(document, &["waitingZones"])? {
            let path = refs.named(
                7,
                required_string(zone, "maneuverPathId")?,
                "waitingZones[].maneuverPathId",
            )?;
            let before_id = required_string(zone, "entryGateId")?;
            let after_id = required_string(zone, "releaseGateId")?;
            let before_gate_value = find_named(gates, before_id, "signals.maneuverGates")?;
            let after_gate_value = find_named(gates, after_id, "signals.maneuverGates")?;
            relations.push(TemplateRelation::WaitingZone {
                path,
                entry_transition_index: required_u32(before_gate_value, "transitionIndex")?,
                release_transition_index: required_u32(after_gate_value, "transitionIndex")?,
                zone: refs.named(9, required_string(zone, "id")?, "waitingZones[].id")?,
                before_gate: refs.named(8, before_id, "waitingZones[].entryGateId")?,
                after_gate: refs.named(8, after_id, "waitingZones[].releaseGateId")?,
                capacity: required_u32(zone, "maxOccupancy")?,
            });
        }
    }
    Ok(())
}

fn append_parking_anchors(
    documents: &[&Value],
    document_refs: &[DocumentRefs],
    relations: &mut Vec<TemplateRelation>,
) -> Result<(), CorridorError> {
    for (document_index, document) in documents.iter().enumerate() {
        let refs = &document_refs[document_index];
        for space in array_at(document, &["parking", "spaces"])? {
            let entry = required_object(space, "entry")?;
            let exit = required_object(space, "exit")?;
            let (entry_high_bits, entry_residual_bits) =
                progress_bits(required_f64(entry, "progress")?)?;
            let (exit_high_bits, exit_residual_bits) =
                progress_bits(required_f64(exit, "progress")?)?;
            relations.push(TemplateRelation::Parking {
                space: refs.named(15, required_string(space, "id")?, "parking.spaces[].id")?,
                entry_edge: refs.named(
                    4,
                    required_string(entry, "edgeId")?,
                    "parking.spaces[].entry.edgeId",
                )?,
                entry_high_bits,
                entry_residual_bits,
                exit_edge: refs.named(
                    4,
                    required_string(exit, "edgeId")?,
                    "parking.spaces[].exit.edgeId",
                )?,
                exit_high_bits,
                exit_residual_bits,
            });
        }
    }
    Ok(())
}

fn append_lane_coverage(
    documents: &[&Value],
    document_refs: &[DocumentRefs],
    relations: &mut Vec<TemplateRelation>,
) -> Result<(), CorridorError> {
    for (document_index, document) in documents.iter().enumerate() {
        let refs = &document_refs[document_index];
        for (section_index, section) in array_at(document, &["roadSections"])?.iter().enumerate() {
            for (lane_index, lane) in array_at(section, &["lanes"])?.iter().enumerate() {
                let lane_ref = *refs
                    .lanes
                    .get(section_index)
                    .and_then(|lanes| lanes.get(lane_index))
                    .ok_or_else(|| CorridorError::Missing("roadSections[].lanes[]".to_owned()))?;
                for (index, edge_id) in array_at(lane, &["edgeIds"])?.iter().enumerate() {
                    relations.push(TemplateRelation::LaneCoverage {
                        lane: lane_ref,
                        index: u32::try_from(index).expect("lane coverage index must fit u32"),
                        edge: refs.named(
                            4,
                            edge_id.as_str().ok_or_else(|| {
                                CorridorError::Missing(
                                    "roadSections[].lanes[].edgeIds[]".to_owned(),
                                )
                            })?,
                            "roadSections[].lanes[].edgeIds[]",
                        )?,
                    });
                }
            }
        }
    }
    Ok(())
}

fn append_junction_internal_edges(
    documents: &[&Value],
    document_refs: &[DocumentRefs],
    relations: &mut Vec<TemplateRelation>,
) -> Result<(), CorridorError> {
    for (document_index, document) in documents.iter().enumerate() {
        let refs = &document_refs[document_index];
        let movements = array_at(document, &["movements"])?;
        for path in array_at(document, &["maneuverPaths"])? {
            let movement_id = required_string(path, "movementId")?;
            let movement = find_named(movements, movement_id, "movements")?;
            let junction = refs.named(
                5,
                required_string(movement, "junctionId")?,
                "movements[].junctionId",
            )?;
            for edge_id in array_at(path, &["internalEdgeIds"])? {
                relations.push(TemplateRelation::JunctionInternalEdge {
                    junction,
                    edge: refs.named(
                        4,
                        edge_id.as_str().ok_or_else(|| {
                            CorridorError::Missing("maneuverPaths[].internalEdgeIds[]".to_owned())
                        })?,
                        "maneuverPaths[].internalEdgeIds[]",
                    )?,
                });
            }
        }
    }
    Ok(())
}

fn build_projected_geometry(
    traffic_documents: &[&Value],
    spatial_binding: Option<(usize, &Value)>,
    synthetic_parking_document: Option<usize>,
    document_refs: &[DocumentRefs],
    frame: Option<EntityRef>,
) -> Result<Vec<TemplateGeometry>, CorridorError> {
    let mut geometry = Vec::new();
    if let Some((traffic_index, spatial)) = spatial_binding {
        let frame =
            frame.ok_or_else(|| CorridorError::Missing("spatial canonical frame".to_owned()))?;
        let traffic = traffic_documents
            .get(traffic_index)
            .copied()
            .ok_or_else(|| CorridorError::Missing("spatial traffic document".to_owned()))?;
        let traffic_refs = document_refs
            .get(traffic_index)
            .ok_or_else(|| CorridorError::Missing("spatial traffic references".to_owned()))?;
        append_spatial_geometry(&mut geometry, traffic, spatial, traffic_refs, frame)?;
    }
    if let Some(parking_index) = synthetic_parking_document {
        let frame = frame.ok_or_else(|| {
            CorridorError::Missing("synthetic parking canonical frame".to_owned())
        })?;
        let parking = traffic_documents
            .get(parking_index)
            .copied()
            .ok_or_else(|| CorridorError::Missing("synthetic parking document".to_owned()))?;
        let parking_refs = document_refs
            .get(parking_index)
            .ok_or_else(|| CorridorError::Missing("synthetic parking references".to_owned()))?;
        append_synthetic_parking_geometry(&mut geometry, parking, parking_refs, frame)?;
    }
    Ok(geometry)
}

fn append_spatial_geometry(
    geometry: &mut Vec<TemplateGeometry>,
    traffic: &Value,
    spatial: &Value,
    traffic_refs: &DocumentRefs,
    frame: EntityRef,
) -> Result<(), CorridorError> {
    let mut spatial_points = BTreeMap::<String, Vec<[u32; 3]>>::new();
    for edge in array_at(spatial, &["edges"])? {
        let id = required_string(edge, "trafficEdgeId")?;
        let mut points = Vec::new();
        for point in array_at(edge, &["centerline", "points"])? {
            let values = point.as_array().ok_or_else(|| {
                CorridorError::Missing("spatial.edges[].centerline.points[]".to_owned())
            })?;
            if values.len() != 3 {
                return Err(CorridorError::Mismatch {
                    path: "spatial.edges[].centerline.points[].length".to_owned(),
                    expected: "3".to_owned(),
                    actual: values.len().to_string(),
                });
            }
            points.push([
                canonical_f32_bits(
                    values[0].as_f64().ok_or_else(|| {
                        CorridorError::Missing("spatial.edges[].centerline.points[][0]".to_owned())
                    })?,
                    "spatial x",
                )?,
                canonical_f32_bits(
                    values[1].as_f64().ok_or_else(|| {
                        CorridorError::Missing("spatial.edges[].centerline.points[][1]".to_owned())
                    })?,
                    "spatial y",
                )?,
                canonical_f32_bits(
                    values[2].as_f64().ok_or_else(|| {
                        CorridorError::Missing("spatial.edges[].centerline.points[][2]".to_owned())
                    })?,
                    "spatial z",
                )?,
            ]);
        }
        if spatial_points.insert(id.to_owned(), points).is_some() {
            return Err(CorridorError::DuplicateReference(format!(
                "spatial trafficEdgeId {id}"
            )));
        }
    }

    for edge in array_at(traffic, &["laneGraph", "edges"])? {
        let id = required_string(edge, "id")?;
        let edge_ref = traffic_refs.named(4, id, "laneGraph.edges[].id")?;
        let points = spatial_points
            .remove(id)
            .ok_or_else(|| CorridorError::UnknownReference(format!("spatial edge {id}")))?;
        for (point_index, bits) in points.into_iter().enumerate() {
            geometry.push(TemplateGeometry {
                edge: edge_ref,
                frame,
                point_index: u32::try_from(point_index).expect("geometry point index must fit u32"),
                x_bits: bits[0],
                y_bits: bits[1],
                z_bits: bits[2],
                coordinate_rule: TemplateGeometryRule::Fixed,
            });
        }
    }
    if !spatial_points.is_empty() {
        return Err(CorridorError::Mismatch {
            path: "spatial edge coverage".to_owned(),
            expected: "no unjoined spatial edges".to_owned(),
            actual: spatial_points.len().to_string(),
        });
    }
    Ok(())
}

fn append_synthetic_parking_geometry(
    geometry: &mut Vec<TemplateGeometry>,
    parking: &Value,
    parking_refs: &DocumentRefs,
    frame: EntityRef,
) -> Result<(), CorridorError> {
    for edge in array_at(parking, &["laneGraph", "edges"])? {
        let edge_ref = parking_refs.named(
            4,
            required_string(edge, "id")?,
            "parking laneGraph.edges[].id",
        )?;
        let x0 = f64::from(edge_ref.local);
        for (point_index, x) in [x0, x0 + 1.0].into_iter().enumerate() {
            geometry.push(TemplateGeometry {
                edge: edge_ref,
                frame,
                point_index: u32::try_from(point_index)
                    .expect("synthetic point index must fit u32"),
                x_bits: canonical_f32_bits(x, "parking synthetic x")?,
                y_bits: 0.0_f32.to_bits(),
                z_bits: 0.0_f32.to_bits(),
                coordinate_rule: TemplateGeometryRule::Fixed,
            });
        }
    }
    Ok(())
}

fn ordered_path_edge_id(path: &Value, transition_index: u32) -> Result<&str, CorridorError> {
    if transition_index == 0 {
        return required_string(path, "entryEdgeId");
    }
    let internal = array_at(path, &["internalEdgeIds"])?;
    let internal_index =
        usize::try_from(transition_index - 1).expect("transition index must fit usize");
    if let Some(value) = internal.get(internal_index) {
        return value
            .as_str()
            .ok_or_else(|| CorridorError::Missing("maneuverPaths[].internalEdgeIds[]".to_owned()));
    }
    if internal_index == internal.len() {
        return required_string(path, "exitEdgeId");
    }
    Err(CorridorError::Mismatch {
        path: "signals.maneuverGates[].transitionIndex".to_owned(),
        expected: format!("0..={}", internal.len()),
        actual: transition_index.to_string(),
    })
}

fn progress_bits(value: f64) -> Result<(u32, u32), CorridorError> {
    let high = canonical_f32(value, "parking progress")?;
    let residual = canonical_f32(f64::from(high) - value, "parking progress residual")?;
    Ok((high.to_bits(), residual.to_bits()))
}

fn canonical_f32(value: f64, path: &str) -> Result<f32, CorridorError> {
    let converted = value as f32;
    if !value.is_finite() || !converted.is_finite() {
        return Err(CorridorError::InvalidNumber(path.to_owned()));
    }
    Ok(if converted == 0.0 { 0.0 } else { converted })
}

fn canonical_f32_bits(value: f64, path: &str) -> Result<u32, CorridorError> {
    canonical_f32(value, path).map(f32::to_bits)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct UnitEntityRef {
    pub(crate) unit: u32,
    pub(crate) entity: EntityRef,
}

#[derive(Clone, Debug)]
pub(crate) struct CompiledDeclaration {
    pub(crate) owner: UnitEntityRef,
    pub(crate) stable_id: [u8; 16],
    pub(crate) fields: Vec<IdentityField>,
}

#[derive(Clone, Debug)]
enum LocalIndexRule {
    Absent,
    Explicit(u32),
    OwnerPayloadOrder,
    GateOrder(u32, [u8; 16]),
    WaitingOrder(u32, u32, [u8; 16]),
}

#[derive(Clone, Debug)]
struct PendingRecord {
    record_kind: u16,
    owner: UnitEntityRef,
    local_index: LocalIndexRule,
    payload: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(crate) struct CorridorCaseOutput {
    pub(crate) summary: CorridorStageSummary,
    pub(crate) records: Vec<SemanticRecord>,
    pub(crate) semantic_record_stream: Vec<u8>,
    pub(crate) materialization: CorridorMaterialization,
}

#[derive(Debug)]
pub(crate) struct CorridorStageExecution {
    graph_profile: GraphProfileId,
    n: u32,
    counts: IdentityAggregateCounts,
    stages: StageBreakdown,
    records: Vec<SemanticRecord>,
    semantic_record_stream: Vec<u8>,
    materialization: CorridorMaterialization,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorridorStageSummary {
    pub graph_profile: GraphProfileId,
    pub n: u32,
    pub counts: IdentityAggregateCounts,
    pub stages: StageBreakdown,
    pub record_kind_counts: BTreeMap<String, u64>,
    pub semantic_digest_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CorridorMaterialization {
    source_spans: Vec<SourceSpanRecord>,
    source_records: Vec<TypedAstStageRecord>,
    source_payload: Vec<u8>,
    typed_records: Vec<TypedAstStageRecord>,
    typed_payload: Vec<u8>,
    hir_records: Vec<HirStageRecord>,
    hir_payload: Vec<u8>,
    mir_records: Vec<MirLirStageRecord>,
    mir_payload: Vec<u8>,
    lir_records: Vec<MirLirStageRecord>,
    lir_payload: Vec<u8>,
    diagnostics: Vec<u8>,
    scratch: Vec<u64>,
    pub(crate) output: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorridorKnownVectorDocument {
    pub schema: &'static str,
    pub schema_version: u32,
    pub source_workload_manifest_sha256: String,
    pub workload_id: &'static str,
    pub n: u32,
    pub string_profile: &'static str,
    pub vectors: Vec<CorridorStageSummary>,
}

pub fn build_corridor_known_vectors(
    trusted: &TrustedContract,
) -> Result<CorridorKnownVectorDocument, CorridorError> {
    let generator = trusted
        .generator_contract()
        .map_err(|error| CorridorError::Contract(error.to_string()))?;
    let identity = trusted
        .identity_contract()
        .map_err(|error| CorridorError::Contract(error.to_string()))?;
    let stage = trusted
        .stage_contract()
        .map_err(|error| CorridorError::Contract(error.to_string()))?;
    let contract = CorridorContract::from_manifest(&trusted.workload_manifest)?;
    let template = contract.load_template(&crate::repository_root())?;
    let mut vectors = Vec::with_capacity(GraphProfileId::ALL.len());
    for graph_profile in GraphProfileId::ALL {
        vectors.push(
            build_corridor_stage_case(
                &generator,
                &identity,
                &stage,
                &contract,
                &template,
                graph_profile,
                1,
            )?
            .summary,
        );
    }
    Ok(CorridorKnownVectorDocument {
        schema: CORRIDOR_KNOWN_VECTOR_SCHEMA,
        schema_version: 1,
        source_workload_manifest_sha256: trusted.descriptor.workload_manifest.sha256.clone(),
        workload_id: CORRIDOR_WORKLOAD_ID,
        n: 1,
        string_profile: SHORT_UNIQUE_PROFILE_ID,
        vectors,
    })
}

pub fn build_corridor_stage_summary(
    trusted: &TrustedContract,
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<CorridorStageSummary, CorridorError> {
    let generator = trusted
        .generator_contract()
        .map_err(|error| CorridorError::Contract(error.to_string()))?;
    let identity = trusted
        .identity_contract()
        .map_err(|error| CorridorError::Contract(error.to_string()))?;
    let stage = trusted
        .stage_contract()
        .map_err(|error| CorridorError::Contract(error.to_string()))?;
    let contract = CorridorContract::from_manifest(&trusted.workload_manifest)?;
    let template = contract.load_template(&crate::repository_root())?;
    Ok(build_corridor_stage_case(
        &generator,
        &identity,
        &stage,
        &contract,
        &template,
        graph_profile,
        n,
    )?
    .summary)
}

pub(crate) fn build_corridor_stage_case(
    generator: &GeneratorContract,
    identity: &IdentityContract,
    stage: &StageContract,
    contract: &CorridorContract,
    template: &CorridorTemplate,
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<CorridorCaseOutput, CorridorError> {
    contract.validate_template(template)?;
    let execution = execute_template_stage_case(
        generator,
        identity,
        stage,
        CORRIDOR_WORKLOAD_ID,
        template,
        graph_profile,
        n,
    )?;
    finalize_template_stage_case(execution)
}

pub(crate) fn execute_template_stage_case(
    generator: &GeneratorContract,
    identity: &IdentityContract,
    stage: &StageContract,
    workload_id: &str,
    template: &CorridorTemplate,
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<CorridorStageExecution, CorridorError> {
    execute_template_stage_case_inner(
        generator,
        identity,
        stage,
        workload_id,
        template,
        graph_profile,
        n,
    )
}

#[allow(clippy::too_many_arguments)]
fn execute_template_stage_case_inner(
    generator: &GeneratorContract,
    identity: &IdentityContract,
    stage: &StageContract,
    workload_id: &str,
    template: &CorridorTemplate,
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<CorridorStageExecution, CorridorError> {
    if n == 0 {
        return Err(CorridorError::Mismatch {
            path: "N".to_owned(),
            expected: "at least 1".to_owned(),
            actual: "0".to_owned(),
        });
    }
    let graph = expand_module_graph(generator, workload_id, graph_profile, n)?;
    let declarations = compile_declarations(identity, template, &graph, n)?;
    let (mut records, unsorted_records) =
        compile_semantic_records(identity, template, &declarations, n)?;
    records.sort_by(|left, right| left.canonical_key().cmp(&right.canonical_key()));
    let semantic_record_stream = encode_semantic_record_stream(identity, &records);
    let counts =
        build_aggregate_counts(stage, template, &graph, &records, &semantic_record_stream)?;
    let stages = build_stage_breakdown(&counts)?;
    let materialization = materialize_corridor_stages(
        generator,
        identity,
        stage,
        template,
        &graph,
        &declarations,
        &unsorted_records,
        &records,
        &semantic_record_stream,
        &counts,
        &stages,
    )?;
    Ok(CorridorStageExecution {
        graph_profile,
        n,
        counts,
        stages,
        records,
        semantic_record_stream,
        materialization,
    })
}

pub(crate) fn finalize_template_stage_case(
    execution: CorridorStageExecution,
) -> Result<CorridorCaseOutput, CorridorError> {
    let CorridorStageExecution {
        graph_profile,
        n,
        counts,
        stages,
        records,
        semantic_record_stream,
        materialization,
    } = execution;
    let semantic_digest_sha256 = lower_hex(&Sha256::digest(&semantic_record_stream));
    let record_kind_counts = count_record_kinds(&records);
    verify_materialization(&materialization, &counts, &stages)?;
    Ok(CorridorCaseOutput {
        summary: CorridorStageSummary {
            graph_profile,
            n,
            counts,
            stages,
            record_kind_counts,
            semantic_digest_sha256,
        },
        records,
        semantic_record_stream,
        materialization,
    })
}

pub(crate) fn template_semantic_payload_bytes_per_unit(
    generator: &GeneratorContract,
    identity: &IdentityContract,
    workload_id: &str,
    template: &CorridorTemplate,
) -> Result<u64, CorridorError> {
    let graph = crate::expand_module_graph(generator, workload_id, GraphProfileId::WideStar, 1)?;
    let declarations = compile_declarations(identity, template, &graph, 1)?;
    let (_, records) = compile_semantic_records(identity, template, &declarations, 1)?;
    records.iter().try_fold(0_u64, |total, record| {
        add_u64(
            total,
            usize_u64(record.payload.len(), "semanticPayloadByteCount"),
            "semanticPayloadByteCount",
        )
    })
}

fn compile_declarations(
    identity: &IdentityContract,
    template: &CorridorTemplate,
    graph: &crate::ExpandedModuleGraph,
    n: u32,
) -> Result<Vec<CompiledDeclaration>, CorridorError> {
    let bindings = identity
        .bindings
        .iter()
        .map(|binding| (binding.entity_kind_code, binding))
        .collect::<BTreeMap<_, _>>();
    let entities = template
        .entities
        .iter()
        .map(|entity| (entity.reference, entity))
        .collect::<BTreeMap<_, _>>();
    let mut compiled = Vec::with_capacity(
        usize::try_from(n)
            .expect("N must fit usize")
            .saturating_mul(template.entities.len()),
    );
    let mut stable_ids = BTreeMap::<UnitEntityRef, [u8; 16]>::new();
    for unit in 0..n {
        let module_name = format!("unit/{unit:08x}");
        let namespace = graph
            .modules
            .iter()
            .find(|module| module.canonical_name == module_name)
            .ok_or_else(|| CorridorError::Missing(module_name.clone()))?
            .namespace_id
            .as_bytes();
        for entity in &template.entities {
            let binding = bindings.get(&entity.reference.kind).ok_or_else(|| {
                CorridorError::Missing(format!("identityBindings[{}]", entity.reference.kind))
            })?;
            let profiled_count = binding
                .fields
                .iter()
                .filter(|field| matches!(field.value, IdentityFieldValue::ProfiledKey { .. }))
                .count();
            let profiled_count =
                u32::try_from(profiled_count).expect("profiled field count must fit u32");
            let mut fields = Vec::with_capacity(binding.fields.len());
            for field in &binding.fields {
                let bytes = match field.value {
                    IdentityFieldValue::Namespace => namespace.to_vec(),
                    IdentityFieldValue::ProfiledKey { kind, local } => {
                        let expanded_local = entity
                            .reference
                            .local
                            .checked_mul(profiled_count)
                            .and_then(|base| base.checked_add(local))
                            .ok_or_else(|| CorridorError::Mismatch {
                                path: "expanded profiled key local index".to_owned(),
                                expected: "u32".to_owned(),
                                actual: "overflow".to_owned(),
                            })?;
                        format!("{kind:02x}/{unit:08x}/{expanded_local:08x}").into_bytes()
                    }
                    IdentityFieldValue::StableId { kind, .. } => {
                        let target = entity
                            .identity_references
                            .get(&field.tag)
                            .copied()
                            .ok_or_else(|| {
                                CorridorError::Missing(format!(
                                    "identity reference tag {} for {:?}",
                                    field.tag, entity.reference
                                ))
                            })?;
                        if target.kind != kind {
                            return Err(CorridorError::Mismatch {
                                path: format!(
                                    "identity reference kind for {:?} tag {}",
                                    entity.reference, field.tag
                                ),
                                expected: kind.to_string(),
                                actual: target.kind.to_string(),
                            });
                        }
                        stable_ids
                            .get(&UnitEntityRef {
                                unit,
                                entity: target,
                            })
                            .copied()
                            .ok_or_else(|| {
                                CorridorError::UnknownReference(format!(
                                    "StableId dependency unit={unit} target={target:?}"
                                ))
                            })?
                            .to_vec()
                    }
                };
                fields.push(IdentityField {
                    tag: field.tag,
                    bytes,
                });
            }
            let canonical = encode_canonical_identity(
                identity.identity_encoding_version(),
                entity.reference.kind,
                &fields,
            );
            if !canonical.starts_with(IDENTITY_MAGIC) {
                return Err(CorridorError::Mismatch {
                    path: "canonical identity magic".to_owned(),
                    expected: "LFID".to_owned(),
                    actual: lower_hex(&canonical[..4]),
                });
            }
            let digest = blake3::hash(&[STABLE_ID_DOMAIN, canonical.as_slice()].concat());
            let mut stable_id = [0_u8; 16];
            stable_id.copy_from_slice(&digest.as_bytes()[..16]);
            let owner = UnitEntityRef {
                unit,
                entity: entity.reference,
            };
            if stable_ids.insert(owner, stable_id).is_some() {
                return Err(CorridorError::DuplicateReference(format!(
                    "compiled declaration {owner:?}"
                )));
            }
            compiled.push(CompiledDeclaration {
                owner,
                stable_id,
                fields,
            });
        }
    }
    if entities.len() != template.entities.len() {
        return Err(CorridorError::DuplicateReference(
            "template entity tuple".to_owned(),
        ));
    }
    Ok(compiled)
}

pub(crate) fn compile_semantic_records(
    _identity: &IdentityContract,
    template: &CorridorTemplate,
    declarations: &[CompiledDeclaration],
    n: u32,
) -> Result<(Vec<SemanticRecord>, Vec<SemanticRecord>), CorridorError> {
    let stable_ids = declarations
        .iter()
        .map(|declaration| (declaration.owner, declaration.stable_id))
        .collect::<BTreeMap<_, _>>();
    let owner_ordinals = owner_ordinals(declarations);
    let mut pending = Vec::with_capacity(
        template
            .entities
            .len()
            .saturating_add(template.relations.len())
            .saturating_add(template.geometry.len())
            .saturating_mul(usize::try_from(n).expect("N must fit usize")),
    );
    for declaration in declarations {
        pending.push(PendingRecord {
            record_kind: 1,
            owner: declaration.owner,
            local_index: LocalIndexRule::Absent,
            payload: crate::identity::encode_identity_payload(&declaration.fields),
        });
    }
    for unit in 0..n {
        for relation in &template.relations {
            pending.push(compile_relation(unit, relation, &stable_ids)?);
        }
        for point in &template.geometry {
            let (x_bits, y_bits, z_bits) = geometry_coordinate_bits(point, unit)?;
            let mut payload = Vec::with_capacity(32);
            payload.extend_from_slice(&stable_id(&stable_ids, unit, point.frame)?);
            append_u32(&mut payload, point.point_index);
            append_u32(&mut payload, x_bits);
            append_u32(&mut payload, y_bits);
            append_u32(&mut payload, z_bits);
            pending.push(PendingRecord {
                record_kind: 5,
                owner: UnitEntityRef {
                    unit,
                    entity: point.edge,
                },
                local_index: LocalIndexRule::Explicit(point.point_index),
                payload,
            });
        }
    }
    assign_local_indexes(&mut pending);
    let mut records = Vec::with_capacity(pending.len());
    for pending_record in pending {
        let stable_id = stable_id(
            &stable_ids,
            pending_record.owner.unit,
            pending_record.owner.entity,
        )?;
        let owner_ordinal = owner_ordinals
            .get(&(pending_record.owner.entity.kind, stable_id))
            .copied()
            .ok_or_else(|| {
                CorridorError::UnknownReference(format!(
                    "owner ordinal for {:?}",
                    pending_record.owner
                ))
            })?;
        let local_index = match pending_record.local_index {
            LocalIndexRule::Absent => ABSENT_LOCAL_INDEX,
            LocalIndexRule::Explicit(value) => value,
            LocalIndexRule::OwnerPayloadOrder
            | LocalIndexRule::GateOrder(..)
            | LocalIndexRule::WaitingOrder(..) => {
                return Err(CorridorError::Missing(
                    "assigned semantic local index".to_owned(),
                ));
            }
        };
        records.push(SemanticRecord {
            record_kind: pending_record.record_kind,
            entity_kind_code: pending_record.owner.entity.kind,
            entity_kind: entity_kind_name(pending_record.owner.entity.kind)?.to_owned(),
            stable_id,
            owner_ordinal,
            local_index,
            payload: pending_record.payload,
        });
    }
    Ok((records.clone(), records))
}

fn geometry_coordinate_bits(
    point: &TemplateGeometry,
    unit: u32,
) -> Result<(u32, u32, u32), CorridorError> {
    match point.coordinate_rule {
        TemplateGeometryRule::Fixed => {
            return Ok((point.x_bits, point.y_bits, point.z_bits));
        }
        TemplateGeometryRule::JunctionGridV1 => {}
    }
    let unit_x = unit % 4_096;
    let unit_y = unit / 4_096;
    let x = unit_x
        .checked_mul(128)
        .and_then(|base| {
            point
                .edge
                .local
                .checked_mul(2)
                .and_then(|edge| base.checked_add(edge))
        })
        .and_then(|base| base.checked_add(point.point_index))
        .ok_or_else(|| contract_error("junction grid x coordinate overflow"))?;
    let y = unit_y
        .checked_mul(128)
        .ok_or_else(|| contract_error("junction grid y coordinate overflow"))?;
    Ok((
        (x as f32).to_bits(),
        (y as f32).to_bits(),
        0.0_f32.to_bits(),
    ))
}

fn compile_relation(
    unit: u32,
    relation: &TemplateRelation,
    stable_ids: &BTreeMap<UnitEntityRef, [u8; 16]>,
) -> Result<PendingRecord, CorridorError> {
    let (record_kind, owner, local_index, payload) = match relation {
        TemplateRelation::Owner { child, parent } => {
            let mut payload = Vec::with_capacity(18);
            append_u16(&mut payload, parent.kind);
            payload.extend_from_slice(&stable_id(stable_ids, unit, *parent)?);
            (2, *child, LocalIndexRule::Absent, payload)
        }
        TemplateRelation::EdgeConnection { source, target } => (
            3,
            *source,
            LocalIndexRule::OwnerPayloadOrder,
            stable_id(stable_ids, unit, *target)?.to_vec(),
        ),
        TemplateRelation::RouteOccurrence { route, index, edge } => {
            let mut payload = Vec::with_capacity(20);
            append_u32(&mut payload, *index);
            payload.extend_from_slice(&stable_id(stable_ids, unit, *edge)?);
            (4, *route, LocalIndexRule::Explicit(*index), payload)
        }
        TemplateRelation::Access {
            rule,
            participant,
            target,
            decision,
        } => {
            let mut payload = Vec::with_capacity(35);
            payload.extend_from_slice(&stable_id(stable_ids, unit, *participant)?);
            append_u16(&mut payload, target.kind);
            payload.extend_from_slice(&stable_id(stable_ids, unit, *target)?);
            payload.push(*decision);
            (6, *rule, LocalIndexRule::OwnerPayloadOrder, payload)
        }
        TemplateRelation::SignalGroup { group, gate } => (
            7,
            *group,
            LocalIndexRule::OwnerPayloadOrder,
            stable_id(stable_ids, unit, *gate)?.to_vec(),
        ),
        TemplateRelation::PhaseState {
            phase,
            group,
            state,
        } => {
            let mut payload = Vec::with_capacity(17);
            payload.extend_from_slice(&stable_id(stable_ids, unit, *group)?);
            payload.push(*state);
            (8, *phase, LocalIndexRule::OwnerPayloadOrder, payload)
        }
        TemplateRelation::Gate {
            path,
            transition_index,
            gate,
            stop_line,
            edge,
            edge_position_bits,
        } => {
            let gate_id = stable_id(stable_ids, unit, *gate)?;
            let mut payload = Vec::with_capacity(56);
            append_u32(&mut payload, 0);
            payload.extend_from_slice(&gate_id);
            payload.extend_from_slice(&stable_id(stable_ids, unit, *stop_line)?);
            payload.extend_from_slice(&stable_id(stable_ids, unit, *edge)?);
            append_u32(&mut payload, *edge_position_bits);
            (
                9,
                *path,
                LocalIndexRule::GateOrder(*transition_index, gate_id),
                payload,
            )
        }
        TemplateRelation::WaitingZone {
            path,
            entry_transition_index,
            release_transition_index,
            zone,
            before_gate,
            after_gate,
            capacity,
        } => {
            let zone_id = stable_id(stable_ids, unit, *zone)?;
            let mut payload = Vec::with_capacity(56);
            append_u32(&mut payload, 0);
            payload.extend_from_slice(&zone_id);
            payload.extend_from_slice(&stable_id(stable_ids, unit, *before_gate)?);
            payload.extend_from_slice(&stable_id(stable_ids, unit, *after_gate)?);
            append_u32(&mut payload, *capacity);
            (
                10,
                *path,
                LocalIndexRule::WaitingOrder(
                    *entry_transition_index,
                    *release_transition_index,
                    zone_id,
                ),
                payload,
            )
        }
        TemplateRelation::Parking {
            space,
            entry_edge,
            entry_high_bits,
            entry_residual_bits,
            exit_edge,
            exit_high_bits,
            exit_residual_bits,
        } => {
            let mut payload = Vec::with_capacity(64);
            payload.extend_from_slice(&stable_id(stable_ids, unit, *space)?);
            payload.extend_from_slice(&stable_id(stable_ids, unit, *entry_edge)?);
            append_u32(&mut payload, *entry_high_bits);
            append_u32(&mut payload, *entry_residual_bits);
            payload.extend_from_slice(&stable_id(stable_ids, unit, *exit_edge)?);
            append_u32(&mut payload, *exit_high_bits);
            append_u32(&mut payload, *exit_residual_bits);
            (11, *space, LocalIndexRule::Absent, payload)
        }
        TemplateRelation::LaneCoverage { lane, index, edge } => {
            let mut payload = Vec::with_capacity(20);
            append_u32(&mut payload, *index);
            payload.extend_from_slice(&stable_id(stable_ids, unit, *edge)?);
            (12, *lane, LocalIndexRule::Explicit(*index), payload)
        }
        TemplateRelation::JunctionInternalEdge { junction, edge } => (
            13,
            *junction,
            LocalIndexRule::OwnerPayloadOrder,
            stable_id(stable_ids, unit, *edge)?.to_vec(),
        ),
    };
    Ok(PendingRecord {
        record_kind,
        owner: UnitEntityRef {
            unit,
            entity: owner,
        },
        local_index,
        payload,
    })
}

fn assign_local_indexes(records: &mut [PendingRecord]) {
    let mut payload_groups = BTreeMap::<(u16, UnitEntityRef), Vec<usize>>::new();
    let mut gate_groups = BTreeMap::<UnitEntityRef, Vec<usize>>::new();
    let mut waiting_groups = BTreeMap::<UnitEntityRef, Vec<usize>>::new();
    for (index, record) in records.iter().enumerate() {
        match record.local_index {
            LocalIndexRule::OwnerPayloadOrder => {
                payload_groups
                    .entry((record.record_kind, record.owner))
                    .or_default()
                    .push(index);
            }
            LocalIndexRule::GateOrder(..) => {
                gate_groups.entry(record.owner).or_default().push(index)
            }
            LocalIndexRule::WaitingOrder(..) => {
                waiting_groups.entry(record.owner).or_default().push(index);
            }
            LocalIndexRule::Absent | LocalIndexRule::Explicit(_) => {}
        }
    }
    for indexes in payload_groups.values_mut() {
        indexes.sort_by(|left, right| records[*left].payload.cmp(&records[*right].payload));
        for (ordinal, index) in indexes.iter().copied().enumerate() {
            records[index].local_index =
                LocalIndexRule::Explicit(u32::try_from(ordinal).expect("local index must fit u32"));
        }
    }
    for indexes in gate_groups.values_mut() {
        indexes.sort_by_key(|index| match records[*index].local_index {
            LocalIndexRule::GateOrder(transition, stable_id) => (transition, stable_id),
            _ => unreachable!("gate group contains only gate order rules"),
        });
        for (ordinal, index) in indexes.iter().copied().enumerate() {
            let ordinal = u32::try_from(ordinal).expect("gate occurrence index must fit u32");
            records[index].payload[..4].copy_from_slice(&ordinal.to_le_bytes());
            records[index].local_index = LocalIndexRule::Explicit(ordinal);
        }
    }
    for indexes in waiting_groups.values_mut() {
        indexes.sort_by_key(|index| match records[*index].local_index {
            LocalIndexRule::WaitingOrder(entry, release, stable_id) => (entry, release, stable_id),
            _ => unreachable!("waiting group contains only waiting order rules"),
        });
        for (ordinal, index) in indexes.iter().copied().enumerate() {
            let ordinal = u32::try_from(ordinal).expect("waiting occurrence index must fit u32");
            records[index].payload[..4].copy_from_slice(&ordinal.to_le_bytes());
            records[index].local_index = LocalIndexRule::Explicit(ordinal);
        }
    }
}

fn owner_ordinals(declarations: &[CompiledDeclaration]) -> BTreeMap<(u16, [u8; 16]), u32> {
    let mut by_kind = BTreeMap::<u16, Vec<[u8; 16]>>::new();
    for declaration in declarations {
        by_kind
            .entry(declaration.owner.entity.kind)
            .or_default()
            .push(declaration.stable_id);
    }
    let mut ordinals = BTreeMap::new();
    for (kind, stable_ids) in &mut by_kind {
        stable_ids.sort_unstable();
        for (ordinal, stable_id) in stable_ids.iter().enumerate() {
            ordinals.insert(
                (*kind, *stable_id),
                u32::try_from(ordinal).expect("owner ordinal must fit u32"),
            );
        }
    }
    ordinals
}

fn stable_id(
    stable_ids: &BTreeMap<UnitEntityRef, [u8; 16]>,
    unit: u32,
    entity: EntityRef,
) -> Result<[u8; 16], CorridorError> {
    stable_ids
        .get(&UnitEntityRef { unit, entity })
        .copied()
        .ok_or_else(|| {
            CorridorError::UnknownReference(format!("compiled StableId unit={unit} {entity:?}"))
        })
}

fn count_record_kinds(records: &[SemanticRecord]) -> BTreeMap<String, u64> {
    let mut counts = BTreeMap::new();
    for record in records {
        let name = match record.record_kind {
            1 => "identity-declaration",
            2 => "owner-relation",
            3 => "edge-connection",
            4 => "route-occurrence",
            5 => "canonical-geometry-point",
            6 => "access-relation",
            7 => "signal-group-relation",
            8 => "signal-phase-state",
            9 => "gate-occurrence",
            10 => "waiting-zone-occurrence",
            11 => "parking-space-anchors",
            12 => "lane-coverage-occurrence",
            13 => "junction-internal-edge-role",
            _ => "unknown",
        };
        *counts.entry(name.to_owned()).or_default() += 1;
    }
    counts
}

fn build_aggregate_counts(
    stage: &StageContract,
    template: &CorridorTemplate,
    graph: &crate::ExpandedModuleGraph,
    records: &[SemanticRecord],
    semantic_record_stream: &[u8],
) -> Result<IdentityAggregateCounts, CorridorError> {
    let per_unit = template.stage_input_counts();
    let n = u64::from(graph.n);
    let module_count = usize_u64(graph.modules.len(), "moduleCount")?;
    let import_edge_count = graph.modules.iter().try_fold(0_u64, |total, module| {
        add_u64(
            total,
            usize_u64(module.imports.len(), "importEdgeCount"),
            "importEdgeCount",
        )
    })?;
    let cross_module_reference_count = graph.modules.iter().try_fold(0_u64, |total, module| {
        add_u64(
            total,
            usize_u64(
                module.cross_module_references.len(),
                "crossModuleReferenceCount",
            ),
            "crossModuleReferenceCount",
        )
    })?;
    let identity_declaration_count = mul_u64(
        required_count(&per_unit, "sourceDeclarationCount")?,
        n,
        "identityDeclarationCount",
    )?;
    let shared_declaration_count = u64::from(graph.graph_profile == GraphProfileId::SharedFaninDag);
    let source_declaration_count = add_u64(
        identity_declaration_count,
        Ok(shared_declaration_count),
        "sourceDeclarationCount",
    )?;
    let identity_field_occurrence_count = mul_u64(
        required_count(&per_unit, "identityFieldOccurrenceCount")?,
        n,
        "identityFieldOccurrenceCount",
    )?;
    let profiled_key_occurrence_count = mul_u64(
        required_count(&per_unit, "profiledKeyOccurrenceCount")?,
        n,
        "profiledKeyOccurrenceCount",
    )?;
    let source_reference_count = add_u64(
        mul_u64(
            required_count(&per_unit, "sourceReferenceCount")?,
            n,
            "sourceReferenceCount",
        )?,
        Ok(cross_module_reference_count),
        "sourceReferenceCount",
    )?;
    let source_relation_count = mul_u64(
        required_count(&per_unit, "sourceRelationCount")?,
        n,
        "sourceRelationCount",
    )?;
    let source_geometry_count = mul_u64(
        required_count(&per_unit, "sourceGeometryCount")?,
        n,
        "sourceGeometryCount",
    )?;
    let source_span_count = sum_u64(
        &[
            source_declaration_count,
            source_reference_count,
            source_relation_count,
            source_geometry_count,
        ],
        "sourceSpanCount",
    )?;
    let source_byte_count = sum_u64(
        &[
            mul_u64(
                usize_u64(
                    stage.declaration_token_bytes_with_lf,
                    "declaration token bytes",
                )?,
                source_declaration_count,
                "sourceByteCount",
            )?,
            mul_u64(
                usize_u64(stage.reference_token_bytes_with_lf, "reference token bytes")?,
                source_reference_count,
                "sourceByteCount",
            )?,
            mul_u64(
                usize_u64(stage.relation_token_bytes_with_lf, "relation token bytes")?,
                source_relation_count,
                "sourceByteCount",
            )?,
            mul_u64(
                usize_u64(stage.geometry_token_bytes_with_lf, "geometry token bytes")?,
                source_geometry_count,
                "sourceByteCount",
            )?,
        ],
        "sourceByteCount",
    )?;

    let string_stats = corridor_string_stats(
        stage,
        template,
        graph,
        identity_declaration_count,
        profiled_key_occurrence_count,
        source_reference_count,
    )?;
    let semantic_output_record = usize_u64(records.len(), "semanticOutputRecord")?;
    let semantic_payload_byte_count = records.iter().try_fold(0_u64, |total, record| {
        add_u64(
            total,
            usize_u64(record.payload.len(), "semanticPayloadByteCount"),
            "semanticPayloadByteCount",
        )
    })?;
    let logical_byte_count = add_u64(
        mul_u64(44, semantic_output_record, "logicalByteCount")?,
        Ok(semantic_payload_byte_count),
        "logicalByteCount",
    )?;
    let output_byte_count = usize_u64(semantic_record_stream.len(), "outputByteCount")?;

    Ok(IdentityAggregateCounts {
        module_count,
        import_edge_count,
        cross_module_reference_count,
        maximum_import_depth: match graph.graph_profile {
            GraphProfileId::WideStar => 1,
            GraphProfileId::DeepChain => u64::from(graph.n),
            GraphProfileId::SharedFaninDag => 3,
        },
        source_document_count: module_count,
        source_byte_count,
        identity_declaration_count,
        source_declaration_count,
        source_span_count,
        identity_field_occurrence_count,
        profiled_key_occurrence_count,
        source_reference_count,
        source_relation_count,
        source_geometry_count,
        symbol_count: source_declaration_count,
        string_item_count: string_stats.item_count,
        maximum_string_bytes: string_stats.maximum_bytes,
        total_string_bytes: string_stats.total_bytes,
        diagnostic_count: 0,
        semantic_output_record,
        semantic_payload_byte_count,
        logical_byte_count,
        output_byte_count,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StringStats {
    item_count: u64,
    maximum_bytes: u64,
    total_bytes: u64,
}

struct CorridorSourceMaterialization {
    spans: Vec<SourceSpanRecord>,
    records: Vec<TypedAstStageRecord>,
    payload: Vec<u8>,
}

fn corridor_string_stats(
    stage: &StageContract,
    _template: &CorridorTemplate,
    graph: &crate::ExpandedModuleGraph,
    identity_declaration_count: u64,
    profiled_key_occurrence_count: u64,
    source_reference_count: u64,
) -> Result<StringStats, CorridorError> {
    let mut item_count = 0_u64;
    let mut total_bytes = 0_u64;
    let mut maximum_bytes = 0_u64;
    let mut include = |bytes: u64| -> Result<(), CorridorError> {
        item_count = item_count
            .checked_add(1)
            .ok_or_else(|| CorridorError::Contract("stringItemCount overflow".to_owned()))?;
        total_bytes = total_bytes
            .checked_add(bytes)
            .ok_or_else(|| CorridorError::Contract("totalStringBytes overflow".to_owned()))?;
        maximum_bytes = maximum_bytes.max(bytes);
        Ok(())
    };
    for module in &graph.modules {
        include(usize_u64(module.canonical_name.len(), "module name")?)?;
        include(usize_u64(
            format!(
                "source/{}/{}.lfsynthetic",
                graph.graph_profile.as_str(),
                module.canonical_name
            )
            .len(),
            "source document key",
        )?)?;
        for import in &module.imports {
            include(usize_u64(import.len(), "import target name")?)?;
        }
    }
    for _ in 0..identity_declaration_count {
        include(32)?;
    }
    for _ in 0..profiled_key_occurrence_count {
        include(20)?;
    }
    for _ in 0..source_reference_count {
        include(30)?;
    }
    if graph.graph_profile == GraphProfileId::SharedFaninDag {
        include(usize_u64(
            stage.shared_constant_name.len(),
            "shared constant name",
        )?)?;
        include(usize_u64(
            stage.shared_constant_value.len(),
            "shared constant value",
        )?)?;
    }
    Ok(StringStats {
        item_count,
        maximum_bytes,
        total_bytes,
    })
}

fn build_stage_breakdown(
    counts: &IdentityAggregateCounts,
) -> Result<StageBreakdown, CorridorError> {
    let source_input_records = sum_u64(
        &[
            counts.module_count,
            counts.import_edge_count,
            counts.source_span_count,
        ],
        "sourceInput.recordCount",
    )?;
    let source_input_payload = add_u64(
        counts.source_byte_count,
        Ok(counts.total_string_bytes),
        "sourceInput.payload",
    )?;
    let typed_records = sum_u64(
        &[
            counts.module_count,
            counts.import_edge_count,
            counts.source_declaration_count,
            counts.identity_field_occurrence_count,
            counts.source_reference_count,
            counts.source_relation_count,
            counts.source_geometry_count,
        ],
        "typedAst.recordCount",
    )?;
    let typed_payload = sum_u64(
        &[
            counts.source_byte_count,
            counts.total_string_bytes,
            mul_u64(20, counts.source_span_count, "typedAst.sourceSpans")?,
        ],
        "typedAst.payload",
    )?;
    let hir_records = sum_u64(
        &[
            counts.module_count,
            counts.import_edge_count,
            counts.symbol_count,
            counts.identity_field_occurrence_count,
            counts.source_reference_count,
            counts.source_relation_count,
            counts.source_geometry_count,
        ],
        "hir.recordCount",
    )?;
    let hir_operand_count = sum_u64(
        &[
            counts.identity_field_occurrence_count,
            counts.import_edge_count,
            counts.source_reference_count,
            mul_u64(2, counts.source_relation_count, "hir.relations")?,
            mul_u64(3, counts.source_geometry_count, "hir.geometry")?,
        ],
        "hir.operandCount",
    )?;
    let hir_payload = add_u64(
        counts.total_string_bytes,
        Ok(mul_u64(4, hir_operand_count, "hir.payload")?),
        "hir.payload",
    )?;
    let source_input = corridor_stage_shape(source_input_records, source_input_payload, 32, 32)?;
    let typed_ast = corridor_stage_shape(typed_records, typed_payload, 32, 32)?;
    let hir = corridor_stage_shape(hir_records, hir_payload, 32, 32)?;
    let mir = corridor_stage_shape(
        counts.semantic_output_record,
        counts.semantic_payload_byte_count,
        44,
        48,
    )?;
    let diagnostics = StageShape {
        record_count: 0,
        payload_logical_bytes: 0,
        logical_bytes: 0,
        record_allocation_bytes: 0,
    };
    let scratch_bytes = mul_u64(
        8,
        counts
            .module_count
            .max(counts.symbol_count)
            .max(counts.semantic_output_record),
        "scratch.logicalBytes",
    )?;
    let scratch = StageShape {
        record_count: 0,
        payload_logical_bytes: scratch_bytes,
        logical_bytes: scratch_bytes,
        record_allocation_bytes: scratch_bytes,
    };
    let output_construction = StageShape {
        record_count: counts.semantic_output_record,
        payload_logical_bytes: counts.semantic_payload_byte_count,
        logical_bytes: counts.output_byte_count,
        record_allocation_bytes: counts.output_byte_count,
    };
    Ok(StageBreakdown {
        source_input,
        typed_ast,
        hir,
        mir,
        canonical_lir: mir,
        diagnostics,
        scratch,
        output_construction,
    })
}

fn corridor_stage_shape(
    record_count: u64,
    payload_logical_bytes: u64,
    logical_record_bytes: u64,
    allocated_record_bytes: u64,
) -> Result<StageShape, CorridorError> {
    Ok(StageShape {
        record_count,
        payload_logical_bytes,
        logical_bytes: add_u64(
            mul_u64(logical_record_bytes, record_count, "stage.logicalBytes")?,
            Ok(payload_logical_bytes),
            "stage.logicalBytes",
        )?,
        record_allocation_bytes: add_u64(
            mul_u64(
                allocated_record_bytes,
                record_count,
                "stage.recordAllocationBytes",
            )?,
            Ok(payload_logical_bytes),
            "stage.recordAllocationBytes",
        )?,
    })
}

#[allow(clippy::too_many_arguments)]
fn materialize_corridor_stages(
    generator: &GeneratorContract,
    _identity: &IdentityContract,
    stage: &StageContract,
    template: &CorridorTemplate,
    graph: &crate::ExpandedModuleGraph,
    declarations: &[CompiledDeclaration],
    unsorted_records: &[SemanticRecord],
    records: &[SemanticRecord],
    semantic_record_stream: &[u8],
    counts: &IdentityAggregateCounts,
    stages: &StageBreakdown,
) -> Result<CorridorMaterialization, CorridorError> {
    let CorridorSourceMaterialization {
        spans: source_spans,
        records: mut source_records,
        payload: source_payload,
    } = materialize_corridor_source(generator, stage, template, graph, counts)?;
    let mut typed_records = Vec::with_capacity(to_usize(
        stages.typed_ast.record_count,
        "typed AST record capacity",
    )?);
    typed_records.extend(
        source_records
            .iter()
            .filter(|record| {
                record.record_kind == stage.record_kind_module
                    || record.record_kind == stage.record_kind_import
            })
            .copied(),
    );
    append_typed_entity_records(
        &mut typed_records,
        stage,
        template,
        graph,
        declarations,
        counts,
    )?;
    let mut typed_payload = source_payload.clone();
    encode_source_spans(&mut typed_payload, &source_spans);

    let hir_count = to_usize(stages.hir.record_count, "HIR record capacity")?;
    let mut hir_records = Vec::with_capacity(hir_count);
    for (index, typed) in typed_records.iter().enumerate() {
        hir_records.push(HirStageRecord {
            record_kind: if typed.record_kind == stage.record_kind_declaration {
                stage.record_kind_symbol
            } else {
                typed.record_kind
            },
            entity_kind: typed.entity_kind,
            module_ordinal: typed.module_ordinal,
            symbol_ordinal: typed.owner_local_index,
            resolved_target_ordinal: stage.absent_ordinal,
            payload_offset: 0,
            payload_length: 0,
        });
        if index + 1 == hir_count {
            break;
        }
    }
    while hir_records.len() < hir_count {
        hir_records.push(HirStageRecord::default());
    }
    let string_start = to_usize(counts.source_byte_count, "source byte count")?;
    let mut hir_payload = source_payload[string_start..].to_vec();
    hir_payload.resize(
        to_usize(stages.hir.payload_logical_bytes, "HIR payload")?,
        0,
    );

    let (mir_records, mir_payload) = materialize_semantic_records(unsorted_records)?;
    let (lir_records, lir_payload) = materialize_semantic_records(records)?;
    let scratch = vec![
        0_u64;
        to_usize(
            stages.scratch.payload_logical_bytes / 8,
            "scratch word count"
        )?
    ];
    source_records.shrink_to_fit();
    Ok(CorridorMaterialization {
        source_spans,
        source_records,
        source_payload,
        typed_records,
        typed_payload,
        hir_records,
        hir_payload,
        mir_records,
        mir_payload,
        lir_records,
        lir_payload,
        diagnostics: Vec::new(),
        scratch,
        output: semantic_record_stream.to_vec(),
    })
}

fn materialize_corridor_source(
    generator: &GeneratorContract,
    stage: &StageContract,
    template: &CorridorTemplate,
    graph: &crate::ExpandedModuleGraph,
    counts: &IdentityAggregateCounts,
) -> Result<CorridorSourceMaterialization, CorridorError> {
    let mut payload = Vec::with_capacity(to_usize(
        counts.source_byte_count + counts.total_string_bytes,
        "source payload",
    )?);
    let mut spans = Vec::with_capacity(to_usize(counts.source_span_count, "source spans")?);
    let mut span_records = Vec::with_capacity(to_usize(counts.source_span_count, "span records")?);
    for (module_ordinal, module) in graph.modules.iter().enumerate() {
        let module_ordinal =
            u32::try_from(module_ordinal).map_err(|_| contract_error("module ordinal overflow"))?;
        let seed = u64::from_str_radix(&module.module_seed_ordinal_hex_u64, 16)
            .map_err(|_| contract_error("invalid module seed ordinal"))?;
        let categories = module_source_categories(template, module)?;
        let mut line = 1_u32;
        for (record_kind, token, count, sequence_kind) in [
            (
                stage.record_kind_declaration,
                stage.declaration_token.as_str(),
                categories.declarations,
                SequenceKind::Declarations,
            ),
            (
                stage.record_kind_reference,
                stage.reference_token.as_str(),
                categories.references,
                SequenceKind::References,
            ),
            (
                stage.record_kind_relation,
                stage.relation_token.as_str(),
                categories.relations,
                SequenceKind::Relations,
            ),
            (
                stage.record_kind_geometry,
                stage.geometry_token.as_str(),
                categories.geometry,
                SequenceKind::Geometry,
            ),
        ] {
            let mut ordinals = (0..count).collect::<Vec<_>>();
            permute_in_place(&mut ordinals, generator, sequence_kind, seed);
            for local in ordinals {
                let offset = payload.len();
                payload.extend_from_slice(token.as_bytes());
                payload.push(b'/');
                payload.extend_from_slice(format!("{local:08x}").as_bytes());
                payload.push(b'\n');
                let length = payload.len() - offset;
                spans.push(SourceSpanRecord {
                    source_document_ordinal: module_ordinal,
                    start_line: line,
                    start_column: 1,
                    end_line: line,
                    end_column: u32::try_from(length)
                        .map_err(|_| contract_error("source token length overflow"))?,
                });
                span_records.push(TypedAstStageRecord {
                    record_kind,
                    entity_kind: ENTITY_KIND_ABSENT,
                    module_ordinal,
                    source_span_ordinal: u32::try_from(spans.len() - 1)
                        .map_err(|_| contract_error("source span ordinal overflow"))?,
                    owner_local_index: local,
                    payload_offset: usize_u64(offset, "source token offset")?,
                    payload_length: usize_u64(length, "source token length")?,
                });
                line = line
                    .checked_add(1)
                    .ok_or_else(|| contract_error("source line overflow"))?;
            }
        }
    }
    if usize_u64(payload.len(), "materialized source bytes")? != counts.source_byte_count {
        return Err(CorridorError::Mismatch {
            path: "materialized source bytes".to_owned(),
            expected: counts.source_byte_count.to_string(),
            actual: payload.len().to_string(),
        });
    }
    let string_base = payload.len();
    let mut records = Vec::with_capacity(to_usize(
        counts.module_count + counts.import_edge_count + counts.source_span_count,
        "source record capacity",
    )?);
    for (module_ordinal, module) in graph.modules.iter().enumerate() {
        let module_ordinal =
            u32::try_from(module_ordinal).map_err(|_| contract_error("module ordinal overflow"))?;
        let offset = payload.len();
        payload.extend_from_slice(module.canonical_name.as_bytes());
        records.push(TypedAstStageRecord {
            record_kind: stage.record_kind_module,
            entity_kind: ENTITY_KIND_ABSENT,
            module_ordinal,
            source_span_ordinal: stage.absent_ordinal,
            owner_local_index: stage.absent_ordinal,
            payload_offset: usize_u64(offset, "module string offset")?,
            payload_length: usize_u64(module.canonical_name.len(), "module string length")?,
        });
    }
    for module in &graph.modules {
        payload.extend_from_slice(
            format!(
                "source/{}/{}.lfsynthetic",
                graph.graph_profile.as_str(),
                module.canonical_name
            )
            .as_bytes(),
        );
    }
    for (module_ordinal, module) in graph.modules.iter().enumerate() {
        let module_ordinal =
            u32::try_from(module_ordinal).map_err(|_| contract_error("module ordinal overflow"))?;
        for (local_index, import) in module.imports.iter().enumerate() {
            let offset = payload.len();
            payload.extend_from_slice(import.as_bytes());
            records.push(TypedAstStageRecord {
                record_kind: stage.record_kind_import,
                entity_kind: ENTITY_KIND_ABSENT,
                module_ordinal,
                source_span_ordinal: stage.absent_ordinal,
                owner_local_index: u32::try_from(local_index)
                    .map_err(|_| contract_error("import local index overflow"))?,
                payload_offset: usize_u64(offset, "import string offset")?,
                payload_length: usize_u64(import.len(), "import string length")?,
            });
        }
    }
    append_corridor_identity_strings(&mut payload, template, graph)?;
    append_corridor_reference_strings(&mut payload, template, graph)?;
    if graph.graph_profile == GraphProfileId::SharedFaninDag {
        payload.extend_from_slice(stage.shared_constant_name.as_bytes());
        payload.extend_from_slice(stage.shared_constant_value.as_bytes());
    }
    if usize_u64(payload.len() - string_base, "materialized string bytes")?
        != counts.total_string_bytes
    {
        return Err(CorridorError::Mismatch {
            path: "materialized string bytes".to_owned(),
            expected: counts.total_string_bytes.to_string(),
            actual: (payload.len() - string_base).to_string(),
        });
    }
    records.extend(span_records);
    Ok(CorridorSourceMaterialization {
        spans,
        records,
        payload,
    })
}

#[derive(Clone, Copy)]
struct SourceCategoryCounts {
    declarations: u32,
    references: u32,
    relations: u32,
    geometry: u32,
}

fn module_source_categories(
    template: &CorridorTemplate,
    module: &crate::ExpandedModule,
) -> Result<SourceCategoryCounts, CorridorError> {
    let is_unit = module.canonical_name.starts_with("unit/");
    let unit = u32::from(is_unit);
    let stage = template.stage_input_counts();
    let scaled = |field: &str| -> Result<u32, CorridorError> {
        u32::try_from(required_count(&stage, field)? * u64::from(unit))
            .map_err(|_| contract_error("module source category overflow"))
    };
    Ok(SourceCategoryCounts {
        declarations: scaled("sourceDeclarationCount")?
            + u32::from(module.canonical_name == "shared/common"),
        references: scaled("sourceReferenceCount")?
            + u32::try_from(module.cross_module_references.len())
                .map_err(|_| contract_error("cross-module reference count overflow"))?,
        relations: scaled("sourceRelationCount")?,
        geometry: scaled("sourceGeometryCount")?,
    })
}

fn append_corridor_identity_strings(
    payload: &mut Vec<u8>,
    template: &CorridorTemplate,
    graph: &crate::ExpandedModuleGraph,
) -> Result<(), CorridorError> {
    for unit in 0..graph.n {
        let module = graph
            .modules
            .iter()
            .find(|module| module.canonical_name == format!("unit/{unit:08x}"))
            .ok_or_else(|| CorridorError::Missing(format!("unit/{unit:08x}")))?;
        for entity in &template.entities {
            payload.extend_from_slice(module.namespace_id.as_bytes());
            let profiled_count = u32::try_from(profiled_key_field_count(entity.reference.kind))
                .map_err(|_| contract_error("profiled field count overflow"))?;
            for local in 0..profiled_count {
                let expanded = entity
                    .reference
                    .local
                    .checked_mul(profiled_count)
                    .and_then(|base| base.checked_add(local))
                    .ok_or_else(|| contract_error("profiled key local index overflow"))?;
                payload.extend_from_slice(
                    format!("{:02x}/{unit:08x}/{expanded:08x}", entity.reference.kind).as_bytes(),
                );
            }
        }
    }
    Ok(())
}

fn append_corridor_reference_strings(
    payload: &mut Vec<u8>,
    template: &CorridorTemplate,
    graph: &crate::ExpandedModuleGraph,
) -> Result<(), CorridorError> {
    let module_ordinals = graph
        .modules
        .iter()
        .enumerate()
        .map(|(ordinal, module)| {
            Ok((
                module.canonical_name.as_str(),
                u32::try_from(ordinal).map_err(|_| contract_error("module ordinal exceeds u32"))?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, CorridorError>>()?;
    for unit in 0..graph.n {
        let module_name = format!("unit/{unit:08x}");
        let module_ordinal = *module_ordinals
            .get(module_name.as_str())
            .ok_or(CorridorError::Missing(module_name))?;
        for entity in &template.entities {
            for target in entity.identity_references.values() {
                append_reference_spelling(payload, *target, module_ordinal);
            }
        }
        let mut relation_references = Vec::new();
        for relation in &template.relations {
            relation.append_stable_references(&mut relation_references);
        }
        for target in relation_references {
            append_reference_spelling(payload, target, module_ordinal);
        }
        for point in &template.geometry {
            append_reference_spelling(payload, point.frame, module_ordinal);
        }
    }
    for module in &graph.modules {
        for reference in &module.cross_module_references {
            let (kind, target_module) = if reference == "shared/common::shared-calibration-anchor" {
                (SHARED_CONSTANT_ENTITY_KIND, "shared/common")
            } else {
                let target = reference
                    .strip_prefix("canonical-first-declaration(")
                    .and_then(|value| value.strip_suffix(')'))
                    .ok_or_else(|| CorridorError::Mismatch {
                        path: "expanded cross-module reference".to_owned(),
                        expected: "canonical-first-declaration(module) or shared anchor".to_owned(),
                        actual: reference.clone(),
                    })?;
                (1, target)
            };
            let target_module_ordinal = *module_ordinals
                .get(target_module)
                .ok_or_else(|| CorridorError::UnknownReference(target_module.to_owned()))?;
            append_reference_spelling(payload, EntityRef { kind, local: 0 }, target_module_ordinal);
        }
    }
    Ok(())
}

fn append_reference_spelling(payload: &mut Vec<u8>, target: EntityRef, module_ordinal: u32) {
    payload.extend_from_slice(
        format!(
            "reference/{:02x}/{module_ordinal:08x}/{:08x}",
            target.kind, target.local
        )
        .as_bytes(),
    );
}

fn append_typed_entity_records(
    records: &mut Vec<TypedAstStageRecord>,
    stage: &StageContract,
    template: &CorridorTemplate,
    graph: &crate::ExpandedModuleGraph,
    declarations: &[CompiledDeclaration],
    counts: &IdentityAggregateCounts,
) -> Result<(), CorridorError> {
    for declaration in declarations {
        let module_ordinal = graph
            .modules
            .iter()
            .position(|module| {
                module.canonical_name == format!("unit/{:08x}", declaration.owner.unit)
            })
            .ok_or_else(|| CorridorError::Missing("unit module".to_owned()))?;
        let module_ordinal =
            u32::try_from(module_ordinal).map_err(|_| contract_error("module ordinal overflow"))?;
        records.push(TypedAstStageRecord {
            record_kind: stage.record_kind_declaration,
            entity_kind: declaration.owner.entity.kind,
            module_ordinal,
            source_span_ordinal: stage.absent_ordinal,
            owner_local_index: declaration.owner.entity.local,
            payload_offset: 0,
            payload_length: 0,
        });
        for field in &declaration.fields {
            records.push(TypedAstStageRecord {
                record_kind: stage.record_kind_identity_field,
                entity_kind: declaration.owner.entity.kind,
                module_ordinal,
                source_span_ordinal: stage.absent_ordinal,
                owner_local_index: u32::from(field.tag),
                payload_offset: 0,
                payload_length: usize_u64(field.bytes.len(), "identity field bytes")?,
            });
        }
    }
    if graph.graph_profile == GraphProfileId::SharedFaninDag {
        records.push(TypedAstStageRecord {
            record_kind: stage.record_kind_declaration,
            entity_kind: SHARED_CONSTANT_ENTITY_KIND,
            module_ordinal: graph
                .modules
                .iter()
                .position(|module| module.canonical_name == "shared/common")
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| CorridorError::Missing("shared/common".to_owned()))?,
            source_span_ordinal: stage.absent_ordinal,
            owner_local_index: 0,
            payload_offset: 0,
            payload_length: 0,
        });
    }
    let remaining = sum_u64(
        &[
            counts.source_reference_count,
            counts.source_relation_count,
            counts.source_geometry_count,
        ],
        "typed remaining records",
    )?;
    for (record_kind, count) in [
        (stage.record_kind_reference, counts.source_reference_count),
        (stage.record_kind_relation, counts.source_relation_count),
        (stage.record_kind_geometry, counts.source_geometry_count),
    ] {
        for local in 0..count {
            records.push(TypedAstStageRecord {
                record_kind,
                entity_kind: ENTITY_KIND_ABSENT,
                module_ordinal: 0,
                source_span_ordinal: stage.absent_ordinal,
                owner_local_index: u32::try_from(local).unwrap_or(u32::MAX),
                payload_offset: 0,
                payload_length: 0,
            });
        }
    }
    debug_assert_eq!(
        remaining,
        counts.source_reference_count + counts.source_relation_count + counts.source_geometry_count
    );
    let expected = counts.module_count
        + counts.import_edge_count
        + counts.source_declaration_count
        + counts.identity_field_occurrence_count
        + remaining;
    if usize_u64(records.len(), "typed records")? != expected {
        return Err(CorridorError::Mismatch {
            path: "typed AST record count".to_owned(),
            expected: expected.to_string(),
            actual: records.len().to_string(),
        });
    }
    let _ = template;
    Ok(())
}

fn materialize_semantic_records(
    records: &[SemanticRecord],
) -> Result<(Vec<MirLirStageRecord>, Vec<u8>), CorridorError> {
    let mut stage_records = Vec::with_capacity(records.len());
    let mut payload = Vec::new();
    for record in records {
        let offset = payload.len();
        payload.extend_from_slice(&record.payload);
        stage_records.push(MirLirStageRecord {
            record_kind: record.record_kind,
            entity_kind: record.entity_kind_code,
            stable_id: record.stable_id,
            owner_ordinal: record.owner_ordinal,
            local_index: record.local_index,
            payload_offset: usize_u64(offset, "semantic payload offset")?,
            payload_length: usize_u64(record.payload.len(), "semantic payload length")?,
        });
    }
    Ok((stage_records, payload))
}

fn verify_materialization(
    materialization: &CorridorMaterialization,
    counts: &IdentityAggregateCounts,
    stages: &StageBreakdown,
) -> Result<(), CorridorError> {
    let checks = [
        (
            "sourceInput",
            usize_u64(materialization.source_records.len(), "source records")?,
            usize_u64(materialization.source_payload.len(), "source payload")?,
            stages.source_input,
        ),
        (
            "typedAst",
            usize_u64(materialization.typed_records.len(), "typed records")?,
            usize_u64(materialization.typed_payload.len(), "typed payload")?,
            stages.typed_ast,
        ),
        (
            "hir",
            usize_u64(materialization.hir_records.len(), "HIR records")?,
            usize_u64(materialization.hir_payload.len(), "HIR payload")?,
            stages.hir,
        ),
        (
            "mir",
            usize_u64(materialization.mir_records.len(), "MIR records")?,
            usize_u64(materialization.mir_payload.len(), "MIR payload")?,
            stages.mir,
        ),
        (
            "canonicalLir",
            usize_u64(materialization.lir_records.len(), "LIR records")?,
            usize_u64(materialization.lir_payload.len(), "LIR payload")?,
            stages.canonical_lir,
        ),
    ];
    for (name, actual_records, actual_payload, expected) in checks {
        if actual_records != expected.record_count
            || actual_payload != expected.payload_logical_bytes
        {
            return Err(CorridorError::Mismatch {
                path: format!("{name} materialization"),
                expected: format!(
                    "{} records / {} payload bytes",
                    expected.record_count, expected.payload_logical_bytes
                ),
                actual: format!("{actual_records} records / {actual_payload} payload bytes"),
            });
        }
    }
    if usize_u64(materialization.source_spans.len(), "source spans")? != counts.source_span_count
        || !materialization.diagnostics.is_empty()
        || usize_u64(materialization.scratch.len(), "scratch words")? * 8
            != stages.scratch.logical_bytes
        || usize_u64(materialization.output.len(), "output bytes")?
            != stages.output_construction.logical_bytes
    {
        return Err(CorridorError::Mismatch {
            path: "non-primary stage materialization".to_owned(),
            expected: "frozen stage shapes".to_owned(),
            actual: "materialized stage shapes differ".to_owned(),
        });
    }
    Ok(())
}

fn encode_source_spans(payload: &mut Vec<u8>, spans: &[SourceSpanRecord]) {
    for span in spans {
        append_u32(payload, span.source_document_ordinal);
        append_u32(payload, span.start_line);
        append_u32(payload, span.start_column);
        append_u32(payload, span.end_line);
        append_u32(payload, span.end_column);
    }
}

fn required_array<'a>(value: &'a Value, field: &str) -> Result<&'a [Value], CorridorError> {
    value
        .get(field)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| CorridorError::Missing(field.to_owned()))
}

fn array_at<'a>(mut value: &'a Value, path: &[&str]) -> Result<&'a [Value], CorridorError> {
    for segment in path {
        value = value
            .get(*segment)
            .ok_or_else(|| CorridorError::Missing(path.join(".")))?;
    }
    value
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| CorridorError::Missing(path.join(".")))
}

fn required_object<'a>(value: &'a Value, field: &str) -> Result<&'a Value, CorridorError> {
    let object = value
        .get(field)
        .ok_or_else(|| CorridorError::Missing(field.to_owned()))?;
    object
        .as_object()
        .ok_or_else(|| CorridorError::Missing(field.to_owned()))?;
    Ok(object)
}

fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, CorridorError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| CorridorError::Missing(field.to_owned()))
}

fn require_string(value: &Value, field: &str, expected: &str) -> Result<(), CorridorError> {
    let actual = required_string(value, field)?;
    if actual != expected {
        return Err(CorridorError::Mismatch {
            path: field.to_owned(),
            expected: expected.to_owned(),
            actual: actual.to_owned(),
        });
    }
    Ok(())
}

fn required_u64(value: &Value, field: &str) -> Result<u64, CorridorError> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| CorridorError::Missing(field.to_owned()))
}

fn require_u64(value: &Value, field: &str, expected: u64) -> Result<(), CorridorError> {
    let actual = value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| CorridorError::Missing(field.to_owned()))?;
    if actual != expected {
        return Err(CorridorError::Mismatch {
            path: field.to_owned(),
            expected: expected.to_string(),
            actual: actual.to_string(),
        });
    }
    Ok(())
}

fn required_u32(value: &Value, field: &str) -> Result<u32, CorridorError> {
    u32::try_from(required_u64(value, field)?)
        .map_err(|_| CorridorError::InvalidNumber(field.to_owned()))
}

fn required_f64(value: &Value, field: &str) -> Result<f64, CorridorError> {
    value
        .get(field)
        .and_then(Value::as_f64)
        .ok_or_else(|| CorridorError::Missing(field.to_owned()))
}

fn require_bool(value: &Value, field: &str, expected: bool) -> Result<(), CorridorError> {
    let actual = value
        .get(field)
        .and_then(Value::as_bool)
        .ok_or_else(|| CorridorError::Missing(field.to_owned()))?;
    if actual != expected {
        return Err(CorridorError::Mismatch {
            path: field.to_owned(),
            expected: expected.to_string(),
            actual: actual.to_string(),
        });
    }
    Ok(())
}

fn require_string_array(
    value: &Value,
    field: &str,
    expected: &[&str],
) -> Result<(), CorridorError> {
    let actual = required_array(value, field)?
        .iter()
        .map(|item| {
            item.as_str()
                .ok_or_else(|| CorridorError::Missing(field.to_owned()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if actual != expected {
        return Err(CorridorError::Mismatch {
            path: field.to_owned(),
            expected: expected.join(", "),
            actual: actual.join(", "),
        });
    }
    Ok(())
}

fn find_named<'a>(
    values: &'a [Value],
    id: &str,
    context: &str,
) -> Result<&'a Value, CorridorError> {
    values
        .iter()
        .find(|value| value.get("id").and_then(Value::as_str) == Some(id))
        .ok_or_else(|| CorridorError::UnknownReference(format!("{context}: {id}")))
}

fn entity_kind_name(kind: u16) -> Result<&'static str, CorridorError> {
    ENTITY_KIND_NAMES
        .get(usize::from(kind))
        .copied()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| CorridorError::Missing(format!("entity kind {kind}")))
}

fn required_count(counts: &BTreeMap<String, u64>, field: &str) -> Result<u64, CorridorError> {
    counts
        .get(field)
        .copied()
        .ok_or_else(|| CorridorError::Missing(field.to_owned()))
}

fn add_u64(
    left: u64,
    right: Result<u64, CorridorError>,
    field: &str,
) -> Result<u64, CorridorError> {
    left.checked_add(right?)
        .ok_or_else(|| contract_error(format!("{field} overflow")))
}

fn mul_u64(left: u64, right: u64, field: &str) -> Result<u64, CorridorError> {
    left.checked_mul(right)
        .ok_or_else(|| contract_error(format!("{field} overflow")))
}

fn sum_u64(values: &[u64], field: &str) -> Result<u64, CorridorError> {
    values.iter().try_fold(0_u64, |total, value| {
        total
            .checked_add(*value)
            .ok_or_else(|| contract_error(format!("{field} overflow")))
    })
}

fn usize_u64(value: usize, field: &str) -> Result<u64, CorridorError> {
    u64::try_from(value).map_err(|_| contract_error(format!("{field} does not fit u64")))
}

fn to_usize(value: u64, field: &str) -> Result<usize, CorridorError> {
    usize::try_from(value).map_err(|_| contract_error(format!("{field} does not fit usize")))
}

fn contract_error(message: impl Into<String>) -> CorridorError {
    CorridorError::Contract(message.into())
}

fn append_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn append_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn lower_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_repository_contract;

    #[test]
    fn bound_fixture_template_matches_every_frozen_per_unit_count() {
        let trusted = load_repository_contract().expect("frozen contract");
        let contract =
            CorridorContract::from_manifest(&trusted.workload_manifest).expect("corridor contract");
        let template = contract
            .load_template(&crate::repository_root())
            .expect("bound corridor fixture template");

        assert_eq!(template.entities.len(), 357);
        assert_eq!(template.relations.len(), 627);
        assert_eq!(template.geometry.len(), 1_398);
        contract
            .validate_template(&template)
            .expect("all frozen per-unit counts");
    }

    #[test]
    fn all_graph_profiles_materialize_the_complete_n1_pipeline() {
        let trusted = load_repository_contract().expect("frozen contract");
        let generator = trusted.generator_contract().expect("generator contract");
        let identity = trusted.identity_contract().expect("identity contract");
        let stage = trusted.stage_contract().expect("stage contract");
        let contract =
            CorridorContract::from_manifest(&trusted.workload_manifest).expect("corridor contract");
        let template = contract
            .load_template(&crate::repository_root())
            .expect("bound corridor fixture template");

        for profile in GraphProfileId::ALL {
            let output = build_corridor_stage_case(
                &generator, &identity, &stage, &contract, &template, profile, 1,
            )
            .expect("complete corridor pipeline");
            assert_eq!(output.records.len(), 2_382);
            assert_eq!(output.summary.counts.semantic_output_record, 2_382);
            assert_eq!(output.summary.counts.semantic_payload_byte_count, 88_167);
            assert_eq!(
                output.semantic_record_stream.len() as u64,
                output.summary.counts.output_byte_count
            );
            assert_eq!(output.materialization.output, output.semantic_record_stream);
            assert_eq!(
                output
                    .summary
                    .record_kind_counts
                    .get("canonical-geometry-point"),
                Some(&1_398)
            );
        }
    }

    #[test]
    fn n2_scales_only_per_unit_semantics_and_preserves_graph_shape_effects() {
        let trusted = load_repository_contract().expect("frozen contract");
        for profile in GraphProfileId::ALL {
            let n1 =
                build_corridor_stage_summary(&trusted, profile, 1).expect("N=1 corridor summary");
            let n2 =
                build_corridor_stage_summary(&trusted, profile, 2).expect("N=2 corridor summary");
            assert_eq!(n2.counts.identity_declaration_count, 714);
            assert_eq!(n2.counts.semantic_output_record, 4_764);
            assert_eq!(
                n2.counts.semantic_payload_byte_count,
                n1.counts.semantic_payload_byte_count * 2
            );
            assert_eq!(
                n2.record_kind_counts["identity-declaration"],
                n1.record_kind_counts["identity-declaration"] * 2
            );
        }
    }

    #[test]
    fn corridor_summary_known_vector_has_exact_frozen_bytes() {
        let bytes = std::fs::read(
            crate::repository_root().join(
                "research/issue-308-compiler-budget-calibration-research/known-vectors/corridor-summary-v1.json",
            ),
        )
        .expect("corridor summary known vector");
        assert_eq!(bytes.len(), CORRIDOR_KNOWN_VECTOR_BYTE_LENGTH);
        assert_eq!(
            lower_hex(&Sha256::digest(&bytes)),
            CORRIDOR_KNOWN_VECTOR_SHA256
        );
    }
}
