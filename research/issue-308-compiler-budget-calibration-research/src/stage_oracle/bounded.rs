//! 正式身份工作负载的有界八阶段独立验证器。
//!
//! 生产管线全部缓冲区仍存续时，本模块用同一个 `ControlledAllocator` 顺序重建来源、
//! Typed AST、HIR、MIR 与规范 LIR。每个规模相关期望缓冲区都通过可失败受控容器
//! 分配；上一阶段比较完成并不再需要后立即释放，避免把验证模型预留冒充实测峰值。

use super::*;
use crate::stage::{HirStageRecord, TypedAstStageRecord};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;

const ABSENT_ORDINAL: u32 = u32::MAX;
const ABSENT_ENTITY_KIND: u16 = 0;
const IDENTITY_MAGIC: &[u8; 4] = b"LFID";
const STABLE_ID_DOMAIN: &[u8] = b"laneflow.stable-id.v1\0";

#[derive(Clone, Copy)]
struct StageKinds {
    module: u16,
    import: u16,
    declaration: u16,
    identity_field: u16,
    reference: u16,
    relation: u16,
    symbol: u16,
}

impl StageKinds {
    fn parse(manifest: &serde_json::Value) -> Result<Self, StageOracleError> {
        let kinds = required_object(
            required_object(manifest, "researchStageModel")?,
            "stageRecordKindCodes",
        )?;
        Ok(Self {
            module: required_u16(kinds, "module")?,
            import: required_u16(kinds, "import")?,
            declaration: required_u16(kinds, "declaration")?,
            identity_field: required_u16(kinds, "identityField")?,
            reference: required_u16(kinds, "referenceOrResolvedReference")?,
            relation: required_u16(kinds, "relation")?,
            symbol: required_u16(kinds, "symbol")?,
        })
    }
}

#[derive(Clone, Copy)]
struct CompactGraph {
    profile: GraphProfileId,
    n: u32,
    group_width: u32,
    group_count: u32,
    unit_base: u32,
    module_count: u32,
}

impl CompactGraph {
    fn parse(
        manifest: &serde_json::Value,
        profile: GraphProfileId,
        n: u32,
    ) -> Result<Self, StageOracleError> {
        if n == 0 {
            return Err(StageOracleError::Invalid {
                path: "N".to_owned(),
                expected: "at least 1".to_owned(),
            });
        }
        let raw = selected_graph_profile(manifest, profile)?;
        let group_width = if profile == GraphProfileId::SharedFaninDag {
            required_u32(raw, "groupWidth")?
        } else {
            1
        };
        if profile == GraphProfileId::SharedFaninDag && group_width != 64 {
            return Err(StageOracleError::Invalid {
                path: "moduleGraphProfiles[shared-fanin-dag-v1].groupWidth".to_owned(),
                expected: "64".to_owned(),
            });
        }
        let group_count = if profile == GraphProfileId::SharedFaninDag {
            n.div_ceil(group_width)
        } else {
            0
        };
        let unit_base = if profile == GraphProfileId::SharedFaninDag {
            group_count
                .checked_add(2)
                .ok_or(StageOracleError::Overflow("unit module base"))?
        } else {
            1
        };
        let module_count = unit_base
            .checked_add(n)
            .ok_or(StageOracleError::Overflow("module count"))?;
        Ok(Self {
            profile,
            n,
            group_width,
            group_count,
            unit_base,
            module_count,
        })
    }

    fn module_name(self, ordinal: u32) -> Result<SmallBytes<14>, StageOracleError> {
        let mut output = SmallBytes::new();
        if ordinal == 0 {
            output.extend(b"root")?;
        } else if self.profile == GraphProfileId::SharedFaninDag && ordinal == 1 {
            output.extend(b"shared/common")?;
        } else if self.profile == GraphProfileId::SharedFaninDag && ordinal < self.unit_base {
            output.extend(b"group/")?;
            output.extend(&hex_u32(ordinal - 2))?;
        } else if let Some(unit) = self.module_to_unit(ordinal) {
            output.extend(b"unit/")?;
            output.extend(&hex_u32(unit))?;
        } else {
            return Err(StageOracleError::Invalid {
                path: "moduleOrdinal".to_owned(),
                expected: format!("canonical ordinal below {}", self.module_count),
            });
        }
        Ok(output)
    }

    fn module_to_unit(self, ordinal: u32) -> Option<u32> {
        ordinal
            .checked_sub(self.unit_base)
            .filter(|unit| *unit < self.n)
    }

    fn unit_module(self, unit: u32) -> Result<u32, StageOracleError> {
        if unit >= self.n {
            return Err(StageOracleError::Invalid {
                path: "unitIndex".to_owned(),
                expected: format!("below {}", self.n),
            });
        }
        self.unit_base
            .checked_add(unit)
            .ok_or(StageOracleError::Overflow("unit module ordinal"))
    }

    fn module_seed(self, ordinal: u32) -> Result<u64, StageOracleError> {
        if ordinal == 0 {
            return Ok(0);
        }
        if self.profile == GraphProfileId::SharedFaninDag {
            if ordinal == 1 {
                return Ok(1);
            }
            if ordinal < self.unit_base {
                return Ok((1_u64 << 40) | u64::from(ordinal - 2));
            }
        }
        let unit = self
            .module_to_unit(ordinal)
            .ok_or(StageOracleError::Invalid {
                path: "moduleOrdinal".to_owned(),
                expected: "canonical module".to_owned(),
            })?;
        Ok((2_u64 << 40) | u64::from(unit))
    }

    fn fill_import_targets(
        self,
        source: u32,
        output: &mut ControlledVec<u32>,
    ) -> Result<(), StageOracleError> {
        output.clear();
        match self.profile {
            GraphProfileId::WideStar => {
                if source == 0 {
                    for unit in 0..self.n {
                        output.try_push(self.unit_module(unit)?)?;
                    }
                }
            }
            GraphProfileId::DeepChain => {
                if source == 0 {
                    output.try_push(self.unit_module(0)?)?;
                } else if let Some(unit) = self.module_to_unit(source)
                    && unit + 1 < self.n
                {
                    output.try_push(self.unit_module(unit + 1)?)?;
                }
            }
            GraphProfileId::SharedFaninDag => {
                if source == 0 {
                    for group in 0..self.group_count {
                        output.try_push(group + 2)?;
                    }
                } else if source >= 2 && source < self.unit_base {
                    let group = source - 2;
                    let start = group
                        .checked_mul(self.group_width)
                        .ok_or(StageOracleError::Overflow("group unit start"))?;
                    let end = start.saturating_add(self.group_width).min(self.n);
                    for unit in start..end {
                        output.try_push(self.unit_module(unit)?)?;
                    }
                } else if self.module_to_unit(source).is_some() {
                    output.try_push(1)?;
                }
            }
        }
        Ok(())
    }

    fn import_count(self) -> u64 {
        match self.profile {
            GraphProfileId::WideStar | GraphProfileId::DeepChain => u64::from(self.n),
            GraphProfileId::SharedFaninDag => u64::from(self.n) * 2 + u64::from(self.group_count),
        }
    }

    fn cross_reference_count(self) -> u64 {
        match self.profile {
            GraphProfileId::WideStar | GraphProfileId::SharedFaninDag => u64::from(self.n),
            GraphProfileId::DeepChain => u64::from(self.n.saturating_sub(1)),
        }
    }

    fn maximum_import_depth(self) -> u64 {
        match self.profile {
            GraphProfileId::WideStar => 1,
            GraphProfileId::DeepChain => u64::from(self.n),
            GraphProfileId::SharedFaninDag => 3,
        }
    }

    fn cross_target(self, ordinal: u32) -> Result<(u16, u32), StageOracleError> {
        match self.profile {
            GraphProfileId::WideStar if ordinal < self.n => Ok((1, self.unit_module(ordinal)?)),
            GraphProfileId::DeepChain if ordinal < self.n.saturating_sub(1) => {
                Ok((1, self.unit_module(ordinal + 1)?))
            }
            GraphProfileId::SharedFaninDag if ordinal < self.n => Ok((SHARED_CONSTANT_KIND, 1)),
            _ => Err(StageOracleError::Invalid {
                path: "crossModuleReferenceOrdinal".to_owned(),
                expected: "canonical cross-module reference".to_owned(),
            }),
        }
    }
}

#[derive(Clone, Copy)]
struct SmallBytes<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> SmallBytes<N> {
    const fn new() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }

    fn extend(&mut self, value: &[u8]) -> Result<(), StageOracleError> {
        let end = self
            .len
            .checked_add(value.len())
            .ok_or(StageOracleError::Overflow("small byte buffer"))?;
        let target = self
            .bytes
            .get_mut(self.len..end)
            .ok_or(StageOracleError::Overflow("small byte buffer"))?;
        target.copy_from_slice(value);
        self.len = end;
        Ok(())
    }

    fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

#[derive(Clone, Copy)]
struct StringLayout {
    string_start: usize,
    namespace_base: usize,
    profiled_key_base: usize,
    reference_base: usize,
    string_end: usize,
    shared_name: Option<(usize, usize)>,
}

