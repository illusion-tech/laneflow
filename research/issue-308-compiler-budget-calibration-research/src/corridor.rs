//! `LF-COMP-CORRIDOR-v1` 的绑定单元配方、八阶段管线与摘要已知向量。
//!
//! 单元配方是研究内部的有类型模板，不再读取 current JSON。模板保留有类型局部序号、
//! 引用、标量和规范几何；规模运行只复制模板并执行确定性阶段降阶。

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
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const CORRIDOR_WORKLOAD_ID: &str = "LF-COMP-CORRIDOR-v1";
pub const CORRIDOR_KNOWN_VECTOR_SCHEMA: &str =
    "laneflow.compiler-calibration-corridor-summary-known-vectors";
const CORRIDOR_UNIT_TEMPLATE_JSON: &str = include_str!("../templates/corridor-unit-v1.json");
#[cfg(test)]
const CORRIDOR_KNOWN_VECTOR_BYTE_LENGTH: usize = 9_101;
#[cfg(test)]
const CORRIDOR_KNOWN_VECTOR_SHA256: &str =
    "eb3fdb0a6ae900568e919475a30a58e525bfb3dd9311e8d3cd51c253c5746e1c";

const ENTITY_KIND_ABSENT: u16 = 0;
const SHARED_CONSTANT_ENTITY_KIND: u16 = 0x00ff;
const SHORT_UNIQUE_PROFILE_ID: &str = "short-unique-v1";

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

        Ok(Self {
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

    pub(crate) fn load_template(&self) -> Result<CorridorTemplate, CorridorError> {
        let template = serde_json::from_str(CORRIDOR_UNIT_TEMPLATE_JSON).map_err(|source| {
            CorridorError::Json {
                path: "templates/corridor-unit-v1.json".to_owned(),
                source,
            }
        })?;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct EntityRef {
    pub(crate) kind: u16,
    pub(crate) local: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TemplateEntity {
    pub(crate) reference: EntityRef,
    pub(crate) identity_references: BTreeMap<u16, EntityRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum TemplateGeometryRule {
    Fixed,
    JunctionGridV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TemplateGeometry {
    pub(crate) edge: EntityRef,
    pub(crate) frame: EntityRef,
    pub(crate) point_index: u32,
    pub(crate) x_bits: u32,
    pub(crate) y_bits: u32,
    pub(crate) z_bits: u32,
    pub(crate) coordinate_rule: TemplateGeometryRule,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    let template = contract.load_template()?;
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
    let template = contract.load_template()?;
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

fn required_object<'a>(value: &'a Value, field: &str) -> Result<&'a Value, CorridorError> {
    let object = value
        .get(field)
        .ok_or_else(|| CorridorError::Missing(field.to_owned()))?;
    object
        .as_object()
        .ok_or_else(|| CorridorError::Missing(field.to_owned()))?;
    Ok(object)
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
            .load_template()
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
            .load_template()
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
