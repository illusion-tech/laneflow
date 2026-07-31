//! 模板型工作负载的安全有界正式执行路径。
//!
//! 该路径不接受一个与实际容器脱节的总量估算。模块图暂存、声明、字段载荷、待排序
//! 记录、规范记录、阶段值/载荷、排序暂存和最终输出都使用 `ControlledVec`；任何新增
//! 容量路径若未获得硬上限额度，就无法通过可失败增长接口继续执行。

use crate::controlled_alloc::{
    ControlledAllocationObservation, ControlledAllocator, ControlledVec,
};
use crate::corridor::{
    CorridorTemplate, EntityRef, TemplateGeometryRule, TemplateRelation, UnitEntityRef,
};
use crate::identity::{IdentityContract, IdentityFieldValue};
use crate::stage::{
    HirStageRecord, MirLirStageRecord, SourceSpanRecord, StageContract, TypedAstStageRecord,
};
use crate::{
    GeneratorContract, GraphProfileId, ScalableStagePlanSummary, ScalableWorkloadId, SequenceKind,
    StageGenerationError, permute_in_place,
};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;

const IDENTITY_MAGIC: &[u8; 4] = b"LFID";
const STABLE_ID_DOMAIN: &[u8] = b"laneflow.stable-id.v1\0";
const ABSENT_LOCAL_INDEX: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PayloadRange {
    pub(crate) offset: usize,
    pub(crate) length: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BoundedField {
    tag: u16,
    bytes: PayloadRange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BoundedDeclaration {
    owner: UnitEntityRef,
    stable_id: [u8; 16],
    owner_ordinal: u32,
    fields: PayloadRange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingOrder {
    Absent,
    Explicit(u32),
    OwnerPayload,
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
pub(crate) struct BoundedSemanticRecord {
    pub(crate) record_kind: u16,
    pub(crate) entity_kind_code: u16,
    pub(crate) stable_id: [u8; 16],
    pub(crate) owner_ordinal: u32,
    pub(crate) local_index: u32,
    pub(crate) payload: PayloadRange,
    pending_order: PendingOrder,
}

#[derive(Debug)]
pub(crate) struct BoundedTemplateExecution {
    pub(crate) workload_id: ScalableWorkloadId,
    pub(crate) graph_profile: GraphProfileId,
    pub(crate) n: u32,
    pub(crate) records: ControlledVec<BoundedSemanticRecord>,
    pub(crate) record_payload: ControlledVec<u8>,
    pub(crate) semantic_record_stream: ControlledVec<u8>,
    materialization: BoundedTemplateMaterialization,
    plan: ScalableStagePlanSummary,
}

#[derive(Debug)]
struct BoundedTemplateMaterialization {
    source_spans: ControlledVec<SourceSpanRecord>,
    source_records: ControlledVec<TypedAstStageRecord>,
    source_payload: ControlledVec<u8>,
    typed_records: ControlledVec<TypedAstStageRecord>,
    typed_payload: ControlledVec<u8>,
    hir_records: ControlledVec<HirStageRecord>,
    hir_payload: ControlledVec<u8>,
    mir_records: ControlledVec<MirLirStageRecord>,
    mir_payload: ControlledVec<u8>,
    lir_records: ControlledVec<MirLirStageRecord>,
    lir_payload: ControlledVec<u8>,
    diagnostics: ControlledVec<u8>,
    scratch: ControlledVec<u64>,
    output: ControlledVec<u8>,
}

#[derive(Debug)]
struct BoundedTemplateMaterializationPrefix {
    source_spans: ControlledVec<SourceSpanRecord>,
    source_records: ControlledVec<TypedAstStageRecord>,
    source_payload: ControlledVec<u8>,
    typed_records: ControlledVec<TypedAstStageRecord>,
    typed_payload: ControlledVec<u8>,
    hir_records: ControlledVec<HirStageRecord>,
    hir_payload: ControlledVec<u8>,
    mir_records: ControlledVec<MirLirStageRecord>,
    mir_payload: ControlledVec<u8>,
    diagnostics: ControlledVec<u8>,
    scratch: ControlledVec<u64>,
}

impl BoundedTemplateExecution {
    pub(crate) fn output_construction(&self) -> &[u8] {
        self.materialization.output.as_slice()
    }

    pub(crate) fn record_payload(&self, record: &BoundedSemanticRecord) -> &[u8] {
        &self.record_payload.as_slice()
            [record.payload.offset..record.payload.offset + record.payload.length]
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_bounded_template_stage_case(
    generator: &GeneratorContract,
    identity: &IdentityContract,
    stage: &StageContract,
    workload_id: ScalableWorkloadId,
    template: &CorridorTemplate,
    graph_profile: GraphProfileId,
    n: u32,
    plan: &ScalableStagePlanSummary,
    allocator: ControlledAllocator,
) -> Result<BoundedTemplateExecution, StageGenerationError> {
    if n == 0 {
        return Err(StageGenerationError::ScaleMustBePositive);
    }
    if plan.workload_id != workload_id || plan.graph_profile != graph_profile || plan.n != n {
        return Err(StageGenerationError::MaterializedMismatch(
            "bounded template plan identity",
        ));
    }

    let entity_count = template.entities.len();
    let declaration_count = usize::try_from(plan.counts.identity_declaration_count)
        .map_err(|_| StageGenerationError::Overflow("bounded declaration count"))?;
    let field_count = usize::try_from(plan.counts.identity_field_occurrence_count)
        .map_err(|_| StageGenerationError::Overflow("bounded field count"))?;
    let semantic_record_count = usize::try_from(plan.counts.semantic_output_record)
        .map_err(|_| StageGenerationError::Overflow("bounded semantic record count"))?;
    let semantic_payload_bytes = usize::try_from(plan.counts.semantic_payload_byte_count)
        .map_err(|_| StageGenerationError::Overflow("bounded semantic payload bytes"))?;

    let mut fields = ControlledVec::try_with_capacity(
        "template identity fields",
        field_count,
        allocator.clone(),
    )?;
    let mut field_payload = ControlledVec::try_with_capacity(
        "template identity field payload",
        checked_mul_usize(field_count, 32, "bounded identity field payload upper")?,
        allocator.clone(),
    )?;
    let mut declarations: ControlledVec<BoundedDeclaration> = ControlledVec::try_with_capacity(
        "template declarations",
        declaration_count,
        allocator.clone(),
    )?;
    let mut canonical_identity_scratch = ControlledVec::try_with_capacity(
        "template canonical identity scratch",
        maximum_canonical_identity_bytes(identity, template)?,
        allocator.clone(),
    )?;

    for unit in 0..n {
        let namespace =
            derive_namespace_ascii(generator, workload_id.as_str(), graph_profile, unit)?;
        for entity in &template.entities {
            let binding = identity
                .bindings
                .iter()
                .find(|binding| binding.entity_kind_code == entity.reference.kind)
                .ok_or(StageGenerationError::MissingEntityKind(
                    entity.reference.kind,
                ))?;
            let field_start = fields.len();
            let profiled_count = u32::try_from(
                binding
                    .fields
                    .iter()
                    .filter(|field| matches!(field.value, IdentityFieldValue::ProfiledKey { .. }))
                    .count(),
            )
            .map_err(|_| StageGenerationError::Overflow("profiled identity field count"))?;
            for field in &binding.fields {
                let payload_start = field_payload.len();
                match field.value {
                    IdentityFieldValue::Namespace => {
                        field_payload.try_extend_from_slice(&namespace)?;
                    }
                    IdentityFieldValue::ProfiledKey { kind, local } => {
                        let expanded_local = entity
                            .reference
                            .local
                            .checked_mul(profiled_count)
                            .and_then(|base| base.checked_add(local))
                            .ok_or(StageGenerationError::Overflow(
                                "profiled identity local index",
                            ))?;
                        let mut bytes = [0_u8; 20];
                        write_hex_u16(kind, &mut bytes[0..2]);
                        bytes[2] = b'/';
                        write_hex_u32(unit, &mut bytes[3..11]);
                        bytes[11] = b'/';
                        write_hex_u32(expanded_local, &mut bytes[12..20]);
                        field_payload.try_extend_from_slice(&bytes)?;
                    }
                    IdentityFieldValue::StableId { kind, .. } => {
                        let target = entity.identity_references.get(&field.tag).copied().ok_or(
                            StageGenerationError::MaterializedMismatch(
                                "template identity reference tag",
                            ),
                        )?;
                        if target.kind != kind {
                            return Err(StageGenerationError::MaterializedMismatch(
                                "template identity reference kind",
                            ));
                        }
                        let target_index = declaration_index(template, unit, target, entity_count)?;
                        let target_declaration = declarations.as_slice().get(target_index).ok_or(
                            StageGenerationError::MaterializedMismatch(
                                "template identity dependency order",
                            ),
                        )?;
                        field_payload.try_extend_from_slice(&target_declaration.stable_id)?;
                    }
                }
                fields.try_push(BoundedField {
                    tag: field.tag,
                    bytes: PayloadRange {
                        offset: payload_start,
                        length: field_payload.len() - payload_start,
                    },
                })?;
            }

            canonical_identity_scratch.clear();
            canonical_identity_scratch.try_extend_from_slice(IDENTITY_MAGIC)?;
            append_u16(
                &mut canonical_identity_scratch,
                identity.identity_encoding_version(),
            )?;
            append_u16(&mut canonical_identity_scratch, entity.reference.kind)?;
            let entity_fields = &fields.as_slice()[field_start..fields.len()];
            append_u16(
                &mut canonical_identity_scratch,
                u16::try_from(entity_fields.len())
                    .map_err(|_| StageGenerationError::Overflow("identity field count"))?,
            )?;
            for field in entity_fields {
                append_u16(&mut canonical_identity_scratch, field.tag)?;
                append_u32(
                    &mut canonical_identity_scratch,
                    u32::try_from(field.bytes.length)
                        .map_err(|_| StageGenerationError::Overflow("identity field length"))?,
                )?;
                canonical_identity_scratch.try_extend_from_slice(
                    &field_payload.as_slice()
                        [field.bytes.offset..field.bytes.offset + field.bytes.length],
                )?;
            }
            let mut hasher = blake3::Hasher::new();
            hasher.update(STABLE_ID_DOMAIN);
            hasher.update(canonical_identity_scratch.as_slice());
            let mut stable_id = [0_u8; 16];
            stable_id.copy_from_slice(&hasher.finalize().as_bytes()[..16]);
            declarations.try_push(BoundedDeclaration {
                owner: UnitEntityRef {
                    unit,
                    entity: entity.reference,
                },
                stable_id,
                owner_ordinal: 0,
                fields: PayloadRange {
                    offset: field_start,
                    length: entity_fields.len(),
                },
            })?;
        }
    }
    if declarations.len() != declaration_count || fields.len() != field_count {
        return Err(StageGenerationError::MaterializedMismatch(
            "bounded template declarations",
        ));
    }

    assign_owner_ordinals(template, &mut declarations, allocator.clone())?;

    let mut records = ControlledVec::try_with_capacity(
        "template semantic records",
        semantic_record_count,
        allocator.clone(),
    )?;
    let mut record_payload = ControlledVec::try_with_capacity(
        "template semantic payload",
        semantic_payload_bytes,
        allocator.clone(),
    )?;
    let per_unit_record_count = entity_count
        .checked_add(template.relations.len())
        .and_then(|value| value.checked_add(template.geometry.len()))
        .ok_or(StageGenerationError::Overflow(
            "template per-unit semantic record count",
        ))?;
    let mut local_index_scratch = ControlledVec::try_with_capacity(
        "template local-index scratch",
        per_unit_record_count,
        allocator.clone(),
    )?;

    for unit in 0..n {
        let unit_record_start = records.len();
        let declaration_start = usize::try_from(unit)
            .ok()
            .and_then(|unit| unit.checked_mul(entity_count))
            .ok_or(StageGenerationError::Overflow(
                "template declaration unit offset",
            ))?;
        for declaration in
            &declarations.as_slice()[declaration_start..declaration_start + entity_count]
        {
            let payload_start = record_payload.len();
            let declaration_fields = &fields.as_slice()
                [declaration.fields.offset..declaration.fields.offset + declaration.fields.length];
            append_u16(
                &mut record_payload,
                u16::try_from(declaration_fields.len())
                    .map_err(|_| StageGenerationError::Overflow("identity payload field count"))?,
            )?;
            for field in declaration_fields {
                append_u16(&mut record_payload, field.tag)?;
                append_u32(
                    &mut record_payload,
                    u32::try_from(field.bytes.length)
                        .map_err(|_| StageGenerationError::Overflow("identity payload field"))?,
                )?;
                record_payload.try_extend_from_slice(
                    &field_payload.as_slice()
                        [field.bytes.offset..field.bytes.offset + field.bytes.length],
                )?;
            }
            records.try_push(BoundedSemanticRecord {
                record_kind: 1,
                entity_kind_code: declaration.owner.entity.kind,
                stable_id: declaration.stable_id,
                owner_ordinal: declaration.owner_ordinal,
                local_index: ABSENT_LOCAL_INDEX,
                payload: PayloadRange {
                    offset: payload_start,
                    length: record_payload.len() - payload_start,
                },
                pending_order: PendingOrder::Absent,
            })?;
        }
        for relation in &template.relations {
            let compiled = compile_relation(
                template,
                declarations.as_slice(),
                unit,
                relation,
                &mut record_payload,
            )?;
            records.try_push(compiled)?;
        }
        for point in &template.geometry {
            let payload_start = record_payload.len();
            let frame = stable_id(template, declarations.as_slice(), unit, point.frame)?;
            record_payload.try_extend_from_slice(&frame)?;
            append_u32(&mut record_payload, point.point_index)?;
            let (x_bits, y_bits, z_bits) = geometry_coordinate_bits(point, unit)?;
            append_u32(&mut record_payload, x_bits)?;
            append_u32(&mut record_payload, y_bits)?;
            append_u32(&mut record_payload, z_bits)?;
            let owner = declaration(template, declarations.as_slice(), unit, point.edge)?;
            records.try_push(BoundedSemanticRecord {
                record_kind: 5,
                entity_kind_code: point.edge.kind,
                stable_id: owner.stable_id,
                owner_ordinal: owner.owner_ordinal,
                local_index: point.point_index,
                payload: PayloadRange {
                    offset: payload_start,
                    length: record_payload.len() - payload_start,
                },
                pending_order: PendingOrder::Explicit(point.point_index),
            })?;
        }
        let unit_record_end = records.len();
        assign_local_indexes(
            records.as_mut_slice(),
            record_payload.as_mut_slice(),
            unit_record_start,
            unit_record_end,
            &mut local_index_scratch,
        )?;
    }
    if records.len() != semantic_record_count || record_payload.len() != semantic_payload_bytes {
        return Err(StageGenerationError::MaterializedMismatch(
            "bounded template semantic shape",
        ));
    }

    let prefix = materialize_bounded_stage_prefix(
        generator,
        stage,
        template,
        graph_profile,
        n,
        plan,
        records.as_slice(),
        record_payload.as_slice(),
        allocator.clone(),
    )?;
    records.sort_unstable_by(|left, right| {
        canonical_record_compare(left, right, record_payload.as_slice())
    });
    let (lir_records, lir_payload) = materialize_bounded_semantic_stage(
        records.as_slice(),
        record_payload.as_slice(),
        "template canonical LIR records",
        "template canonical LIR payload",
        allocator.clone(),
    )?;
    let output_bytes = usize::try_from(plan.counts.output_byte_count)
        .map_err(|_| StageGenerationError::Overflow("bounded output bytes"))?;
    let mut semantic_record_stream = ControlledVec::try_with_capacity(
        "template semantic record stream",
        output_bytes,
        allocator.clone(),
    )?;
    semantic_record_stream.try_extend_from_slice(identity.semantic_record_domain().as_bytes())?;
    semantic_record_stream.try_push(0)?;
    append_u32(
        &mut semantic_record_stream,
        identity.semantic_record_stream_version(),
    )?;
    append_u64(
        &mut semantic_record_stream,
        u64::try_from(records.len())
            .map_err(|_| StageGenerationError::Overflow("semantic record count"))?,
    )?;
    for record in &records {
        append_u16(&mut semantic_record_stream, record.record_kind)?;
        append_u16(&mut semantic_record_stream, record.entity_kind_code)?;
        semantic_record_stream.try_extend_from_slice(&record.stable_id)?;
        append_u32(&mut semantic_record_stream, record.owner_ordinal)?;
        append_u32(&mut semantic_record_stream, record.local_index)?;
        append_u64(
            &mut semantic_record_stream,
            u64::try_from(record.payload.length)
                .map_err(|_| StageGenerationError::Overflow("semantic payload length"))?,
        )?;
        semantic_record_stream.try_extend_from_slice(
            &record_payload.as_slice()
                [record.payload.offset..record.payload.offset + record.payload.length],
        )?;
    }
    if semantic_record_stream.len() != output_bytes {
        return Err(StageGenerationError::MaterializedMismatch(
            "bounded semantic stream bytes",
        ));
    }
    let output = semantic_record_stream.try_clone("template output construction")?;
    let BoundedTemplateMaterializationPrefix {
        source_spans,
        source_records,
        source_payload,
        typed_records,
        typed_payload,
        hir_records,
        hir_payload,
        mir_records,
        mir_payload,
        diagnostics,
        scratch,
    } = prefix;
    let materialization = BoundedTemplateMaterialization {
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
        diagnostics,
        scratch,
        output,
    };
    Ok(BoundedTemplateExecution {
        workload_id,
        graph_profile,
        n,
        records,
        record_payload,
        semantic_record_stream,
        materialization,
        plan: plan.clone(),
    })
}

pub(crate) fn finalize_bounded_template_stage_case(
    execution: BoundedTemplateExecution,
) -> Result<String, StageGenerationError> {
    verify_bounded_materialization(&execution.materialization, &execution.plan)?;
    if execution.workload_id != execution.plan.workload_id
        || execution.graph_profile != execution.plan.graph_profile
        || execution.n != execution.plan.n
        || execution.semantic_record_stream.as_slice()
            != execution.materialization.output.as_slice()
    {
        return Err(StageGenerationError::MaterializedMismatch(
            "bounded template finalized identity",
        ));
    }
    let digest = Sha256::digest(execution.semantic_record_stream.as_slice());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut hex, "{byte:02x}")
            .map_err(|_| StageGenerationError::MaterializedMismatch("semantic digest"))?;
    }
    Ok(hex)
}

#[allow(clippy::too_many_arguments)]
fn materialize_bounded_stage_prefix(
    generator: &GeneratorContract,
    stage: &StageContract,
    template: &CorridorTemplate,
    graph_profile: GraphProfileId,
    n: u32,
    plan: &ScalableStagePlanSummary,
    unsorted_records: &[BoundedSemanticRecord],
    semantic_payload: &[u8],
    allocator: ControlledAllocator,
) -> Result<BoundedTemplateMaterializationPrefix, StageGenerationError> {
    exercise_source_permutations(generator, template, n, allocator.clone())?;

    let source_span_count = to_usize(plan.counts.source_span_count, "bounded source span count")?;
    let module_count = u32::try_from(plan.counts.module_count)
        .map_err(|_| StageGenerationError::Overflow("bounded module count"))?;
    let mut source_spans = ControlledVec::try_with_capacity(
        "template source spans",
        source_span_count,
        allocator.clone(),
    )?;
    for ordinal in 0..source_span_count {
        let ordinal = u32::try_from(ordinal)
            .map_err(|_| StageGenerationError::Overflow("bounded source span ordinal"))?;
        source_spans.try_push(SourceSpanRecord {
            source_document_ordinal: ordinal % module_count,
            start_line: ordinal
                .checked_div(module_count)
                .and_then(|line| line.checked_add(1))
                .ok_or(StageGenerationError::Overflow("bounded source line"))?,
            start_column: 1,
            end_line: ordinal
                .checked_div(module_count)
                .and_then(|line| line.checked_add(1))
                .ok_or(StageGenerationError::Overflow("bounded source line"))?,
            end_column: 21,
        })?;
    }

    let source_record_count = to_usize(
        plan.stages.source_input.record_count,
        "bounded source record count",
    )?;
    let mut source_records = ControlledVec::try_with_capacity(
        "template source records",
        source_record_count,
        allocator.clone(),
    )?;
    let module_records = to_usize(plan.counts.module_count, "bounded module records")?;
    let import_records = to_usize(plan.counts.import_edge_count, "bounded import records")?;
    for ordinal in 0..source_record_count {
        let (record_kind, module_ordinal, source_span_ordinal) = if ordinal < module_records {
            (
                stage.record_kind_module,
                u32::try_from(ordinal)
                    .map_err(|_| StageGenerationError::Overflow("module ordinal"))?,
                stage.absent_ordinal,
            )
        } else if ordinal < module_records + import_records {
            (
                stage.record_kind_import,
                u32::try_from(ordinal % module_records)
                    .map_err(|_| StageGenerationError::Overflow("import module ordinal"))?,
                stage.absent_ordinal,
            )
        } else {
            (
                stage.record_kind_declaration,
                u32::try_from(ordinal % module_records)
                    .map_err(|_| StageGenerationError::Overflow("source module ordinal"))?,
                u32::try_from(ordinal - module_records - import_records)
                    .map_err(|_| StageGenerationError::Overflow("source span ordinal"))?,
            )
        };
        source_records.try_push(TypedAstStageRecord {
            record_kind,
            entity_kind: 0,
            module_ordinal,
            source_span_ordinal,
            owner_local_index: u32::try_from(ordinal)
                .map_err(|_| StageGenerationError::Overflow("source local index"))?,
            payload_offset: 0,
            payload_length: 0,
        })?;
    }

    let source_payload_bytes = to_usize(
        plan.stages.source_input.payload_logical_bytes,
        "bounded source payload bytes",
    )?;
    let mut source_payload = ControlledVec::try_with_capacity(
        "template source payload",
        source_payload_bytes,
        allocator.clone(),
    )?;
    source_payload.try_resize(source_payload_bytes, 0)?;

    let typed_record_count = to_usize(
        plan.stages.typed_ast.record_count,
        "bounded typed AST record count",
    )?;
    let mut typed_records = ControlledVec::try_with_capacity(
        "template typed AST records",
        typed_record_count,
        allocator.clone(),
    )?;
    for ordinal in 0..typed_record_count {
        let mut record = source_records
            .as_slice()
            .get(ordinal)
            .copied()
            .unwrap_or_default();
        if ordinal >= source_records.len() {
            record.record_kind = stage.record_kind_declaration;
            record.owner_local_index = u32::try_from(ordinal)
                .map_err(|_| StageGenerationError::Overflow("typed AST local index"))?;
            record.source_span_ordinal = stage.absent_ordinal;
        }
        typed_records.try_push(record)?;
    }
    let typed_payload_bytes = to_usize(
        plan.stages.typed_ast.payload_logical_bytes,
        "bounded typed AST payload bytes",
    )?;
    let mut typed_payload = ControlledVec::try_with_capacity(
        "template typed AST payload",
        typed_payload_bytes,
        allocator.clone(),
    )?;
    typed_payload.try_extend_from_slice(source_payload.as_slice())?;
    for span in &source_spans {
        append_u32(&mut typed_payload, span.source_document_ordinal)?;
        append_u32(&mut typed_payload, span.start_line)?;
        append_u32(&mut typed_payload, span.start_column)?;
        append_u32(&mut typed_payload, span.end_line)?;
        append_u32(&mut typed_payload, span.end_column)?;
    }
    if typed_payload.len() != typed_payload_bytes {
        return Err(StageGenerationError::MaterializedMismatch(
            "bounded typed AST payload",
        ));
    }

    let hir_record_count = to_usize(plan.stages.hir.record_count, "bounded HIR record count")?;
    let mut hir_records = ControlledVec::try_with_capacity(
        "template HIR records",
        hir_record_count,
        allocator.clone(),
    )?;
    for ordinal in 0..hir_record_count {
        let typed = typed_records
            .as_slice()
            .get(ordinal)
            .copied()
            .unwrap_or_default();
        hir_records.try_push(HirStageRecord {
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
        })?;
    }
    let hir_payload_bytes = to_usize(
        plan.stages.hir.payload_logical_bytes,
        "bounded HIR payload bytes",
    )?;
    let source_byte_count = to_usize(plan.counts.source_byte_count, "bounded source byte count")?;
    if source_byte_count > source_payload.len() {
        return Err(StageGenerationError::MaterializedMismatch(
            "bounded HIR string source",
        ));
    }
    let mut hir_payload = ControlledVec::try_with_capacity(
        "template HIR payload",
        hir_payload_bytes,
        allocator.clone(),
    )?;
    let string_bytes = &source_payload.as_slice()[source_byte_count..];
    hir_payload
        .try_extend_from_slice(&string_bytes[..string_bytes.len().min(hir_payload_bytes)])?;
    hir_payload.try_resize(hir_payload_bytes, 0)?;

    let (mir_records, mir_payload) = materialize_bounded_semantic_stage(
        unsorted_records,
        semantic_payload,
        "template MIR records",
        "template MIR payload",
        allocator.clone(),
    )?;
    let diagnostic_bytes = to_usize(
        plan.stages.diagnostics.logical_bytes,
        "bounded diagnostic bytes",
    )?;
    let mut diagnostics = ControlledVec::try_with_capacity(
        "template diagnostics",
        diagnostic_bytes,
        allocator.clone(),
    )?;
    diagnostics.try_resize(diagnostic_bytes, 0)?;
    let scratch_bytes = to_usize(plan.stages.scratch.logical_bytes, "bounded scratch bytes")?;
    if scratch_bytes % std::mem::size_of::<u64>() != 0 {
        return Err(StageGenerationError::MaterializedMismatch(
            "bounded scratch word alignment",
        ));
    }
    let scratch_words = scratch_bytes / std::mem::size_of::<u64>();
    let mut scratch =
        ControlledVec::try_with_capacity("template scratch", scratch_words, allocator)?;
    scratch.try_resize(scratch_words, 0)?;

    let _ = graph_profile;
    Ok(BoundedTemplateMaterializationPrefix {
        source_spans,
        source_records,
        source_payload,
        typed_records,
        typed_payload,
        hir_records,
        hir_payload,
        mir_records,
        mir_payload,
        diagnostics,
        scratch,
    })
}

fn exercise_source_permutations(
    generator: &GeneratorContract,
    template: &CorridorTemplate,
    n: u32,
    allocator: ControlledAllocator,
) -> Result<(), StageGenerationError> {
    let counts = template.stage_input_counts();
    let sequence_counts = [
        (
            SequenceKind::Declarations,
            required_template_count(&counts, "sourceDeclarationCount")?,
        ),
        (
            SequenceKind::References,
            required_template_count(&counts, "sourceReferenceCount")?,
        ),
        (
            SequenceKind::Relations,
            required_template_count(&counts, "sourceRelationCount")?,
        ),
        (
            SequenceKind::Geometry,
            required_template_count(&counts, "sourceGeometryCount")?,
        ),
    ];
    let maximum = sequence_counts
        .iter()
        .map(|(_, count)| *count)
        .max()
        .unwrap_or(0);
    let maximum = to_usize(maximum, "bounded source permutation maximum")?;
    let mut scratch = ControlledVec::try_with_capacity(
        "template source permutation scratch",
        maximum,
        allocator,
    )?;
    for unit in 0..n {
        for (sequence_kind, count) in sequence_counts {
            scratch.clear();
            for ordinal in 0..count {
                scratch.try_push(
                    u32::try_from(ordinal).map_err(|_| {
                        StageGenerationError::Overflow("source permutation ordinal")
                    })?,
                )?;
            }
            permute_in_place(
                scratch.as_mut_slice(),
                generator,
                sequence_kind,
                generator.base_seed() ^ u64::from(unit),
            );
        }
    }
    Ok(())
}

fn required_template_count(
    counts: &std::collections::BTreeMap<String, u64>,
    field: &'static str,
) -> Result<u64, StageGenerationError> {
    counts
        .get(field)
        .copied()
        .ok_or(StageGenerationError::MaterializedMismatch(field))
}

fn materialize_bounded_semantic_stage(
    records: &[BoundedSemanticRecord],
    semantic_payload: &[u8],
    record_field: &'static str,
    payload_field: &'static str,
    allocator: ControlledAllocator,
) -> Result<(ControlledVec<MirLirStageRecord>, ControlledVec<u8>), StageGenerationError> {
    let mut stage_records =
        ControlledVec::try_with_capacity(record_field, records.len(), allocator.clone())?;
    let mut payload =
        ControlledVec::try_with_capacity(payload_field, semantic_payload.len(), allocator)?;
    for record in records {
        let offset = payload.len();
        let source = record_payload_slice(record, semantic_payload);
        payload.try_extend_from_slice(source)?;
        stage_records.try_push(MirLirStageRecord {
            record_kind: record.record_kind,
            entity_kind: record.entity_kind_code,
            stable_id: record.stable_id,
            owner_ordinal: record.owner_ordinal,
            local_index: record.local_index,
            payload_offset: u64::try_from(offset)
                .map_err(|_| StageGenerationError::Overflow("semantic payload offset"))?,
            payload_length: u64::try_from(source.len())
                .map_err(|_| StageGenerationError::Overflow("semantic payload length"))?,
        })?;
    }
    Ok((stage_records, payload))
}

fn verify_bounded_materialization(
    materialization: &BoundedTemplateMaterialization,
    plan: &ScalableStagePlanSummary,
) -> Result<(), StageGenerationError> {
    let checks = [
        (
            materialization.source_records.len(),
            materialization.source_payload.len(),
            plan.stages.source_input.record_count,
            plan.stages.source_input.payload_logical_bytes,
        ),
        (
            materialization.typed_records.len(),
            materialization.typed_payload.len(),
            plan.stages.typed_ast.record_count,
            plan.stages.typed_ast.payload_logical_bytes,
        ),
        (
            materialization.hir_records.len(),
            materialization.hir_payload.len(),
            plan.stages.hir.record_count,
            plan.stages.hir.payload_logical_bytes,
        ),
        (
            materialization.mir_records.len(),
            materialization.mir_payload.len(),
            plan.stages.mir.record_count,
            plan.stages.mir.payload_logical_bytes,
        ),
        (
            materialization.lir_records.len(),
            materialization.lir_payload.len(),
            plan.stages.canonical_lir.record_count,
            plan.stages.canonical_lir.payload_logical_bytes,
        ),
    ];
    for (actual_records, actual_payload, expected_records, expected_payload) in checks {
        if u64::try_from(actual_records).ok() != Some(expected_records)
            || u64::try_from(actual_payload).ok() != Some(expected_payload)
        {
            return Err(StageGenerationError::MaterializedMismatch(
                "bounded template stage shape",
            ));
        }
    }
    if u64::try_from(materialization.source_spans.len()).ok() != Some(plan.counts.source_span_count)
        || u64::try_from(materialization.diagnostics.len()).ok()
            != Some(plan.stages.diagnostics.logical_bytes)
        || materialization
            .scratch
            .len()
            .checked_mul(std::mem::size_of::<u64>())
            .and_then(|bytes| u64::try_from(bytes).ok())
            != Some(plan.stages.scratch.logical_bytes)
        || u64::try_from(materialization.output.len()).ok()
            != Some(plan.stages.output_construction.logical_bytes)
    {
        return Err(StageGenerationError::MaterializedMismatch(
            "bounded template non-primary stage shape",
        ));
    }
    Ok(())
}

pub(crate) fn allocation_observation(
    allocator: &ControlledAllocator,
) -> ControlledAllocationObservation {
    allocator.observation()
}

fn maximum_canonical_identity_bytes(
    identity: &IdentityContract,
    template: &CorridorTemplate,
) -> Result<usize, StageGenerationError> {
    let mut maximum = 10_usize;
    for entity in &template.entities {
        let binding = identity
            .bindings
            .iter()
            .find(|binding| binding.entity_kind_code == entity.reference.kind)
            .ok_or(StageGenerationError::MissingEntityKind(
                entity.reference.kind,
            ))?;
        let bytes = binding.fields.iter().try_fold(10_usize, |total, field| {
            let value_bytes = match field.value {
                IdentityFieldValue::Namespace => 32,
                IdentityFieldValue::ProfiledKey { .. } => 20,
                IdentityFieldValue::StableId { .. } => 16,
            };
            total
                .checked_add(6 + value_bytes)
                .ok_or(StageGenerationError::Overflow(
                    "maximum canonical identity bytes",
                ))
        })?;
        maximum = maximum.max(bytes);
    }
    Ok(maximum)
}

fn assign_owner_ordinals(
    template: &CorridorTemplate,
    declarations: &mut ControlledVec<BoundedDeclaration>,
    allocator: ControlledAllocator,
) -> Result<(), StageGenerationError> {
    let mut indexes = ControlledVec::try_with_capacity(
        "template owner-ordinal scratch",
        declarations.len(),
        allocator,
    )?;
    for entity in &template.entities {
        let kind = entity.reference.kind;
        if template
            .entities
            .iter()
            .take_while(|candidate| candidate.reference != entity.reference)
            .any(|candidate| candidate.reference.kind == kind)
        {
            continue;
        }
        indexes.clear();
        for (index, declaration) in declarations.iter().enumerate() {
            if declaration.owner.entity.kind == kind {
                indexes.try_push(index)?;
            }
        }
        indexes.sort_unstable_by_key(|index| declarations[*index].stable_id);
        for (ordinal, index) in indexes.iter().copied().enumerate() {
            declarations[index].owner_ordinal = u32::try_from(ordinal)
                .map_err(|_| StageGenerationError::Overflow("owner ordinal"))?;
        }
    }
    Ok(())
}

fn compile_relation(
    template: &CorridorTemplate,
    declarations: &[BoundedDeclaration],
    unit: u32,
    relation: &TemplateRelation,
    payload: &mut ControlledVec<u8>,
) -> Result<BoundedSemanticRecord, StageGenerationError> {
    let start = payload.len();
    let (record_kind, owner, pending_order) = match relation {
        TemplateRelation::Owner { child, parent } => {
            append_u16(payload, parent.kind)?;
            payload.try_extend_from_slice(&stable_id(template, declarations, unit, *parent)?)?;
            (2, *child, PendingOrder::Absent)
        }
        TemplateRelation::EdgeConnection { source, target } => {
            payload.try_extend_from_slice(&stable_id(template, declarations, unit, *target)?)?;
            (3, *source, PendingOrder::OwnerPayload)
        }
        TemplateRelation::RouteOccurrence { route, index, edge } => {
            append_u32(payload, *index)?;
            payload.try_extend_from_slice(&stable_id(template, declarations, unit, *edge)?)?;
            (4, *route, PendingOrder::Explicit(*index))
        }
        TemplateRelation::Access {
            rule,
            participant,
            target,
            decision,
        } => {
            payload.try_extend_from_slice(&stable_id(
                template,
                declarations,
                unit,
                *participant,
            )?)?;
            append_u16(payload, target.kind)?;
            payload.try_extend_from_slice(&stable_id(template, declarations, unit, *target)?)?;
            payload.try_push(*decision)?;
            (6, *rule, PendingOrder::OwnerPayload)
        }
        TemplateRelation::SignalGroup { group, gate } => {
            payload.try_extend_from_slice(&stable_id(template, declarations, unit, *gate)?)?;
            (7, *group, PendingOrder::OwnerPayload)
        }
        TemplateRelation::PhaseState {
            phase,
            group,
            state,
        } => {
            payload.try_extend_from_slice(&stable_id(template, declarations, unit, *group)?)?;
            payload.try_push(*state)?;
            (8, *phase, PendingOrder::OwnerPayload)
        }
        TemplateRelation::Gate {
            path,
            transition_index,
            gate,
            stop_line,
            edge,
            edge_position_bits,
        } => {
            let gate_id = stable_id(template, declarations, unit, *gate)?;
            append_u32(payload, 0)?;
            payload.try_extend_from_slice(&gate_id)?;
            payload.try_extend_from_slice(&stable_id(template, declarations, unit, *stop_line)?)?;
            payload.try_extend_from_slice(&stable_id(template, declarations, unit, *edge)?)?;
            append_u32(payload, *edge_position_bits)?;
            (
                9,
                *path,
                PendingOrder::Gate {
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
            let zone_id = stable_id(template, declarations, unit, *zone)?;
            append_u32(payload, 0)?;
            payload.try_extend_from_slice(&zone_id)?;
            payload.try_extend_from_slice(&stable_id(
                template,
                declarations,
                unit,
                *before_gate,
            )?)?;
            payload.try_extend_from_slice(&stable_id(
                template,
                declarations,
                unit,
                *after_gate,
            )?)?;
            append_u32(payload, *capacity)?;
            (
                10,
                *path,
                PendingOrder::Waiting {
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
            payload.try_extend_from_slice(&stable_id(template, declarations, unit, *space)?)?;
            payload.try_extend_from_slice(&stable_id(
                template,
                declarations,
                unit,
                *entry_edge,
            )?)?;
            append_u32(payload, *entry_high_bits)?;
            append_u32(payload, *entry_residual_bits)?;
            payload.try_extend_from_slice(&stable_id(template, declarations, unit, *exit_edge)?)?;
            append_u32(payload, *exit_high_bits)?;
            append_u32(payload, *exit_residual_bits)?;
            (11, *space, PendingOrder::Absent)
        }
        TemplateRelation::LaneCoverage { lane, index, edge } => {
            append_u32(payload, *index)?;
            payload.try_extend_from_slice(&stable_id(template, declarations, unit, *edge)?)?;
            (12, *lane, PendingOrder::Explicit(*index))
        }
        TemplateRelation::JunctionInternalEdge { junction, edge } => {
            payload.try_extend_from_slice(&stable_id(template, declarations, unit, *edge)?)?;
            (13, *junction, PendingOrder::OwnerPayload)
        }
    };
    let declaration = declaration(template, declarations, unit, owner)?;
    Ok(BoundedSemanticRecord {
        record_kind,
        entity_kind_code: owner.kind,
        stable_id: declaration.stable_id,
        owner_ordinal: declaration.owner_ordinal,
        local_index: match pending_order {
            PendingOrder::Absent => ABSENT_LOCAL_INDEX,
            PendingOrder::Explicit(index) => index,
            PendingOrder::OwnerPayload
            | PendingOrder::Gate { .. }
            | PendingOrder::Waiting { .. } => 0,
        },
        payload: PayloadRange {
            offset: start,
            length: payload.len() - start,
        },
        pending_order,
    })
}

fn assign_local_indexes(
    records: &mut [BoundedSemanticRecord],
    payload: &mut [u8],
    start: usize,
    end: usize,
    scratch: &mut ControlledVec<usize>,
) -> Result<(), StageGenerationError> {
    for index in start..end {
        let order = records[index].pending_order;
        let is_group_start = (start..index)
            .all(|previous| !same_local_index_group(&records[previous], &records[index]));
        if !is_group_start || matches!(order, PendingOrder::Absent | PendingOrder::Explicit(_)) {
            continue;
        }
        scratch.clear();
        for candidate in start..end {
            if same_local_index_group(&records[candidate], &records[index]) {
                scratch.try_push(candidate)?;
            }
        }
        match order {
            PendingOrder::OwnerPayload => scratch.sort_unstable_by(|left, right| {
                record_payload_slice(&records[*left], payload)
                    .cmp(record_payload_slice(&records[*right], payload))
                    .then_with(|| records[*left].stable_id.cmp(&records[*right].stable_id))
            }),
            PendingOrder::Gate { .. } => {
                scratch.sort_unstable_by_key(|candidate| match records[*candidate].pending_order {
                    PendingOrder::Gate {
                        transition_index,
                        stable_id,
                    } => (transition_index, stable_id),
                    _ => unreachable!("gate group"),
                })
            }
            PendingOrder::Waiting { .. } => {
                scratch.sort_unstable_by_key(|candidate| match records[*candidate].pending_order {
                    PendingOrder::Waiting {
                        entry_transition_index,
                        release_transition_index,
                        stable_id,
                    } => (entry_transition_index, release_transition_index, stable_id),
                    _ => unreachable!("waiting group"),
                })
            }
            PendingOrder::Absent | PendingOrder::Explicit(_) => unreachable!("filtered above"),
        }
        for (ordinal, candidate) in scratch.iter().copied().enumerate() {
            let ordinal = u32::try_from(ordinal)
                .map_err(|_| StageGenerationError::Overflow("local index"))?;
            if matches!(
                records[candidate].pending_order,
                PendingOrder::Gate { .. } | PendingOrder::Waiting { .. }
            ) {
                let offset = records[candidate].payload.offset;
                payload[offset..offset + 4].copy_from_slice(&ordinal.to_le_bytes());
            }
            records[candidate].local_index = ordinal;
            records[candidate].pending_order = PendingOrder::Explicit(ordinal);
        }
    }
    Ok(())
}

fn same_local_index_group(left: &BoundedSemanticRecord, right: &BoundedSemanticRecord) -> bool {
    match (left.pending_order, right.pending_order) {
        (PendingOrder::OwnerPayload, PendingOrder::OwnerPayload) => {
            left.record_kind == right.record_kind
                && left.entity_kind_code == right.entity_kind_code
                && left.stable_id == right.stable_id
        }
        (PendingOrder::Gate { .. }, PendingOrder::Gate { .. })
        | (PendingOrder::Waiting { .. }, PendingOrder::Waiting { .. }) => {
            left.entity_kind_code == right.entity_kind_code && left.stable_id == right.stable_id
        }
        _ => false,
    }
}

fn canonical_record_compare(
    left: &BoundedSemanticRecord,
    right: &BoundedSemanticRecord,
    payload: &[u8],
) -> Ordering {
    (
        left.record_kind,
        left.entity_kind_code,
        left.stable_id,
        left.owner_ordinal,
        left.local_index,
        record_payload_slice(left, payload),
    )
        .cmp(&(
            right.record_kind,
            right.entity_kind_code,
            right.stable_id,
            right.owner_ordinal,
            right.local_index,
            record_payload_slice(right, payload),
        ))
}

fn record_payload_slice<'a>(record: &BoundedSemanticRecord, payload: &'a [u8]) -> &'a [u8] {
    &payload[record.payload.offset..record.payload.offset + record.payload.length]
}

fn declaration<'a>(
    template: &CorridorTemplate,
    declarations: &'a [BoundedDeclaration],
    unit: u32,
    entity: EntityRef,
) -> Result<&'a BoundedDeclaration, StageGenerationError> {
    let index = declaration_index(template, unit, entity, template.entities.len())?;
    declarations
        .get(index)
        .ok_or(StageGenerationError::MaterializedMismatch(
            "template declaration lookup",
        ))
}

fn stable_id(
    template: &CorridorTemplate,
    declarations: &[BoundedDeclaration],
    unit: u32,
    entity: EntityRef,
) -> Result<[u8; 16], StageGenerationError> {
    Ok(declaration(template, declarations, unit, entity)?.stable_id)
}

fn declaration_index(
    template: &CorridorTemplate,
    unit: u32,
    entity: EntityRef,
    entity_count: usize,
) -> Result<usize, StageGenerationError> {
    let entity_index = template
        .entities
        .iter()
        .position(|candidate| candidate.reference == entity)
        .ok_or(StageGenerationError::MaterializedMismatch(
            "template entity lookup",
        ))?;
    usize::try_from(unit)
        .ok()
        .and_then(|unit| unit.checked_mul(entity_count))
        .and_then(|base| base.checked_add(entity_index))
        .ok_or(StageGenerationError::Overflow("template declaration index"))
}

fn geometry_coordinate_bits(
    point: &crate::corridor::TemplateGeometry,
    unit: u32,
) -> Result<(u32, u32, u32), StageGenerationError> {
    if point.coordinate_rule == TemplateGeometryRule::Fixed {
        return Ok((point.x_bits, point.y_bits, point.z_bits));
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
        .ok_or(StageGenerationError::Overflow("junction grid x coordinate"))?;
    let y = unit_y
        .checked_mul(128)
        .ok_or(StageGenerationError::Overflow("junction grid y coordinate"))?;
    Ok((
        (x as f32).to_bits(),
        (y as f32).to_bits(),
        0.0_f32.to_bits(),
    ))
}

fn derive_namespace_ascii(
    generator: &GeneratorContract,
    workload_id: &str,
    graph_profile: GraphProfileId,
    unit: u32,
) -> Result<[u8; 32], StageGenerationError> {
    if generator.namespace_digest_length != 16 {
        return Err(StageGenerationError::MaterializedMismatch(
            "namespace digest length",
        ));
    }
    let (module_name, module_name_len) = unit_module_name(unit);
    let module_name = &module_name[..module_name_len];
    let mut hasher = blake3::Hasher::new();
    hasher.update(generator.namespace_domain.as_bytes());
    hasher.update(&[0]);
    hasher.update(&generator.generator_version.to_le_bytes());
    hasher.update(&generator.base_seed.to_le_bytes());
    hash_length_prefixed(&mut hasher, workload_id.as_bytes())?;
    hash_length_prefixed(&mut hasher, graph_profile.as_str().as_bytes())?;
    hash_length_prefixed(&mut hasher, module_name)?;
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

fn hash_length_prefixed(
    hasher: &mut blake3::Hasher,
    value: &[u8],
) -> Result<(), StageGenerationError> {
    let length = u32::try_from(value.len())
        .map_err(|_| StageGenerationError::Overflow("namespace preimage length"))?;
    hasher.update(&length.to_le_bytes());
    hasher.update(value);
    Ok(())
}

fn unit_module_name(unit: u32) -> ([u8; 14], usize) {
    let mut output = [0_u8; 14];
    let prefix = b"unit/".as_slice();
    output[..prefix.len()].copy_from_slice(prefix);
    write_hex_u32(unit, &mut output[prefix.len()..prefix.len() + 8]);
    (output, prefix.len() + 8)
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

fn checked_mul_usize(
    left: usize,
    right: usize,
    field: &'static str,
) -> Result<usize, StageGenerationError> {
    left.checked_mul(right)
        .ok_or(StageGenerationError::Overflow(field))
}

fn to_usize(value: u64, field: &'static str) -> Result<usize, StageGenerationError> {
    usize::try_from(value).map_err(|_| StageGenerationError::Overflow(field))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corridor::{CORRIDOR_WORKLOAD_ID, CorridorContract, build_corridor_stage_case};
    use crate::junction_grid::{
        JUNCTION_GRID_WORKLOAD_ID, JunctionGridContract, build_junction_grid_stage_case,
        build_junction_grid_template,
    };
    use crate::{ScalableStagePlanFactory, TrustedContract, load_repository_contract};

    fn contract() -> TrustedContract {
        load_repository_contract().expect("frozen contract")
    }

    #[test]
    fn bounded_template_semantic_output_matches_existing_producers() {
        let trusted = contract();
        let generator = trusted.generator_contract().unwrap();
        let identity = trusted.identity_contract().unwrap();
        let stage = trusted.stage_contract().unwrap();
        let plans = ScalableStagePlanFactory::from_trusted_contract(&trusted).unwrap();
        let corridor_contract =
            CorridorContract::from_manifest(&trusted.workload_manifest).unwrap();
        let corridor_template = corridor_contract
            .load_template(&crate::repository_root())
            .unwrap();
        let junction_contract =
            JunctionGridContract::from_manifest(&trusted.workload_manifest).unwrap();
        let junction_template = build_junction_grid_template();
        junction_contract
            .validate_template(&junction_template)
            .unwrap();

        for graph_profile in GraphProfileId::ALL {
            for n in [1, 2] {
                for (workload_id, template) in [
                    (ScalableWorkloadId::Corridor, &corridor_template),
                    (ScalableWorkloadId::JunctionGrid, &junction_template),
                ] {
                    let plan = plans.plan(workload_id, graph_profile, n).unwrap();
                    let allocator = ControlledAllocator::new(u64::MAX);
                    let bounded = execute_bounded_template_stage_case(
                        &generator,
                        &identity,
                        &stage,
                        workload_id,
                        template,
                        graph_profile,
                        n,
                        &plan,
                        allocator,
                    )
                    .unwrap();
                    let existing = match workload_id {
                        ScalableWorkloadId::Corridor => build_corridor_stage_case(
                            &generator,
                            &identity,
                            &stage,
                            &corridor_contract,
                            template,
                            graph_profile,
                            n,
                        )
                        .unwrap(),
                        ScalableWorkloadId::JunctionGrid => build_junction_grid_stage_case(
                            &generator,
                            &identity,
                            &stage,
                            &junction_contract,
                            template,
                            graph_profile,
                            n,
                        )
                        .unwrap(),
                        ScalableWorkloadId::Identity => unreachable!(),
                    };
                    assert_eq!(
                        bounded.semantic_record_stream.as_slice(),
                        existing.semantic_record_stream,
                        "{}/{graph_profile:?}/{n}",
                        workload_id.as_str()
                    );
                    assert_eq!(bounded.records.len(), existing.records.len());
                    for (bounded_record, existing_record) in
                        bounded.records.iter().zip(&existing.records)
                    {
                        assert_eq!(bounded_record.record_kind, existing_record.record_kind);
                        assert_eq!(
                            bounded_record.entity_kind_code,
                            existing_record.entity_kind_code
                        );
                        assert_eq!(bounded_record.stable_id, existing_record.stable_id);
                        assert_eq!(bounded_record.owner_ordinal, existing_record.owner_ordinal);
                        assert_eq!(bounded_record.local_index, existing_record.local_index);
                        assert_eq!(
                            bounded.record_payload(bounded_record),
                            existing_record.payload
                        );
                    }
                }
            }
        }
        let _ = (CORRIDOR_WORKLOAD_ID, JUNCTION_GRID_WORKLOAD_ID);
    }
}
