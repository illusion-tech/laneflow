//! #308 基线失败关闭与三十二次恢复资格。
//!
//! 该实现只验证冻结的完整研究管线基线。资源预检和语义诊断在进入后续阶段前失败；失败
//! 诊断使用可失败保留的实例私有缓冲区，运行结束后清空语义值，仅保留稳定容量。

use crate::corridor::{CorridorContract, TemplateRelation};
use crate::{
    DIAGNOSTIC_LIMIT_ERROR_CODE, GraphProfileId, LIMIT_EXCEEDED_ERROR_CODE, LimitDimensionId,
    LimitQualificationError, LimitQualificationPlanner, ScalableAttributionCompilerInstance,
    ScalableStagePlanFactory, ScalableStagePlanSummary, ScalableWorkloadId, TimingError,
    TrustedContract, UNKNOWN_REFERENCE_ERROR_CODE, enforce_selected_limit,
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::mem::size_of;
use thiserror::Error;

pub const CLEANUP_FAILURE_ITERATION_COUNT: u32 = 32;
pub const CLEANUP_GROUP_RUN_COUNT: usize = 35;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupScaleRole {
    Calibration,
    Stress,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum CleanupFailureCase {
    #[serde(rename = "limit/source-byte-count/plus-one")]
    SourceByteLimitPlusOne,
    #[serde(rename = "semantic/missing-reference-per-unit")]
    MissingReferencePerUnit,
    #[serde(rename = "diagnostic/cap-plus-one")]
    DiagnosticCapPlusOne,
}

impl CleanupFailureCase {
    pub const ALL: [Self; 3] = [
        Self::SourceByteLimitPlusOne,
        Self::MissingReferencePerUnit,
        Self::DiagnosticCapPlusOne,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SourceByteLimitPlusOne => "limit/source-byte-count/plus-one",
            Self::MissingReferencePerUnit => "semantic/missing-reference-per-unit",
            Self::DiagnosticCapPlusOne => "diagnostic/cap-plus-one",
        }
    }

    pub const fn workload_id(self) -> ScalableWorkloadId {
        match self {
            Self::SourceByteLimitPlusOne => ScalableWorkloadId::Identity,
            Self::MissingReferencePerUnit | Self::DiagnosticCapPlusOne => {
                ScalableWorkloadId::Corridor
            }
        }
    }

    const fn input_variant_id(self) -> &'static str {
        match self {
            Self::SourceByteLimitPlusOne => "canonical-valid-v1",
            Self::MissingReferencePerUnit => "corridor-missing-reference-per-unit-v1",
            Self::DiagnosticCapPlusOne => "corridor-diagnostic-cap-plus-one-v1",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupPhase {
    BaselineSuccess,
    FailureIteration,
    RecoverySuccess,
    FreshInstanceOracle,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupRunObservation {
    pub experiment_id: String,
    pub sequence_index: u32,
    pub phase: CleanupPhase,
    pub compiler_instance_id: String,
    pub workload_id: ScalableWorkloadId,
    pub graph_profile: GraphProfileId,
    pub n: u32,
    pub case_id: CleanupFailureCase,
    pub input_variant_id: String,
    pub success: bool,
    pub stable_compiler_error_code: Option<String>,
    pub diagnostic_count: u64,
    pub diagnostics_truncated: bool,
    pub partial_output_record_count: u64,
    pub output_record_count: u64,
    pub stage_plan: ScalableStagePlanSummary,
    pub semantic_digest_sha256: Option<String>,
    pub diagnostic_digest_sha256: String,
    pub live_requested_bytes: u64,
    pub retained_capacity_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupExperiment {
    pub experiment_id: String,
    pub scale_role: CleanupScaleRole,
    pub case_id: CleanupFailureCase,
    pub workload_id: ScalableWorkloadId,
    pub graph_profile: GraphProfileId,
    pub n: u32,
    pub runs: Vec<CleanupRunObservation>,
}

#[derive(Clone, Copy, Debug)]
struct FailureDiagnostic {
    source_ordinal: u64,
    error_code: &'static str,
}

#[derive(Debug)]
struct BaselineCleanupCompilerInstance {
    compiler_instance_id: String,
    workload_id: ScalableWorkloadId,
    graph_profile: GraphProfileId,
    n: u32,
    plan: ScalableStagePlanSummary,
    compiler: ScalableAttributionCompilerInstance,
    diagnostics: Vec<FailureDiagnostic>,
    corridor_route_occurrences_per_unit: Option<u64>,
}

impl BaselineCleanupCompilerInstance {
    fn new(
        trusted: &TrustedContract,
        compiler_instance_id: String,
        workload_id: ScalableWorkloadId,
        graph_profile: GraphProfileId,
        n: u32,
    ) -> Result<Self, CleanupError> {
        let plan = ScalableStagePlanFactory::from_trusted_contract(trusted)?.plan(
            workload_id,
            graph_profile,
            n,
        )?;
        let compiler = ScalableAttributionCompilerInstance::from_trusted_contract_with_id(
            trusted,
            compiler_instance_id.clone(),
            workload_id,
        )?;
        let corridor_route_occurrences_per_unit = if workload_id == ScalableWorkloadId::Corridor {
            let contract = CorridorContract::from_manifest(&trusted.workload_manifest)?;
            let template = contract.load_template(&crate::repository_root())?;
            let count = u64::try_from(
                template
                    .relations
                    .iter()
                    .filter(|relation| matches!(relation, TemplateRelation::RouteOccurrence { .. }))
                    .count(),
            )
            .map_err(|_| CleanupError::RouteOccurrenceCountOverflow)?;
            if count == 0 {
                return Err(CleanupError::MissingRouteOccurrenceBasis);
            }
            Some(count)
        } else {
            None
        };
        Ok(Self {
            compiler_instance_id,
            workload_id,
            graph_profile,
            n,
            plan,
            compiler,
            diagnostics: Vec::new(),
            corridor_route_occurrences_per_unit,
        })
    }

    fn run_valid(
        &mut self,
        experiment_id: &str,
        sequence_index: u32,
        phase: CleanupPhase,
        case_id: CleanupFailureCase,
    ) -> Result<CleanupRunObservation, CleanupError> {
        let semantic_digest_sha256 = self.compiler.run_unmeasured(self.graph_profile, self.n)?;
        let retained_capacity_bytes = self.retained_capacity_bytes()?;
        Ok(CleanupRunObservation {
            experiment_id: experiment_id.to_owned(),
            sequence_index,
            phase,
            compiler_instance_id: self.compiler_instance_id.clone(),
            workload_id: self.workload_id,
            graph_profile: self.graph_profile,
            n: self.n,
            case_id,
            input_variant_id: "canonical-valid-v1".to_owned(),
            success: true,
            stable_compiler_error_code: None,
            diagnostic_count: 0,
            diagnostics_truncated: false,
            partial_output_record_count: 0,
            output_record_count: self.plan.counts.semantic_output_record,
            stage_plan: self.plan.clone(),
            semantic_digest_sha256: Some(semantic_digest_sha256),
            diagnostic_digest_sha256: empty_diagnostic_digest(),
            live_requested_bytes: 0,
            retained_capacity_bytes,
        })
    }

    fn run_failure(
        &mut self,
        planner: &LimitQualificationPlanner,
        experiment_id: &str,
        sequence_index: u32,
        case_id: CleanupFailureCase,
    ) -> Result<CleanupRunObservation, CleanupError> {
        self.diagnostics.clear();
        let (candidate_count, retained_count, error_code, diagnostics_truncated) = match case_id {
            CleanupFailureCase::SourceByteLimitPlusOne => {
                let pair = planner.plan_pair(
                    LimitDimensionId::SourceByteCount,
                    self.graph_profile,
                    self.n,
                    None,
                )?;
                let violation = enforce_selected_limit(&pair, pair.plus_one_limit_value)
                    .expect_err("plus-one plan must exceed the selected limit");
                if violation.error_code != LIMIT_EXCEEDED_ERROR_CODE {
                    return Err(CleanupError::UnexpectedLimitErrorCode);
                }
                (1, 1, LIMIT_EXCEEDED_ERROR_CODE, false)
            }
            CleanupFailureCase::MissingReferencePerUnit => {
                self.require_route_occurrence_candidates(u64::from(self.n))?;
                (
                    u64::from(self.n),
                    u64::from(self.n),
                    UNKNOWN_REFERENCE_ERROR_CODE,
                    false,
                )
            }
            CleanupFailureCase::DiagnosticCapPlusOne => {
                let retained = u64::from(self.n);
                let candidates = retained
                    .checked_add(1)
                    .ok_or(CleanupError::DiagnosticCountOverflow)?;
                self.require_route_occurrence_candidates(candidates)?;
                (candidates, retained, DIAGNOSTIC_LIMIT_ERROR_CODE, true)
            }
        };
        let retained_count_usize = usize::try_from(retained_count)
            .map_err(|_| CleanupError::DiagnosticCountTooLarge(retained_count))?;
        if self.diagnostics.capacity() < retained_count_usize {
            self.diagnostics
                .try_reserve_exact(retained_count_usize - self.diagnostics.len())
                .map_err(|source| CleanupError::DiagnosticReserve {
                    requested: retained_count,
                    source,
                })?;
        }
        for source_ordinal in 0..candidate_count {
            if self.diagnostics.len() == retained_count_usize {
                break;
            }
            self.diagnostics.push(FailureDiagnostic {
                source_ordinal,
                error_code,
            });
        }
        if self.diagnostics.len() != retained_count_usize {
            return Err(CleanupError::DiagnosticShapeMismatch);
        }
        let diagnostic_digest_sha256 = diagnostic_digest(&self.diagnostics);
        let diagnostic_count = u64::try_from(self.diagnostics.len())
            .map_err(|_| CleanupError::DiagnosticCountTooLarge(retained_count))?;
        self.diagnostics.clear();
        let live_requested_bytes = self.failure_live_requested_bytes()?;
        let retained_capacity_bytes = self.retained_capacity_bytes()?;
        Ok(CleanupRunObservation {
            experiment_id: experiment_id.to_owned(),
            sequence_index,
            phase: CleanupPhase::FailureIteration,
            compiler_instance_id: self.compiler_instance_id.clone(),
            workload_id: self.workload_id,
            graph_profile: self.graph_profile,
            n: self.n,
            case_id,
            input_variant_id: case_id.input_variant_id().to_owned(),
            success: false,
            stable_compiler_error_code: Some(error_code.to_owned()),
            diagnostic_count,
            diagnostics_truncated,
            partial_output_record_count: 0,
            output_record_count: 0,
            stage_plan: self.plan.clone(),
            semantic_digest_sha256: None,
            diagnostic_digest_sha256,
            live_requested_bytes,
            retained_capacity_bytes,
        })
    }

    fn require_route_occurrence_candidates(&self, required: u64) -> Result<(), CleanupError> {
        let per_unit = self
            .corridor_route_occurrences_per_unit
            .ok_or(CleanupError::MissingRouteOccurrenceBasis)?;
        let available = per_unit
            .checked_mul(u64::from(self.n))
            .ok_or(CleanupError::RouteOccurrenceCountOverflow)?;
        if required > available {
            return Err(CleanupError::InsufficientRouteOccurrenceBasis {
                required,
                available,
            });
        }
        Ok(())
    }

    fn failure_live_requested_bytes(&self) -> Result<u64, CleanupError> {
        u64::try_from(self.diagnostics.len())
            .ok()
            .and_then(|len| len.checked_mul(u64::try_from(size_of::<FailureDiagnostic>()).ok()?))
            .ok_or(CleanupError::RetainedCapacityOverflow)
    }

    fn retained_capacity_bytes(&self) -> Result<u64, CleanupError> {
        let pipeline = self.compiler.retained_capacity_bytes()?.total;
        let diagnostics = u64::try_from(self.diagnostics.capacity())
            .ok()
            .and_then(|capacity| {
                capacity.checked_mul(u64::try_from(size_of::<FailureDiagnostic>()).ok()?)
            })
            .ok_or(CleanupError::RetainedCapacityOverflow)?;
        pipeline
            .checked_add(diagnostics)
            .ok_or(CleanupError::RetainedCapacityOverflow)
    }
}

pub fn run_cleanup_experiment(
    trusted: &TrustedContract,
    experiment_id: String,
    scale_role: CleanupScaleRole,
    case_id: CleanupFailureCase,
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<CleanupExperiment, CleanupError> {
    if experiment_id.is_empty() {
        return Err(CleanupError::EmptyExperimentId);
    }
    if n == 0 {
        return Err(CleanupError::ScaleMustBePositive);
    }
    validate_cleanup_manifest(&trusted.workload_manifest)?;
    let workload_id = case_id.workload_id();
    let planner = LimitQualificationPlanner::from_trusted_contract(trusted)?;
    let primary_instance_id = format!("{experiment_id}/instance/main");
    let fresh_instance_id = format!("{experiment_id}/instance/fresh");
    let mut primary = BaselineCleanupCompilerInstance::new(
        trusted,
        primary_instance_id,
        workload_id,
        graph_profile,
        n,
    )?;
    let mut runs = Vec::with_capacity(CLEANUP_GROUP_RUN_COUNT);
    runs.push(primary.run_valid(&experiment_id, 0, CleanupPhase::BaselineSuccess, case_id)?);
    for sequence_index in 1..=CLEANUP_FAILURE_ITERATION_COUNT {
        runs.push(primary.run_failure(&planner, &experiment_id, sequence_index, case_id)?);
    }
    runs.push(primary.run_valid(&experiment_id, 33, CleanupPhase::RecoverySuccess, case_id)?);
    let mut fresh = BaselineCleanupCompilerInstance::new(
        trusted,
        fresh_instance_id,
        workload_id,
        graph_profile,
        n,
    )?;
    runs.push(fresh.run_valid(
        &experiment_id,
        34,
        CleanupPhase::FreshInstanceOracle,
        case_id,
    )?);
    let experiment = CleanupExperiment {
        experiment_id,
        scale_role,
        case_id,
        workload_id,
        graph_profile,
        n,
        runs,
    };
    validate_cleanup_experiment(&experiment)?;
    Ok(experiment)
}

fn validate_cleanup_manifest(manifest: &Value) -> Result<(), CleanupError> {
    let contract = object(manifest, "baselineCleanupObservationContract")?;
    require_u64(contract, "revision", 1)?;
    require_string(
        contract,
        "candidateId",
        "baseline-std-randomstate-stable-vec-v1",
    )?;
    require_u64(contract, "expectedExperimentCount", 6)?;
    require_u64(
        contract,
        "groupSize",
        u64::try_from(CLEANUP_GROUP_RUN_COUNT).expect("closed group size fits u64"),
    )?;
    require_string(
        contract,
        "sequenceRule",
        "exactly sequenceIndex 0, 1..32, 33 and 34 with no omission or duplicate",
    )?;

    let bindings = manifest
        .get("cleanupExperimentBindings")
        .and_then(Value::as_array)
        .ok_or(CleanupError::Manifest("cleanupExperimentBindings"))?;
    if bindings.len() != CleanupFailureCase::ALL.len() {
        return Err(CleanupError::Manifest("cleanupExperimentBindings.length"));
    }
    for (index, case_id) in CleanupFailureCase::ALL.into_iter().enumerate() {
        let binding = bindings
            .get(index)
            .ok_or(CleanupError::Manifest("cleanupExperimentBindings[]"))?;
        require_string(binding, "caseId", case_id.as_str())?;
        require_string(binding, "workloadId", case_id.workload_id().as_str())?;
        require_string(binding, "inputVariantId", case_id.input_variant_id())?;
        let source = object(binding, "scaleSource")?;
        require_string(source, "workloadId", case_id.workload_id().as_str())?;
        require_u64(source, "workloadRevision", 1)?;
        require_string(source, "graphProfile", "shared-fanin-dag-v1")?;
        require_string(source, "stringProfile", "short-unique-v1")?;
        require_u64(source, "generatorVersion", 1)?;
        require_string(source, "caseId", "not-applicable")?;
        let roles =
            binding
                .get("scaleRoles")
                .and_then(Value::as_array)
                .ok_or(CleanupError::Manifest(
                    "cleanupExperimentBindings[].scaleRoles",
                ))?;
        let actual = roles.iter().filter_map(Value::as_str).collect::<Vec<_>>();
        if actual != ["calibration", "stress"] {
            return Err(CleanupError::Manifest(
                "cleanupExperimentBindings[].scaleRoles",
            ));
        }
    }
    Ok(())
}

fn object<'a>(value: &'a Value, field: &'static str) -> Result<&'a Value, CleanupError> {
    value
        .get(field)
        .filter(|candidate| candidate.is_object())
        .ok_or(CleanupError::Manifest(field))
}

fn require_string(value: &Value, field: &'static str, expected: &str) -> Result<(), CleanupError> {
    if value.get(field).and_then(Value::as_str) == Some(expected) {
        Ok(())
    } else {
        Err(CleanupError::Manifest(field))
    }
}

fn require_u64(value: &Value, field: &'static str, expected: u64) -> Result<(), CleanupError> {
    if value.get(field).and_then(Value::as_u64) == Some(expected) {
        Ok(())
    } else {
        Err(CleanupError::Manifest(field))
    }
}

pub fn validate_cleanup_experiment(experiment: &CleanupExperiment) -> Result<(), CleanupError> {
    if experiment.runs.len() != CLEANUP_GROUP_RUN_COUNT {
        return Err(CleanupError::RunCount {
            actual: experiment.runs.len(),
        });
    }
    let main_instance_id = experiment
        .runs
        .first()
        .ok_or(CleanupError::RunCount { actual: 0 })?
        .compiler_instance_id
        .as_str();
    for (index, run) in experiment.runs.iter().enumerate() {
        if run.sequence_index != u32::try_from(index).expect("cleanup run count fits u32")
            || run.experiment_id != experiment.experiment_id
            || run.workload_id != experiment.workload_id
            || run.graph_profile != experiment.graph_profile
            || run.n != experiment.n
            || run.case_id != experiment.case_id
        {
            return Err(CleanupError::RunIdentity { index });
        }
        let expected_phase = match index {
            0 => CleanupPhase::BaselineSuccess,
            1..=32 => CleanupPhase::FailureIteration,
            33 => CleanupPhase::RecoverySuccess,
            34 => CleanupPhase::FreshInstanceOracle,
            _ => unreachable!("closed cleanup group size"),
        };
        if run.phase != expected_phase {
            return Err(CleanupError::RunPhase { index });
        }
        if index <= 33 && run.compiler_instance_id != main_instance_id {
            return Err(CleanupError::PrimaryInstanceChanged { index });
        }
    }
    if experiment.runs[34].compiler_instance_id == main_instance_id {
        return Err(CleanupError::FreshInstanceReused);
    }

    let first_failure_retained = experiment.runs[1].retained_capacity_bytes;
    for run in &experiment.runs[1..=32] {
        let (expected_code, expected_diagnostics, expected_truncated) = match experiment.case_id {
            CleanupFailureCase::SourceByteLimitPlusOne => (LIMIT_EXCEEDED_ERROR_CODE, 1, false),
            CleanupFailureCase::MissingReferencePerUnit => {
                (UNKNOWN_REFERENCE_ERROR_CODE, u64::from(experiment.n), false)
            }
            CleanupFailureCase::DiagnosticCapPlusOne => {
                (DIAGNOSTIC_LIMIT_ERROR_CODE, u64::from(experiment.n), true)
            }
        };
        if run.success
            || run.stable_compiler_error_code.as_deref() != Some(expected_code)
            || run.diagnostic_count != expected_diagnostics
            || run.diagnostics_truncated != expected_truncated
            || run.partial_output_record_count != 0
            || run.output_record_count != 0
            || run.semantic_digest_sha256.is_some()
            || run.live_requested_bytes != 0
        {
            return Err(CleanupError::FailureObservation {
                sequence_index: run.sequence_index,
            });
        }
        if run.sequence_index >= 2 && run.retained_capacity_bytes > first_failure_retained {
            return Err(CleanupError::RetainedCapacityGrew {
                sequence_index: run.sequence_index,
                first: first_failure_retained,
                actual: run.retained_capacity_bytes,
            });
        }
    }

    let baseline = &experiment.runs[0];
    let recovery = &experiment.runs[33];
    let fresh = &experiment.runs[34];
    for run in [baseline, recovery, fresh] {
        if !run.success
            || run.stable_compiler_error_code.is_some()
            || run.diagnostic_count != 0
            || run.diagnostics_truncated
            || run.partial_output_record_count != 0
            || run.semantic_digest_sha256.is_none()
            || run.diagnostic_digest_sha256 != empty_diagnostic_digest()
        {
            return Err(CleanupError::SuccessObservation {
                sequence_index: run.sequence_index,
            });
        }
    }
    if recovery.semantic_digest_sha256 != fresh.semantic_digest_sha256
        || recovery.stage_plan != fresh.stage_plan
        || recovery.output_record_count != fresh.output_record_count
        || recovery.stable_compiler_error_code != fresh.stable_compiler_error_code
        || recovery.partial_output_record_count != fresh.partial_output_record_count
        || baseline.semantic_digest_sha256 != recovery.semantic_digest_sha256
    {
        return Err(CleanupError::RecoveryMismatch);
    }
    Ok(())
}

fn diagnostic_digest(diagnostics: &[FailureDiagnostic]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"LANEFLOW-COMPILER-CALIBRATION-DIAGNOSTIC-V1\0");
    for diagnostic in diagnostics {
        hasher.update(diagnostic.source_ordinal.to_le_bytes());
        hasher.update(
            u32::try_from(diagnostic.error_code.len())
                .expect("closed diagnostic code length fits u32")
                .to_le_bytes(),
        );
        hasher.update(diagnostic.error_code.as_bytes());
    }
    lower_hex(&hasher.finalize())
}

fn empty_diagnostic_digest() -> String {
    diagnostic_digest(&[])
}

fn lower_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

#[derive(Debug, Error)]
pub enum CleanupError {
    #[error("清理实验清单字段缺失或与冻结契约不一致：{0}")]
    Manifest(&'static str),
    #[error("清理实验标识符不能为空")]
    EmptyExperimentId,
    #[error("清理实验规模 N 必须为正")]
    ScaleMustBePositive,
    #[error("清理实验运行数必须为 35：actual={actual}")]
    RunCount { actual: usize },
    #[error("清理实验运行身份错误：index={index}")]
    RunIdentity { index: usize },
    #[error("清理实验运行阶段错误：index={index}")]
    RunPhase { index: usize },
    #[error("清理实验主实例在序号 {index} 被替换")]
    PrimaryInstanceChanged { index: usize },
    #[error("清理实验新实例判定基准复用了主实例身份")]
    FreshInstanceReused,
    #[error("清理实验失败观察不闭合：sequenceIndex={sequence_index}")]
    FailureObservation { sequence_index: u32 },
    #[error(
        "清理实验失败保留容量继续增长：sequenceIndex={sequence_index}, first={first}, actual={actual}"
    )]
    RetainedCapacityGrew {
        sequence_index: u32,
        first: u64,
        actual: u64,
    },
    #[error("清理实验成功观察不闭合：sequenceIndex={sequence_index}")]
    SuccessObservation { sequence_index: u32 },
    #[error("恢复成功与新实例判定基准不一致")]
    RecoveryMismatch,
    #[error("诊断候选计数溢出")]
    DiagnosticCountOverflow,
    #[error("走廊路线出现项计数溢出")]
    RouteOccurrenceCountOverflow,
    #[error("走廊失败变体缺少实际路线出现项基数")]
    MissingRouteOccurrenceBasis,
    #[error("走廊失败变体路线出现项不足：required={required}, available={available}")]
    InsufficientRouteOccurrenceBasis { required: u64, available: u64 },
    #[error("诊断计数不能表示为 usize：{0}")]
    DiagnosticCountTooLarge(u64),
    #[error("诊断缓冲区保留 {requested} 项失败")]
    DiagnosticReserve {
        requested: u64,
        #[source]
        source: std::collections::TryReserveError,
    },
    #[error("诊断缓冲区结果形状不匹配")]
    DiagnosticShapeMismatch,
    #[error("保留容量字节计算溢出")]
    RetainedCapacityOverflow,
    #[error("资源加一失败返回了非规范错误码")]
    UnexpectedLimitErrorCode,
    #[error(transparent)]
    Limit(#[from] LimitQualificationError),
    #[error(transparent)]
    Plan(#[from] crate::ScalePlanError),
    #[error(transparent)]
    Timing(#[from] TimingError),
    #[error(transparent)]
    Corridor(#[from] crate::CorridorError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_repository_contract;

    #[test]
    fn every_cleanup_case_builds_the_exact_thirty_five_run_sequence() {
        let trusted = load_repository_contract().expect("trusted contract");
        for case_id in CleanupFailureCase::ALL {
            let experiment = run_cleanup_experiment(
                &trusted,
                format!("cleanup/{}/calibration", case_id.as_str()),
                CleanupScaleRole::Calibration,
                case_id,
                GraphProfileId::SharedFaninDag,
                1,
            )
            .expect("cleanup experiment");
            assert_eq!(experiment.runs.len(), CLEANUP_GROUP_RUN_COUNT);
            assert_eq!(
                experiment
                    .runs
                    .iter()
                    .filter(|run| run.phase == CleanupPhase::FailureIteration)
                    .count(),
                CLEANUP_FAILURE_ITERATION_COUNT as usize
            );
        }
    }

    #[test]
    fn calibration_and_stress_roles_form_the_frozen_six_groups() {
        let trusted = load_repository_contract().expect("trusted contract");
        let mut groups = Vec::new();
        for case_id in CleanupFailureCase::ALL {
            for (role, n) in [
                (CleanupScaleRole::Calibration, 1),
                (CleanupScaleRole::Stress, 2),
            ] {
                groups.push(
                    run_cleanup_experiment(
                        &trusted,
                        format!("cleanup/{}/{role:?}", case_id.as_str()),
                        role,
                        case_id,
                        GraphProfileId::SharedFaninDag,
                        n,
                    )
                    .expect("cleanup group"),
                );
            }
        }
        assert_eq!(groups.len(), 6);
        assert_eq!(
            groups.iter().map(|group| group.runs.len()).sum::<usize>(),
            6 * CLEANUP_GROUP_RUN_COUNT
        );
    }

    #[test]
    fn validator_rejects_instance_reuse_partial_output_and_capacity_growth() {
        let trusted = load_repository_contract().expect("trusted contract");
        let baseline = run_cleanup_experiment(
            &trusted,
            "cleanup/fault-injection".to_owned(),
            CleanupScaleRole::Calibration,
            CleanupFailureCase::MissingReferencePerUnit,
            GraphProfileId::SharedFaninDag,
            1,
        )
        .expect("baseline cleanup experiment");

        let mut reused = baseline.clone();
        reused.runs[34].compiler_instance_id = reused.runs[0].compiler_instance_id.clone();
        assert!(matches!(
            validate_cleanup_experiment(&reused),
            Err(CleanupError::FreshInstanceReused)
        ));

        let mut partial = baseline.clone();
        partial.runs[8].partial_output_record_count = 1;
        assert!(matches!(
            validate_cleanup_experiment(&partial),
            Err(CleanupError::FailureObservation { sequence_index: 8 })
        ));

        let mut growth = baseline;
        growth.runs[12].retained_capacity_bytes = growth.runs[1].retained_capacity_bytes + 1;
        assert!(matches!(
            validate_cleanup_experiment(&growth),
            Err(CleanupError::RetainedCapacityGrew {
                sequence_index: 12,
                ..
            })
        ));
    }

    #[test]
    fn cleanup_manifest_drift_is_rejected_before_running_a_group() {
        let mut trusted = load_repository_contract().expect("trusted contract");
        trusted.workload_manifest["baselineCleanupObservationContract"]["groupSize"] =
            Value::from(34);
        assert!(matches!(
            run_cleanup_experiment(
                &trusted,
                "cleanup/manifest-drift".to_owned(),
                CleanupScaleRole::Calibration,
                CleanupFailureCase::SourceByteLimitPlusOne,
                GraphProfileId::SharedFaninDag,
                1,
            ),
            Err(CleanupError::Manifest("groupSize"))
        ));
    }
}