struct ExpectedSource {
    spans: ControlledVec<SourceSpanRecord>,
    records: ControlledVec<TypedAstStageRecord>,
    payload: ControlledVec<u8>,
    strings: StringLayout,
}

struct ExpectedTyped {
    records: ControlledVec<TypedAstStageRecord>,
    payload: ControlledVec<u8>,
}

struct ExpectedHir {
    records: ControlledVec<HirStageRecord>,
    payload: ControlledVec<u8>,
}

struct ExpectedMir {
    records: ControlledVec<MirLirStageRecord>,
    payload: ControlledVec<u8>,
}

pub(crate) fn verify_identity_stage_exact_bounded(
    manifest: &serde_json::Value,
    graph_profile: GraphProfileId,
    n: u32,
    produced: &IdentityStageCaseOutput,
    allocator: ControlledAllocator,
) -> Result<(), StageOracleError> {
    let bindings = parse_identity_bindings(manifest)?;
    let relations = parse_owner_relations(manifest, &bindings)?;
    let graph = CompactGraph::parse(manifest, graph_profile, n)?;
    let kinds = StageKinds::parse(manifest)?;
    let expected_summary = expected_summary(
        manifest,
        graph,
        &bindings,
        &relations,
        &produced.output_construction,
    )?;
    if produced.summary != expected_summary {
        return Err(StageOracleError::Mismatch("stage summary"));
    }

    let source = build_source(
        manifest,
        graph,
        &bindings,
        &relations,
        kinds,
        &expected_summary,
        allocator.clone(),
    )?;
    if source.spans.as_slice() != produced.source_spans {
        return Err(StageOracleError::Mismatch("source spans"));
    }
    if source.records.as_slice() != produced.source_input_records {
        return Err(StageOracleError::Mismatch("source input records"));
    }
    if source.payload.as_slice() != produced.source_input_payload {
        return Err(StageOracleError::Mismatch("source input payload"));
    }
    if produced.source_string_range != (source.strings.string_start..source.strings.string_end) {
        return Err(StageOracleError::Mismatch("source string range"));
    }

    let typed = build_typed(
        graph,
        &bindings,
        kinds,
        &source,
        &expected_summary,
        allocator.clone(),
    )?;
    if typed.records.as_slice() != produced.typed_ast_records
        || typed.payload.as_slice() != produced.typed_ast_payload
    {
        return Err(StageOracleError::Mismatch("typed AST stage"));
    }
    let strings = source.strings;
    drop(source);

    let hir = build_hir(
        graph,
        &bindings,
        kinds,
        strings,
        &typed,
        &expected_summary,
        allocator.clone(),
    )?;
    if hir.payload.as_slice() != produced.hir_payload {
        return Err(StageOracleError::Mismatch("HIR payload"));
    }
    if hir.records.as_slice() != produced.hir_records {
        return Err(StageOracleError::Mismatch("HIR records"));
    }
    drop(typed);

    let mir = build_mir(
        manifest,
        graph,
        &bindings,
        &relations,
        &expected_summary,
        allocator.clone(),
    )?;
    if mir.records.as_slice() != produced.mir_records
        || mir.payload.as_slice() != produced.mir_payload
    {
        return Err(StageOracleError::Mismatch("MIR stage"));
    }
    drop(hir);

    verify_lir_and_output(
        manifest,
        &mir,
        produced,
        allocator,
        expected_summary.stages.canonical_lir.record_count,
    )?;
    if !produced.diagnostics.is_empty() {
        return Err(StageOracleError::Mismatch("diagnostics"));
    }
    if produced.scratch_capacity_bytes > expected_summary.stages.scratch.logical_bytes {
        return Err(StageOracleError::Mismatch("scratch capacity"));
    }
    Ok(())
}

fn expected_summary(
    manifest: &serde_json::Value,
    graph: CompactGraph,
    bindings: &[OracleBinding],
    relations: &[OracleRelation],
    output: &[u8],
) -> Result<IdentityStageSummary, StageOracleError> {
    let stage = StageConstants::parse(manifest)?;
    let field_count = count_fields(bindings, |_| true)?;
    let profiled_count = count_fields(bindings, |field| {
        matches!(field, OracleIdentityField::ProfiledKey { .. })
    })?;
    let stable_count = count_fields(bindings, |field| {
        matches!(field, OracleIdentityField::StableId { .. })
    })?;
    let binding_count = as_u64(bindings.len(), "binding count")?;
    let relation_count = as_u64(relations.len(), "relation count")?;
    let units = u64::from(graph.n);
    let shared_count = u64::from(graph.profile == GraphProfileId::SharedFaninDag);
    let module_count = u64::from(graph.module_count);
    let import_count = graph.import_count();
    let cross_count = graph.cross_reference_count();
    let identity_declarations = checked_mul("identity declarations", units, binding_count)?;
    let source_declarations =
        checked_add("source declarations", identity_declarations, shared_count)?;
    let identity_fields = checked_mul("identity fields", units, field_count)?;
    let profiled_fields = checked_mul("profiled fields", units, profiled_count)?;
    let source_relations = checked_mul("source relations", units, relation_count)?;
    let source_references = sum(&[
        checked_mul("stable references", units, stable_count)?,
        source_relations,
        cross_count,
    ])?;
    let source_spans = sum(&[source_declarations, source_references, source_relations])?;
    let source_bytes = sum(&[
        checked_mul(
            "declaration source bytes",
            stage.declaration_token_bytes_with_lf,
            source_declarations,
        )?,
        checked_mul(
            "reference source bytes",
            stage.reference_token_bytes_with_lf,
            source_references,
        )?,
        checked_mul(
            "relation source bytes",
            stage.relation_token_bytes_with_lf,
            source_relations,
        )?,
    ])?;
    let (string_items, total_string_bytes, maximum_string_bytes) = string_statistics(
        manifest,
        graph,
        binding_count,
        profiled_fields,
        source_references,
    )?;
    let semantic_records = checked_mul(
        "semantic records",
        units,
        checked_add("records per unit", binding_count, relation_count)?,
    )?;
    let identity_payload_per_unit = bindings.iter().try_fold(0_u64, |total, binding| {
        let payload = binding.fields.iter().try_fold(2_u64, |bytes, field| {
            checked_add(
                "identity payload",
                bytes,
                checked_add("identity field header", 6, field_value_length(*field))?,
            )
        })?;
        checked_add("identity payload per unit", total, payload)
    })?;
    let semantic_payload = checked_mul(
        "semantic payload",
        units,
        checked_add(
            "semantic payload per unit",
            identity_payload_per_unit,
            checked_mul("relation payload", relation_count, 18)?,
        )?,
    )?;
    let source_input = stage_shape(
        sum(&[module_count, import_count, source_spans])?,
        checked_add("source input payload", source_bytes, total_string_bytes)?,
        stage.typed_ast_logical_record_bytes,
        stage.typed_ast_allocation_record_bytes,
    )?;
    let typed_ast = stage_shape(
        sum(&[
            module_count,
            import_count,
            source_declarations,
            identity_fields,
            source_references,
            source_relations,
        ])?,
        sum(&[
            source_bytes,
            total_string_bytes,
            checked_mul("source span payload", source_spans, 20)?,
        ])?,
        stage.typed_ast_logical_record_bytes,
        stage.typed_ast_allocation_record_bytes,
    )?;
    let symbols = source_declarations;
    let hir_operands = sum(&[
        identity_fields,
        import_count,
        source_references,
        checked_mul("relation operands", source_relations, 2)?,
    ])?;
    let hir = stage_shape(
        sum(&[
            module_count,
            import_count,
            symbols,
            identity_fields,
            source_references,
            source_relations,
        ])?,
        checked_add(
            "HIR payload",
            total_string_bytes,
            checked_mul("HIR operands", hir_operands, 4)?,
        )?,
        stage.hir_logical_record_bytes,
        stage.hir_allocation_record_bytes,
    )?;
    let mir = stage_shape(
        semantic_records,
        semantic_payload,
        stage.mir_lir_logical_record_bytes,
        stage.mir_lir_allocation_record_bytes,
    )?;
    let output_bytes = as_u64(output.len(), "output bytes")?;
    let expected_output_bytes = sum(&[
        as_u64(
            required_string(manifest, "semanticRecordDomainUtf8NulTerminated")?.len(),
            "semantic domain",
        )?,
        1,
        4,
        8,
        checked_mul("output record headers", semantic_records, 36)?,
        semantic_payload,
    ])?;
    if output_bytes != expected_output_bytes {
        return Err(StageOracleError::Mismatch("output construction length"));
    }
    let scratch_bytes = checked_mul(
        "scratch bytes",
        8,
        module_count.max(symbols).max(semantic_records),
    )?;
    let stages = StageBreakdown {
        source_input,
        typed_ast,
        hir,
        mir,
        canonical_lir: mir,
        diagnostics: StageShape {
            record_count: 0,
            payload_logical_bytes: 0,
            logical_bytes: 0,
            record_allocation_bytes: 0,
        },
        scratch: StageShape {
            record_count: 0,
            payload_logical_bytes: scratch_bytes,
            logical_bytes: scratch_bytes,
            record_allocation_bytes: scratch_bytes,
        },
        output_construction: StageShape {
            record_count: semantic_records,
            payload_logical_bytes: semantic_payload,
            logical_bytes: expected_output_bytes,
            record_allocation_bytes: expected_output_bytes,
        },
    };
    Ok(IdentityStageSummary {
        graph_profile: graph.profile,
        n: graph.n,
        counts: IdentityAggregateCounts {
            module_count,
            import_edge_count: import_count,
            cross_module_reference_count: cross_count,
            maximum_import_depth: graph.maximum_import_depth(),
            source_document_count: module_count,
            source_byte_count: source_bytes,
            identity_declaration_count: identity_declarations,
            source_declaration_count: source_declarations,
            source_span_count: source_spans,
            identity_field_occurrence_count: identity_fields,
            profiled_key_occurrence_count: profiled_fields,
            source_reference_count: source_references,
            source_relation_count: source_relations,
            source_geometry_count: 0,
            symbol_count: symbols,
            string_item_count: string_items,
            maximum_string_bytes,
            total_string_bytes,
            diagnostic_count: 0,
            semantic_output_record: semantic_records,
            semantic_payload_byte_count: semantic_payload,
            logical_byte_count: mir.logical_bytes,
            output_byte_count: expected_output_bytes,
        },
        stages,
        semantic_digest_sha256: lower_hex(&Sha256::digest(output)),
    })
}

