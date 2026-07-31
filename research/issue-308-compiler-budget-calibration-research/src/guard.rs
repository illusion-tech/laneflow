//! #308 研究停止护栏的父进程预检。
//!
//! 本模块负责从受信任工作负载清单重算规模形状、预测值和本机阈值，并提供系统物理
//! 内存与 Windows 进程私有字节观察。子进程受控分配和父进程监控执行位于 pipeline /
//! pilot，跨平台终止状态分类位于 process_protocol；Evidence v1 写出仍由后续切片
//! 负责。

use crate::{
    GraphProfileId, ScalableStagePlanFactory, ScalableStagePlanSummary, ScalableWorkloadId,
    StageBreakdown, TrustedContract,
};
use serde::Serialize;
use sysinfo::{
    IS_SUPPORTED_SYSTEM, MemoryRefreshKind, Pid, ProcessRefreshKind, ProcessesToUpdate, System,
};

const GIBIBYTE: u64 = 1_073_741_824;
const PREDICTION_SAFETY_FACTOR_NUMERATOR: u64 = 5;
const PREDICTION_SAFETY_FACTOR_DENOMINATOR: u64 = 4;

pub const COMPILER_CONTROLLED_HARD_CEILING_BYTES: u64 = 16 * GIBIBYTE;
pub const PRIVATE_MEMORY_HARD_CEILING_BYTES: u64 = 24 * GIBIBYTE;
pub const WALL_TIME_HARD_CEILING_NS: u64 = 60_000_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemMemoryObservation {
    pub physical_memory_bytes: u64,
    pub available_physical_memory_bytes: u64,
}

#[derive(Debug)]
pub struct SystemMemoryMonitor {
    system: System,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildProcessMemoryObservation {
    pub private_bytes: u64,
    pub available_physical_memory_bytes: u64,
}

#[derive(Debug)]
pub struct ChildProcessMemoryMonitor {
    system: System,
}

impl ChildProcessMemoryMonitor {
    pub fn new() -> Result<Self, GuardError> {
        if !IS_SUPPORTED_SYSTEM {
            return Err(GuardError::UnsupportedSystemMemoryProvider);
        }
        if !cfg!(windows) {
            return Err(GuardError::UnsupportedPrivateMemoryProvider);
        }
        Ok(Self {
            system: System::new(),
        })
    }

    pub fn observe(
        &mut self,
        child_pid: u32,
    ) -> Result<Option<ChildProcessMemoryObservation>, GuardError> {
        let pid = Pid::from_u32(child_pid);
        self.system
            .refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing().with_memory().without_tasks(),
        );
        let available_physical_memory_bytes = self.system.available_memory();
        let Some(process) = self.system.process(pid) else {
            return Ok(None);
        };
        // sysinfo 0.39.6 在 Windows 后端把 virtual_memory() 绑定到
        // PROCESS_MEMORY_COUNTERS_EX.PrivateUsage。本研究精确绑定该依赖版本，其他平台
        // 不得把通用虚拟内存值冒充进程私有字节。
        #[cfg(windows)]
        let private_bytes = process.virtual_memory();
        #[cfg(not(windows))]
        let private_bytes = 0;
        Ok(Some(ChildProcessMemoryObservation {
            private_bytes,
            available_physical_memory_bytes,
        }))
    }
}

impl SystemMemoryMonitor {
    pub fn new() -> Result<Self, GuardError> {
        if !IS_SUPPORTED_SYSTEM {
            return Err(GuardError::UnsupportedSystemMemoryProvider);
        }
        let mut system = System::new();
        system.refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
        validate_memory_observation(system.total_memory(), system.available_memory())?;
        Ok(Self { system })
    }

