//! `LF-COMP-JUNCTION-GRID-v1` 的独立领域展开与完整记录流预言机。
//!
//! 这里重新实现路口公式，并故意反转关系输入顺序；它不调用生产者模板构造函数，
//! 也不调用生产者身份、记录编码或规范排序实现。

use crate::corridor::{
    CorridorTemplate, EntityRef, TemplateEntity, TemplateGeometry, TemplateGeometryRule,
    TemplateRelation,
};
use crate::corridor_oracle::build_template_oracle_records;
use crate::junction_grid::{
    JUNCTION_GRID_WORKLOAD_ID, JunctionGridContract, JunctionGridError,
    build_junction_grid_stage_case, build_junction_grid_template,
};
use crate::{GraphProfileId, TrustedContract};
use serde::Serialize;
use std::collections::BTreeMap;

const EDGE_POSITION_BITS: u32 = 1.0_f32.to_bits();

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JunctionGridOracleVerificationReport {
    pub checked_cases: u32,
    pub checked_n1_cases: u32,
    pub checked_n2_cases: u32,
    pub independent_formula_projection_checked: bool,
    pub reversed_relation_input_checked: bool,
}

pub fn verify_junction_grid_oracle_matrix(
    trusted: &TrustedContract,
) -> Result<JunctionGridOracleVerificationReport, JunctionGridOracleError> {
    let contract = JunctionGridContract::from_manifest(&trusted.workload_manifest)?;
    let producer_template = build_junction_grid_template();
    let independent_template = build_independent_template();
    contract.validate_template(&producer_template)?;
    contract.validate_template(&independent_template)?;
    verify_template_projection(&producer_template, &independent_template)?;

    let mut checked_cases = 0_u32;
    for graph_profile in GraphProfileId::ALL {
        for n in [1, 2] {
            verify_junction_grid_oracle_case_with_templates(
                trusted,
                &contract,
                &producer_template,
                &independent_template,
                graph_profile,
                n,
            )?;
            checked_cases = checked_cases
                .checked_add(1)
                .ok_or_else(|| JunctionGridOracleError::Contract("checkedCases overflow".into()))?;
        }
    }
    Ok(JunctionGridOracleVerificationReport {
        checked_cases,
        checked_n1_cases: 3,
        checked_n2_cases: 3,
        independent_formula_projection_checked: true,
        reversed_relation_input_checked: true,
    })
}