fn string_statistics(
    manifest: &serde_json::Value,
    graph: CompactGraph,
    binding_count: u64,
    profiled_fields: u64,
    source_references: u64,
) -> Result<(u64, u64, u64), StageOracleError> {
    let mut module_name_bytes = 0_u64;
    let mut source_key_bytes = 0_u64;
    let mut maximum = 0_u64;
    for module in 0..graph.module_count {
        let name = graph.module_name(module)?;
        let name_len = as_u64(name.as_slice().len(), "module name")?;
        module_name_bytes = checked_add("module names", module_name_bytes, name_len)?;
        maximum = maximum.max(name_len);
        let key_len = sum(&[
            7,
            as_u64(graph.profile.as_str().len(), "graph profile")?,
            1,
            name_len,
            12,
        ])?;
        source_key_bytes = checked_add("source keys", source_key_bytes, key_len)?;
        maximum = maximum.max(key_len);
    }
    let import_name_bytes = match graph.profile {
        GraphProfileId::WideStar | GraphProfileId::DeepChain => u64::from(graph.n) * 13,
        GraphProfileId::SharedFaninDag => {
            u64::from(graph.group_count) * 14 + u64::from(graph.n) * 26
        }
    };
    maximum = maximum.max(if graph.profile == GraphProfileId::SharedFaninDag {
        14
    } else {
        13
    });
    let shared = selected_graph_profile(manifest, graph.profile)?.get("sharedSourceConstant");
    let (shared_items, shared_bytes, shared_max) = match shared {
        Some(value) => {
            let name = required_string(value, "nameUtf8")?;
            let payload = required_string(value, "valueUtf8")?;
            (
                2,
                as_u64(name.len() + payload.len(), "shared strings")?,
                as_u64(name.len().max(payload.len()), "shared string")?,
            )
        }
        None => (0, 0, 0),
    };
    maximum = maximum.max(32).max(30).max(shared_max);
    let units = u64::from(graph.n);
    let namespace_items = checked_mul("namespace strings", units, binding_count)?;
    let string_items = sum(&[
        u64::from(graph.module_count) * 2,
        graph.import_count(),
        namespace_items,
        profiled_fields,
        source_references,
        shared_items,
    ])?;
    let total = sum(&[
        module_name_bytes,
        source_key_bytes,
        import_name_bytes,
        checked_mul("namespace bytes", namespace_items, 32)?,
        checked_mul("profiled key bytes", profiled_fields, 20)?,
        checked_mul("reference bytes", source_references, 30)?,
        shared_bytes,
    ])?;
    Ok((string_items, total, maximum))
}

fn build_source(
    manifest: &serde_json::Value,
    graph: CompactGraph,
    bindings: &[OracleBinding],
    relations: &[OracleRelation],
    kinds: StageKinds,
    summary: &IdentityStageSummary,
    allocator: ControlledAllocator,
) -> Result<ExpectedSource, StageOracleError> {
    let mut payload = ControlledVec::try_with_capacity(
        "identity oracle source payload",
        to_usize(summary.stages.source_input.payload_logical_bytes)?,
        allocator.clone(),
    )?;
    let mut spans = ControlledVec::try_with_capacity(
        "identity oracle source spans",
        to_usize(summary.counts.source_span_count)?,
        allocator.clone(),
    )?;
    let source_rule = required_object(manifest, "sourceSpanRule")?;
    let token_kinds = required_object(source_rule, "sourceTokenKindUtf8")?;
    let declaration_token = required_string(token_kinds, "declarations")?;
    let reference_token = required_string(token_kinds, "references")?;
    let relation_token = required_string(token_kinds, "relations")?;
    let stable_per_unit = count_fields(bindings, |field| {
        matches!(field, OracleIdentityField::StableId { .. })
    })?;
    let relation_per_unit = as_u64(relations.len(), "relation count")?;
    for module in 0..graph.module_count {
        let mut line = 1_u32;
        let declarations = module_declaration_count(graph, bindings, module)?;
        let references = module_reference_count(graph, stable_per_unit, relation_per_unit, module)?;
        let relation_count = if graph.module_to_unit(module).is_some() {
            relation_per_unit
        } else {
            0
        };
        append_tokens(
            &mut payload,
            &mut spans,
            module,
            &mut line,
            declaration_token,
            declarations,
        )?;
        append_tokens(
            &mut payload,
            &mut spans,
            module,
            &mut line,
            reference_token,
            references,
        )?;
        append_tokens(
            &mut payload,
            &mut spans,
            module,
            &mut line,
            relation_token,
            relation_count,
        )?;
    }
    let string_start = payload.len();
    let mut records = ControlledVec::try_with_capacity(
        "identity oracle source records",
        to_usize(summary.stages.source_input.record_count)?,
        allocator.clone(),
    )?;
    for module in 0..graph.module_count {
        let start = payload.len();
        append_bytes(&mut payload, graph.module_name(module)?.as_slice())?;
        records.try_push(TypedAstStageRecord {
            record_kind: kinds.module,
            entity_kind: ABSENT_ENTITY_KIND,
            module_ordinal: module,
            source_span_ordinal: ABSENT_ORDINAL,
            owner_local_index: ABSENT_ORDINAL,
            payload_offset: to_u64(start)?,
            payload_length: to_u64(payload.len() - start)?,
        })?;
    }
    for module in 0..graph.module_count {
        append_bytes(&mut payload, b"source/")?;
        append_bytes(&mut payload, graph.profile.as_str().as_bytes())?;
        payload.try_push(b'/')?;
        append_bytes(&mut payload, graph.module_name(module)?.as_slice())?;
        append_bytes(&mut payload, b".lfsynthetic")?;
    }
    let permutation = OraclePermutation::parse(manifest)?;
    let mut scratch = ControlledVec::new("identity oracle source permutation", allocator.clone());
    for module in 0..graph.module_count {
        graph.fill_import_targets(module, &mut scratch)?;
        permutation.permute_imports(scratch.as_mut_slice(), graph.module_seed(module)?);
        for (input_ordinal, target) in scratch.iter().copied().enumerate() {
            let start = payload.len();
            append_bytes(&mut payload, graph.module_name(target)?.as_slice())?;
            records.try_push(TypedAstStageRecord {
                record_kind: kinds.import,
                entity_kind: ABSENT_ENTITY_KIND,
                module_ordinal: module,
                source_span_ordinal: ABSENT_ORDINAL,
                owner_local_index: u32::try_from(input_ordinal)
                    .map_err(|_| StageOracleError::Overflow("import input ordinal"))?,
                payload_offset: to_u64(start)?,
                payload_length: to_u64(payload.len() - start)?,
            })?;
        }
    }
    let namespace_base = payload.len();
    for unit in 0..graph.n {
        let namespace = derive_namespace(manifest, graph, unit)?;
        for _ in bindings {
            append_bytes(&mut payload, &namespace)?;
        }
    }
    let profiled_key_base = payload.len();
    for unit in 0..graph.n {
        for binding in bindings {
            for field in &binding.fields {
                if let OracleIdentityField::ProfiledKey { kind, local, .. } = field {
                    append_bytes(&mut payload, &hex_u16(*kind))?;
                    payload.try_push(b'/')?;
                    append_bytes(&mut payload, &hex_u32(unit))?;
                    payload.try_push(b'/')?;
                    append_bytes(&mut payload, &hex_u32(*local))?;
                }
            }
        }
    }
    let reference_base = payload.len();
    for unit in 0..graph.n {
        let module = graph.unit_module(unit)?;
        for binding in bindings {
            for field in &binding.fields {
                if let OracleIdentityField::StableId { kind, .. } = field {
                    append_reference(&mut payload, *kind, module, 0)?;
                }
            }
        }
    }
    for unit in 0..graph.n {
        let module = graph.unit_module(unit)?;
        for relation in relations {
            append_reference(&mut payload, relation.parent_kind, module, 0)?;
        }
    }
    for ordinal in 0..u32::try_from(graph.cross_reference_count())
        .map_err(|_| StageOracleError::Overflow("cross reference count"))?
    {
        let (kind, module) = graph.cross_target(ordinal)?;
        append_reference(&mut payload, kind, module, 0)?;
    }
    let mut shared_name = None;
    if graph.profile == GraphProfileId::SharedFaninDag {
        let shared = required_object(
            selected_graph_profile(manifest, graph.profile)?,
            "sharedSourceConstant",
        )?;
        let name = required_string(shared, "nameUtf8")?.as_bytes();
        shared_name = Some((payload.len(), name.len()));
        append_bytes(&mut payload, name)?;
        append_bytes(
            &mut payload,
            required_string(shared, "valueUtf8")?.as_bytes(),
        )?;
    }
    let string_end = payload.len();
    let strings = StringLayout {
        string_start,
        namespace_base,
        profiled_key_base,
        reference_base,
        string_end,
        shared_name,
    };
    append_source_records(
        graph,
        bindings,
        relations,
        kinds,
        strings,
        stable_per_unit,
        &permutation,
        &mut scratch,
        &mut records,
    )?;
    drop(scratch);
    Ok(ExpectedSource {
        spans,
        records,
        payload,
        strings,
    })
}

