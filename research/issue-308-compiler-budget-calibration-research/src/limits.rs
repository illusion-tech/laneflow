//! #308 私有编译限制维度与配对资格计划。
//!
//! 本模块只实现研究私有的限制模型，不定义未来生产 `CompileLimits`。普通维度由受信任
//! 清单和 O(1) 规模计划重算；编译器控制存续字节必须来自两个独立 attribution 副本，
//! 不能用公式或被测候选自报值替代。

use crate::{
    GraphProfileId, ScalableStagePlanFactory, ScalableStagePlanSummary, ScalableWorkloadId,
    ScalableWorkloadParseError, ScalePlanError, TrustedContract,
};
use serde::Serialize;
use serde_json::Value;
use std::str::FromStr;
use thiserror::Error;

pub const LIMIT_EXCEEDED_ERROR_CODE: &str = "LF-COMP-RESEARCH-E-LIMIT-EXCEEDED";
pub const UNKNOWN_REFERENCE_ERROR_CODE: &str = "LF-COMP-RESEARCH-E-UNKNOWN-REFERENCE";
pub const DIAGNOSTIC_LIMIT_ERROR_CODE: &str = "LF-COMP-RESEARCH-E-DIAGNOSTIC-LIMIT";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LimitDimensionId {
    ModuleCount,
    ImportEdgeCount,
    SourceByteCount,
    DeclarationCount,
    IdentityFieldOccurrenceCount,
    SymbolCount,
    ReferenceCount,
    RelationOccurrenceCount,
    StringItemCount,
    SingleStringByteCount,
    TotalStringByteCount,
    GeometryPointCount,
    RouteOccurrenceCount,
    ManeuverGateCount,
    WaitingZoneCount,
    DiagnosticCount,
    TypedAstRecordCount,
    HirRecordCount,
    MirRecordCount,
    LirRecordCount,
    StageScratchByteCount,
    OutputByteCount,
    CompilerControlledLiveByteCount,
}