fn verify_junction_grid_oracle_case_with_templates(
    trusted: &TrustedContract,
    contract: &JunctionGridContract,
    producer_template: &CorridorTemplate,
    independent_template: &CorridorTemplate,
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<crate::CorridorStageSummary, JunctionGridOracleError> {
    let generator = trusted
        .generator_contract()
        .map_err(|error| JunctionGridOracleError::Contract(error.to_string()))?;
    let identity = trusted
        .identity_contract()
        .map_err(|error| JunctionGridOracleError::Contract(error.to_string()))?;
    let stage = trusted
        .stage_contract()
        .map_err(|error| JunctionGridOracleError::Contract(error.to_string()))?;
    let produced = build_junction_grid_stage_case(
        &generator,
        &identity,
        &stage,
        contract,
        producer_template,
        graph_profile,
        n,
    )?;
    let oracle = build_template_oracle_records(
        &trusted.workload_manifest,
        JUNCTION_GRID_WORKLOAD_ID,
        independent_template,
        graph_profile,
        n,
    )?;
    if produced.records != oracle.records
        || produced.semantic_record_stream != oracle.stream
        || produced.materialization.output != produced.semantic_record_stream
    {
        return Err(JunctionGridOracleError::Mismatch { graph_profile, n });
    }
    Ok(produced.summary)
}

pub(crate) fn build_independent_template() -> CorridorTemplate {
    let mut entities = Vec::with_capacity(166);
    for local in 0..32 {
        entities.push(independent_entity(4, local, &[]));
    }
    entities.push(independent_entity(5, 0, &[]));
    for movement_index in 0..12 {
        entities.push(independent_entity(
            6,
            movement_index,
            &[(34, independent_ref(5, 0))],
        ));
    }
    for entry_direction in 0..4 {
        let mut allowed_rank = 0_u32;
        for exit_direction in 0..4 {
            if entry_direction == exit_direction {
                continue;
            }
            let movement_index = entry_direction * 3 + allowed_rank;
            allowed_rank += 1;
            entities.push(independent_entity(
                7,
                movement_index,
                &[
                    (11, independent_ref(6, movement_index)),
                    (12, independent_ref(4, entry_direction)),
                    (13, independent_ref(4, 4 + exit_direction)),
                ],
            ));
        }
    }
    for movement_index in 0..12 {
        for gate_transition_index in 0..3 {
            entities.push(independent_entity(
                8,
                movement_index * 3 + gate_transition_index,
                &[(14, independent_ref(7, movement_index))],
            ));
        }
    }
    for movement_index in 0..12 {
        for waiting_zone_pair_index in 0..2 {
            entities.push(independent_entity(
                9,
                movement_index * 2 + waiting_zone_pair_index,
                &[(14, independent_ref(7, movement_index))],
            ));
        }
    }
    for local in 0..36 {
        entities.push(independent_entity(10, local, &[]));
    }
    for local in 0..12 {
        entities.push(independent_entity(21, local, &[]));
    }
    entities.push(independent_entity(22, 0, &[]));

    let mut relations = Vec::with_capacity(252);
    for entry_direction in 0..4 {
        let mut allowed_rank = 0_u32;
        for exit_direction in 0..4 {
            if entry_direction == exit_direction {
                continue;
            }
            let movement_index = entry_direction * 3 + allowed_rank;
            allowed_rank += 1;
            let junction = independent_ref(5, 0);
            let movement = independent_ref(6, movement_index);
            let path = independent_ref(7, movement_index);
            let path_edges = [
                independent_ref(4, entry_direction),
                independent_ref(4, 8 + 2 * movement_index),
                independent_ref(4, 9 + 2 * movement_index),
                independent_ref(4, 4 + exit_direction),
            ];
            relations.push(TemplateRelation::Owner {
                child: movement,
                parent: junction,
            });
            relations.push(TemplateRelation::Owner {
                child: path,
                parent: movement,
            });
            for gate_transition_index in 0..3 {
                let gate = independent_ref(8, movement_index * 3 + gate_transition_index);
                relations.push(TemplateRelation::Owner {
                    child: gate,
                    parent: path,
                });
                relations.push(TemplateRelation::EdgeConnection {
                    source: path_edges[gate_transition_index as usize],
                    target: path_edges[gate_transition_index as usize + 1],
                });
                relations.push(TemplateRelation::Gate {
                    path,
                    transition_index: gate_transition_index,
                    gate,
                    stop_line: independent_ref(10, movement_index * 3 + gate_transition_index),
                    edge: path_edges[gate_transition_index as usize],
                    edge_position_bits: EDGE_POSITION_BITS,
                });
            }
            for (waiting_zone_pair_index, pair) in
                [(0_u32, 1_u32), (1_u32, 2_u32)].into_iter().enumerate()
            {
                let waiting_zone_pair_index = waiting_zone_pair_index as u32;
                relations.push(TemplateRelation::Owner {
                    child: independent_ref(9, movement_index * 2 + waiting_zone_pair_index),
                    parent: path,
                });
                relations.push(TemplateRelation::WaitingZone {
                    path,
                    entry_transition_index: pair.0,
                    release_transition_index: pair.1,
                    zone: independent_ref(9, movement_index * 2 + waiting_zone_pair_index),
                    before_gate: independent_ref(8, movement_index * 3 + pair.0),
                    after_gate: independent_ref(8, movement_index * 3 + pair.1),
                    capacity: waiting_zone_pair_index + 1,
                });
            }
            for (occurrence_index, edge) in path_edges.into_iter().enumerate() {
                relations.push(TemplateRelation::RouteOccurrence {
                    route: independent_ref(21, movement_index),
                    index: occurrence_index as u32,
                    edge,
                });
            }
            relations.extend([
                TemplateRelation::JunctionInternalEdge {
                    junction,
                    edge: independent_ref(4, 8 + 2 * movement_index),
                },
                TemplateRelation::JunctionInternalEdge {
                    junction,
                    edge: independent_ref(4, 9 + 2 * movement_index),
                },
            ]);
        }
    }
    relations.reverse();

    let mut geometry = Vec::with_capacity(64);
    for lane_edge_local_index in 0..32 {
        for point_index in 0..2 {
            geometry.push(TemplateGeometry {
                edge: independent_ref(4, lane_edge_local_index),
                frame: independent_ref(22, 0),
                point_index,
                x_bits: ((lane_edge_local_index * 2 + point_index) as f32).to_bits(),
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

fn independent_entity(
    kind: u16,
    local: u32,
    identity_references: &[(u16, EntityRef)],
) -> TemplateEntity {
    TemplateEntity {
        reference: independent_ref(kind, local),
        identity_references: identity_references.iter().copied().collect(),
    }
}

const fn independent_ref(kind: u16, local: u32) -> EntityRef {
    EntityRef { kind, local }
}

fn verify_template_projection(
    producer: &CorridorTemplate,
    independent: &CorridorTemplate,
) -> Result<(), JunctionGridOracleError> {
    if producer.entities != independent.entities || producer.geometry != independent.geometry {
        return Err(JunctionGridOracleError::Projection(
            "实体或网格几何公式投影不一致".to_owned(),
        ));
    }
    let producer_relations = relation_multiset(&producer.relations);
    let independent_relations = relation_multiset(&independent.relations);
    if producer_relations != independent_relations {
        return Err(JunctionGridOracleError::Projection(
            "关系公式投影不一致".to_owned(),
        ));
    }
    Ok(())
}

fn relation_multiset(relations: &[TemplateRelation]) -> BTreeMap<String, u32> {
    let mut counts = BTreeMap::new();
    for relation in relations {
        *counts.entry(format!("{relation:?}")).or_default() += 1;
    }
    counts
}

#[derive(Debug, thiserror::Error)]
pub enum JunctionGridOracleError {
    #[error(transparent)]
    Junction(#[from] JunctionGridError),
    #[error(transparent)]
    CorridorOracle(#[from] crate::corridor_oracle::CorridorOracleError),
    #[error("路口网格独立预言机与生产者不一致：graphProfile={graph_profile:?}, N={n}")]
    Mismatch {
        graph_profile: GraphProfileId,
        n: u32,
    },
    #[error("路口网格独立公式投影失败：{0}")]
    Projection(String),
    #[error("路口网格独立预言机契约错误：{0}")]
    Contract(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_formula_oracle_matches_all_n1_and_n2_cases() {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        let report =
            verify_junction_grid_oracle_matrix(&trusted).expect("junction grid oracle matrix");
        assert_eq!(report.checked_cases, 6);
        assert!(report.independent_formula_projection_checked);
        assert!(report.reversed_relation_input_checked);
    }
}