#[allow(clippy::too_many_arguments)]
fn append_source_records(
    graph: CompactGraph,
    bindings: &[OracleBinding],
    relations: &[OracleRelation],
    kinds: StageKinds,
    strings: StringLayout,
    stable_per_unit: u64,
    permutation: &OraclePermutation,
    scratch: &mut ControlledVec<u32>,
    records: &mut ControlledVec<TypedAstStageRecord>,
) -> Result<(), StageOracleError> {
    let relation_per_unit = as_u64(relations.len(), "relation count")?;
    let mut span_base = 0_u32;
    for module in 0..graph.module_count {
        let declarations = module_declaration_count(graph, bindings, module)?;
        let references = module_reference_count(graph, stable_per_unit, relation_per_unit, module)?;
        let relation_count = if graph.module_to_unit(module).is_some() {
            relation_per_unit
        } else {
            0
        };
        let reference_span_base = span_base
            .checked_add(to_u32(declarations)?)
            .ok_or(StageOracleError::Overflow("reference span base"))?;
        let relation_span_base = reference_span_base
            .checked_add(to_u32(references)?)
            .ok_or(StageOracleError::Overflow("relation span base"))?;
        fill_ordinals(scratch, declarations)?;
        permutation.permute(
            scratch.as_mut_slice(),
            permutation.declarations_sequence_kind,
            graph.module_seed(module)?,
        );
        for ordinal in scratch.iter().copied() {
            let (entity_kind, offset, length) =
                declaration_source_value(graph, bindings, strings, module, ordinal)?;
            records.try_push(TypedAstStageRecord {
                record_kind: kinds.declaration,
                entity_kind,
                module_ordinal: module,
                source_span_ordinal: span_base
                    .checked_add(ordinal)
                    .ok_or(StageOracleError::Overflow("declaration span"))?,
                owner_local_index: 0,
                payload_offset: offset,
                payload_length: length,
            })?;
        }
        fill_ordinals(scratch, references)?;
        permutation.permute(
            scratch.as_mut_slice(),
            permutation.references_sequence_kind,
            graph.module_seed(module)?,
        );
        for ordinal in scratch.iter().copied() {
            let descriptor = source_reference_descriptor(
                graph,
                bindings,
                relations,
                stable_per_unit,
                module,
                ordinal,
            )?;
            records.try_push(TypedAstStageRecord {
                record_kind: kinds.reference,
                entity_kind: descriptor.0,
                module_ordinal: module,
                source_span_ordinal: reference_span_base
                    .checked_add(ordinal)
                    .ok_or(StageOracleError::Overflow("reference span"))?,
                owner_local_index: ordinal,
                payload_offset: to_u64(
                    strings
                        .reference_base
                        .checked_add(
                            to_usize(u64::from(descriptor.1))?
                                .checked_mul(30)
                                .ok_or(StageOracleError::Overflow("reference offset"))?,
                        )
                        .ok_or(StageOracleError::Overflow("reference offset"))?,
                )?,
                payload_length: 30,
            })?;
        }
        fill_ordinals(scratch, relation_count)?;
        permutation.permute(
            scratch.as_mut_slice(),
            permutation.relations_sequence_kind,
            graph.module_seed(module)?,
        );
        for ordinal in scratch.iter().copied() {
            let unit = graph
                .module_to_unit(module)
                .ok_or(StageOracleError::Mismatch("relation module"))?;
            let relation = relations
                .get(
                    usize::try_from(ordinal)
                        .map_err(|_| StageOracleError::Overflow("relation binding ordinal"))?,
                )
                .ok_or(StageOracleError::Mismatch("relation binding"))?;
            let global = u64::from(graph.n)
                .checked_mul(stable_per_unit)
                .and_then(|base| base.checked_add(u64::from(unit).checked_mul(relation_per_unit)?))
                .and_then(|base| base.checked_add(u64::from(ordinal)))
                .ok_or(StageOracleError::Overflow("relation reference ordinal"))?;
            records.try_push(TypedAstStageRecord {
                record_kind: kinds.relation,
                entity_kind: relation.child_kind,
                module_ordinal: module,
                source_span_ordinal: relation_span_base
                    .checked_add(ordinal)
                    .ok_or(StageOracleError::Overflow("relation span"))?,
                owner_local_index: ordinal,
                payload_offset: to_u64(
                    strings
                        .reference_base
                        .checked_add(
                            to_usize(global)?
                                .checked_mul(30)
                                .ok_or(StageOracleError::Overflow("relation offset"))?,
                        )
                        .ok_or(StageOracleError::Overflow("relation offset"))?,
                )?,
                payload_length: 30,
            })?;
        }
        span_base = relation_span_base
            .checked_add(to_u32(relation_count)?)
            .ok_or(StageOracleError::Overflow("module span count"))?;
    }
    Ok(())
}

fn build_typed(
    graph: CompactGraph,
    bindings: &[OracleBinding],
    kinds: StageKinds,
    source: &ExpectedSource,
    summary: &IdentityStageSummary,
    allocator: ControlledAllocator,
) -> Result<ExpectedTyped, StageOracleError> {
    let sort_allocator = allocator.clone();
    let mut records = ControlledVec::try_with_capacity(
        "identity oracle typed AST records",
        to_usize(summary.stages.typed_ast.record_count)?,
        allocator.clone(),
    )?;
    for record in &source.records {
        records.try_push(*record)?;
        if record.record_kind != kinds.declaration || record.entity_kind == SHARED_CONSTANT_KIND {
            continue;
        }
        let binding_index = binding_index(bindings, record.entity_kind)?;
        let binding = &bindings[binding_index];
        let unit = graph
            .module_to_unit(record.module_ordinal)
            .ok_or(StageOracleError::Mismatch("declaration module"))?;
        let profiled_before = fields_before(bindings, binding_index, |field| {
            matches!(field, OracleIdentityField::ProfiledKey { .. })
        })?;
        let stable_before = fields_before(bindings, binding_index, |field| {
            matches!(field, OracleIdentityField::StableId { .. })
        })?;
        let profiled_per_unit = count_fields(bindings, |field| {
            matches!(field, OracleIdentityField::ProfiledKey { .. })
        })?;
        let stable_per_unit = count_fields(bindings, |field| {
            matches!(field, OracleIdentityField::StableId { .. })
        })?;
        let mut binding_profiled = 0_u64;
        let mut binding_stable = 0_u64;
        for field in &binding.fields {
            let (tag, offset, length) =
                match field {
                    OracleIdentityField::Namespace { tag } => (*tag, record.payload_offset, 32),
                    OracleIdentityField::ProfiledKey { tag, .. } => {
                        let ordinal = u64::from(unit)
                            .checked_mul(profiled_per_unit)
                            .and_then(|base| base.checked_add(profiled_before))
                            .and_then(|base| base.checked_add(binding_profiled))
                            .ok_or(StageOracleError::Overflow("profiled key ordinal"))?;
                        binding_profiled += 1;
                        (
                            *tag,
                            to_u64(
                                source
                                    .strings
                                    .profiled_key_base
                                    .checked_add(
                                        to_usize(ordinal)?.checked_mul(20).ok_or(
                                            StageOracleError::Overflow("profiled key offset"),
                                        )?,
                                    )
                                    .ok_or(StageOracleError::Overflow("profiled key offset"))?,
                            )?,
                            20,
                        )
                    }
                    OracleIdentityField::StableId { tag, .. } => {
                        let ordinal = u64::from(unit)
                            .checked_mul(stable_per_unit)
                            .and_then(|base| base.checked_add(stable_before))
                            .and_then(|base| base.checked_add(binding_stable))
                            .ok_or(StageOracleError::Overflow("stable reference ordinal"))?;
                        binding_stable += 1;
                        (
                            *tag,
                            to_u64(
                                source
                                    .strings
                                    .reference_base
                                    .checked_add(to_usize(ordinal)?.checked_mul(30).ok_or(
                                        StageOracleError::Overflow("stable reference offset"),
                                    )?)
                                    .ok_or(StageOracleError::Overflow("stable reference offset"))?,
                            )?,
                            30,
                        )
                    }
                };
            records.try_push(TypedAstStageRecord {
                record_kind: kinds.identity_field,
                entity_kind: binding.entity_kind_code,
                module_ordinal: record.module_ordinal,
                source_span_ordinal: record.source_span_ordinal,
                owner_local_index: u32::from(tag),
                payload_offset: offset,
                payload_length: length,
            })?;
        }
    }
    let mut payload = ControlledVec::try_with_capacity(
        "identity oracle typed AST payload",
        to_usize(summary.stages.typed_ast.payload_logical_bytes)?,
        allocator,
    )?;
    payload.try_extend_from_slice(source.payload.as_slice())?;
    for span in &source.spans {
        for value in [
            span.source_document_ordinal,
            span.start_line,
            span.start_column,
            span.end_line,
            span.end_column,
        ] {
            payload.try_extend_from_slice(&value.to_le_bytes())?;
        }
    }
    stable_sort_controlled(
        &mut records,
        "identity oracle typed AST stable sort order",
        "identity oracle typed AST stable sort output",
        sort_allocator,
        |left, right| typed_order(left, right, payload.as_slice()),
    )?;
    Ok(ExpectedTyped { records, payload })
}

