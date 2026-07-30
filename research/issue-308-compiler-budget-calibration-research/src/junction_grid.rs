//! `LF-COMP-JUNCTION-GRID-v1` 的密集路口生成器、八阶段管线与摘要已知向量。
//!
//! 本工作负载只表达冻结的关系密度与规范排序压力，不表达信号策略、冲突裁决、
//! 排队溢流或现实城市代表性。

use crate::corridor::{
    CorridorCaseOutput, CorridorError, CorridorStageSummary, CorridorTemplate, EntityRef,
    TemplateEntity, TemplateGeometry, TemplateGeometryRule, TemplateRelation,
    build_template_stage_case,
};
use crate::{GraphProfileId, TrustedContract};
use serde::Serialize;
use serde_json::Value;
#[cfg(test)]
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const JUNCTION_GRID_WORKLOAD_ID: &str = "LF-COMP-JUNCTION-GRID-v1";
pub const JUNCTION_GRID_KNOWN_VECTOR_SCHEMA: &str =
    "laneflow.compiler-calibration-junction-grid-summary-known-vectors";

const SHORT_UNIQUE_PROFILE_ID: &str = "short-unique-v1";
const EDGE_POSITION_BITS: u32 = 1.0_f32.to_bits();
#[cfg(test)]
const JUNCTION_GRID_KNOWN_VECTOR_BYTE_LENGTH: usize = 8_601;
#[cfg(test)]
const JUNCTION_GRID_KNOWN_VECTOR_SHA256: &str =
    "b11712cb527111432e6fba20370c54578cd9a8bb45b9305eea7f3f17a4f84a96";

const EXPECTED_STAGE_INPUTS: [(&str, u64); 6] = [
    ("sourceDeclarationCount", 166),
    ("identityFieldOccurrenceCount", 464),
    ("profiledKeyOccurrenceCount", 190),
    ("sourceReferenceCount", 544),
    ("sourceRelationCount", 252),
    ("sourceGeometryCount", 64),
];

const EXPECTED_PER_UNIT_COUNTS: [(&str, u64); 17] = [
    ("LaneEdge", 32),
    ("edgeConnection", 36),
    ("Junction", 1),
    ("Movement", 12),
    ("ManeuverPath", 12),
    ("StaticRoute", 12),
    ("routeOccurrence", 48),
    ("StopLine", 36),
    ("ManeuverGate", 36),
    ("WaitingZone", 24),
    ("CanonicalFrame", 1),
    ("canonicalGeometryPoint", 64),
    ("ownerRelation", 84),
    ("gateOccurrence", 36),
    ("waitingZoneOccurrence", 24),
    ("junctionInternalEdgeRole", 24),
    ("semanticOutputRecord", 482),
];

#[derive(Clone, Debug)]
pub struct JunctionGridContract {
    expected_stage_inputs: BTreeMap<String, u64>,
    expected_per_unit_counts: BTreeMap<String, u64>,
}