    pub fn observe(&mut self) -> Result<SystemMemoryObservation, GuardError> {
        self.system
            .refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
        validate_memory_observation(self.system.total_memory(), self.system.available_memory())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardThresholds {
    pub compiler_controlled_bytes: u64,
    pub private_bytes: u64,
    pub minimum_available_physical_memory_bytes: u64,
    pub wall_time_ns: u64,
    pub prediction_safety_factor_numerator: u64,
    pub prediction_safety_factor_denominator: u64,
}

impl GuardThresholds {
    pub fn from_physical_memory_bytes(physical_memory_bytes: u64) -> Result<Self, GuardError> {
        let compiler_controlled_bytes =
            (physical_memory_bytes / 4).min(COMPILER_CONTROLLED_HARD_CEILING_BYTES);
        let private_bytes = (physical_memory_bytes / 3).min(PRIVATE_MEMORY_HARD_CEILING_BYTES);
        let minimum_available_physical_memory_bytes = physical_memory_bytes / 4;
        if compiler_controlled_bytes == 0
            || private_bytes == 0
            || minimum_available_physical_memory_bytes == 0
        {
            return Err(GuardError::InvalidPhysicalMemory {
                physical_memory_bytes,
            });
        }
        Ok(Self {
            compiler_controlled_bytes,
            private_bytes,
            minimum_available_physical_memory_bytes,
            wall_time_ns: WALL_TIME_HARD_CEILING_NS,
            prediction_safety_factor_numerator: PREDICTION_SAFETY_FACTOR_NUMERATOR,
            prediction_safety_factor_denominator: PREDICTION_SAFETY_FACTOR_DENOMINATOR,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardCompletedLevelObservation {
    pub n: u32,
    pub primary_record_count: u64,
    pub peak_live_requested_bytes: u64,
    pub private_bytes: u64,
    pub wall_time_ns: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GuardPredictionBasis {
    ManifestSingleBufferLowerBoundV1,
    PreviousLevelLinearTimesFiveFourths,
    FirstLevelMonitorOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GuardTrigger {
    PredictedCompilerControlledBytes,
    PredictedPrivateBytes,
    PredictedWallTime,
    AvailablePhysicalMemory,
    TypedOrdinal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardPreflightReport {
    pub workload_id: String,
    pub graph_profile: String,
    pub n: u32,
    pub primary_record_count: u64,
    pub maximum_typed_ordinal: u64,
    pub logical_bytes_lower_bound: u64,
    pub compiler_controlled_prediction_basis: GuardPredictionBasis,
    pub private_bytes_prediction_basis: GuardPredictionBasis,
    pub wall_time_prediction_basis: GuardPredictionBasis,
    pub predicted_compiler_controlled_bytes: u64,
    pub predicted_private_bytes: Option<u64>,
    pub predicted_wall_time_ns: Option<u64>,
    pub memory_observation: SystemMemoryObservation,
    pub thresholds: GuardThresholds,
    pub triggers: Vec<GuardTrigger>,
    pub allows_child_start: bool,
}

#[derive(Clone, Debug)]
pub struct ScalableGuardPlanner {
    plans: ScalableStagePlanFactory,
}

impl ScalableGuardPlanner {
    pub fn from_trusted_contract(trusted: &TrustedContract) -> Result<Self, GuardError> {
        validate_guard_prediction_contract(&trusted.workload_manifest)?;
        Ok(Self {
            plans: ScalableStagePlanFactory::from_trusted_contract(trusted)?,
        })
    }

    pub fn evaluate(
        &self,
        workload_id: ScalableWorkloadId,
        graph_profile: GraphProfileId,
        n: u32,
        memory_observation: SystemMemoryObservation,
        previous: Option<GuardCompletedLevelObservation>,
    ) -> Result<GuardPreflightReport, GuardError> {
        validate_memory_observation(
            memory_observation.physical_memory_bytes,
            memory_observation.available_physical_memory_bytes,
        )?;
        let plan = self.plans.plan(workload_id, graph_profile, n)?;
        evaluate_scalable_guard_preflight(&self.plans, plan, memory_observation, previous)
    }

    pub fn evaluate_pilot(
        &self,
        workload_id: ScalableWorkloadId,
        graph_profile: GraphProfileId,
        n: u32,
        memory_observation: SystemMemoryObservation,
        previous: Option<GuardCompletedLevelObservation>,
    ) -> Result<GuardPreflightReport, GuardError> {
        self.evaluate(workload_id, graph_profile, n, memory_observation, previous)
    }
}

pub fn evaluate_identity_guard_preflight(
    trusted: &TrustedContract,
    graph_profile: GraphProfileId,
    n: u32,
    memory_observation: SystemMemoryObservation,
    previous: Option<GuardCompletedLevelObservation>,
) -> Result<GuardPreflightReport, GuardError> {
    ScalableGuardPlanner::from_trusted_contract(trusted)?.evaluate(
        ScalableWorkloadId::Identity,
        graph_profile,
        n,
        memory_observation,
        previous,
    )
}

fn evaluate_scalable_guard_preflight(
    plans: &ScalableStagePlanFactory,
    plan: ScalableStagePlanSummary,
    memory_observation: SystemMemoryObservation,
    previous: Option<GuardCompletedLevelObservation>,
) -> Result<GuardPreflightReport, GuardError> {
    let thresholds =
        GuardThresholds::from_physical_memory_bytes(memory_observation.physical_memory_bytes)?;
    let primary_record_count = plan.primary_record_count;
    if primary_record_count == 0 {
        return Err(GuardError::InvalidPrimaryRecordCount);
    }
    let maximum_typed_ordinal = maximum_typed_ordinal(&plan);
    let logical_bytes_lower_bound = manifest_single_buffer_lower_bound(&plan.stages);

    let (
        compiler_controlled_prediction_basis,
        private_bytes_prediction_basis,
        wall_time_prediction_basis,
        predicted_compiler_controlled_bytes,
        predicted_private_bytes,
        predicted_wall_time_ns,
    ) = if let Some(previous) = previous {
        validate_previous_observation(previous, plan.n)?;
        let previous_plan = plans.plan(plan.workload_id, plan.graph_profile, previous.n)?;
        if previous_plan.primary_record_count != previous.primary_record_count {
            return Err(GuardError::InvalidPreviousObservation);
        }
        let compiler_linear = checked_linear_prediction(
            previous.peak_live_requested_bytes,
            primary_record_count,
            previous.primary_record_count,
        )?;
        (
            GuardPredictionBasis::PreviousLevelLinearTimesFiveFourths,
            GuardPredictionBasis::PreviousLevelLinearTimesFiveFourths,
            GuardPredictionBasis::PreviousLevelLinearTimesFiveFourths,
            logical_bytes_lower_bound.max(compiler_linear),
            Some(checked_linear_prediction(
                previous.private_bytes,
                primary_record_count,
                previous.primary_record_count,
            )?),
            Some(checked_linear_prediction(
                previous.wall_time_ns,
                primary_record_count,
                previous.primary_record_count,
            )?),
        )
    } else {
        if plan.n != 1 {
            return Err(GuardError::MissingPreviousObservation { n: plan.n });
        }
        (
            GuardPredictionBasis::ManifestSingleBufferLowerBoundV1,
            GuardPredictionBasis::FirstLevelMonitorOnly,
            GuardPredictionBasis::FirstLevelMonitorOnly,
            logical_bytes_lower_bound,
            None,
            None,
        )
    };

    let mut triggers = Vec::new();
    triggers
        .try_reserve_exact(5)
        .map_err(GuardError::TriggerAllocation)?;
    if maximum_typed_ordinal > u64::from(u32::MAX) {
        triggers.push(GuardTrigger::TypedOrdinal);
    }
    if predicted_compiler_controlled_bytes >= thresholds.compiler_controlled_bytes {
        triggers.push(GuardTrigger::PredictedCompilerControlledBytes);
    }
    if predicted_private_bytes.is_some_and(|value| value >= thresholds.private_bytes) {
        triggers.push(GuardTrigger::PredictedPrivateBytes);
    }
    if predicted_wall_time_ns.is_some_and(|value| value >= thresholds.wall_time_ns) {
        triggers.push(GuardTrigger::PredictedWallTime);
    }
    if memory_observation.available_physical_memory_bytes
        < thresholds.minimum_available_physical_memory_bytes
    {
        triggers.push(GuardTrigger::AvailablePhysicalMemory);
    }

    Ok(GuardPreflightReport {
        workload_id: plan.workload_id.as_str().to_owned(),
        graph_profile: plan.graph_profile.as_str().to_owned(),
        n: plan.n,
        primary_record_count,
        maximum_typed_ordinal,
        logical_bytes_lower_bound,
        compiler_controlled_prediction_basis,
        private_bytes_prediction_basis,
        wall_time_prediction_basis,
        predicted_compiler_controlled_bytes,
        predicted_private_bytes,
        predicted_wall_time_ns,
        memory_observation,
        thresholds,
        allows_child_start: triggers.is_empty(),
        triggers,
    })
}

fn validate_memory_observation(
    physical_memory_bytes: u64,
    available_physical_memory_bytes: u64,
) -> Result<SystemMemoryObservation, GuardError> {
    if physical_memory_bytes == 0 || available_physical_memory_bytes > physical_memory_bytes {
        return Err(GuardError::InvalidMemoryObservation {
            physical_memory_bytes,
            available_physical_memory_bytes,
        });
    }
    Ok(SystemMemoryObservation {
        physical_memory_bytes,
        available_physical_memory_bytes,
    })
}

fn validate_previous_observation(
    previous: GuardCompletedLevelObservation,
    n: u32,
) -> Result<(), GuardError> {
    if previous.n == 0 || previous.primary_record_count == 0 || previous.wall_time_ns == 0 {
        return Err(GuardError::InvalidPreviousObservation);
    }
    let expected_n = previous
        .n
        .checked_mul(2)
        .ok_or(GuardError::CheckedArithmetic)?;
    if expected_n != n {
        return Err(GuardError::InvalidPreviousObservation);
    }
    Ok(())
}

fn maximum_typed_ordinal(plan: &ScalableStagePlanSummary) -> u64 {
    [
        plan.counts.module_count,
        plan.counts.source_span_count,
        plan.counts.symbol_count,
        plan.counts.semantic_output_record,
        plan.stages.source_input.record_count,
        plan.stages.typed_ast.record_count,
        plan.stages.hir.record_count,
        plan.stages.mir.record_count,
        plan.stages.canonical_lir.record_count,
        plan.stages.output_construction.record_count,
    ]
    .into_iter()
    .map(|count| count.saturating_sub(1))
    .max()
    .unwrap_or(0)
}

fn manifest_single_buffer_lower_bound(stages: &StageBreakdown) -> u64 {
    [
        stages.source_input.logical_bytes,
        stages.typed_ast.record_allocation_bytes,
        stages.hir.record_allocation_bytes,
        stages.mir.record_allocation_bytes,
        stages.canonical_lir.record_allocation_bytes,
        stages.diagnostics.logical_bytes,
        stages.scratch.logical_bytes,
        stages.output_construction.logical_bytes,
    ]
    .into_iter()
    .max()
    .unwrap_or(0)
}

fn checked_linear_prediction(
    previous_value: u64,
    next_primary_record_count: u64,
    previous_primary_record_count: u64,
) -> Result<u64, GuardError> {
    let numerator = u128::from(previous_value)
        .checked_mul(u128::from(next_primary_record_count))
        .and_then(|value| value.checked_mul(u128::from(PREDICTION_SAFETY_FACTOR_NUMERATOR)))
        .ok_or(GuardError::CheckedArithmetic)?;
    let denominator = u128::from(previous_primary_record_count)
        .checked_mul(u128::from(PREDICTION_SAFETY_FACTOR_DENOMINATOR))
        .ok_or(GuardError::CheckedArithmetic)?;
    if denominator == 0 {
        return Err(GuardError::CheckedArithmetic);
    }
    let quotient = numerator / denominator;
    let rounded = quotient
        .checked_add(u128::from(numerator % denominator != 0))
        .ok_or(GuardError::CheckedArithmetic)?;
    u64::try_from(rounded).map_err(|_| GuardError::CheckedArithmetic)
}

fn validate_guard_prediction_contract(manifest: &serde_json::Value) -> Result<(), GuardError> {
    let guard = required_object(manifest, "guardPredictionContract")?;
    require_string(guard, "id", "research-stop-guard-prediction-v1")?;
    let primary_by_workload = required_object(guard, "primaryRecordCountByWorkload")?;
    let primary = required_object(primary_by_workload, "LF-COMP-ID-v1")?;
    require_string(primary, "aggregate", "sum")?;
    require_string_array(primary, "operands", &["identityFieldOccurrenceCount"])?;
    let primary = required_object(primary_by_workload, "LF-COMP-CORRIDOR-v1")?;
    require_string(primary, "aggregate", "sum")?;
    require_string_array(
        primary,
        "operands",
        &["sourceRelationCount", "sourceGeometryCount"],
    )?;
    let primary = required_object(primary_by_workload, "LF-COMP-JUNCTION-GRID-v1")?;
    require_string(primary, "aggregate", "sum")?;
    require_string_array(
        primary,
        "operands",
        &["gateOccurrence", "waitingZoneOccurrence", "routeOccurrence"],
    )?;

    let lower = required_object(guard, "compilerControlledLowerBound")?;
    require_string(lower, "basis", "manifest-single-buffer-lower-bound-v1")?;
    require_string(lower, "aggregate", "maximum")?;
    let operands = lower
        .get("singleBufferOperands")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| GuardError::ManifestContract("singleBufferOperands".to_owned()))?;
    let expected = [
        ("sourceInput", "logicalBytes"),
        ("typedAst", "recordAllocationBytes"),
        ("hir", "recordAllocationBytes"),
        ("mir", "recordAllocationBytes"),
        ("canonicalLir", "recordAllocationBytes"),
        ("diagnostics", "logicalBytes"),
        ("scratch", "logicalBytes"),
        ("outputConstruction", "logicalBytes"),
    ];
    if operands.len() != expected.len()
        || operands
            .iter()
            .zip(expected)
            .any(|(actual, (stage, field))| {
                actual.get("stage").and_then(serde_json::Value::as_str) != Some(stage)
                    || actual.get("field").and_then(serde_json::Value::as_str) != Some(field)
            })
    {
        return Err(GuardError::ManifestContract(
            "singleBufferOperands".to_owned(),
        ));
    }
    require_string(
        lower,
        "fixedOverheadRule",
        "no separate fixed-overhead term exists in protocol v1",
    )?;
    require_string(
        guard,
        "firstLevelPrediction",
        "predictedCompilerControlledBytes = logicalBytesLowerBound",
    )?;
    require_string(
        guard,
        "laterLevelCompilerControlledLinearTerm",
        "ceil(previousPeakLiveRequestedBytes * nextPrimaryRecordCount * 5 / (previousPrimaryRecordCount * 4))",
    )?;
    require_string(
        guard,
        "laterLevelCompilerControlledPrediction",
        "predictedCompilerControlledBytes = max(logicalBytesLowerBound, laterLevelCompilerControlledLinearTerm)",
    )?;
    require_string(
        guard,
        "laterLevelPrivateBytesPrediction",
        "predictedPrivateBytes = ceil(previousPrivateBytes * nextPrimaryRecordCount * 5 / (previousPrimaryRecordCount * 4))",
    )?;
    require_string(
        guard,
        "laterLevelWallTimePrediction",
        "predictedWallTimeNs = ceil(previousWallTimeNs * nextPrimaryRecordCount * 5 / (previousPrimaryRecordCount * 4))",
    )?;
    require_string(
        guard,
        "integerArithmetic",
        "evaluate checked u128 products; compute ceiling division from quotient and nonzero remainder without numerator-plus-denominator-minus-one; convert the final result with u64::try_from",
    )?;
    require_string(
        guard,
        "candidateIndependenceRule",
        "the lower bound is derived only from the bound manifest and selected workload level; candidates cannot add, subtract, or measure a private fixed-overhead value",
    )?;
    Ok(())
}

fn required_object<'a>(
    value: &'a serde_json::Value,
    field: &str,
) -> Result<&'a serde_json::Value, GuardError> {
    value
        .get(field)
        .filter(|candidate| candidate.is_object())
        .ok_or_else(|| GuardError::ManifestContract(field.to_owned()))
}

fn require_string(
    value: &serde_json::Value,
    field: &str,
    expected: &str,
) -> Result<(), GuardError> {
    if value.get(field).and_then(serde_json::Value::as_str) != Some(expected) {
        return Err(GuardError::ManifestContract(field.to_owned()));
    }
    Ok(())
}

fn require_string_array(
    value: &serde_json::Value,
    field: &str,
    expected: &[&str],
) -> Result<(), GuardError> {
    let actual = value
        .get(field)
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| GuardError::ManifestContract(field.to_owned()))?;
    if actual.len() != expected.len()
        || actual
            .iter()
            .zip(expected)
            .any(|(actual, expected)| actual.as_str() != Some(expected))
    {
        return Err(GuardError::ManifestContract(field.to_owned()));
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum GuardError {
    #[error(transparent)]
    Stage(#[from] crate::StageGenerationError),
    #[error(transparent)]
    ScalePlan(#[from] crate::ScalePlanError),
    #[error("系统信息提供程序不支持当前操作系统")]
    UnsupportedSystemMemoryProvider,
    #[error("当前平台没有经 #308 绑定的进程私有字节监控提供程序")]
    UnsupportedPrivateMemoryProvider,
    #[error("物理内存字节数无法形成正护栏阈值：{physical_memory_bytes}")]
    InvalidPhysicalMemory { physical_memory_bytes: u64 },
    #[error(
        "系统内存观察无效：physicalMemoryBytes={physical_memory_bytes}, availablePhysicalMemoryBytes={available_physical_memory_bytes}"
    )]
    InvalidMemoryObservation {
        physical_memory_bytes: u64,
        available_physical_memory_bytes: u64,
    },
    #[error("前一完成级别观察与严格二倍规模或预测输入不一致")]
    InvalidPreviousObservation,
    #[error("规模 N={n} 不是首级，必须提供前一完成级别观察")]
    MissingPreviousObservation { n: u32 },
    #[error("主记录数必须为正")]
    InvalidPrimaryRecordCount,
    #[error("停止护栏预测发生受检算术失败")]
    CheckedArithmetic,
    #[error("停止护栏触发器容量预留失败")]
    TriggerAllocation(#[source] std::collections::TryReserveError),
    #[error("工作负载清单停止护栏契约不匹配：{0}")]
    ManifestContract(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_repository_contract;

    const TEST_PHYSICAL_MEMORY_BYTES: u64 = 64 * GIBIBYTE;

    #[test]
    fn first_level_uses_manifest_lower_bound_and_live_memory_threshold() {
        let trusted = load_repository_contract().expect("frozen contract");
        let report = evaluate_identity_guard_preflight(
            &trusted,
            GraphProfileId::WideStar,
            1,
            SystemMemoryObservation {
                physical_memory_bytes: TEST_PHYSICAL_MEMORY_BYTES,
                available_physical_memory_bytes: 32 * GIBIBYTE,
            },
            None,
        )
        .expect("first-level preflight");

        assert_eq!(report.primary_record_count, 57);
        assert_eq!(report.logical_bytes_lower_bound, 7_743);
        assert_eq!(
            report.predicted_compiler_controlled_bytes,
            report.logical_bytes_lower_bound
        );
        assert_eq!(report.predicted_private_bytes, None);
        assert_eq!(report.predicted_wall_time_ns, None);
        assert_eq!(report.thresholds.compiler_controlled_bytes, 16 * GIBIBYTE);
        assert_eq!(
            report.thresholds.private_bytes,
            TEST_PHYSICAL_MEMORY_BYTES / 3
        );
        assert_eq!(
            report.thresholds.minimum_available_physical_memory_bytes,
            16 * GIBIBYTE
        );
        assert!(report.triggers.is_empty());
        assert!(report.allows_child_start);
    }

    #[test]
    fn available_memory_below_one_quarter_rejects_before_start() {
        let trusted = load_repository_contract().expect("frozen contract");
        let report = evaluate_identity_guard_preflight(
            &trusted,
            GraphProfileId::WideStar,
            1,
            SystemMemoryObservation {
                physical_memory_bytes: TEST_PHYSICAL_MEMORY_BYTES,
                available_physical_memory_bytes: (16 * GIBIBYTE) - 1,
            },
            None,
        )
        .expect("guard report");

        assert_eq!(report.triggers, [GuardTrigger::AvailablePhysicalMemory]);
        assert!(!report.allows_child_start);
    }

    #[test]
    fn later_level_predictions_use_checked_exact_ceiling_arithmetic() {
        let trusted = load_repository_contract().expect("frozen contract");
        let report = evaluate_identity_guard_preflight(
            &trusted,
            GraphProfileId::DeepChain,
            2,
            SystemMemoryObservation {
                physical_memory_bytes: TEST_PHYSICAL_MEMORY_BYTES,
                available_physical_memory_bytes: 32 * GIBIBYTE,
            },
            Some(GuardCompletedLevelObservation {
                n: 1,
                primary_record_count: 57,
                peak_live_requested_bytes: 10_001,
                private_bytes: 201,
                wall_time_ns: 301,
            }),
        )
        .expect("later-level preflight");

        assert_eq!(report.primary_record_count, 114);
        assert_eq!(report.predicted_compiler_controlled_bytes, 25_003);
        assert_eq!(report.predicted_private_bytes, Some(503));
        assert_eq!(report.predicted_wall_time_ns, Some(753));
        assert!(report.allows_child_start);
    }

    #[test]
    fn pilot_later_level_cannot_fall_back_to_first_level_prediction() {
        let trusted = load_repository_contract().expect("frozen contract");
        let planner = ScalableGuardPlanner::from_trusted_contract(&trusted).expect("guard planner");
        assert!(matches!(
            planner.evaluate_pilot(
                ScalableWorkloadId::Identity,
                GraphProfileId::WideStar,
                2,
                SystemMemoryObservation {
                    physical_memory_bytes: TEST_PHYSICAL_MEMORY_BYTES,
                    available_physical_memory_bytes: 32 * GIBIBYTE,
                },
                None,
            ),
            Err(GuardError::MissingPreviousObservation { n: 2 })
        ));
    }

    #[test]
    fn arithmetic_overflow_fails_closed() {
        let trusted = load_repository_contract().expect("frozen contract");
        assert!(matches!(
            evaluate_identity_guard_preflight(
                &trusted,
                GraphProfileId::WideStar,
                2,
                SystemMemoryObservation {
                    physical_memory_bytes: TEST_PHYSICAL_MEMORY_BYTES,
                    available_physical_memory_bytes: 32 * GIBIBYTE,
                },
                Some(GuardCompletedLevelObservation {
                    n: 1,
                    primary_record_count: 57,
                    peak_live_requested_bytes: u64::MAX,
                    private_bytes: u64::MAX,
                    wall_time_ns: u64::MAX,
                }),
            ),
            Err(GuardError::CheckedArithmetic)
        ));
    }

    #[test]
    fn previous_primary_record_count_must_match_the_bound_manifest() {
        let trusted = load_repository_contract().expect("frozen contract");
        assert!(matches!(
            evaluate_identity_guard_preflight(
                &trusted,
                GraphProfileId::WideStar,
                2,
                SystemMemoryObservation {
                    physical_memory_bytes: TEST_PHYSICAL_MEMORY_BYTES,
                    available_physical_memory_bytes: 32 * GIBIBYTE,
                },
                Some(GuardCompletedLevelObservation {
                    n: 1,
                    primary_record_count: 58,
                    peak_live_requested_bytes: 1,
                    private_bytes: 1,
                    wall_time_ns: 1,
                }),
            ),
            Err(GuardError::InvalidPreviousObservation)
        ));
    }

    #[test]
    fn typed_ordinal_overflow_is_rejected_without_materializing_the_level() {
        let trusted = load_repository_contract().expect("frozen contract");
        let previous_n = 1_073_741_824;
        let report = evaluate_identity_guard_preflight(
            &trusted,
            GraphProfileId::WideStar,
            2_147_483_648,
            SystemMemoryObservation {
                physical_memory_bytes: TEST_PHYSICAL_MEMORY_BYTES,
                available_physical_memory_bytes: 32 * GIBIBYTE,
            },
            Some(GuardCompletedLevelObservation {
                n: previous_n,
                primary_record_count: 57 * u64::from(previous_n),
                peak_live_requested_bytes: 1,
                private_bytes: 1,
                wall_time_ns: 1,
            }),
        )
        .expect("large level preflight remains allocation-free");
        assert!(report.triggers.contains(&GuardTrigger::TypedOrdinal));
        assert!(!report.allows_child_start);
    }

    #[test]
    fn guard_manifest_formula_drift_is_rejected_before_prediction() {
        let mut trusted = load_repository_contract().expect("frozen contract");
        trusted.workload_manifest["guardPredictionContract"]["firstLevelPrediction"] =
            serde_json::Value::String("changed".to_owned());
        assert!(matches!(
            evaluate_identity_guard_preflight(
                &trusted,
                GraphProfileId::WideStar,
                1,
                SystemMemoryObservation {
                    physical_memory_bytes: TEST_PHYSICAL_MEMORY_BYTES,
                    available_physical_memory_bytes: 32 * GIBIBYTE,
                },
                None,
            ),
            Err(GuardError::ManifestContract(field)) if field == "firstLevelPrediction"
        ));
    }
}