fn build_hir(
    graph: CompactGraph,
    bindings: &[OracleBinding],
    kinds: StageKinds,
    strings: StringLayout,
    typed: &ExpectedTyped,
    summary: &IdentityStageSummary,
    allocator: ControlledAllocator,
) -> Result<ExpectedHir, StageOracleError> {
    let sort_allocator = allocator.clone();
    let mut payload = ControlledVec::try_with_capacity(
        "identity oracle HIR payload",
        to_usize(summary.stages.hir.payload_logical_bytes)?,
        allocator.clone(),
    )?;
    payload.try_extend_from_slice(
        typed
            .payload
            .as_slice()
            .get(strings.string_start..strings.string_end)
            .ok_or(StageOracleError::Mismatch("typed string range"))?,
    )?;
    let string_bytes = payload.len();
    let mut records = ControlledVec::try_with_capacity(
        "identity oracle HIR records",
        to_usize(summary.stages.hir.record_count)?,
        allocator,
    )?;
    for record in &typed.records {
        let mut hir = HirStageRecord {
            record_kind: record.record_kind,
            entity_kind: record.entity_kind,
            module_ordinal: record.module_ordinal,
            symbol_ordinal: ABSENT_ORDINAL,
            resolved_target_ordinal: ABSENT_ORDINAL,
            payload_offset: 0,
            payload_length: 0,
        };
        if record.record_kind == kinds.import {
            let target = parse_module_name(
                graph,
                record_payload(
                    typed.payload.as_slice(),
                    record.payload_offset,
                    record.payload_length,
                )?,
            )?;
            hir.resolved_target_ordinal = target;
            append_hir_operands(&mut hir, &mut payload, string_bytes, &[target])?;
        } else if record.record_kind == kinds.declaration {
            hir.record_kind = kinds.symbol;
            hir.symbol_ordinal =
                symbol_ordinal(graph, bindings, record.module_ordinal, record.entity_kind)?;
        } else if record.record_kind == kinds.identity_field {
            hir.symbol_ordinal =
                symbol_ordinal(graph, bindings, record.module_ordinal, record.entity_kind)?;
            let field = identity_field(bindings, record.entity_kind, record.owner_local_index)?;
            if matches!(field, OracleIdentityField::StableId { .. }) {
                let (_, target_module, _) = parse_reference(record_payload(
                    typed.payload.as_slice(),
                    record.payload_offset,
                    record.payload_length,
                )?)?;
                let target_kind = match field {
                    OracleIdentityField::StableId { kind, .. } => *kind,
                    _ => unreachable!(),
                };
                hir.resolved_target_ordinal =
                    symbol_ordinal(graph, bindings, target_module, target_kind)?;
            }
            append_hir_operands(
                &mut hir,
                &mut payload,
                string_bytes,
                &[record.owner_local_index],
            )?;
        } else if record.record_kind == kinds.reference {
            let (kind, module, _) = parse_reference(record_payload(
                typed.payload.as_slice(),
                record.payload_offset,
                record.payload_length,
            )?)?;
            hir.resolved_target_ordinal = symbol_ordinal(graph, bindings, module, kind)?;
            let target = hir.resolved_target_ordinal;
            append_hir_operands(&mut hir, &mut payload, string_bytes, &[target])?;
        } else if record.record_kind == kinds.relation {
            let (parent_kind, parent_module, _) = parse_reference(record_payload(
                typed.payload.as_slice(),
                record.payload_offset,
                record.payload_length,
            )?)?;
            hir.symbol_ordinal =
                symbol_ordinal(graph, bindings, record.module_ordinal, record.entity_kind)?;
            hir.resolved_target_ordinal =
                symbol_ordinal(graph, bindings, parent_module, parent_kind)?;
            let values = [hir.symbol_ordinal, hir.resolved_target_ordinal];
            append_hir_operands(&mut hir, &mut payload, string_bytes, &values)?;
        }
        records.try_push(hir)?;
    }
    stable_sort_controlled(
        &mut records,
        "identity oracle HIR stable sort order",
        "identity oracle HIR stable sort output",
        sort_allocator,
        |left, right| hir_order(left, right, payload.as_slice()),
    )?;
    Ok(ExpectedHir { records, payload })
}

fn build_mir(
    manifest: &serde_json::Value,
    graph: CompactGraph,
    bindings: &[OracleBinding],
    relations: &[OracleRelation],
    summary: &IdentityStageSummary,
    allocator: ControlledAllocator,
) -> Result<ExpectedMir, StageOracleError> {
    let identity_encoding_version = required_u16(manifest, "identityEncodingVersion")?;
    let mut records = ControlledVec::try_with_capacity(
        "identity oracle MIR records",
        to_usize(summary.stages.mir.record_count)?,
        allocator.clone(),
    )?;
    let mut payload = ControlledVec::try_with_capacity(
        "identity oracle MIR payload",
        to_usize(summary.stages.mir.payload_logical_bytes)?,
        allocator,
    )?;
    let mut stable_ids = Vec::<[u8; 16]>::with_capacity(bindings.len());
    let mut identity_payload = Vec::<u8>::with_capacity(256);
    let mut canonical = Vec::<u8>::with_capacity(264);
    for unit in 0..graph.n {
        stable_ids.clear();
        let namespace = derive_namespace(manifest, graph, unit)?;
        for binding in bindings {
            identity_payload.clear();
            identity_payload.extend_from_slice(
                &u16::try_from(binding.fields.len())
                    .map_err(|_| StageOracleError::Overflow("identity field count"))?
                    .to_le_bytes(),
            );
            for field in &binding.fields {
                let (tag, value): (u16, &[u8]) = match field {
                    OracleIdentityField::Namespace { tag } => (*tag, &namespace),
                    OracleIdentityField::ProfiledKey {
                        tag, kind, local, ..
                    } => {
                        let value = profiled_key(*kind, unit, *local);
                        identity_payload.extend_from_slice(&tag.to_le_bytes());
                        identity_payload.extend_from_slice(&(20_u32).to_le_bytes());
                        identity_payload.extend_from_slice(&value);
                        continue;
                    }
                    OracleIdentityField::StableId { tag, kind, .. } => {
                        let index = binding_index(bindings, *kind)?;
                        let value = stable_ids
                            .get(index)
                            .ok_or(StageOracleError::Mismatch("stable ID dependency"))?;
                        (*tag, value)
                    }
                };
                identity_payload.extend_from_slice(&tag.to_le_bytes());
                identity_payload.extend_from_slice(
                    &u32::try_from(value.len())
                        .map_err(|_| StageOracleError::Overflow("identity field length"))?
                        .to_le_bytes(),
                );
                identity_payload.extend_from_slice(value);
            }
            canonical.clear();
            canonical.extend_from_slice(IDENTITY_MAGIC);
            canonical.extend_from_slice(&identity_encoding_version.to_le_bytes());
            canonical.extend_from_slice(&binding.entity_kind_code.to_le_bytes());
            canonical.extend_from_slice(&identity_payload);
            let mut hasher = blake3::Hasher::new();
            hasher.update(STABLE_ID_DOMAIN);
            hasher.update(&canonical);
            let mut stable_id = [0_u8; 16];
            stable_id.copy_from_slice(&hasher.finalize().as_bytes()[..16]);
            stable_ids.push(stable_id);
            let offset = to_u64(payload.len())?;
            payload.try_extend_from_slice(&identity_payload)?;
            records.try_push(MirLirStageRecord {
                record_kind: 1,
                entity_kind: binding.entity_kind_code,
                stable_id,
                owner_ordinal: ABSENT_ORDINAL,
                local_index: ABSENT_ORDINAL,
                payload_offset: offset,
                payload_length: to_u64(identity_payload.len())?,
            })?;
        }
        for relation in relations {
            let child = stable_ids[binding_index(bindings, relation.child_kind)?];
            let parent = stable_ids[binding_index(bindings, relation.parent_kind)?];
            let offset = to_u64(payload.len())?;
            payload.try_extend_from_slice(&relation.parent_kind.to_le_bytes())?;
            payload.try_extend_from_slice(&parent)?;
            records.try_push(MirLirStageRecord {
                record_kind: 2,
                entity_kind: relation.child_kind,
                stable_id: child,
                owner_ordinal: ABSENT_ORDINAL,
                local_index: ABSENT_ORDINAL,
                payload_offset: offset,
                payload_length: 18,
            })?;
        }
    }
    Ok(ExpectedMir { records, payload })
}