impl JunctionGridContract {
    pub fn from_manifest(manifest: &Value) -> Result<Self, JunctionGridError> {
        let workload = required_array(manifest, "workloads")?
            .iter()
            .find(|candidate| {
                candidate.get("id").and_then(Value::as_str) == Some(JUNCTION_GRID_WORKLOAD_ID)
            })
            .ok_or_else(|| JunctionGridError::Missing(JUNCTION_GRID_WORKLOAD_ID.to_owned()))?;
        require_bool(workload, "scalable", true)?;
        require_string_array(
            workload,
            "graphProfiles",
            &["wide-star-v1", "deep-chain-v1", "shared-fanin-dag-v1"],
        )?;
        require_string_array(workload, "stringProfiles", &[SHORT_UNIQUE_PROFILE_ID])?;
        validate_unit_construction(required_object(workload, "unitConstruction")?)?;
        validate_identity_expansion(manifest)?;
        validate_semantic_scalar_encodings(manifest)?;

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
                .map(|(name, value)| (name.to_owned(), value))
                .collect(),
            expected_per_unit_counts: EXPECTED_PER_UNIT_COUNTS
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect(),
        })
    }

    pub(crate) fn validate_template(
        &self,
        template: &CorridorTemplate,
    ) -> Result<(), JunctionGridError> {
        validate_count_map(
            "perUnitStageInputs",
            &self.expected_stage_inputs,
            &template.stage_input_counts(),
        )?;
        validate_count_map(
            "perUnitCounts",
            &self.expected_per_unit_counts,
            &template.domain_counts(),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JunctionGridKnownVectorDocument {
    pub schema: &'static str,
    pub schema_version: u32,
    pub source_workload_manifest_sha256: String,
    pub workload_id: &'static str,
    pub n: u32,
    pub string_profile: &'static str,
    pub vectors: Vec<CorridorStageSummary>,
}

pub fn build_junction_grid_known_vectors(
    trusted: &TrustedContract,
) -> Result<JunctionGridKnownVectorDocument, JunctionGridError> {
    let generator = trusted
        .generator_contract()
        .map_err(|error| JunctionGridError::Contract(error.to_string()))?;
    let identity = trusted
        .identity_contract()
        .map_err(|error| JunctionGridError::Contract(error.to_string()))?;
    let stage = trusted
        .stage_contract()
        .map_err(|error| JunctionGridError::Contract(error.to_string()))?;
    let contract = JunctionGridContract::from_manifest(&trusted.workload_manifest)?;
    let template = build_junction_grid_template();
    contract.validate_template(&template)?;
    let mut vectors = Vec::with_capacity(GraphProfileId::ALL.len());
    for graph_profile in GraphProfileId::ALL {
        vectors.push(
            build_junction_grid_stage_case(
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
    Ok(JunctionGridKnownVectorDocument {
        schema: JUNCTION_GRID_KNOWN_VECTOR_SCHEMA,
        schema_version: 1,
        source_workload_manifest_sha256: trusted.descriptor.workload_manifest.sha256.clone(),
        workload_id: JUNCTION_GRID_WORKLOAD_ID,
        n: 1,
        string_profile: SHORT_UNIQUE_PROFILE_ID,
        vectors,
    })
}

pub fn build_junction_grid_stage_summary(
    trusted: &TrustedContract,
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<CorridorStageSummary, JunctionGridError> {
    let generator = trusted
        .generator_contract()
        .map_err(|error| JunctionGridError::Contract(error.to_string()))?;
    let identity = trusted
        .identity_contract()
        .map_err(|error| JunctionGridError::Contract(error.to_string()))?;
    let stage = trusted
        .stage_contract()
        .map_err(|error| JunctionGridError::Contract(error.to_string()))?;
    let contract = JunctionGridContract::from_manifest(&trusted.workload_manifest)?;
    let template = build_junction_grid_template();
    Ok(build_junction_grid_stage_case(
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

pub(crate) fn build_junction_grid_stage_case(
    generator: &crate::GeneratorContract,
    identity: &crate::identity::IdentityContract,
    stage: &crate::stage::StageContract,
    contract: &JunctionGridContract,
    template: &CorridorTemplate,
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<CorridorCaseOutput, JunctionGridError> {
    contract.validate_template(template)?;
    Ok(build_template_stage_case(
        generator,
        identity,
        stage,
        JUNCTION_GRID_WORKLOAD_ID,
        template,
        graph_profile,
        n,
    )?)
}

pub(crate) fn build_junction_grid_template() -> CorridorTemplate {
    let mut entities = Vec::with_capacity(166);
    for local in 0..32 {
        entities.push(entity(4, local, []));
    }
    entities.push(entity(5, 0, []));
    for movement in 0..12 {
        entities.push(entity(6, movement, [(34, entity_ref(5, 0))]));
    }
    for (movement, entry, exit) in movements() {
        entities.push(entity(
            7,
            movement,
            [
                (11, entity_ref(6, movement)),
                (12, entity_ref(4, entry)),
                (13, entity_ref(4, 4 + exit)),
            ],
        ));
    }
    for movement in 0..12 {
        for transition in 0..3 {
            entities.push(entity(
                8,
                movement * 3 + transition,
                [(14, entity_ref(7, movement))],
            ));
        }
    }
    for movement in 0..12 {
        for pair in 0..2 {
            entities.push(entity(
                9,
                movement * 2 + pair,
                [(14, entity_ref(7, movement))],
            ));
        }
    }
    for local in 0..36 {
        entities.push(entity(10, local, []));
    }
    for local in 0..12 {
        entities.push(entity(21, local, []));
    }
    entities.push(entity(22, 0, []));

    let mut relations = Vec::with_capacity(252);
    for (movement, entry, exit) in movements() {
        let junction = entity_ref(5, 0);
        let movement_ref = entity_ref(6, movement);
        let path = entity_ref(7, movement);
        let route = entity_ref(21, movement);
        let edges = [
            entity_ref(4, entry),
            entity_ref(4, 8 + 2 * movement),
            entity_ref(4, 9 + 2 * movement),
            entity_ref(4, 4 + exit),
        ];
        relations.push(TemplateRelation::Owner {
            child: movement_ref,
            parent: junction,
        });
        relations.push(TemplateRelation::Owner {
            child: path,
            parent: movement_ref,
        });
        for transition in 0..3 {
            let gate = entity_ref(8, movement * 3 + transition);
            relations.push(TemplateRelation::Owner {
                child: gate,
                parent: path,
            });
            relations.push(TemplateRelation::EdgeConnection {
                source: edges[transition as usize],
                target: edges[transition as usize + 1],
            });
            relations.push(TemplateRelation::Gate {
                path,
                transition_index: transition,
                gate,
                stop_line: entity_ref(10, movement * 3 + transition),
                edge: edges[transition as usize],
                edge_position_bits: EDGE_POSITION_BITS,
            });
        }
        for (pair, (entry_transition, release_transition)) in
            [(0_u32, 1_u32), (1_u32, 2_u32)].into_iter().enumerate()
        {
            let pair = pair as u32;
            let zone = entity_ref(9, movement * 2 + pair);
            relations.push(TemplateRelation::Owner {
                child: zone,
                parent: path,
            });
            relations.push(TemplateRelation::WaitingZone {
                path,
                entry_transition_index: entry_transition,
                release_transition_index: release_transition,
                zone,
                before_gate: entity_ref(8, movement * 3 + entry_transition),
                after_gate: entity_ref(8, movement * 3 + release_transition),
                capacity: pair + 1,
            });
        }
        for (index, edge) in edges.into_iter().enumerate() {
            relations.push(TemplateRelation::RouteOccurrence {
                route,
                index: index as u32,
                edge,
            });
        }
        relations.push(TemplateRelation::JunctionInternalEdge {
            junction,
            edge: entity_ref(4, 8 + 2 * movement),
        });
        relations.push(TemplateRelation::JunctionInternalEdge {
            junction,
            edge: entity_ref(4, 9 + 2 * movement),
        });
    }
    let mut geometry = Vec::with_capacity(64);
    for lane_edge_local in 0..32 {
        for point_index in 0..2 {
            geometry.push(TemplateGeometry {
                edge: entity_ref(4, lane_edge_local),
                frame: entity_ref(22, 0),
                point_index,
                x_bits: ((lane_edge_local * 2 + point_index) as f32).to_bits(),
                y_bits: 0.0_f32.to_bits(),
                z_bits: 0.0_f32.to_bits(),
                coordinate_rule: TemplateGeometryRule::JunctionGridV1,
            });
        }
    }
    CorridorTemplate {
        entities,
        relations,
        geometry,
    }
}

fn movements() -> Vec<(u32, u32, u32)> {
    let mut result = Vec::with_capacity(12);
    for entry in 0..4 {
        for exit in 0..4 {
            if exit != entry {
                let movement = result.len() as u32;
                result.push((movement, entry, exit));
            }
        }
    }
    result
}

fn entity<const N: usize>(
    kind: u16,
    local: u32,
    identity_references: [(u16, EntityRef); N],
) -> TemplateEntity {
    TemplateEntity {
        reference: entity_ref(kind, local),
        identity_references: identity_references.into_iter().collect(),
    }
}

const fn entity_ref(kind: u16, local: u32) -> EntityRef {
    EntityRef { kind, local }
}

fn validate_unit_construction(value: &Value) -> Result<(), JunctionGridError> {
    require_u64_array(value, "directionCodes", &[0, 1, 2, 3])?;
    require_string(
        value,
        "movementOrder",
        "entryDirection ascending, then exitDirection ascending excluding exitDirection == entryDirection",
    )?;
    require_string(
        value,
        "movementIndexFormula",
        "entryDirection * 3 + rank(exitDirection among allowed ascending exits)",
    )?;
    require_string(value, "approachLaneEdgeLocalIndexFormula", "entryDirection")?;
    require_string(value, "exitLaneEdgeLocalIndexFormula", "4 + exitDirection")?;
    require_string(
        value,
        "firstInternalLaneEdgeLocalIndexFormula",
        "8 + 2 * movementIndex",
    )?;
    require_string(
        value,
        "secondInternalLaneEdgeLocalIndexFormula",
        "9 + 2 * movementIndex",
    )?;
    require_string_array(
        value,
        "pathLaneEdgeOrder",
        &["approach", "firstInternal", "secondInternal", "exit"],
    )?;
    require_u64_array(value, "gateTransitionIndices", &[0, 1, 2])?;
    require_string(
        value,
        "gateOccurrenceLaneEdgeFormula",
        "pathLaneEdgeOrder[gateTransitionIndex]",
    )?;
    require_u64(
        value,
        "gateOccurrenceEdgePositionCanonicalF32BitsU32Le",
        u64::from(EDGE_POSITION_BITS),
    )?;
    require_string(value, "stopLineRule", "one distinct StopLine per gate")?;
    require_string(
        value,
        "stopLineLaneEdgeFormula",
        "pathLaneEdgeOrder[gateTransitionIndex]",
    )?;
    require_string(value, "stopLineLocation", "edgeEnd")?;
    require_pair_array(value, "waitingZoneGatePairs", &[(0, 1), (1, 2)])?;
    require_string(
        value,
        "waitingZoneCapacityFormula",
        "waitingZonePairIndex + 1",
    )?;
    require_u64(value, "gridWidthUnits", 4_096)?;
    require_string(value, "unitXFormula", "unitIndex mod 4096")?;
    require_string(value, "unitYFormula", "floor(unitIndex / 4096)")?;
    let geometry = required_object(value, "geometryPointFormula")?;
    require_string_array(
        geometry,
        "point0",
        &[
            "canonicalF32(unitX * 128 + laneEdgeLocalIndex * 2)",
            "canonicalF32(unitY * 128)",
            "0.0",
        ],
    )?;
    require_string_array(
        geometry,
        "point1",
        &[
            "canonicalF32(unitX * 128 + laneEdgeLocalIndex * 2 + 1)",
            "canonicalF32(unitY * 128)",
            "0.0",
        ],
    )
}

fn validate_identity_expansion(manifest: &Value) -> Result<(), JunctionGridError> {
    let expansion = required_object(manifest, "identityBindingExpansion")?;
    let ordinal_by_workload = required_object(expansion, "declarationOrdinalByWorkload")?;
    require_exact_string_object(
        required_object(ordinal_by_workload, JUNCTION_GRID_WORKLOAD_ID)?,
        &[
            ("LaneEdge", "unitConstruction lane-edge local index 0..31"),
            ("Junction", "0"),
            ("Movement", "movementIndex"),
            ("ManeuverPath", "movementIndex"),
            ("ManeuverGate", "movementIndex * 3 + gateTransitionIndex"),
            ("WaitingZone", "movementIndex * 2 + waitingZonePairIndex"),
            ("StopLine", "movementIndex * 3 + gateTransitionIndex"),
            ("StaticRoute", "movementIndex"),
            ("CanonicalFrame", "0"),
        ],
        "declarationOrdinalByWorkload",
    )?;
    let profiled = required_object(expansion, "profiledKeyExpression")?;
    let profiled_by_workload = required_object(profiled, "expandedKeyFormulaByWorkload")?;
    require_string(
        profiled_by_workload,
        JUNCTION_GRID_WORKLOAD_ID,
        "short-unique-v1 profiledKeyFormula with kindCode=current entity kind, unitIndex=current workload unit index, localIndex=expandedLocalIndex",
    )?;
    let stable = required_object(expansion, "stableIdExpression")?;
    let resolution_by_workload = required_object(stable, "referenceResolutionByWorkload")?;
    require_exact_string_object(
        required_object(resolution_by_workload, JUNCTION_GRID_WORKLOAD_ID)?,
        &[
            ("Movement.owning-junction", "Junction ordinal 0"),
            (
                "ManeuverPath.owning-movement",
                "Movement ordinal movementIndex",
            ),
            (
                "ManeuverPath.entry-lane-edge",
                "LaneEdge ordinal entryDirection",
            ),
            (
                "ManeuverPath.exit-lane-edge",
                "LaneEdge ordinal 4 + exitDirection",
            ),
            (
                "ManeuverGate.owning-maneuver-path",
                "ManeuverPath ordinal movementIndex",
            ),
            (
                "WaitingZone.owning-maneuver-path",
                "ManeuverPath ordinal movementIndex",
            ),
        ],
        "referenceResolutionByWorkload",
    )
}

fn validate_semantic_scalar_encodings(manifest: &Value) -> Result<(), JunctionGridError> {
    let encodings = required_object(manifest, "semanticScalarEncodings")?;
    require_u64(encodings, "version", 1)?;
    let gate = required_object(encodings, "gateOccurrenceConstruction")?;
    require_string(
        gate,
        "occurrenceOrder",
        "ascending transitionIndex, then maneuverGateStableIdBytes16 unsigned lexicographic",
    )?;
    require_string(
        gate,
        "occurrenceIndexFormula",
        "zero-based position in occurrenceOrder for the owning ManeuverPath",
    )?;
    require_string(
        gate,
        "laneEdgeFormula",
        "orderedManeuverPathLaneEdgeStableIds[transitionIndex]",
    )?;
    require_string(gate, "edgePositionFormula", "canonicalF32(+1.0)")?;
    require_u64(
        gate,
        "edgePositionCanonicalF32BitsU32Le",
        u64::from(EDGE_POSITION_BITS),
    )?;
    require_string(
        gate,
        "stopLineConstraint",
        "the referenced StopLine belongs to orderedManeuverPathLaneEdgeStableIds[transitionIndex] and current v0.10 location edgeEnd",
    )?;
    let waiting = required_object(encodings, "waitingZoneOccurrenceConstruction")?;
    require_string(
        waiting,
        "occurrenceOrder",
        "ascending entry gate transitionIndex, then release gate transitionIndex, then waitingZoneStableIdBytes16 unsigned lexicographic",
    )?;
    require_string(
        waiting,
        "occurrenceIndexFormula",
        "zero-based position in occurrenceOrder for the owning ManeuverPath",
    )?;
    require_string(
        waiting,
        "beforeGateFormula",
        "entryGateId resolved to ManeuverGate StableId128",
    )?;
    require_string(
        waiting,
        "afterGateFormula",
        "releaseGateId resolved to ManeuverGate StableId128",
    )?;
    require_string(
        waiting,
        "junctionGridCapacityFormula",
        "waitingZonePairIndex + 1",
    )
}

fn validate_count_map(
    prefix: &str,
    expected: &BTreeMap<String, u64>,
    actual: &BTreeMap<String, u64>,
) -> Result<(), JunctionGridError> {
    for (field, expected_value) in expected {
        let actual_value = actual.get(field).copied().unwrap_or(0);
        if actual_value != *expected_value {
            return Err(JunctionGridError::Mismatch {
                path: format!("{prefix}.{field}"),
                expected: expected_value.to_string(),
                actual: actual_value.to_string(),
            });
        }
    }
    Ok(())
}

fn required_array<'a>(value: &'a Value, field: &str) -> Result<&'a [Value], JunctionGridError> {
    value
        .get(field)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| JunctionGridError::Missing(field.to_owned()))
}

fn required_object<'a>(value: &'a Value, field: &str) -> Result<&'a Value, JunctionGridError> {
    value
        .get(field)
        .filter(|candidate| candidate.is_object())
        .ok_or_else(|| JunctionGridError::Missing(field.to_owned()))
}

fn require_bool(value: &Value, field: &str, expected: bool) -> Result<(), JunctionGridError> {
    let actual = value.get(field).and_then(Value::as_bool);
    require_equal(
        field,
        expected.to_string(),
        actual.map(|item| item.to_string()),
    )
}

fn require_u64(value: &Value, field: &str, expected: u64) -> Result<(), JunctionGridError> {
    let actual = value.get(field).and_then(Value::as_u64);
    require_equal(
        field,
        expected.to_string(),
        actual.map(|item| item.to_string()),
    )
}

fn require_string(value: &Value, field: &str, expected: &str) -> Result<(), JunctionGridError> {
    let actual = value.get(field).and_then(Value::as_str);
    require_equal(field, expected.to_owned(), actual.map(str::to_owned))
}

fn require_string_array(
    value: &Value,
    field: &str,
    expected: &[&str],
) -> Result<(), JunctionGridError> {
    let actual = required_array(value, field)?
        .iter()
        .map(|item| item.as_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>();
    require_equal(
        field,
        format!("{expected:?}"),
        actual.map(|items| format!("{items:?}")),
    )
}

fn require_u64_array(
    value: &Value,
    field: &str,
    expected: &[u64],
) -> Result<(), JunctionGridError> {
    let actual = required_array(value, field)?
        .iter()
        .map(Value::as_u64)
        .collect::<Option<Vec<_>>>();
    require_equal(
        field,
        format!("{expected:?}"),
        actual.map(|items| format!("{items:?}")),
    )
}

fn require_pair_array(
    value: &Value,
    field: &str,
    expected: &[(u64, u64)],
) -> Result<(), JunctionGridError> {
    let actual = required_array(value, field)?
        .iter()
        .map(|item| {
            let pair = item.as_array()?;
            if pair.len() != 2 {
                return None;
            }
            Some((pair.first()?.as_u64()?, pair.get(1)?.as_u64()?))
        })
        .collect::<Option<Vec<_>>>();
    require_equal(
        field,
        format!("{expected:?}"),
        actual.map(|items| format!("{items:?}")),
    )
}

fn require_exact_string_object(
    value: &Value,
    expected: &[(&str, &str)],
    path: &str,
) -> Result<(), JunctionGridError> {
    let object = value
        .as_object()
        .ok_or_else(|| JunctionGridError::Missing(path.to_owned()))?;
    if object.len() != expected.len() {
        return Err(JunctionGridError::Mismatch {
            path: path.to_owned(),
            expected: format!("{} fields", expected.len()),
            actual: format!("{} fields", object.len()),
        });
    }
    for (field, expected_value) in expected {
        require_string(value, field, expected_value)?;
    }
    Ok(())
}

fn require_equal(
    path: &str,
    expected: String,
    actual: Option<String>,
) -> Result<(), JunctionGridError> {
    if actual.as_deref() != Some(expected.as_str()) {
        return Err(JunctionGridError::Mismatch {
            path: path.to_owned(),
            expected,
            actual: actual.unwrap_or_else(|| "<missing>".to_owned()),
        });
    }
    Ok(())
}

#[cfg(test)]
fn lower_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

#[derive(Debug, thiserror::Error)]
pub enum JunctionGridError {
    #[error(transparent)]
    Corridor(#[from] CorridorError),
    #[error("路口网格清单缺少 `{0}`")]
    Missing(String),
    #[error("路口网格清单不一致：{path} 应为 {expected}，实际为 {actual}")]
    Mismatch {
        path: String,
        expected: String,
        actual: String,
    },
    #[error("路口网格研究契约错误：{0}")]
    Contract(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_template_matches_all_frozen_per_unit_counts() {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        let contract = JunctionGridContract::from_manifest(&trusted.workload_manifest)
            .expect("junction contract");
        contract
            .validate_template(&build_junction_grid_template())
            .expect("junction template");
    }

    #[test]
    fn parallel_identity_and_scalar_registrations_fail_closed_on_drift() {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        let mut identity_drift = trusted.workload_manifest.clone();
        identity_drift["identityBindingExpansion"]["declarationOrdinalByWorkload"]
            [JUNCTION_GRID_WORKLOAD_ID]["ManeuverGate"] =
            Value::String("source traversal order".to_owned());
        assert!(JunctionGridContract::from_manifest(&identity_drift).is_err());

        let mut scalar_drift = trusted.workload_manifest.clone();
        scalar_drift["semanticScalarEncodings"]["waitingZoneOccurrenceConstruction"]["junctionGridCapacityFormula"] =
            Value::String("waitingZonePairIndex".to_owned());
        assert!(JunctionGridContract::from_manifest(&scalar_drift).is_err());
    }

    #[test]
    fn all_graph_profiles_materialize_the_complete_n1_pipeline() {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        for profile in GraphProfileId::ALL {
            let summary =
                build_junction_grid_stage_summary(&trusted, profile, 1).expect("junction summary");
            assert_eq!(summary.counts.identity_declaration_count, 166);
            assert_eq!(summary.counts.source_relation_count, 252);
            assert_eq!(summary.counts.source_geometry_count, 64);
            assert_eq!(summary.counts.semantic_output_record, 482);
            assert_eq!(
                summary
                    .record_kind_counts
                    .get("waiting-zone-occurrence")
                    .copied(),
                Some(24)
            );
        }
    }

    #[test]
    fn n2_preserves_per_unit_semantics_and_changes_grid_geometry() {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        let generator = trusted.generator_contract().expect("generator contract");
        let identity = trusted.identity_contract().expect("identity contract");
        let stage = trusted.stage_contract().expect("stage contract");
        let contract = JunctionGridContract::from_manifest(&trusted.workload_manifest)
            .expect("junction contract");
        let template = build_junction_grid_template();
        let n1 = build_junction_grid_stage_case(
            &generator,
            &identity,
            &stage,
            &contract,
            &template,
            GraphProfileId::WideStar,
            1,
        )
        .expect("N=1");
        let n2 = build_junction_grid_stage_case(
            &generator,
            &identity,
            &stage,
            &contract,
            &template,
            GraphProfileId::WideStar,
            2,
        )
        .expect("N=2");
        assert_eq!(n2.summary.counts.identity_declaration_count, 2 * 166);
        assert_eq!(n2.summary.counts.source_relation_count, 2 * 252);
        assert_eq!(n2.summary.counts.source_geometry_count, 2 * 64);
        assert_eq!(n2.summary.counts.semantic_output_record, 2 * 482);
        assert_ne!(
            n1.summary.semantic_digest_sha256,
            n2.summary.semantic_digest_sha256
        );
        let second_unit_geometry = n2
            .records
            .iter()
            .filter(|record| record.record_kind == 5)
            .filter(|record| {
                let x_bits = u32::from_le_bytes(record.payload[20..24].try_into().expect("x bits"));
                f32::from_bits(x_bits) >= 128.0
            })
            .count();
        assert_eq!(second_unit_geometry, 64);
    }

    #[test]
    fn junction_grid_summary_known_vector_has_exact_frozen_bytes() {
        let path = crate::repository_root().join(
            "research/issue-308-compiler-budget-calibration-research/known-vectors/junction-grid-summary-v1.json",
        );
        let bytes = std::fs::read(path).expect("junction grid summary known vector");
        assert_eq!(bytes.len(), JUNCTION_GRID_KNOWN_VECTOR_BYTE_LENGTH);
        assert_eq!(
            lower_hex(&Sha256::digest(&bytes)),
            JUNCTION_GRID_KNOWN_VECTOR_SHA256
        );
        let trusted = crate::load_repository_contract().expect("frozen contract");
        let generated =
            serde_json::to_string_pretty(&build_junction_grid_known_vectors(&trusted).unwrap())
                .expect("serialize junction vectors")
                + "\n";
        assert_eq!(generated.as_bytes(), bytes);
    }
}
