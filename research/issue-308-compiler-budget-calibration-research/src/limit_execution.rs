//! #308 全限制维度配对、独立 attribution 基线与清理实验执行。
//!
//! 本模块只消费正式阶梯已选择的校准/压力规模。普通限制在禁止的线性工作前执行私有
//! 预检；诊断限制实际构造有界诊断；编译器控制存续字节由两个新 attribution 子进程
//! 取得共同峰值后，再由同一基线候选执行 at-bound/plus-one。

use crate::ladder_runner::{FormalLadderExecution, decode_child_execution};
use crate::pilot::{run_monitored_command_child, run_monitored_scalable_role_child};
use crate::timing::ScalableFailureInput;
use crate::{
    ATTRIBUTION_BINARY_ID, BASE_SCALE_STRING_PROFILE, ChildProcessMonitorReport, CleanupError,
    CleanupExperiment, CleanupFailureCase, CleanupScaleRole, DIAGNOSTIC_LIMIT_ERROR_CODE,
    DUPLICATE_OWNER_ERROR_CODE, ExternalStateObservation, FailureInputDigestBinding,
    FormalLadderRunnerError, GENERATOR_VERSION_V1, GraphProfileId, GuardThresholds,
    InvalidationReason, LIMIT_EXCEEDED_ERROR_CODE, LimitDimensionId, LimitPairMode, LimitPairPlan,
    LimitQualificationError, LimitQualificationPlanner, LiveByteBaseline, LiveByteBaselineReplica,
    ProcessObservation, RunStatus, SCALABLE_ATTRIBUTION_CHILD_SCHEMA,
    SCALABLE_ATTRIBUTION_CHILD_SCHEMA_VERSION, ScalableAttributionChildReport,
    ScalableAttributionCompilerInstance, ScalableAttributionOutcome, ScalableStagePlanSummary,
    ScalableTimingCompilerInstance, ScalableWorkloadId, TIMING_BINARY_ID, TimingError,
    TrustedContract, UNKNOWN_REFERENCE_ERROR_CODE, WORKLOAD_REVISION_V1, run_cleanup_experiment,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;
use thiserror::Error;

pub const EXPECTED_LIMIT_SCALE_COUNT: usize = 18;
pub const EXPECTED_LIMIT_PAIR_COUNT: usize = 138;
pub const EXPECTED_LIVE_BYTE_BASELINE_RUN_COUNT: usize = 12;
pub const EXPECTED_CLEANUP_EXPERIMENT_COUNT: usize = 6;
pub const EXPECTED_DUPLICATE_OWNER_QUALIFICATION_COUNT: usize = 6;
pub const LIMIT_SIDE_CHILD_SCHEMA: &str = "laneflow.compiler-calibration-limit-side-child";
pub const LIMIT_SIDE_CHILD_SCHEMA_VERSION: u32 = 1;
pub const SEMANTIC_FAILURE_CHILD_SCHEMA: &str =
    "laneflow.compiler-calibration-semantic-failure-child";
pub const SEMANTIC_FAILURE_CHILD_SCHEMA_VERSION: u32 = 1;
pub const CLEANUP_CHILD_SCHEMA: &str = "laneflow.compiler-calibration-cleanup-child";
pub const CLEANUP_CHILD_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LimitRunOutcome {
    Success,
    CompilerError,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LimitPairSide {
    AtBound,
    PlusOne,
}

impl LimitPairSide {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AtBound => "at-bound",
            Self::PlusOne => "plus-one",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitQualificationScale {
    pub workload_id: ScalableWorkloadId,
    pub graph_profile: GraphProfileId,
    pub b: u32,
    pub scale_role: CleanupScaleRole,
    pub n: u32,
    pub guard_thresholds: GuardThresholds,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitBaselineProcessRun {
    pub run_id: String,
    pub scale: LimitQualificationScale,
    pub replica_index: u32,
    pub compiler_instance_id: String,
    pub status: RunStatus,
    pub invalidation_reasons: Vec<InvalidationReason>,
    pub process: ProcessObservation,
    pub child: Option<ScalableAttributionChildReport>,
    pub monitor: ChildProcessMonitorReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_state: Option<ExternalStateObservation>,
    pub kill_error: Option<String>,
    pub monitor_error: Option<String>,
    pub stderr: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitPairRunObservation {
    pub run_id: String,
    pub input_digest_sha256: String,
    pub expected_outcome: LimitRunOutcome,
    pub actual_outcome: LimitRunOutcome,
    pub selected_limit_value: u64,
    pub stable_compiler_error_code: Option<String>,
    pub diagnostic_count: u64,
    pub diagnostics_truncated: bool,
    pub partial_output_record_count: u64,
    pub output_record_count: u64,
    pub semantic_digest_sha256: Option<String>,
    pub diagnostic_digest_sha256: String,
    pub live_requested_bytes_after_run: u64,
    pub peak_live_requested_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitSideChildReport {
    pub schema: String,
    pub schema_version: u32,
    pub binary_id: String,
    pub child_pid: u32,
    pub compiler_instance_id: String,
    pub side: LimitPairSide,
    pub observation: LimitPairRunObservation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticFailureChildReport {
    pub schema: String,
    pub schema_version: u32,
    pub binary_id: String,
    pub child_pid: u32,
    pub compiler_instance_id: String,
    pub case_id: String,
    pub input_variant_id: String,
    pub observation: LimitPairRunObservation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupChildReport {
    pub schema: String,
    pub schema_version: u32,
    pub binary_id: String,
    pub child_pid: u32,
    pub experiment: CleanupExperiment,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitoredQualificationRun<T> {
    pub status: RunStatus,
    pub invalidation_reasons: Vec<InvalidationReason>,
    pub process: ProcessObservation,
    pub child: Option<T>,
    pub monitor: ChildProcessMonitorReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_state: Option<ExternalStateObservation>,
    pub kill_error: Option<String>,
    pub monitor_error: Option<String>,
    pub stderr: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitPairExecution {
    pub scale: LimitQualificationScale,
    pub pair: LimitPairPlan,
    pub at_bound: MonitoredQualificationRun<LimitSideChildReport>,
    pub plus_one: MonitoredQualificationRun<LimitSideChildReport>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateOwnerQualification {
    pub scale: LimitQualificationScale,
    pub case_id: String,
    pub input_variant_id: String,
    pub run: MonitoredQualificationRun<SemanticFailureChildReport>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupQualification {
    pub scale: LimitQualificationScale,
    pub case_id: CleanupFailureCase,
    pub run: MonitoredQualificationRun<CleanupChildReport>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitQualificationBundle {
    pub scales: Vec<LimitQualificationScale>,
    pub live_byte_baseline_runs: Vec<LimitBaselineProcessRun>,
    pub limit_pairs: Vec<LimitPairExecution>,
    pub duplicate_owner_qualifications: Vec<DuplicateOwnerQualification>,
    pub cleanup_experiments: Vec<CleanupQualification>,
}

pub fn measure_limit_side_child(
    trusted: &TrustedContract,
    binary_id: &str,
    compiler_instance_id: String,
    scale: &LimitQualificationScale,
    pair: &LimitPairPlan,
    side: LimitPairSide,
) -> Result<LimitSideChildReport, LimitQualificationExecutionError> {
    let expected_binary_id = match pair.binding.pair_mode {
        LimitPairMode::BaselineLiveBytePrescanV1 => ATTRIBUTION_BINARY_ID,
        LimitPairMode::SuccessAtBound | LimitPairMode::DiagnosticCapOnSemanticFailure => {
            TIMING_BINARY_ID
        }
    };
    if binary_id != expected_binary_id {
        return Err(LimitQualificationExecutionError::WrongLimitSideBinary {
            expected: expected_binary_id,
            actual: binary_id.to_owned(),
        });
    }
    if pair.binding.workload_id != scale.workload_id
        || pair.graph_profile != scale.graph_profile
        || pair.n != scale.n
    {
        return Err(LimitQualificationExecutionError::LimitSideScaleMismatch);
    }
    let (at_bound, plus_one) = execute_limit_pair_payloads(trusted, scale, pair)?;
    let observation = match side {
        LimitPairSide::AtBound => at_bound,
        LimitPairSide::PlusOne => plus_one,
    };
    let expected_instance_id = format!("{}/compiler-instance", observation.run_id);
    if compiler_instance_id != expected_instance_id {
        return Err(LimitQualificationExecutionError::LimitSideCompilerInstanceMismatch);
    }
    Ok(LimitSideChildReport {
        schema: LIMIT_SIDE_CHILD_SCHEMA.to_owned(),
        schema_version: LIMIT_SIDE_CHILD_SCHEMA_VERSION,
        binary_id: binary_id.to_owned(),
        child_pid: std::process::id(),
        compiler_instance_id,
        side,
        observation,
    })
}

fn execute_limit_pair_payloads(
    trusted: &TrustedContract,
    scale: &LimitQualificationScale,
    pair: &LimitPairPlan,
) -> Result<(LimitPairRunObservation, LimitPairRunObservation), LimitQualificationExecutionError> {
    Ok(match pair.binding.pair_mode {
        LimitPairMode::SuccessAtBound => execute_ordinary_limit_pair(trusted, scale, pair)?,
        LimitPairMode::DiagnosticCapOnSemanticFailure => {
            execute_diagnostic_limit_pair(trusted, scale, pair)?
        }
        LimitPairMode::BaselineLiveBytePrescanV1 => {
            execute_live_byte_limit_pair(trusted, scale, pair)?
        }
    })
}

pub fn measure_duplicate_owner_child(
    trusted: &TrustedContract,
    compiler_instance_id: String,
    scale: &LimitQualificationScale,
) -> Result<SemanticFailureChildReport, LimitQualificationExecutionError> {
    if scale.workload_id != ScalableWorkloadId::Corridor {
        return Err(LimitQualificationExecutionError::DuplicateOwnerWrongWorkload);
    }
    let observation = execute_duplicate_owner_observation(trusted, scale)?;
    if compiler_instance_id != format!("{}/compiler-instance", observation.run_id) {
        return Err(LimitQualificationExecutionError::LimitSideCompilerInstanceMismatch);
    }
    Ok(SemanticFailureChildReport {
        schema: SEMANTIC_FAILURE_CHILD_SCHEMA.to_owned(),
        schema_version: SEMANTIC_FAILURE_CHILD_SCHEMA_VERSION,
        binary_id: TIMING_BINARY_ID.to_owned(),
        child_pid: std::process::id(),
        compiler_instance_id,
        case_id: "semantic/duplicate-owner-per-unit".to_owned(),
        input_variant_id: "corridor-duplicate-owner-per-unit-v1".to_owned(),
        observation,
    })
}

pub fn measure_cleanup_child(
    trusted: &TrustedContract,
    scale: &LimitQualificationScale,
    case_id: CleanupFailureCase,
) -> Result<CleanupChildReport, LimitQualificationExecutionError> {
    if scale.workload_id != case_id.workload_id()
        || scale.graph_profile != GraphProfileId::SharedFaninDag
    {
        return Err(LimitQualificationExecutionError::CleanupChildScaleMismatch);
    }
    let experiment_id = format!(
        "cleanup/{}/{}/n-{}",
        case_id.as_str(),
        scale.scale_role.as_str(),
        scale.n
    );
    let experiment = run_cleanup_experiment(
        trusted,
        experiment_id,
        scale.scale_role,
        case_id,
        scale.graph_profile,
        scale.n,
        scale.b,
    )?;
    Ok(CleanupChildReport {
        schema: CLEANUP_CHILD_SCHEMA.to_owned(),
        schema_version: CLEANUP_CHILD_SCHEMA_VERSION,
        binary_id: ATTRIBUTION_BINARY_ID.to_owned(),
        child_pid: std::process::id(),
        experiment,
    })
}

pub fn derive_limit_qualification_scales(
    formal_ladders: &[FormalLadderExecution],
) -> Result<Vec<LimitQualificationScale>, LimitQualificationExecutionError> {
    let mut scales = Vec::with_capacity(EXPECTED_LIMIT_SCALE_COUNT);
    let mut identities = BTreeSet::new();
    for ladder in formal_ladders {
        let graph_profile = parse_graph_profile(&ladder.graph_profile)?;
        let analysis = ladder.analysis.as_ref().ok_or(
            LimitQualificationExecutionError::MissingScaleSelection {
                workload_id: ladder.workload_id,
                graph_profile,
            },
        )?;
        for (scale_role, n) in [
            (
                CleanupScaleRole::Calibration,
                analysis.scale_selection.calibration_n,
            ),
            (CleanupScaleRole::Stress, analysis.scale_selection.stress_n),
        ] {
            let n = n.ok_or(LimitQualificationExecutionError::MissingSelectedScale {
                workload_id: ladder.workload_id,
                graph_profile,
                scale_role,
            })?;
            let level = ladder
                .levels
                .iter()
                .find(|level| level.n == n && level.complete)
                .ok_or(LimitQualificationExecutionError::SelectedScaleNotComplete {
                    workload_id: ladder.workload_id,
                    graph_profile,
                    scale_role,
                    n,
                })?;
            if !identities.insert((ladder.workload_id, graph_profile, scale_role)) {
                return Err(LimitQualificationExecutionError::DuplicateScaleIdentity {
                    workload_id: ladder.workload_id,
                    graph_profile,
                    scale_role,
                });
            }
            scales.push(LimitQualificationScale {
                workload_id: ladder.workload_id,
                graph_profile,
                b: ladder.b,
                scale_role,
                n,
                guard_thresholds: level.guard_preflight.thresholds,
            });
        }
    }
    let expected_identities = ScalableWorkloadId::ALL
        .into_iter()
        .flat_map(|workload_id| {
            GraphProfileId::ALL
                .into_iter()
                .flat_map(move |graph_profile| {
                    [CleanupScaleRole::Calibration, CleanupScaleRole::Stress]
                        .into_iter()
                        .map(move |scale_role| (workload_id, graph_profile, scale_role))
                })
        })
        .collect::<BTreeSet<_>>();
    if identities != expected_identities || scales.len() != EXPECTED_LIMIT_SCALE_COUNT {
        return Err(LimitQualificationExecutionError::IncompleteScaleSet {
            actual: scales.len(),
        });
    }
    Ok(scales)
}

pub fn run_live_byte_baseline_processes(
    attribution_executable: &Path,
    scale: &LimitQualificationScale,
) -> Result<Vec<LimitBaselineProcessRun>, LimitQualificationExecutionError> {
    if scale.workload_id != ScalableWorkloadId::Identity {
        return Err(
            LimitQualificationExecutionError::LiveByteBaselineWrongWorkload(scale.workload_id),
        );
    }
    let mut runs = Vec::with_capacity(2);
    for replica in 0..2_usize {
        let run_id = format!(
            "limit-baseline/{}/{}/{}/n-{}/replica-{replica}",
            scale.workload_id.as_str(),
            scale.graph_profile.as_str(),
            scale.scale_role.as_str(),
            scale.n
        );
        let compiler_instance_id = format!("{run_id}/compiler-instance");
        let execution = run_monitored_scalable_role_child(
            attribution_executable,
            replica,
            "run-preflight",
            &compiler_instance_id,
            scale.workload_id,
            scale.graph_profile,
            scale.n,
            scale.guard_thresholds,
        )?;
        let decoded = decode_child_execution(
            execution,
            ATTRIBUTION_BINARY_ID,
            |report: &ScalableAttributionChildReport| {
                validate_live_byte_baseline_child(report, &compiler_instance_id, scale)
            },
            |report| report.outcome == ScalableAttributionOutcome::GuardedInChild,
        )?;
        runs.push(LimitBaselineProcessRun {
            run_id,
            scale: scale.clone(),
            replica_index: u32::try_from(replica).expect("两个存续字节基线副本索引适合 u32"),
            compiler_instance_id,
            status: decoded.status,
            invalidation_reasons: decoded.invalidation_reasons,
            process: decoded.process,
            child: decoded.child,
            monitor: decoded.monitor,
            external_state: decoded.external_state,
            kill_error: decoded.kill_error,
            monitor_error: decoded.monitor_error,
            stderr: decoded.stderr,
        });
    }
    validate_live_byte_baseline_runs(&runs, scale)?;
    Ok(runs)
}

pub fn run_limit_qualification_bundle(
    trusted: &TrustedContract,
    timing_executable: &Path,
    attribution_executable: &Path,
    formal_ladders: &[FormalLadderExecution],
) -> Result<LimitQualificationBundle, LimitQualificationExecutionError> {
    let scales = derive_limit_qualification_scales(formal_ladders)?;
    let planner = LimitQualificationPlanner::from_trusted_contract(trusted)?;
    let mut live_byte_baseline_runs = Vec::with_capacity(EXPECTED_LIVE_BYTE_BASELINE_RUN_COUNT);
    let mut limit_pairs = Vec::with_capacity(EXPECTED_LIMIT_PAIR_COUNT);
    for scale in &scales {
        let live_runs = if scale.workload_id == ScalableWorkloadId::Identity {
            let runs = run_live_byte_baseline_processes(attribution_executable, scale)?;
            live_byte_baseline_runs.extend(runs.iter().cloned());
            Some(runs)
        } else {
            None
        };
        for binding in planner
            .bindings()
            .iter()
            .filter(|binding| binding.workload_id == scale.workload_id)
        {
            let baseline =
                if binding.dimension_id == LimitDimensionId::CompilerControlledLiveByteCount {
                    Some(live_byte_baseline_from_runs(
                        live_runs
                            .as_deref()
                            .ok_or(LimitQualificationExecutionError::MissingLiveByteBaselineRuns)?,
                        scale,
                    )?)
                } else {
                    None
                };
            let pair =
                planner.plan_pair(binding.dimension_id, scale.graph_profile, scale.n, baseline)?;
            limit_pairs.push(execute_limit_pair(
                timing_executable,
                attribution_executable,
                scale,
                pair,
            )?);
        }
    }

    let mut cleanup_experiments = Vec::with_capacity(EXPECTED_CLEANUP_EXPERIMENT_COUNT);
    for case_id in CleanupFailureCase::ALL {
        for scale_role in [CleanupScaleRole::Calibration, CleanupScaleRole::Stress] {
            let scale = scales
                .iter()
                .find(|scale| {
                    scale.workload_id == case_id.workload_id()
                        && scale.graph_profile == GraphProfileId::SharedFaninDag
                        && scale.scale_role == scale_role
                })
                .ok_or(LimitQualificationExecutionError::MissingCleanupScale {
                    case_id,
                    scale_role,
                })?;
            cleanup_experiments.push(CleanupQualification {
                scale: scale.clone(),
                case_id,
                run: run_cleanup_process(attribution_executable, scale, case_id)?,
            });
        }
    }
    let duplicate_owner_qualifications = scales
        .iter()
        .filter(|scale| scale.workload_id == ScalableWorkloadId::Corridor)
        .map(|scale| run_duplicate_owner_process(timing_executable, scale))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(LimitQualificationBundle {
        scales,
        live_byte_baseline_runs,
        limit_pairs,
        duplicate_owner_qualifications,
        cleanup_experiments,
    })
}

pub fn validate_limit_qualification_bundle(
    bundle: &LimitQualificationBundle,
) -> Result<(), LimitQualificationExecutionError> {
    if bundle.scales.len() != EXPECTED_LIMIT_SCALE_COUNT {
        return Err(LimitQualificationExecutionError::IncompleteScaleSet {
            actual: bundle.scales.len(),
        });
    }
    if bundle.limit_pairs.len() != EXPECTED_LIMIT_PAIR_COUNT {
        return Err(LimitQualificationExecutionError::IncompleteLimitPairSet {
            actual: bundle.limit_pairs.len(),
        });
    }
    if bundle.live_byte_baseline_runs.len() != EXPECTED_LIVE_BYTE_BASELINE_RUN_COUNT {
        return Err(
            LimitQualificationExecutionError::IncompleteLiveByteBaselineSet {
                actual: bundle.live_byte_baseline_runs.len(),
            },
        );
    }
    if bundle.cleanup_experiments.len() != EXPECTED_CLEANUP_EXPERIMENT_COUNT {
        return Err(LimitQualificationExecutionError::IncompleteCleanupSet {
            actual: bundle.cleanup_experiments.len(),
        });
    }
    if bundle.duplicate_owner_qualifications.len() != EXPECTED_DUPLICATE_OWNER_QUALIFICATION_COUNT {
        return Err(
            LimitQualificationExecutionError::IncompleteDuplicateOwnerQualificationSet {
                actual: bundle.duplicate_owner_qualifications.len(),
            },
        );
    }
    let mut pair_identities = BTreeSet::new();
    for execution in &bundle.limit_pairs {
        let at_bound = valid_limit_side_observation(&execution.at_bound, LimitPairSide::AtBound)?;
        let plus_one = valid_limit_side_observation(&execution.plus_one, LimitPairSide::PlusOne)?;
        let identity = (
            execution.scale.workload_id,
            execution.scale.graph_profile,
            execution.scale.scale_role,
            execution.pair.binding.dimension_id,
        );
        if !pair_identities.insert(identity)
            || execution.pair.n != execution.scale.n
            || execution.pair.graph_profile != execution.scale.graph_profile
            || at_bound.selected_limit_value != execution.pair.at_bound_limit_value
            || plus_one.selected_limit_value != execution.pair.plus_one_limit_value
            || at_bound.expected_outcome != at_bound.actual_outcome
            || plus_one.expected_outcome != plus_one.actual_outcome
            || at_bound.partial_output_record_count != 0
            || plus_one.partial_output_record_count != 0
        {
            return Err(LimitQualificationExecutionError::InvalidLimitPairObservation);
        }
    }
    if pair_identities.len() != EXPECTED_LIMIT_PAIR_COUNT {
        return Err(LimitQualificationExecutionError::IncompleteLimitPairSet {
            actual: pair_identities.len(),
        });
    }
    for qualification in &bundle.cleanup_experiments {
        let experiment = valid_cleanup_experiment(&qualification.run)?;
        if qualification.scale.workload_id != experiment.workload_id
            || qualification.scale.graph_profile != experiment.graph_profile
            || qualification.scale.scale_role != experiment.scale_role
            || qualification.scale.n != experiment.n
            || qualification.case_id != experiment.case_id
        {
            return Err(LimitQualificationExecutionError::InvalidCleanupChild);
        }
        crate::validate_cleanup_experiment(experiment)?;
    }
    let expected_duplicate_owner_identities =
        [CleanupScaleRole::Calibration, CleanupScaleRole::Stress]
            .into_iter()
            .flat_map(|scale_role| {
                GraphProfileId::ALL
                    .into_iter()
                    .map(move |graph_profile| (graph_profile, scale_role))
            })
            .collect::<BTreeSet<_>>();
    let mut actual_duplicate_owner_identities = BTreeSet::new();
    for qualification in &bundle.duplicate_owner_qualifications {
        let child = valid_duplicate_owner_child(&qualification.run)?;
        let run = &child.observation;
        if qualification.scale.workload_id != ScalableWorkloadId::Corridor
            || qualification.case_id != "semantic/duplicate-owner-per-unit"
            || qualification.input_variant_id != "corridor-duplicate-owner-per-unit-v1"
            || child.case_id != qualification.case_id
            || child.input_variant_id != qualification.input_variant_id
            || run.expected_outcome != LimitRunOutcome::CompilerError
            || run.actual_outcome != LimitRunOutcome::CompilerError
            || run.stable_compiler_error_code.as_deref() != Some(DUPLICATE_OWNER_ERROR_CODE)
            || run.diagnostic_count != u64::from(qualification.scale.n)
            || run.diagnostics_truncated
            || run.partial_output_record_count != 0
            || run.output_record_count != 0
            || run.semantic_digest_sha256.is_some()
            || run.live_requested_bytes_after_run != 0
            || !actual_duplicate_owner_identities.insert((
                qualification.scale.graph_profile,
                qualification.scale.scale_role,
            ))
        {
            return Err(LimitQualificationExecutionError::InvalidDuplicateOwnerQualification);
        }
    }
    if actual_duplicate_owner_identities != expected_duplicate_owner_identities {
        return Err(
            LimitQualificationExecutionError::IncompleteDuplicateOwnerQualificationSet {
                actual: actual_duplicate_owner_identities.len(),
            },
        );
    }
    Ok(())
}

fn valid_limit_side_observation(
    run: &MonitoredQualificationRun<LimitSideChildReport>,
    expected_side: LimitPairSide,
) -> Result<&LimitPairRunObservation, LimitQualificationExecutionError> {
    if run.status != RunStatus::Valid
        || !run.invalidation_reasons.is_empty()
        || run.process.exit_kind != crate::ProcessExitKind::Success
    {
        return Err(LimitQualificationExecutionError::InvalidLimitPairObservation);
    }
    let child = run
        .child
        .as_ref()
        .ok_or(LimitQualificationExecutionError::InvalidLimitPairObservation)?;
    if child.side != expected_side {
        return Err(LimitQualificationExecutionError::InvalidLimitPairObservation);
    }
    Ok(&child.observation)
}

fn valid_duplicate_owner_child(
    run: &MonitoredQualificationRun<SemanticFailureChildReport>,
) -> Result<&SemanticFailureChildReport, LimitQualificationExecutionError> {
    if run.status != RunStatus::Valid
        || !run.invalidation_reasons.is_empty()
        || run.process.exit_kind != crate::ProcessExitKind::Success
    {
        return Err(LimitQualificationExecutionError::InvalidDuplicateOwnerQualification);
    }
    run.child
        .as_ref()
        .ok_or(LimitQualificationExecutionError::InvalidDuplicateOwnerQualification)
}

fn valid_cleanup_experiment(
    run: &MonitoredQualificationRun<CleanupChildReport>,
) -> Result<&CleanupExperiment, LimitQualificationExecutionError> {
    if run.status != RunStatus::Valid
        || !run.invalidation_reasons.is_empty()
        || run.process.exit_kind != crate::ProcessExitKind::Success
    {
        return Err(LimitQualificationExecutionError::InvalidCleanupChild);
    }
    Ok(&run
        .child
        .as_ref()
        .ok_or(LimitQualificationExecutionError::InvalidCleanupChild)?
        .experiment)
}

fn execute_duplicate_owner_observation(
    trusted: &TrustedContract,
    scale: &LimitQualificationScale,
) -> Result<LimitPairRunObservation, LimitQualificationExecutionError> {
    let run_id = format!(
        "failure/semantic-duplicate-owner/{}/{}/n-{}",
        scale.graph_profile.as_str(),
        scale.scale_role.as_str(),
        scale.n
    );
    let input_digest_sha256 = crate::input_digest::failure_input_digest_with_private_limits(
        &failure_input_binding(
            trusted,
            scale,
            &crate::ScalableStagePlanFactory::from_trusted_contract(trusted)?.plan(
                ScalableWorkloadId::Corridor,
                scale.graph_profile,
                scale.n,
            )?,
            "semantic/duplicate-owner-per-unit",
            "corridor-duplicate-owner-per-unit-v1",
            "not-applicable",
            &[],
        ),
        None,
        &[],
    );
    let mut compiler = ScalableTimingCompilerInstance::from_trusted_contract_with_id(
        trusted,
        format!("{run_id}/compiler-instance"),
        ScalableWorkloadId::Corridor,
    )?;
    let report = compiler.run_failure(
        scale.graph_profile,
        scale.n,
        ScalableFailureInput::DuplicateOwnerPerUnit,
    )?;
    Ok(LimitPairRunObservation {
        run_id,
        input_digest_sha256,
        expected_outcome: LimitRunOutcome::CompilerError,
        actual_outcome: LimitRunOutcome::CompilerError,
        selected_limit_value: 0,
        stable_compiler_error_code: Some(report.stable_compiler_error_code.to_owned()),
        diagnostic_count: report.diagnostic_count,
        diagnostics_truncated: report.diagnostics_truncated,
        partial_output_record_count: report.partial_output_record_count,
        output_record_count: report.output_record_count,
        semantic_digest_sha256: None,
        diagnostic_digest_sha256: report.diagnostic_digest_sha256,
        live_requested_bytes_after_run: report.live_requested_bytes_after_run,
        peak_live_requested_bytes: None,
    })
}

fn run_duplicate_owner_process(
    timing_executable: &Path,
    scale: &LimitQualificationScale,
) -> Result<DuplicateOwnerQualification, LimitQualificationExecutionError> {
    let run_id = format!(
        "failure/semantic-duplicate-owner/{}/{}/n-{}",
        scale.graph_profile.as_str(),
        scale.scale_role.as_str(),
        scale.n
    );
    let compiler_instance_id = format!("{run_id}/compiler-instance");
    let arguments = [
        "run-duplicate-owner".to_owned(),
        compiler_instance_id.clone(),
        serde_json::to_string(scale)?,
    ];
    let execution =
        run_monitored_command_child(timing_executable, 0, &arguments, scale.guard_thresholds)?;
    let decoded = decode_child_execution(
        execution,
        TIMING_BINARY_ID,
        |report: &SemanticFailureChildReport| {
            if report.schema == SEMANTIC_FAILURE_CHILD_SCHEMA
                && report.schema_version == SEMANTIC_FAILURE_CHILD_SCHEMA_VERSION
                && report.binary_id == TIMING_BINARY_ID
                && report.compiler_instance_id == compiler_instance_id
                && report.case_id == "semantic/duplicate-owner-per-unit"
                && report.input_variant_id == "corridor-duplicate-owner-per-unit-v1"
                && report.observation.run_id == run_id
            {
                Ok(())
            } else {
                Err("semantic-failure-child-protocol".to_owned())
            }
        },
        |_| false,
    )?;
    if decoded
        .child
        .as_ref()
        .is_some_and(|report| decoded.process.child_pid.value != Some(u64::from(report.child_pid)))
    {
        return Err(LimitQualificationExecutionError::SemanticFailureChildPidMismatch);
    }
    Ok(DuplicateOwnerQualification {
        scale: scale.clone(),
        case_id: "semantic/duplicate-owner-per-unit".to_owned(),
        input_variant_id: "corridor-duplicate-owner-per-unit-v1".to_owned(),
        run: MonitoredQualificationRun {
            status: decoded.status,
            invalidation_reasons: decoded.invalidation_reasons,
            process: decoded.process,
            child: decoded.child,
            monitor: decoded.monitor,
            external_state: decoded.external_state,
            kill_error: decoded.kill_error,
            monitor_error: decoded.monitor_error,
            stderr: decoded.stderr,
        },
    })
}

fn run_cleanup_process(
    attribution_executable: &Path,
    scale: &LimitQualificationScale,
    case_id: CleanupFailureCase,
) -> Result<MonitoredQualificationRun<CleanupChildReport>, LimitQualificationExecutionError> {
    let experiment_id = format!(
        "cleanup/{}/{}/n-{}",
        case_id.as_str(),
        scale.scale_role.as_str(),
        scale.n
    );
    let arguments = [
        "run-cleanup".to_owned(),
        serde_json::to_string(scale)?,
        serde_json::to_string(&case_id)?,
    ];
    let execution = run_monitored_command_child(
        attribution_executable,
        0,
        &arguments,
        scale.guard_thresholds,
    )?;
    let decoded = decode_child_execution(
        execution,
        ATTRIBUTION_BINARY_ID,
        |report: &CleanupChildReport| {
            let experiment = &report.experiment;
            if report.schema == CLEANUP_CHILD_SCHEMA
                && report.schema_version == CLEANUP_CHILD_SCHEMA_VERSION
                && report.binary_id == ATTRIBUTION_BINARY_ID
                && experiment.experiment_id == experiment_id
                && experiment.scale_role == scale.scale_role
                && experiment.case_id == case_id
                && experiment.workload_id == scale.workload_id
                && experiment.graph_profile == scale.graph_profile
                && experiment.n == scale.n
            {
                Ok(())
            } else {
                Err("cleanup-child-protocol".to_owned())
            }
        },
        |_| false,
    )?;
    if decoded
        .child
        .as_ref()
        .is_some_and(|report| decoded.process.child_pid.value != Some(u64::from(report.child_pid)))
    {
        return Err(LimitQualificationExecutionError::CleanupChildPidMismatch);
    }
    Ok(MonitoredQualificationRun {
        status: decoded.status,
        invalidation_reasons: decoded.invalidation_reasons,
        process: decoded.process,
        child: decoded.child,
        monitor: decoded.monitor,
        external_state: decoded.external_state,
        kill_error: decoded.kill_error,
        monitor_error: decoded.monitor_error,
        stderr: decoded.stderr,
    })
}

fn execute_limit_pair(
    timing_executable: &Path,
    attribution_executable: &Path,
    scale: &LimitQualificationScale,
    pair: LimitPairPlan,
) -> Result<LimitPairExecution, LimitQualificationExecutionError> {
    let (executable, binary_id) = match pair.binding.pair_mode {
        LimitPairMode::BaselineLiveBytePrescanV1 => (attribution_executable, ATTRIBUTION_BINARY_ID),
        LimitPairMode::SuccessAtBound | LimitPairMode::DiagnosticCapOnSemanticFailure => {
            (timing_executable, TIMING_BINARY_ID)
        }
    };
    let at_bound =
        run_limit_side_process(executable, binary_id, scale, &pair, LimitPairSide::AtBound)?;
    let plus_one =
        run_limit_side_process(executable, binary_id, scale, &pair, LimitPairSide::PlusOne)?;
    Ok(LimitPairExecution {
        scale: scale.clone(),
        pair,
        at_bound,
        plus_one,
    })
}

fn run_limit_side_process(
    executable: &Path,
    binary_id: &'static str,
    scale: &LimitQualificationScale,
    pair: &LimitPairPlan,
    side: LimitPairSide,
) -> Result<MonitoredQualificationRun<LimitSideChildReport>, LimitQualificationExecutionError> {
    let run_id = limit_run_id(scale, pair.binding.dimension_id, side.as_str());
    let compiler_instance_id = format!("{run_id}/compiler-instance");
    let scale_json = serde_json::to_string(scale)?;
    let pair_json = serde_json::to_string(pair)?;
    let arguments = [
        "run-limit-side".to_owned(),
        compiler_instance_id.clone(),
        scale_json,
        pair_json,
        side.as_str().to_owned(),
    ];
    let execution = run_monitored_command_child(executable, 0, &arguments, scale.guard_thresholds)?;
    let decoded = decode_child_execution(
        execution,
        binary_id,
        |report: &LimitSideChildReport| {
            if report.schema == LIMIT_SIDE_CHILD_SCHEMA
                && report.schema_version == LIMIT_SIDE_CHILD_SCHEMA_VERSION
                && report.binary_id == binary_id
                && report.compiler_instance_id == compiler_instance_id
                && report.side == side
                && report.observation.run_id == run_id
                && report.observation.selected_limit_value
                    == match side {
                        LimitPairSide::AtBound => pair.at_bound_limit_value,
                        LimitPairSide::PlusOne => pair.plus_one_limit_value,
                    }
            {
                Ok(())
            } else {
                Err("limit-side-child-protocol".to_owned())
            }
        },
        |_| false,
    )?;
    if decoded
        .child
        .as_ref()
        .is_some_and(|report| decoded.process.child_pid.value != Some(u64::from(report.child_pid)))
    {
        return Err(LimitQualificationExecutionError::LimitSideChildPidMismatch);
    }
    Ok(MonitoredQualificationRun {
        status: decoded.status,
        invalidation_reasons: decoded.invalidation_reasons,
        process: decoded.process,
        child: decoded.child,
        monitor: decoded.monitor,
        external_state: decoded.external_state,
        kill_error: decoded.kill_error,
        monitor_error: decoded.monitor_error,
        stderr: decoded.stderr,
    })
}

fn execute_ordinary_limit_pair(
    trusted: &TrustedContract,
    scale: &LimitQualificationScale,
    pair: &LimitPairPlan,
) -> Result<(LimitPairRunObservation, LimitPairRunObservation), LimitQualificationExecutionError> {
    let at_bound_run_id = limit_run_id(scale, pair.binding.dimension_id, "at-bound");
    let mut instance = ScalableTimingCompilerInstance::from_trusted_contract_with_id(
        trusted,
        format!("{at_bound_run_id}/compiler-instance"),
        scale.workload_id,
    )?;
    let digest = instance.run_unmeasured_with_selected_limit(
        scale.graph_profile,
        scale.n,
        pair.binding.dimension_id,
        pair.at_bound_limit_value,
    )?;
    let plan = crate::ScalableStagePlanFactory::from_trusted_contract(trusted)?.plan(
        scale.workload_id,
        scale.graph_profile,
        scale.n,
    )?;
    let at_bound = LimitPairRunObservation {
        run_id: at_bound_run_id,
        input_digest_sha256: limit_pair_input_digest(
            trusted,
            &plan,
            scale,
            pair,
            LimitPairSide::AtBound,
        ),
        expected_outcome: LimitRunOutcome::Success,
        actual_outcome: LimitRunOutcome::Success,
        selected_limit_value: pair.at_bound_limit_value,
        stable_compiler_error_code: None,
        diagnostic_count: 0,
        diagnostics_truncated: false,
        partial_output_record_count: 0,
        output_record_count: plan.counts.semantic_output_record,
        semantic_digest_sha256: Some(digest),
        diagnostic_digest_sha256: empty_diagnostic_digest(),
        live_requested_bytes_after_run: 0,
        peak_live_requested_bytes: None,
    };
    let plus_one_run_id = limit_run_id(scale, pair.binding.dimension_id, "plus-one");
    let mut plus_one_instance = ScalableTimingCompilerInstance::from_trusted_contract_with_id(
        trusted,
        format!("{plus_one_run_id}/compiler-instance"),
        scale.workload_id,
    )?;
    let (dimension_id, selected_limit_value, actual_value) = match plus_one_instance
        .run_unmeasured_with_selected_limit(
            scale.graph_profile,
            scale.n,
            pair.binding.dimension_id,
            pair.plus_one_limit_value,
        ) {
        Err(TimingError::SelectedLimitExceeded {
            dimension_id,
            selected_limit_value,
            actual_value,
        }) => (dimension_id, selected_limit_value, actual_value),
        Ok(_) => return Err(LimitQualificationExecutionError::OrdinaryPlusOneSucceeded),
        Err(source) => return Err(source.into()),
    };
    if dimension_id != pair.binding.dimension_id
        || selected_limit_value != pair.plus_one_limit_value
        || actual_value != pair.exact_dimension_value
    {
        return Err(LimitQualificationExecutionError::OrdinaryLimitFailureMismatch);
    }
    let plus_one = LimitPairRunObservation {
        run_id: plus_one_run_id,
        input_digest_sha256: limit_pair_input_digest(
            trusted,
            &plan,
            scale,
            pair,
            LimitPairSide::PlusOne,
        ),
        expected_outcome: LimitRunOutcome::CompilerError,
        actual_outcome: LimitRunOutcome::CompilerError,
        selected_limit_value: pair.plus_one_limit_value,
        stable_compiler_error_code: Some(LIMIT_EXCEEDED_ERROR_CODE.to_owned()),
        diagnostic_count: 1,
        diagnostics_truncated: false,
        partial_output_record_count: 0,
        output_record_count: 0,
        semantic_digest_sha256: None,
        diagnostic_digest_sha256: limit_diagnostic_digest(
            dimension_id.one_based_code_u8(),
            LIMIT_EXCEEDED_ERROR_CODE,
            selected_limit_value,
            actual_value,
        ),
        live_requested_bytes_after_run: 0,
        peak_live_requested_bytes: None,
    };
    Ok((at_bound, plus_one))
}

fn execute_diagnostic_limit_pair(
    trusted: &TrustedContract,
    scale: &LimitQualificationScale,
    pair: &LimitPairPlan,
) -> Result<(LimitPairRunObservation, LimitPairRunObservation), LimitQualificationExecutionError> {
    let plan = crate::ScalableStagePlanFactory::from_trusted_contract(trusted)?.plan(
        scale.workload_id,
        scale.graph_profile,
        scale.n,
    )?;
    let at_bound_run_id = limit_run_id(scale, pair.binding.dimension_id, "at-bound");
    let mut at_bound_instance = ScalableTimingCompilerInstance::from_trusted_contract_with_id(
        trusted,
        format!("{at_bound_run_id}/compiler-instance"),
        scale.workload_id,
    )?;
    let at_bound_report = at_bound_instance.run_failure(
        scale.graph_profile,
        scale.n,
        ScalableFailureInput::MissingReferencePerUnit,
    )?;
    let plus_one_run_id = limit_run_id(scale, pair.binding.dimension_id, "plus-one");
    let mut plus_one_instance = ScalableTimingCompilerInstance::from_trusted_contract_with_id(
        trusted,
        format!("{plus_one_run_id}/compiler-instance"),
        scale.workload_id,
    )?;
    let plus_one_report = plus_one_instance.run_failure(
        scale.graph_profile,
        scale.n,
        ScalableFailureInput::MissingReferencePerUnitWithMaximumDiagnostics {
            maximum_diagnostics: pair.plus_one_limit_value,
        },
    )?;
    if at_bound_report.stable_compiler_error_code != UNKNOWN_REFERENCE_ERROR_CODE
        || at_bound_report.diagnostic_count != pair.exact_dimension_value
        || at_bound_report.diagnostics_truncated
        || plus_one_report.stable_compiler_error_code != DIAGNOSTIC_LIMIT_ERROR_CODE
        || plus_one_report.diagnostic_count != pair.plus_one_limit_value
        || !plus_one_report.diagnostics_truncated
    {
        return Err(LimitQualificationExecutionError::InvalidDiagnosticLimitPair);
    }
    let at_bound = LimitPairRunObservation {
        run_id: at_bound_run_id,
        input_digest_sha256: limit_pair_input_digest(
            trusted,
            &plan,
            scale,
            pair,
            LimitPairSide::AtBound,
        ),
        expected_outcome: LimitRunOutcome::CompilerError,
        actual_outcome: LimitRunOutcome::CompilerError,
        selected_limit_value: pair.at_bound_limit_value,
        stable_compiler_error_code: Some(UNKNOWN_REFERENCE_ERROR_CODE.to_owned()),
        diagnostic_count: at_bound_report.diagnostic_count,
        diagnostics_truncated: at_bound_report.diagnostics_truncated,
        partial_output_record_count: at_bound_report.partial_output_record_count,
        output_record_count: at_bound_report.output_record_count,
        semantic_digest_sha256: None,
        diagnostic_digest_sha256: at_bound_report.diagnostic_digest_sha256,
        live_requested_bytes_after_run: at_bound_report.live_requested_bytes_after_run,
        peak_live_requested_bytes: None,
    };
    let plus_one = LimitPairRunObservation {
        run_id: plus_one_run_id,
        input_digest_sha256: limit_pair_input_digest(
            trusted,
            &plan,
            scale,
            pair,
            LimitPairSide::PlusOne,
        ),
        expected_outcome: LimitRunOutcome::CompilerError,
        actual_outcome: LimitRunOutcome::CompilerError,
        selected_limit_value: pair.plus_one_limit_value,
        stable_compiler_error_code: Some(DIAGNOSTIC_LIMIT_ERROR_CODE.to_owned()),
        diagnostic_count: plus_one_report.diagnostic_count,
        diagnostics_truncated: plus_one_report.diagnostics_truncated,
        partial_output_record_count: plus_one_report.partial_output_record_count,
        output_record_count: plus_one_report.output_record_count,
        semantic_digest_sha256: None,
        diagnostic_digest_sha256: plus_one_report.diagnostic_digest_sha256,
        live_requested_bytes_after_run: plus_one_report.live_requested_bytes_after_run,
        peak_live_requested_bytes: None,
    };
    Ok((at_bound, plus_one))
}

fn execute_live_byte_limit_pair(
    trusted: &TrustedContract,
    scale: &LimitQualificationScale,
    pair: &LimitPairPlan,
) -> Result<(LimitPairRunObservation, LimitPairRunObservation), LimitQualificationExecutionError> {
    let at_bound_run_id = limit_run_id(scale, pair.binding.dimension_id, "at-bound");
    let mut at_bound_instance =
        ScalableAttributionCompilerInstance::from_trusted_contract_with_id_and_allocation_ceiling(
            trusted,
            format!("{at_bound_run_id}/compiler-instance"),
            scale.workload_id,
            pair.at_bound_limit_value,
        )?;
    let digest = at_bound_instance.run_unmeasured(scale.graph_profile, scale.n)?;
    let at_bound_snapshot = at_bound_instance.allocation_snapshot()?;
    if at_bound_snapshot.peak_live_requested_bytes != pair.exact_dimension_value {
        return Err(
            LimitQualificationExecutionError::LiveByteAtBoundPeakMismatch {
                expected: pair.exact_dimension_value,
                actual: at_bound_snapshot.peak_live_requested_bytes,
            },
        );
    }
    let plan = crate::ScalableStagePlanFactory::from_trusted_contract(trusted)?.plan(
        scale.workload_id,
        scale.graph_profile,
        scale.n,
    )?;
    let at_bound = LimitPairRunObservation {
        run_id: at_bound_run_id,
        input_digest_sha256: limit_pair_input_digest(
            trusted,
            &plan,
            scale,
            pair,
            LimitPairSide::AtBound,
        ),
        expected_outcome: LimitRunOutcome::Success,
        actual_outcome: LimitRunOutcome::Success,
        selected_limit_value: pair.at_bound_limit_value,
        stable_compiler_error_code: None,
        diagnostic_count: 0,
        diagnostics_truncated: false,
        partial_output_record_count: 0,
        output_record_count: plan.counts.semantic_output_record,
        semantic_digest_sha256: Some(digest),
        diagnostic_digest_sha256: empty_diagnostic_digest(),
        live_requested_bytes_after_run: at_bound_snapshot.live_requested_bytes,
        peak_live_requested_bytes: Some(at_bound_snapshot.peak_live_requested_bytes),
    };

    let plus_one_run_id = limit_run_id(scale, pair.binding.dimension_id, "plus-one");
    let mut plus_one_instance =
        ScalableAttributionCompilerInstance::from_trusted_contract_with_id_and_allocation_ceiling(
            trusted,
            format!("{plus_one_run_id}/compiler-instance"),
            scale.workload_id,
            pair.plus_one_limit_value,
        )?;
    match plus_one_instance.run_unmeasured(scale.graph_profile, scale.n) {
        Ok(_) => return Err(LimitQualificationExecutionError::LiveBytePlusOneSucceeded),
        Err(TimingError::StageGeneration(
            crate::StageGenerationError::ControlledAllocationHardCeiling {
                hard_ceiling_bytes, ..
            },
        )) if hard_ceiling_bytes == pair.plus_one_limit_value => {}
        Err(source) => {
            return Err(
                LimitQualificationExecutionError::UnexpectedLiveByteFailure {
                    detail: source.to_string(),
                },
            );
        }
    }
    let plus_one_snapshot = plus_one_instance.allocation_snapshot()?;
    if plus_one_snapshot.live_requested_bytes > pair.plus_one_limit_value
        || plus_one_snapshot.peak_live_requested_bytes > pair.plus_one_limit_value
    {
        return Err(LimitQualificationExecutionError::LiveByteFailureAccounting);
    }
    let plus_one = LimitPairRunObservation {
        run_id: plus_one_run_id,
        input_digest_sha256: limit_pair_input_digest(
            trusted,
            &plan,
            scale,
            pair,
            LimitPairSide::PlusOne,
        ),
        expected_outcome: LimitRunOutcome::CompilerError,
        actual_outcome: LimitRunOutcome::CompilerError,
        selected_limit_value: pair.plus_one_limit_value,
        stable_compiler_error_code: Some(LIMIT_EXCEEDED_ERROR_CODE.to_owned()),
        diagnostic_count: 1,
        diagnostics_truncated: false,
        partial_output_record_count: 0,
        output_record_count: 0,
        semantic_digest_sha256: None,
        diagnostic_digest_sha256: limit_diagnostic_digest(
            pair.binding.dimension_code_u8,
            LIMIT_EXCEEDED_ERROR_CODE,
            pair.plus_one_limit_value,
            pair.exact_dimension_value,
        ),
        live_requested_bytes_after_run: plus_one_snapshot.live_requested_bytes,
        peak_live_requested_bytes: Some(plus_one_snapshot.peak_live_requested_bytes),
    };
    Ok((at_bound, plus_one))
}

fn live_byte_baseline_from_runs(
    runs: &[LimitBaselineProcessRun],
    scale: &LimitQualificationScale,
) -> Result<LiveByteBaseline, LimitQualificationExecutionError> {
    let [left, right] = runs else {
        return Err(LimitQualificationExecutionError::MissingLiveByteBaselineRuns);
    };
    Ok(LiveByteBaseline {
        replicas: [
            baseline_replica(left, scale)?,
            baseline_replica(right, scale)?,
        ],
    })
}

fn baseline_replica(
    run: &LimitBaselineProcessRun,
    scale: &LimitQualificationScale,
) -> Result<LiveByteBaselineReplica, LimitQualificationExecutionError> {
    let peak = run
        .child
        .as_ref()
        .and_then(|child| child.guard_peak_live_requested_bytes)
        .ok_or(LimitQualificationExecutionError::InvalidLiveByteBaselineRun)?;
    Ok(LiveByteBaselineReplica {
        run_id: run.run_id.clone(),
        workload_id: scale.workload_id,
        graph_profile: scale.graph_profile,
        n: scale.n,
        peak_live_requested_bytes: peak,
    })
}

fn validate_live_byte_baseline_runs(
    runs: &[LimitBaselineProcessRun],
    scale: &LimitQualificationScale,
) -> Result<(), LimitQualificationExecutionError> {
    let [left, right] = runs else {
        return Err(LimitQualificationExecutionError::MissingLiveByteBaselineRuns);
    };
    if left.run_id == right.run_id
        || left.compiler_instance_id == right.compiler_instance_id
        || left.status != RunStatus::Valid
        || right.status != RunStatus::Valid
    {
        return Err(LimitQualificationExecutionError::InvalidLiveByteBaselineRun);
    }
    let left_child = left
        .child
        .as_ref()
        .ok_or(LimitQualificationExecutionError::InvalidLiveByteBaselineRun)?;
    let right_child = right
        .child
        .as_ref()
        .ok_or(LimitQualificationExecutionError::InvalidLiveByteBaselineRun)?;
    if left_child.child_pid == right_child.child_pid
        || left.process.child_pid.value != Some(u64::from(left_child.child_pid))
        || right.process.child_pid.value != Some(u64::from(right_child.child_pid))
    {
        return Err(LimitQualificationExecutionError::InvalidLiveByteBaselineRun);
    }
    let left_peak = baseline_replica(left, scale)?.peak_live_requested_bytes;
    let right_peak = baseline_replica(right, scale)?.peak_live_requested_bytes;
    if left_peak != right_peak {
        return Err(
            LimitQualificationExecutionError::LiveByteBaselineDisagreement {
                left: left_peak,
                right: right_peak,
            },
        );
    }
    Ok(())
}

fn validate_live_byte_baseline_child(
    report: &ScalableAttributionChildReport,
    compiler_instance_id: &str,
    scale: &LimitQualificationScale,
) -> Result<(), String> {
    if report.schema != SCALABLE_ATTRIBUTION_CHILD_SCHEMA
        || report.schema_version != SCALABLE_ATTRIBUTION_CHILD_SCHEMA_VERSION
        || report.binary_id != ATTRIBUTION_BINARY_ID
        || !report.allocation_instrumentation_enabled
        || report.compiler_instance_id != compiler_instance_id
        || report.workload_id != scale.workload_id
        || report.workload_revision != WORKLOAD_REVISION_V1
        || report.graph_profile != scale.graph_profile.as_str()
        || report.string_profile != crate::BASE_SCALE_STRING_PROFILE
        || report.generator_version != GENERATOR_VERSION_V1
        || report.n != scale.n
        || report.controlled_allocation_hard_ceiling_bytes
            != scale.guard_thresholds.compiler_controlled_bytes
        || report.outcome != ScalableAttributionOutcome::Success
        || report
            .guard_peak_live_requested_bytes
            .is_none_or(|peak| peak == 0)
        || report.allocation.as_ref().is_none_or(|allocation| {
            Some(allocation.peak_live_requested_bytes) != report.guard_peak_live_requested_bytes
        })
        || report.retained_capacity_bytes.is_none()
        || report.semantic_digest_sha256.is_none()
        || report.controlled_allocation_guard.is_some()
    {
        return Err("limit-baseline-attribution-report".to_owned());
    }
    Ok(())
}

fn limit_diagnostic_digest(
    dimension_code_u8: u8,
    error_code: &'static str,
    selected_limit_value: u64,
    observed_value: u64,
) -> String {
    crate::diagnostic::limit_exceeded_diagnostic_digest(
        error_code,
        dimension_code_u8,
        selected_limit_value,
        observed_value,
    )
}

fn empty_diagnostic_digest() -> String {
    crate::diagnostic::empty_diagnostic_digest()
}

fn limit_run_id(
    scale: &LimitQualificationScale,
    dimension_id: LimitDimensionId,
    side: &str,
) -> String {
    format!(
        "limit/{}/{}/{}/n-{}/{}/{}",
        scale.workload_id.as_str(),
        scale.graph_profile.as_str(),
        scale.scale_role.as_str(),
        scale.n,
        dimension_id.as_str(),
        side
    )
}

fn limit_pair_input_digest(
    trusted: &TrustedContract,
    plan: &ScalableStagePlanSummary,
    scale: &LimitQualificationScale,
    pair: &LimitPairPlan,
    side: LimitPairSide,
) -> String {
    let selected_limit_value = match side {
        LimitPairSide::AtBound => pair.at_bound_limit_value,
        LimitPairSide::PlusOne => pair.plus_one_limit_value,
    };
    crate::input_digest::failure_input_digest_with_private_limits(
        &failure_input_binding(
            trusted,
            scale,
            plan,
            &format!(
                "limit/{}/{}",
                pair.binding.dimension_id.as_str(),
                side.as_str()
            ),
            &pair.binding.input_variant_id,
            limit_value_basis(pair.binding.pair_mode),
            &pair.basis_run_ids,
        ),
        Some((pair.binding.dimension_id, selected_limit_value)),
        &[
            ("exact-dimension-value", pair.exact_dimension_value),
            ("selected-limit-value", selected_limit_value),
        ],
    )
}

fn failure_input_binding<'a>(
    trusted: &'a TrustedContract,
    scale: &'a LimitQualificationScale,
    plan: &'a ScalableStagePlanSummary,
    case_id: &'a str,
    input_variant_id: &'a str,
    value_basis: &'a str,
    basis_run_ids: &'a [String],
) -> FailureInputDigestBinding<'a> {
    FailureInputDigestBinding {
        workload_manifest_sha256: &trusted.descriptor.workload_manifest.sha256,
        workload_id: scale.workload_id,
        workload_revision: WORKLOAD_REVISION_V1,
        graph_profile: scale.graph_profile,
        string_profile: BASE_SCALE_STRING_PROFILE,
        generator_version: GENERATOR_VERSION_V1,
        n: scale.n,
        b: scale.b,
        scale_role: scale.scale_role.as_str(),
        case_id,
        input_variant_id,
        counts: &plan.counts,
        value_basis,
        basis_run_ids,
    }
}

const fn limit_value_basis(pair_mode: LimitPairMode) -> &'static str {
    match pair_mode {
        LimitPairMode::SuccessAtBound => "canonical-level-exact-value",
        LimitPairMode::DiagnosticCapOnSemanticFailure => "diagnostic-input-count",
        LimitPairMode::BaselineLiveBytePrescanV1 => "baseline-live-byte-prescan-v1",
    }
}

fn parse_graph_profile(value: &str) -> Result<GraphProfileId, LimitQualificationExecutionError> {
    GraphProfileId::ALL
        .into_iter()
        .find(|profile| profile.as_str() == value)
        .ok_or_else(|| LimitQualificationExecutionError::InvalidGraphProfile(value.to_owned()))
}

#[derive(Debug, Error)]
pub enum LimitQualificationExecutionError {
    #[error("正式阶梯缺少规模选择：{workload_id:?}/{graph_profile:?}")]
    MissingScaleSelection {
        workload_id: ScalableWorkloadId,
        graph_profile: GraphProfileId,
    },
    #[error("正式阶梯缺少 {scale_role:?} 规模：{workload_id:?}/{graph_profile:?}")]
    MissingSelectedScale {
        workload_id: ScalableWorkloadId,
        graph_profile: GraphProfileId,
        scale_role: CleanupScaleRole,
    },
    #[error("选中规模不是完整正式级别：{workload_id:?}/{graph_profile:?}/{scale_role:?}/N={n}")]
    SelectedScaleNotComplete {
        workload_id: ScalableWorkloadId,
        graph_profile: GraphProfileId,
        scale_role: CleanupScaleRole,
        n: u32,
    },
    #[error("重复的限制规模身份：{workload_id:?}/{graph_profile:?}/{scale_role:?}")]
    DuplicateScaleIdentity {
        workload_id: ScalableWorkloadId,
        graph_profile: GraphProfileId,
        scale_role: CleanupScaleRole,
    },
    #[error("限制规模集合不完整：actual={actual}")]
    IncompleteScaleSet { actual: usize },
    #[error("限制配对集合不完整：actual={actual}")]
    IncompleteLimitPairSet { actual: usize },
    #[error("存续字节基线运行集合不完整：actual={actual}")]
    IncompleteLiveByteBaselineSet { actual: usize },
    #[error("清理实验集合不完整：actual={actual}")]
    IncompleteCleanupSet { actual: usize },
    #[error("重复所有者语义失败资格集合不完整：actual={actual}")]
    IncompleteDuplicateOwnerQualificationSet { actual: usize },
    #[error("重复所有者语义失败资格观察不闭合")]
    InvalidDuplicateOwnerQualification,
    #[error("重复所有者语义失败资格只能绑定 LF-COMP-CORRIDOR-v1")]
    DuplicateOwnerWrongWorkload,
    #[error("重复所有者语义失败子进程报告的 pid 与父进程观察不一致")]
    SemanticFailureChildPidMismatch,
    #[error("未知模块图配置档：{0}")]
    InvalidGraphProfile(String),
    #[error("存续字节基线只能绑定 LF-COMP-ID-v1，实际为 {0:?}")]
    LiveByteBaselineWrongWorkload(ScalableWorkloadId),
    #[error("存续字节基线缺少恰好两个进程运行")]
    MissingLiveByteBaselineRuns,
    #[error("存续字节基线进程运行无效")]
    InvalidLiveByteBaselineRun,
    #[error("两个 attribution 基线进程峰值不一致：left={left}, right={right}")]
    LiveByteBaselineDisagreement { left: u64, right: u64 },
    #[error("限制配对观察不闭合")]
    InvalidLimitPairObservation,
    #[error("普通限制 plus-one 错误地完成了真实编译管线")]
    OrdinaryPlusOneSucceeded,
    #[error("普通限制真实失败观察与冻结配对不一致")]
    OrdinaryLimitFailureMismatch,
    #[error("限制侧子进程要求二进制 {expected}，实际为 {actual}")]
    WrongLimitSideBinary {
        expected: &'static str,
        actual: String,
    },
    #[error("限制侧子进程的规模与配对不一致")]
    LimitSideScaleMismatch,
    #[error("限制侧子进程的编译器实例身份不一致")]
    LimitSideCompilerInstanceMismatch,
    #[error("限制侧子进程报告的 pid 与父进程观察不一致")]
    LimitSideChildPidMismatch,
    #[error("清理子进程的规模与失败用例不一致")]
    CleanupChildScaleMismatch,
    #[error("清理子进程运行无效")]
    InvalidCleanupChild,
    #[error("清理子进程报告的 pid 与父进程观察不一致")]
    CleanupChildPidMismatch,
    #[error("无法序列化限制侧子进程参数：{0}")]
    SerializeChildArgument(#[from] serde_json::Error),
    #[error("缺少清理实验规模：{case_id:?}/{scale_role:?}")]
    MissingCleanupScale {
        case_id: CleanupFailureCase,
        scale_role: CleanupScaleRole,
    },
    #[error("诊断计数溢出或无法表示")]
    DiagnosticCountOverflow,
    #[error("诊断候选不足：required={required}, available={available}")]
    InsufficientDiagnosticCandidates { required: u64, available: u64 },
    #[error("无法为 {count} 条诊断保留容量：{source}")]
    DiagnosticReserve {
        count: u64,
        source: std::collections::TryReserveError,
    },
    #[error("存续字节 at-bound 峰值不匹配：expected={expected}, actual={actual}")]
    LiveByteAtBoundPeakMismatch { expected: u64, actual: u64 },
    #[error("存续字节 plus-one 错误地成功")]
    LiveBytePlusOneSucceeded,
    #[error("存续字节 plus-one 返回了非受控分配上限错误：{detail}")]
    UnexpectedLiveByteFailure { detail: String },
    #[error("存续字节 plus-one 失败后的分配记账越界")]
    LiveByteFailureAccounting,
    #[error("诊断限制 at-bound/plus-one 没有形成精确截断边界")]
    InvalidDiagnosticLimitPair,
    #[error(transparent)]
    Limit(#[from] LimitQualificationError),
    #[error(transparent)]
    LimitExceeded(#[from] crate::LimitExceeded),
    #[error(transparent)]
    Timing(#[from] TimingError),
    #[error(transparent)]
    Corridor(#[from] crate::CorridorError),
    #[error(transparent)]
    Cleanup(#[from] CleanupError),
    #[error(transparent)]
    FormalRunner(#[from] FormalLadderRunnerError),
    #[error(transparent)]
    Pilot(#[from] crate::PilotError),
    #[error(transparent)]
    Contract(#[from] crate::ContractError),
    #[error(transparent)]
    ScalePlan(#[from] crate::ScalePlanError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        FormalLadderAnalysis, FormalLadderExecutionDisposition, FormalLadderLevelExecution,
        FormalScaleSelection, FormalScaleSelectionDisposition, GuardPredictionBasis,
        GuardPreflightReport, SystemMemoryObservation,
    };

    fn scale(workload_id: ScalableWorkloadId) -> LimitQualificationScale {
        LimitQualificationScale {
            workload_id,
            graph_profile: GraphProfileId::WideStar,
            b: 1,
            scale_role: CleanupScaleRole::Calibration,
            n: 1,
            guard_thresholds: GuardThresholds::from_physical_memory_bytes(4 * 1024 * 1024 * 1024)
                .expect("guard thresholds"),
        }
    }

    fn formal_ladder(
        workload_id: ScalableWorkloadId,
        graph_profile: GraphProfileId,
    ) -> FormalLadderExecution {
        let thresholds = GuardThresholds::from_physical_memory_bytes(4 * 1024 * 1024 * 1024)
            .expect("guard thresholds");
        let level = |n| FormalLadderLevelExecution {
            n,
            primary_record_count: u64::from(n),
            canonical_lir_record_count: u64::from(n),
            guard_preflight: GuardPreflightReport {
                workload_id: workload_id.as_str().to_owned(),
                graph_profile: graph_profile.as_str().to_owned(),
                n,
                primary_record_count: u64::from(n),
                maximum_typed_ordinal: u64::from(n),
                logical_bytes_lower_bound: 1,
                compiler_controlled_prediction_basis:
                    GuardPredictionBasis::ManifestSingleBufferLowerBoundV1,
                private_bytes_prediction_basis: GuardPredictionBasis::FirstLevelMonitorOnly,
                wall_time_prediction_basis: GuardPredictionBasis::FirstLevelMonitorOnly,
                predicted_compiler_controlled_bytes: 1,
                predicted_private_bytes: None,
                predicted_wall_time_ns: None,
                memory_observation: SystemMemoryObservation {
                    physical_memory_bytes: 4 * 1024 * 1024 * 1024,
                    available_physical_memory_bytes: 3 * 1024 * 1024 * 1024,
                },
                thresholds,
                triggers: Vec::new(),
                allows_child_start: true,
            },
            attribution_preflight: None,
            timing_guard_run: None,
            oracle: None,
            formal_runs: Vec::new(),
            completed_guard_observation: None,
            complete: true,
        };
        FormalLadderExecution {
            schema: "test-formal-ladder".to_owned(),
            schema_version: 1,
            candidate_id: crate::BASELINE_CANDIDATE_ID.to_owned(),
            workload_id,
            workload_revision: WORKLOAD_REVISION_V1,
            graph_profile: graph_profile.as_str().to_owned(),
            string_profile: crate::BASE_SCALE_STRING_PROFILE.to_owned(),
            generator_version: GENERATOR_VERSION_V1,
            b: 1,
            disposition: FormalLadderExecutionDisposition::Complete,
            levels: vec![level(1), level(2)],
            analysis: Some(FormalLadderAnalysis {
                round_summaries: Vec::new(),
                batch_summaries: Vec::new(),
                adjacent_level_ratios: Vec::new(),
                knees: Vec::new(),
                scale_selection: FormalScaleSelection {
                    selection_rule: "test".to_owned(),
                    disposition: FormalScaleSelectionDisposition::NoObservedKnee,
                    calibration_n: Some(1),
                    stress_n: Some(2),
                    first_confirmed_knee_n: None,
                },
            }),
            terminal_guard_preflight: None,
        }
    }

    #[test]
    fn formal_ladders_derive_exactly_eighteen_closed_scale_strata() {
        let ladders = ScalableWorkloadId::ALL
            .into_iter()
            .flat_map(|workload_id| {
                GraphProfileId::ALL
                    .into_iter()
                    .map(move |graph_profile| formal_ladder(workload_id, graph_profile))
            })
            .collect::<Vec<_>>();
        let scales = derive_limit_qualification_scales(&ladders).expect("closed scales");
        assert_eq!(scales.len(), EXPECTED_LIMIT_SCALE_COUNT);
        assert_eq!(
            scales
                .iter()
                .filter(|scale| scale.scale_role == CleanupScaleRole::Calibration)
                .count(),
            9
        );
        assert_eq!(
            scales
                .iter()
                .filter(|scale| scale.scale_role == CleanupScaleRole::Stress)
                .count(),
            9
        );
        let incomplete = derive_limit_qualification_scales(&ladders[..8]);
        assert!(matches!(
            incomplete,
            Err(LimitQualificationExecutionError::IncompleteScaleSet { .. })
        ));
    }

    #[test]
    fn duplicate_owner_qualification_covers_every_corridor_selected_scale() {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        let mut qualifications = Vec::new();
        for graph_profile in GraphProfileId::ALL {
            for scale_role in [CleanupScaleRole::Calibration, CleanupScaleRole::Stress] {
                let mut scale = scale(ScalableWorkloadId::Corridor);
                scale.graph_profile = graph_profile;
                scale.scale_role = scale_role;
                scale.n = 2;
                qualifications.push(
                    execute_duplicate_owner_observation(&trusted, &scale)
                        .expect("duplicate-owner qualification"),
                );
            }
        }
        assert_eq!(
            qualifications.len(),
            EXPECTED_DUPLICATE_OWNER_QUALIFICATION_COUNT
        );
        assert!(qualifications.iter().all(|qualification| {
            qualification.stable_compiler_error_code.as_deref() == Some(DUPLICATE_OWNER_ERROR_CODE)
                && qualification.diagnostic_count == 2
                && qualification.live_requested_bytes_after_run == 0
        }));
    }

    #[test]
    fn every_dimension_executes_a_closed_at_bound_plus_one_pair_at_n_one() {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        let planner = LimitQualificationPlanner::from_trusted_contract(&trusted)
            .expect("qualification planner");
        for binding in planner.bindings() {
            let scale = scale(binding.workload_id);
            let baseline =
                if binding.dimension_id == LimitDimensionId::CompilerControlledLiveByteCount {
                    let mut peaks = Vec::new();
                    for replica in 0..2 {
                        let mut instance =
                            ScalableAttributionCompilerInstance::from_trusted_contract_with_id(
                                &trusted,
                                format!("unit-live-baseline-{replica}"),
                                binding.workload_id,
                            )
                            .expect("baseline instance");
                        instance
                            .run_unmeasured(scale.graph_profile, scale.n)
                            .expect("baseline run");
                        peaks.push(
                            instance
                                .allocation_snapshot()
                                .expect("allocation snapshot")
                                .peak_live_requested_bytes,
                        );
                    }
                    assert_eq!(peaks[0], peaks[1]);
                    Some(LiveByteBaseline {
                        replicas: [
                            LiveByteBaselineReplica {
                                run_id: "unit-live-baseline-0".to_owned(),
                                workload_id: binding.workload_id,
                                graph_profile: scale.graph_profile,
                                n: scale.n,
                                peak_live_requested_bytes: peaks[0],
                            },
                            LiveByteBaselineReplica {
                                run_id: "unit-live-baseline-1".to_owned(),
                                workload_id: binding.workload_id,
                                graph_profile: scale.graph_profile,
                                n: scale.n,
                                peak_live_requested_bytes: peaks[1],
                            },
                        ],
                    })
                } else {
                    None
                };
            let pair = planner
                .plan_pair(binding.dimension_id, scale.graph_profile, scale.n, baseline)
                .expect("pair plan");
            let (at_bound, plus_one) =
                execute_limit_pair_payloads(&trusted, &scale, &pair).expect("pair execution");
            assert_eq!(at_bound.expected_outcome, at_bound.actual_outcome);
            assert_eq!(plus_one.expected_outcome, plus_one.actual_outcome);
            assert_eq!(at_bound.partial_output_record_count, 0);
            assert_eq!(plus_one.partial_output_record_count, 0);
        }
    }

    #[test]
    fn diagnostic_pair_retains_n_then_n_minus_one_in_canonical_order() {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        let planner = LimitQualificationPlanner::from_trusted_contract(&trusted)
            .expect("qualification planner");
        let mut scale = scale(ScalableWorkloadId::Corridor);
        scale.n = 4;
        let pair = planner
            .plan_pair(
                LimitDimensionId::DiagnosticCount,
                scale.graph_profile,
                scale.n,
                None,
            )
            .expect("diagnostic pair");
        let (at_bound, plus_one) =
            execute_limit_pair_payloads(&trusted, &scale, &pair).expect("pair execution");
        assert_eq!(at_bound.diagnostic_count, 4);
        assert!(!at_bound.diagnostics_truncated);
        assert_eq!(
            at_bound.stable_compiler_error_code.as_deref(),
            Some(UNKNOWN_REFERENCE_ERROR_CODE)
        );
        assert_eq!(plus_one.diagnostic_count, 3);
        assert!(plus_one.diagnostics_truncated);
        assert_eq!(
            plus_one.stable_compiler_error_code.as_deref(),
            Some(DIAGNOSTIC_LIMIT_ERROR_CODE)
        );
    }
}
