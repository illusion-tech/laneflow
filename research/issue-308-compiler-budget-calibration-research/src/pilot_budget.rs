//! 从基础规模原始样本重算冷实例临时性能预算。
//!
//! 这里只核对会影响预算的数据事实并写出普通 JSON/Markdown，不实现 Evidence v1、
//! 制品防篡改或产品服务等级协议。

use crate::{
    BASE_SCALE_AGGREGATION_METHOD, BASE_SCALE_PILOT_CHECKPOINT_SCHEMA,
    BASE_SCALE_PILOT_CHECKPOINT_SCHEMA_VERSION, BASE_SCALE_SELECTION_RULE,
    BASE_SCALE_STRING_PROFILE, BASELINE_CANDIDATE_ID, BaseScaleOracleRun, BaseScalePilotCheckpoint,
    BaseScalePilotLevel, BaseScalePilotRun, BaseScalePilotRunKind, BaseScaleSelection,
    CLOCK_QUANTUM_MULTIPLIER, ChildProcessMonitorReport, FORMAL_PROTOCOL_CHECKPOINT_SCHEMA,
    FORMAL_PROTOCOL_CHECKPOINT_SCHEMA_VERSION, FORMAL_PROTOCOL_ID,
    FRESH_PROCESS_PILOT_SAMPLE_COUNT, GENERATOR_VERSION_V1, GraphProfileId,
    GuardCompletedLevelObservation, GuardPredictionBasis, GuardPreflightReport, GuardThresholds,
    MAXIMUM_RELATIVE_MAD_PERCENT, ORACLE_BINARY_ID, ProcessExitKind, ProcessObservation, RunStatus,
    SCALABLE_ORACLE_CHILD_SCHEMA, SCALABLE_ORACLE_CHILD_SCHEMA_VERSION,
    SCALABLE_TIMING_CHILD_SCHEMA, SCALABLE_TIMING_CHILD_SCHEMA_VERSION, ScalableOracleOutcome,
    ScalableTimingOutcome, ScalableWorkloadId, TIMING_BINARY_ID, TerminationKind,
    WORKLOAD_REVISION_V1,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

pub const PILOT_BUDGET_REPORT_SCHEMA: &str = "laneflow.compiler-calibration-pilot-budget-report";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PilotBudgetRequest {
    pub input_path: PathBuf,
    pub json_output_path: PathBuf,
    pub markdown_output_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PilotBudgetOutcome {
    pub verified_pilot_identity_count: usize,
    pub estimate_count: usize,
    pub json_output_path: PathBuf,
    pub markdown_output_path: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckpointInput {
    schema: String,
    schema_version: u32,
    base_scale_pilot: BaseScalePilotCheckpoint,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Analysis {
    workload_id: ScalableWorkloadId,
    graph_profile: String,
    b: u32,
    source_run_ids: Vec<String>,
    source_oracle_run_id: String,
    semantic_digest_sha256: String,
    wall_values: Vec<u64>,
    wall_median: u64,
    wall_mad: u64,
    peak_values: Vec<u64>,
    peak_median: u64,
    peak_mad: u64,
    completed_guard_observation: GuardCompletedLevelObservation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    schema: String,
    schema_version: u32,
    budget_basis: String,
    scope: String,
    coverage: Coverage,
    source_checkpoint: SourceCheckpoint,
    verified_pilot_identity_count: usize,
    clock_quantum_ns: u64,
    pilot_budget_estimates: Vec<Estimate>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceCheckpoint {
    byte_length: u64,
    sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct Coverage {
    metrics: Vec<String>,
    sample_kinds: Vec<String>,
    omitted: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct Estimate {
    workload_id: ScalableWorkloadId,
    graph_profile: String,
    measurement_scale: String,
    n: u32,
    metric: String,
    sample_kind: String,
    binary_mode: String,
    sample_count: usize,
    source_run_ids: Vec<String>,
    source_oracle_run_id: String,
    semantic_digest_sha256: String,
    raw_values: Vec<u64>,
    median: u64,
    median_absolute_deviation: u64,
    observed_upper: u64,
    within_run_spread_ratio: Ratio,
    rounding_quantum: u64,
    suggested_pilot_budget: u64,
    unit: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct Ratio {
    numerator: u64,
    denominator: u64,
}

pub fn recompute_pilot_budget(
    request: &PilotBudgetRequest,
) -> Result<PilotBudgetOutcome, PilotBudgetError> {
    let bytes = fs::read(&request.input_path).map_err(|source| PilotBudgetError::Read {
        path: request.input_path.clone(),
        source,
    })?;
    let source_checkpoint = SourceCheckpoint {
        byte_length: u64::try_from(bytes.len())
            .map_err(|_| invalid("来源检查点长度无法表示为 u64"))?,
        sha256: lower_hex(&Sha256::digest(&bytes)),
    };
    let checkpoint = serde_json::from_slice(&bytes).map_err(PilotBudgetError::Parse)?;
    let report = build_report(checkpoint, source_checkpoint)?;
    let json = serde_json::to_string_pretty(&report).map_err(PilotBudgetError::Serialize)? + "\n";
    write(&request.json_output_path, json.as_bytes())?;
    write(
        &request.markdown_output_path,
        render_markdown(&report).as_bytes(),
    )?;
    Ok(PilotBudgetOutcome {
        verified_pilot_identity_count: report.verified_pilot_identity_count,
        estimate_count: report.pilot_budget_estimates.len(),
        json_output_path: request.json_output_path.clone(),
        markdown_output_path: request.markdown_output_path.clone(),
    })
}

fn build_report(
    checkpoint: CheckpointInput,
    source_checkpoint: SourceCheckpoint,
) -> Result<Report, PilotBudgetError> {
    ensure(
        checkpoint.schema == FORMAL_PROTOCOL_CHECKPOINT_SCHEMA
            && checkpoint.schema_version == FORMAL_PROTOCOL_CHECKPOINT_SCHEMA_VERSION,
        "输入不是受支持的正式执行检查点",
    )?;
    let pilot = checkpoint.base_scale_pilot;
    ensure(
        pilot.schema == BASE_SCALE_PILOT_CHECKPOINT_SCHEMA
            && pilot.schema_version == BASE_SCALE_PILOT_CHECKPOINT_SCHEMA_VERSION
            && pilot.protocol_id == FORMAL_PROTOCOL_ID,
        "基础规模检查点协议身份错误",
    )?;
    ensure(pilot.active_selection.is_none(), "基础规模发现尚未完成")?;
    ensure(pilot.clock_quantum_ns > 0, "时钟量子必须是正整数")?;
    let required_median_wall_time_ns = pilot
        .clock_quantum_ns
        .checked_mul(CLOCK_QUANTUM_MULTIPLIER)
        .ok_or_else(|| invalid("时钟可靠阈值溢出"))?;
    ensure(
        pilot.required_median_wall_time_ns == required_median_wall_time_ns,
        "基础规模时钟可靠阈值没有由时钟量子精确派生",
    )?;

    let mut runs = BTreeMap::new();
    for run in &pilot.runs {
        ensure(
            !run.run_id.is_empty() && runs.insert(run.run_id.as_str(), run).is_none(),
            "基础规模运行 ID 为空或重复",
        )?;
    }
    let mut oracle_runs = BTreeMap::new();
    for run in &pilot.oracle_runs {
        ensure(
            !run.run_id.is_empty() && oracle_runs.insert(run.run_id.as_str(), run).is_none(),
            "基础规模预言机 ID 为空或重复",
        )?;
    }

    let mut identities = BTreeSet::new();
    let mut analyses = Vec::new();
    for selection in &pilot.selections {
        ensure(
            identities.insert((selection.workload_id, selection.graph_profile.clone())),
            "基础规模自然身份重复",
        )?;
        analyses.push(analyze(
            selection,
            &runs,
            &oracle_runs,
            required_median_wall_time_ns,
        )?);
    }
    ensure(
        identities == expected_identities(),
        "基础规模结果未覆盖三个工作负载与三种模块图配置档",
    )?;

    let mut estimates = Vec::with_capacity(18);
    for analysis in analyses {
        estimates.push(estimate(
            &analysis,
            "wall-time-ns",
            &analysis.wall_values,
            analysis.wall_median,
            analysis.wall_mad,
            pilot.clock_quantum_ns,
            "nanosecond",
        )?);
        estimates.push(estimate(
            &analysis,
            "peak-live-requested-bytes",
            &analysis.peak_values,
            analysis.peak_median,
            analysis.peak_mad,
            1,
            "byte",
        )?);
    }
    Ok(Report {
        schema: PILOT_BUDGET_REPORT_SCHEMA.to_owned(),
        schema_version: 1,
        budget_basis: "base-scale-pilot".to_owned(),
        scope: "供 #292 使用的冷实例临时研究估算；不是正式 R0 研究预算或产品 SLA".to_owned(),
        coverage: Coverage {
            metrics: vec![
                "wall-time-ns".to_owned(),
                "peak-live-requested-bytes".to_owned(),
            ],
            sample_kinds: vec!["cold-instance".to_owned()],
            omitted: [
                "stable-capacity-reuse",
                "retained-capacity-bytes",
                "private-bytes",
                "commit-peak-bytes",
                "candidate-comparison",
                "growth-slope",
                "formal-knee-selection",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        },
        source_checkpoint,
        verified_pilot_identity_count: identities.len(),
        clock_quantum_ns: pilot.clock_quantum_ns,
        pilot_budget_estimates: estimates,
    })
}

fn analyze(
    selection: &BaseScaleSelection,
    runs: &BTreeMap<&str, &BaseScalePilotRun>,
    oracle_runs: &BTreeMap<&str, &BaseScaleOracleRun>,
    required_median_wall_time_ns: u64,
) -> Result<Analysis, PilotBudgetError> {
    ensure(
        selection.candidate_id == BASELINE_CANDIDATE_ID
            && selection.workload_revision == WORKLOAD_REVISION_V1
            && selection.string_profile == BASE_SCALE_STRING_PROFILE
            && selection.generator_version == GENERATOR_VERSION_V1
            && selection.selection_rule == BASE_SCALE_SELECTION_RULE,
        "基础规模选择身份与冻结协议不一致",
    )?;
    let b = selection
        .b
        .value
        .ok_or_else(|| invalid("自然身份没有可靠基础规模 B"))?;
    ensure(selection.b.reason.is_none(), "已观察到 B 仍携带不可用原因")?;

    let mut analyzed_levels = Vec::with_capacity(selection.pilot_levels.len());
    let mut previous_observation = None;
    for level in &selection.pilot_levels {
        let analysis = analyze_level(
            selection,
            level,
            runs,
            oracle_runs,
            required_median_wall_time_ns,
            previous_observation,
        )?;
        let qualifies = analysis.wall_median >= required_median_wall_time_ns
            && relative_mad_within_limit(analysis.wall_median, analysis.wall_mad);
        ensure(
            level.qualifies == qualifies,
            format!("N={} 的缓存资格与原始样本重算结果不一致", level.n),
        )?;
        previous_observation = Some(analysis.completed_guard_observation);
        analyzed_levels.push((level.n, qualifies, analysis));
    }
    validate_scale_path(
        analyzed_levels
            .iter()
            .map(|(n, qualifies, _)| (*n, *qualifies)),
        b,
    )?;

    let (_, _, analysis) = analyzed_levels
        .pop()
        .expect("validated non-empty pilot levels");
    ensure(analysis.b == b, "B 与最终重算级别不一致")?;
    Ok(analysis)
}

fn analyze_level(
    selection: &BaseScaleSelection,
    level: &BaseScalePilotLevel,
    runs: &BTreeMap<&str, &BaseScalePilotRun>,
    oracle_runs: &BTreeMap<&str, &BaseScaleOracleRun>,
    required_median_wall_time_ns: u64,
    previous_observation: Option<GuardCompletedLevelObservation>,
) -> Result<Analysis, PilotBudgetError> {
    let n = level.n;
    ensure(
        level.aggregation_method == BASE_SCALE_AGGREGATION_METHOD
            && level.all_semantic_digests_equal
            && level.all_guards_clear
            && level.complete_counts_equal
            && level.complete_typed_output_equal
            && !level.semantic_digest.is_empty()
            && level.semantic_digest == level.oracle_semantic_digest,
        format!("N={n} 未满足护栏或语义正确性要求"),
    )?;
    ensure(
        level.minimum_reliable_wall_time_ns == required_median_wall_time_ns
            && level.wall_time_median_ns > 0,
        format!("N={n} 的时钟可靠阈值或墙钟中位数无效"),
    )?;
    ensure(
        level.contributing_run_ids.len() == FRESH_PROCESS_PILOT_SAMPLE_COUNT
            && level
                .contributing_run_ids
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                == FRESH_PROCESS_PILOT_SAMPLE_COUNT,
        format!("N={n} 必须引用七个不同冷实例运行"),
    )?;

    let mut wall_values = Vec::with_capacity(FRESH_PROCESS_PILOT_SAMPLE_COUNT);
    let mut peak_values = Vec::with_capacity(FRESH_PROCESS_PILOT_SAMPLE_COUNT);
    let mut private_peak_values = Vec::with_capacity(FRESH_PROCESS_PILOT_SAMPLE_COUNT);
    let mut primary_record_counts = BTreeSet::new();
    let mut compiler_instance_ids = BTreeSet::new();
    let mut attempt_identity = None;
    for (position, run_id) in level.contributing_run_ids.iter().enumerate() {
        let run = runs
            .get(run_id.as_str())
            .ok_or_else(|| invalid(format!("缺少基础规模运行 {run_id}")))?;
        let expected_attempt_id = format!(
            "pilot/{}/{}/n-{n}/attempt-{}",
            selection.workload_id.as_str(),
            selection.graph_profile,
            run.retry_ordinal
        );
        let expected_run_id = format!("{expected_attempt_id}/pilot-sample-{position}");
        ensure(
            run.status == RunStatus::Valid
                && run.invalidation_reasons.is_empty()
                && run.run_kind == BaseScalePilotRunKind::ColdInstance
                && run.pilot_sample_position == position as u32
                && run.workload_id == selection.workload_id
                && run.workload_revision == WORKLOAD_REVISION_V1
                && run.graph_profile == selection.graph_profile
                && run.string_profile == BASE_SCALE_STRING_PROFILE
                && run.generator_version == GENERATOR_VERSION_V1
                && run.n == n
                && run.attempt_id == expected_attempt_id
                && run.run_id == expected_run_id,
            format!("基础规模运行 {run_id} 的身份或状态错误"),
        )?;
        match &attempt_identity {
            Some((attempt_id, retry_ordinal)) => ensure(
                attempt_id == &run.attempt_id && *retry_ordinal == run.retry_ordinal,
                format!("N={n} 的七个 timing 样本不属于同一次重试尝试"),
            )?,
            None => attempt_identity = Some((run.attempt_id.clone(), run.retry_ordinal)),
        }
        primary_record_counts.insert(validate_clear_preflight(
            &run.guard_preflight,
            selection,
            n,
            run_id,
            previous_observation,
        )?);
        let child = run
            .child
            .as_ref()
            .ok_or_else(|| invalid(format!("基础规模运行 {run_id} 缺少子进程报告")))?;
        let compiler_instance_is_unique = !child.compiler_instance_id.is_empty()
            && compiler_instance_ids.insert(child.compiler_instance_id.as_str());
        let child_wall_time_ns = positive(child.wall_time_ns, "冷实例墙钟")?;
        let child_peak_live_requested_bytes =
            positive(child.guard_peak_live_requested_bytes, "冷实例峰值请求字节")?;
        ensure(
            successful_process(&run.process, TIMING_BINARY_ID, child.child_pid)
                && child.schema == SCALABLE_TIMING_CHILD_SCHEMA
                && child.schema_version == SCALABLE_TIMING_CHILD_SCHEMA_VERSION
                && child.binary_id == TIMING_BINARY_ID
                && child.child_pid > 0
                && compiler_instance_is_unique
                && child.outcome == ScalableTimingOutcome::Success
                && !child.allocation_instrumentation_enabled
                && child.workload_id == selection.workload_id
                && child.workload_revision == WORKLOAD_REVISION_V1
                && child.graph_profile == selection.graph_profile
                && child.string_profile == BASE_SCALE_STRING_PROFILE
                && child.generator_version == GENERATOR_VERSION_V1
                && child.n == n
                && child.controlled_allocation_hard_ceiling_bytes
                    == run.guard_preflight.thresholds.compiler_controlled_bytes
                && child_peak_live_requested_bytes
                    <= child.controlled_allocation_hard_ceiling_bytes
                && child.controlled_allocation_guard.is_none()
                && child.semantic_digest_sha256.as_deref() == Some(&level.semantic_digest),
            format!("基础规模运行 {run_id} 的子进程报告错误"),
        )?;
        private_peak_values.push(validate_clear_monitor(
            run.monitor.as_ref(),
            &run.guard_preflight,
            Some(child_wall_time_ns),
            run.kill_error.as_deref(),
            run.monitor_error.as_deref(),
            run_id,
        )?);
        wall_values.push(child_wall_time_ns);
        peak_values.push(child_peak_live_requested_bytes);
    }
    ensure(
        primary_record_counts.len() == 1,
        format!("N={n} 的七个 timing 预检主记录计数不一致"),
    )?;
    ensure(
        peak_values.iter().copied().collect::<BTreeSet<_>>().len() == 1,
        format!("N={n} 的七个受控分配峰值不一致"),
    )?;
    let timing_primary_record_count = *primary_record_counts
        .first()
        .expect("validated timing preflight count");
    ensure(
        median_and_mad(&wall_values)?
            == (
                level.wall_time_median_ns,
                level.wall_time_median_absolute_deviation_ns,
            ),
        format!("N={n} 墙钟汇总与七个原始样本不一致"),
    )?;

    let oracle_run = oracle_runs
        .get(level.oracle_run_id.as_str())
        .ok_or_else(|| invalid(format!("缺少基础规模预言机 {}", level.oracle_run_id)))?;
    let oracle = oracle_run
        .child
        .as_ref()
        .ok_or_else(|| invalid("基础规模预言机缺少子进程报告"))?;
    let (attempt_id, _) = attempt_identity
        .as_ref()
        .ok_or_else(|| invalid(format!("N={n} 缺少 timing 尝试身份")))?;
    let referenced_run_ids = level
        .contributing_run_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    ensure(
        runs.values()
            .filter(|run| {
                run.status == RunStatus::Valid
                    && run.run_kind == BaseScalePilotRunKind::ColdInstance
                    && run.workload_id == selection.workload_id
                    && run.graph_profile == selection.graph_profile
                    && run.n == n
            })
            .all(|run| {
                run.attempt_id == *attempt_id && referenced_run_ids.contains(run.run_id.as_str())
            }),
        format!("N={n} 存在未被唯一七样本尝试引用的有效冷运行"),
    )?;
    ensure(
        level.oracle_run_id == format!("{attempt_id}/oracle"),
        format!("N={n} 的预言机不属于七样本重试尝试"),
    )?;
    let oracle_primary_record_count = validate_clear_preflight(
        &oracle_run.guard_preflight,
        selection,
        n,
        &level.oracle_run_id,
        previous_observation,
    )?;
    let oracle_peak_live_requested_bytes =
        positive(oracle.guard_peak_live_requested_bytes, "预言机峰值请求字节")?;
    ensure(
        successful_process(&oracle_run.process, ORACLE_BINARY_ID, oracle.child_pid)
            && oracle_run.status == RunStatus::Valid
            && oracle_run.invalidation_reasons.is_empty()
            && oracle_run.workload_id == selection.workload_id
            && oracle_run.workload_revision == WORKLOAD_REVISION_V1
            && oracle_run.graph_profile == selection.graph_profile
            && oracle_run.string_profile == BASE_SCALE_STRING_PROFILE
            && oracle_run.generator_version == GENERATOR_VERSION_V1
            && oracle_run.n == n
            && oracle.schema == SCALABLE_ORACLE_CHILD_SCHEMA
            && oracle.schema_version == SCALABLE_ORACLE_CHILD_SCHEMA_VERSION
            && oracle.binary_id == ORACLE_BINARY_ID
            && oracle.oracle_run_id == level.oracle_run_id
            && oracle.child_pid > 0
            && oracle.outcome == ScalableOracleOutcome::Success
            && oracle.workload_id == selection.workload_id
            && oracle.workload_revision == WORKLOAD_REVISION_V1
            && oracle.graph_profile == selection.graph_profile
            && oracle.string_profile == BASE_SCALE_STRING_PROFILE
            && oracle.generator_version == GENERATOR_VERSION_V1
            && oracle.n == n
            && oracle.controlled_allocation_hard_ceiling_bytes
                == oracle_run
                    .guard_preflight
                    .thresholds
                    .compiler_controlled_bytes
            && oracle_peak_live_requested_bytes <= oracle.controlled_allocation_hard_ceiling_bytes
            && oracle.primary_record_count == Some(timing_primary_record_count)
            && oracle_primary_record_count == timing_primary_record_count
            && oracle.semantic_digest_sha256.as_deref() == Some(&level.semantic_digest)
            && oracle.complete_counts_equal
            && oracle.complete_typed_output_equal
            && oracle.controlled_allocation_guard.is_none(),
        format!("N={n} 未通过独立预言机"),
    )?;
    let oracle_private_bytes = validate_clear_monitor(
        oracle_run.monitor.as_ref(),
        &oracle_run.guard_preflight,
        None,
        oracle_run.kill_error.as_deref(),
        oracle_run.monitor_error.as_deref(),
        &level.oracle_run_id,
    )?;

    let observed_wall = *wall_values
        .iter()
        .max()
        .ok_or_else(|| invalid("缺少墙钟"))?;
    let observed_peak = (*peak_values
        .iter()
        .max()
        .ok_or_else(|| invalid("缺少峰值请求字节"))?)
    .max(oracle_peak_live_requested_bytes);
    let observed_private_bytes = (*private_peak_values
        .iter()
        .max()
        .ok_or_else(|| invalid("缺少私有字节峰值"))?)
    .max(oracle_private_bytes);
    let completed_guard_observation = GuardCompletedLevelObservation {
        n,
        primary_record_count: timing_primary_record_count,
        peak_live_requested_bytes: observed_peak,
        private_bytes: observed_private_bytes,
        wall_time_ns: observed_wall,
    };
    ensure(
        level.completed_level_guard_observation == completed_guard_observation,
        format!("N={n} 完整护栏观察与原始样本不一致"),
    )?;
    let (peak_median, peak_mad) = median_and_mad(&peak_values)?;
    Ok(Analysis {
        workload_id: selection.workload_id,
        graph_profile: selection.graph_profile.clone(),
        b: n,
        source_run_ids: level.contributing_run_ids.clone(),
        source_oracle_run_id: level.oracle_run_id.clone(),
        semantic_digest_sha256: level.semantic_digest.clone(),
        wall_values,
        wall_median: level.wall_time_median_ns,
        wall_mad: level.wall_time_median_absolute_deviation_ns,
        peak_values,
        peak_median,
        peak_mad,
        completed_guard_observation,
    })
}

fn validate_clear_preflight(
    preflight: &GuardPreflightReport,
    selection: &BaseScaleSelection,
    n: u32,
    run_id: &str,
    previous_observation: Option<GuardCompletedLevelObservation>,
) -> Result<u64, PilotBudgetError> {
    let expected_thresholds = GuardThresholds::from_physical_memory_bytes(
        preflight.memory_observation.physical_memory_bytes,
    )
    .map_err(|_| invalid(format!("运行 {run_id} 的物理内存观察无法派生护栏阈值")))?;
    let prediction_shape_is_valid = match previous_observation {
        Some(previous) => {
            let predicted_compiler_controlled_bytes = checked_guard_prediction(
                previous.peak_live_requested_bytes,
                preflight.primary_record_count,
                previous.primary_record_count,
                expected_thresholds.prediction_safety_factor_numerator,
                expected_thresholds.prediction_safety_factor_denominator,
            )?;
            let predicted_private_bytes = checked_guard_prediction(
                previous.private_bytes,
                preflight.primary_record_count,
                previous.primary_record_count,
                expected_thresholds.prediction_safety_factor_numerator,
                expected_thresholds.prediction_safety_factor_denominator,
            )?;
            let predicted_wall_time_ns = checked_guard_prediction(
                previous.wall_time_ns,
                preflight.primary_record_count,
                previous.primary_record_count,
                expected_thresholds.prediction_safety_factor_numerator,
                expected_thresholds.prediction_safety_factor_denominator,
            )?;
            previous.n.checked_mul(2) == Some(n)
                && preflight.compiler_controlled_prediction_basis
                    == GuardPredictionBasis::PreviousLevelLinearTimesFiveFourths
                && preflight.private_bytes_prediction_basis
                    == GuardPredictionBasis::PreviousLevelLinearTimesFiveFourths
                && preflight.wall_time_prediction_basis
                    == GuardPredictionBasis::PreviousLevelLinearTimesFiveFourths
                && preflight.predicted_compiler_controlled_bytes
                    == preflight
                        .logical_bytes_lower_bound
                        .max(predicted_compiler_controlled_bytes)
                && preflight.predicted_private_bytes == Some(predicted_private_bytes)
                && preflight.predicted_wall_time_ns == Some(predicted_wall_time_ns)
        }
        None => {
            n == 1
                && preflight.compiler_controlled_prediction_basis
                    == GuardPredictionBasis::ManifestSingleBufferLowerBoundV1
                && preflight.private_bytes_prediction_basis
                    == GuardPredictionBasis::FirstLevelMonitorOnly
                && preflight.wall_time_prediction_basis
                    == GuardPredictionBasis::FirstLevelMonitorOnly
                && preflight.predicted_compiler_controlled_bytes
                    == preflight.logical_bytes_lower_bound
                && preflight.predicted_private_bytes.is_none()
                && preflight.predicted_wall_time_ns.is_none()
        }
    };
    ensure(
        preflight.workload_id == selection.workload_id.as_str()
            && preflight.graph_profile == selection.graph_profile
            && preflight.n == n
            && preflight.primary_record_count > 0
            && preflight.memory_observation.available_physical_memory_bytes
                <= preflight.memory_observation.physical_memory_bytes
            && preflight.thresholds == expected_thresholds
            && preflight.logical_bytes_lower_bound > 0
            && prediction_shape_is_valid
            && preflight.maximum_typed_ordinal <= u64::from(u32::MAX)
            && preflight.predicted_compiler_controlled_bytes
                < preflight.thresholds.compiler_controlled_bytes
            && preflight
                .predicted_private_bytes
                .is_none_or(|bytes| bytes < preflight.thresholds.private_bytes)
            && preflight
                .predicted_wall_time_ns
                .is_none_or(|wall_time_ns| wall_time_ns < preflight.thresholds.wall_time_ns)
            && preflight.memory_observation.available_physical_memory_bytes
                >= preflight.thresholds.minimum_available_physical_memory_bytes
            && preflight.triggers.is_empty()
            && preflight.allows_child_start,
        format!("运行 {run_id} 的启动前护栏证据不清洁"),
    )?;
    Ok(preflight.primary_record_count)
}

fn checked_guard_prediction(
    previous_value: u64,
    current_primary_record_count: u64,
    previous_primary_record_count: u64,
    safety_factor_numerator: u64,
    safety_factor_denominator: u64,
) -> Result<u64, PilotBudgetError> {
    ensure(
        previous_value > 0
            && current_primary_record_count > 0
            && previous_primary_record_count > 0
            && safety_factor_numerator > 0
            && safety_factor_denominator > 0,
        "护栏预测输入必须为正整数",
    )?;
    let numerator = u128::from(previous_value)
        .checked_mul(u128::from(current_primary_record_count))
        .and_then(|value| value.checked_mul(u128::from(safety_factor_numerator)))
        .ok_or_else(|| invalid("护栏预测分子溢出"))?;
    let denominator = u128::from(previous_primary_record_count)
        .checked_mul(u128::from(safety_factor_denominator))
        .ok_or_else(|| invalid("护栏预测分母溢出"))?;
    let quotient = numerator / denominator;
    let rounded = quotient
        .checked_add(u128::from(numerator % denominator != 0))
        .ok_or_else(|| invalid("护栏预测上取整溢出"))?;
    u64::try_from(rounded).map_err(|_| invalid("护栏预测无法表示为 u64"))
}

fn validate_clear_monitor(
    monitor: Option<&ChildProcessMonitorReport>,
    preflight: &GuardPreflightReport,
    child_wall_time_ns: Option<u64>,
    kill_error: Option<&str>,
    monitor_error: Option<&str>,
    run_id: &str,
) -> Result<u64, PilotBudgetError> {
    let monitor = monitor.ok_or_else(|| invalid(format!("运行 {run_id} 缺少父进程监控报告")))?;
    let last_private_bytes = positive(monitor.last_private_bytes.value, "最后私有字节观察")?;
    let peak_private_bytes = positive(monitor.peak_private_bytes.value, "私有字节峰值")?;
    ensure(
        monitor.observation_count > 0
            && monitor.last_private_bytes.reason.is_none()
            && monitor.peak_private_bytes.reason.is_none()
            && peak_private_bytes >= last_private_bytes
            && peak_private_bytes < preflight.thresholds.private_bytes
            && monitor.elapsed_wall_time_ns > 0
            && monitor.elapsed_wall_time_ns < preflight.thresholds.wall_time_ns
            && child_wall_time_ns
                .is_none_or(|wall_time_ns| monitor.elapsed_wall_time_ns >= wall_time_ns)
            && monitor.trigger.is_none()
            && kill_error.is_none()
            && monitor_error.is_none(),
        format!("运行 {run_id} 的父进程监控或终止诊断不清洁"),
    )?;
    Ok(peak_private_bytes)
}

fn successful_process(process: &ProcessObservation, binary_id: &str, child_pid: u32) -> bool {
    process.coordinator_pid > 0
        && process.binary_id == binary_id
        && process.exit_kind == ProcessExitKind::Success
        && process.exit_code.value == Some(0)
        && process.exit_code.reason.is_none()
        && process.child_pid.value == Some(u64::from(child_pid))
        && process.child_pid.reason.is_none()
        && process.termination.kind == TerminationKind::ExitCode
        && process.termination.signal_number.value.is_none()
        && process.termination.signal_number.reason.is_some()
        && process.termination.raw_platform_status.value.is_none()
        && process.termination.raw_platform_status.reason.is_some()
}

fn relative_mad_within_limit(median: u64, mad: u64) -> bool {
    median > 0
        && u128::from(mad) * 100 <= u128::from(median) * u128::from(MAXIMUM_RELATIVE_MAD_PERCENT)
}

#[allow(clippy::too_many_arguments)]
fn estimate(
    analysis: &Analysis,
    metric: &str,
    values: &[u64],
    median: u64,
    mad: u64,
    quantum: u64,
    unit: &str,
) -> Result<Estimate, PilotBudgetError> {
    let observed_upper = *values.iter().max().ok_or_else(|| invalid("预算指标为空"))?;
    let ratio = Ratio::new(observed_upper, median)?;
    Ok(Estimate {
        workload_id: analysis.workload_id,
        graph_profile: analysis.graph_profile.clone(),
        measurement_scale: "base-scale".to_owned(),
        n: analysis.b,
        metric: metric.to_owned(),
        sample_kind: "cold-instance".to_owned(),
        binary_mode: "timing".to_owned(),
        sample_count: values.len(),
        source_run_ids: analysis.source_run_ids.clone(),
        source_oracle_run_id: analysis.source_oracle_run_id.clone(),
        semantic_digest_sha256: analysis.semantic_digest_sha256.clone(),
        raw_values: values.to_vec(),
        median,
        median_absolute_deviation: mad,
        observed_upper,
        within_run_spread_ratio: ratio,
        rounding_quantum: quantum,
        suggested_pilot_budget: ceil_scaled(observed_upper, ratio, quantum)?,
        unit: unit.to_owned(),
    })
}

impl Ratio {
    fn new(numerator: u64, denominator: u64) -> Result<Self, PilotBudgetError> {
        ensure(numerator > 0 && denominator > 0, "预算比值必须为正")?;
        let divisor = gcd(numerator, denominator);
        Ok(Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }
}

fn ceil_scaled(value: u64, ratio: Ratio, quantum: u64) -> Result<u64, PilotBudgetError> {
    ensure(quantum > 0, "预算舍入量子必须为正")?;
    let numerator = u128::from(value) * u128::from(ratio.numerator);
    let denominator = u128::from(ratio.denominator) * u128::from(quantum);
    let result = numerator.div_ceil(denominator) * u128::from(quantum);
    u64::try_from(result).map_err(|_| invalid("预算无法表示为 u64"))
}

fn median_and_mad(values: &[u64]) -> Result<(u64, u64), PilotBudgetError> {
    ensure(
        !values.is_empty() && values.len() % 2 == 1,
        "中位数输入必须是非空奇数项",
    )?;
    ensure(values.iter().all(|value| *value > 0), "指标必须是正整数")?;
    let mut ordered = values.to_vec();
    ordered.sort_unstable();
    let median = ordered[ordered.len() / 2];
    let mut deviations = values
        .iter()
        .map(|value| value.abs_diff(median))
        .collect::<Vec<_>>();
    deviations.sort_unstable();
    Ok((median, deviations[deviations.len() / 2]))
}

fn validate_scale_path(
    levels: impl IntoIterator<Item = (u32, bool)>,
    b: u32,
) -> Result<(), PilotBudgetError> {
    let mut levels = levels.into_iter().peekable();
    ensure(levels.peek().is_some(), "基础规模选择缺少试运行级别")?;
    let mut expected_n = 1_u32;
    let mut last_n = None;
    while let Some((n, qualifies)) = levels.next() {
        ensure(n == expected_n, "基础规模试运行没有从一开始严格二倍递增")?;
        let is_last = levels.peek().is_none();
        ensure(
            qualifies == is_last,
            "B 必须是逐级试运行中的首个且最后一个合格级别",
        )?;
        last_n = Some(n);
        if !is_last {
            expected_n = expected_n
                .checked_mul(2)
                .ok_or_else(|| invalid("基础规模试运行级别溢出"))?;
        }
    }
    ensure(last_n == Some(b), "B 与逐级试运行的首个合格级别不一致")
}

fn expected_identities() -> BTreeSet<(ScalableWorkloadId, String)> {
    ScalableWorkloadId::ALL
        .into_iter()
        .flat_map(|workload| {
            GraphProfileId::ALL
                .into_iter()
                .map(move |profile| (workload, profile.as_str().to_owned()))
        })
        .collect()
}

fn render_markdown(report: &Report) -> String {
    let mut output = format!(
        "# #308 冷实例临时性能预算\n\n\
         > 这是供 #292 审阅的临时研究估算，不是正式 R0 研究预算，也不是产品 SLA。\n\n\
         - 已从原始样本重新计算基础规模自然身份：{} 个\n\
         - 每个自然身份使用七个独立冷实例样本，并核对语义摘要与独立预言机\n\
         - 预算按观测上界乘以同组样本的最大值/中位数离散比后向上取整\n\
         - 来源检查点：{} 字节，SHA-256 `{}`\n\
         - 时钟量子：{} ns\n\n\
         | 工作负载 | 模块图 | 测量规模 | N | 指标 | 原始值 | 中位数 | MAD | 观测上界 | 预算放大比 | 建议预算 | 单位 |\n\
         | --- | --- | --- | ---: | --- | --- | ---: | ---: | ---: | ---: | ---: | --- |\n",
        report.verified_pilot_identity_count,
        report.source_checkpoint.byte_length,
        report.source_checkpoint.sha256,
        report.clock_quantum_ns
    );
    for item in &report.pilot_budget_estimates {
        let raw_values = item
            .raw_values
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        output.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {}/{} | {} | {} |\n",
            item.workload_id.as_str(),
            item.graph_profile,
            item.measurement_scale,
            item.n,
            item.metric,
            raw_values,
            item.median,
            item.median_absolute_deviation,
            item.observed_upper,
            item.within_run_spread_ratio.numerator,
            item.within_run_spread_ratio.denominator,
            item.suggested_pilot_budget,
            item.unit
        ));
    }
    output
}

fn lower_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;

    bytes.iter().fold(
        String::with_capacity(bytes.len() * 2),
        |mut output, byte| {
            write!(output, "{byte:02x}").expect("writing to a String cannot fail");
            output
        },
    )
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), PilotBudgetError> {
    fs::write(path, bytes).map_err(|source| PilotBudgetError::Write {
        path: path.to_path_buf(),
        source,
    })
}

fn positive(value: Option<u64>, name: &str) -> Result<u64, PilotBudgetError> {
    let value = value.ok_or_else(|| invalid(format!("缺少{name}")))?;
    ensure(value > 0, format!("{name} 必须为正"))?;
    Ok(value)
}

fn gcd(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left
}

fn ensure(condition: bool, detail: impl Into<String>) -> Result<(), PilotBudgetError> {
    if condition {
        Ok(())
    } else {
        Err(invalid(detail))
    }
}

fn invalid(detail: impl Into<String>) -> PilotBudgetError {
    PilotBudgetError::InvalidData {
        detail: detail.into(),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PilotBudgetError {
    #[error("无法读取预算输入 `{path}`：{source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("预算输入 JSON 无法解析：{0}")]
    Parse(serde_json::Error),
    #[error("预算数据无效：{detail}")]
    InvalidData { detail: String },
    #[error("无法序列化预算结果：{0}")]
    Serialize(serde_json::Error),
    #[error("无法写出预算结果 `{path}`：{source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_statistics_and_budget_rounding_are_exact() {
        assert_eq!(median_and_mad(&[1, 2, 3, 4, 100]).unwrap(), (3, 1));
        let ratio = Ratio::new(12, 8).unwrap();
        assert_eq!(
            ratio,
            Ratio {
                numerator: 3,
                denominator: 2
            }
        );
        assert_eq!(ceil_scaled(101, ratio, 10).unwrap(), 160);
    }

    #[test]
    fn expected_identity_matrix_is_three_by_three() {
        assert_eq!(expected_identities().len(), 9);
    }

    #[test]
    fn base_scale_path_requires_strict_doubling_and_first_qualifying_last_level() {
        assert!(validate_scale_path([(1, false), (2, false), (4, true)], 4).is_ok());
        assert!(validate_scale_path([], 1).is_err());
        assert!(validate_scale_path([(2, true)], 2).is_err());
        assert!(validate_scale_path([(1, false), (4, true)], 4).is_err());
        assert!(validate_scale_path([(1, true), (2, true)], 2).is_err());
        assert!(validate_scale_path([(1, false), (2, true)], 4).is_err());
    }

    #[test]
    fn relative_mad_limit_uses_the_frozen_exact_ratio() {
        assert!(relative_mad_within_limit(100, 2));
        assert!(!relative_mad_within_limit(100, 3));
        assert!(relative_mad_within_limit(50, 1));
        assert!(!relative_mad_within_limit(49, 1));
        assert!(!relative_mad_within_limit(0, 0));
    }

    #[test]
    fn guard_prediction_uses_exact_five_fourths_ceiling_arithmetic() {
        assert_eq!(checked_guard_prediction(3, 2, 1, 5, 4).unwrap(), 8);
        assert_eq!(checked_guard_prediction(4, 2, 1, 5, 4).unwrap(), 10);
        assert!(checked_guard_prediction(1, 1, 0, 5, 4).is_err());
    }
}
