use crate::identity::{IDENTITY_WORKLOAD_ID, IdentityContract, IdentityFieldValue};
use crate::pipeline::build_identity_stage_case;
use crate::{GraphProfileId, TrustedContract};
use serde::{Deserialize, Serialize};

const SHORT_UNIQUE_PROFILE_ID: &str = "short-unique-v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageContract {
    pub(crate) absent_ordinal: u32,
    pub(crate) record_kind_module: u16,
    pub(crate) record_kind_import: u16,
    pub(crate) record_kind_declaration: u16,
    pub(crate) record_kind_identity_field: u16,
    pub(crate) record_kind_reference: u16,
    pub(crate) record_kind_relation: u16,
    pub(crate) record_kind_geometry: u16,
    pub(crate) record_kind_symbol: u16,
    pub(crate) declaration_token: String,
    pub(crate) reference_token: String,
    pub(crate) relation_token: String,
    pub(crate) geometry_token: String,
    pub(crate) declaration_token_bytes_with_lf: usize,
    pub(crate) reference_token_bytes_with_lf: usize,
    pub(crate) relation_token_bytes_with_lf: usize,
    pub(crate) geometry_token_bytes_with_lf: usize,
    pub(crate) shared_constant_name: String,
    pub(crate) shared_constant_value: String,
}

