//! 可扩展工作负载的安全有界独立预言机。
//!
//! 本模块不调用受测路径的声明解析、关系展开、局部序号或规范排序辅助函数。生产者与
//! 预言机只共享冻结输入值类型、受控容器和编码常量；预言机使用自己的扁平声明表、
//! 全局 `(kind, stable-id)` 序号排序和记录遍历。生产者八阶段输出与预言机完整有类型
//! 输出同时绑定同一个 `ControlledAllocator`，因此任一路径新增容量都必须先取得额度。

use crate::bounded_template::{
    BoundedTemplateExecution, PayloadRange, execute_bounded_template_stage_case,
    finalize_bounded_template_stage_case,
};
use crate::controlled_alloc::{
    ControlledAllocationObservation, ControlledAllocator, ControlledVec,
};
use crate::corridor::{
    CorridorContract, CorridorTemplate, EntityRef, TemplateEntity, TemplateGeometryRule,
    TemplateRelation, UnitEntityRef,
};
use crate::identity::{IdentityContract, IdentityFieldValue};
use crate::junction_grid::{JunctionGridContract, build_junction_grid_template};
use crate::junction_grid_oracle::build_independent_template;
use crate::{
    GeneratorContract, GraphProfileId, ScalableStagePlanFactory, ScalableStagePlanSummary,
    ScalableWorkloadId, StageGenerationError, TrustedContract,
};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::BTreeMap;