impl LimitDimensionId {
    pub const ALL: [Self; 23] = [
        Self::ModuleCount,
        Self::ImportEdgeCount,
        Self::SourceByteCount,
        Self::DeclarationCount,
        Self::IdentityFieldOccurrenceCount,
        Self::SymbolCount,
        Self::ReferenceCount,
        Self::RelationOccurrenceCount,
        Self::StringItemCount,
        Self::SingleStringByteCount,
        Self::TotalStringByteCount,
        Self::GeometryPointCount,
        Self::RouteOccurrenceCount,
        Self::ManeuverGateCount,
        Self::WaitingZoneCount,
        Self::DiagnosticCount,
        Self::TypedAstRecordCount,
        Self::HirRecordCount,
        Self::MirRecordCount,
        Self::LirRecordCount,
        Self::StageScratchByteCount,
        Self::OutputByteCount,
        Self::CompilerControlledLiveByteCount,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ModuleCount => "module-count",
            Self::ImportEdgeCount => "import-edge-count",
            Self::SourceByteCount => "source-byte-count",
            Self::DeclarationCount => "declaration-count",
            Self::IdentityFieldOccurrenceCount => "identity-field-occurrence-count",
            Self::SymbolCount => "symbol-count",
            Self::ReferenceCount => "reference-count",
            Self::RelationOccurrenceCount => "relation-occurrence-count",
            Self::StringItemCount => "string-item-count",
            Self::SingleStringByteCount => "single-string-byte-count",
            Self::TotalStringByteCount => "total-string-byte-count",
            Self::GeometryPointCount => "geometry-point-count",
            Self::RouteOccurrenceCount => "route-occurrence-count",
            Self::ManeuverGateCount => "maneuver-gate-count",
            Self::WaitingZoneCount => "waiting-zone-count",
            Self::DiagnosticCount => "diagnostic-count",
            Self::TypedAstRecordCount => "typed-ast-record-count",
            Self::HirRecordCount => "hir-record-count",
            Self::MirRecordCount => "mir-record-count",
            Self::LirRecordCount => "lir-record-count",
            Self::StageScratchByteCount => "stage-scratch-byte-count",
            Self::OutputByteCount => "output-byte-count",
            Self::CompilerControlledLiveByteCount => "compiler-controlled-live-byte-count",
        }
    }

    pub fn one_based_code_u8(self) -> u8 {
        let index = Self::ALL
            .iter()
            .position(|candidate| *candidate == self)
            .expect("closed limit dimension must be registered");
        u8::try_from(index + 1).expect("closed limit dimension count fits u8")
    }

    fn parse(value: &str) -> Result<Self, LimitQualificationError> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == value)
            .ok_or_else(|| LimitQualificationError::UnknownDimension(value.to_owned()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LimitPairMode {
    SuccessAtBound,
    DiagnosticCapOnSemanticFailure,
    BaselineLiveBytePrescanV1,
}

impl LimitPairMode {
    fn parse(value: Option<&str>) -> Result<Self, LimitQualificationError> {
        match value.unwrap_or("success-at-bound") {
            "success-at-bound" => Ok(Self::SuccessAtBound),
            "diagnostic-cap-on-semantic-failure" => Ok(Self::DiagnosticCapOnSemanticFailure),
            "baseline-live-byte-prescan-v1" => Ok(Self::BaselineLiveBytePrescanV1),
            actual => Err(LimitQualificationError::UnknownPairMode(actual.to_owned())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitDimensionBinding {
    pub dimension_id: LimitDimensionId,
    pub dimension_code_u8: u8,
    pub workload_id: ScalableWorkloadId,
    pub input_variant_id: String,
    pub pair_mode: LimitPairMode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitPairPlan {
    pub binding: LimitDimensionBinding,
    pub graph_profile: GraphProfileId,
    pub n: u32,
    pub basis_run_ids: Vec<String>,
    pub exact_dimension_value: u64,
    pub at_bound_limit_value: u64,
    pub plus_one_limit_value: u64,
}

#[derive(Clone, Debug)]
pub struct LimitQualificationPlanner {
    plans: ScalableStagePlanFactory,
    bindings: Vec<LimitDimensionBinding>,
    manifest: Value,
}

impl LimitQualificationPlanner {
    pub fn from_trusted_contract(
        trusted: &TrustedContract,
    ) -> Result<Self, LimitQualificationError> {
        let bindings = parse_limit_bindings(&trusted.workload_manifest)?;
        Ok(Self {
            plans: ScalableStagePlanFactory::from_trusted_contract(trusted)?,
            bindings,
            manifest: trusted.workload_manifest.clone(),
        })
    }

    pub fn bindings(&self) -> &[LimitDimensionBinding] {
        &self.bindings
    }

    pub fn plan_pair(
        &self,
        dimension_id: LimitDimensionId,
        graph_profile: GraphProfileId,
        n: u32,
        live_byte_baseline: Option<LiveByteBaseline>,
    ) -> Result<LimitPairPlan, LimitQualificationError> {
        let binding = self
            .bindings
            .iter()
            .find(|binding| binding.dimension_id == dimension_id)
            .ok_or(LimitQualificationError::MissingDimensionBinding(
                dimension_id,
            ))?
            .clone();
        let plan = self.plans.plan(binding.workload_id, graph_profile, n)?;
        let (exact_dimension_value, basis_run_ids) = match dimension_id {
            LimitDimensionId::CompilerControlledLiveByteCount => {
                let baseline =
                    live_byte_baseline.ok_or(LimitQualificationError::MissingLiveByteBaseline)?;
                baseline.validate(&binding, graph_profile, n)?
            }
            _ => {
                if live_byte_baseline.is_some() {
                    return Err(LimitQualificationError::UnexpectedLiveByteBaseline(
                        dimension_id,
                    ));
                }
                (
                    exact_plan_value(&self.manifest, dimension_id, &plan)?,
                    Vec::new(),
                )
            }
        };
        if exact_dimension_value == 0 {
            return Err(LimitQualificationError::ZeroExactDimensionValue(
                dimension_id,
            ));
        }
        Ok(LimitPairPlan {
            binding,
            graph_profile,
            n,
            basis_run_ids,
            exact_dimension_value,
            at_bound_limit_value: exact_dimension_value,
            plus_one_limit_value: exact_dimension_value - 1,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveByteBaselineReplica {
    pub run_id: String,
    pub workload_id: ScalableWorkloadId,
    pub graph_profile: GraphProfileId,
    pub n: u32,
    pub peak_live_requested_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveByteBaseline {
    pub replicas: [LiveByteBaselineReplica; 2],
}

impl LiveByteBaseline {
    fn validate(
        self,
        binding: &LimitDimensionBinding,
        graph_profile: GraphProfileId,
        n: u32,
    ) -> Result<(u64, Vec<String>), LimitQualificationError> {
        let [left, right] = self.replicas;
        for replica in [&left, &right] {
            if replica.run_id.is_empty()
                || replica.workload_id != binding.workload_id
                || replica.graph_profile != graph_profile
                || replica.n != n
            {
                return Err(LimitQualificationError::InvalidLiveByteBaselineIdentity);
            }
            if replica.peak_live_requested_bytes == 0 {
                return Err(LimitQualificationError::ZeroLiveByteBaseline);
            }
        }
        if left.run_id == right.run_id {
            return Err(LimitQualificationError::DuplicateLiveByteBaselineRunId);
        }
        if left.peak_live_requested_bytes != right.peak_live_requested_bytes {
            return Err(LimitQualificationError::LiveByteBaselineDisagreement {
                left: left.peak_live_requested_bytes,
                right: right.peak_live_requested_bytes,
            });
        }
        Ok((
            left.peak_live_requested_bytes,
            vec![left.run_id, right.run_id],
        ))
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error(
    "{error_code}：限制 {dimension_id:?}（代码 {dimension_code_u8}）实际值 {actual_value} 超过选中上限 {selected_limit_value}"
)]
pub struct LimitExceeded {
    pub error_code: &'static str,
    pub dimension_id: LimitDimensionId,
    pub dimension_code_u8: u8,
    pub actual_value: u64,
    pub selected_limit_value: u64,
}

pub fn enforce_selected_limit(
    pair: &LimitPairPlan,
    selected_limit_value: u64,
) -> Result<(), LimitExceeded> {
    if pair.exact_dimension_value <= selected_limit_value {
        return Ok(());
    }
    Err(LimitExceeded {
        error_code: LIMIT_EXCEEDED_ERROR_CODE,
        dimension_id: pair.binding.dimension_id,
        dimension_code_u8: pair.binding.dimension_code_u8,
        actual_value: pair.exact_dimension_value,
        selected_limit_value,
    })
}

fn parse_limit_bindings(
    manifest: &Value,
) -> Result<Vec<LimitDimensionBinding>, LimitQualificationError> {
    let dimensions = manifest
        .get("limitDimensions")
        .and_then(Value::as_array)
        .ok_or(LimitQualificationError::Manifest("limitDimensions"))?;
    let parsed_dimensions = dimensions
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or(LimitQualificationError::Manifest("limitDimensions[]"))
                .and_then(LimitDimensionId::parse)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if parsed_dimensions.as_slice() != LimitDimensionId::ALL {
        return Err(LimitQualificationError::DimensionRegistryMismatch);
    }

    let bindings = manifest
        .get("limitDimensionBindings")
        .and_then(Value::as_array)
        .ok_or(LimitQualificationError::Manifest("limitDimensionBindings"))?;
    if bindings.len() != LimitDimensionId::ALL.len() {
        return Err(LimitQualificationError::DimensionBindingCount {
            actual: bindings.len(),
        });
    }
    bindings
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let dimension_id = LimitDimensionId::parse(required_string(value, "dimensionId")?)?;
            if dimension_id != LimitDimensionId::ALL[index] {
                return Err(LimitQualificationError::DimensionBindingOrder {
                    index,
                    actual: dimension_id,
                });
            }
            let workload_id = ScalableWorkloadId::from_str(required_string(value, "workloadId")?)?;
            let pair_mode = LimitPairMode::parse(value.get("pairMode").and_then(Value::as_str))?;
            let input_variant_id = value
                .get("inputVariantId")
                .and_then(Value::as_str)
                .unwrap_or("canonical-valid-v1")
                .to_owned();
            validate_special_binding(dimension_id, workload_id, pair_mode, &input_variant_id)?;
            Ok(LimitDimensionBinding {
                dimension_id,
                dimension_code_u8: dimension_id.one_based_code_u8(),
                workload_id,
                input_variant_id,
                pair_mode,
            })
        })
        .collect()
}

fn validate_special_binding(
    dimension_id: LimitDimensionId,
    workload_id: ScalableWorkloadId,
    pair_mode: LimitPairMode,
    input_variant_id: &str,
) -> Result<(), LimitQualificationError> {
    let valid = match dimension_id {
        LimitDimensionId::DiagnosticCount => {
            workload_id == ScalableWorkloadId::Corridor
                && pair_mode == LimitPairMode::DiagnosticCapOnSemanticFailure
                && input_variant_id == "corridor-missing-reference-per-unit-v1"
        }
        LimitDimensionId::CompilerControlledLiveByteCount => {
            workload_id == ScalableWorkloadId::Identity
                && pair_mode == LimitPairMode::BaselineLiveBytePrescanV1
                && input_variant_id == "canonical-valid-v1"
        }
        _ => pair_mode == LimitPairMode::SuccessAtBound && input_variant_id == "canonical-valid-v1",
    };
    if valid {
        Ok(())
    } else {
        Err(LimitQualificationError::InvalidSpecialBinding(dimension_id))
    }
}

fn exact_plan_value(
    manifest: &Value,
    dimension_id: LimitDimensionId,
    plan: &ScalableStagePlanSummary,
) -> Result<u64, LimitQualificationError> {
    let value = match dimension_id {
        LimitDimensionId::ModuleCount => plan.counts.module_count,
        LimitDimensionId::ImportEdgeCount => plan.counts.import_edge_count,
        LimitDimensionId::SourceByteCount => plan.counts.source_byte_count,
        LimitDimensionId::DeclarationCount => plan.counts.source_declaration_count,
        LimitDimensionId::IdentityFieldOccurrenceCount => {
            plan.counts.identity_field_occurrence_count
        }
        LimitDimensionId::SymbolCount => plan.counts.symbol_count,
        LimitDimensionId::ReferenceCount => plan.counts.source_reference_count,
        LimitDimensionId::RelationOccurrenceCount => plan.counts.source_relation_count,
        LimitDimensionId::StringItemCount => plan.counts.string_item_count,
        LimitDimensionId::SingleStringByteCount => plan.counts.maximum_string_bytes,
        LimitDimensionId::TotalStringByteCount => plan.counts.total_string_bytes,
        LimitDimensionId::GeometryPointCount => plan.counts.source_geometry_count,
        LimitDimensionId::RouteOccurrenceCount => {
            per_unit_count(manifest, plan.workload_id, "routeOccurrence", plan.n)?
        }
        LimitDimensionId::ManeuverGateCount => {
            per_unit_count(manifest, plan.workload_id, "ManeuverGate", plan.n)?
        }
        LimitDimensionId::WaitingZoneCount => {
            per_unit_count(manifest, plan.workload_id, "WaitingZone", plan.n)?
        }
        LimitDimensionId::DiagnosticCount => u64::from(plan.n),
        LimitDimensionId::TypedAstRecordCount => plan.stages.typed_ast.record_count,
        LimitDimensionId::HirRecordCount => plan.stages.hir.record_count,
        LimitDimensionId::MirRecordCount => plan.stages.mir.record_count,
        LimitDimensionId::LirRecordCount => plan.stages.canonical_lir.record_count,
        LimitDimensionId::StageScratchByteCount => plan.stages.scratch.logical_bytes,
        LimitDimensionId::OutputByteCount => plan.counts.output_byte_count,
        LimitDimensionId::CompilerControlledLiveByteCount => {
            return Err(LimitQualificationError::MissingLiveByteBaseline);
        }
    };
    Ok(value)
}

fn per_unit_count(
    manifest: &Value,
    workload_id: ScalableWorkloadId,
    field: &'static str,
    n: u32,
) -> Result<u64, LimitQualificationError> {
    let workload = manifest
        .get("workloads")
        .and_then(Value::as_array)
        .and_then(|workloads| {
            workloads.iter().find(|workload| {
                workload.get("id").and_then(Value::as_str) == Some(workload_id.as_str())
            })
        })
        .ok_or(LimitQualificationError::WorkloadMissing(workload_id))?;
    let per_unit = workload
        .get("perUnitCounts")
        .and_then(Value::as_object)
        .and_then(|counts| counts.get(field))
        .and_then(Value::as_u64)
        .ok_or(LimitQualificationError::PerUnitCountMissing { workload_id, field })?;
    per_unit
        .checked_mul(u64::from(n))
        .ok_or(LimitQualificationError::CountOverflow { workload_id, field })
}

fn required_string<'a>(
    value: &'a Value,
    field: &'static str,
) -> Result<&'a str, LimitQualificationError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or(LimitQualificationError::Manifest(field))
}

#[derive(Debug, Error)]
pub enum LimitQualificationError {
    #[error("限制资格清单字段缺失或类型错误：{0}")]
    Manifest(&'static str),
    #[error("未知限制维度：{0}")]
    UnknownDimension(String),
    #[error("未知限制配对模式：{0}")]
    UnknownPairMode(String),
    #[error("limitDimensions 与冻结的二十三项维度或顺序不一致")]
    DimensionRegistryMismatch,
    #[error("limitDimensionBindings 数量错误：actual={actual}")]
    DimensionBindingCount { actual: usize },
    #[error("限制绑定顺序错误：index={index}, actual={actual:?}")]
    DimensionBindingOrder {
        index: usize,
        actual: LimitDimensionId,
    },
    #[error("限制维度缺少唯一绑定：{0:?}")]
    MissingDimensionBinding(LimitDimensionId),
    #[error("限制维度的特殊绑定不符合清单契约：{0:?}")]
    InvalidSpecialBinding(LimitDimensionId),
    #[error("工作负载缺失：{0:?}")]
    WorkloadMissing(ScalableWorkloadId),
    #[error("工作负载 {workload_id:?} 缺少每单元计数 {field}")]
    PerUnitCountMissing {
        workload_id: ScalableWorkloadId,
        field: &'static str,
    },
    #[error("工作负载 {workload_id:?} 的每单元计数 {field} 乘法溢出")]
    CountOverflow {
        workload_id: ScalableWorkloadId,
        field: &'static str,
    },
    #[error("限制维度的精确值必须为正：{0:?}")]
    ZeroExactDimensionValue(LimitDimensionId),
    #[error("编译器控制存续字节限制缺少两个独立 attribution 副本")]
    MissingLiveByteBaseline,
    #[error("普通限制维度不得携带存续字节基线：{0:?}")]
    UnexpectedLiveByteBaseline(LimitDimensionId),
    #[error("存续字节基线副本身份不匹配")]
    InvalidLiveByteBaselineIdentity,
    #[error("两个存续字节基线副本必须使用不同 runId")]
    DuplicateLiveByteBaselineRunId,
    #[error("存续字节基线峰值必须为正")]
    ZeroLiveByteBaseline,
    #[error("两个 attribution 副本峰值不一致：left={left}, right={right}")]
    LiveByteBaselineDisagreement { left: u64, right: u64 },
    #[error(transparent)]
    ScalePlan(#[from] ScalePlanError),
    #[error(transparent)]
    Workload(#[from] ScalableWorkloadParseError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ScalableAttributionCompilerInstance, ScalableTimingCompilerInstance,
        load_repository_contract,
    };

    #[test]
    fn manifest_registers_every_limit_dimension_once_in_code_order() {
        let trusted = load_repository_contract().expect("trusted contract");
        let planner =
            LimitQualificationPlanner::from_trusted_contract(&trusted).expect("limit planner");
        assert_eq!(planner.bindings().len(), LimitDimensionId::ALL.len());
        for (index, binding) in planner.bindings().iter().enumerate() {
            assert_eq!(binding.dimension_id, LimitDimensionId::ALL[index]);
            assert_eq!(binding.dimension_code_u8, u8::try_from(index + 1).unwrap());
        }
    }

    #[test]
    fn every_formula_dimension_has_a_positive_at_bound_and_exact_plus_one_pair() {
        let trusted = load_repository_contract().expect("trusted contract");
        let planner =
            LimitQualificationPlanner::from_trusted_contract(&trusted).expect("limit planner");
        for profile in GraphProfileId::ALL {
            for dimension in LimitDimensionId::ALL {
                if dimension == LimitDimensionId::CompilerControlledLiveByteCount {
                    continue;
                }
                let pair = planner
                    .plan_pair(dimension, profile, 1, None)
                    .expect("formula-backed pair");
                assert!(pair.exact_dimension_value > 0);
                enforce_selected_limit(&pair, pair.at_bound_limit_value)
                    .expect("at-bound must pass");
                let violation = enforce_selected_limit(&pair, pair.plus_one_limit_value)
                    .expect_err("plus-one must fail before the selected work");
                assert_eq!(violation.error_code, LIMIT_EXCEEDED_ERROR_CODE);
                assert_eq!(violation.dimension_id, dimension);
                assert_eq!(violation.actual_value, violation.selected_limit_value + 1);
            }
        }
    }

    #[test]
    fn ordinary_at_bound_plans_execute_each_bound_workload() {
        let trusted = load_repository_contract().expect("trusted contract");
        let planner =
            LimitQualificationPlanner::from_trusted_contract(&trusted).expect("limit planner");
        for workload_id in ScalableWorkloadId::ALL {
            let binding = planner
                .bindings()
                .iter()
                .find(|binding| {
                    binding.workload_id == workload_id
                        && binding.pair_mode == LimitPairMode::SuccessAtBound
                })
                .expect("ordinary binding");
            let pair = planner
                .plan_pair(binding.dimension_id, GraphProfileId::WideStar, 1, None)
                .expect("ordinary pair");
            enforce_selected_limit(&pair, pair.at_bound_limit_value).expect("at-bound preflight");
            let mut instance = ScalableTimingCompilerInstance::from_trusted_contract_with_id(
                &trusted,
                format!("limit/at-bound/{}", workload_id.as_str()),
                workload_id,
            )
            .expect("compiler instance");
            instance
                .run_unmeasured(GraphProfileId::WideStar, 1)
                .expect("canonical workload executes after at-bound preflight");
        }
    }

    #[test]
    fn live_byte_pair_requires_two_matching_independent_attribution_replicas() {
        let trusted = load_repository_contract().expect("trusted contract");
        let profile = GraphProfileId::WideStar;
        let mut peaks = Vec::new();
        for replica in 0..2 {
            let mut instance = ScalableAttributionCompilerInstance::from_trusted_contract_with_id(
                &trusted,
                format!("limit/live-byte/baseline/{replica}"),
                ScalableWorkloadId::Identity,
            )
            .expect("attribution instance");
            instance
                .run_unmeasured(profile, 1)
                .expect("attribution baseline");
            let snapshot = instance.allocation_snapshot().expect("allocation snapshot");
            peaks.push(snapshot.peak_live_requested_bytes);
        }
        assert_eq!(peaks[0], peaks[1]);

        let baseline = LiveByteBaseline {
            replicas: [
                LiveByteBaselineReplica {
                    run_id: "limit/live-byte/baseline/0".to_owned(),
                    workload_id: ScalableWorkloadId::Identity,
                    graph_profile: profile,
                    n: 1,
                    peak_live_requested_bytes: peaks[0],
                },
                LiveByteBaselineReplica {
                    run_id: "limit/live-byte/baseline/1".to_owned(),
                    workload_id: ScalableWorkloadId::Identity,
                    graph_profile: profile,
                    n: 1,
                    peak_live_requested_bytes: peaks[1],
                },
            ],
        };
        let planner =
            LimitQualificationPlanner::from_trusted_contract(&trusted).expect("limit planner");
        let pair = planner
            .plan_pair(
                LimitDimensionId::CompilerControlledLiveByteCount,
                profile,
                1,
                Some(baseline),
            )
            .expect("live-byte pair");
        assert_eq!(pair.basis_run_ids.len(), 2);

        let mut at_bound =
            ScalableAttributionCompilerInstance::from_trusted_contract_with_id_and_allocation_ceiling(
                &trusted,
                "limit/live-byte/at-bound".to_owned(),
                ScalableWorkloadId::Identity,
                pair.at_bound_limit_value,
            )
            .expect("at-bound instance");
        at_bound
            .run_unmeasured(profile, 1)
            .expect("at-bound allocation succeeds");

        let mut plus_one =
            ScalableAttributionCompilerInstance::from_trusted_contract_with_id_and_allocation_ceiling(
                &trusted,
                "limit/live-byte/plus-one".to_owned(),
                ScalableWorkloadId::Identity,
                pair.plus_one_limit_value,
            )
            .expect("plus-one instance");
        plus_one
            .run_unmeasured(profile, 1)
            .expect_err("plus-one allocation must fail");
        assert!(
            plus_one
                .allocation_snapshot()
                .expect("post-failure allocation snapshot")
                .live_requested_bytes
                <= pair.plus_one_limit_value
        );
    }

    #[test]
    fn live_byte_baseline_disagreement_fails_closed() {
        let trusted = load_repository_contract().expect("trusted contract");
        let planner =
            LimitQualificationPlanner::from_trusted_contract(&trusted).expect("limit planner");
        let error = planner
            .plan_pair(
                LimitDimensionId::CompilerControlledLiveByteCount,
                GraphProfileId::WideStar,
                1,
                Some(LiveByteBaseline {
                    replicas: [
                        LiveByteBaselineReplica {
                            run_id: "left".to_owned(),
                            workload_id: ScalableWorkloadId::Identity,
                            graph_profile: GraphProfileId::WideStar,
                            n: 1,
                            peak_live_requested_bytes: 100,
                        },
                        LiveByteBaselineReplica {
                            run_id: "right".to_owned(),
                            workload_id: ScalableWorkloadId::Identity,
                            graph_profile: GraphProfileId::WideStar,
                            n: 1,
                            peak_live_requested_bytes: 101,
                        },
                    ],
                }),
            )
            .expect_err("disagreement must fail closed");
        assert!(matches!(
            error,
            LimitQualificationError::LiveByteBaselineDisagreement {
                left: 100,
                right: 101
            }
        ));
    }
}
