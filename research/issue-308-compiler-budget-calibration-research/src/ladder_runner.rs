//! 正式规模阶梯的父进程编排。
//!
//! runner 只负责编排已经分离的 attribution、oracle 与 timing 子进程，保存原始进程
//! 事实，并把完整有效级别交给 `ladder` 纯函数分析。Evidence v1 的来源绑定、写出与
//! 独立验证位于 evidence / evidence_assembly。

use crate::environment::installed_formal_environment;
use crate::pilot::{
    MonitoredChildExecution, monitor_invalidation_reasons, run_monitored_scalable_oracle,
    run_monitored_scalable_role_child,
};
use crate::{
    ATTRIBUTION_BINARY_ID, BASE_SCALE_STRING_PROFILE, BASELINE_CANDIDATE_ID,
    BaseScalePilotCheckpoint, BaseScaleSelection, ChildProcessMonitorReport,
    ExternalStateObservation, FormalLadderAnalysis, FormalLadderCompletedLevel, FormalLadderError,
    FormalLadderRoundRun, FormalScaleSelectionDisposition, GENERATOR_VERSION_V1, GraphProfileId,
    GuardCompletedLevelObservation, GuardPreflightReport, InvalidationReason, ORACLE_BINARY_ID,
    PilotError, ProcessObservation, ProcessProtocolError, RunStatus,
    SCALABLE_ATTRIBUTION_CHILD_SCHEMA, SCALABLE_ATTRIBUTION_CHILD_SCHEMA_VERSION,
    SCALABLE_LADDER_CHILD_SCHEMA, SCALABLE_LADDER_CHILD_SCHEMA_VERSION,
    SCALABLE_ORACLE_CHILD_SCHEMA, SCALABLE_ORACLE_CHILD_SCHEMA_VERSION,
    STABLE_CAPACITY_SAMPLE_COUNT, STABLE_CAPACITY_WARMUP_COUNT, ScalableAttributionChildReport,
    ScalableAttributionOutcome, ScalableGuardPlanner, ScalableLadderBinaryMode,
    ScalableLadderChildReport, ScalableLadderOutcome, ScalableLadderSample,
    ScalableOracleChildReport, ScalableOracleOutcome, ScalableStagePlanFactory, ScalableWorkloadId,
    SystemMemoryMonitor, TIMING_BINARY_ID, TrustedContract, WORKLOAD_REVISION_V1,
    analyze_formal_ladder,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const FORMAL_LADDER_EXECUTION_SCHEMA: &str =
    "laneflow.compiler-calibration-formal-ladder-execution";
pub const FORMAL_LADDER_EXECUTION_SCHEMA_VERSION: u32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FormalLadderExecutionDisposition {
    Complete,
    GuardedBeforeMinimumLevels,
    GuardedAfterMinimumLevels,
    InvalidRun,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormalAttributionPreflightRun {
    pub run_id: String,
    pub compiler_instance_id: String,
    pub status: RunStatus,
    pub invalidation_reasons: Vec<InvalidationReason>,
    pub process: ProcessObservation,
    pub child: Option<ScalableAttributionChildReport>,
    pub monitor: ChildProcessMonitorReport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_state: Option<ExternalStateObservation>,
    pub kill_error: Option<String>,
    pub monitor_error: Option<String>,
    pub stderr: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormalTimingGuardRun {
    pub run_id: String,
    pub compiler_instance_id: String,
    pub status: RunStatus,
    pub invalidation_reasons: Vec<InvalidationReason>,
    pub process: ProcessObservation,
    pub child: Option<ScalableLadderChildReport>,
    pub monitor: ChildProcessMonitorReport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_state: Option<ExternalStateObservation>,
    pub kill_error: Option<String>,
    pub monitor_error: Option<String>,
    pub stderr: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormalOracleRun {
    pub run_id: String,
    pub status: RunStatus,
    pub invalidation_reasons: Vec<InvalidationReason>,
    pub process: ProcessObservation,
    pub child: Option<ScalableOracleChildReport>,
    pub monitor: ChildProcessMonitorReport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_state: Option<ExternalStateObservation>,
    pub kill_error: Option<String>,
    pub monitor_error: Option<String>,
    pub stderr: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormalLadderProcessRun {
    pub run_id: String,
    pub attempt_id: String,
    pub retry_ordinal: u32,
    pub batch: u32,
    pub round: u32,
    pub execution_position: u32,
    pub binary_mode: ScalableLadderBinaryMode,
    pub compiler_instance_id: String,
    pub status: RunStatus,
    pub invalidation_reasons: Vec<InvalidationReason>,
    pub process: ProcessObservation,
    pub child: Option<ScalableLadderChildReport>,
    pub monitor: ChildProcessMonitorReport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_state: Option<ExternalStateObservation>,
    pub kill_error: Option<String>,
    pub monitor_error: Option<String>,
    pub stderr: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormalGuardPreflightRun {
    pub run_id: String,
    pub process: ProcessObservation,
    pub guard_preflight: GuardPreflightReport,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormalLadderLevelExecution {
    pub n: u32,
    pub primary_record_count: u64,
    pub canonical_lir_record_count: u64,
    pub guard_preflight: GuardPreflightReport,
    pub attribution_preflight: Option<FormalAttributionPreflightRun>,
    pub timing_guard_run: Option<FormalTimingGuardRun>,
    pub oracle: Option<FormalOracleRun>,
    pub formal_runs: Vec<FormalLadderProcessRun>,
    pub completed_guard_observation: Option<GuardCompletedLevelObservation>,
    pub complete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormalLadderExecution {
    pub schema: String,
    pub schema_version: u32,
    pub candidate_id: String,
    pub workload_id: ScalableWorkloadId,
    pub workload_revision: u32,
    pub graph_profile: String,
    pub string_profile: String,
    pub generator_version: u32,
    pub b: u32,
    pub disposition: FormalLadderExecutionDisposition,
    pub levels: Vec<FormalLadderLevelExecution>,
    pub analysis: Option<FormalLadderAnalysis>,
    pub terminal_guard_preflight: Option<FormalGuardPreflightRun>,
}

enum LevelPreparation {
    Ready(FormalLadderLevelExecution),
    Guarded(GuardPreflightReport),
    Invalid(FormalLadderLevelExecution),
}

pub fn run_formal_ladders(
    trusted: &TrustedContract,
    timing_executable: &Path,
    attribution_executable: &Path,
    oracle_executable: &Path,
    base_scale_checkpoint: &BaseScalePilotCheckpoint,
    mut persist: impl FnMut(
        &[FormalLadderExecution],
        Option<&FormalLadderExecution>,
    ) -> Result<(), FormalLadderRunnerError>,
) -> Result<Vec<FormalLadderExecution>, FormalLadderRunnerError> {
    let mut completed = Vec::new();
    persist(&completed, None)?;
    for selection in &base_scale_checkpoint.selections {
        let Some(b) = selection.b.value else {
            continue;
        };
        let graph_profile = parse_graph_profile(&selection.graph_profile)?;
        let execution = run_one_formal_ladder(
            trusted,
            timing_executable,
            attribution_executable,
            oracle_executable,
            selection,
            graph_profile,
            b,
            |active| persist(&completed, Some(active)),
        )?;
        persist(&completed, Some(&execution))?;
        completed.push(execution);
        persist(&completed, None)?;
    }
    Ok(completed)
}

#[allow(clippy::too_many_arguments)]
fn run_one_formal_ladder(
    trusted: &TrustedContract,
    timing_executable: &Path,
    attribution_executable: &Path,
    oracle_executable: &Path,
    selection: &BaseScaleSelection,
    graph_profile: GraphProfileId,
    b: u32,
    mut persist: impl FnMut(&FormalLadderExecution) -> Result<(), FormalLadderRunnerError>,
) -> Result<FormalLadderExecution, FormalLadderRunnerError> {
    validate_selection(selection, b)?;
    let guard_planner = ScalableGuardPlanner::from_trusted_contract(trusted)?;
    let plans = ScalableStagePlanFactory::from_trusted_contract(trusted)?;
    let mut system_memory = SystemMemoryMonitor::new()?;
    let mut execution = FormalLadderExecution {
        schema: FORMAL_LADDER_EXECUTION_SCHEMA.to_owned(),
        schema_version: FORMAL_LADDER_EXECUTION_SCHEMA_VERSION,
        candidate_id: BASELINE_CANDIDATE_ID.to_owned(),
        workload_id: selection.workload_id,
        workload_revision: WORKLOAD_REVISION_V1,
        graph_profile: selection.graph_profile.clone(),
        string_profile: BASE_SCALE_STRING_PROFILE.to_owned(),
        generator_version: GENERATOR_VERSION_V1,
        b,
        disposition: FormalLadderExecutionDisposition::InvalidRun,
        levels: Vec::new(),
        analysis: None,
        terminal_guard_preflight: None,
    };
    persist(&execution)?;

    let mut previous = previous_pilot_observation(selection, b);
    let mut next_n = b;
    while execution.levels.len() < crate::FORMAL_LADDER_MINIMUM_LEVEL_COUNT {
        let level = match prepare_level(
            timing_executable,
            attribution_executable,
            oracle_executable,
            &guard_planner,
            &plans,
            &mut system_memory,
            &execution,
            graph_profile,
            next_n,
            previous,
        )? {
            LevelPreparation::Ready(level) => level,
            LevelPreparation::Guarded(guard) => {
                execution.terminal_guard_preflight = Some(terminal_guard_run(&execution, guard));
                execution.disposition =
                    FormalLadderExecutionDisposition::GuardedBeforeMinimumLevels;
                persist(&execution)?;
                return Ok(execution);
            }
            LevelPreparation::Invalid(level) => {
                execution.levels.push(level);
                execution.disposition = FormalLadderExecutionDisposition::InvalidRun;
                persist(&execution)?;
                return Ok(execution);
            }
        };
        previous = level.completed_guard_observation;
        execution.levels.push(level);
        persist(&execution)?;
        next_n = next_n
            .checked_mul(2)
            .ok_or(FormalLadderRunnerError::ScaleOverflow)?;
    }

    if !run_balanced_rounds(
        timing_executable,
        attribution_executable,
        &mut execution,
        0,
        &mut persist,
    )? {
        execution.disposition = FormalLadderExecutionDisposition::InvalidRun;
        persist(&execution)?;
        return Ok(execution);
    }
    execution.analysis = analyze_execution(&execution)?;
    persist(&execution)?;

    loop {
        if execution.analysis.as_ref().is_some_and(|analysis| {
            analysis.scale_selection.disposition == FormalScaleSelectionDisposition::ConfirmedKnee
        }) {
            execution.disposition = FormalLadderExecutionDisposition::Complete;
            persist(&execution)?;
            return Ok(execution);
        }

        let previous = execution
            .levels
            .last()
            .and_then(|level| level.completed_guard_observation);
        let level = match prepare_level(
            timing_executable,
            attribution_executable,
            oracle_executable,
            &guard_planner,
            &plans,
            &mut system_memory,
            &execution,
            graph_profile,
            next_n,
            previous,
        )? {
            LevelPreparation::Ready(level) => level,
            LevelPreparation::Guarded(guard) => {
                execution.terminal_guard_preflight = Some(terminal_guard_run(&execution, guard));
                execution.disposition = FormalLadderExecutionDisposition::GuardedAfterMinimumLevels;
                persist(&execution)?;
                return Ok(execution);
            }
            LevelPreparation::Invalid(level) => {
                execution.levels.push(level);
                execution.disposition = FormalLadderExecutionDisposition::InvalidRun;
                persist(&execution)?;
                return Ok(execution);
            }
        };
        execution.levels.push(level);
        let experiment_ordinal = u32::try_from(execution.levels.len())
            .map_err(|_| FormalLadderRunnerError::ExecutionPositionOverflow)?;
        if !run_balanced_rounds(
            timing_executable,
            attribution_executable,
            &mut execution,
            experiment_ordinal,
            &mut persist,
        )? {
            execution.disposition = FormalLadderExecutionDisposition::InvalidRun;
            persist(&execution)?;
            return Ok(execution);
        }
        execution.analysis = analyze_execution(&execution)?;
        persist(&execution)?;
        next_n = next_n
            .checked_mul(2)
            .ok_or(FormalLadderRunnerError::ScaleOverflow)?;
    }
}

#[allow(clippy::too_many_arguments)]
fn terminal_guard_run(
    execution: &FormalLadderExecution,
    guard_preflight: GuardPreflightReport,
) -> FormalGuardPreflightRun {
    FormalGuardPreflightRun {
        run_id: format!(
            "formal/{}/{}/n-{}/terminal-guard-preflight",
            execution.workload_id.as_str(),
            execution.graph_profile,
            guard_preflight.n
        ),
        process: ProcessObservation::guarded_before_start(std::process::id(), TIMING_BINARY_ID),
        guard_preflight,
    }
}

#[allow(clippy::too_many_arguments)]
fn prepare_level(
    timing_executable: &Path,
    attribution_executable: &Path,
    oracle_executable: &Path,
    guard_planner: &ScalableGuardPlanner,
    plans: &ScalableStagePlanFactory,
    system_memory: &mut SystemMemoryMonitor,
    execution: &FormalLadderExecution,
    graph_profile: GraphProfileId,
    n: u32,
    previous: Option<GuardCompletedLevelObservation>,
) -> Result<LevelPreparation, FormalLadderRunnerError> {
    let guard_preflight = guard_planner.evaluate(
        execution.workload_id,
        graph_profile,
        n,
        system_memory.observe()?,
        previous,
    )?;
    if !guard_preflight.allows_child_start {
        return Ok(LevelPreparation::Guarded(guard_preflight));
    }
    let plan = plans.plan(execution.workload_id, graph_profile, n)?;
    let preflight_run_id = format!(
        "formal/{}/{}/n-{n}/attribution-preflight",
        execution.workload_id.as_str(),
        execution.graph_profile
    );
    let preflight_instance_id = format!("{preflight_run_id}/compiler-instance");
    let preflight_execution = run_monitored_scalable_role_child(
        attribution_executable,
        0,
        "run-preflight",
        &preflight_instance_id,
        execution.workload_id,
        graph_profile,
        n,
        guard_preflight.thresholds,
    )?;
    let preflight = decode_child_execution(
        preflight_execution,
        ATTRIBUTION_BINARY_ID,
        |report: &ScalableAttributionChildReport| {
            validate_attribution_preflight(
                report,
                &preflight_instance_id,
                execution.workload_id,
                graph_profile,
                n,
                guard_preflight.thresholds.compiler_controlled_bytes,
            )
        },
        |report| report.outcome == ScalableAttributionOutcome::GuardedInChild,
    )?;
    let attribution_preflight = FormalAttributionPreflightRun {
        run_id: preflight_run_id,
        compiler_instance_id: preflight_instance_id,
        status: preflight.status,
        invalidation_reasons: preflight.invalidation_reasons,
        process: preflight.process,
        child: preflight.child,
        monitor: preflight.monitor,
        external_state: preflight.external_state,
        kill_error: preflight.kill_error,
        monitor_error: preflight.monitor_error,
        stderr: preflight.stderr,
    };
    if attribution_preflight.status != RunStatus::Valid {
        return Ok(LevelPreparation::Invalid(FormalLadderLevelExecution {
            n,
            primary_record_count: plan.primary_record_count,
            canonical_lir_record_count: plan.stages.canonical_lir.record_count,
            guard_preflight,
            attribution_preflight: Some(attribution_preflight),
            timing_guard_run: None,
            oracle: None,
            formal_runs: Vec::new(),
            completed_guard_observation: None,
            complete: false,
        }));
    }
    let expected_digest = attribution_preflight
        .child
        .as_ref()
        .and_then(|child| child.semantic_digest_sha256.as_deref())
        .ok_or(FormalLadderRunnerError::InvalidPreflight { n })?;

    let timing_run_id = format!(
        "formal/{}/{}/n-{n}/timing-guard-observation",
        execution.workload_id.as_str(),
        execution.graph_profile
    );
    let timing_instance_id = format!("{timing_run_id}/compiler-instance");
    let timing_execution = run_monitored_scalable_role_child(
        timing_executable,
        1,
        "run-ladder",
        &timing_instance_id,
        execution.workload_id,
        graph_profile,
        n,
        guard_preflight.thresholds,
    )?;
    let timing = decode_child_execution(
        timing_execution,
        TIMING_BINARY_ID,
        |report: &ScalableLadderChildReport| {
            validate_ladder_report(
                report,
                &timing_instance_id,
                execution.workload_id,
                &execution.graph_profile,
                n,
                guard_preflight.thresholds.compiler_controlled_bytes,
                ScalableLadderBinaryMode::Timing,
                expected_digest,
            )
        },
        |report| report.outcome == ScalableLadderOutcome::GuardedInChild,
    )?;
    let timing_guard_run = FormalTimingGuardRun {
        run_id: timing_run_id,
        compiler_instance_id: timing_instance_id,
        status: timing.status,
        invalidation_reasons: timing.invalidation_reasons,
        process: timing.process,
        child: timing.child,
        monitor: timing.monitor,
        external_state: timing.external_state,
        kill_error: timing.kill_error,
        monitor_error: timing.monitor_error,
        stderr: timing.stderr,
    };
    if timing_guard_run.status != RunStatus::Valid {
        return Ok(LevelPreparation::Invalid(FormalLadderLevelExecution {
            n,
            primary_record_count: plan.primary_record_count,
            canonical_lir_record_count: plan.stages.canonical_lir.record_count,
            guard_preflight,
            attribution_preflight: Some(attribution_preflight),
            timing_guard_run: Some(timing_guard_run),
            oracle: None,
            formal_runs: Vec::new(),
            completed_guard_observation: None,
            complete: false,
        }));
    }

    let oracle_run_id = format!(
        "formal/{}/{}/n-{n}/oracle",
        execution.workload_id.as_str(),
        execution.graph_profile
    );
    let oracle_execution = run_monitored_scalable_oracle(
        oracle_executable,
        0,
        &oracle_run_id,
        execution.workload_id,
        graph_profile,
        n,
        guard_preflight.thresholds,
    )?;
    let oracle = decode_child_execution(
        oracle_execution,
        ORACLE_BINARY_ID,
        |report: &ScalableOracleChildReport| {
            validate_oracle(
                report,
                &oracle_run_id,
                execution.workload_id,
                graph_profile,
                n,
                guard_preflight.thresholds.compiler_controlled_bytes,
                plan.primary_record_count,
                expected_digest,
            )
        },
        |report| report.outcome == ScalableOracleOutcome::GuardedInChild,
    )?;
    let oracle = FormalOracleRun {
        run_id: oracle_run_id,
        status: oracle.status,
        invalidation_reasons: oracle.invalidation_reasons,
        process: oracle.process,
        child: oracle.child,
        monitor: oracle.monitor,
        external_state: oracle.external_state,
        kill_error: oracle.kill_error,
        monitor_error: oracle.monitor_error,
        stderr: oracle.stderr,
    };
    if oracle.status != RunStatus::Valid {
        return Ok(LevelPreparation::Invalid(FormalLadderLevelExecution {
            n,
            primary_record_count: plan.primary_record_count,
            canonical_lir_record_count: plan.stages.canonical_lir.record_count,
            guard_preflight,
            attribution_preflight: Some(attribution_preflight),
            timing_guard_run: Some(timing_guard_run),
            oracle: Some(oracle),
            formal_runs: Vec::new(),
            completed_guard_observation: None,
            complete: false,
        }));
    }

    let preflight_child = attribution_preflight
        .child
        .as_ref()
        .ok_or(FormalLadderRunnerError::InvalidPreflight { n })?;
    let timing_wall_time_ns = timing_guard_run
        .child
        .as_ref()
        .ok_or(FormalLadderRunnerError::InvalidPreflight { n })?
        .cold_instance
        .iter()
        .chain(
            timing_guard_run
                .child
                .as_ref()
                .expect("validated timing guard child")
                .stable_capacity_reuse
                .iter(),
        )
        .filter_map(|sample| sample.wall_time_ns)
        .max()
        .ok_or(FormalLadderRunnerError::InvalidPreflight { n })?;
    let completed_guard_observation = GuardCompletedLevelObservation {
        n,
        primary_record_count: plan.primary_record_count,
        peak_live_requested_bytes: maximum_observed_controlled_peak(
            [
                preflight_child.guard_peak_live_requested_bytes,
                oracle
                    .child
                    .as_ref()
                    .and_then(|child| child.guard_peak_live_requested_bytes),
            ],
            n,
        )?,
        private_bytes: maximum_observed_private_bytes([
            &attribution_preflight.monitor,
            &timing_guard_run.monitor,
            &oracle.monitor,
        ])?,
        wall_time_ns: timing_wall_time_ns,
    };
    Ok(LevelPreparation::Ready(FormalLadderLevelExecution {
        n,
        primary_record_count: plan.primary_record_count,
        canonical_lir_record_count: plan.stages.canonical_lir.record_count,
        guard_preflight,
        attribution_preflight: Some(attribution_preflight),
        timing_guard_run: Some(timing_guard_run),
        oracle: Some(oracle),
        formal_runs: Vec::new(),
        completed_guard_observation: Some(completed_guard_observation),
        complete: false,
    }))
}

fn run_balanced_rounds(
    timing_executable: &Path,
    attribution_executable: &Path,
    execution: &mut FormalLadderExecution,
    experiment_ordinal: u32,
    persist: &mut impl FnMut(&FormalLadderExecution) -> Result<(), FormalLadderRunnerError>,
) -> Result<bool, FormalLadderRunnerError> {
    execution.analysis = None;
    for level in &mut execution.levels {
        level.formal_runs.clear();
        level.completed_guard_observation = None;
        level.complete = false;
    }
    persist(execution)?;
    let level_count = execution.levels.len();
    for batch in 0..crate::FORMAL_LADDER_BATCH_COUNT {
        for round in 0..crate::FORMAL_LADDER_ROUND_COUNT {
            for position in 0..level_count {
                let level_index = rotated_level_index(level_count, round, position);
                let mut level = execution.levels.remove(level_index);
                let valid = run_level_modes(
                    timing_executable,
                    attribution_executable,
                    execution,
                    &mut level,
                    batch,
                    round,
                    experiment_ordinal,
                    u32::try_from(position)
                        .map_err(|_| FormalLadderRunnerError::ExecutionPositionOverflow)?,
                )?;
                execution.levels.insert(level_index, level);
                if !valid {
                    let invalid_run = execution.levels[level_index]
                        .formal_runs
                        .iter()
                        .rev()
                        .find(|run| run.status != RunStatus::Valid)
                        .ok_or(FormalLadderRunnerError::MissingInvalidFormalRun)?;
                    let invalid_attempt_id = invalid_run.attempt_id.clone();
                    let invalidation_reasons = invalid_run.invalidation_reasons.clone();
                    invalidate_formal_attempt(
                        execution,
                        &invalid_attempt_id,
                        &invalidation_reasons,
                    );
                    persist(execution)?;
                    return Ok(false);
                }
            }
            persist(execution)?;
        }
    }
    for level in &mut execution.levels {
        level.completed_guard_observation = Some(completed_level_observation(level)?);
        level.complete = true;
    }
    persist(execution)?;
    Ok(true)
}

fn rotated_level_index(level_count: usize, round: u32, position: usize) -> usize {
    (position + round as usize) % level_count
}

#[allow(clippy::too_many_arguments)]
fn run_level_modes(
    timing_executable: &Path,
    attribution_executable: &Path,
    execution: &FormalLadderExecution,
    level: &mut FormalLadderLevelExecution,
    batch: u32,
    round: u32,
    experiment_ordinal: u32,
    execution_position: u32,
) -> Result<bool, FormalLadderRunnerError> {
    let modes = if round.is_multiple_of(2) {
        [
            ScalableLadderBinaryMode::Attribution,
            ScalableLadderBinaryMode::Timing,
        ]
    } else {
        [
            ScalableLadderBinaryMode::Timing,
            ScalableLadderBinaryMode::Attribution,
        ]
    };
    for mode in modes {
        let executable = match mode {
            ScalableLadderBinaryMode::Timing => timing_executable,
            ScalableLadderBinaryMode::Attribution => attribution_executable,
        };
        let mode_token = match mode {
            ScalableLadderBinaryMode::Timing => "timing",
            ScalableLadderBinaryMode::Attribution => "attribution",
        };
        let attempt_id = format!(
            "formal/{}/{}/experiment-{experiment_ordinal}/batch-{batch}/round-{round}/{mode_token}/attempt-0",
            execution.workload_id.as_str(),
            execution.graph_profile,
        );
        let run_id = format!("{attempt_id}/n-{}/run", level.n);
        let compiler_instance_id = format!("{run_id}/compiler-instance");
        let graph_profile = parse_graph_profile(&execution.graph_profile)?;
        let expected_digest = expected_level_digest(level)?.to_owned();
        let monitored = run_monitored_scalable_role_child(
            executable,
            level.formal_runs.len(),
            "run-ladder",
            &compiler_instance_id,
            execution.workload_id,
            graph_profile,
            level.n,
            level.guard_preflight.thresholds,
        )?;
        let decoded = decode_child_execution(
            monitored,
            match mode {
                ScalableLadderBinaryMode::Timing => TIMING_BINARY_ID,
                ScalableLadderBinaryMode::Attribution => ATTRIBUTION_BINARY_ID,
            },
            |report: &ScalableLadderChildReport| {
                validate_ladder_report(
                    report,
                    &compiler_instance_id,
                    execution.workload_id,
                    &execution.graph_profile,
                    level.n,
                    level.guard_preflight.thresholds.compiler_controlled_bytes,
                    mode,
                    &expected_digest,
                )
            },
            |report| report.outcome == ScalableLadderOutcome::GuardedInChild,
        )?;
        let run = FormalLadderProcessRun {
            run_id,
            attempt_id,
            retry_ordinal: 0,
            batch,
            round,
            execution_position,
            binary_mode: mode,
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
        };
        let status = run.status;
        level.formal_runs.push(run);
        if status != RunStatus::Valid {
            return Ok(false);
        }
    }
    Ok(true)
}

fn invalidate_formal_attempt(
    execution: &mut FormalLadderExecution,
    attempt_id: &str,
    invalidation_reasons: &[InvalidationReason],
) {
    for run in execution
        .levels
        .iter_mut()
        .flat_map(|level| &mut level.formal_runs)
        .filter(|run| run.attempt_id == attempt_id)
    {
        run.status = RunStatus::Invalid;
        run.invalidation_reasons = invalidation_reasons.to_vec();
    }
}

fn analyze_execution(
    execution: &FormalLadderExecution,
) -> Result<Option<FormalLadderAnalysis>, FormalLadderRunnerError> {
    let completed = execution
        .levels
        .iter()
        .filter(|level| level.complete)
        .map(|level| {
            let runs = level
                .formal_runs
                .iter()
                .map(|run| {
                    Ok(FormalLadderRoundRun {
                        batch: run.batch,
                        round: run.round,
                        binary_mode: run.binary_mode,
                        report: run.child.clone().ok_or(
                            FormalLadderRunnerError::MissingValidChildReport { n: level.n },
                        )?,
                    })
                })
                .collect::<Result<Vec<_>, FormalLadderRunnerError>>()?;
            Ok(FormalLadderCompletedLevel {
                workload_id: execution.workload_id,
                graph_profile: execution.graph_profile.clone(),
                n: level.n,
                primary_record_count: level.primary_record_count,
                canonical_lir_record_count: level.canonical_lir_record_count,
                runs,
            })
        })
        .collect::<Result<Vec<_>, FormalLadderRunnerError>>()?;
    if completed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(analyze_formal_ladder(&completed)?))
    }
}

fn completed_level_observation(
    level: &FormalLadderLevelExecution,
) -> Result<GuardCompletedLevelObservation, FormalLadderRunnerError> {
    let wall_time_ns = level
        .formal_runs
        .iter()
        .filter(|run| run.binary_mode == ScalableLadderBinaryMode::Timing)
        .flat_map(|run| run.child.iter())
        .flat_map(|child| {
            child
                .cold_instance
                .iter()
                .chain(child.stable_capacity_reuse.iter())
        })
        .filter_map(|sample| sample.wall_time_ns)
        .max()
        .ok_or(FormalLadderRunnerError::MissingFormalObservation {
            n: level.n,
            metric: "wall-time-ns",
        })?;
    let peak_live_requested_bytes = maximum_observed_controlled_peak(
        level
            .formal_runs
            .iter()
            .filter(|run| run.binary_mode == ScalableLadderBinaryMode::Attribution)
            .flat_map(|run| run.child.iter())
            .flat_map(|child| {
                child
                    .cold_instance
                    .iter()
                    .chain(child.stable_capacity_reuse.iter())
            })
            .map(|sample| Some(sample.guard_peak_live_requested_bytes))
            .chain(std::iter::once(
                level
                    .attribution_preflight
                    .as_ref()
                    .and_then(|run| run.child.as_ref())
                    .and_then(|child| child.guard_peak_live_requested_bytes),
            ))
            .chain(std::iter::once(
                level
                    .oracle
                    .as_ref()
                    .and_then(|run| run.child.as_ref())
                    .and_then(|child| child.guard_peak_live_requested_bytes),
            )),
        level.n,
    )?;
    let private_bytes = maximum_observed_private_bytes(
        level
            .formal_runs
            .iter()
            .map(|run| &run.monitor)
            .chain(level.attribution_preflight.iter().map(|run| &run.monitor))
            .chain(level.timing_guard_run.iter().map(|run| &run.monitor))
            .chain(level.oracle.iter().map(|run| &run.monitor)),
    )?;
    Ok(GuardCompletedLevelObservation {
        n: level.n,
        primary_record_count: level.primary_record_count,
        peak_live_requested_bytes,
        private_bytes,
        wall_time_ns,
    })
}

fn maximum_observed_private_bytes<'a>(
    reports: impl IntoIterator<Item = &'a ChildProcessMonitorReport>,
) -> Result<u64, FormalLadderRunnerError> {
    reports
        .into_iter()
        .filter_map(|report| report.peak_private_bytes.value)
        .max()
        .filter(|value| *value > 0)
        .ok_or(FormalLadderRunnerError::MissingPrivateBytes)
}

fn maximum_observed_controlled_peak(
    peaks: impl IntoIterator<Item = Option<u64>>,
    n: u32,
) -> Result<u64, FormalLadderRunnerError> {
    peaks
        .into_iter()
        .flatten()
        .filter(|peak| *peak > 0)
        .max()
        .ok_or(FormalLadderRunnerError::MissingFormalObservation {
            n,
            metric: "peak-live-requested-bytes",
        })
}

fn expected_level_digest(
    level: &FormalLadderLevelExecution,
) -> Result<&str, FormalLadderRunnerError> {
    level
        .attribution_preflight
        .as_ref()
        .and_then(|run| run.child.as_ref())
        .and_then(|child| child.semantic_digest_sha256.as_deref())
        .ok_or(FormalLadderRunnerError::InvalidPreflight { n: level.n })
}

fn validate_selection(
    selection: &BaseScaleSelection,
    b: u32,
) -> Result<(), FormalLadderRunnerError> {
    if b == 0
        || selection.candidate_id != BASELINE_CANDIDATE_ID
        || selection.workload_revision != WORKLOAD_REVISION_V1
        || selection.string_profile != BASE_SCALE_STRING_PROFILE
        || selection.generator_version != GENERATOR_VERSION_V1
    {
        return Err(FormalLadderRunnerError::InvalidBaseScaleSelection);
    }
    Ok(())
}

fn previous_pilot_observation(
    selection: &BaseScaleSelection,
    b: u32,
) -> Option<GuardCompletedLevelObservation> {
    b.checked_div(2)
        .filter(|previous| *previous > 0)
        .and_then(|previous| {
            selection
                .pilot_levels
                .iter()
                .find(|level| level.n == previous)
                .map(|level| level.completed_level_guard_observation)
        })
}

fn parse_graph_profile(value: &str) -> Result<GraphProfileId, FormalLadderRunnerError> {
    GraphProfileId::ALL
        .into_iter()
        .find(|profile| profile.as_str() == value)
        .ok_or_else(|| FormalLadderRunnerError::InvalidGraphProfile {
            value: value.to_owned(),
        })
}

#[allow(clippy::too_many_arguments)]
fn validate_attribution_preflight(
    report: &ScalableAttributionChildReport,
    compiler_instance_id: &str,
    workload_id: ScalableWorkloadId,
    graph_profile: GraphProfileId,
    n: u32,
    controlled_ceiling: u64,
) -> Result<(), String> {
    if report.schema != SCALABLE_ATTRIBUTION_CHILD_SCHEMA
        || report.schema_version != SCALABLE_ATTRIBUTION_CHILD_SCHEMA_VERSION
        || report.binary_id != ATTRIBUTION_BINARY_ID
        || !report.allocation_instrumentation_enabled
        || report.compiler_instance_id != compiler_instance_id
        || report.workload_id != workload_id
        || report.workload_revision != WORKLOAD_REVISION_V1
        || report.graph_profile != graph_profile.as_str()
        || report.string_profile != BASE_SCALE_STRING_PROFILE
        || report.generator_version != GENERATOR_VERSION_V1
        || report.n != n
        || report.controlled_allocation_hard_ceiling_bytes != controlled_ceiling
    {
        return Err("attribution-preflight-envelope".to_owned());
    }
    if report.outcome == ScalableAttributionOutcome::Success
        && (report.guard_peak_live_requested_bytes.is_none()
            || report.allocation.is_none()
            || report.attribution_wall_time_ns_diagnostic.is_none()
            || report.retained_capacity_bytes.is_none()
            || report.semantic_digest_sha256.is_none()
            || report.controlled_allocation_guard.is_some())
    {
        return Err("attribution-preflight-success-payload".to_owned());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_oracle(
    report: &ScalableOracleChildReport,
    oracle_run_id: &str,
    workload_id: ScalableWorkloadId,
    graph_profile: GraphProfileId,
    n: u32,
    controlled_ceiling: u64,
    primary_record_count: u64,
    expected_digest: &str,
) -> Result<(), String> {
    if report.schema != SCALABLE_ORACLE_CHILD_SCHEMA
        || report.schema_version != SCALABLE_ORACLE_CHILD_SCHEMA_VERSION
        || report.binary_id != ORACLE_BINARY_ID
        || report.oracle_run_id != oracle_run_id
        || report.workload_id != workload_id
        || report.workload_revision != WORKLOAD_REVISION_V1
        || report.graph_profile != graph_profile.as_str()
        || report.string_profile != BASE_SCALE_STRING_PROFILE
        || report.generator_version != GENERATOR_VERSION_V1
        || report.n != n
        || report.controlled_allocation_hard_ceiling_bytes != controlled_ceiling
    {
        return Err("oracle-envelope".to_owned());
    }
    if report.outcome == ScalableOracleOutcome::Success
        && (report.primary_record_count != Some(primary_record_count)
            || report.guard_peak_live_requested_bytes.is_none()
            || report.semantic_digest_sha256.as_deref() != Some(expected_digest)
            || !report.complete_counts_equal
            || !report.complete_typed_output_equal
            || report.controlled_allocation_guard.is_some())
    {
        return Err("oracle-success-payload".to_owned());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_ladder_report(
    report: &ScalableLadderChildReport,
    compiler_instance_id: &str,
    workload_id: ScalableWorkloadId,
    graph_profile: &str,
    n: u32,
    controlled_ceiling: u64,
    mode: ScalableLadderBinaryMode,
    expected_digest: &str,
) -> Result<(), String> {
    let expected_binary_id = match mode {
        ScalableLadderBinaryMode::Timing => TIMING_BINARY_ID,
        ScalableLadderBinaryMode::Attribution => ATTRIBUTION_BINARY_ID,
    };
    if report.schema != SCALABLE_LADDER_CHILD_SCHEMA
        || report.schema_version != SCALABLE_LADDER_CHILD_SCHEMA_VERSION
        || report.binary_id != expected_binary_id
        || report.binary_mode != mode
        || report.allocation_instrumentation_enabled
            != (mode == ScalableLadderBinaryMode::Attribution)
        || report.compiler_instance_id != compiler_instance_id
        || report.workload_id != workload_id
        || report.workload_revision != WORKLOAD_REVISION_V1
        || report.graph_profile != graph_profile
        || report.string_profile != BASE_SCALE_STRING_PROFILE
        || report.generator_version != GENERATOR_VERSION_V1
        || report.n != n
        || report.controlled_allocation_hard_ceiling_bytes != controlled_ceiling
    {
        return Err("formal-ladder-envelope".to_owned());
    }
    if report.outcome == ScalableLadderOutcome::Success {
        let cold = report
            .cold_instance
            .as_ref()
            .ok_or_else(|| "formal-ladder-cold".to_owned())?;
        if report.warmup_count != STABLE_CAPACITY_WARMUP_COUNT
            || report.stable_capacity_reuse.len() != STABLE_CAPACITY_SAMPLE_COUNT as usize
            || !valid_sample_shape(cold, mode, 0)
            || cold.semantic_digest_sha256 != expected_digest
            || report
                .stable_capacity_reuse
                .iter()
                .enumerate()
                .any(|(ordinal, sample)| {
                    sample.semantic_digest_sha256 != expected_digest
                        || !valid_sample_shape(sample, mode, ordinal as u32)
                })
            || report.retained_capacity_bytes.is_none()
            || report.controlled_allocation_guard.is_some()
        {
            return Err("formal-ladder-success-payload".to_owned());
        }
    }
    Ok(())
}

fn valid_sample_shape(
    sample: &ScalableLadderSample,
    mode: ScalableLadderBinaryMode,
    expected_ordinal: u32,
) -> bool {
    if sample.sample_ordinal != expected_ordinal {
        return false;
    }
    match mode {
        ScalableLadderBinaryMode::Timing => {
            sample.wall_time_ns.is_some()
                && sample.attribution_wall_time_ns_diagnostic.is_none()
                && sample.allocation.is_none()
        }
        ScalableLadderBinaryMode::Attribution => {
            sample.wall_time_ns.is_none()
                && sample.attribution_wall_time_ns_diagnostic.is_some()
                && sample.allocation.is_some()
        }
    }
}

pub(crate) struct DecodedChild<T> {
    pub(crate) status: RunStatus,
    pub(crate) invalidation_reasons: Vec<InvalidationReason>,
    pub(crate) process: ProcessObservation,
    pub(crate) child: Option<T>,
    pub(crate) monitor: ChildProcessMonitorReport,
    pub(crate) external_state: Option<ExternalStateObservation>,
    pub(crate) kill_error: Option<String>,
    pub(crate) monitor_error: Option<String>,
    pub(crate) stderr: String,
}

pub(crate) fn decode_child_execution<T: DeserializeOwned>(
    execution: MonitoredChildExecution,
    binary_id: &str,
    validate: impl FnOnce(&T) -> Result<(), String>,
    guarded: impl FnOnce(&T) -> bool,
) -> Result<DecodedChild<T>, FormalLadderRunnerError> {
    let (
        child_pid,
        output,
        monitor,
        external_state,
        monitor_invalid,
        kill_error,
        mut monitor_error,
    ) = match execution {
        MonitoredChildExecution::Exited {
            child_pid,
            output,
            monitor,
            external_state,
        } => (
            child_pid,
            output,
            monitor,
            external_state,
            false,
            None,
            None,
        ),
        MonitoredChildExecution::InvalidatedByMonitor {
            child_pid,
            output,
            monitor,
            kill_error,
            monitor_error,
            external_state,
        } => (
            child_pid,
            output,
            monitor,
            external_state,
            true,
            kill_error,
            monitor_error,
        ),
    };
    let mut invalidation_reasons = if monitor_invalid {
        monitor_invalidation_reasons(monitor.trigger, output.status.success())
            .ok_or(FormalLadderRunnerError::MissingMonitorTrigger)?
    } else if output.status.success() {
        Vec::new()
    } else {
        vec![InvalidationReason::ChildAbnormalExit]
    };
    let mut child = None;
    let mut child_guarded = false;
    if !monitor_invalid && output.status.success() {
        match serde_json::from_slice::<T>(&output.stdout) {
            Ok(parsed) => match validate(&parsed) {
                Ok(()) => {
                    child_guarded = guarded(&parsed);
                    child = Some(parsed);
                }
                Err(error) => {
                    invalidation_reasons.push(InvalidationReason::ChildAbnormalExit);
                    monitor_error = Some(format!("invalid-child-report:{error}"));
                }
            },
            Err(error) => {
                invalidation_reasons.push(InvalidationReason::ChildAbnormalExit);
                monitor_error = Some(format!("invalid-child-report-json:{error}"));
            }
        }
    }
    if child_guarded {
        invalidation_reasons.push(InvalidationReason::ResearchStopGuardrailTriggered);
    }
    if let (Some(external_state), Some(environment)) =
        (&external_state, installed_formal_environment())
    {
        invalidation_reasons.extend(external_state.invalidation_reasons(environment));
    }
    invalidation_reasons.sort_unstable();
    invalidation_reasons.dedup();
    let status = if child_guarded {
        RunStatus::Invalid
    } else if invalidation_reasons.is_empty() {
        RunStatus::Valid
    } else {
        RunStatus::Invalid
    };
    let process = if child_guarded {
        ProcessObservation::guarded_in_child(
            std::process::id(),
            child_pid,
            binary_id,
            output.status,
        )?
    } else if output.status.success() {
        ProcessObservation::success(std::process::id(), child_pid, binary_id, output.status)?
    } else if monitor_invalid {
        ProcessObservation::invalid_monitor_termination(
            std::process::id(),
            child_pid,
            binary_id,
            output.status,
        )?
    } else {
        ProcessObservation::invalid_abnormal_exit(
            std::process::id(),
            child_pid,
            binary_id,
            output.status,
        )?
    };
    Ok(DecodedChild {
        status,
        invalidation_reasons,
        process,
        child,
        monitor,
        external_state,
        kill_error,
        monitor_error,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

#[derive(Debug, thiserror::Error)]
pub enum FormalLadderRunnerError {
    #[error(transparent)]
    Guard(#[from] crate::GuardError),
    #[error(transparent)]
    ScalePlan(#[from] crate::ScalePlanError),
    #[error(transparent)]
    Pilot(#[from] PilotError),
    #[error(transparent)]
    ProcessProtocol(#[from] ProcessProtocolError),
    #[error(transparent)]
    Analysis(#[from] FormalLadderError),
    #[error("基础规模选择与正式阶梯冻结身份不一致")]
    InvalidBaseScaleSelection,
    #[error("无法解析模块图配置档 {value}")]
    InvalidGraphProfile { value: String },
    #[error("正式阶梯规模二倍扩展溢出")]
    ScaleOverflow,
    #[error("正式阶梯执行位置无法表示为 u32")]
    ExecutionPositionOverflow,
    #[error("正式阶梯 N={n} 的插桩预检无效")]
    InvalidPreflight { n: u32 },
    #[error("正式阶梯无效轮次缺少对应原始运行")]
    MissingInvalidFormalRun,
    #[error("正式阶梯 N={n} 缺少有效子进程报告")]
    MissingValidChildReport { n: u32 },
    #[error("正式阶梯 N={n} 缺少 {metric} 观察")]
    MissingFormalObservation { n: u32, metric: &'static str },
    #[error("正式阶梯缺少进程私有字节观察")]
    MissingPrivateBytes,
    #[error("受监控子进程无对应监控触发")]
    MissingMonitorTrigger,
    #[error("正式阶梯检查点写出失败：{detail}")]
    CheckpointPersistence { detail: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_rotation_covers_each_level_once_per_round() {
        for level_count in 5_usize..=8 {
            for round in 0..crate::FORMAL_LADDER_ROUND_COUNT {
                let order = (0..level_count)
                    .map(|position| rotated_level_index(level_count, round, position))
                    .collect::<Vec<_>>();
                let mut sorted = order.clone();
                sorted.sort_unstable();
                assert_eq!(sorted, (0..level_count).collect::<Vec<_>>());
                assert_eq!(order[0], round as usize % level_count);
            }
        }
    }

    #[test]
    fn even_rounds_run_memory_first_and_odd_rounds_run_timing_first() {
        for round in 0..crate::FORMAL_LADDER_ROUND_COUNT {
            let first = if round % 2 == 0 {
                ScalableLadderBinaryMode::Attribution
            } else {
                ScalableLadderBinaryMode::Timing
            };
            assert_eq!(
                first,
                if round % 2 == 0 {
                    ScalableLadderBinaryMode::Attribution
                } else {
                    ScalableLadderBinaryMode::Timing
                }
            );
        }
    }

    #[test]
    fn sample_shape_keeps_timing_and_attribution_metrics_separate() {
        let timing = ScalableLadderSample {
            sample_ordinal: 3,
            wall_time_ns: Some(1),
            attribution_wall_time_ns_diagnostic: None,
            semantic_digest_sha256: "00".repeat(32),
            guard_peak_live_requested_bytes: 1,
            allocation: None,
        };
        assert!(valid_sample_shape(
            &timing,
            ScalableLadderBinaryMode::Timing,
            3
        ));
        assert!(!valid_sample_shape(
            &timing,
            ScalableLadderBinaryMode::Attribution,
            3
        ));

        let attribution = ScalableLadderSample {
            sample_ordinal: 4,
            wall_time_ns: None,
            attribution_wall_time_ns_diagnostic: Some(1),
            semantic_digest_sha256: "00".repeat(32),
            guard_peak_live_requested_bytes: 1,
            allocation: Some(crate::IdentityAllocationSnapshot::default()),
        };
        assert!(valid_sample_shape(
            &attribution,
            ScalableLadderBinaryMode::Attribution,
            4
        ));
        assert!(!valid_sample_shape(
            &attribution,
            ScalableLadderBinaryMode::Timing,
            4
        ));
    }

    #[test]
    fn completed_controlled_peak_includes_higher_oracle_observation() {
        assert_eq!(
            maximum_observed_controlled_peak([Some(100), Some(175)], 8).unwrap(),
            175
        );
    }

    #[test]
    fn completed_controlled_peak_rejects_missing_or_zero_observations() {
        assert!(matches!(
            maximum_observed_controlled_peak([None, Some(0)], 8),
            Err(FormalLadderRunnerError::MissingFormalObservation {
                n: 8,
                metric: "peak-live-requested-bytes"
            })
        ));
    }
}