fn verify_lir_and_output(
    manifest: &serde_json::Value,
    mir: &ExpectedMir,
    produced: &IdentityStageCaseOutput,
    allocator: ControlledAllocator,
    record_count: u64,
) -> Result<(), StageOracleError> {
    let count = to_usize(record_count)?;
    let mut owner_ordinals = ControlledVec::try_with_capacity(
        "identity oracle owner ordinals",
        count,
        allocator.clone(),
    )?;
    owner_ordinals.try_resize(count, ABSENT_ORDINAL)?;
    let mut scratch =
        ControlledVec::try_with_capacity("identity oracle LIR sort", count, allocator.clone())?;
    for kind in 1..=22_u16 {
        scratch.clear();
        for (index, record) in mir.records.iter().enumerate() {
            if record.record_kind == 1 && record.entity_kind == kind {
                scratch.try_push(index)?;
            }
        }
        scratch.sort_unstable_by_key(|index| mir.records[*index].stable_id);
        for (ordinal, index) in scratch.iter().copied().enumerate() {
            owner_ordinals[index] =
                u32::try_from(ordinal).map_err(|_| StageOracleError::Overflow("owner ordinal"))?;
        }
        for (index, record) in mir.records.iter().enumerate() {
            if record.record_kind == 1 || record.entity_kind != kind {
                continue;
            }
            let position = scratch
                .as_slice()
                .binary_search_by_key(&record.stable_id, |candidate| {
                    mir.records[*candidate].stable_id
                })
                .map_err(|_| StageOracleError::Mismatch("relation owner ordinal"))?;
            owner_ordinals[index] =
                u32::try_from(position).map_err(|_| StageOracleError::Overflow("owner ordinal"))?;
        }
    }
    scratch.clear();
    for index in 0..mir.records.len() {
        scratch.try_push(index)?;
    }
    scratch.sort_unstable_by(|left, right| {
        canonical_order(
            &mir.records[*left],
            owner_ordinals[*left],
            &mir.records[*right],
            owner_ordinals[*right],
            mir.payload.as_slice(),
        )
    });
    let mut expected_records = ControlledVec::try_with_capacity(
        "identity oracle canonical LIR records",
        count,
        allocator.clone(),
    )?;
    let mut expected_payload = ControlledVec::try_with_capacity(
        "identity oracle canonical LIR payload",
        mir.payload.len(),
        allocator,
    )?;
    for source_index in scratch.iter().copied() {
        let source = mir.records[source_index];
        let source_payload = record_payload(
            mir.payload.as_slice(),
            source.payload_offset,
            source.payload_length,
        )?;
        let payload_offset = to_u64(expected_payload.len())?;
        expected_payload.try_extend_from_slice(source_payload)?;
        expected_records.try_push(MirLirStageRecord {
            owner_ordinal: owner_ordinals[source_index],
            payload_offset,
            ..source
        })?;
    }
    if expected_records.as_slice() != produced.canonical_lir_records
        || expected_payload.as_slice() != produced.canonical_lir_payload
    {
        return Err(StageOracleError::Mismatch("canonical LIR stage"));
    }
    verify_output_stream(
        manifest,
        expected_records.as_slice(),
        expected_payload.as_slice(),
        &produced.output_construction,
    )
}

fn verify_output_stream(
    manifest: &serde_json::Value,
    records: &[MirLirStageRecord],
    payload: &[u8],
    actual: &[u8],
) -> Result<(), StageOracleError> {
    let mut cursor = ByteCursor::new(actual);
    cursor.take_exact(
        required_string(manifest, "semanticRecordDomainUtf8NulTerminated")?.as_bytes(),
        "output domain",
    )?;
    cursor.take_exact(&[0], "output domain terminator")?;
    cursor.take_exact(
        &required_u32(manifest, "semanticRecordStreamVersion")?.to_le_bytes(),
        "output stream version",
    )?;
    cursor.take_exact(&to_u64(records.len())?.to_le_bytes(), "output record count")?;
    for record in records {
        cursor.take_exact(&record.record_kind.to_le_bytes(), "output record kind")?;
        cursor.take_exact(&record.entity_kind.to_le_bytes(), "output entity kind")?;
        cursor.take_exact(&record.stable_id, "output stable ID")?;
        cursor.take_exact(&record.owner_ordinal.to_le_bytes(), "output owner ordinal")?;
        cursor.take_exact(&record.local_index.to_le_bytes(), "output local index")?;
        cursor.take_exact(
            &record.payload_length.to_le_bytes(),
            "output payload length",
        )?;
        cursor.take_exact(
            record_payload(payload, record.payload_offset, record.payload_length)?,
            "output payload",
        )?;
    }
    cursor.finish("output construction")
}

struct ByteCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> ByteCursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take_exact(&mut self, expected: &[u8], field: &'static str) -> Result<(), StageOracleError> {
        let end = self
            .offset
            .checked_add(expected.len())
            .ok_or(StageOracleError::Overflow("byte cursor"))?;
        if self.bytes.get(self.offset..end) != Some(expected) {
            return Err(StageOracleError::Mismatch(field));
        }
        self.offset = end;
        Ok(())
    }

    fn finish(self, field: &'static str) -> Result<(), StageOracleError> {
        if self.offset != self.bytes.len() {
            return Err(StageOracleError::Mismatch(field));
        }
        Ok(())
    }
}

fn append_tokens(
    payload: &mut ControlledVec<u8>,
    spans: &mut ControlledVec<SourceSpanRecord>,
    module: u32,
    line: &mut u32,
    token: &str,
    count: u64,
) -> Result<(), StageOracleError> {
    for local in 0..to_u32(count)? {
        let start = payload.len();
        append_bytes(payload, token.as_bytes())?;
        payload.try_push(b'/')?;
        append_bytes(payload, &hex_u32(local))?;
        payload.try_push(b'\n')?;
        spans.try_push(SourceSpanRecord {
            source_document_ordinal: module,
            start_line: *line,
            start_column: 1,
            end_line: *line,
            end_column: u32::try_from(payload.len() - start)
                .map_err(|_| StageOracleError::Overflow("source token length"))?,
        })?;
        *line = line
            .checked_add(1)
            .ok_or(StageOracleError::Overflow("source line"))?;
    }
    Ok(())
}

fn append_reference(
    output: &mut ControlledVec<u8>,
    kind: u16,
    module: u32,
    local: u32,
) -> Result<(), StageOracleError> {
    append_bytes(output, b"reference/")?;
    append_bytes(output, &hex_u16(kind))?;
    output.try_push(b'/')?;
    append_bytes(output, &hex_u32(module))?;
    output.try_push(b'/')?;
    append_bytes(output, &hex_u32(local))
}

fn derive_namespace(
    manifest: &serde_json::Value,
    graph: CompactGraph,
    unit: u32,
) -> Result<[u8; 32], StageOracleError> {
    let namespace = required_object(manifest, "namespaceDerivation")?;
    let domain = required_string(namespace, "domainUtf8NulTerminated")?;
    let generator_version = required_u32(manifest, "generatorVersion")?;
    let base_seed = parse_hex_u64(required_string(manifest, "baseSeedHexU64")?)?;
    let module = graph.module_name(graph.unit_module(unit)?)?;
    let mut preimage = Vec::with_capacity(128);
    preimage.extend_from_slice(domain.as_bytes());
    preimage.push(0);
    preimage.extend_from_slice(&generator_version.to_le_bytes());
    preimage.extend_from_slice(&base_seed.to_le_bytes());
    append_length_prefixed(&mut preimage, IDENTITY_WORKLOAD_ID.as_bytes())?;
    append_length_prefixed(&mut preimage, graph.profile.as_str().as_bytes())?;
    append_length_prefixed(&mut preimage, module.as_slice())?;
    let digest = blake3::hash(&preimage);
    let mut output = [0_u8; 32];
    for (index, byte) in digest.as_bytes()[..16].iter().copied().enumerate() {
        output[index * 2] = hex_digit(byte >> 4);
        output[index * 2 + 1] = hex_digit(byte & 0x0f);
    }
    Ok(output)
}

