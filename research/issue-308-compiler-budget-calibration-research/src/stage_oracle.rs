//! 阶段聚合输入与公式的独立精确研究预言机。
//!
//! 本模块直接从受信任原始清单重建模块、导入、引用、字符串与阶段公式输入，不调用
//! `generator`、`manifest` 或 `stage` 的解析、图展开、字符串枚举和公式求值辅助函数。
//! 只共享图配置档枚举、身份预言机的不可变结果以及阶段摘要值类型。

use crate::GraphProfileId;
use crate::identity::IdentityCaseOutput;
use crate::oracle::{ExactOracleError, build_identity_oracle_case};
use crate::stage::{
    IdentityAggregateCounts, IdentityStageCaseOutput, IdentityStageSummary, MirLirStageRecord,
    SourceSpanRecord, StageBreakdown, StageShape,
};
use std::collections::{BTreeMap, BTreeSet};

const IDENTITY_WORKLOAD_ID: &str = "LF-COMP-ID-v1";
const SHORT_UNIQUE_PROFILE_ID: &str = "short-unique-v1";
const SHARED_CONSTANT_KIND: u16 = 0x00ff;

pub(crate) fn build_identity_stage_oracle(
    manifest: &serde_json::Value,
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<IdentityStageSummary, StageOracleError> {
    if n == 0 {
        return Err(StageOracleError::Invalid {
            path: "N".to_owned(),
            expected: "at least 1".to_owned(),
        });
    }

    let identity = build_identity_oracle_case(manifest, graph_profile, n)?;
    let profile = selected_graph_profile(manifest, graph_profile)?;
    let graph = OracleGraph::expand(profile, graph_profile, n)?;
    let bindings = parse_identity_bindings(manifest)?;
    let relations = parse_owner_relations(manifest, &bindings)?;
    let workload = selected_identity_workload(manifest)?;
    validate_per_unit_stage_inputs(workload, &bindings, &relations)?;
    validate_short_unique_profile(manifest)?;
    let stage_constants = StageConstants::parse(manifest)?;

    let identity_declaration_count =
        as_u64(identity.declarations.len(), "identityDeclarationCount")?;
    let identity_field_occurrence_count = checked_mul(
        "identityFieldOccurrenceCount",
        as_u64(
            bindings
                .iter()
                .try_fold(0_usize, |total, binding| {
                    total.checked_add(binding.fields.len())
                })
                .ok_or(StageOracleError::Overflow("identityFieldOccurrenceCount"))?,
            "identityFieldOccurrenceCount",
        )?,
        u64::from(n),
    )?;
    let profiled_key_occurrence_count = checked_mul(
        "profiledKeyOccurrenceCount",
        as_u64(
            bindings
                .iter()
                .flat_map(|binding| &binding.fields)
                .filter(|field| matches!(field, OracleIdentityField::ProfiledKey { .. }))
                .count(),
            "profiledKeyOccurrenceCount",
        )?,
        u64::from(n),
    )?;
    let stable_identity_reference_count = checked_mul(
        "sourceReferenceCount",
        as_u64(
            bindings
                .iter()
                .flat_map(|binding| &binding.fields)
                .filter(|field| matches!(field, OracleIdentityField::StableId { .. }))
                .count(),
            "sourceReferenceCount",
        )?,
        u64::from(n),
    )?;
    let source_relation_count = checked_mul(
        "sourceRelationCount",
        as_u64(relations.len(), "sourceRelationCount")?,
        u64::from(n),
    )?;
    let source_declaration_count = checked_add(
        "sourceDeclarationCount",
        identity_declaration_count,
        graph.shared_constant_record_count,
    )?;
    let source_reference_count = sum(&[
        stable_identity_reference_count,
        source_relation_count,
        as_u64(
            graph.cross_module_targets.len(),
            "crossModuleReferenceCount",
        )?,
    ])?;
    let source_geometry_count = 0_u64;
    let source_span_count = sum(&[
        source_declaration_count,
        source_reference_count,
        source_relation_count,
        source_geometry_count,
    ])?;
    let source_byte_count = sum(&[
        checked_mul(
            "sourceByteCount",
            stage_constants.declaration_token_bytes_with_lf,
            source_declaration_count,
        )?,
        checked_mul(
            "sourceByteCount",
            stage_constants.reference_token_bytes_with_lf,
            source_reference_count,
        )?,
        checked_mul(
            "sourceByteCount",
            stage_constants.relation_token_bytes_with_lf,
            source_relation_count,
        )?,
        checked_mul(
            "sourceByteCount",
            stage_constants.geometry_token_bytes_with_lf,
            source_geometry_count,
        )?,
    ])?;

    let strings = enumerate_strings(
        manifest,
        &graph,
        &bindings,
        &relations,
        &identity,
        graph_profile,
        n,
    )?;
    let string_item_count = as_u64(strings.len(), "stringItemCount")?;
    let total_string_bytes = strings.iter().try_fold(0_u64, |total, item| {
        checked_add(
            "totalStringBytes",
            total,
            as_u64(item.len(), "totalStringBytes")?,
        )
    })?;
    let maximum_string_bytes = strings
        .iter()
        .map(Vec::len)
        .max()
        .map_or(Ok(0_u64), |value| as_u64(value, "maximumStringBytes"))?;

    let semantic_output_record = as_u64(identity.raw_records.len(), "semanticOutputRecord")?;
    let semantic_payload_byte_count =
        identity
            .raw_records
            .iter()
            .try_fold(0_u64, |total, record| {
                checked_add(
                    "semanticPayloadByteCount",
                    total,
                    as_u64(record.payload.len(), "semanticPayloadByteCount")?,
                )
            })?;

    let module_count = as_u64(graph.modules.len(), "moduleCount")?;
    let import_edge_count = graph.modules.iter().try_fold(0_u64, |total, module| {
        checked_add(
            "importEdgeCount",
            total,
            as_u64(module.imports.len(), "importEdgeCount")?,
        )
    })?;
    let cross_module_reference_count = as_u64(
        graph.cross_module_targets.len(),
        "crossModuleReferenceCount",
    )?;
    let maximum_import_depth = u64::from(graph.maximum_import_depth()?);

    let source_input_record_count = sum(&[module_count, import_edge_count, source_span_count])?;
    let source_input_payload =
        checked_add("sourceInput.payload", source_byte_count, total_string_bytes)?;
    let source_input = stage_shape(
        source_input_record_count,
        source_input_payload,
        stage_constants.typed_ast_logical_record_bytes,
        stage_constants.typed_ast_allocation_record_bytes,
    )?;

    let typed_ast_record_count = sum(&[
        module_count,
        import_edge_count,
        source_declaration_count,
        identity_field_occurrence_count,
        source_reference_count,
        source_relation_count,
        source_geometry_count,
    ])?;
    let typed_ast_payload = sum(&[
        source_byte_count,
        total_string_bytes,
        checked_mul(
            "typedAst.sourceSpans",
            stage_constants.source_span_logical_bytes,
            source_span_count,
        )?,
    ])?;
    let typed_ast = stage_shape(
        typed_ast_record_count,
        typed_ast_payload,
        stage_constants.typed_ast_logical_record_bytes,
        stage_constants.typed_ast_allocation_record_bytes,
    )?;

    let symbol_count = source_declaration_count;
    let hir_record_count = sum(&[
        module_count,
        import_edge_count,
        symbol_count,
        identity_field_occurrence_count,
        source_reference_count,
        source_relation_count,
        source_geometry_count,
    ])?;
    let hir_operand_count = sum(&[
        identity_field_occurrence_count,
        import_edge_count,
        source_reference_count,
        checked_mul("hir.relations", 2, source_relation_count)?,
        checked_mul("hir.geometry", 3, source_geometry_count)?,
    ])?;
    let hir_payload = checked_add(
        "hir.payload",
        total_string_bytes,
        checked_mul("hir.operands", 4, hir_operand_count)?,
    )?;
    let hir = stage_shape(
        hir_record_count,
        hir_payload,
        stage_constants.hir_logical_record_bytes,
        stage_constants.hir_allocation_record_bytes,
    )?;

    let mir = stage_shape(
        semantic_output_record,
        semantic_payload_byte_count,
        stage_constants.mir_lir_logical_record_bytes,
        stage_constants.mir_lir_allocation_record_bytes,
    )?;
    let diagnostics = StageShape {
        record_count: 0,
        payload_logical_bytes: 0,
        logical_bytes: 0,
        record_allocation_bytes: 0,
    };
    let scratch_bytes = checked_mul(
        "scratch.logicalBytes",
        8,
        module_count.max(symbol_count).max(semantic_output_record),
    )?;
    let scratch = StageShape {
        record_count: 0,
        payload_logical_bytes: scratch_bytes,
        logical_bytes: scratch_bytes,
        record_allocation_bytes: scratch_bytes,
    };
    let output_bytes = sum(&[
        54,
        checked_mul("outputConstruction.records", 36, semantic_output_record)?,
        semantic_payload_byte_count,
    ])?;
    let output_construction = StageShape {
        record_count: semantic_output_record,
        payload_logical_bytes: semantic_payload_byte_count,
        logical_bytes: output_bytes,
        record_allocation_bytes: output_bytes,
    };
    let stages = StageBreakdown {
        source_input,
        typed_ast,
        hir,
        mir,
        canonical_lir: mir,
        diagnostics,
        scratch,
        output_construction,
    };
    let counts = IdentityAggregateCounts {
        module_count,
        import_edge_count,
        cross_module_reference_count,
        maximum_import_depth,
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
        symbol_count,
        string_item_count,
        maximum_string_bytes,
        total_string_bytes,
        diagnostic_count: 0,
        semantic_output_record,
        semantic_payload_byte_count,
        logical_byte_count: mir.logical_bytes,
        output_byte_count: output_bytes,
    };
    Ok(IdentityStageSummary {
        graph_profile,
        n,
        counts,
        stages,
        semantic_digest_sha256: identity.semantic_digest_sha256,
    })
}

pub(crate) fn verify_identity_stage_exact(
    manifest: &serde_json::Value,
    graph_profile: GraphProfileId,
    n: u32,
    produced: &IdentityStageCaseOutput,
) -> Result<(), StageOracleError> {
    let expected_summary = build_identity_stage_oracle(manifest, graph_profile, n)?;
    if produced.summary != expected_summary {
        return Err(StageOracleError::Mismatch("stage summary"));
    }

    let identity = build_identity_oracle_case(manifest, graph_profile, n)?;
    let profile = selected_graph_profile(manifest, graph_profile)?;
    let graph = OracleGraph::expand(profile, graph_profile, n)?;
    let bindings = parse_identity_bindings(manifest)?;
    let relations = parse_owner_relations(manifest, &bindings)?;
    let strings = enumerate_strings(
        manifest,
        &graph,
        &bindings,
        &relations,
        &identity,
        graph_profile,
        n,
    )?;
    let expected_string_bytes = strings.concat();
    if produced.source_input_payload[produced.source_string_range.clone()] != expected_string_bytes
    {
        return Err(StageOracleError::Mismatch("source string bytes"));
    }

    let (expected_source_documents, expected_spans) =
        build_source_documents(manifest, &graph, &bindings, &relations, graph_profile, n)?;
    let mut expected_source_payload =
        Vec::with_capacity(expected_source_documents.len() + expected_string_bytes.len());
    expected_source_payload.extend_from_slice(&expected_source_documents);
    expected_source_payload.extend_from_slice(&expected_string_bytes);
    if produced.source_input_payload != expected_source_payload {
        return Err(StageOracleError::Mismatch("source input payload"));
    }
    if produced.source_spans != expected_spans {
        return Err(StageOracleError::Mismatch("source spans"));
    }

    verify_mir_exact(&identity, produced)?;
    let (expected_lir_records, expected_lir_payload) = build_expected_lir(&identity)?;
    if produced.canonical_lir_records != expected_lir_records {
        return Err(StageOracleError::Mismatch("canonical LIR records"));
    }
    if produced.canonical_lir_payload != expected_lir_payload {
        return Err(StageOracleError::Mismatch("canonical LIR payload"));
    }
    if produced.output_construction != identity.semantic_record_stream {
        return Err(StageOracleError::Mismatch("output construction"));
    }
    if !produced.diagnostics.is_empty() {
        return Err(StageOracleError::Mismatch("diagnostics"));
    }
    if produced.scratch_capacity_bytes > expected_summary.stages.scratch.logical_bytes {
        return Err(StageOracleError::Mismatch("scratch capacity"));
    }
    Ok(())
}

fn build_source_documents(
    manifest: &serde_json::Value,
    graph: &OracleGraph,
    bindings: &[OracleBinding],
    relations: &[OracleRelation],
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<(Vec<u8>, Vec<SourceSpanRecord>), StageOracleError> {
    let source_span_rule = required_object(manifest, "sourceSpanRule")?;
    let token_kinds = required_object(source_span_rule, "sourceTokenKindUtf8")?;
    let declaration_token = required_string(token_kinds, "declarations")?;
    let reference_token = required_string(token_kinds, "references")?;
    let relation_token = required_string(token_kinds, "relations")?;
    let stable_fields_per_unit = bindings
        .iter()
        .flat_map(|binding| &binding.fields)
        .filter(|field| matches!(field, OracleIdentityField::StableId { .. }))
        .count();

    let mut bytes = Vec::new();
    let mut spans = Vec::new();
    for (module_ordinal, module) in graph.modules.iter().enumerate() {
        let module_ordinal = u32::try_from(module_ordinal)
            .map_err(|_| StageOracleError::Overflow("moduleOrdinal"))?;
        let unit_index = module
            .name
            .strip_prefix("unit/")
            .map(parse_hex_u32)
            .transpose()?;
        let declaration_count = if unit_index.is_some() {
            bindings.len()
        } else if module.name == "shared/common" {
            usize::try_from(graph.shared_constant_record_count)
                .map_err(|_| StageOracleError::Overflow("sourceDeclarationCount"))?
        } else {
            0
        };
        let cross_reference_count = match graph_profile {
            GraphProfileId::WideStar if module.name == "root" => usize::try_from(n)
                .map_err(|_| StageOracleError::Overflow("sourceReferenceCount"))?,
            GraphProfileId::DeepChain => {
                usize::from(unit_index.is_some_and(|unit| unit.saturating_add(1) < n))
            }
            GraphProfileId::SharedFaninDag => usize::from(unit_index.is_some()),
            _ => 0,
        };
        let reference_count = if unit_index.is_some() {
            stable_fields_per_unit
                .checked_add(relations.len())
                .and_then(|count| count.checked_add(cross_reference_count))
                .ok_or(StageOracleError::Overflow("sourceReferenceCount"))?
        } else {
            cross_reference_count
        };
        let relation_count = if unit_index.is_some() {
            relations.len()
        } else {
            0
        };

        let mut line = 1_u32;
        append_oracle_tokens(
            &mut bytes,
            &mut spans,
            module_ordinal,
            &mut line,
            declaration_token,
            declaration_count,
        )?;
        append_oracle_tokens(
            &mut bytes,
            &mut spans,
            module_ordinal,
            &mut line,
            reference_token,
            reference_count,
        )?;
        append_oracle_tokens(
            &mut bytes,
            &mut spans,
            module_ordinal,
            &mut line,
            relation_token,
            relation_count,
        )?;
    }
    Ok((bytes, spans))
}

fn append_oracle_tokens(
    bytes: &mut Vec<u8>,
    spans: &mut Vec<SourceSpanRecord>,
    module_ordinal: u32,
    line: &mut u32,
    token: &str,
    count: usize,
) -> Result<(), StageOracleError> {
    for local in 0..count {
        let start = bytes.len();
        bytes.extend_from_slice(token.as_bytes());
        bytes.push(b'/');
        bytes.extend_from_slice(
            format!(
                "{:08x}",
                u32::try_from(local)
                    .map_err(|_| StageOracleError::Overflow("source token local ordinal"))?
            )
            .as_bytes(),
        );
        bytes.push(b'\n');
        let token_length = bytes
            .len()
            .checked_sub(start)
            .ok_or(StageOracleError::Overflow("source token length"))?;
        spans.push(SourceSpanRecord {
            source_document_ordinal: module_ordinal,
            start_line: *line,
            start_column: 1,
            end_line: *line,
            end_column: u32::try_from(token_length)
                .map_err(|_| StageOracleError::Overflow("source token length"))?,
        });
        *line = line
            .checked_add(1)
            .ok_or(StageOracleError::Overflow("source line"))?;
    }
    Ok(())
}

type MirSemanticKey = (u16, u16, [u8; 16], u32, Vec<u8>);

fn verify_mir_exact(
    identity: &IdentityCaseOutput,
    produced: &IdentityStageCaseOutput,
) -> Result<(), StageOracleError> {
    let absent_ordinal = u32::MAX;
    let mut actual = Vec::with_capacity(produced.mir_records.len());
    for record in &produced.mir_records {
        if record.owner_ordinal != absent_ordinal {
            return Err(StageOracleError::Mismatch("MIR owner ordinal"));
        }
        actual.push((
            record.record_kind,
            record.entity_kind,
            record.stable_id,
            record.local_index,
            record_payload(
                &produced.mir_payload,
                record.payload_offset,
                record.payload_length,
            )?
            .to_vec(),
        ));
    }
    actual.sort_unstable();
    let mut expected = identity
        .raw_records
        .iter()
        .map(|record| {
            (
                record.record_kind,
                record.entity_kind_code,
                record.stable_id,
                record.local_index,
                record.payload.clone(),
            )
        })
        .collect::<Vec<MirSemanticKey>>();
    expected.sort_unstable();
    if actual != expected {
        return Err(StageOracleError::Mismatch("MIR semantic records"));
    }
    Ok(())
}

fn build_expected_lir(
    identity: &IdentityCaseOutput,
) -> Result<(Vec<MirLirStageRecord>, Vec<u8>), StageOracleError> {
    let mut records = Vec::with_capacity(identity.raw_records.len());
    let mut payload = Vec::new();
    for record in &identity.raw_records {
        let payload_offset =
            u64::try_from(payload.len()).map_err(|_| StageOracleError::Overflow("LIR payload"))?;
        payload.extend_from_slice(&record.payload);
        records.push(MirLirStageRecord {
            record_kind: record.record_kind,
            entity_kind: record.entity_kind_code,
            stable_id: record.stable_id,
            owner_ordinal: record.owner_ordinal,
            local_index: record.local_index,
            payload_offset,
            payload_length: u64::try_from(record.payload.len())
                .map_err(|_| StageOracleError::Overflow("LIR payload"))?,
        });
    }
    Ok((records, payload))
}

fn record_payload(payload: &[u8], offset: u64, length: u64) -> Result<&[u8], StageOracleError> {
    let start =
        usize::try_from(offset).map_err(|_| StageOracleError::Overflow("record payload offset"))?;
    let length =
        usize::try_from(length).map_err(|_| StageOracleError::Overflow("record payload length"))?;
    let end = start
        .checked_add(length)
        .ok_or(StageOracleError::Overflow("record payload range"))?;
    payload
        .get(start..end)
        .ok_or(StageOracleError::Mismatch("record payload range"))
}

#[derive(Clone, Debug)]
struct OracleGraph {
    modules: Vec<OracleModule>,
    cross_module_targets: Vec<OracleReferenceTarget>,
    shared_constant_record_count: u64,
    shared_constant_strings: Vec<Vec<u8>>,
}

#[derive(Clone, Debug)]
struct OracleModule {
    name: String,
    imports: Vec<String>,
}

#[derive(Clone, Copy, Debug)]
struct OracleReferenceTarget {
    kind: u16,
    module_ordinal: u32,
    local_ordinal: u32,
}

impl OracleGraph {
    fn expand(
        profile: &serde_json::Value,
        graph_profile: GraphProfileId,
        n: u32,
    ) -> Result<Self, StageOracleError> {
        let shared_constant_record_count =
            required_u64(profile, "sharedSourceConstantRecordCount")?;
        let mut modules = vec![OracleModule {
            name: "root".to_owned(),
            imports: Vec::new(),
        }];
        if graph_profile == GraphProfileId::SharedFaninDag {
            modules.push(OracleModule {
                name: "shared/common".to_owned(),
                imports: Vec::new(),
            });
        }
        let group_width = if graph_profile == GraphProfileId::SharedFaninDag {
            required_u32(profile, "groupWidth")?
        } else {
            1
        };
        let group_count = if graph_profile == GraphProfileId::SharedFaninDag {
            n.div_ceil(group_width)
        } else {
            0
        };
        for group_index in 0..group_count {
            modules.push(OracleModule {
                name: format!("group/{group_index:08x}"),
                imports: Vec::new(),
            });
        }
        for unit_index in 0..n {
            modules.push(OracleModule {
                name: format!("unit/{unit_index:08x}"),
                imports: Vec::new(),
            });
        }
        let ordinals = modules
            .iter()
            .enumerate()
            .map(|(ordinal, module)| {
                Ok((
                    module.name.clone(),
                    u32::try_from(ordinal)
                        .map_err(|_| StageOracleError::Overflow("moduleOrdinal"))?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, StageOracleError>>()?;

        let mut cross_module_targets = Vec::new();
        match graph_profile {
            GraphProfileId::WideStar => {
                for unit_index in 0..n {
                    let target = format!("unit/{unit_index:08x}");
                    modules[0].imports.push(target.clone());
                    cross_module_targets.push(OracleReferenceTarget {
                        kind: 1,
                        module_ordinal: ordinals[&target],
                        local_ordinal: 0,
                    });
                }
            }
            GraphProfileId::DeepChain => {
                modules[0].imports.push("unit/00000000".to_owned());
                for unit_index in 0..n.saturating_sub(1) {
                    let source = format!("unit/{unit_index:08x}");
                    let target = format!("unit/{:08x}", unit_index + 1);
                    let source_ordinal = usize::try_from(ordinals[&source])
                        .map_err(|_| StageOracleError::Overflow("moduleOrdinal"))?;
                    modules[source_ordinal].imports.push(target.clone());
                    cross_module_targets.push(OracleReferenceTarget {
                        kind: 1,
                        module_ordinal: ordinals[&target],
                        local_ordinal: 0,
                    });
                }
            }
            GraphProfileId::SharedFaninDag => {
                for group_index in 0..group_count {
                    modules[0].imports.push(format!("group/{group_index:08x}"));
                }
                for unit_index in 0..n {
                    let group = format!("group/{:08x}", unit_index / group_width);
                    let unit = format!("unit/{unit_index:08x}");
                    let group_ordinal = usize::try_from(ordinals[&group])
                        .map_err(|_| StageOracleError::Overflow("moduleOrdinal"))?;
                    let unit_ordinal = usize::try_from(ordinals[&unit])
                        .map_err(|_| StageOracleError::Overflow("moduleOrdinal"))?;
                    modules[group_ordinal].imports.push(unit);
                    modules[unit_ordinal]
                        .imports
                        .push("shared/common".to_owned());
                    cross_module_targets.push(OracleReferenceTarget {
                        kind: SHARED_CONSTANT_KIND,
                        module_ordinal: ordinals["shared/common"],
                        local_ordinal: 0,
                    });
                }
            }
        }

        let shared_constant_strings = match profile.get("sharedSourceConstant") {
            Some(shared) => vec![
                required_string(shared, "nameUtf8")?.as_bytes().to_vec(),
                required_string(shared, "valueUtf8")?.as_bytes().to_vec(),
            ],
            None => Vec::new(),
        };
        if as_u64(
            shared_constant_strings.len(),
            "sharedConstantStringItemCount",
        )? != shared_constant_record_count.saturating_mul(2)
        {
            return Err(StageOracleError::Invalid {
                path: "moduleGraphProfiles[].sharedSourceConstant".to_owned(),
                expected: "two strings exactly when one shared source constant exists".to_owned(),
            });
        }

        Ok(Self {
            modules,
            cross_module_targets,
            shared_constant_record_count,
            shared_constant_strings,
        })
    }

    fn maximum_import_depth(&self) -> Result<u32, StageOracleError> {
        let by_name = self
            .modules
            .iter()
            .map(|module| (module.name.as_str(), module))
            .collect::<BTreeMap<_, _>>();
        fn visit<'a>(
            name: &'a str,
            by_name: &BTreeMap<&'a str, &'a OracleModule>,
            visiting: &mut BTreeSet<&'a str>,
            memo: &mut BTreeMap<&'a str, u32>,
        ) -> Result<u32, StageOracleError> {
            if let Some(depth) = memo.get(name) {
                return Ok(*depth);
            }
            if !visiting.insert(name) {
                return Err(StageOracleError::Invalid {
                    path: "moduleGraphProfiles[].edges".to_owned(),
                    expected: "acyclic module graph".to_owned(),
                });
            }
            let module = by_name
                .get(name)
                .ok_or_else(|| StageOracleError::Missing(name.to_owned()))?;
            let mut depth = 0_u32;
            for target in &module.imports {
                depth = depth.max(
                    visit(target.as_str(), by_name, visiting, memo)?
                        .checked_add(1)
                        .ok_or(StageOracleError::Overflow("maximumImportDepth"))?,
                );
            }
            visiting.remove(name);
            memo.insert(name, depth);
            Ok(depth)
        }

        let mut visiting = BTreeSet::new();
        let mut memo = BTreeMap::new();
        self.modules.iter().try_fold(0_u32, |maximum, module| {
            Ok(maximum.max(visit(
                module.name.as_str(),
                &by_name,
                &mut visiting,
                &mut memo,
            )?))
        })
    }
}

#[derive(Clone, Debug)]
struct OracleBinding {
    entity_kind_code: u16,
    entity_kind: String,
    fields: Vec<OracleIdentityField>,
}

#[derive(Clone, Copy, Debug)]
enum OracleIdentityField {
    Namespace,
    ProfiledKey { kind: u16, local: u32 },
    StableId { kind: u16 },
}

#[derive(Clone, Copy, Debug)]
struct OracleRelation {
    parent_kind: u16,
}

fn enumerate_strings(
    manifest: &serde_json::Value,
    graph: &OracleGraph,
    bindings: &[OracleBinding],
    relations: &[OracleRelation],
    identity: &IdentityCaseOutput,
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<Vec<Vec<u8>>, StageOracleError> {
    let permutation = OraclePermutation::parse(manifest)?;
    let module_ordinals = graph
        .modules
        .iter()
        .enumerate()
        .map(|(ordinal, module)| {
            Ok((
                module.name.as_str(),
                u32::try_from(ordinal).map_err(|_| StageOracleError::Overflow("moduleOrdinal"))?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, StageOracleError>>()?;
    let mut strings = Vec::new();
    strings.extend(
        graph
            .modules
            .iter()
            .map(|module| module.name.as_bytes().to_vec()),
    );
    strings.extend(graph.modules.iter().map(|module| {
        format!(
            "source/{}/{}.lfsynthetic",
            graph_profile.as_str(),
            module.name
        )
        .into_bytes()
    }));
    for module in &graph.modules {
        let mut imports = module.imports.clone();
        permutation.permute_imports(
            &mut imports,
            oracle_module_seed_ordinal(module.name.as_str())?,
        );
        strings.extend(imports.iter().map(|target| target.as_bytes().to_vec()));
    }
    for namespace in &identity.unit_namespaces {
        for _ in bindings {
            strings.push(namespace.as_bytes().to_vec());
        }
    }
    for unit_index in 0..n {
        for binding in bindings {
            for field in &binding.fields {
                if let OracleIdentityField::ProfiledKey { kind, local } = field {
                    strings.push(format!("{kind:02x}/{unit_index:08x}/{local:08x}").into_bytes());
                }
            }
        }
    }
    for unit_index in 0..n {
        let module = format!("unit/{unit_index:08x}");
        let module_ordinal = *module_ordinals
            .get(module.as_str())
            .ok_or_else(|| StageOracleError::Missing(module.clone()))?;
        for binding in bindings {
            for field in &binding.fields {
                if let OracleIdentityField::StableId { kind } = field {
                    strings.push(reference_spelling(*kind, module_ordinal, 0));
                }
            }
        }
    }
    for unit_index in 0..n {
        let module = format!("unit/{unit_index:08x}");
        let module_ordinal = *module_ordinals
            .get(module.as_str())
            .ok_or_else(|| StageOracleError::Missing(module.clone()))?;
        for relation in relations {
            strings.push(reference_spelling(relation.parent_kind, module_ordinal, 0));
        }
    }
    strings.extend(graph.cross_module_targets.iter().map(|target| {
        reference_spelling(target.kind, target.module_ordinal, target.local_ordinal)
    }));
    strings.extend(graph.shared_constant_strings.iter().cloned());
    Ok(strings)
}

fn reference_spelling(kind: u16, module_ordinal: u32, local_ordinal: u32) -> Vec<u8> {
    format!("reference/{kind:02x}/{module_ordinal:08x}/{local_ordinal:08x}").into_bytes()
}

fn parse_identity_bindings(
    manifest: &serde_json::Value,
) -> Result<Vec<OracleBinding>, StageOracleError> {
    required_array(manifest, "identityBindings")?
        .iter()
        .map(|binding| {
            let entity_kind_code = required_u16(binding, "entityKindCode")?;
            let entity_kind = required_string(binding, "entityKind")?.to_owned();
            let fields = required_array(binding, "fields")?
                .iter()
                .map(|field| {
                    let value = required_string(field, "value")?;
                    if value == "namespace" {
                        return Ok(OracleIdentityField::Namespace);
                    }
                    if let Some((kind, local)) = parse_kind_local(value, "profiled-key")? {
                        if kind != entity_kind_code {
                            return Err(StageOracleError::Invalid {
                                path: format!("identityBindings[{entity_kind}].fields[].value"),
                                expected: "profiled key kind equal to owner entity kind".to_owned(),
                            });
                        }
                        return Ok(OracleIdentityField::ProfiledKey { kind, local });
                    }
                    if let Some((kind, _local)) = parse_kind_local(value, "stable-id")? {
                        return Ok(OracleIdentityField::StableId { kind });
                    }
                    Err(StageOracleError::Invalid {
                        path: format!("identityBindings[{entity_kind}].fields[].value"),
                        expected: "namespace, profiled-key, or stable-id expression".to_owned(),
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(OracleBinding {
                entity_kind_code,
                entity_kind,
                fields,
            })
        })
        .collect()
}

fn parse_owner_relations(
    manifest: &serde_json::Value,
    bindings: &[OracleBinding],
) -> Result<Vec<OracleRelation>, StageOracleError> {
    let kinds = bindings
        .iter()
        .map(|binding| (binding.entity_kind.as_str(), binding.entity_kind_code))
        .collect::<BTreeMap<_, _>>();
    required_array(selected_identity_workload(manifest)?, "ownerRelations")?
        .iter()
        .map(|relation| {
            let relation = relation
                .as_str()
                .ok_or_else(|| StageOracleError::InvalidType("ownerRelations[]".to_owned()))?;
            let (child, parent) =
                relation
                    .split_once("->")
                    .ok_or_else(|| StageOracleError::Invalid {
                        path: "ownerRelations[]".to_owned(),
                        expected: "Child->Parent".to_owned(),
                    })?;
            if !kinds.contains_key(child) {
                return Err(StageOracleError::Invalid {
                    path: "ownerRelations[]".to_owned(),
                    expected: format!("known child entity kind, got {child}"),
                });
            }
            let parent_kind =
                kinds
                    .get(parent)
                    .copied()
                    .ok_or_else(|| StageOracleError::Invalid {
                        path: "ownerRelations[]".to_owned(),
                        expected: format!("known parent entity kind, got {parent}"),
                    })?;
            Ok(OracleRelation { parent_kind })
        })
        .collect()
}

fn parse_kind_local(value: &str, prefix: &str) -> Result<Option<(u16, u32)>, StageOracleError> {
    let Some(inner) = value
        .strip_prefix(prefix)
        .and_then(|value| value.strip_prefix("(kind="))
        .and_then(|value| value.strip_suffix(')'))
    else {
        return Ok(None);
    };
    let (kind, local) = inner
        .split_once(",local=")
        .ok_or_else(|| StageOracleError::Invalid {
            path: "identityBindings[].fields[].value".to_owned(),
            expected: format!("{prefix}(kind=K,local=S)"),
        })?;
    Ok(Some((
        kind.parse()
            .map_err(|_| StageOracleError::InvalidType("identity field kind".to_owned()))?,
        local
            .parse()
            .map_err(|_| StageOracleError::InvalidType("identity field local".to_owned()))?,
    )))
}

fn validate_per_unit_stage_inputs(
    workload: &serde_json::Value,
    bindings: &[OracleBinding],
    relations: &[OracleRelation],
) -> Result<(), StageOracleError> {
    let inputs = required_object(workload, "perUnitStageInputs")?;
    let declaration_count = as_u64(bindings.len(), "sourceDeclarationCount")?;
    let field_count = as_u64(
        bindings.iter().map(|binding| binding.fields.len()).sum(),
        "identityFieldOccurrenceCount",
    )?;
    let profiled_count = as_u64(
        bindings
            .iter()
            .flat_map(|binding| &binding.fields)
            .filter(|field| matches!(field, OracleIdentityField::ProfiledKey { .. }))
            .count(),
        "profiledKeyOccurrenceCount",
    )?;
    let stable_count = as_u64(
        bindings
            .iter()
            .flat_map(|binding| &binding.fields)
            .filter(|field| matches!(field, OracleIdentityField::StableId { .. }))
            .count(),
        "sourceReferenceCount",
    )?;
    let relation_count = as_u64(relations.len(), "sourceRelationCount")?;
    for (field, actual) in [
        ("sourceDeclarationCount", declaration_count),
        ("identityFieldOccurrenceCount", field_count),
        ("profiledKeyOccurrenceCount", profiled_count),
        (
            "sourceReferenceCount",
            checked_add("sourceReferenceCount", stable_count, relation_count)?,
        ),
        ("sourceRelationCount", relation_count),
        ("sourceGeometryCount", 0),
    ] {
        let expected = required_u64(inputs, field)?;
        if actual != expected {
            return Err(StageOracleError::Invalid {
                path: format!("workloads[LF-COMP-ID-v1].perUnitStageInputs.{field}"),
                expected: format!("{expected}, recomputed {actual}"),
            });
        }
    }
    Ok(())
}

fn validate_short_unique_profile(manifest: &serde_json::Value) -> Result<(), StageOracleError> {
    let profile = required_array(manifest, "stringProfiles")?
        .iter()
        .find(|profile| {
            profile.get("id").and_then(serde_json::Value::as_str) == Some(SHORT_UNIQUE_PROFILE_ID)
        })
        .ok_or_else(|| StageOracleError::Missing(SHORT_UNIQUE_PROFILE_ID.to_owned()))?;
    for (field, expected) in [
        ("profiledKeyLengthBytes", 20_u64),
        ("sharedPrefixLengthBytes", 0_u64),
    ] {
        if field == "sharedPrefixLengthBytes" && profile.get(field).is_none() {
            continue;
        }
        if required_u64(profile, field)? != expected {
            return Err(StageOracleError::Invalid {
                path: format!("stringProfiles[short-unique-v1].{field}"),
                expected: expected.to_string(),
            });
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct OraclePermutation {
    base_seed: u64,
    imports_sequence_kind: u8,
    increment: u64,
    multiplier_1: u64,
    multiplier_2: u64,
}

impl OraclePermutation {
    fn parse(manifest: &serde_json::Value) -> Result<Self, StageOracleError> {
        let permutation = required_object(manifest, "permutation")?;
        let constants = required_array(permutation, "splitmix64ConstantsHexU64")?;
        if constants.len() != 3 {
            return Err(StageOracleError::Invalid {
                path: "permutation.splitmix64ConstantsHexU64".to_owned(),
                expected: "exactly three hexadecimal u64 constants".to_owned(),
            });
        }
        let sequence_kinds = required_object(permutation, "sequenceKinds")?;
        Ok(Self {
            base_seed: parse_hex_u64(required_string(manifest, "baseSeedHexU64")?)?,
            imports_sequence_kind: u8::try_from(required_u64(sequence_kinds, "imports")?)
                .map_err(|_| StageOracleError::InvalidType("sequenceKinds.imports".to_owned()))?,
            increment: parse_hex_u64(constants[0].as_str().ok_or_else(|| {
                StageOracleError::InvalidType("permutation.splitmix64ConstantsHexU64[0]".to_owned())
            })?)?,
            multiplier_1: parse_hex_u64(constants[1].as_str().ok_or_else(|| {
                StageOracleError::InvalidType("permutation.splitmix64ConstantsHexU64[1]".to_owned())
            })?)?,
            multiplier_2: parse_hex_u64(constants[2].as_str().ok_or_else(|| {
                StageOracleError::InvalidType("permutation.splitmix64ConstantsHexU64[2]".to_owned())
            })?)?,
        })
    }

    fn permute_imports<T>(&self, values: &mut [T], module_seed_ordinal: u64) {
        let mut state =
            self.base_seed ^ (u64::from(self.imports_sequence_kind) << 56) ^ module_seed_ordinal;
        for index in (1..values.len()).rev() {
            state = state.wrapping_add(self.increment);
            let mut value = state;
            value = (value ^ (value >> 30)).wrapping_mul(self.multiplier_1);
            value = (value ^ (value >> 27)).wrapping_mul(self.multiplier_2);
            value ^= value >> 31;
            let modulus = u64::try_from(index + 1).expect("slice length must fit u64");
            let swap_index = usize::try_from(value % modulus).expect("swap index must fit usize");
            values.swap(index, swap_index);
        }
    }
}

fn oracle_module_seed_ordinal(module_name: &str) -> Result<u64, StageOracleError> {
    if module_name == "root" {
        return Ok(0);
    }
    if module_name == "shared/common" {
        return Ok(1);
    }
    if let Some(index) = module_name.strip_prefix("group/") {
        return Ok((1_u64 << 40) | u64::from(parse_hex_u32(index)?));
    }
    if let Some(index) = module_name.strip_prefix("unit/") {
        return Ok((2_u64 << 40) | u64::from(parse_hex_u32(index)?));
    }
    Err(StageOracleError::Invalid {
        path: "canonical module name".to_owned(),
        expected: format!(
            "root, shared/common, group/{{g:08x}}, or unit/{{i:08x}}; got {module_name}"
        ),
    })
}

fn parse_hex_u64(value: &str) -> Result<u64, StageOracleError> {
    if value.len() != 16
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(StageOracleError::InvalidType(format!(
            "lowercase hexadecimal u64 {value}"
        )));
    }
    u64::from_str_radix(value, 16)
        .map_err(|_| StageOracleError::InvalidType(format!("hexadecimal u64 {value}")))
}

fn parse_hex_u32(value: &str) -> Result<u32, StageOracleError> {
    if value.len() != 8
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(StageOracleError::InvalidType(format!(
            "lowercase hexadecimal u32 {value}"
        )));
    }
    u32::from_str_radix(value, 16)
        .map_err(|_| StageOracleError::InvalidType(format!("hexadecimal u32 {value}")))
}

#[derive(Clone, Copy, Debug)]
struct StageConstants {
    source_span_logical_bytes: u64,
    typed_ast_logical_record_bytes: u64,
    typed_ast_allocation_record_bytes: u64,
    hir_logical_record_bytes: u64,
    hir_allocation_record_bytes: u64,
    mir_lir_logical_record_bytes: u64,
    mir_lir_allocation_record_bytes: u64,
    declaration_token_bytes_with_lf: u64,
    reference_token_bytes_with_lf: u64,
    relation_token_bytes_with_lf: u64,
    geometry_token_bytes_with_lf: u64,
}

impl StageConstants {
    fn parse(manifest: &serde_json::Value) -> Result<Self, StageOracleError> {
        let source_span_rule = required_object(manifest, "sourceSpanRule")?;
        let token_lengths = required_object(source_span_rule, "sourceTokenByteLengthIncludingLf")?;
        let stage = required_object(manifest, "researchStageModel")?;
        let source_span = required_object(stage, "sourceSpanLayout")?;
        let typed_ast = required_object(stage, "typedAstRecordLayout")?;
        let hir = required_object(stage, "hirRecordLayout")?;
        let mir_lir = required_object(stage, "mirAndLirRecordLayout")?;
        Ok(Self {
            source_span_logical_bytes: required_u64(source_span, "logicalFieldBytes")?,
            typed_ast_logical_record_bytes: required_u64(typed_ast, "logicalFieldBytes")?,
            typed_ast_allocation_record_bytes: required_u64(typed_ast, "reprCSizeBytes")?,
            hir_logical_record_bytes: required_u64(hir, "logicalFieldBytes")?,
            hir_allocation_record_bytes: required_u64(hir, "reprCSizeBytes")?,
            mir_lir_logical_record_bytes: required_u64(mir_lir, "logicalFieldBytes")?,
            mir_lir_allocation_record_bytes: required_u64(mir_lir, "reprCSizeBytes")?,
            declaration_token_bytes_with_lf: required_u64(token_lengths, "declarations")?,
            reference_token_bytes_with_lf: required_u64(token_lengths, "references")?,
            relation_token_bytes_with_lf: required_u64(token_lengths, "relations")?,
            geometry_token_bytes_with_lf: required_u64(token_lengths, "geometry")?,
        })
    }
}

fn selected_graph_profile(
    manifest: &serde_json::Value,
    graph_profile: GraphProfileId,
) -> Result<&serde_json::Value, StageOracleError> {
    required_array(manifest, "moduleGraphProfiles")?
        .iter()
        .find(|profile| {
            profile.get("id").and_then(serde_json::Value::as_str) == Some(graph_profile.as_str())
        })
        .ok_or_else(|| StageOracleError::Missing(graph_profile.as_str().to_owned()))
}

fn selected_identity_workload(
    manifest: &serde_json::Value,
) -> Result<&serde_json::Value, StageOracleError> {
    required_array(manifest, "workloads")?
        .iter()
        .find(|workload| {
            workload.get("id").and_then(serde_json::Value::as_str) == Some(IDENTITY_WORKLOAD_ID)
        })
        .ok_or_else(|| StageOracleError::Missing(IDENTITY_WORKLOAD_ID.to_owned()))
}

fn stage_shape(
    record_count: u64,
    payload_logical_bytes: u64,
    logical_record_bytes: u64,
    allocation_record_bytes: u64,
) -> Result<StageShape, StageOracleError> {
    Ok(StageShape {
        record_count,
        payload_logical_bytes,
        logical_bytes: checked_add(
            "stage.logicalBytes",
            checked_mul("stage.records", logical_record_bytes, record_count)?,
            payload_logical_bytes,
        )?,
        record_allocation_bytes: checked_add(
            "stage.recordAllocationBytes",
            checked_mul("stage.records", allocation_record_bytes, record_count)?,
            payload_logical_bytes,
        )?,
    })
}

fn required_object<'a>(
    value: &'a serde_json::Value,
    field: &str,
) -> Result<&'a serde_json::Value, StageOracleError> {
    value
        .get(field)
        .filter(|candidate| candidate.is_object())
        .ok_or_else(|| StageOracleError::Missing(field.to_owned()))
}

fn required_array<'a>(
    value: &'a serde_json::Value,
    field: &str,
) -> Result<&'a [serde_json::Value], StageOracleError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| StageOracleError::Missing(field.to_owned()))
}

fn required_string<'a>(
    value: &'a serde_json::Value,
    field: &str,
) -> Result<&'a str, StageOracleError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| StageOracleError::InvalidType(field.to_owned()))
}

fn required_u64(value: &serde_json::Value, field: &str) -> Result<u64, StageOracleError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| StageOracleError::InvalidType(field.to_owned()))
}

fn required_u32(value: &serde_json::Value, field: &str) -> Result<u32, StageOracleError> {
    u32::try_from(required_u64(value, field)?)
        .map_err(|_| StageOracleError::InvalidType(field.to_owned()))
}

fn required_u16(value: &serde_json::Value, field: &str) -> Result<u16, StageOracleError> {
    u16::try_from(required_u64(value, field)?)
        .map_err(|_| StageOracleError::InvalidType(field.to_owned()))
}

fn sum(values: &[u64]) -> Result<u64, StageOracleError> {
    values.iter().try_fold(0_u64, |total, value| {
        checked_add("stage formula", total, *value)
    })
}

fn checked_add(field: &'static str, left: u64, right: u64) -> Result<u64, StageOracleError> {
    left.checked_add(right)
        .ok_or(StageOracleError::Overflow(field))
}

fn checked_mul(field: &'static str, left: u64, right: u64) -> Result<u64, StageOracleError> {
    left.checked_mul(right)
        .ok_or(StageOracleError::Overflow(field))
}

fn as_u64(value: usize, field: &'static str) -> Result<u64, StageOracleError> {
    u64::try_from(value).map_err(|_| StageOracleError::Overflow(field))
}

#[derive(Debug, thiserror::Error)]
pub enum StageOracleError {
    #[error(transparent)]
    Identity(#[from] ExactOracleError),
    #[error("阶段独立预言机缺少清单路径 {0}")]
    Missing(String),
    #[error("阶段独立预言机清单字段类型错误：{0}")]
    InvalidType(String),
    #[error("阶段独立预言机字段 {path} 不匹配：期望 {expected}")]
    Invalid { path: String, expected: String },
    #[error("阶段独立预言机算术溢出：{0}")]
    Overflow(&'static str),
    #[error("生产者与阶段独立预言机的精确内容不一致：{0}")]
    Mismatch(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{load_repository_contract, pipeline::build_identity_stage_case};

    #[test]
    fn independently_recomputed_n1_shapes_match_frozen_values() {
        let trusted = load_repository_contract().expect("frozen contract");
        let expected = [
            (GraphProfileId::WideStar, 4_839, 7_743, 6_003),
            (GraphProfileId::DeepChain, 4_760, 7_644, 5_939),
            (GraphProfileId::SharedFaninDag, 5_250, 8_174, 6_401),
        ];
        for (profile, source_input, typed_ast, hir) in expected {
            let summary = build_identity_stage_oracle(&trusted.workload_manifest, profile, 1)
                .expect("stage oracle");
            assert_eq!(summary.stages.source_input.logical_bytes, source_input);
            assert_eq!(summary.stages.typed_ast.logical_bytes, typed_ast);
            assert_eq!(summary.stages.hir.logical_bytes, hir);
            assert_eq!(summary.stages.canonical_lir.logical_bytes, 3_334);
            assert_eq!(summary.stages.output_construction.logical_bytes, 3_132);
        }
    }

    #[test]
    fn exact_stage_oracle_rejects_same_length_string_substitution() {
        let trusted = load_repository_contract().expect("frozen contract");
        let generator = trusted.generator_contract().expect("generator");
        let identity = trusted.identity_contract().expect("identity");
        let stage = trusted.stage_contract().expect("stage");
        let mut produced =
            build_identity_stage_case(&generator, &identity, &stage, GraphProfileId::WideStar, 2)
                .expect("stage case");
        let first_string_byte = produced.source_string_range.start;
        produced.source_input_payload[first_string_byte] ^= 1;

        assert!(matches!(
            verify_identity_stage_exact(
                &trusted.workload_manifest,
                GraphProfileId::WideStar,
                2,
                &produced,
            ),
            Err(StageOracleError::Mismatch("source string bytes"))
        ));
    }
}
