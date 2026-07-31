//! 可扩展工作负载的计时区外规模计划。
//!
//! 正式停止护栏不能为了预测下一级而先物化下一级管线。本模块从受信任清单的模块图、
//! 阶段公式和每工作单元计数建立 O(1) 规模计划；模板型工作负载只在执行器启动时用
//! `N = 1` 的固定单位规范记录校准语义载荷常数。生产者对照测试另行覆盖三种模块图
//! 及共享汇入分组边界。

use crate::corridor::{CorridorTemplate, template_semantic_payload_bytes_per_unit};
use crate::junction_grid::build_junction_grid_template;
use crate::stage::IdentityStagePlan;
use crate::{
    CorridorContract, GraphProfileId, IdentityAggregateCounts, IdentityContract,
    JunctionGridContract, ScalableWorkloadId, StageBreakdown, StageContract, StageShape,
    TrustedContract,
};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScalableStagePlanSummary {
    pub workload_id: ScalableWorkloadId,
    pub graph_profile: GraphProfileId,
    pub n: u32,
    pub counts: IdentityAggregateCounts,
    pub stages: StageBreakdown,
    pub primary_record_count: u64,
}

#[derive(Clone, Debug)]
pub struct ScalableStagePlanFactory {
    identity: IdentityContract,
    stage: StageContract,
    corridor: Option<TemplatePlanBasis>,
    junction_grid: Option<TemplatePlanBasis>,
}

#[derive(Clone, Debug)]
struct TemplatePlanBasis {
    source_declaration_per_unit: u64,
    identity_field_per_unit: u64,
    profiled_key_per_unit: u64,
    source_reference_per_unit: u64,
    source_relation_per_unit: u64,
    source_geometry_per_unit: u64,
    semantic_output_per_unit: u64,
    semantic_payload_bytes_per_unit: u64,
    primary_record_per_unit: u64,
}

#[derive(Clone, Copy, Debug)]
struct ModuleGraphShape {
    module_count: u64,
    import_edge_count: u64,
    cross_module_reference_count: u64,
    maximum_import_depth: u64,
    canonical_module_name_bytes: u64,
    source_document_key_bytes: u64,
    import_target_module_name_bytes: u64,
    maximum_canonical_module_name_bytes: u64,
    maximum_source_document_key_bytes: u64,
    maximum_import_target_module_name_bytes: u64,
    shared_constant_string_item_count: u64,
    shared_constant_string_bytes: u64,
    maximum_shared_constant_string_bytes: u64,
}

impl ScalableStagePlanFactory {
    pub fn from_trusted_contract(trusted: &TrustedContract) -> Result<Self, ScalePlanError> {
        validate_stage_formula_contract(&trusted.workload_manifest)?;
        let corridor = TemplatePlanBasis::from_manifest_and_observed_payload(
            &trusted.workload_manifest,
            ScalableWorkloadId::Corridor,
            template_payload_basis(trusted, ScalableWorkloadId::Corridor)?,
        )?;
        let junction_grid = TemplatePlanBasis::from_manifest_and_observed_payload(
            &trusted.workload_manifest,
            ScalableWorkloadId::JunctionGrid,
            template_payload_basis(trusted, ScalableWorkloadId::JunctionGrid)?,
        )?;
        Ok(Self {
            identity: trusted.identity_contract()?,
            stage: trusted.stage_contract()?,
            corridor: Some(corridor),
            junction_grid: Some(junction_grid),
        })
    }

    pub(crate) fn from_trusted_contract_for_workload(
        trusted: &TrustedContract,
        workload_id: ScalableWorkloadId,
    ) -> Result<Self, ScalePlanError> {
        validate_stage_formula_contract(&trusted.workload_manifest)?;
        let template_basis = |workload_id| {
            TemplatePlanBasis::from_manifest_and_observed_payload(
                &trusted.workload_manifest,
                workload_id,
                template_payload_basis(trusted, workload_id)?,
            )
        };
        let (corridor, junction_grid) = match workload_id {
            ScalableWorkloadId::Identity => (None, None),
            ScalableWorkloadId::Corridor => {
                (Some(template_basis(ScalableWorkloadId::Corridor)?), None)
            }
            ScalableWorkloadId::JunctionGrid => (
                None,
                Some(template_basis(ScalableWorkloadId::JunctionGrid)?),
            ),
        };
        Ok(Self {
            identity: trusted.identity_contract()?,
            stage: trusted.stage_contract()?,
            corridor,
            junction_grid,
        })
    }