fn append_length_prefixed(output: &mut Vec<u8>, value: &[u8]) -> Result<(), StageOracleError> {
    output.extend_from_slice(
        &u32::try_from(value.len())
            .map_err(|_| StageOracleError::Overflow("length-prefixed value"))?
            .to_le_bytes(),
    );
    output.extend_from_slice(value);
    Ok(())
}

fn module_declaration_count(
    graph: CompactGraph,
    bindings: &[OracleBinding],
    module: u32,
) -> Result<u64, StageOracleError> {
    if graph.module_to_unit(module).is_some() {
        as_u64(bindings.len(), "declaration count")
    } else if graph.profile == GraphProfileId::SharedFaninDag && module == 1 {
        Ok(1)
    } else {
        Ok(0)
    }
}

fn module_reference_count(
    graph: CompactGraph,
    stable_per_unit: u64,
    relation_per_unit: u64,
    module: u32,
) -> Result<u64, StageOracleError> {
    let cross = match graph.profile {
        GraphProfileId::WideStar if module == 0 => u64::from(graph.n),
        GraphProfileId::DeepChain => u64::from(
            graph
                .module_to_unit(module)
                .is_some_and(|unit| unit + 1 < graph.n),
        ),
        GraphProfileId::SharedFaninDag => u64::from(graph.module_to_unit(module).is_some()),
        _ => 0,
    };
    if graph.module_to_unit(module).is_some() {
        sum(&[stable_per_unit, relation_per_unit, cross])
    } else {
        Ok(cross)
    }
}

fn declaration_source_value(
    graph: CompactGraph,
    bindings: &[OracleBinding],
    strings: StringLayout,
    module: u32,
    ordinal: u32,
) -> Result<(u16, u64, u64), StageOracleError> {
    if graph.profile == GraphProfileId::SharedFaninDag && module == 1 {
        if ordinal != 0 {
            return Err(StageOracleError::Mismatch("shared declaration ordinal"));
        }
        let (shared_name_offset, shared_name_len) = strings
            .shared_name
            .ok_or(StageOracleError::Mismatch("shared constant string layout"))?;
        return Ok((
            SHARED_CONSTANT_KIND,
            to_u64(shared_name_offset)?,
            to_u64(shared_name_len)?,
        ));
    }
    let unit = graph
        .module_to_unit(module)
        .ok_or(StageOracleError::Mismatch("declaration module"))?;
    let binding = bindings
        .get(usize::try_from(ordinal).map_err(|_| StageOracleError::Overflow("binding ordinal"))?)
        .ok_or(StageOracleError::Mismatch("declaration binding"))?;
    let global = u64::from(unit)
        .checked_mul(as_u64(bindings.len(), "binding count")?)
        .and_then(|base| base.checked_add(u64::from(ordinal)))
        .ok_or(StageOracleError::Overflow("declaration ordinal"))?;
    Ok((
        binding.entity_kind_code,
        to_u64(
            strings
                .namespace_base
                .checked_add(
                    to_usize(global)?
                        .checked_mul(32)
                        .ok_or(StageOracleError::Overflow("namespace offset"))?,
                )
                .ok_or(StageOracleError::Overflow("namespace offset"))?,
        )?,
        32,
    ))
}

fn source_reference_descriptor(
    graph: CompactGraph,
    bindings: &[OracleBinding],
    relations: &[OracleRelation],
    stable_per_unit: u64,
    module: u32,
    ordinal: u32,
) -> Result<(u16, u32), StageOracleError> {
    let relations_per_unit =
        u32::try_from(relations.len()).map_err(|_| StageOracleError::Overflow("relation count"))?;
    if let Some(unit) = graph.module_to_unit(module) {
        if u64::from(ordinal) < stable_per_unit {
            let binding = stable_field_by_ordinal(bindings, ordinal)?;
            let global = u64::from(unit)
                .checked_mul(stable_per_unit)
                .and_then(|base| base.checked_add(u64::from(ordinal)))
                .ok_or(StageOracleError::Overflow("stable reference ordinal"))?;
            return Ok((binding.entity_kind_code, to_u32(global)?));
        }
        let after_stable = ordinal
            .checked_sub(to_u32(stable_per_unit)?)
            .ok_or(StageOracleError::Overflow("reference ordinal"))?;
        if after_stable < relations_per_unit {
            let relation = relations[usize::try_from(after_stable)
                .map_err(|_| StageOracleError::Overflow("relation ordinal"))?];
            let global = u64::from(graph.n)
                .checked_mul(stable_per_unit)
                .and_then(|base| {
                    base.checked_add(u64::from(unit).checked_mul(u64::from(relations_per_unit))?)
                })
                .and_then(|base| base.checked_add(u64::from(after_stable)))
                .ok_or(StageOracleError::Overflow("relation reference ordinal"))?;
            return Ok((relation.child_kind, to_u32(global)?));
        }
        let cross_local = after_stable - relations_per_unit;
        let cross_global = match graph.profile {
            GraphProfileId::DeepChain if cross_local == 0 && unit + 1 < graph.n => unit,
            GraphProfileId::SharedFaninDag if cross_local == 0 => unit,
            _ => {
                return Err(StageOracleError::Mismatch("unit cross reference ordinal"));
            }
        };
        let base = u64::from(graph.n)
            .checked_mul(stable_per_unit + u64::from(relations_per_unit))
            .ok_or(StageOracleError::Overflow("cross reference base"))?;
        return Ok((ABSENT_ENTITY_KIND, to_u32(base + u64::from(cross_global))?));
    }
    if graph.profile == GraphProfileId::WideStar && module == 0 && ordinal < graph.n {
        let base = u64::from(graph.n)
            .checked_mul(stable_per_unit + u64::from(relations_per_unit))
            .ok_or(StageOracleError::Overflow("cross reference base"))?;
        return Ok((ABSENT_ENTITY_KIND, to_u32(base + u64::from(ordinal))?));
    }
    Err(StageOracleError::Mismatch("cross reference module"))
}

fn stable_field_by_ordinal(
    bindings: &[OracleBinding],
    ordinal: u32,
) -> Result<&OracleBinding, StageOracleError> {
    let mut current = 0_u32;
    for binding in bindings {
        for field in &binding.fields {
            if matches!(field, OracleIdentityField::StableId { .. }) {
                if current == ordinal {
                    return Ok(binding);
                }
                current += 1;
            }
        }
    }
    Err(StageOracleError::Mismatch("stable field ordinal"))
}

fn fill_ordinals(output: &mut ControlledVec<u32>, count: u64) -> Result<(), StageOracleError> {
    output.clear();
    for ordinal in 0..to_u32(count)? {
        output.try_push(ordinal)?;
    }
    Ok(())
}

fn symbol_ordinal(
    graph: CompactGraph,
    bindings: &[OracleBinding],
    module: u32,
    kind: u16,
) -> Result<u32, StageOracleError> {
    if graph.profile == GraphProfileId::SharedFaninDag
        && module == 1
        && kind == SHARED_CONSTANT_KIND
    {
        return Ok(0);
    }
    let unit = graph
        .module_to_unit(module)
        .ok_or(StageOracleError::Mismatch("symbol module"))?;
    let binding = u32::try_from(binding_index(bindings, kind)?)
        .map_err(|_| StageOracleError::Overflow("binding index"))?;
    u32::from(graph.profile == GraphProfileId::SharedFaninDag)
        .checked_add(
            unit.checked_mul(
                u32::try_from(bindings.len())
                    .map_err(|_| StageOracleError::Overflow("binding count"))?,
            )
            .ok_or(StageOracleError::Overflow("symbol ordinal"))?,
        )
        .and_then(|base| base.checked_add(binding))
        .ok_or(StageOracleError::Overflow("symbol ordinal"))
}

fn binding_index(bindings: &[OracleBinding], kind: u16) -> Result<usize, StageOracleError> {
    bindings
        .binary_search_by_key(&kind, |binding| binding.entity_kind_code)
        .map_err(|_| StageOracleError::Mismatch("entity kind"))
}

fn identity_field(
    bindings: &[OracleBinding],
    kind: u16,
    tag: u32,
) -> Result<&OracleIdentityField, StageOracleError> {
    bindings[binding_index(bindings, kind)?]
        .fields
        .iter()
        .find(|field| field_tag(**field) == tag as u16)
        .ok_or(StageOracleError::Mismatch("identity field tag"))
}

fn field_tag(field: OracleIdentityField) -> u16 {
    match field {
        OracleIdentityField::Namespace { tag }
        | OracleIdentityField::ProfiledKey { tag, .. }
        | OracleIdentityField::StableId { tag, .. } => tag,
    }
}

fn field_value_length(field: OracleIdentityField) -> u64 {
    match field {
        OracleIdentityField::Namespace { .. } => 32,
        OracleIdentityField::ProfiledKey { .. } => 20,
        OracleIdentityField::StableId { .. } => 16,
    }
}