const IDENTITY_MAGIC: &[u8; 4] = b"LFID";
const STABLE_ID_DOMAIN: &[u8] = b"laneflow.stable-id.v1\0";
const ABSENT_LOCAL_INDEX: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OracleDeclaration {
    owner: UnitEntityRef,
    stable_id: [u8; 16],
    owner_ordinal: u32,
    identity_payload: PayloadRange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OracleLocalOrder {
    Absent,
    Explicit(u32),
    Payload,
    Gate {
        transition_index: u32,
        stable_id: [u8; 16],
    },
    Waiting {
        entry_transition_index: u32,
        release_transition_index: u32,
        stable_id: [u8; 16],
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OracleRecord {
    record_kind: u16,
    entity_kind_code: u16,
    stable_id: [u8; 16],
    owner_ordinal: u32,
    local_index: u32,
    payload: PayloadRange,
    local_order: OracleLocalOrder,
}

#[derive(Debug)]
struct BoundedOracleOutput {
    records: ControlledVec<OracleRecord>,
    payload: ControlledVec<u8>,
    stream: ControlledVec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BoundedOracleVerification {
    pub(crate) primary_record_count: u64,
    pub(crate) semantic_digest_sha256: String,
    pub(crate) complete_counts_equal: bool,
    pub(crate) complete_typed_output_equal: bool,
    pub(crate) allocation: ControlledAllocationObservation,
}

pub(crate) fn verify_bounded_scalable_oracle(
    trusted: &TrustedContract,
    workload_id: ScalableWorkloadId,
    graph_profile: GraphProfileId,
    n: u32,
    hard_ceiling_bytes: u64,
) -> Result<BoundedOracleVerification, BoundedOracleError> {
    let generator = trusted.generator_contract()?;
    let identity = trusted.identity_contract()?;
    let stage = trusted.stage_contract()?;
    let (producer_template, oracle_template, plan) =
        templates_and_plan(trusted, &identity, workload_id, graph_profile, n)?;
    let allocator = ControlledAllocator::new(hard_ceiling_bytes);
    allocator.begin_request()?;

    let producer = execute_bounded_template_stage_case(
        &generator,
        &identity,
        &stage,
        workload_id,
        &producer_template,
        graph_profile,
        n,
        &plan,
        allocator.clone(),
    )?;
    let oracle = execute_independent_oracle(
        &generator,
        &identity,
        &oracle_template,
        graph_profile,
        n,
        &plan,
        allocator.clone(),
    )?;
    let complete_typed_output_equal = complete_output_equal(&producer, &oracle);
    if !complete_typed_output_equal {
        return Err(BoundedOracleError::CompleteOutputMismatch {
            workload_id,
            graph_profile,
            n,
        });
    }
    let oracle_digest = lower_hex(&Sha256::digest(oracle.stream.as_slice()));
    let producer_digest = finalize_bounded_template_stage_case(producer)?;
    if producer_digest != oracle_digest {
        return Err(BoundedOracleError::DigestMismatch {
            workload_id,
            graph_profile,
            n,
        });
    }
    drop(oracle);
    let allocation = allocator.observation();
    Ok(BoundedOracleVerification {
        primary_record_count: plan.primary_record_count,
        semantic_digest_sha256: oracle_digest,
        complete_counts_equal: true,
        complete_typed_output_equal,
        allocation,
    })
}

fn templates_and_plan(
    trusted: &TrustedContract,
    identity: &IdentityContract,
    workload_id: ScalableWorkloadId,
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<(CorridorTemplate, CorridorTemplate, ScalableStagePlanSummary), BoundedOracleError> {
    match workload_id {
        ScalableWorkloadId::Identity => {
            let template = build_identity_template(identity);
            let plan =
                ScalableStagePlanFactory::from_trusted_contract_for_workload(trusted, workload_id)?
                    .plan(workload_id, graph_profile, n)?;
            Ok((template.clone(), template, plan))
        }
        ScalableWorkloadId::Corridor => {
            let contract = CorridorContract::from_manifest(&trusted.workload_manifest)?;
            let template = contract.load_template(&crate::repository_root())?;
            let plan = ScalableStagePlanFactory::from_trusted_contract_for_template_workload(
                trusted,
                workload_id,
                &template,
            )?
            .plan(workload_id, graph_profile, n)?;
            Ok((template.clone(), template, plan))
        }
        ScalableWorkloadId::JunctionGrid => {
            let contract = JunctionGridContract::from_manifest(&trusted.workload_manifest)?;
            let producer_template = build_junction_grid_template();
            let oracle_template = build_independent_template();
            contract.validate_template(&producer_template)?;
            contract.validate_template(&oracle_template)?;
            let plan = ScalableStagePlanFactory::from_trusted_contract_for_template_workload(
                trusted,
                workload_id,
                &producer_template,
            )?
            .plan(workload_id, graph_profile, n)?;
            Ok((producer_template, oracle_template, plan))
        }
    }
}

fn build_identity_template(identity: &IdentityContract) -> CorridorTemplate {
    let entities = identity
        .bindings
        .iter()
        .map(|binding| {
            let identity_references = binding
                .fields
                .iter()
                .filter_map(|field| match field.value {
                    IdentityFieldValue::StableId { kind, .. } => {
                        Some((field.tag, EntityRef { kind, local: 0 }))
                    }
                    IdentityFieldValue::Namespace | IdentityFieldValue::ProfiledKey { .. } => None,
                })
                .collect::<BTreeMap<_, _>>();
            TemplateEntity {
                reference: EntityRef {
                    kind: binding.entity_kind_code,
                    local: 0,
                },
                identity_references,
            }
        })
        .collect();
    let relations = identity
        .owner_relations
        .iter()
        .map(|relation| TemplateRelation::Owner {
            child: EntityRef {
                kind: relation.child_kind,
                local: 0,
            },
            parent: EntityRef {
                kind: relation.parent_kind,
                local: 0,
            },
        })
        .collect();
    CorridorTemplate {
        entities,
        relations,
        geometry: Vec::new(),
    }
}

fn execute_independent_oracle(
    generator: &GeneratorContract,
    identity: &IdentityContract,
    template: &CorridorTemplate,
    graph_profile: GraphProfileId,
    n: u32,
    plan: &ScalableStagePlanSummary,
    allocator: ControlledAllocator,
) -> Result<BoundedOracleOutput, StageGenerationError> {
    if n == 0 {
        return Err(StageGenerationError::ScaleMustBePositive);
    }
    let declaration_count = to_usize(
        plan.counts.identity_declaration_count,
        "oracle declaration count",
    )?;
    let identity_payload_capacity = identity_payload_capacity_per_unit(identity, template)?
        .checked_mul(to_usize(u64::from(n), "oracle scale")?)
        .ok_or(StageGenerationError::Overflow(
            "oracle identity payload capacity",
        ))?;
    let mut identity_payload = ControlledVec::try_with_capacity(
        "oracle identity payload",
        identity_payload_capacity,
        allocator.clone(),
    )?;
    let mut declarations: ControlledVec<OracleDeclaration> = ControlledVec::try_with_capacity(
        "oracle declarations",
        declaration_count,
        allocator.clone(),
    )?;
    let mut canonical = ControlledVec::try_with_capacity(
        "oracle canonical identity scratch",
        maximum_identity_bytes(identity)?,
        allocator.clone(),
    )?;

    for unit in 0..n {
        let namespace = oracle_namespace(generator, plan.workload_id, graph_profile, unit)?;
        for entity in &template.entities {
            let binding = identity
                .bindings
                .iter()
                .find(|binding| binding.entity_kind_code == entity.reference.kind)
                .ok_or(StageGenerationError::MissingEntityKind(
                    entity.reference.kind,
                ))?;
            let profiled_count = u32::try_from(
                binding
                    .fields
                    .iter()
                    .filter(|field| matches!(field.value, IdentityFieldValue::ProfiledKey { .. }))
                    .count(),
            )
            .map_err(|_| StageGenerationError::Overflow("oracle profiled field count"))?;
            let payload_start = identity_payload.len();
            append_u16(
                &mut identity_payload,
                u16::try_from(binding.fields.len())
                    .map_err(|_| StageGenerationError::Overflow("oracle identity field count"))?,
            )?;
            for field in &binding.fields {
                append_u16(&mut identity_payload, field.tag)?;
                match field.value {
                    IdentityFieldValue::Namespace => {
                        append_u32(&mut identity_payload, 32)?;
                        identity_payload.try_extend_from_slice(&namespace)?;
                    }
                    IdentityFieldValue::ProfiledKey { kind, local } => {
                        append_u32(&mut identity_payload, 20)?;
                        let expanded_local = entity
                            .reference
                            .local
                            .checked_mul(profiled_count)
                            .and_then(|base| base.checked_add(local))
                            .ok_or(StageGenerationError::Overflow("oracle profiled key local"))?;
                        let mut key = [0_u8; 20];
                        write_hex_u16(kind, &mut key[0..2]);
                        key[2] = b'/';
                        write_hex_u32(unit, &mut key[3..11]);
                        key[11] = b'/';
                        write_hex_u32(expanded_local, &mut key[12..20]);
                        identity_payload.try_extend_from_slice(&key)?;
                    }
                    IdentityFieldValue::StableId { kind, .. } => {
                        append_u32(&mut identity_payload, 16)?;
                        let target = entity.identity_references.get(&field.tag).copied().ok_or(
                            StageGenerationError::MaterializedMismatch("oracle identity reference"),
                        )?;
                        if target.kind != kind {
                            return Err(StageGenerationError::MaterializedMismatch(
                                "oracle identity reference kind",
                            ));
                        }
                        let target =
                            oracle_declaration(template, declarations.as_slice(), unit, target)?;
                        identity_payload.try_extend_from_slice(&target.stable_id)?;
                    }
                }
            }
            let payload = PayloadRange {
                offset: payload_start,
                length: identity_payload.len() - payload_start,
            };
            canonical.clear();
            canonical.try_extend_from_slice(IDENTITY_MAGIC)?;
            append_u16(&mut canonical, identity.identity_encoding_version())?;
            append_u16(&mut canonical, binding.entity_kind_code)?;
            canonical.try_extend_from_slice(
                &identity_payload.as_slice()[payload.offset..payload.offset + payload.length],
            )?;
            let mut hasher = blake3::Hasher::new();
            hasher.update(STABLE_ID_DOMAIN);
            hasher.update(canonical.as_slice());
            let mut stable_id = [0_u8; 16];
            stable_id.copy_from_slice(&hasher.finalize().as_bytes()[..16]);
            declarations.try_push(OracleDeclaration {
                owner: UnitEntityRef {
                    unit,
                    entity: entity.reference,
                },
                stable_id,
                owner_ordinal: 0,
                identity_payload: payload,
            })?;
        }
    }
    if declarations.len() != declaration_count
        || identity_payload.len() != identity_payload_capacity
    {
        return Err(StageGenerationError::MaterializedMismatch(
            "oracle declarations",
        ));
    }
    assign_oracle_owner_ordinals(&mut declarations, allocator.clone())?;

    let record_count = to_usize(
        plan.counts.semantic_output_record,
        "oracle semantic record count",
    )?;
    let payload_capacity = to_usize(
        plan.counts.semantic_payload_byte_count,
        "oracle semantic payload bytes",
    )?;
    let mut records = ControlledVec::try_with_capacity(
        "oracle semantic records",
        record_count,
        allocator.clone(),
    )?;
    let mut payload = ControlledVec::try_with_capacity(
        "oracle semantic payload",
        payload_capacity,
        allocator.clone(),
    )?;
    let mut local_scratch = ControlledVec::try_with_capacity(
        "oracle local-index scratch",
        template
            .entities
            .len()
            .checked_add(template.relations.len())
            .and_then(|value| value.checked_add(template.geometry.len()))
            .ok_or(StageGenerationError::Overflow("oracle local-index scratch"))?,
        allocator.clone(),
    )?;

    for unit in 0..n {
        let unit_start = records.len();
        let declaration_start = usize::try_from(unit)
            .ok()
            .and_then(|unit| unit.checked_mul(template.entities.len()))
            .ok_or(StageGenerationError::Overflow(
                "oracle unit declaration start",
            ))?;
        let declaration_end = declaration_start
            .checked_add(template.entities.len())
            .ok_or(StageGenerationError::Overflow(
                "oracle unit declaration end",
            ))?;
        let unit_declarations = declarations
            .as_slice()
            .get(declaration_start..declaration_end)
            .ok_or(StageGenerationError::MaterializedMismatch(
                "oracle unit declarations",
            ))?;
        for declaration in unit_declarations {
            let start = payload.len();
            let source = &identity_payload.as_slice()[declaration.identity_payload.offset
                ..declaration.identity_payload.offset + declaration.identity_payload.length];
            payload.try_extend_from_slice(source)?;
            records.try_push(OracleRecord {
                record_kind: 1,
                entity_kind_code: declaration.owner.entity.kind,
                stable_id: declaration.stable_id,
                owner_ordinal: declaration.owner_ordinal,
                local_index: ABSENT_LOCAL_INDEX,
                payload: PayloadRange {
                    offset: start,
                    length: source.len(),
                },
                local_order: OracleLocalOrder::Absent,
            })?;
        }
        for relation in &template.relations {
            records.try_push(oracle_relation(
                template,
                declarations.as_slice(),
                unit,
                relation,
                &mut payload,
            )?)?;
        }
        for point in &template.geometry {
            let start = payload.len();
            payload.try_extend_from_slice(&oracle_stable_id(
                template,
                declarations.as_slice(),
                unit,
                point.frame,
            )?)?;
            append_u32(&mut payload, point.point_index)?;
            let (x, y, z) = oracle_geometry(point, unit)?;
            append_u32(&mut payload, x)?;
            append_u32(&mut payload, y)?;
            append_u32(&mut payload, z)?;
            let owner = oracle_declaration(template, declarations.as_slice(), unit, point.edge)?;
            records.try_push(OracleRecord {
                record_kind: 5,
                entity_kind_code: point.edge.kind,
                stable_id: owner.stable_id,
                owner_ordinal: owner.owner_ordinal,
                local_index: point.point_index,
                payload: PayloadRange {
                    offset: start,
                    length: payload.len() - start,
                },
                local_order: OracleLocalOrder::Explicit(point.point_index),
            })?;
        }
        let unit_end = records.len();
        assign_oracle_local_indexes(
            records.as_mut_slice(),
            payload.as_mut_slice(),
            unit_start,
            unit_end,
            &mut local_scratch,
        )?;
    }
    if records.len() != record_count || payload.len() != payload_capacity {
        return Err(StageGenerationError::MaterializedMismatch(
            "oracle semantic shape",
        ));
    }
    records.sort_unstable_by(|left, right| oracle_record_compare(left, right, payload.as_slice()));

    let stream_bytes = to_usize(plan.counts.output_byte_count, "oracle stream bytes")?;
    let mut stream =
        ControlledVec::try_with_capacity("oracle semantic stream", stream_bytes, allocator)?;
    stream.try_extend_from_slice(identity.semantic_record_domain().as_bytes())?;
    stream.try_push(0)?;
    append_u32(&mut stream, identity.semantic_record_stream_version())?;
    append_u64(
        &mut stream,
        u64::try_from(records.len())
            .map_err(|_| StageGenerationError::Overflow("oracle stream record count"))?,
    )?;
    for record in &records {
        append_u16(&mut stream, record.record_kind)?;
        append_u16(&mut stream, record.entity_kind_code)?;
        stream.try_extend_from_slice(&record.stable_id)?;
        append_u32(&mut stream, record.owner_ordinal)?;
        append_u32(&mut stream, record.local_index)?;
        append_u64(
            &mut stream,
            u64::try_from(record.payload.length)
                .map_err(|_| StageGenerationError::Overflow("oracle record payload length"))?,
        )?;
        stream.try_extend_from_slice(oracle_payload(record, payload.as_slice()))?;
    }
    if stream.len() != stream_bytes {
        return Err(StageGenerationError::MaterializedMismatch(
            "oracle semantic stream",
        ));
    }
    Ok(BoundedOracleOutput {
        records,
        payload,
        stream,
    })
}

fn identity_payload_capacity_per_unit(
    identity: &IdentityContract,
    template: &CorridorTemplate,
) -> Result<usize, StageGenerationError> {
    template.entities.iter().try_fold(0_usize, |total, entity| {
        let binding = identity
            .bindings
            .iter()
            .find(|binding| binding.entity_kind_code == entity.reference.kind)
            .ok_or(StageGenerationError::MissingEntityKind(
                entity.reference.kind,
            ))?;
        binding.fields.iter().try_fold(
            total
                .checked_add(2)
                .ok_or(StageGenerationError::Overflow("oracle identity payload"))?,
            |binding_total, field| {
                let bytes = match field.value {
                    IdentityFieldValue::Namespace => 32,
                    IdentityFieldValue::ProfiledKey { .. } => 20,
                    IdentityFieldValue::StableId { .. } => 16,
                };
                binding_total
                    .checked_add(6 + bytes)
                    .ok_or(StageGenerationError::Overflow("oracle identity payload"))
            },
        )
    })
}

fn maximum_identity_bytes(identity: &IdentityContract) -> Result<usize, StageGenerationError> {
    identity
        .bindings
        .iter()
        .try_fold(10_usize, |maximum, binding| {
            let payload = binding.fields.iter().try_fold(2_usize, |total, field| {
                let bytes = match field.value {
                    IdentityFieldValue::Namespace => 32,
                    IdentityFieldValue::ProfiledKey { .. } => 20,
                    IdentityFieldValue::StableId { .. } => 16,
                };
                total
                    .checked_add(6 + bytes)
                    .ok_or(StageGenerationError::Overflow("oracle canonical identity"))
            })?;
            Ok(maximum.max(
                8_usize
                    .checked_add(payload)
                    .ok_or(StageGenerationError::Overflow("oracle canonical identity"))?,
            ))
        })
}

fn assign_oracle_owner_ordinals(
    declarations: &mut ControlledVec<OracleDeclaration>,
    allocator: ControlledAllocator,
) -> Result<(), StageGenerationError> {
    let mut indexes = ControlledVec::try_with_capacity(
        "oracle owner-ordinal scratch",
        declarations.len(),
        allocator,
    )?;
    for index in 0..declarations.len() {
        indexes.try_push(index)?;
    }
    indexes.sort_unstable_by_key(|index| {
        (
            declarations[*index].owner.entity.kind,
            declarations[*index].stable_id,
        )
    });
    let mut previous_kind = None;
    let mut ordinal = 0_u32;
    for index in indexes.iter().copied() {
        let kind = declarations[index].owner.entity.kind;
        if previous_kind != Some(kind) {
            previous_kind = Some(kind);
            ordinal = 0;
        }
        declarations[index].owner_ordinal = ordinal;
        ordinal = ordinal
            .checked_add(1)
            .ok_or(StageGenerationError::Overflow("oracle owner ordinal"))?;
    }
    Ok(())
}

fn oracle_relation(
    template: &CorridorTemplate,
    declarations: &[OracleDeclaration],
    unit: u32,
    relation: &TemplateRelation,
    payload: &mut ControlledVec<u8>,
) -> Result<OracleRecord, StageGenerationError> {
    let start = payload.len();
    let (record_kind, owner, local_order) = match relation {
        TemplateRelation::Owner { child, parent } => {
            append_u16(payload, parent.kind)?;
            payload.try_extend_from_slice(&oracle_stable_id(
                template,
                declarations,
                unit,
                *parent,
            )?)?;
            (2, *child, OracleLocalOrder::Absent)
        }
        TemplateRelation::EdgeConnection { source, target } => {
            payload.try_extend_from_slice(&oracle_stable_id(
                template,
                declarations,
                unit,
                *target,
            )?)?;
            (3, *source, OracleLocalOrder::Payload)
        }
        TemplateRelation::RouteOccurrence { route, index, edge } => {
            append_u32(payload, *index)?;
            payload.try_extend_from_slice(&oracle_stable_id(
                template,
                declarations,
                unit,
                *edge,
            )?)?;
            (4, *route, OracleLocalOrder::Explicit(*index))
        }
        TemplateRelation::Access {
            rule,
            participant,
            target,
            decision,
        } => {
            payload.try_extend_from_slice(&oracle_stable_id(
                template,
                declarations,
                unit,
                *participant,
            )?)?;
            append_u16(payload, target.kind)?;
            payload.try_extend_from_slice(&oracle_stable_id(
                template,
                declarations,
                unit,
                *target,
            )?)?;
            payload.try_push(*decision)?;
            (6, *rule, OracleLocalOrder::Payload)
        }
        TemplateRelation::SignalGroup { group, gate } => {
            payload.try_extend_from_slice(&oracle_stable_id(
                template,
                declarations,
                unit,
                *gate,
            )?)?;
            (7, *group, OracleLocalOrder::Payload)
        }
        TemplateRelation::PhaseState {
            phase,
            group,
            state,
        } => {
            payload.try_extend_from_slice(&oracle_stable_id(
                template,
                declarations,
                unit,
                *group,
            )?)?;
            payload.try_push(*state)?;
            (8, *phase, OracleLocalOrder::Payload)
        }
        TemplateRelation::Gate {
            path,
            transition_index,
            gate,
            stop_line,
            edge,
            edge_position_bits,
        } => {
            let gate_id = oracle_stable_id(template, declarations, unit, *gate)?;
            append_u32(payload, 0)?;
            payload.try_extend_from_slice(&gate_id)?;
            payload.try_extend_from_slice(&oracle_stable_id(
                template,
                declarations,
                unit,
                *stop_line,
            )?)?;
            payload.try_extend_from_slice(&oracle_stable_id(
                template,
                declarations,
                unit,
                *edge,
            )?)?;
            append_u32(payload, *edge_position_bits)?;
            (
                9,
                *path,
                OracleLocalOrder::Gate {
                    transition_index: *transition_index,
                    stable_id: gate_id,
                },
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
            let zone_id = oracle_stable_id(template, declarations, unit, *zone)?;
            append_u32(payload, 0)?;
            payload.try_extend_from_slice(&zone_id)?;
            payload.try_extend_from_slice(&oracle_stable_id(
                template,
                declarations,
                unit,
                *before_gate,
            )?)?;
            payload.try_extend_from_slice(&oracle_stable_id(
                template,
                declarations,
                unit,
                *after_gate,
            )?)?;
            append_u32(payload, *capacity)?;
            (
                10,
                *path,
                OracleLocalOrder::Waiting {
                    entry_transition_index: *entry_transition_index,
                    release_transition_index: *release_transition_index,
                    stable_id: zone_id,
                },
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
            payload.try_extend_from_slice(&oracle_stable_id(
                template,
                declarations,
                unit,
                *space,
            )?)?;
            payload.try_extend_from_slice(&oracle_stable_id(
                template,
                declarations,
                unit,
                *entry_edge,
            )?)?;
            append_u32(payload, *entry_high_bits)?;
            append_u32(payload, *entry_residual_bits)?;
            payload.try_extend_from_slice(&oracle_stable_id(
                template,
                declarations,
                unit,
                *exit_edge,
            )?)?;
            append_u32(payload, *exit_high_bits)?;
            append_u32(payload, *exit_residual_bits)?;
            (11, *space, OracleLocalOrder::Absent)
        }
        TemplateRelation::LaneCoverage { lane, index, edge } => {
            append_u32(payload, *index)?;
            payload.try_extend_from_slice(&oracle_stable_id(
                template,
                declarations,
                unit,
                *edge,
            )?)?;
            (12, *lane, OracleLocalOrder::Explicit(*index))
        }
        TemplateRelation::JunctionInternalEdge { junction, edge } => {
            payload.try_extend_from_slice(&oracle_stable_id(
                template,
                declarations,
                unit,
                *edge,
            )?)?;
            (13, *junction, OracleLocalOrder::Payload)
        }
    };
    let declaration = oracle_declaration(template, declarations, unit, owner)?;
    Ok(OracleRecord {
        record_kind,
        entity_kind_code: owner.kind,
        stable_id: declaration.stable_id,
        owner_ordinal: declaration.owner_ordinal,
        local_index: match local_order {
            OracleLocalOrder::Absent => ABSENT_LOCAL_INDEX,
            OracleLocalOrder::Explicit(index) => index,
            OracleLocalOrder::Payload
            | OracleLocalOrder::Gate { .. }
            | OracleLocalOrder::Waiting { .. } => 0,
        },
        payload: PayloadRange {
            offset: start,
            length: payload.len() - start,
        },
        local_order,
    })
}

fn assign_oracle_local_indexes(
    records: &mut [OracleRecord],
    payload: &mut [u8],
    start: usize,
    end: usize,
    scratch: &mut ControlledVec<usize>,
) -> Result<(), StageGenerationError> {
    for anchor in start..end {
        if matches!(
            records[anchor].local_order,
            OracleLocalOrder::Absent | OracleLocalOrder::Explicit(_)
        ) || (start..anchor)
            .any(|candidate| oracle_same_group(&records[candidate], &records[anchor]))
        {
            continue;
        }
        scratch.clear();
        for candidate in start..end {
            if oracle_same_group(&records[candidate], &records[anchor]) {
                scratch.try_push(candidate)?;
            }
        }
        match records[anchor].local_order {
            OracleLocalOrder::Payload => scratch.sort_unstable_by(|left, right| {
                oracle_payload(&records[*left], payload)
                    .cmp(oracle_payload(&records[*right], payload))
                    .then_with(|| records[*left].stable_id.cmp(&records[*right].stable_id))
            }),
            OracleLocalOrder::Gate { .. } => {
                scratch.sort_unstable_by_key(|candidate| match records[*candidate].local_order {
                    OracleLocalOrder::Gate {
                        transition_index,
                        stable_id,
                    } => (transition_index, stable_id),
                    _ => unreachable!("oracle gate group"),
                })
            }
            OracleLocalOrder::Waiting { .. } => {
                scratch.sort_unstable_by_key(|candidate| match records[*candidate].local_order {
                    OracleLocalOrder::Waiting {
                        entry_transition_index,
                        release_transition_index,
                        stable_id,
                    } => (entry_transition_index, release_transition_index, stable_id),
                    _ => unreachable!("oracle waiting group"),
                })
            }
            OracleLocalOrder::Absent | OracleLocalOrder::Explicit(_) => {
                unreachable!("filtered oracle group")
            }
        }
        for (ordinal, candidate) in scratch.iter().copied().enumerate() {
            let ordinal = u32::try_from(ordinal)
                .map_err(|_| StageGenerationError::Overflow("oracle local index"))?;
            if matches!(
                records[candidate].local_order,
                OracleLocalOrder::Gate { .. } | OracleLocalOrder::Waiting { .. }
            ) {
                let offset = records[candidate].payload.offset;
                payload[offset..offset + 4].copy_from_slice(&ordinal.to_le_bytes());
            }
            records[candidate].local_index = ordinal;
            records[candidate].local_order = OracleLocalOrder::Explicit(ordinal);
        }
    }
    Ok(())
}

fn oracle_same_group(left: &OracleRecord, right: &OracleRecord) -> bool {
    match (left.local_order, right.local_order) {
        (OracleLocalOrder::Payload, OracleLocalOrder::Payload) => {
            left.record_kind == right.record_kind
                && left.entity_kind_code == right.entity_kind_code
                && left.stable_id == right.stable_id
        }
        (OracleLocalOrder::Gate { .. }, OracleLocalOrder::Gate { .. })
        | (OracleLocalOrder::Waiting { .. }, OracleLocalOrder::Waiting { .. }) => {
            left.entity_kind_code == right.entity_kind_code && left.stable_id == right.stable_id
        }
        _ => false,
    }
}

fn oracle_record_compare(left: &OracleRecord, right: &OracleRecord, payload: &[u8]) -> Ordering {
    (
        left.record_kind,
        left.entity_kind_code,
        left.stable_id,
        left.owner_ordinal,
        left.local_index,
        oracle_payload(left, payload),
    )
        .cmp(&(
            right.record_kind,
            right.entity_kind_code,
            right.stable_id,
            right.owner_ordinal,
            right.local_index,
            oracle_payload(right, payload),
        ))
}

fn oracle_payload<'a>(record: &OracleRecord, payload: &'a [u8]) -> &'a [u8] {
    &payload[record.payload.offset..record.payload.offset + record.payload.length]
}

fn oracle_declaration<'a>(
    template: &CorridorTemplate,
    declarations: &'a [OracleDeclaration],
    unit: u32,
    entity: EntityRef,
) -> Result<&'a OracleDeclaration, StageGenerationError> {
    let entity_index = template
        .entities
        .iter()
        .position(|candidate| candidate.reference == entity)
        .ok_or(StageGenerationError::MaterializedMismatch(
            "oracle template entity lookup",
        ))?;
    let declaration_index = usize::try_from(unit)
        .ok()
        .and_then(|unit| unit.checked_mul(template.entities.len()))
        .and_then(|base| base.checked_add(entity_index))
        .ok_or(StageGenerationError::Overflow("oracle declaration lookup"))?;
    let declaration =
        declarations
            .get(declaration_index)
            .ok_or(StageGenerationError::MaterializedMismatch(
                "oracle declaration lookup",
            ))?;
    if declaration.owner.unit != unit || declaration.owner.entity != entity {
        return Err(StageGenerationError::MaterializedMismatch(
            "oracle declaration lookup identity",
        ));
    }
    Ok(declaration)
}

fn oracle_stable_id(
    template: &CorridorTemplate,
    declarations: &[OracleDeclaration],
    unit: u32,
    entity: EntityRef,
) -> Result<[u8; 16], StageGenerationError> {
    Ok(oracle_declaration(template, declarations, unit, entity)?.stable_id)
}

fn oracle_geometry(
    point: &crate::corridor::TemplateGeometry,
    unit: u32,
) -> Result<(u32, u32, u32), StageGenerationError> {
    if point.coordinate_rule == TemplateGeometryRule::Fixed {
        return Ok((point.x_bits, point.y_bits, point.z_bits));
    }
    let x = (unit % 4_096)
        .checked_mul(128)
        .and_then(|base| {
            point
                .edge
                .local
                .checked_mul(2)
                .and_then(|edge| base.checked_add(edge))
        })
        .and_then(|base| base.checked_add(point.point_index))
        .ok_or(StageGenerationError::Overflow("oracle junction x"))?;
    let y = (unit / 4_096)
        .checked_mul(128)
        .ok_or(StageGenerationError::Overflow("oracle junction y"))?;
    Ok((
        (x as f32).to_bits(),
        (y as f32).to_bits(),
        0.0_f32.to_bits(),
    ))
}

fn oracle_namespace(
    generator: &GeneratorContract,
    workload_id: ScalableWorkloadId,
    graph_profile: GraphProfileId,
    unit: u32,
) -> Result<[u8; 32], StageGenerationError> {
    let mut module_name = [0_u8; 13];
    module_name[..5].copy_from_slice(b"unit/");
    write_hex_u32(unit, &mut module_name[5..13]);
    let mut hasher = blake3::Hasher::new();
    hasher.update(generator.namespace_domain.as_bytes());
    hasher.update(&[0]);
    hasher.update(&generator.generator_version.to_le_bytes());
    hasher.update(&generator.base_seed.to_le_bytes());
    oracle_length_prefixed(&mut hasher, workload_id.as_str().as_bytes())?;
    oracle_length_prefixed(&mut hasher, graph_profile.as_str().as_bytes())?;
    oracle_length_prefixed(&mut hasher, &module_name)?;
    let digest = hasher.finalize();
    let selected = &digest.as_bytes()[generator.namespace_digest_offset
        ..generator.namespace_digest_offset + generator.namespace_digest_length];
    let mut output = [0_u8; 32];
    for (index, byte) in selected.iter().copied().enumerate() {
        output[index * 2] = hex_digit(byte >> 4);
        output[index * 2 + 1] = hex_digit(byte & 0x0f);
    }
    Ok(output)
}

fn oracle_length_prefixed(
    hasher: &mut blake3::Hasher,
    value: &[u8],
) -> Result<(), StageGenerationError> {
    let length = u32::try_from(value.len())
        .map_err(|_| StageGenerationError::Overflow("oracle namespace value"))?;
    hasher.update(&length.to_le_bytes());
    hasher.update(value);
    Ok(())
}

fn complete_output_equal(
    producer: &BoundedTemplateExecution,
    oracle: &BoundedOracleOutput,
) -> bool {
    producer.semantic_record_stream.as_slice() == oracle.stream.as_slice()
        && producer.records.len() == oracle.records.len()
        && producer
            .records
            .iter()
            .zip(&oracle.records)
            .all(|(produced, expected)| {
                produced.record_kind == expected.record_kind
                    && produced.entity_kind_code == expected.entity_kind_code
                    && produced.stable_id == expected.stable_id
                    && produced.owner_ordinal == expected.owner_ordinal
                    && produced.local_index == expected.local_index
                    && producer.record_payload(produced)
                        == oracle_payload(expected, oracle.payload.as_slice())
            })
}

fn append_u16(output: &mut ControlledVec<u8>, value: u16) -> Result<(), StageGenerationError> {
    output.try_extend_from_slice(&value.to_le_bytes())?;
    Ok(())
}

fn append_u32(output: &mut ControlledVec<u8>, value: u32) -> Result<(), StageGenerationError> {
    output.try_extend_from_slice(&value.to_le_bytes())?;
    Ok(())
}

fn append_u64(output: &mut ControlledVec<u8>, value: u64) -> Result<(), StageGenerationError> {
    output.try_extend_from_slice(&value.to_le_bytes())?;
    Ok(())
}

fn write_hex_u16(value: u16, output: &mut [u8]) {
    debug_assert_eq!(output.len(), 2);
    output[0] = hex_digit(((value >> 4) & 0x0f) as u8);
    output[1] = hex_digit((value & 0x0f) as u8);
}

fn write_hex_u32(value: u32, output: &mut [u8]) {
    debug_assert_eq!(output.len(), 8);
    for (index, destination) in output.iter_mut().enumerate() {
        let shift = (7 - index) * 4;
        *destination = hex_digit(((value >> shift) & 0x0f) as u8);
    }
}

fn hex_digit(value: u8) -> u8 {
    match value {
        0..=9 => b'0' + value,
        10..=15 => b'a' + value - 10,
        _ => unreachable!("hex nibble"),
    }
}

fn lower_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

fn to_usize(value: u64, field: &'static str) -> Result<usize, StageGenerationError> {
    usize::try_from(value).map_err(|_| StageGenerationError::Overflow(field))
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum BoundedOracleError {
    #[error(transparent)]
    Manifest(#[from] crate::ManifestContractError),
    #[error(transparent)]
    Identity(#[from] crate::IdentityContractError),
    #[error(transparent)]
    Stage(#[from] crate::StageContractError),
    #[error(transparent)]
    ScalePlan(#[from] crate::ScalePlanError),
    #[error(transparent)]
    Corridor(#[from] crate::CorridorError),
    #[error(transparent)]
    JunctionGrid(#[from] crate::JunctionGridError),
    #[error(transparent)]
    Generation(#[from] StageGenerationError),
    #[error(
        "受控生产者与独立预言机完整输出不一致：workload={workload_id:?}, graphProfile={graph_profile:?}, N={n}"
    )]
    CompleteOutputMismatch {
        workload_id: ScalableWorkloadId,
        graph_profile: GraphProfileId,
        n: u32,
    },
    #[error(
        "受控生产者与独立预言机摘要不一致：workload={workload_id:?}, graphProfile={graph_profile:?}, N={n}"
    )]
    DigestMismatch {
        workload_id: ScalableWorkloadId,
        graph_profile: GraphProfileId,
        n: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_repository_contract;

    #[test]
    fn bounded_oracle_matches_all_three_producers_at_small_scales() {
        let trusted = load_repository_contract().expect("frozen contract");
        for workload_id in ScalableWorkloadId::ALL {
            for graph_profile in GraphProfileId::ALL {
                for n in [1, 2] {
                    let result = verify_bounded_scalable_oracle(
                        &trusted,
                        workload_id,
                        graph_profile,
                        n,
                        u64::MAX,
                    )
                    .unwrap_or_else(|error| {
                        panic!(
                            "{}/{}/{n}: {error}",
                            workload_id.as_str(),
                            graph_profile.as_str()
                        )
                    });
                    assert!(result.complete_counts_equal);
                    assert!(result.complete_typed_output_equal);
                    assert!(result.allocation.peak_live_requested_bytes > 0);
                    assert_eq!(result.allocation.live_requested_bytes, 0);
                }
            }
        }
    }

    #[test]
    fn bounded_oracle_enforces_actual_peak_at_exact_boundary() {
        let trusted = load_repository_contract().expect("frozen contract");
        for workload_id in ScalableWorkloadId::ALL {
            let baseline = verify_bounded_scalable_oracle(
                &trusted,
                workload_id,
                GraphProfileId::WideStar,
                1,
                u64::MAX,
            )
            .expect("unbounded baseline");
            let peak = baseline.allocation.peak_live_requested_bytes;
            assert!(peak > 0);

            let exact = verify_bounded_scalable_oracle(
                &trusted,
                workload_id,
                GraphProfileId::WideStar,
                1,
                peak,
            )
            .expect("exact peak must succeed");
            assert_eq!(exact.allocation.peak_live_requested_bytes, peak);

            for _ in 0..2 {
                assert!(matches!(
                    verify_bounded_scalable_oracle(
                        &trusted,
                        workload_id,
                        GraphProfileId::WideStar,
                        1,
                        peak - 1,
                    ),
                    Err(BoundedOracleError::Generation(
                        StageGenerationError::ControlledAllocationHardCeiling { .. }
                    ))
                ));
            }
        }
    }
}