    pub(crate) fn from_trusted_contract_for_template_workload(
        trusted: &TrustedContract,
        workload_id: ScalableWorkloadId,
        template: &CorridorTemplate,
    ) -> Result<Self, ScalePlanError> {
        validate_stage_formula_contract(&trusted.workload_manifest)?;
        if workload_id == ScalableWorkloadId::Identity {
            return Err(ScalePlanError::Manifest(
                "identity cannot use a template plan factory".to_owned(),
            ));
        }
        let identity = trusted.identity_contract()?;
        let generator = trusted.generator_contract()?;
        let basis = TemplatePlanBasis::from_manifest_and_observed_payload(
            &trusted.workload_manifest,
            workload_id,
            template_semantic_payload_bytes_per_unit(
                &generator,
                &identity,
                workload_id.as_str(),
                template,
            )?,
        )?;
        let (corridor, junction_grid) = match workload_id {
            ScalableWorkloadId::Corridor => (Some(basis), None),
            ScalableWorkloadId::JunctionGrid => (None, Some(basis)),
            ScalableWorkloadId::Identity => unreachable!("rejected above"),
        };
        Ok(Self {
            identity,
            stage: trusted.stage_contract()?,
            corridor,
            junction_grid,
        })
    }

    pub fn plan(
        &self,
        workload_id: ScalableWorkloadId,
        graph_profile: GraphProfileId,
        n: u32,
    ) -> Result<ScalableStagePlanSummary, ScalePlanError> {
        match workload_id {
            ScalableWorkloadId::Identity => {
                let plan =
                    IdentityStagePlan::prepare(&self.identity, &self.stage, graph_profile, n)?;
                let summary = plan.plan_summary();
                Ok(ScalableStagePlanSummary {
                    workload_id,
                    graph_profile,
                    n,
                    primary_record_count: summary.counts.identity_field_occurrence_count,
                    counts: summary.counts,
                    stages: summary.stages,
                })
            }
            ScalableWorkloadId::Corridor => build_template_plan(
                &self.stage,
                workload_id,
                self.corridor
                    .as_ref()
                    .ok_or(ScalePlanError::UnavailableWorkload { workload_id })?,
                graph_profile,
                n,
            ),
            ScalableWorkloadId::JunctionGrid => build_template_plan(
                &self.stage,
                workload_id,
                self.junction_grid
                    .as_ref()
                    .ok_or(ScalePlanError::UnavailableWorkload { workload_id })?,
                graph_profile,
                n,
            ),
        }
    }
}

impl TemplatePlanBasis {
    fn from_manifest_and_observed_payload(
        manifest: &serde_json::Value,
        workload_id: ScalableWorkloadId,
        semantic_payload_bytes_per_unit: u64,
    ) -> Result<Self, ScalePlanError> {
        let workloads = manifest
            .get("workloads")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| ScalePlanError::Manifest("workloads".to_owned()))?;
        let workload = workloads
            .iter()
            .find(|candidate| {
                candidate.get("id").and_then(serde_json::Value::as_str)
                    == Some(workload_id.as_str())
            })
            .ok_or_else(|| ScalePlanError::Manifest(workload_id.as_str().to_owned()))?;
        let stage_inputs = object(workload, "perUnitStageInputs")?;
        let domain_counts = object(workload, "perUnitCounts")?;
        let primary_record_per_unit = match workload_id {
            ScalableWorkloadId::Corridor => checked_add(
                integer(domain_counts, "sourceRelationCount")
                    .or_else(|_| integer(stage_inputs, "sourceRelationCount"))?,
                integer(domain_counts, "canonicalGeometryPoint")
                    .or_else(|_| integer(stage_inputs, "sourceGeometryCount"))?,
                "corridor primary record count",
            )?,
            ScalableWorkloadId::JunctionGrid => sum(&[
                integer(domain_counts, "gateOccurrence")?,
                integer(domain_counts, "waitingZoneOccurrence")?,
                integer(domain_counts, "routeOccurrence")?,
            ])?,
            ScalableWorkloadId::Identity => {
                return Err(ScalePlanError::Manifest(
                    "identity cannot use template plan basis".to_owned(),
                ));
            }
        };
        Ok(Self {
            source_declaration_per_unit: integer(stage_inputs, "sourceDeclarationCount")?,
            identity_field_per_unit: integer(stage_inputs, "identityFieldOccurrenceCount")?,
            profiled_key_per_unit: integer(stage_inputs, "profiledKeyOccurrenceCount")?,
            source_reference_per_unit: integer(stage_inputs, "sourceReferenceCount")?,
            source_relation_per_unit: integer(stage_inputs, "sourceRelationCount")?,
            source_geometry_per_unit: integer(stage_inputs, "sourceGeometryCount")?,
            semantic_output_per_unit: integer(domain_counts, "semanticOutputRecord")?,
            semantic_payload_bytes_per_unit,
            primary_record_per_unit,
        })
    }
}