impl StageContract {
    pub fn from_manifest(manifest: &serde_json::Value) -> Result<Self, StageContractError> {
        let source_span = object(manifest, "sourceSpanRule")?;
        string(
            source_span,
            "sourceTokenFormula",
            "sourceTokenKindUtf8 || '/' || kindLocalOrdinalHex8",
        )?;
        string(
            source_span,
            "sourceDocumentBytesFormula",
            "concatenate(sourceTokenUtf8 || LF in canonicalSequenceOrder)",
        )?;
        string_array(
            source_span,
            "canonicalSequenceOrder",
            &["declarations", "references", "relations", "geometry"],
        )?;
        let token_kinds = object(source_span, "sourceTokenKindUtf8")?;
        let token_lengths = object(source_span, "sourceTokenByteLengthIncludingLf")?;

        let stage = object(manifest, "researchStageModel")?;
        integer(stage, "version", 1)?;
        string(stage, "scope", "non-production-research-shape-only")?;
        string(
            stage,
            "storageRule",
            "one contiguous value buffer plus one contiguous payload byte buffer per stage; no per-record heap allocation",
        )?;
        integer(stage, "absentOrdinalU32", u64::from(u32::MAX))?;
        let record_kinds = object(stage, "stageRecordKindCodes")?;
        layout(object(stage, "sourceSpanLayout")?, 20, 20)?;
        layout(object(stage, "typedAstRecordLayout")?, 32, 32)?;
        layout(object(stage, "hirRecordLayout")?, 32, 32)?;
        layout(object(stage, "mirAndLirRecordLayout")?, 44, 48)?;

        let string_model = object(stage, "stringAggregateModel")?;
        string(
            string_model,
            "canonicalModuleOrder",
            "root, shared/common when present, group/{g:08x} by g, unit/{i:08x} by i",
        )?;
        string(
            string_model,
            "sourceDocumentKeyFormula",
            "source/{graphProfileId}/{canonicalModuleName}.lfsynthetic",
        )?;
        let namespace = object(string_model, "namespaceString")?;
        string(namespace, "encoding", "lowercase-ascii-hex")?;
        integer(namespace, "byteLength", 32)?;
        let source_reference = object(string_model, "sourceReferenceSpelling")?;
        string(
            source_reference,
            "formula",
            "reference/{targetKindCodeHex2}/{targetModuleOrdinalHex8}/{targetLocalOrdinalHex8}",
        )?;
        string(source_reference, "encoding", "ascii")?;
        integer(source_reference, "byteLength", 30)?;
        string(source_reference, "sharedConstantTargetKindCodeHex2", "ff")?;
        let shared = object(string_model, "sharedConstantStrings")?;
        let shared_constant_name = required_string(shared, "nameUtf8")?.to_owned();
        let shared_constant_value = required_string(shared, "valueUtf8")?.to_owned();
        integer(
            shared,
            "nameByteLength",
            as_u64(shared_constant_name.len(), "shared constant name")?,
        )?;
        integer(
            shared,
            "valueByteLength",
            as_u64(shared_constant_value.len(), "shared constant value")?,
        )?;

        validate_short_unique_profile(manifest)?;
        validate_identity_stage_inputs(manifest)?;

        Ok(Self {
            absent_ordinal: u32::MAX,
            record_kind_module: required_u16(record_kinds, "module")?,
            record_kind_import: required_u16(record_kinds, "import")?,
            record_kind_declaration: required_u16(record_kinds, "declaration")?,
            record_kind_identity_field: required_u16(record_kinds, "identityField")?,
            record_kind_reference: required_u16(record_kinds, "referenceOrResolvedReference")?,
            record_kind_relation: required_u16(record_kinds, "relation")?,
            record_kind_geometry: required_u16(record_kinds, "geometry")?,
            record_kind_symbol: required_u16(record_kinds, "symbol")?,
            declaration_token: required_string(token_kinds, "declarations")?.to_owned(),
            reference_token: required_string(token_kinds, "references")?.to_owned(),
            relation_token: required_string(token_kinds, "relations")?.to_owned(),
            geometry_token: required_string(token_kinds, "geometry")?.to_owned(),
            declaration_token_bytes_with_lf: required_usize(token_lengths, "declarations")?,
            reference_token_bytes_with_lf: required_usize(token_lengths, "references")?,
            relation_token_bytes_with_lf: required_usize(token_lengths, "relations")?,
            geometry_token_bytes_with_lf: required_usize(token_lengths, "geometry")?,
            shared_constant_name,
            shared_constant_value,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityStageSummary {
    pub graph_profile: GraphProfileId,
    pub n: u32,
    pub counts: IdentityAggregateCounts,
    pub stages: StageBreakdown,
    pub semantic_digest_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityStagePlanSummary {
    pub graph_profile: GraphProfileId,
    pub n: u32,
    pub counts: IdentityAggregateCounts,
    pub stages: StageBreakdown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityAggregateCounts {
    pub module_count: u64,
    pub import_edge_count: u64,
    pub cross_module_reference_count: u64,
    pub maximum_import_depth: u64,
    pub source_document_count: u64,
    pub source_byte_count: u64,
    pub identity_declaration_count: u64,
    pub source_declaration_count: u64,
    pub source_span_count: u64,
    pub identity_field_occurrence_count: u64,
    pub profiled_key_occurrence_count: u64,
    pub source_reference_count: u64,
    pub source_relation_count: u64,
    pub source_geometry_count: u64,
    pub symbol_count: u64,
    pub string_item_count: u64,
    pub maximum_string_bytes: u64,
    pub total_string_bytes: u64,
    pub diagnostic_count: u64,
    pub semantic_output_record: u64,
    pub semantic_payload_byte_count: u64,
    pub logical_byte_count: u64,
    pub output_byte_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageBreakdown {
    pub source_input: StageShape,
    pub typed_ast: StageShape,
    pub hir: StageShape,
    pub mir: StageShape,
    pub canonical_lir: StageShape,
    pub diagnostics: StageShape,
    pub scratch: StageShape,
    pub output_construction: StageShape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageShape {
    pub record_count: u64,
    pub payload_logical_bytes: u64,
    pub logical_bytes: u64,
    pub record_allocation_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageRetainedCapacityBytes {
    pub source_input: u64,
    pub typed_ast: u64,
    pub hir: u64,
    pub mir: u64,
    pub canonical_lir: u64,
    pub diagnostics: u64,
    pub scratch: u64,
    pub output_construction: u64,
    pub total: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SourceSpanRecord {
    pub(crate) source_document_ordinal: u32,
    pub(crate) start_line: u32,
    pub(crate) start_column: u32,
    pub(crate) end_line: u32,
    pub(crate) end_column: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TypedAstStageRecord {
    pub(crate) record_kind: u16,
    pub(crate) entity_kind: u16,
    pub(crate) module_ordinal: u32,
    pub(crate) source_span_ordinal: u32,
    pub(crate) owner_local_index: u32,
    pub(crate) payload_offset: u64,
    pub(crate) payload_length: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HirStageRecord {
    pub(crate) record_kind: u16,
    pub(crate) entity_kind: u16,
    pub(crate) module_ordinal: u32,
    pub(crate) symbol_ordinal: u32,
    pub(crate) resolved_target_ordinal: u32,
    pub(crate) payload_offset: u64,
    pub(crate) payload_length: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MirLirStageRecord {
    pub(crate) record_kind: u16,
    pub(crate) entity_kind: u16,
    pub(crate) stable_id: [u8; 16],
    pub(crate) owner_ordinal: u32,
    pub(crate) local_index: u32,
    pub(crate) payload_offset: u64,
    pub(crate) payload_length: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IdentityStageCaseOutput {
    pub(crate) summary: IdentityStageSummary,
    pub(crate) string_bytes: Vec<u8>,
    pub(crate) source_spans: Vec<SourceSpanRecord>,
    pub(crate) source_input_records: Vec<TypedAstStageRecord>,
    pub(crate) source_input_payload: Vec<u8>,
    pub(crate) typed_ast_records: Vec<TypedAstStageRecord>,
    pub(crate) typed_ast_payload: Vec<u8>,
    pub(crate) hir_records: Vec<HirStageRecord>,
    pub(crate) hir_payload: Vec<u8>,
    pub(crate) mir_records: Vec<MirLirStageRecord>,
    pub(crate) mir_payload: Vec<u8>,
    pub(crate) canonical_lir_records: Vec<MirLirStageRecord>,
    pub(crate) canonical_lir_payload: Vec<u8>,
    pub(crate) diagnostics: Vec<u8>,
    pub(crate) scratch_capacity_bytes: u64,
    pub(crate) output_construction: Vec<u8>,
    pub(crate) source_scratch: Vec<u64>,
    pub(crate) namespace_preimage_scratch: Vec<u8>,
    pub(crate) mir_stable_id_scratch: Vec<[u8; 16]>,
    pub(crate) mir_canonical_identity_scratch: Vec<u8>,
    pub(crate) mir_identity_payload_scratch: Vec<u8>,
    pub(crate) lir_sort_scratch: Vec<usize>,
    pub(crate) lir_owner_ordinal_scratch: Vec<u32>,
}

#[derive(Clone, Debug)]
pub(crate) struct IdentityStagePlan {
    pub(crate) graph_profile: GraphProfileId,
    pub(crate) n: u32,
    pub(crate) group_count: u32,
    pub(crate) unit_module_base: u32,
    pub(crate) binding_count: u32,
    pub(crate) profiled_fields_per_unit: u32,
    pub(crate) stable_fields_per_unit: u32,
    pub(crate) relation_count_per_unit: u32,
    pub(crate) counts: IdentityAggregateCounts,
    pub(crate) stages: StageBreakdown,
}

impl IdentityStagePlan {
    pub(crate) fn prepare(
        identity: &IdentityContract,
        stage: &StageContract,
        graph_profile: GraphProfileId,
        n: u32,
    ) -> Result<Self, StageGenerationError> {
        if n == 0 {
            return Err(StageGenerationError::ScaleMustBePositive);
        }

        let group_count = if graph_profile == GraphProfileId::SharedFaninDag {
            n.div_ceil(64)
        } else {
            0
        };
        let (module_count, import_edge_count, cross_module_reference_count, maximum_import_depth) =
            match graph_profile {
                GraphProfileId::WideStar => (u64::from(n) + 1, u64::from(n), u64::from(n), 1),
                GraphProfileId::DeepChain => (
                    u64::from(n) + 1,
                    u64::from(n),
                    u64::from(n.saturating_sub(1)),
                    u64::from(n),
                ),
                GraphProfileId::SharedFaninDag => (
                    u64::from(n) + u64::from(group_count) + 2,
                    checked_add(
                        "importEdgeCount",
                        checked_mul("importEdgeCount", 2, u64::from(n))?,
                        u64::from(group_count),
                    )?,
                    u64::from(n),
                    3,
                ),
            };
        let unit_module_base = if graph_profile == GraphProfileId::SharedFaninDag {
            group_count
                .checked_add(2)
                .ok_or(StageGenerationError::Overflow("unitModuleBase"))?
        } else {
            1
        };

        let binding_count = u32::try_from(identity.bindings.len())
            .map_err(|_| StageGenerationError::Overflow("identity bindings"))?;
        let relation_count_per_unit = u32::try_from(identity.owner_relations.len())
            .map_err(|_| StageGenerationError::Overflow("owner relations"))?;
        let identity_field_occurrences_per_unit =
            identity.bindings.iter().try_fold(0_u64, |total, binding| {
                checked_add(
                    "identityFieldOccurrenceCount",
                    total,
                    as_u64(binding.fields.len(), "identity fields")?,
                )
            })?;
        let profiled_fields_per_unit = count_identity_fields(identity, |value| {
            matches!(value, IdentityFieldValue::ProfiledKey { .. })
        })?;
        let stable_fields_per_unit = count_identity_fields(identity, |value| {
            matches!(value, IdentityFieldValue::StableId { .. })
        })?;

        let identity_declaration_count = checked_mul(
            "identityDeclarationCount",
            u64::from(binding_count),
            u64::from(n),
        )?;
        let source_declaration_count = checked_add(
            "sourceDeclarationCount",
            identity_declaration_count,
            u64::from(graph_profile == GraphProfileId::SharedFaninDag),
        )?;
        let identity_field_occurrence_count = checked_mul(
            "identityFieldOccurrenceCount",
            identity_field_occurrences_per_unit,
            u64::from(n),
        )?;
        let profiled_key_occurrence_count = checked_mul(
            "profiledKeyOccurrenceCount",
            u64::from(profiled_fields_per_unit),
            u64::from(n),
        )?;
        let source_relation_count = checked_mul(
            "sourceRelationCount",
            u64::from(relation_count_per_unit),
            u64::from(n),
        )?;
        let source_reference_count = sum(&[
            checked_mul(
                "sourceReferenceCount",
                u64::from(stable_fields_per_unit),
                u64::from(n),
            )?,
            source_relation_count,
            cross_module_reference_count,
        ])?;
        let source_geometry_count = 0;
        let source_span_count = sum(&[
            source_declaration_count,
            source_reference_count,
            source_relation_count,
            source_geometry_count,
        ])?;
        let source_byte_count = sum(&[
            checked_mul(
                "sourceByteCount",
                as_u64(
                    stage.declaration_token_bytes_with_lf,
                    "declaration token bytes",
                )?,
                source_declaration_count,
            )?,
            checked_mul(
                "sourceByteCount",
                as_u64(stage.reference_token_bytes_with_lf, "reference token bytes")?,
                source_reference_count,
            )?,
            checked_mul(
                "sourceByteCount",
                as_u64(stage.relation_token_bytes_with_lf, "relation token bytes")?,
                source_relation_count,
            )?,
            checked_mul(
                "sourceByteCount",
                as_u64(stage.geometry_token_bytes_with_lf, "geometry token bytes")?,
                source_geometry_count,
            )?,
        ])?;

        let module_name_bytes = module_name_bytes(graph_profile, n, group_count)?;
        let maximum_module_name_bytes = if graph_profile == GraphProfileId::SharedFaninDag {
            14
        } else {
            13
        };
        let source_document_prefix_bytes = as_u64(
            "source/".len() + graph_profile.as_str().len() + 1 + ".lfsynthetic".len(),
            "source document prefix",
        )?;
        let source_document_key_bytes = checked_add(
            "sourceDocumentKeyByteCount",
            checked_mul(
                "sourceDocumentKeyByteCount",
                source_document_prefix_bytes,
                module_count,
            )?,
            module_name_bytes,
        )?;
        let import_target_name_bytes = match graph_profile {
            GraphProfileId::WideStar | GraphProfileId::DeepChain => {
                checked_mul("importTargetModuleNameByteCount", 13, u64::from(n))?
            }
            GraphProfileId::SharedFaninDag => checked_add(
                "importTargetModuleNameByteCount",
                checked_mul(
                    "importTargetModuleNameByteCount",
                    14,
                    u64::from(group_count),
                )?,
                checked_mul("importTargetModuleNameByteCount", 26, u64::from(n))?,
            )?,
        };
        let shared_string_count = u64::from(graph_profile == GraphProfileId::SharedFaninDag) * 2;
        let shared_string_bytes = if graph_profile == GraphProfileId::SharedFaninDag {
            as_u64(
                stage.shared_constant_name.len() + stage.shared_constant_value.len(),
                "shared string bytes",
            )?
        } else {
            0
        };
        let string_item_count = sum(&[
            module_count,
            module_count,
            import_edge_count,
            identity_declaration_count,
            profiled_key_occurrence_count,
            source_reference_count,
            shared_string_count,
        ])?;
        let total_string_bytes = sum(&[
            module_name_bytes,
            source_document_key_bytes,
            import_target_name_bytes,
            checked_mul("namespaceStringBytes", 32, identity_declaration_count)?,
            checked_mul("profiledKeyStringBytes", 20, profiled_key_occurrence_count)?,
            checked_mul("sourceReferenceStringBytes", 30, source_reference_count)?,
            shared_string_bytes,
        ])?;
        let maximum_string_bytes = [
            maximum_module_name_bytes,
            checked_add(
                "maximumSourceDocumentKeyBytes",
                source_document_prefix_bytes,
                maximum_module_name_bytes,
            )?,
            maximum_module_name_bytes,
            32,
            20,
            30,
            u64::try_from(
                stage
                    .shared_constant_name
                    .len()
                    .max(stage.shared_constant_value.len()),
            )
            .map_err(|_| StageGenerationError::Overflow("maximum shared string bytes"))?,
        ]
        .into_iter()
        .max()
        .expect("non-empty maximum candidates");

        let semantic_output_record = checked_mul(
            "semanticOutputRecord",
            u64::from(
                binding_count
                    .checked_add(relation_count_per_unit)
                    .ok_or(StageGenerationError::Overflow("semanticOutputRecord"))?,
            ),
            u64::from(n),
        )?;
        let semantic_payload_per_unit =
            identity.bindings.iter().try_fold(0_u64, |total, binding| {
                let field_bytes = binding
                    .fields
                    .iter()
                    .try_fold(0_u64, |field_total, field| {
                        let value_bytes = match field.value {
                            IdentityFieldValue::Namespace => 32,
                            IdentityFieldValue::ProfiledKey { .. } => 20,
                            IdentityFieldValue::StableId { .. } => 16,
                        };
                        checked_add(
                            "semanticPayloadByteCount",
                            field_total,
                            checked_add("semanticPayloadByteCount", 6, value_bytes)?,
                        )
                    })?;
                checked_add(
                    "semanticPayloadByteCount",
                    total,
                    checked_add("semanticPayloadByteCount", 2, field_bytes)?,
                )
            })?;
        let semantic_payload_per_unit = checked_add(
            "semanticPayloadByteCount",
            semantic_payload_per_unit,
            checked_mul(
                "semanticPayloadByteCount",
                18,
                u64::from(relation_count_per_unit),
            )?,
        )?;
        let semantic_payload_byte_count = checked_mul(
            "semanticPayloadByteCount",
            semantic_payload_per_unit,
            u64::from(n),
        )?;

        let source_input_record_count = sum(&[module_count, import_edge_count, source_span_count])?;
        let source_input_payload =
            checked_add("sourceInput.payload", source_byte_count, total_string_bytes)?;
        let source_input = stage_shape(source_input_record_count, source_input_payload, 32, 32)?;
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
            checked_mul("typedAst.sourceSpans", 20, source_span_count)?,
        ])?;
        let typed_ast = stage_shape(typed_ast_record_count, typed_ast_payload, 32, 32)?;
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
        let hir = stage_shape(hir_record_count, hir_payload, 32, 32)?;
        let mir = stage_shape(semantic_output_record, semantic_payload_byte_count, 44, 48)?;
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

        Ok(Self {
            graph_profile,
            n,
            group_count,
            unit_module_base,
            binding_count,
            profiled_fields_per_unit,
            stable_fields_per_unit,
            relation_count_per_unit,
            counts,
            stages,
        })
    }

    pub(crate) fn summary(&self, semantic_digest_sha256: String) -> IdentityStageSummary {
        IdentityStageSummary {
            graph_profile: self.graph_profile,
            n: self.n,
            counts: self.counts.clone(),
            stages: self.stages.clone(),
            semantic_digest_sha256,
        }
    }

    pub(crate) fn plan_summary(&self) -> IdentityStagePlanSummary {
        IdentityStagePlanSummary {
            graph_profile: self.graph_profile,
            n: self.n,
            counts: self.counts.clone(),
            stages: self.stages.clone(),
        }
    }
}

pub fn build_identity_stage_plan_summary(
    trusted: &TrustedContract,
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<IdentityStagePlanSummary, StageGenerationError> {
    let identity = trusted.identity_contract()?;
    let stage = StageContract::from_manifest(&trusted.workload_manifest)?;
    Ok(IdentityStagePlan::prepare(&identity, &stage, graph_profile, n)?.plan_summary())
}

pub fn build_identity_stage_summary(
    trusted: &TrustedContract,
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<IdentityStageSummary, StageGenerationError> {
    let generator = trusted.generator_contract()?;
    let identity = trusted.identity_contract()?;
    let stage = StageContract::from_manifest(&trusted.workload_manifest)?;
    Ok(build_identity_stage_case(&generator, &identity, &stage, graph_profile, n)?.summary)
}

fn count_identity_fields(
    identity: &IdentityContract,
    predicate: impl Fn(IdentityFieldValue) -> bool,
) -> Result<u32, StageGenerationError> {
    identity.bindings.iter().try_fold(0_u32, |total, binding| {
        let count = u32::try_from(
            binding
                .fields
                .iter()
                .filter(|field| predicate(field.value))
                .count(),
        )
        .map_err(|_| StageGenerationError::Overflow("identity field count"))?;
        total
            .checked_add(count)
            .ok_or(StageGenerationError::Overflow("identity field count"))
    })
}

fn module_name_bytes(
    graph_profile: GraphProfileId,
    n: u32,
    group_count: u32,
) -> Result<u64, StageGenerationError> {
    match graph_profile {
        GraphProfileId::WideStar | GraphProfileId::DeepChain => checked_add(
            "canonicalModuleNameByteCount",
            4,
            checked_mul("canonicalModuleNameByteCount", 13, u64::from(n))?,
        ),
        GraphProfileId::SharedFaninDag => sum(&[
            4,
            13,
            checked_mul("canonicalModuleNameByteCount", 14, u64::from(group_count))?,
            checked_mul("canonicalModuleNameByteCount", 13, u64::from(n))?,
        ]),
    }
}

fn stage_shape(
    record_count: u64,
    payload_logical_bytes: u64,
    logical_record_bytes: u64,
    allocated_record_bytes: u64,
) -> Result<StageShape, StageGenerationError> {
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
            checked_mul("stage.records", allocated_record_bytes, record_count)?,
            payload_logical_bytes,
        )?,
    })
}

fn validate_short_unique_profile(manifest: &serde_json::Value) -> Result<(), StageContractError> {
    let profile = array(manifest, "stringProfiles")?
        .iter()
        .find(|profile| {
            profile.get("id").and_then(serde_json::Value::as_str) == Some(SHORT_UNIQUE_PROFILE_ID)
        })
        .ok_or_else(|| StageContractError::Missing("stringProfiles[short-unique-v1]".to_owned()))?;
    string(
        profile,
        "profiledKeyFormula",
        "kindCodeHex2 || '/' || unitIndexHex8 || '/' || localIndexHex8",
    )?;
    integer(profile, "profiledKeyLengthBytes", 20)?;
    string(profile, "encoding", "ascii")
}

fn validate_identity_stage_inputs(manifest: &serde_json::Value) -> Result<(), StageContractError> {
    let workload = array(manifest, "workloads")?
        .iter()
        .find(|workload| {
            workload.get("id").and_then(serde_json::Value::as_str) == Some(IDENTITY_WORKLOAD_ID)
        })
        .ok_or_else(|| StageContractError::Missing("workloads[LF-COMP-ID-v1]".to_owned()))?;
    let inputs = object(workload, "perUnitStageInputs")?;
    for (field, expected) in [
        ("sourceDeclarationCount", 22),
        ("identityFieldOccurrenceCount", 57),
        ("profiledKeyOccurrenceCount", 24),
        ("sourceReferenceCount", 21),
        ("sourceRelationCount", 10),
        ("sourceGeometryCount", 0),
    ] {
        integer(inputs, field, expected)?;
    }
    Ok(())
}

fn layout(
    value: &serde_json::Value,
    logical_bytes: u64,
    repr_c_bytes: u64,
) -> Result<(), StageContractError> {
    integer(value, "logicalFieldBytes", logical_bytes)?;
    integer(value, "reprCSizeBytes", repr_c_bytes)
}

fn object<'a>(
    value: &'a serde_json::Value,
    field: &str,
) -> Result<&'a serde_json::Value, StageContractError> {
    value
        .get(field)
        .filter(|candidate| candidate.is_object())
        .ok_or_else(|| StageContractError::Missing(field.to_owned()))
}

fn array<'a>(
    value: &'a serde_json::Value,
    field: &str,
) -> Result<&'a [serde_json::Value], StageContractError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| StageContractError::Missing(field.to_owned()))
}

fn required_string<'a>(
    value: &'a serde_json::Value,
    field: &str,
) -> Result<&'a str, StageContractError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| StageContractError::Missing(field.to_owned()))
}

fn required_u16(value: &serde_json::Value, field: &str) -> Result<u16, StageContractError> {
    let number = value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| StageContractError::Missing(field.to_owned()))?;
    u16::try_from(number).map_err(|_| StageContractError::Invalid(field.to_owned()))
}

fn required_usize(value: &serde_json::Value, field: &str) -> Result<usize, StageContractError> {
    let number = value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| StageContractError::Missing(field.to_owned()))?;
    usize::try_from(number).map_err(|_| StageContractError::Invalid(field.to_owned()))
}

fn string(
    value: &serde_json::Value,
    field: &str,
    expected: &str,
) -> Result<(), StageContractError> {
    if required_string(value, field)? != expected {
        return Err(StageContractError::Invalid(field.to_owned()));
    }
    Ok(())
}

fn integer(
    value: &serde_json::Value,
    field: &str,
    expected: u64,
) -> Result<(), StageContractError> {
    if value.get(field).and_then(serde_json::Value::as_u64) != Some(expected) {
        return Err(StageContractError::Invalid(field.to_owned()));
    }
    Ok(())
}

fn string_array(
    value: &serde_json::Value,
    field: &str,
    expected: &[&str],
) -> Result<(), StageContractError> {
    let actual = array(value, field)?
        .iter()
        .map(|item| {
            item.as_str()
                .ok_or_else(|| StageContractError::Invalid(format!("{field}[]")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if actual != expected {
        return Err(StageContractError::Invalid(field.to_owned()));
    }
    Ok(())
}

pub(crate) fn sum(values: &[u64]) -> Result<u64, StageGenerationError> {
    values
        .iter()
        .try_fold(0_u64, |total, value| checked_add("sum", total, *value))
}

pub(crate) fn checked_add(
    field: &'static str,
    left: u64,
    right: u64,
) -> Result<u64, StageGenerationError> {
    left.checked_add(right)
        .ok_or(StageGenerationError::Overflow(field))
}

pub(crate) fn checked_mul(
    field: &'static str,
    left: u64,
    right: u64,
) -> Result<u64, StageGenerationError> {
    left.checked_mul(right)
        .ok_or(StageGenerationError::Overflow(field))
}

pub(crate) fn as_u64(value: usize, field: &'static str) -> Result<u64, StageGenerationError> {
    u64::try_from(value).map_err(|_| StageGenerationError::Overflow(field))
}

pub(crate) fn to_usize(value: u64, field: &'static str) -> Result<usize, StageGenerationError> {
    usize::try_from(value).map_err(|_| StageGenerationError::Overflow(field))
}

#[derive(Debug, thiserror::Error)]
pub enum StageContractError {
    #[error("阶段模型清单缺少路径 {0}")]
    Missing(String),
    #[error("阶段模型清单字段不匹配：{0}")]
    Invalid(String),
    #[error("阶段模型清单算术溢出：{0}")]
    Overflow(&'static str),
}

impl From<StageGenerationError> for StageContractError {
    fn from(value: StageGenerationError) -> Self {
        match value {
            StageGenerationError::Overflow(field) => Self::Overflow(field),
            _ => Self::Invalid("unexpected stage-generation error".to_owned()),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StageGenerationError {
    #[error(transparent)]
    GeneratorContract(#[from] crate::ManifestContractError),
    #[error(transparent)]
    IdentityContract(#[from] crate::IdentityContractError),
    #[error(transparent)]
    StageContract(#[from] StageContractError),
    #[error("研究工作负载规模 N 必须至少为 1")]
    ScaleMustBePositive,
    #[error("阶段模型算术溢出：{0}")]
    Overflow(&'static str),
    #[error("研究阶段容量预留失败：{field}")]
    AllocationFailed {
        field: &'static str,
        #[source]
        source: std::collections::TryReserveError,
    },
    #[error("受控分配容量算术溢出：{0}")]
    ControlledAllocationCapacityOverflow(&'static str),
    #[error("受控分配系统容量预留失败：{0}")]
    ControlledAllocationFailed(&'static str),
    #[error(
        "guard/allocation-hard-ceiling：字段 {field} 的容量请求将越过受控分配硬上限；hardCeilingBytes={hard_ceiling_bytes}, liveRequestedBytes={live_requested_bytes}, requestedBytes={requested_bytes}"
    )]
    ControlledAllocationHardCeiling {
        field: &'static str,
        hard_ceiling_bytes: u64,
        live_requested_bytes: u64,
        requested_bytes: u64,
    },
    #[error("阶段模型缺少实体种类 {0}")]
    MissingEntityKind(u16),
    #[error("阶段模型无法解析模块序号 {0}")]
    InvalidModuleOrdinal(u32),
    #[error("阶段模型无法解析来源引用")]
    InvalidSourceReference,
    #[error("阶段模型符号解析不一致：module={module_ordinal}, kind={entity_kind}")]
    InvalidSymbol {
        module_ordinal: u32,
        entity_kind: u16,
    },
    #[error("阶段模型记录或载荷不一致：{0}")]
    MaterializedMismatch(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_repository_contract;

    #[test]
    fn repr_c_layouts_match_the_frozen_stage_contract() {
        assert_eq!(std::mem::size_of::<SourceSpanRecord>(), 20);
        assert_eq!(std::mem::size_of::<TypedAstStageRecord>(), 32);
        assert_eq!(std::mem::size_of::<HirStageRecord>(), 32);
        assert_eq!(std::mem::size_of::<MirLirStageRecord>(), 48);
    }

    #[test]
    fn identity_stage_summaries_match_the_frozen_n1_shapes() {
        let trusted = load_repository_contract().expect("frozen contract");
        let expected = [
            (GraphProfileId::WideStar, 57, 4_839, 114, 7_743, 114, 6_003),
            (GraphProfileId::DeepChain, 56, 4_760, 113, 7_644, 113, 5_939),
            (
                GraphProfileId::SharedFaninDag,
                62,
                5_250,
                119,
                8_174,
                119,
                6_401,
            ),
        ];
        for (
            graph_profile,
            source_records,
            source_bytes,
            typed_records,
            typed_bytes,
            hir_records,
            hir_bytes,
        ) in expected
        {
            let summary =
                build_identity_stage_summary(&trusted, graph_profile, 1).expect("stage summary");
            assert_eq!(summary.stages.source_input.record_count, source_records);
            assert_eq!(summary.stages.source_input.logical_bytes, source_bytes);
            assert_eq!(summary.stages.typed_ast.record_count, typed_records);
            assert_eq!(summary.stages.typed_ast.logical_bytes, typed_bytes);
            assert_eq!(summary.stages.hir.record_count, hir_records);
            assert_eq!(summary.stages.hir.logical_bytes, hir_bytes);
            assert_eq!(summary.stages.mir.logical_bytes, 3_334);
            assert_eq!(summary.stages.canonical_lir.logical_bytes, 3_334);
            assert_eq!(summary.stages.scratch.logical_bytes, 256);
            assert_eq!(summary.stages.output_construction.logical_bytes, 3_132);
        }
    }
}