fn fields_before(
    bindings: &[OracleBinding],
    binding_index: usize,
    predicate: impl Fn(OracleIdentityField) -> bool,
) -> Result<u64, StageOracleError> {
    bindings
        .iter()
        .take(binding_index)
        .try_fold(0_u64, |total, binding| {
            binding.fields.iter().try_fold(total, |count, field| {
                if predicate(*field) {
                    checked_add("field ordinal", count, 1)
                } else {
                    Ok(count)
                }
            })
        })
}

fn count_fields(
    bindings: &[OracleBinding],
    predicate: impl Fn(OracleIdentityField) -> bool,
) -> Result<u64, StageOracleError> {
    bindings.iter().try_fold(0_u64, |total, binding| {
        binding.fields.iter().try_fold(total, |count, field| {
            if predicate(*field) {
                checked_add("field count", count, 1)
            } else {
                Ok(count)
            }
        })
    })
}

fn append_hir_operands(
    record: &mut HirStageRecord,
    payload: &mut ControlledVec<u8>,
    string_bytes: usize,
    values: &[u32],
) -> Result<(), StageOracleError> {
    let offset = payload.len();
    if offset < string_bytes {
        return Err(StageOracleError::Mismatch("HIR operand partition"));
    }
    for value in values {
        payload.try_extend_from_slice(&value.to_le_bytes())?;
    }
    record.payload_offset = to_u64(offset)?;
    record.payload_length = to_u64(values.len() * 4)?;
    Ok(())
}

fn stable_sort_controlled<T: Copy>(
    values: &mut ControlledVec<T>,
    order_field: &'static str,
    output_field: &'static str,
    allocator: ControlledAllocator,
    mut compare: impl FnMut(&T, &T) -> Ordering,
) -> Result<(), StageOracleError> {
    let mut order = ControlledVec::try_with_capacity(order_field, values.len(), allocator.clone())?;
    for index in 0..values.len() {
        order.try_push(index)?;
    }
    order.sort_unstable_by(|left, right| {
        compare(&values[*left], &values[*right]).then_with(|| left.cmp(right))
    });
    let mut sorted = ControlledVec::try_with_capacity(output_field, values.len(), allocator)?;
    for index in order.iter().copied() {
        sorted.try_push(values[index])?;
    }
    values.as_mut_slice().copy_from_slice(sorted.as_slice());
    Ok(())
}

fn parse_reference(bytes: &[u8]) -> Result<(u16, u32, u32), StageOracleError> {
    if bytes.len() != 30 || &bytes[..10] != b"reference/" || bytes[12] != b'/' || bytes[21] != b'/'
    {
        return Err(StageOracleError::Mismatch("reference spelling"));
    }
    Ok((
        parse_hex_u16(&bytes[10..12])?,
        parse_hex_u32_bytes(&bytes[13..21])?,
        parse_hex_u32_bytes(&bytes[22..30])?,
    ))
}

fn parse_module_name(graph: CompactGraph, bytes: &[u8]) -> Result<u32, StageOracleError> {
    if bytes == b"root" {
        return Ok(0);
    }
    if graph.profile == GraphProfileId::SharedFaninDag && bytes == b"shared/common" {
        return Ok(1);
    }
    if let Some(hex) = bytes.strip_prefix(b"group/") {
        let group = parse_hex_u32_bytes(hex)?;
        if graph.profile == GraphProfileId::SharedFaninDag && group < graph.group_count {
            return group
                .checked_add(2)
                .ok_or(StageOracleError::Overflow("group module ordinal"));
        }
    }
    if let Some(hex) = bytes.strip_prefix(b"unit/") {
        return graph.unit_module(parse_hex_u32_bytes(hex)?);
    }
    Err(StageOracleError::Mismatch("module name"))
}

fn typed_order(
    left: &TypedAstStageRecord,
    right: &TypedAstStageRecord,
    payload: &[u8],
) -> Ordering {
    typed_prefix(left).cmp(&typed_prefix(right)).then_with(|| {
        record_payload(payload, left.payload_offset, left.payload_length)
            .expect("expected typed payload range")
            .cmp(
                record_payload(payload, right.payload_offset, right.payload_length)
                    .expect("expected typed payload range"),
            )
    })
}

fn typed_prefix(record: &TypedAstStageRecord) -> (u32, u16, u8, u32, u16, u32) {
    let (presence, span) = if record.source_span_ordinal == ABSENT_ORDINAL {
        (0, 0)
    } else {
        (1, record.source_span_ordinal)
    };
    (
        record.module_ordinal,
        record.record_kind,
        presence,
        span,
        record.entity_kind,
        record.owner_local_index,
    )
}

fn hir_order(left: &HirStageRecord, right: &HirStageRecord, payload: &[u8]) -> Ordering {
    let left_target = (
        u8::from(left.resolved_target_ordinal != ABSENT_ORDINAL),
        left.resolved_target_ordinal,
    );
    let right_target = (
        u8::from(right.resolved_target_ordinal != ABSENT_ORDINAL),
        right.resolved_target_ordinal,
    );
    (
        left.module_ordinal,
        left.record_kind,
        left.entity_kind,
        left.symbol_ordinal,
        left_target,
    )
        .cmp(&(
            right.module_ordinal,
            right.record_kind,
            right.entity_kind,
            right.symbol_ordinal,
            right_target,
        ))
        .then_with(|| {
            record_payload(payload, left.payload_offset, left.payload_length)
                .expect("expected HIR payload range")
                .cmp(
                    record_payload(payload, right.payload_offset, right.payload_length)
                        .expect("expected HIR payload range"),
                )
        })
}

fn canonical_order(
    left: &MirLirStageRecord,
    left_owner: u32,
    right: &MirLirStageRecord,
    right_owner: u32,
    payload: &[u8],
) -> Ordering {
    (
        left.record_kind,
        left.entity_kind,
        left.stable_id,
        left_owner,
        left.local_index,
    )
        .cmp(&(
            right.record_kind,
            right.entity_kind,
            right.stable_id,
            right_owner,
            right.local_index,
        ))
        .then_with(|| {
            record_payload(payload, left.payload_offset, left.payload_length)
                .expect("expected MIR payload range")
                .cmp(
                    record_payload(payload, right.payload_offset, right.payload_length)
                        .expect("expected MIR payload range"),
                )
        })
}

fn record_payload(payload: &[u8], offset: u64, length: u64) -> Result<&[u8], StageOracleError> {
    let start = to_usize(offset)?;
    let end = start
        .checked_add(to_usize(length)?)
        .ok_or(StageOracleError::Overflow("payload range"))?;
    payload
        .get(start..end)
        .ok_or(StageOracleError::Mismatch("payload range"))
}

fn profiled_key(kind: u16, unit: u32, local: u32) -> [u8; 20] {
    let mut value = [0_u8; 20];
    value[..2].copy_from_slice(&hex_u16(kind));
    value[2] = b'/';
    value[3..11].copy_from_slice(&hex_u32(unit));
    value[11] = b'/';
    value[12..20].copy_from_slice(&hex_u32(local));
    value
}

fn hex_u16(value: u16) -> [u8; 2] {
    [
        hex_digit(((value >> 4) & 0x0f) as u8),
        hex_digit((value & 0x0f) as u8),
    ]
}

fn hex_u32(value: u32) -> [u8; 8] {
    let mut output = [0_u8; 8];
    for (index, item) in output.iter_mut().enumerate() {
        let shift = 28 - index * 4;
        *item = hex_digit(((value >> shift) & 0x0f) as u8);
    }
    output
}

fn hex_digit(value: u8) -> u8 {
    match value {
        0..=9 => b'0' + value,
        _ => b'a' + value - 10,
    }
}

fn parse_hex_u16(value: &[u8]) -> Result<u16, StageOracleError> {
    u16::from_str_radix(
        std::str::from_utf8(value).map_err(|_| StageOracleError::Mismatch("hex u16"))?,
        16,
    )
    .map_err(|_| StageOracleError::Mismatch("hex u16"))
}

fn parse_hex_u32_bytes(value: &[u8]) -> Result<u32, StageOracleError> {
    u32::from_str_radix(
        std::str::from_utf8(value).map_err(|_| StageOracleError::Mismatch("hex u32"))?,
        16,
    )
    .map_err(|_| StageOracleError::Mismatch("hex u32"))
}

fn append_bytes(output: &mut ControlledVec<u8>, value: &[u8]) -> Result<(), StageOracleError> {
    output.try_extend_from_slice(value)?;
    Ok(())
}

fn to_usize(value: u64) -> Result<usize, StageOracleError> {
    usize::try_from(value).map_err(|_| StageOracleError::Overflow("usize conversion"))
}

fn to_u64(value: usize) -> Result<u64, StageOracleError> {
    u64::try_from(value).map_err(|_| StageOracleError::Overflow("u64 conversion"))
}

fn to_u32(value: u64) -> Result<u32, StageOracleError> {
    u32::try_from(value).map_err(|_| StageOracleError::Overflow("u32 conversion"))
}

fn lower_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}