fn template_payload_basis(
    trusted: &TrustedContract,
    workload_id: ScalableWorkloadId,
) -> Result<u64, ScalePlanError> {
    let identity = trusted.identity_contract()?;
    let generator = trusted.generator_contract()?;
    let template = match workload_id {
        ScalableWorkloadId::Corridor => {
            let contract = CorridorContract::from_manifest(&trusted.workload_manifest)?;
            contract.load_template(&crate::repository_root())?
        }
        ScalableWorkloadId::JunctionGrid => {
            let contract = JunctionGridContract::from_manifest(&trusted.workload_manifest)?;
            let template = build_junction_grid_template();
            contract.validate_template(&template)?;
            template
        }
        ScalableWorkloadId::Identity => {
            return Err(ScalePlanError::Manifest(
                "identity has its own exact plan".to_owned(),
            ));
        }
    };
    Ok(template_semantic_payload_bytes_per_unit(
        &generator,
        &identity,
        workload_id.as_str(),
        &template,
    )?)
}

fn build_template_plan(
    stage: &StageContract,
    workload_id: ScalableWorkloadId,
    basis: &TemplatePlanBasis,
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<ScalableStagePlanSummary, ScalePlanError> {
    if n == 0 {
        return Err(ScalePlanError::ScaleMustBePositive);
    }
    let graph = module_graph_shape(graph_profile, n)?;
    let n = u64::from(n);
    let identity_declaration_count = checked_mul(
        basis.source_declaration_per_unit,
        n,
        "identity declarations",
    )?;
    let source_declaration_count = checked_add(
        identity_declaration_count,
        u64::from(graph_profile == GraphProfileId::SharedFaninDag),
        "source declarations",
    )?;
    let identity_field_occurrence_count =
        checked_mul(basis.identity_field_per_unit, n, "identity fields")?;
    let profiled_key_occurrence_count =
        checked_mul(basis.profiled_key_per_unit, n, "profiled keys")?;
    let source_reference_count = checked_add(
        checked_mul(basis.source_reference_per_unit, n, "source references")?,
        graph.cross_module_reference_count,
        "source references",
    )?;
    let source_relation_count = checked_mul(basis.source_relation_per_unit, n, "source relations")?;
    let source_geometry_count = checked_mul(basis.source_geometry_per_unit, n, "source geometry")?;
    let source_span_count = sum(&[
        source_declaration_count,
        source_reference_count,
        source_relation_count,
        source_geometry_count,
    ])?;
    let source_byte_count = sum(&[
        checked_mul(
            usize_u64(stage.declaration_token_bytes_with_lf)?,
            source_declaration_count,
            "source declaration bytes",
        )?,
        checked_mul(
            usize_u64(stage.reference_token_bytes_with_lf)?,
            source_reference_count,
            "source reference bytes",
        )?,
        checked_mul(
            usize_u64(stage.relation_token_bytes_with_lf)?,
            source_relation_count,
            "source relation bytes",
        )?,
        checked_mul(
            usize_u64(stage.geometry_token_bytes_with_lf)?,
            source_geometry_count,
            "source geometry bytes",
        )?,
    ])?;
    let string_item_count = sum(&[
        graph.module_count,
        graph.module_count,
        graph.import_edge_count,
        identity_declaration_count,
        profiled_key_occurrence_count,
        source_reference_count,
        graph.shared_constant_string_item_count,
    ])?;
    let total_string_bytes = sum(&[
        graph.canonical_module_name_bytes,
        graph.source_document_key_bytes,
        graph.import_target_module_name_bytes,
        checked_mul(32, identity_declaration_count, "namespace strings")?,
        checked_mul(20, profiled_key_occurrence_count, "profiled keys")?,
        checked_mul(30, source_reference_count, "reference spellings")?,
        graph.shared_constant_string_bytes,
    ])?;
    let maximum_string_bytes = [
        graph.maximum_canonical_module_name_bytes,
        graph.maximum_source_document_key_bytes,
        graph.maximum_import_target_module_name_bytes,
        u64::from(identity_declaration_count > 0) * 32,
        u64::from(profiled_key_occurrence_count > 0) * 20,
        u64::from(source_reference_count > 0) * 30,
        graph.maximum_shared_constant_string_bytes,
    ]
    .into_iter()
    .max()
    .unwrap_or(0);
    let semantic_output_record =
        checked_mul(basis.semantic_output_per_unit, n, "semantic output records")?;
    let semantic_payload_byte_count = checked_mul(
        basis.semantic_payload_bytes_per_unit,
        n,
        "semantic payload bytes",
    )?;
    let logical_byte_count = checked_add(
        checked_mul(44, semantic_output_record, "logical bytes")?,
        semantic_payload_byte_count,
        "logical bytes",
    )?;
    let output_byte_count = sum(&[
        54,
        checked_mul(36, semantic_output_record, "output envelopes")?,
        semantic_payload_byte_count,
    ])?;
    let counts = IdentityAggregateCounts {
        module_count: graph.module_count,
        import_edge_count: graph.import_edge_count,
        cross_module_reference_count: graph.cross_module_reference_count,
        maximum_import_depth: graph.maximum_import_depth,
        source_document_count: graph.module_count,
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
        string_item_count,
        maximum_string_bytes,
        total_string_bytes,
        diagnostic_count: 0,
        semantic_output_record,
        semantic_payload_byte_count,
        logical_byte_count,
        output_byte_count,
    };
    let stages = template_stage_breakdown(&counts)?;
    Ok(ScalableStagePlanSummary {
        workload_id,
        graph_profile,
        n: u32::try_from(n).map_err(|_| ScalePlanError::Overflow("N"))?,
        primary_record_count: checked_mul(basis.primary_record_per_unit, n, "primary records")?,
        counts,
        stages,
    })
}

fn module_graph_shape(
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<ModuleGraphShape, ScalePlanError> {
    let n = u64::from(n);
    let group_count = if graph_profile == GraphProfileId::SharedFaninDag {
        n.div_ceil(64)
    } else {
        0
    };
    let (
        module_count,
        import_edge_count,
        cross_module_reference_count,
        maximum_import_depth,
        canonical_module_name_bytes,
        import_target_module_name_bytes,
        maximum_canonical_module_name_bytes,
        shared_constant_string_item_count,
        shared_constant_string_bytes,
        maximum_shared_constant_string_bytes,
    ) = match graph_profile {
        GraphProfileId::WideStar => (
            checked_add(n, 1, "module count")?,
            n,
            n,
            1,
            checked_add(4, checked_mul(13, n, "module names")?, "module names")?,
            checked_mul(13, n, "import target names")?,
            13,
            0,
            0,
            0,
        ),
        GraphProfileId::DeepChain => (
            checked_add(n, 1, "module count")?,
            n,
            n.saturating_sub(1),
            n,
            checked_add(4, checked_mul(13, n, "module names")?, "module names")?,
            checked_mul(13, n, "import target names")?,
            13,
            0,
            0,
            0,
        ),
        GraphProfileId::SharedFaninDag => (
            sum(&[n, group_count, 2])?,
            sum(&[checked_mul(2, n, "import edges")?, group_count])?,
            n,
            3,
            sum(&[
                4,
                13,
                checked_mul(14, group_count, "group module names")?,
                checked_mul(13, n, "unit module names")?,
            ])?,
            sum(&[
                checked_mul(14, group_count, "group import targets")?,
                checked_mul(26, n, "unit and shared import targets")?,
            ])?,
            14,
            2,
            57,
            32,
        ),
    };
    let source_document_prefix = usize_u64("source/".len())?;
    let source_document_separator = usize_u64("/".len())?;
    let source_document_suffix = usize_u64(".lfsynthetic".len())?;
    let profile_bytes = usize_u64(graph_profile.as_str().len())?;
    let per_document_fixed = sum(&[
        source_document_prefix,
        profile_bytes,
        source_document_separator,
        source_document_suffix,
    ])?;
    Ok(ModuleGraphShape {
        module_count,
        import_edge_count,
        cross_module_reference_count,
        maximum_import_depth,
        canonical_module_name_bytes,
        source_document_key_bytes: checked_add(
            checked_mul(per_document_fixed, module_count, "source document keys")?,
            canonical_module_name_bytes,
            "source document keys",
        )?,
        import_target_module_name_bytes,
        maximum_canonical_module_name_bytes,
        maximum_source_document_key_bytes: checked_add(
            per_document_fixed,
            maximum_canonical_module_name_bytes,
            "maximum source document key",
        )?,
        maximum_import_target_module_name_bytes: maximum_canonical_module_name_bytes,
        shared_constant_string_item_count,
        shared_constant_string_bytes,
        maximum_shared_constant_string_bytes,
    })
}

fn template_stage_breakdown(
    counts: &IdentityAggregateCounts,
) -> Result<StageBreakdown, ScalePlanError> {
    let source_input_records = sum(&[
        counts.module_count,
        counts.import_edge_count,
        counts.source_span_count,
    ])?;
    let source_input_payload = checked_add(
        counts.source_byte_count,
        counts.total_string_bytes,
        "source input payload",
    )?;
    let typed_records = sum(&[
        counts.module_count,
        counts.import_edge_count,
        counts.source_declaration_count,
        counts.identity_field_occurrence_count,
        counts.source_reference_count,
        counts.source_relation_count,
        counts.source_geometry_count,
    ])?;
    let typed_payload = sum(&[
        counts.source_byte_count,
        counts.total_string_bytes,
        checked_mul(20, counts.source_span_count, "typed source spans")?,
    ])?;
    let hir_records = sum(&[
        counts.module_count,
        counts.import_edge_count,
        counts.symbol_count,
        counts.identity_field_occurrence_count,
        counts.source_reference_count,
        counts.source_relation_count,
        counts.source_geometry_count,
    ])?;
    let hir_operands = sum(&[
        counts.identity_field_occurrence_count,
        counts.import_edge_count,
        counts.source_reference_count,
        checked_mul(2, counts.source_relation_count, "HIR relations")?,
        checked_mul(3, counts.source_geometry_count, "HIR geometry")?,
    ])?;
    let hir_payload = checked_add(
        counts.total_string_bytes,
        checked_mul(4, hir_operands, "HIR operands")?,
        "HIR payload",
    )?;
    let source_input = stage_shape(source_input_records, source_input_payload, 32, 32)?;
    let typed_ast = stage_shape(typed_records, typed_payload, 32, 32)?;
    let hir = stage_shape(hir_records, hir_payload, 32, 32)?;
    let mir = stage_shape(
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
    let scratch_bytes = checked_mul(
        8,
        counts
            .module_count
            .max(counts.symbol_count)
            .max(counts.semantic_output_record),
        "scratch bytes",
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

fn stage_shape(
    record_count: u64,
    payload_logical_bytes: u64,
    logical_record_bytes: u64,
    allocated_record_bytes: u64,
) -> Result<StageShape, ScalePlanError> {
    Ok(StageShape {
        record_count,
        payload_logical_bytes,
        logical_bytes: checked_add(
            checked_mul(logical_record_bytes, record_count, "stage logical records")?,
            payload_logical_bytes,
            "stage logical bytes",
        )?,
        record_allocation_bytes: checked_add(
            checked_mul(
                allocated_record_bytes,
                record_count,
                "stage allocated records",
            )?,
            payload_logical_bytes,
            "stage allocation bytes",
        )?,
    })
}

fn validate_stage_formula_contract(manifest: &serde_json::Value) -> Result<(), ScalePlanError> {
    let model = object(manifest, "researchStageModel")?;
    if integer(model, "version")? != 1 {
        return Err(ScalePlanError::Manifest(
            "researchStageModel.version".to_owned(),
        ));
    }
    let aggregate = object(model, "aggregateInputFormulas")?;
    require_string(
        aggregate,
        "semanticOutputRecord",
        "N * perUnitCounts.semanticOutputRecord",
    )?;
    require_string(
        aggregate,
        "semanticPayloadByteCount",
        "sum exact payload byte lengths obtained from recordKinds, identityBindings, stringProfile, and the canonical records for this level",
    )?;
    let guard = object(manifest, "guardPredictionContract")?;
    let primary = object(guard, "primaryRecordCountByWorkload")?;
    validate_primary_operands(
        object(primary, ScalableWorkloadId::Corridor.as_str())?,
        &["sourceRelationCount", "sourceGeometryCount"],
    )?;
    validate_primary_operands(
        object(primary, ScalableWorkloadId::JunctionGrid.as_str())?,
        &["gateOccurrence", "waitingZoneOccurrence", "routeOccurrence"],
    )
}

fn validate_primary_operands(
    value: &serde_json::Value,
    expected: &[&str],
) -> Result<(), ScalePlanError> {
    require_string(value, "aggregate", "sum")?;
    let operands = value
        .get("operands")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| ScalePlanError::Manifest("operands".to_owned()))?;
    if operands.len() != expected.len()
        || operands
            .iter()
            .zip(expected)
            .any(|(actual, expected)| actual.as_str() != Some(expected))
    {
        return Err(ScalePlanError::Manifest("operands".to_owned()));
    }
    Ok(())
}

fn object<'a>(
    value: &'a serde_json::Value,
    field: &str,
) -> Result<&'a serde_json::Value, ScalePlanError> {
    value
        .get(field)
        .filter(|candidate| candidate.is_object())
        .ok_or_else(|| ScalePlanError::Manifest(field.to_owned()))
}

fn integer(value: &serde_json::Value, field: &str) -> Result<u64, ScalePlanError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| ScalePlanError::Manifest(field.to_owned()))
}

fn require_string(
    value: &serde_json::Value,
    field: &str,
    expected: &str,
) -> Result<(), ScalePlanError> {
    if value.get(field).and_then(serde_json::Value::as_str) != Some(expected) {
        return Err(ScalePlanError::Manifest(field.to_owned()));
    }
    Ok(())
}

fn checked_add(left: u64, right: u64, field: &'static str) -> Result<u64, ScalePlanError> {
    left.checked_add(right)
        .ok_or(ScalePlanError::Overflow(field))
}

fn checked_mul(left: u64, right: u64, field: &'static str) -> Result<u64, ScalePlanError> {
    left.checked_mul(right)
        .ok_or(ScalePlanError::Overflow(field))
}

fn sum(values: &[u64]) -> Result<u64, ScalePlanError> {
    values
        .iter()
        .try_fold(0_u64, |total, value| checked_add(total, *value, "sum"))
}

fn usize_u64(value: usize) -> Result<u64, ScalePlanError> {
    u64::try_from(value).map_err(|_| ScalePlanError::Overflow("usize conversion"))
}

#[derive(Debug, thiserror::Error)]
pub enum ScalePlanError {
    #[error(transparent)]
    Stage(#[from] crate::StageGenerationError),
    #[error(transparent)]
    IdentityContract(#[from] crate::IdentityContractError),
    #[error(transparent)]
    StageContract(#[from] crate::StageContractError),
    #[error(transparent)]
    ManifestContract(#[from] crate::ManifestContractError),
    #[error(transparent)]
    Corridor(#[from] crate::CorridorError),
    #[error(transparent)]
    JunctionGrid(#[from] crate::JunctionGridError),
    #[error("规模 N 必须大于零")]
    ScaleMustBePositive,
    #[error("规模计划算术溢出：{0}")]
    Overflow(&'static str),
    #[error("工作负载清单规模计划契约不匹配：{0}")]
    Manifest(String),
    #[error("规模计划工厂没有准备工作负载 {workload_id:?}")]
    UnavailableWorkload { workload_id: ScalableWorkloadId },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        build_corridor_stage_summary, build_identity_stage_summary,
        build_junction_grid_stage_summary, load_repository_contract,
    };

    #[test]
    fn constant_time_plans_match_all_three_producers_at_n_one_and_two() {
        let trusted = load_repository_contract().expect("trusted contract");
        let factory =
            ScalableStagePlanFactory::from_trusted_contract(&trusted).expect("plan factory");
        for workload_id in ScalableWorkloadId::ALL {
            for graph_profile in GraphProfileId::ALL {
                for n in [1, 2] {
                    let planned = factory
                        .plan(workload_id, graph_profile, n)
                        .expect("scale plan");
                    let (counts, stages) = match workload_id {
                        ScalableWorkloadId::Identity => {
                            let produced = build_identity_stage_summary(&trusted, graph_profile, n)
                                .expect("identity producer");
                            (produced.counts, produced.stages)
                        }
                        ScalableWorkloadId::Corridor => {
                            let produced = build_corridor_stage_summary(&trusted, graph_profile, n)
                                .expect("corridor producer");
                            (produced.counts, produced.stages)
                        }
                        ScalableWorkloadId::JunctionGrid => {
                            let produced =
                                build_junction_grid_stage_summary(&trusted, graph_profile, n)
                                    .expect("junction-grid producer");
                            (produced.counts, produced.stages)
                        }
                    };
                    assert_eq!(planned.counts, counts);
                    assert_eq!(planned.stages, stages);
                }
            }
        }
    }

    #[test]
    fn primary_record_counts_follow_each_workloads_frozen_formula() {
        let trusted = load_repository_contract().expect("trusted contract");
        let factory =
            ScalableStagePlanFactory::from_trusted_contract(&trusted).expect("plan factory");
        for graph_profile in GraphProfileId::ALL {
            assert_eq!(
                factory
                    .plan(ScalableWorkloadId::Identity, graph_profile, 1)
                    .expect("identity plan")
                    .primary_record_count,
                57
            );
            assert_eq!(
                factory
                    .plan(ScalableWorkloadId::Corridor, graph_profile, 1)
                    .expect("corridor plan")
                    .primary_record_count,
                2_025
            );
            assert_eq!(
                factory
                    .plan(ScalableWorkloadId::JunctionGrid, graph_profile, 1)
                    .expect("junction-grid plan")
                    .primary_record_count,
                108
            );
        }
    }

    #[test]
    fn plans_match_producers_across_the_shared_fanin_group_boundary() {
        let trusted = load_repository_contract().expect("trusted contract");
        let factory =
            ScalableStagePlanFactory::from_trusted_contract(&trusted).expect("plan factory");
        let graph_profile = GraphProfileId::SharedFaninDag;
        let n = 65;
        for workload_id in ScalableWorkloadId::ALL {
            let planned = factory
                .plan(workload_id, graph_profile, n)
                .expect("scale plan");
            let (counts, stages) = match workload_id {
                ScalableWorkloadId::Identity => {
                    let produced = build_identity_stage_summary(&trusted, graph_profile, n)
                        .expect("identity producer");
                    (produced.counts, produced.stages)
                }
                ScalableWorkloadId::Corridor => {
                    let produced = build_corridor_stage_summary(&trusted, graph_profile, n)
                        .expect("corridor producer");
                    (produced.counts, produced.stages)
                }
                ScalableWorkloadId::JunctionGrid => {
                    let produced = build_junction_grid_stage_summary(&trusted, graph_profile, n)
                        .expect("junction-grid producer");
                    (produced.counts, produced.stages)
                }
            };
            assert_eq!(planned.counts, counts);
            assert_eq!(planned.stages, stages);
        }
    }
}
