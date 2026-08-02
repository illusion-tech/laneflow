//! #308 编译器预算校准的紧凑 Evidence v1。
//!
//! 原始正式执行 JSON 是逐次运行的唯一事实源。Evidence 只绑定该原始制品并发布独立
//! 重算后的规模选择、正式阶梯、预算与候选分类；绝不复制数千条运行记录。

use crate::{
    CandidateDisposition, ContractError, FORMAL_PROTOCOL_CHECKPOINT_SCHEMA,
    FORMAL_PROTOCOL_CHECKPOINT_SCHEMA_VERSION, FormalLadderAnalysis, FormalLadderBatchSummary,
    FormalLadderCompletedLevel, FormalLadderRoundRun, FormalProtocolCheckpoint, RunStatus,
    TrustedContract, analyze_formal_ladder, load_repository_contract, repository_root,
    validate_limit_qualification_bundle, verify_current_fixtures_oracle,
};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

pub const EVIDENCE_SCHEMA_ID: &str = "laneflow.compiler-calibration-evidence";
pub const EVIDENCE_SCHEMA_VERSION: u64 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvidenceWriteRequest {
    pub checkpoint_path: PathBuf,
    pub output_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceWriteOutcome {
    pub output_path: PathBuf,
    pub byte_length: u64,
    pub sha256: String,
    pub verification: EvidenceVerificationReport,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceVerificationReport {
    pub schema: String,
    pub schema_version: u64,
    pub source_commit: String,
    pub raw_byte_length: u64,
    pub raw_sha256: String,
    pub process_run_count: u64,
    pub invalid_run_count: u64,
    pub guarded_run_count: u64,
    pub base_scale_count: usize,
    pub formal_ladder_count: usize,
    pub completed_formal_level_count: u64,
    pub budget_recommendation_count: usize,
    pub candidate_classification_count: usize,
}

pub fn write_evidence_v1(
    request: &EvidenceWriteRequest,
) -> Result<EvidenceWriteOutcome, EvidenceError> {
    if request.output_path.exists() {
        return Err(EvidenceError::OutputAlreadyExists {
            path: request.output_path.clone(),
        });
    }
    let trusted = load_repository_contract()?;
    let raw_bytes = fs::read(&request.checkpoint_path).map_err(|source| EvidenceError::Read {
        path: request.checkpoint_path.clone(),
        source,
    })?;
    let checkpoint = parse_checkpoint(&request.checkpoint_path, &raw_bytes)?;
    let raw_path = repository_relative_path(&request.checkpoint_path)?;
    let raw_sha256 = sha256_hex(&raw_bytes);
    let document = build_document(
        &trusted,
        &checkpoint,
        &raw_path,
        u64::try_from(raw_bytes.len()).expect("raw byte length fits u64"),
        &raw_sha256,
    )?;
    validate_schema(&trusted, &document)?;
    let verification = report(&document)?;
    let bytes = serde_json::to_vec_pretty(&document)?;
    write_atomically(&request.output_path, &bytes)?;
    Ok(EvidenceWriteOutcome {
        output_path: request.output_path.clone(),
        byte_length: u64::try_from(bytes.len() + 1).expect("evidence byte length fits u64"),
        sha256: sha256_with_newline(&bytes),
        verification,
    })
}

pub fn verify_evidence_v1(path: &Path) -> Result<EvidenceVerificationReport, EvidenceError> {
    let trusted = load_repository_contract()?;
    let evidence_bytes = fs::read(path).map_err(|source| EvidenceError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let document: Value =
        serde_json::from_slice(&evidence_bytes).map_err(|source| EvidenceError::InvalidJson {
            path: path.to_path_buf(),
            source,
        })?;
    validate_schema(&trusted, &document)?;
    let raw_path = required_string(&document, "/rawExecution/path")?;
    let raw_file = resolve_repository_path(raw_path)?;
    let raw_bytes = fs::read(&raw_file).map_err(|source| EvidenceError::Read {
        path: raw_file.clone(),
        source,
    })?;
    let raw_length = u64::try_from(raw_bytes.len()).expect("raw byte length fits u64");
    let raw_sha256 = sha256_hex(&raw_bytes);
    expect_u64(&document, "/rawExecution/byteLength", raw_length)?;
    expect_string(&document, "/rawExecution/sha256", &raw_sha256)?;
    let checkpoint = parse_checkpoint(&raw_file, &raw_bytes)?;
    let expected = build_document(&trusted, &checkpoint, raw_path, raw_length, &raw_sha256)?;
    if document != expected {
        return Err(EvidenceError::RecomputationMismatch);
    }
    report(&document)
}

fn build_document(
    trusted: &TrustedContract,
    checkpoint: &FormalProtocolCheckpoint,
    raw_path: &str,
    raw_byte_length: u64,
    raw_sha256: &str,
) -> Result<Value, EvidenceError> {
    validate_source(checkpoint)?;
    crate::pilot_budget::validate_base_scale_pilot(&checkpoint.base_scale_pilot)
        .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    let fixture_verification = verify_current_fixtures_oracle(trusted)
        .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    let fixture_child =
        checkpoint.current_fixtures.child.as_ref().ok_or_else(|| {
            EvidenceError::InvalidCheckpoint("当前固定样例缺少子进程报告".to_owned())
        })?;
    if checkpoint.current_fixtures.status != RunStatus::Valid
        || fixture_child.verification != fixture_verification
    {
        return Err(EvidenceError::InvalidCheckpoint(
            "当前固定样例独立复算不一致".to_owned(),
        ));
    }

    let analyses = recompute_formal_analyses(trusted, checkpoint)?;
    let (envelopes, budgets) =
        derive_budgets(&analyses, checkpoint.base_scale_pilot.clock_quantum_ns)?;
    let candidate_matrix = checkpoint
        .candidate_matrix
        .as_ref()
        .ok_or_else(|| EvidenceError::InvalidCheckpoint("缺少候选矩阵".to_owned()))?;
    if candidate_matrix.active_execution.is_some()
        || candidate_matrix
            .executions
            .iter()
            .any(|execution| !execution.complete)
    {
        return Err(EvidenceError::InvalidCheckpoint(
            "候选矩阵未完成".to_owned(),
        ));
    }
    let candidate_results = candidate_results(trusted, checkpoint, candidate_matrix, &envelopes)?;
    let limit = checkpoint
        .limit_qualification
        .as_ref()
        .ok_or_else(|| EvidenceError::InvalidCheckpoint("缺少限制资格结果".to_owned()))?;
    validate_limit_qualification_bundle(limit)
        .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    let coverage = coverage(checkpoint, candidate_matrix);

    Ok(json!({
        "schema": EVIDENCE_SCHEMA_ID,
        "schemaVersion": EVIDENCE_SCHEMA_VERSION,
        "rawExecution": {
            "schema": FORMAL_PROTOCOL_CHECKPOINT_SCHEMA,
            "schemaVersion": FORMAL_PROTOCOL_CHECKPOINT_SCHEMA_VERSION,
            "path": raw_path,
            "byteLength": raw_byte_length,
            "sha256": raw_sha256
        },
        "source": {
            "measurementCommit": checkpoint.source.source_commit,
            "harnessCommit": checkpoint.source.harness_commit,
            "dirty": checkpoint.source.dirty,
            "cargoLockSha256": checkpoint.source.cargo_lock_sha256,
            "contractDescriptorSha256": trusted.descriptor_sha256,
            "workloadManifestSha256": trusted.descriptor.workload_manifest.sha256,
            "evidenceSchemaSha256": trusted.descriptor.evidence_schema.sha256,
            "binaries": checkpoint.source.binaries
        },
        "environment": checkpoint.environment,
        "protocol": {
            "id": checkpoint.protocol_id,
            "clockQuantumNs": checkpoint.base_scale_pilot.clock_quantum_ns,
            "batchCount": crate::FORMAL_LADDER_BATCH_COUNT,
            "roundCountPerBatch": crate::FORMAL_LADDER_ROUND_COUNT
        },
        "coverage": coverage,
        "results": {
            "currentFixtures": {
                "caseCount": fixture_child.cases.len(),
                "verification": fixture_verification
            },
            "baseScaleSelections": checkpoint.base_scale_pilot.selections,
            "formalLadders": analyses,
            "reproducibilityEnvelopes": envelopes,
            "budgetRecommendations": budgets,
            "limitQualification": {
                "scales": limit.scales,
                "liveByteBaselineRunCount": limit.live_byte_baseline_runs.len(),
                "limitPairCount": limit.limit_pairs.len(),
                "duplicateOwnerQualificationCount": limit.duplicate_owner_qualifications.len(),
                "cleanupExperimentCount": limit.cleanup_experiments.len()
            },
            "candidateMatrix": {
                "scope": candidate_matrix.scope,
                "scales": candidate_matrix.scales,
                "safetyAudit": candidate_matrix.safety_audit,
                "constantHashQualifications": candidate_matrix.constant_hash_qualifications
                    .iter().map(|execution| &execution.qualification).collect::<Vec<_>>(),
                "executions": candidate_results
            }
        }
    }))
}

fn validate_source(checkpoint: &FormalProtocolCheckpoint) -> Result<(), EvidenceError> {
    if checkpoint.schema != FORMAL_PROTOCOL_CHECKPOINT_SCHEMA
        || checkpoint.schema_version != FORMAL_PROTOCOL_CHECKPOINT_SCHEMA_VERSION
        || checkpoint.protocol_id != crate::FORMAL_PROTOCOL_ID
        || checkpoint.source.dirty
        || !is_git_commit(&checkpoint.source.source_commit)
        || !is_git_commit(&checkpoint.source.harness_commit)
    {
        return Err(EvidenceError::InvalidCheckpoint(
            "原始检查点身份或源码状态无效".to_owned(),
        ));
    }
    let lock =
        fs::read(repository_root().join("Cargo.lock")).map_err(|source| EvidenceError::Read {
            path: repository_root().join("Cargo.lock"),
            source,
        })?;
    if checkpoint.source.cargo_lock_sha256 != sha256_hex(&lock) {
        return Err(EvidenceError::InvalidCheckpoint(
            "原始检查点 Cargo.lock 绑定不匹配".to_owned(),
        ));
    }
    validate_binary_sources(&checkpoint.source.binaries)
}

fn validate_binary_sources(
    binaries: &[crate::FormalBinarySourceSnapshot],
) -> Result<(), EvidenceError> {
    let mut binary_digests = BTreeMap::new();
    let mut unique_digests = BTreeSet::new();
    for binary in binaries {
        if !is_sha256(&binary.sha256)
            || !unique_digests.insert(binary.sha256.as_str())
            || binary_digests
                .insert(binary.binary_id.as_str(), binary.sha256.as_str())
                .is_some()
        {
            return Err(EvidenceError::InvalidCheckpoint(
                "原始检查点二进制角色/摘要重复或摘要无效".to_owned(),
            ));
        }
    }
    let expected_binary_ids = [
        crate::TIMING_BINARY_ID,
        crate::ATTRIBUTION_BINARY_ID,
        crate::ORACLE_BINARY_ID,
    ];
    if binary_digests.len() != expected_binary_ids.len()
        || expected_binary_ids
            .iter()
            .any(|binary_id| !binary_digests.contains_key(binary_id))
    {
        return Err(EvidenceError::InvalidCheckpoint(
            "原始检查点没有精确绑定三个研究二进制".to_owned(),
        ));
    }
    Ok(())
}

fn is_git_commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn recompute_formal_analyses(
    trusted: &TrustedContract,
    checkpoint: &FormalProtocolCheckpoint,
) -> Result<Vec<Value>, EvidenceError> {
    if checkpoint.active_formal_ladder.is_some()
        || checkpoint.formal_ladders.len() != checkpoint.base_scale_pilot.selections.len()
    {
        return Err(EvidenceError::InvalidCheckpoint(
            "正式阶梯集合尚未完整结束".to_owned(),
        ));
    }
    validate_formal_ladder_base_scales(checkpoint)?;
    let plans = crate::ScalableStagePlanFactory::from_trusted_contract(trusted)
        .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    let guard_planner = crate::ScalableGuardPlanner::from_trusted_contract(trusted)
        .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    checkpoint
        .formal_ladders
        .iter()
        .map(|ladder| {
            let graph_profile = parse_graph_profile(&ladder.graph_profile)?;
            if ladder.schema != crate::FORMAL_LADDER_EXECUTION_SCHEMA
                || ladder.schema_version != crate::FORMAL_LADDER_EXECUTION_SCHEMA_VERSION
                || ladder.candidate_id != crate::BASELINE_CANDIDATE_ID
                || ladder.workload_revision != crate::WORKLOAD_REVISION_V1
                || ladder.string_profile != crate::BASE_SCALE_STRING_PROFILE
                || ladder.generator_version != crate::GENERATOR_VERSION_V1
                || ladder.b == 0
                || ladder.disposition != crate::FormalLadderExecutionDisposition::Complete
                || ladder.terminal_guard_preflight.is_some()
                || ladder.levels.len() != crate::FORMAL_LADDER_MINIMUM_LEVEL_COUNT
                || ladder.levels.iter().any(|level| !level.complete)
            {
                return Err(EvidenceError::InvalidCheckpoint(format!(
                    "正式阶梯 {} / {} 的执行封套不完整",
                    ladder.workload_id.as_str(),
                    ladder.graph_profile
                )));
            }
            let experiment_ordinal = validate_formal_rotation(ladder)?;
            let mut previous_guard_observation =
                formal_previous_pilot_observation(checkpoint, ladder)?;
            let completed = ladder
                .levels
                .iter()
                .enumerate()
                .map(|(level_index, level)| {
                    let expected_formal_run_count = (crate::FORMAL_LADDER_BATCH_COUNT
                        * crate::FORMAL_LADDER_ROUND_COUNT
                        * 2) as usize;
                    if !level.guard_preflight.allows_child_start
                        || !level.guard_preflight.triggers.is_empty()
                        || level.completed_guard_observation.is_none()
                        || level.formal_runs.len() != expected_formal_run_count
                    {
                        return Err(EvidenceError::InvalidCheckpoint(format!(
                            "正式阶梯 N={} 的级别封套不完整",
                            level.n
                        )));
                    }
                    validate_formal_guard_preflight(
                        &guard_planner,
                        ladder,
                        level,
                        graph_profile,
                        previous_guard_observation,
                        checkpoint.environment.physical_memory_bytes,
                    )?;
                    let plan = plans
                        .plan(ladder.workload_id, graph_profile, level.n)
                        .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
                    if level.primary_record_count != plan.primary_record_count
                        || level.canonical_lir_record_count
                            != plan.stages.canonical_lir.record_count
                    {
                        return Err(EvidenceError::InvalidCheckpoint(format!(
                            "正式阶梯 N={} 的工作负载计数无法独立复算",
                            level.n
                        )));
                    }
                    let expected_digest =
                        validate_formal_level_preflight(checkpoint, ladder, level, graph_profile)?;
                    let oracle = level.oracle.as_ref().ok_or_else(|| {
                        EvidenceError::InvalidCheckpoint(format!(
                            "正式阶梯 N={} 缺少预言机运行",
                            level.n
                        ))
                    })?;
                    let oracle_child = oracle.child.as_ref().ok_or_else(|| {
                        EvidenceError::InvalidCheckpoint(format!(
                            "正式阶梯 N={} 缺少预言机报告",
                            level.n
                        ))
                    })?;
                    let expected_oracle_run_id = format!(
                        "formal/{}/{}/n-{}/oracle",
                        ladder.workload_id.as_str(),
                        ladder.graph_profile,
                        level.n
                    );
                    validate_formal_oracle_envelope(
                        checkpoint,
                        ladder,
                        level,
                        oracle,
                        oracle_child,
                        graph_profile,
                        &expected_oracle_run_id,
                        &expected_digest,
                    )?;
                    let independent_oracle = crate::verify_scalable_oracle_child(
                        trusted,
                        oracle_child.oracle_run_id.clone(),
                        ladder.workload_id,
                        graph_profile,
                        level.n,
                        oracle_child.controlled_allocation_hard_ceiling_bytes,
                    )
                    .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
                    if independent_oracle.outcome != oracle_child.outcome
                        || independent_oracle.primary_record_count
                            != oracle_child.primary_record_count
                        || independent_oracle.semantic_digest_sha256
                            != oracle_child.semantic_digest_sha256
                        || !oracle_child.complete_counts_equal
                        || !oracle_child.complete_typed_output_equal
                    {
                        return Err(EvidenceError::InvalidCheckpoint(format!(
                            "正式阶梯 N={} 的预言机结果无法独立复算",
                            level.n
                        )));
                    }
                    let completed_guard_observation =
                        validate_completed_guard_observation(level, oracle_child)?;
                    previous_guard_observation = Some(completed_guard_observation);
                    let runs = level
                        .formal_runs
                        .iter()
                        .map(|run| {
                            validate_formal_round_envelope(
                                checkpoint,
                                ladder,
                                level,
                                run,
                                &expected_digest,
                                experiment_ordinal,
                                level_index,
                            )
                        })
                        .collect::<Result<Vec<_>, EvidenceError>>()?;
                    Ok(FormalLadderCompletedLevel {
                        workload_id: ladder.workload_id,
                        graph_profile: ladder.graph_profile.clone(),
                        n: level.n,
                        primary_record_count: level.primary_record_count,
                        canonical_lir_record_count: level.canonical_lir_record_count,
                        runs,
                    })
                })
                .collect::<Result<Vec<_>, EvidenceError>>()?;
            let analysis = analyze_formal_ladder(&completed)
                .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
            if ladder.analysis.as_ref() != Some(&analysis) {
                return Err(EvidenceError::InvalidCheckpoint(format!(
                    "正式阶梯 {} / {} 的分析结果无法独立复算",
                    ladder.workload_id.as_str(),
                    ladder.graph_profile
                )));
            }
            Ok(json!({
                "candidateId": ladder.candidate_id,
                "workloadId": ladder.workload_id,
                "graphProfile": ladder.graph_profile,
                "b": ladder.b,
                "disposition": ladder.disposition,
                "completedLevels": completed.iter().map(|level| json!({
                    "n": level.n,
                    "primaryRecordCount": level.primary_record_count,
                    "canonicalLirRecordCount": level.canonical_lir_record_count
                })).collect::<Vec<_>>(),
                "analysis": analysis
            }))
        })
        .collect()
}

fn validate_formal_ladder_base_scales(
    checkpoint: &FormalProtocolCheckpoint,
) -> Result<(), EvidenceError> {
    let mut selected = BTreeMap::new();
    for selection in &checkpoint.base_scale_pilot.selections {
        let graph_profile = parse_graph_profile(&selection.graph_profile)?;
        let b = selection.b.value.ok_or_else(|| {
            EvidenceError::InvalidCheckpoint(format!(
                "基础规模 {} / {} 没有可靠 B",
                selection.workload_id.as_str(),
                selection.graph_profile
            ))
        })?;
        if selection.b.reason.is_some()
            || selected
                .insert((selection.workload_id, graph_profile), b)
                .is_some()
        {
            return Err(EvidenceError::InvalidCheckpoint(
                "基础规模自然身份重复或 B 无效".to_owned(),
            ));
        }
    }

    let mut bound = BTreeSet::new();
    for ladder in &checkpoint.formal_ladders {
        let graph_profile = parse_graph_profile(&ladder.graph_profile)?;
        let identity = (ladder.workload_id, graph_profile);
        let expected_b = selected.get(&identity).ok_or_else(|| {
            EvidenceError::InvalidCheckpoint(format!(
                "正式阶梯 {} / {} 没有对应基础规模",
                ladder.workload_id.as_str(),
                ladder.graph_profile
            ))
        })?;
        if !bound.insert(identity)
            || ladder.b != *expected_b
            || ladder.levels.first().map(|level| level.n) != Some(*expected_b)
        {
            return Err(EvidenceError::InvalidCheckpoint(format!(
                "正式阶梯 {} / {} 未绑定选定 B 与首级",
                ladder.workload_id.as_str(),
                ladder.graph_profile
            )));
        }
    }
    if bound.len() != selected.len() {
        return Err(EvidenceError::InvalidCheckpoint(
            "正式阶梯没有一一覆盖全部基础规模自然身份".to_owned(),
        ));
    }
    Ok(())
}

fn formal_previous_pilot_observation(
    checkpoint: &FormalProtocolCheckpoint,
    ladder: &crate::FormalLadderExecution,
) -> Result<Option<crate::GuardCompletedLevelObservation>, EvidenceError> {
    let selection = checkpoint
        .base_scale_pilot
        .selections
        .iter()
        .find(|selection| {
            selection.workload_id == ladder.workload_id
                && selection.graph_profile == ladder.graph_profile
        })
        .ok_or_else(|| {
            EvidenceError::InvalidCheckpoint(format!(
                "正式阶梯 {} / {} 缺少基础规模选择",
                ladder.workload_id.as_str(),
                ladder.graph_profile
            ))
        })?;
    let Some(previous_n) = ladder.b.checked_div(2).filter(|value| *value > 0) else {
        return Ok(None);
    };
    selection
        .pilot_levels
        .iter()
        .find(|level| level.n == previous_n)
        .map(|level| Some(level.completed_level_guard_observation))
        .ok_or_else(|| {
            EvidenceError::InvalidCheckpoint(format!(
                "正式阶梯 B={} 缺少前一基础规模完成观察",
                ladder.b
            ))
        })
}

fn validate_formal_rotation(ladder: &crate::FormalLadderExecution) -> Result<u32, EvidenceError> {
    let first_attempt_id = ladder
        .levels
        .first()
        .and_then(|level| level.formal_runs.first())
        .map(|run| run.attempt_id.as_str())
        .ok_or_else(|| EvidenceError::InvalidCheckpoint("正式阶梯缺少轮转运行".to_owned()))?;
    let prefix = format!(
        "formal/{}/{}/experiment-",
        ladder.workload_id.as_str(),
        ladder.graph_profile
    );
    let ordinal_text = first_attempt_id
        .strip_prefix(&prefix)
        .and_then(|remainder| remainder.split_once('/').map(|(ordinal, _)| ordinal))
        .ok_or_else(|| EvidenceError::InvalidCheckpoint("正式阶梯实验身份无效".to_owned()))?;
    let experiment_ordinal = ordinal_text
        .parse::<u32>()
        .ok()
        .filter(|ordinal| ordinal.to_string() == ordinal_text)
        .ok_or_else(|| EvidenceError::InvalidCheckpoint("正式阶梯实验序号无效".to_owned()))?;
    let level_count = ladder.levels.len();

    for (level_index, level) in ladder.levels.iter().enumerate() {
        let mut cursor = 0;
        for batch in 0..crate::FORMAL_LADDER_BATCH_COUNT {
            for round in 0..crate::FORMAL_LADDER_ROUND_COUNT {
                let expected_position =
                    (level_index + level_count - (round as usize % level_count)) % level_count;
                let modes = if round.is_multiple_of(2) {
                    [
                        crate::ScalableLadderBinaryMode::Attribution,
                        crate::ScalableLadderBinaryMode::Timing,
                    ]
                } else {
                    [
                        crate::ScalableLadderBinaryMode::Timing,
                        crate::ScalableLadderBinaryMode::Attribution,
                    ]
                };
                for mode in modes {
                    let run = level.formal_runs.get(cursor).ok_or_else(|| {
                        EvidenceError::InvalidCheckpoint(format!(
                            "正式阶梯 N={} 的轮转运行不完整",
                            level.n
                        ))
                    })?;
                    let mode_token = match mode {
                        crate::ScalableLadderBinaryMode::Timing => "timing",
                        crate::ScalableLadderBinaryMode::Attribution => "attribution",
                    };
                    let expected_attempt_id = format!(
                        "{prefix}{experiment_ordinal}/batch-{batch}/round-{round}/{mode_token}/attempt-0"
                    );
                    if run.batch != batch
                        || run.round != round
                        || usize::try_from(run.execution_position).ok() != Some(expected_position)
                        || run.binary_mode != mode
                        || run.attempt_id != expected_attempt_id
                    {
                        return Err(EvidenceError::InvalidCheckpoint(format!(
                            "正式阶梯 N={} 未遵守五级平衡轮转",
                            level.n
                        )));
                    }
                    cursor += 1;
                }
            }
        }
        if cursor != level.formal_runs.len() {
            return Err(EvidenceError::InvalidCheckpoint(format!(
                "正式阶梯 N={} 含轮转之外的运行",
                level.n
            )));
        }
    }
    Ok(experiment_ordinal)
}

fn validate_formal_guard_preflight(
    guard_planner: &crate::ScalableGuardPlanner,
    ladder: &crate::FormalLadderExecution,
    level: &crate::FormalLadderLevelExecution,
    graph_profile: crate::GraphProfileId,
    previous: Option<crate::GuardCompletedLevelObservation>,
    expected_physical_memory_bytes: u64,
) -> Result<(), EvidenceError> {
    if level
        .guard_preflight
        .memory_observation
        .physical_memory_bytes
        != expected_physical_memory_bytes
    {
        return Err(EvidenceError::InvalidCheckpoint(format!(
            "正式阶梯 N={} 的物理内存观察与环境不一致",
            level.n
        )));
    }
    let expected = guard_planner
        .evaluate(
            ladder.workload_id,
            graph_profile,
            level.n,
            level.guard_preflight.memory_observation,
            previous,
        )
        .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    if level.guard_preflight != expected {
        return Err(EvidenceError::InvalidCheckpoint(format!(
            "正式阶梯 N={} 的护栏预检无法从前级观察复算",
            level.n
        )));
    }
    Ok(())
}

fn validate_formal_level_preflight(
    checkpoint: &FormalProtocolCheckpoint,
    ladder: &crate::FormalLadderExecution,
    level: &crate::FormalLadderLevelExecution,
    graph_profile: crate::GraphProfileId,
) -> Result<String, EvidenceError> {
    let attribution = level.attribution_preflight.as_ref().ok_or_else(|| {
        EvidenceError::InvalidCheckpoint(format!("正式阶梯 N={} 缺少归因预检运行", level.n))
    })?;
    let attribution_child = attribution.child.as_ref().ok_or_else(|| {
        EvidenceError::InvalidCheckpoint(format!("正式阶梯 N={} 缺少归因预检报告", level.n))
    })?;
    let expected_attribution_id = format!(
        "formal/{}/{}/n-{}/attribution-preflight",
        ladder.workload_id.as_str(),
        ladder.graph_profile,
        level.n
    );
    if attribution.run_id != expected_attribution_id
        || attribution.status != RunStatus::Valid
        || !attribution.invalidation_reasons.is_empty()
        || attribution.kill_error.is_some()
        || attribution.monitor_error.is_some()
        || attribution_child.outcome != crate::ScalableAttributionOutcome::Success
        || !external_state_is_clear(attribution.external_state.as_ref(), checkpoint)
        || !crate::pilot_budget::successful_process(
            &attribution.process,
            crate::ATTRIBUTION_BINARY_ID,
            attribution_child.child_pid,
        )
    {
        return Err(EvidenceError::InvalidCheckpoint(format!(
            "正式阶梯 N={} 的归因预检进程封套无效",
            level.n
        )));
    }
    crate::ladder_runner::validate_attribution_preflight(
        attribution_child,
        &attribution.compiler_instance_id,
        ladder.workload_id,
        graph_profile,
        level.n,
        level.guard_preflight.thresholds.compiler_controlled_bytes,
    )
    .map_err(EvidenceError::InvalidCheckpoint)?;
    crate::pilot_budget::validate_clear_monitor(
        Some(&attribution.monitor),
        &level.guard_preflight,
        None,
        attribution.kill_error.as_deref(),
        attribution.monitor_error.as_deref(),
        &attribution.run_id,
    )
    .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    let expected_digest = attribution_child
        .semantic_digest_sha256
        .as_deref()
        .ok_or_else(|| {
            EvidenceError::InvalidCheckpoint(format!(
                "正式阶梯 N={} 的归因预检缺少语义摘要",
                level.n
            ))
        })?
        .to_owned();

    let timing = level.timing_guard_run.as_ref().ok_or_else(|| {
        EvidenceError::InvalidCheckpoint(format!("正式阶梯 N={} 缺少时延护栏运行", level.n))
    })?;
    let timing_child = timing.child.as_ref().ok_or_else(|| {
        EvidenceError::InvalidCheckpoint(format!("正式阶梯 N={} 缺少时延护栏报告", level.n))
    })?;
    let expected_timing_id = format!(
        "formal/{}/{}/n-{}/timing-guard-observation",
        ladder.workload_id.as_str(),
        ladder.graph_profile,
        level.n
    );
    if timing.run_id != expected_timing_id
        || timing.status != RunStatus::Valid
        || !timing.invalidation_reasons.is_empty()
        || timing.kill_error.is_some()
        || timing.monitor_error.is_some()
        || timing_child.outcome != crate::ScalableLadderOutcome::Success
        || !external_state_is_clear(timing.external_state.as_ref(), checkpoint)
        || !crate::pilot_budget::successful_process(
            &timing.process,
            crate::TIMING_BINARY_ID,
            timing_child.child_pid,
        )
    {
        return Err(EvidenceError::InvalidCheckpoint(format!(
            "正式阶梯 N={} 的时延护栏进程封套无效",
            level.n
        )));
    }
    crate::ladder_runner::validate_ladder_report(
        timing_child,
        &timing.compiler_instance_id,
        ladder.workload_id,
        &ladder.graph_profile,
        level.n,
        level.guard_preflight.thresholds.compiler_controlled_bytes,
        crate::ScalableLadderBinaryMode::Timing,
        &expected_digest,
    )
    .map_err(EvidenceError::InvalidCheckpoint)?;
    let child_wall_time = timing_child
        .cold_instance
        .as_ref()
        .and_then(|sample| sample.wall_time_ns);
    crate::pilot_budget::validate_clear_monitor(
        Some(&timing.monitor),
        &level.guard_preflight,
        child_wall_time,
        timing.kill_error.as_deref(),
        timing.monitor_error.as_deref(),
        &timing.run_id,
    )
    .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    Ok(expected_digest)
}

fn validate_completed_guard_observation(
    level: &crate::FormalLadderLevelExecution,
    oracle_child: &crate::ScalableOracleChildReport,
) -> Result<crate::GuardCompletedLevelObservation, EvidenceError> {
    let attribution = level
        .attribution_preflight
        .as_ref()
        .and_then(|run| run.child.as_ref())
        .expect("validated attribution preflight");
    let timing = level
        .timing_guard_run
        .as_ref()
        .expect("validated timing guard");
    let observation = level
        .completed_guard_observation
        .as_ref()
        .expect("validated completed guard observation");
    let expected_peak = [
        attribution.guard_peak_live_requested_bytes,
        oracle_child.guard_peak_live_requested_bytes,
    ]
    .into_iter()
    .flatten()
    .max();
    let expected_private = [
        level
            .attribution_preflight
            .as_ref()
            .and_then(|run| run.monitor.peak_private_bytes.value),
        timing.monitor.peak_private_bytes.value,
        level
            .oracle
            .as_ref()
            .and_then(|run| run.monitor.peak_private_bytes.value),
    ]
    .into_iter()
    .flatten()
    .max();
    let expected = crate::GuardCompletedLevelObservation {
        n: level.n,
        primary_record_count: level.primary_record_count,
        peak_live_requested_bytes: expected_peak.ok_or_else(|| {
            EvidenceError::InvalidCheckpoint(format!("正式阶梯 N={} 缺少受控分配峰值", level.n))
        })?,
        private_bytes: expected_private.ok_or_else(|| {
            EvidenceError::InvalidCheckpoint(format!("正式阶梯 N={} 缺少私有字节峰值", level.n))
        })?,
        wall_time_ns: timing.monitor.elapsed_wall_time_ns,
    };
    if *observation != expected {
        return Err(EvidenceError::InvalidCheckpoint(format!(
            "正式阶梯 N={} 的完成护栏观察无法复算",
            level.n
        )));
    }
    Ok(expected)
}

#[allow(clippy::too_many_arguments)]
fn validate_formal_oracle_envelope(
    checkpoint: &FormalProtocolCheckpoint,
    ladder: &crate::FormalLadderExecution,
    level: &crate::FormalLadderLevelExecution,
    oracle: &crate::FormalOracleRun,
    child: &crate::ScalableOracleChildReport,
    graph_profile: crate::GraphProfileId,
    expected_run_id: &str,
    expected_digest: &str,
) -> Result<(), EvidenceError> {
    if oracle.run_id != expected_run_id
        || oracle.status != RunStatus::Valid
        || !oracle.invalidation_reasons.is_empty()
        || oracle.kill_error.is_some()
        || oracle.monitor_error.is_some()
        || !external_state_is_clear(oracle.external_state.as_ref(), checkpoint)
        || !crate::pilot_budget::successful_process(
            &oracle.process,
            crate::ORACLE_BINARY_ID,
            child.child_pid,
        )
    {
        return Err(EvidenceError::InvalidCheckpoint(format!(
            "正式阶梯 N={} 的预言机进程封套无效",
            level.n
        )));
    }
    crate::ladder_runner::validate_oracle(
        child,
        expected_run_id,
        ladder.workload_id,
        graph_profile,
        level.n,
        level.guard_preflight.thresholds.compiler_controlled_bytes,
        level.primary_record_count,
        expected_digest,
    )
    .map_err(EvidenceError::InvalidCheckpoint)?;
    crate::pilot_budget::validate_clear_monitor(
        Some(&oracle.monitor),
        &level.guard_preflight,
        None,
        oracle.kill_error.as_deref(),
        oracle.monitor_error.as_deref(),
        expected_run_id,
    )
    .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    Ok(())
}

fn validate_formal_round_envelope(
    checkpoint: &FormalProtocolCheckpoint,
    ladder: &crate::FormalLadderExecution,
    level: &crate::FormalLadderLevelExecution,
    run: &crate::FormalLadderProcessRun,
    expected_digest: &str,
    experiment_ordinal: u32,
    level_index: usize,
) -> Result<FormalLadderRoundRun, EvidenceError> {
    let child = run.child.as_ref().ok_or_else(|| {
        EvidenceError::InvalidCheckpoint(format!("正式阶梯 N={} 缺少子进程报告", level.n))
    })?;
    let (mode_token, expected_binary_id) = match run.binary_mode {
        crate::ScalableLadderBinaryMode::Timing => ("timing", crate::TIMING_BINARY_ID),
        crate::ScalableLadderBinaryMode::Attribution => {
            ("attribution", crate::ATTRIBUTION_BINARY_ID)
        }
    };
    let expected_attempt_id = format!(
        "formal/{}/{}/experiment-{experiment_ordinal}/batch-{}/round-{}/{mode_token}/attempt-0",
        ladder.workload_id.as_str(),
        ladder.graph_profile,
        run.batch,
        run.round
    );
    let level_count = ladder.levels.len();
    let expected_position =
        (level_index + level_count - (run.round as usize % level_count)) % level_count;
    let expected_run_id = format!("{}/n-{}/run", run.attempt_id, level.n);
    let expected_compiler_instance_id = format!("{expected_run_id}/compiler-instance");
    let child_wall_time = (run.binary_mode == crate::ScalableLadderBinaryMode::Timing)
        .then(|| {
            child
                .cold_instance
                .as_ref()
                .and_then(|sample| sample.wall_time_ns)
        })
        .flatten();
    if run.status != RunStatus::Valid
        || !run.invalidation_reasons.is_empty()
        || run.retry_ordinal != 0
        || run.batch >= crate::FORMAL_LADDER_BATCH_COUNT
        || run.round >= crate::FORMAL_LADDER_ROUND_COUNT
        || usize::try_from(run.execution_position).ok() != Some(expected_position)
        || run.attempt_id != expected_attempt_id
        || run.run_id != expected_run_id
        || run.compiler_instance_id != expected_compiler_instance_id
        || run.kill_error.is_some()
        || run.monitor_error.is_some()
        || !external_state_is_clear(run.external_state.as_ref(), checkpoint)
        || !crate::pilot_budget::successful_process(
            &run.process,
            expected_binary_id,
            child.child_pid,
        )
    {
        return Err(EvidenceError::InvalidCheckpoint(format!(
            "正式阶梯运行 {} 的进程封套无效",
            run.run_id
        )));
    }
    crate::ladder_runner::validate_ladder_report(
        child,
        &run.compiler_instance_id,
        ladder.workload_id,
        &ladder.graph_profile,
        level.n,
        level.guard_preflight.thresholds.compiler_controlled_bytes,
        run.binary_mode,
        expected_digest,
    )
    .map_err(EvidenceError::InvalidCheckpoint)?;
    crate::pilot_budget::validate_clear_monitor(
        Some(&run.monitor),
        &level.guard_preflight,
        child_wall_time,
        run.kill_error.as_deref(),
        run.monitor_error.as_deref(),
        &run.run_id,
    )
    .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    Ok(FormalLadderRoundRun {
        batch: run.batch,
        round: run.round,
        binary_mode: run.binary_mode,
        report: child.clone(),
    })
}

fn external_state_is_clear(
    observation: Option<&crate::ExternalStateObservation>,
    checkpoint: &FormalProtocolCheckpoint,
) -> bool {
    observation.is_some_and(|observation| {
        observation
            .invalidation_reasons(&checkpoint.environment)
            .is_empty()
    })
}

#[derive(Clone)]
struct BudgetBasis {
    metric: String,
    stratum: Value,
    batch_zero_id: String,
    batch_one_id: String,
    observed_upper: u64,
    repeat_ratio: Ratio,
}

fn derive_budgets(
    analyses: &[Value],
    clock_quantum_ns: u64,
) -> Result<(Vec<Value>, Vec<Value>), EvidenceError> {
    let mut bases = Vec::new();
    let mut envelope_by_metric = BTreeMap::<String, Ratio>::new();
    for ladder in analyses {
        let analysis: FormalLadderAnalysis = serde_json::from_value(ladder["analysis"].clone())?;
        let mut pairs = BTreeMap::<String, [Option<&FormalLadderBatchSummary>; 2]>::new();
        for summary in &analysis.batch_summaries {
            let batch = usize::try_from(summary.batch).expect("batch fits usize");
            if batch > 1 {
                return Err(EvidenceError::InvalidCheckpoint(
                    "正式批次编号超界".to_owned(),
                ));
            }
            let key = batch_key(summary)?;
            if pairs.entry(key).or_default()[batch]
                .replace(summary)
                .is_some()
            {
                return Err(EvidenceError::InvalidCheckpoint(
                    "正式批次汇总重复".to_owned(),
                ));
            }
        }
        for pair in pairs.into_values() {
            let [Some(zero), Some(one)] = pair else {
                return Err(EvidenceError::InvalidCheckpoint(
                    "正式批次汇总不完整".to_owned(),
                ));
            };
            let metric = token(&zero.metric)?;
            let ratio = Ratio::new(zero.median.max(one.median), zero.median.min(one.median))?;
            envelope_by_metric
                .entry(metric.clone())
                .and_modify(|current| {
                    if ratio.cmp(current) == Ordering::Greater {
                        *current = ratio;
                    }
                })
                .or_insert(ratio);
            let observed_upper = analysis
                .round_summaries
                .iter()
                .filter(|round| {
                    round.workload_id == zero.workload_id
                        && round.graph_profile == zero.graph_profile
                        && round.n == zero.n
                        && round.metric == zero.metric
                        && round.sample_kind == zero.sample_kind
                        && round.binary_mode == zero.binary_mode
                })
                .map(|round| round.median)
                .max()
                .ok_or_else(|| EvidenceError::InvalidCheckpoint("预算缺少轮次汇总".to_owned()))?;
            bases.push(BudgetBasis {
                metric,
                stratum: json!({
                    "workloadId": zero.workload_id,
                    "graphProfile": zero.graph_profile,
                    "n": zero.n,
                    "sampleKind": zero.sample_kind,
                    "binaryMode": zero.binary_mode
                }),
                batch_zero_id: zero.summary_id.clone(),
                batch_one_id: one.summary_id.clone(),
                observed_upper,
                repeat_ratio: ratio,
            });
        }
    }
    let envelopes = envelope_by_metric
        .iter()
        .map(|(metric, ratio)| json!({"metric": metric, "repeatRatio": ratio.json()}))
        .collect::<Vec<_>>();
    let budgets = bases
        .into_iter()
        .map(|basis| {
            let envelope = envelope_by_metric[&basis.metric];
            let quantum = if basis.metric == "wall-time-ns" {
                clock_quantum_ns
            } else {
                1
            };
            let value = ceil_ratio_to_quantum(basis.observed_upper, envelope, quantum)?;
            Ok(json!({
                "recommendationKind": "r0-budget-v1",
                "stratum": basis.stratum,
                "metric": basis.metric,
                "batch0SummaryId": basis.batch_zero_id,
                "batch1SummaryId": basis.batch_one_id,
                "batchRepeatRatio": basis.repeat_ratio.json(),
                "reproducibilityEnvelope": envelope.json(),
                "observedUpper": basis.observed_upper,
                "roundingQuantum": quantum,
                "value": value,
                "unit": if basis.metric == "wall-time-ns" { "nanosecond" } else { "byte" },
                "scope": "R0 research input for #292; not product SLA"
            }))
        })
        .collect::<Result<Vec<_>, EvidenceError>>()?;
    Ok((envelopes, budgets))
}

fn candidate_results(
    trusted: &TrustedContract,
    checkpoint: &FormalProtocolCheckpoint,
    matrix: &crate::CandidateMatrixExecutionBundle,
    envelopes: &[Value],
) -> Result<Vec<Value>, EvidenceError> {
    let wall_envelope_value = envelopes
        .iter()
        .find(|entry| entry["metric"] == "wall-time-ns")
        .ok_or_else(|| EvidenceError::InvalidCheckpoint("缺少墙钟复现包络".to_owned()))?;
    let wall_envelope = Ratio::from_json(&wall_envelope_value["repeatRatio"])?;
    validate_candidate_matrix_envelope(trusted, checkpoint, matrix)?;
    let expected_safety = crate::candidate_matrix::expected_candidate_safety_assessments(trusted)
        .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    if matrix.safety_audit.tool != "cargo-deny 0.20.2"
        || matrix.safety_audit.advisory_error_count != 0
        || matrix.safety_audit.license_error_count != 0
        || matrix.safety_audit.source_error_count != 0
        || matrix.safety_audit.ban_error_count != 0
        || matrix.safety_audit.assessments != expected_safety
    {
        return Err(EvidenceError::InvalidCheckpoint(
            "候选安全审计不能绑定冻结注册表".to_owned(),
        ));
    }
    let constant_hash_qualifications = validate_constant_hash_qualifications(checkpoint, matrix)?;
    matrix
        .executions
        .iter()
        .map(|execution| {
            let validated = validate_candidate_execution(
                trusted,
                checkpoint,
                &expected_safety,
                &constant_hash_qualifications,
                execution,
            )?;
            let comparisons = execution
                .roster
                .entries
                .iter()
                .filter(|entry| entry.disposition == CandidateDisposition::PerformanceParticipant)
                .map(|entry| {
                    let zero = candidate_batch_ratio(&validated, &entry.candidate_id, 0)?;
                    let one = candidate_batch_ratio(&validated, &entry.candidate_id, 1)?;
                    let decision = classify_candidate([zero, one], wall_envelope)?;
                    Ok(json!({
                        "candidateId": entry.candidate_id,
                        "batch0MedianRatio": zero.json(),
                        "batch1MedianRatio": one.json(),
                        "decision": decision
                    }))
                })
                .collect::<Result<Vec<_>, EvidenceError>>()?;
            Ok(json!({
                "stratum": execution.stratum,
                "baselineId": execution.roster.baseline_id,
                "rosterEntries": execution.roster.entries,
                "comparisons": comparisons
            }))
        })
        .collect()
}

fn validate_candidate_matrix_envelope(
    trusted: &TrustedContract,
    checkpoint: &FormalProtocolCheckpoint,
    matrix: &crate::CandidateMatrixExecutionBundle,
) -> Result<(), EvidenceError> {
    let expected_scope = crate::CandidatePerformanceScopeContract::from_trusted_contract(trusted)
        .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    let expected_scales =
        crate::resolve_candidate_performance_scales(&expected_scope, &checkpoint.formal_ladders)
            .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    let mut expected_strata = Vec::with_capacity(expected_scales.len() * 3);
    for scale in &expected_scales {
        for key_domain in [
            crate::CandidateKeyDomain::ExternalString,
            crate::CandidateKeyDomain::ValidatedFixedKey,
            crate::CandidateKeyDomain::CanonicalOutputOrder,
        ] {
            expected_strata.push(
                crate::CandidatePipelineStratum::from_scope(&expected_scope, key_domain, *scale)
                    .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?,
            );
        }
    }
    let actual_strata = matrix
        .executions
        .iter()
        .map(|execution| execution.stratum.clone())
        .collect::<Vec<_>>();
    if matrix.schema != crate::CANDIDATE_MATRIX_CHECKPOINT_SCHEMA
        || matrix.schema_version != crate::CANDIDATE_MATRIX_CHECKPOINT_SCHEMA_VERSION
        || matrix.scope != expected_scope
        || matrix.scales != expected_scales
        || matrix.active_execution.is_some()
        || matrix
            .executions
            .iter()
            .any(|execution| !execution.complete)
        || actual_strata != expected_strata
    {
        return Err(EvidenceError::InvalidCheckpoint(
            "候选矩阵没有精确覆盖三个规模与三个键域".to_owned(),
        ));
    }
    Ok(())
}

fn validate_constant_hash_qualifications(
    checkpoint: &FormalProtocolCheckpoint,
    matrix: &crate::CandidateMatrixExecutionBundle,
) -> Result<Vec<crate::ConstantHashQualification>, EvidenceError> {
    let thresholds = crate::GuardThresholds::from_physical_memory_bytes(
        checkpoint.environment.physical_memory_bytes,
    )
    .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    let mut executions = BTreeMap::new();
    for execution in &matrix.constant_hash_qualifications {
        if executions
            .insert(execution.qualification.candidate_id.as_str(), execution)
            .is_some()
        {
            return Err(EvidenceError::InvalidCheckpoint(
                "常量哈希资格身份重复".to_owned(),
            ));
        }
    }
    if executions.len() != crate::candidate_matrix::FAST_HASH_CANDIDATES.len() {
        return Err(EvidenceError::InvalidCheckpoint(
            "常量哈希资格集合不完整".to_owned(),
        ));
    }

    crate::candidate_matrix::FAST_HASH_CANDIDATES
        .iter()
        .map(|candidate_id| {
            let execution = executions.get(candidate_id).copied().ok_or_else(|| {
                EvidenceError::InvalidCheckpoint(format!("候选 {candidate_id} 缺少常量哈希资格"))
            })?;
            let mut observations = Vec::with_capacity(6);
            let mut cursor = 0;
            for input_variant_id in [
                "constant-hash-canonical-valid-v1",
                "constant-hash-missing-reference-v1",
            ] {
                for repeat in 0..2_u32 {
                    observations.push(validate_constant_hash_run(
                        checkpoint,
                        &execution.runs,
                        cursor,
                        candidate_id,
                        input_variant_id,
                        crate::ConstantHashRole::CandidateUnderTest,
                        repeat,
                        crate::TIMING_BINARY_ID,
                        thresholds,
                    )?);
                    cursor += 1;
                }
                observations.push(validate_constant_hash_run(
                    checkpoint,
                    &execution.runs,
                    cursor,
                    candidate_id,
                    input_variant_id,
                    crate::ConstantHashRole::ExactResearchOracle,
                    0,
                    crate::ORACLE_BINARY_ID,
                    thresholds,
                )?);
                cursor += 1;
            }
            if cursor != execution.runs.len() {
                return Err(EvidenceError::InvalidCheckpoint(format!(
                    "候选 {candidate_id} 含资格协议之外的运行"
                )));
            }
            let rebuilt = crate::candidate_matrix::build_constant_hash_qualification(
                candidate_id,
                observations,
            );
            if execution.qualification != rebuilt {
                return Err(EvidenceError::InvalidCheckpoint(format!(
                    "候选 {candidate_id} 的常量哈希资格无法从运行重建"
                )));
            }
            Ok(rebuilt)
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn validate_constant_hash_run(
    checkpoint: &FormalProtocolCheckpoint,
    runs: &[crate::ConstantHashProcessRun],
    cursor: usize,
    candidate_id: &str,
    input_variant_id: &str,
    role: crate::ConstantHashRole,
    repeat: u32,
    expected_binary_id: &str,
    thresholds: crate::GuardThresholds,
) -> Result<crate::ConstantHashObservation, EvidenceError> {
    let run = runs.get(cursor).ok_or_else(|| {
        EvidenceError::InvalidCheckpoint(format!("候选 {candidate_id} 的常量哈希运行不完整"))
    })?;
    let child = run.child.as_ref().ok_or_else(|| {
        EvidenceError::InvalidCheckpoint(format!("候选 {candidate_id} 缺少常量哈希子报告"))
    })?;
    let role_id = match role {
        crate::ConstantHashRole::CandidateUnderTest => "candidate",
        crate::ConstantHashRole::ExactResearchOracle => "oracle",
    };
    let expected_run_id =
        format!("constant-hash/{candidate_id}/{input_variant_id}/{role_id}/repeat-{repeat}");
    if run.run_id != expected_run_id
        || run.status != RunStatus::Valid
        || !run.invalidation_reasons.is_empty()
        || run.kill_error.is_some()
        || run.monitor_error.is_some()
        || !external_state_is_clear(run.external_state.as_ref(), checkpoint)
        || !crate::pilot_budget::successful_process(
            &run.process,
            expected_binary_id,
            child.child_pid,
        )
    {
        return Err(EvidenceError::InvalidCheckpoint(format!(
            "候选 {candidate_id} 的常量哈希进程封套无效"
        )));
    }
    crate::candidate_matrix::validate_constant_hash_child_report(
        child,
        expected_binary_id,
        candidate_id,
        input_variant_id,
        role,
        repeat,
    )
    .map_err(EvidenceError::InvalidCheckpoint)?;
    crate::pilot_budget::validate_clear_monitor_with_thresholds(
        Some(&run.monitor),
        &thresholds,
        None,
        run.kill_error.as_deref(),
        run.monitor_error.as_deref(),
        &run.run_id,
    )
    .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    Ok(child.observation.clone())
}

struct ValidatedCandidateExecution<'a> {
    execution: &'a crate::CandidatePipelineExecution,
    samples: BTreeMap<(u32, u32, &'a str), &'a crate::CandidatePipelinePerformanceSample>,
    participant_count: usize,
}

struct CandidateQualificationBinding {
    semantic_digest_sha256: String,
    baseline_checksums: crate::CandidatePipelineChecksums,
}

impl<'a> ValidatedCandidateExecution<'a> {
    fn sample(
        &self,
        batch: u32,
        round: u32,
        candidate_id: &str,
    ) -> Result<&'a crate::CandidatePipelinePerformanceSample, EvidenceError> {
        self.samples
            .get(&(batch, round, candidate_id))
            .copied()
            .ok_or_else(|| EvidenceError::InvalidCheckpoint("候选平衡轮次缺少样本".to_owned()))
    }
}

fn validate_candidate_execution<'a>(
    trusted: &TrustedContract,
    checkpoint: &FormalProtocolCheckpoint,
    expected_safety: &[crate::CandidateSafetyAssessment],
    constant_hash_qualifications: &[crate::ConstantHashQualification],
    execution: &'a crate::CandidatePipelineExecution,
) -> Result<ValidatedCandidateExecution<'a>, EvidenceError> {
    if execution.roster.stratum != execution.stratum {
        return Err(EvidenceError::InvalidCheckpoint(
            "候选参赛名单与执行分层不一致".to_owned(),
        ));
    }
    let qualification = validate_candidate_qualification_runs(trusted, checkpoint, execution)?;
    let registry = crate::CandidateRegistry::from_trusted_contract(trusted)
        .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    let available_candidate_ids = registry
        .candidates_for(execution.stratum.key_domain)
        .map(|candidate| candidate.id.clone())
        .collect::<BTreeSet<_>>();
    let rebuilt_roster = crate::candidate_matrix::rebuild_pipeline_candidate_roster(
        trusted,
        execution.stratum.clone(),
        expected_safety,
        constant_hash_qualifications,
        &available_candidate_ids,
        execution.roster.oracle_run.clone(),
        execution.roster.candidate_runs.clone(),
    )
    .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    if execution.roster != rebuilt_roster {
        return Err(EvidenceError::InvalidCheckpoint(
            "候选参赛名单无法从冻结注册表与资格运行重建".to_owned(),
        ));
    }
    let participants = execution.roster.participant_ids();
    let participant_count = participants.len();
    let expected_schedule = crate::build_two_batch_balanced_schedule(&participants)
        .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    if execution.schedule != expected_schedule {
        return Err(EvidenceError::InvalidCheckpoint(
            "候选平衡顺序无法从参赛名单重算".to_owned(),
        ));
    }
    let thresholds = crate::GuardThresholds::from_physical_memory_bytes(
        checkpoint.environment.physical_memory_bytes,
    )
    .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;

    let mut attempts = BTreeMap::new();
    for attempt in &execution.attempts {
        if attempts.insert(attempt.run_id.as_str(), attempt).is_some() {
            return Err(EvidenceError::InvalidCheckpoint(
                "候选原始尝试运行 ID 重复".to_owned(),
            ));
        }
    }
    let mut samples = BTreeMap::new();
    let mut referenced_attempts = BTreeSet::new();
    for sample in &execution.samples {
        let key = (sample.batch, sample.round, sample.candidate_id.as_str());
        if samples.insert(key, sample).is_some() {
            return Err(EvidenceError::InvalidCheckpoint(
                "候选平衡轮次样本重复".to_owned(),
            ));
        }
        let attempt = attempts
            .get(sample.run_id.as_str())
            .copied()
            .ok_or_else(|| EvidenceError::InvalidCheckpoint("候选样本缺少原始尝试".to_owned()))?;
        validate_candidate_attempt(
            checkpoint,
            execution,
            sample,
            attempt,
            &qualification,
            thresholds,
        )?;
        referenced_attempts.insert(attempt.run_id.as_str());
    }

    let expected_sample_count = execution
        .schedule
        .len()
        .checked_mul(participant_count)
        .ok_or_else(|| EvidenceError::InvalidCheckpoint("候选样本基数溢出".to_owned()))?;
    if samples.len() != expected_sample_count
        || execution.attempts.iter().any(|attempt| {
            attempt.status == RunStatus::Valid
                && !referenced_attempts.contains(attempt.run_id.as_str())
        })
    {
        return Err(EvidenceError::InvalidCheckpoint(
            "候选有效尝试与正式样本不是一一对应".to_owned(),
        ));
    }

    for scheduled in &execution.schedule {
        let mut round_samples = Vec::with_capacity(participant_count);
        let mut selected_attempt = None::<(&str, u32)>;
        for (position, candidate_id) in scheduled.participant_order.iter().enumerate() {
            let sample = samples
                .get(&(scheduled.batch, scheduled.round, candidate_id.as_str()))
                .copied()
                .ok_or_else(|| {
                    EvidenceError::InvalidCheckpoint("候选平衡轮次缺少样本".to_owned())
                })?;
            if usize::try_from(sample.position).ok() != Some(position) {
                return Err(EvidenceError::InvalidCheckpoint(
                    "候选样本执行位置与平衡顺序不一致".to_owned(),
                ));
            }
            let attempt = attempts
                .get(sample.run_id.as_str())
                .copied()
                .expect("validated candidate attempt");
            match selected_attempt {
                None => {
                    selected_attempt = Some((&attempt.round_attempt_id, attempt.retry_ordinal));
                }
                Some((round_attempt_id, retry_ordinal))
                    if round_attempt_id == attempt.round_attempt_id
                        && retry_ordinal == attempt.retry_ordinal => {}
                Some(_) => {
                    return Err(EvidenceError::InvalidCheckpoint(
                        "候选轮次混用了不同重试尝试".to_owned(),
                    ));
                }
            }
            round_samples.push((*sample).clone());
        }
        crate::candidate_matrix::validate_pipeline_round_semantics(
            &round_samples,
            &execution.roster.baseline_id,
        )
        .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    }

    Ok(ValidatedCandidateExecution {
        execution,
        samples,
        participant_count,
    })
}

fn validate_candidate_qualification_runs(
    trusted: &TrustedContract,
    checkpoint: &FormalProtocolCheckpoint,
    execution: &crate::CandidatePipelineExecution,
) -> Result<CandidateQualificationBinding, EvidenceError> {
    let stratum = execution.stratum.clone();
    let thresholds = crate::GuardThresholds::from_physical_memory_bytes(
        checkpoint.environment.physical_memory_bytes,
    )
    .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    let plans = crate::ScalableStagePlanFactory::from_trusted_contract(trusted)
        .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    let registry = crate::CandidateRegistry::from_trusted_contract(trusted)
        .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    let expected_candidate_ids = registry
        .candidates_for(stratum.key_domain)
        .map(|candidate| candidate.id.as_str())
        .collect::<BTreeSet<_>>();
    let baseline_id = registry
        .baseline_id(stratum.key_domain)
        .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    let plan = plans
        .plan(stratum.workload_id, stratum.graph_profile, stratum.n)
        .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    let oracle = &execution.roster.oracle_run;
    let oracle_child = oracle
        .child
        .as_ref()
        .ok_or_else(|| EvidenceError::InvalidCheckpoint("候选资格缺少精确预言机报告".to_owned()))?;
    let expected_oracle_id = format!(
        "candidate-qualification/{}/{}/{}/{}/n-{}/oracle",
        stratum.scale_role.as_str(),
        stratum.key_domain.as_str(),
        stratum.workload_id.as_str(),
        stratum.graph_profile.as_str(),
        stratum.n
    );
    if oracle.run_id != expected_oracle_id
        || oracle.status != RunStatus::Valid
        || !oracle.invalidation_reasons.is_empty()
        || oracle.kill_error.is_some()
        || oracle.monitor_error.is_some()
        || !external_state_is_clear(oracle.external_state.as_ref(), checkpoint)
        || !crate::pilot_budget::successful_process(
            &oracle.process,
            crate::ORACLE_BINARY_ID,
            oracle_child.child_pid,
        )
    {
        return Err(EvidenceError::InvalidCheckpoint(
            "候选资格精确预言机进程封套无效".to_owned(),
        ));
    }
    crate::candidate_matrix::validate_candidate_oracle_report(
        oracle_child,
        &expected_oracle_id,
        stratum.workload_id,
        stratum.graph_profile,
        stratum.n,
        thresholds.compiler_controlled_bytes,
        plan.primary_record_count,
    )
    .map_err(EvidenceError::InvalidCheckpoint)?;
    crate::pilot_budget::validate_clear_monitor_with_thresholds(
        Some(&oracle.monitor),
        &thresholds,
        None,
        oracle.kill_error.as_deref(),
        oracle.monitor_error.as_deref(),
        &oracle.run_id,
    )
    .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    let independent_oracle = crate::verify_scalable_oracle_child(
        trusted,
        oracle_child.oracle_run_id.clone(),
        stratum.workload_id,
        stratum.graph_profile,
        stratum.n,
        thresholds.compiler_controlled_bytes,
    )
    .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    if independent_oracle.outcome != oracle_child.outcome
        || independent_oracle.primary_record_count != oracle_child.primary_record_count
        || independent_oracle.semantic_digest_sha256 != oracle_child.semantic_digest_sha256
        || !oracle_child.complete_counts_equal
        || !oracle_child.complete_typed_output_equal
    {
        return Err(EvidenceError::InvalidCheckpoint(
            "候选资格精确预言机结果无法独立复算".to_owned(),
        ));
    }

    let mut candidate_ids = BTreeSet::new();
    for run in &execution.roster.candidate_runs {
        if !candidate_ids.insert(run.candidate_id.as_str()) {
            return Err(EvidenceError::InvalidCheckpoint(
                "候选资格运行身份重复".to_owned(),
            ));
        }
        let child = run.child.as_ref().ok_or_else(|| {
            EvidenceError::InvalidCheckpoint(format!(
                "候选 {} 的资格运行缺少子报告",
                run.candidate_id
            ))
        })?;
        let expected_run_id = format!(
            "candidate-qualification/{}/{}/{}/{}/n-{}/candidate-{}",
            stratum.scale_role.as_str(),
            stratum.key_domain.as_str(),
            stratum.workload_id.as_str(),
            stratum.graph_profile.as_str(),
            stratum.n,
            run.candidate_id
        );
        let compiler_instance_id = format!("{expected_run_id}/compiler");
        if run.run_id != expected_run_id
            || run.status != RunStatus::Valid
            || !run.invalidation_reasons.is_empty()
            || run.kill_error.is_some()
            || run.monitor_error.is_some()
            || !external_state_is_clear(run.external_state.as_ref(), checkpoint)
            || !crate::pilot_budget::successful_process(
                &run.process,
                crate::TIMING_BINARY_ID,
                child.child_pid,
            )
        {
            return Err(EvidenceError::InvalidCheckpoint(format!(
                "候选 {} 的资格进程封套无效",
                run.candidate_id
            )));
        }
        crate::candidate_matrix::validate_candidate_pipeline_report(
            child,
            &compiler_instance_id,
            &run.candidate_id,
            stratum.key_domain,
            stratum.workload_id,
            stratum.graph_profile,
            stratum.n,
            thresholds.compiler_controlled_bytes,
        )
        .map_err(EvidenceError::InvalidCheckpoint)?;
        crate::pilot_budget::validate_clear_monitor_with_thresholds(
            Some(&run.monitor),
            &thresholds,
            child.wall_time_ns,
            run.kill_error.as_deref(),
            run.monitor_error.as_deref(),
            &run.run_id,
        )
        .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    }
    if candidate_ids != expected_candidate_ids {
        return Err(EvidenceError::InvalidCheckpoint(
            "候选资格运行没有精确覆盖冻结全特性注册表".to_owned(),
        ));
    }
    let semantic_digest_sha256 = independent_oracle
        .semantic_digest_sha256
        .ok_or_else(|| EvidenceError::InvalidCheckpoint("候选资格预言机缺少语义摘要".to_owned()))?;
    let baseline_checksums = execution
        .roster
        .candidate_runs
        .iter()
        .find(|run| run.candidate_id == baseline_id)
        .and_then(|run| run.child.as_ref())
        .and_then(|child| child.candidate_pipeline_checksums)
        .ok_or_else(|| {
            EvidenceError::InvalidCheckpoint("候选基线资格运行缺少管线校验和".to_owned())
        })?;
    Ok(CandidateQualificationBinding {
        semantic_digest_sha256,
        baseline_checksums,
    })
}

fn validate_candidate_attempt(
    checkpoint: &FormalProtocolCheckpoint,
    execution: &crate::CandidatePipelineExecution,
    sample: &crate::CandidatePipelinePerformanceSample,
    attempt: &crate::CandidatePipelinePerformanceAttempt,
    qualification: &CandidateQualificationBinding,
    thresholds: crate::GuardThresholds,
) -> Result<(), EvidenceError> {
    let child = attempt
        .child
        .as_ref()
        .ok_or_else(|| EvidenceError::InvalidCheckpoint("候选有效尝试缺少子进程报告".to_owned()))?;
    let stratum = &execution.stratum;
    let expected_round_attempt_id = format!(
        "candidate-pipeline/{}/{}/{}/{}/{}/batch-{}/round-{}/attempt-{}",
        stratum.scale_role.as_str(),
        stratum.key_domain.as_str(),
        stratum.workload_id.as_str(),
        stratum.graph_profile.as_str(),
        stratum.n,
        attempt.batch,
        attempt.round,
        attempt.retry_ordinal
    );
    let expected_run_id = format!(
        "{expected_round_attempt_id}/position-{}/candidate-{}",
        attempt.position, attempt.candidate_id
    );
    let compiler_instance_id = format!("{expected_run_id}/compiler");
    if attempt.status != RunStatus::Valid
        || !attempt.invalidation_reasons.is_empty()
        || attempt.round_attempt_id != expected_round_attempt_id
        || attempt.run_id != expected_run_id
        || attempt.batch != sample.batch
        || attempt.round != sample.round
        || attempt.position != sample.position
        || attempt.candidate_id != sample.candidate_id
        || attempt.child.as_ref() != Some(&sample.child)
        || attempt.monitor != sample.monitor
        || attempt.kill_error.is_some()
        || attempt.monitor_error.is_some()
        || !external_state_is_clear(attempt.external_state.as_ref(), checkpoint)
        || child.outcome != crate::CandidatePipelineOutcome::Success
        || child.semantic_digest_sha256.as_deref()
            != Some(qualification.semantic_digest_sha256.as_str())
        || child.candidate_pipeline_checksums != Some(qualification.baseline_checksums)
        || !crate::pilot_budget::successful_process(
            &attempt.process,
            crate::TIMING_BINARY_ID,
            child.child_pid,
        )
    {
        return Err(EvidenceError::InvalidCheckpoint(format!(
            "候选运行 {} 的有效进程封套无效",
            attempt.run_id
        )));
    }
    crate::candidate_matrix::validate_candidate_pipeline_report(
        child,
        &compiler_instance_id,
        &attempt.candidate_id,
        stratum.key_domain,
        stratum.workload_id,
        stratum.graph_profile,
        stratum.n,
        thresholds.compiler_controlled_bytes,
    )
    .map_err(EvidenceError::InvalidCheckpoint)?;
    crate::pilot_budget::validate_clear_monitor_with_thresholds(
        Some(&attempt.monitor),
        &thresholds,
        child.wall_time_ns,
        attempt.kill_error.as_deref(),
        attempt.monitor_error.as_deref(),
        &attempt.run_id,
    )
    .map_err(|error| EvidenceError::InvalidCheckpoint(error.to_string()))?;
    Ok(())
}

fn candidate_batch_ratio(
    validated: &ValidatedCandidateExecution<'_>,
    candidate_id: &str,
    batch: u32,
) -> Result<Ratio, EvidenceError> {
    let mut ratios = Vec::new();
    for scheduled in validated
        .execution
        .schedule
        .iter()
        .filter(|round| round.batch == batch)
    {
        let baseline = validated.sample(
            batch,
            scheduled.round,
            &validated.execution.roster.baseline_id,
        )?;
        let candidate = validated.sample(batch, scheduled.round, candidate_id)?;
        ratios.push(Ratio::new(
            candidate
                .child
                .wall_time_ns
                .ok_or_else(|| EvidenceError::InvalidCheckpoint("候选样本缺少墙钟".to_owned()))?,
            baseline.child.wall_time_ns.ok_or_else(|| {
                EvidenceError::InvalidCheckpoint("候选基线样本缺少墙钟".to_owned())
            })?,
        )?);
    }
    if ratios.len() != validated.participant_count * 2 {
        return Err(EvidenceError::InvalidCheckpoint(
            "候选平衡轮次不完整".to_owned(),
        ));
    }
    median_ratio(&ratios)
}

fn classify_candidate(ratios: [Ratio; 2], envelope: Ratio) -> Result<&'static str, EvidenceError> {
    let improvement = ratios.iter().all(|ratio| {
        ratio.numerator * envelope.numerator < ratio.denominator * envelope.denominator
    });
    let regression = ratios.iter().all(|ratio| {
        ratio.numerator * envelope.denominator > ratio.denominator * envelope.numerator
    });
    let noise = ratios.iter().all(|ratio| {
        ratio.numerator * envelope.numerator >= ratio.denominator * envelope.denominator
            && ratio.numerator * envelope.denominator <= ratio.denominator * envelope.numerator
    });
    Ok(if improvement {
        "repeatable-improvement"
    } else if regression {
        "repeatable-regression"
    } else if noise {
        "noise-no-difference"
    } else {
        "insufficient-evidence"
    })
}

#[derive(Clone, Copy, Debug)]
struct Ratio {
    numerator: u128,
    denominator: u128,
}

impl Ratio {
    fn new(numerator: u64, denominator: u64) -> Result<Self, EvidenceError> {
        if numerator == 0 || denominator == 0 {
            return Err(EvidenceError::InvalidCheckpoint(
                "比值分子或分母为零".to_owned(),
            ));
        }
        Ok(Self::reduce(u128::from(numerator), u128::from(denominator)))
    }

    fn reduce(numerator: u128, denominator: u128) -> Self {
        let divisor = gcd(numerator, denominator);
        Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        }
    }

    fn cmp(&self, other: &Self) -> Ordering {
        (self.numerator * other.denominator).cmp(&(other.numerator * self.denominator))
    }

    fn json(self) -> Value {
        json!({
            "numerator": self.numerator.to_string(),
            "denominator": self.denominator.to_string()
        })
    }

    fn from_json(value: &Value) -> Result<Self, EvidenceError> {
        let numerator = required_string(value, "/numerator")?
            .parse::<u128>()
            .map_err(|_| EvidenceError::InvalidCheckpoint("比值分子无效".to_owned()))?;
        let denominator = required_string(value, "/denominator")?
            .parse::<u128>()
            .map_err(|_| EvidenceError::InvalidCheckpoint("比值分母无效".to_owned()))?;
        Ok(Self::reduce(numerator, denominator))
    }
}

fn median_ratio(values: &[Ratio]) -> Result<Ratio, EvidenceError> {
    if values.is_empty() {
        return Err(EvidenceError::InvalidCheckpoint("比值集合为空".to_owned()));
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(Ratio::cmp);
    if sorted.len() % 2 == 1 {
        Ok(sorted[sorted.len() / 2])
    } else {
        let left = sorted[sorted.len() / 2 - 1];
        let right = sorted[sorted.len() / 2];
        Ok(Ratio::reduce(
            left.numerator * right.denominator + right.numerator * left.denominator,
            2 * left.denominator * right.denominator,
        ))
    }
}

fn coverage(
    checkpoint: &FormalProtocolCheckpoint,
    candidate_matrix: &crate::CandidateMatrixExecutionBundle,
) -> Value {
    let mut counts = [0_u64; 3];
    let mut add = |status: RunStatus| {
        counts[match status {
            RunStatus::Valid => 0,
            RunStatus::Invalid => 1,
            RunStatus::Guarded => 2,
        }] += 1;
    };
    add(checkpoint.current_fixtures.status);
    for run in &checkpoint.base_scale_pilot.runs {
        add(run.status);
    }
    for run in &checkpoint.base_scale_pilot.oracle_runs {
        add(run.status);
    }
    for ladder in &checkpoint.formal_ladders {
        for level in &ladder.levels {
            if let Some(run) = &level.attribution_preflight {
                add(run.status);
            }
            if let Some(run) = &level.timing_guard_run {
                add(run.status);
            }
            if let Some(run) = &level.oracle {
                add(run.status);
            }
            for run in &level.formal_runs {
                add(run.status);
            }
        }
    }
    if let Some(limit) = &checkpoint.limit_qualification {
        for run in &limit.live_byte_baseline_runs {
            add(run.status);
        }
        for pair in &limit.limit_pairs {
            add(pair.at_bound.status);
            add(pair.plus_one.status);
        }
        for qualification in &limit.duplicate_owner_qualifications {
            add(qualification.run.status);
        }
        for experiment in &limit.cleanup_experiments {
            add(experiment.run.status);
        }
    }
    for qualification in &candidate_matrix.constant_hash_qualifications {
        for run in &qualification.runs {
            add(run.status);
        }
    }
    for execution in &candidate_matrix.executions {
        add(execution.roster.oracle_run.status);
        for run in &execution.roster.candidate_runs {
            add(run.status);
        }
        for run in &execution.attempts {
            add(run.status);
        }
    }
    json!({
        "processRunCount": counts.iter().sum::<u64>(),
        "validRunCount": counts[0],
        "invalidRunCount": counts[1],
        "guardedRunCount": counts[2],
        "baseScaleCount": checkpoint.base_scale_pilot.selections.len(),
        "formalLadderCount": checkpoint.formal_ladders.len(),
        "completedFormalLevelCount": checkpoint.formal_ladders.iter()
            .flat_map(|ladder| &ladder.levels).filter(|level| level.complete).count(),
        "limitPairCount": checkpoint.limit_qualification.as_ref().map_or(0, |limit| limit.limit_pairs.len()),
        "candidateExecutionCount": candidate_matrix.executions.len()
    })
}

fn batch_key(summary: &FormalLadderBatchSummary) -> Result<String, EvidenceError> {
    Ok(format!(
        "{}/{}/{}/{}/{}/{}",
        summary.workload_id.as_str(),
        summary.graph_profile,
        summary.n,
        token(&summary.metric)?,
        token(&summary.sample_kind)?,
        token(&summary.binary_mode)?
    ))
}

fn parse_graph_profile(value: &str) -> Result<crate::GraphProfileId, EvidenceError> {
    match value {
        "wide-star-v1" => Ok(crate::GraphProfileId::WideStar),
        "deep-chain-v1" => Ok(crate::GraphProfileId::DeepChain),
        "shared-fanin-dag-v1" => Ok(crate::GraphProfileId::SharedFaninDag),
        _ => Err(EvidenceError::InvalidCheckpoint(format!(
            "未知模块图配置档 {value}"
        ))),
    }
}

fn token(value: &impl Serialize) -> Result<String, EvidenceError> {
    serde_json::to_value(value)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| EvidenceError::InvalidCheckpoint("枚举无法序列化为字符串".to_owned()))
}

fn ceil_ratio_to_quantum(observed: u64, ratio: Ratio, quantum: u64) -> Result<u64, EvidenceError> {
    let numerator = u128::from(observed) * ratio.numerator;
    let scaled = numerator.div_ceil(ratio.denominator);
    let quantum = u128::from(quantum.max(1));
    u64::try_from(scaled.div_ceil(quantum) * quantum)
        .map_err(|_| EvidenceError::InvalidCheckpoint("预算建议溢出".to_owned()))
}

fn gcd(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left
}

fn validate_schema(trusted: &TrustedContract, document: &Value) -> Result<(), EvidenceError> {
    jsonschema::draft202012::validate(&trusted.evidence_schema, document).map_err(|error| {
        EvidenceError::Schema(format!(
            "{}（实例路径：{}；Schema 路径：{}）",
            error,
            error.instance_path(),
            error.schema_path()
        ))
    })
}

fn report(document: &Value) -> Result<EvidenceVerificationReport, EvidenceError> {
    Ok(EvidenceVerificationReport {
        schema: required_string(document, "/schema")?.to_owned(),
        schema_version: required_u64(document, "/schemaVersion")?,
        source_commit: required_string(document, "/source/measurementCommit")?.to_owned(),
        raw_byte_length: required_u64(document, "/rawExecution/byteLength")?,
        raw_sha256: required_string(document, "/rawExecution/sha256")?.to_owned(),
        process_run_count: required_u64(document, "/coverage/processRunCount")?,
        invalid_run_count: required_u64(document, "/coverage/invalidRunCount")?,
        guarded_run_count: required_u64(document, "/coverage/guardedRunCount")?,
        base_scale_count: required_array(document, "/results/baseScaleSelections")?.len(),
        formal_ladder_count: required_array(document, "/results/formalLadders")?.len(),
        completed_formal_level_count: required_u64(
            document,
            "/coverage/completedFormalLevelCount",
        )?,
        budget_recommendation_count: required_array(document, "/results/budgetRecommendations")?
            .len(),
        candidate_classification_count: required_array(
            document,
            "/results/candidateMatrix/executions",
        )?
        .iter()
        .map(|execution| required_array(execution, "/comparisons").map(<[_]>::len))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .sum(),
    })
}

fn parse_checkpoint(path: &Path, bytes: &[u8]) -> Result<FormalProtocolCheckpoint, EvidenceError> {
    serde_json::from_slice(bytes).map_err(|source| EvidenceError::InvalidJson {
        path: path.to_path_buf(),
        source,
    })
}

fn repository_relative_path(path: &Path) -> Result<String, EvidenceError> {
    let root = fs::canonicalize(repository_root()).map_err(|source| EvidenceError::Read {
        path: repository_root(),
        source,
    })?;
    let absolute = fs::canonicalize(path).map_err(|source| EvidenceError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let relative = absolute
        .strip_prefix(&root)
        .map_err(|_| EvidenceError::UnsafePath(absolute.clone()))?;
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(EvidenceError::UnsafePath(absolute));
    }
    Ok(relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

fn resolve_repository_path(path: &str) -> Result<PathBuf, EvidenceError> {
    let relative = Path::new(path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(EvidenceError::UnsafePath(relative.to_path_buf()));
    }
    Ok(repository_root().join(relative))
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), EvidenceError> {
    let parent = path
        .parent()
        .filter(|parent| parent.is_dir())
        .ok_or_else(|| EvidenceError::InvalidOutput(path.to_path_buf()))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| EvidenceError::InvalidOutput(path.to_path_buf()))?;
    let mut temporary_name = OsString::from(".");
    temporary_name.push(file_name);
    temporary_name.push(format!(".{}.tmp", std::process::id()));
    let temporary = parent.join(temporary_name);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|source| EvidenceError::Write {
            path: temporary.clone(),
            source,
        })?;
    if let Err(source) = file.write_all(bytes).and_then(|()| file.write_all(b"\n")) {
        drop(file);
        let _ = fs::remove_file(&temporary);
        return Err(EvidenceError::Write {
            path: temporary,
            source,
        });
    }
    drop(file);
    fs::rename(&temporary, path).map_err(|source| EvidenceError::Write {
        path: path.to_path_buf(),
        source,
    })
}

fn required_string<'a>(value: &'a Value, pointer: &str) -> Result<&'a str, EvidenceError> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| EvidenceError::MissingField(pointer.to_owned()))
}

fn required_u64(value: &Value, pointer: &str) -> Result<u64, EvidenceError> {
    value
        .pointer(pointer)
        .and_then(Value::as_u64)
        .ok_or_else(|| EvidenceError::MissingField(pointer.to_owned()))
}

fn required_array<'a>(value: &'a Value, pointer: &str) -> Result<&'a [Value], EvidenceError> {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| EvidenceError::MissingField(pointer.to_owned()))
}

fn expect_string(value: &Value, pointer: &str, expected: &str) -> Result<(), EvidenceError> {
    let actual = required_string(value, pointer)?;
    if actual != expected {
        return Err(EvidenceError::BindingMismatch(pointer.to_owned()));
    }
    Ok(())
}

fn expect_u64(value: &Value, pointer: &str, expected: u64) -> Result<(), EvidenceError> {
    if required_u64(value, pointer)? != expected {
        return Err(EvidenceError::BindingMismatch(pointer.to_owned()));
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex_digest(Sha256::digest(bytes))
}

fn sha256_with_newline(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.update(b"\n");
    hex_digest(hasher.finalize())
}

fn hex_digest(digest: impl AsRef<[u8]>) -> String {
    let mut output = String::with_capacity(64);
    for byte in digest.as_ref() {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("String write cannot fail");
    }
    output
}

#[derive(Debug, thiserror::Error)]
pub enum EvidenceError {
    #[error(transparent)]
    Contract(#[from] ContractError),
    #[error("无法读取 {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} 不是有效 JSON: {source}")]
    InvalidJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("无法序列化或解析派生结果: {0}")]
    Json(#[from] serde_json::Error),
    #[error("原始正式执行结果无效：{0}")]
    InvalidCheckpoint(String),
    #[error("Evidence 不满足紧凑 Schema：{0}")]
    Schema(String),
    #[error("Evidence 与绑定原始结果的独立重算不一致")]
    RecomputationMismatch,
    #[error("Evidence 字段绑定不匹配：{0}")]
    BindingMismatch(String),
    #[error("Evidence 缺少字段或类型错误：{0}")]
    MissingField(String),
    #[error("路径不是安全的仓库相对路径：{0}")]
    UnsafePath(PathBuf),
    #[error("Evidence 输出已存在：{path}")]
    OutputAlreadyExists { path: PathBuf },
    #[error("Evidence 输出路径无效：{0}")]
    InvalidOutput(PathBuf),
    #[error("无法写入 {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binary_source(binary_id: &str, digest_byte: &str) -> crate::FormalBinarySourceSnapshot {
        crate::FormalBinarySourceSnapshot {
            binary_id: binary_id.to_owned(),
            sha256: digest_byte.repeat(32),
        }
    }

    #[test]
    fn exact_ratio_median_is_reduced_without_floating_point() {
        let result = median_ratio(&[Ratio::new(1, 2).unwrap(), Ratio::new(3, 2).unwrap()]).unwrap();
        assert_eq!(result.numerator, 1);
        assert_eq!(result.denominator, 1);
    }

    #[test]
    fn repository_path_rejects_parent_traversal() {
        assert!(matches!(
            resolve_repository_path("../outside.json"),
            Err(EvidenceError::UnsafePath(_))
        ));
    }

    #[test]
    fn binary_sources_require_the_exact_roles_and_canonical_sha256() {
        let valid = [
            binary_source(crate::TIMING_BINARY_ID, "01"),
            binary_source(crate::ATTRIBUTION_BINARY_ID, "02"),
            binary_source(crate::ORACLE_BINARY_ID, "03"),
        ];
        assert!(validate_binary_sources(&valid).is_ok());

        let duplicate = [
            binary_source(crate::TIMING_BINARY_ID, "01"),
            binary_source(crate::TIMING_BINARY_ID, "02"),
            binary_source(crate::ORACLE_BINARY_ID, "03"),
        ];
        assert!(validate_binary_sources(&duplicate).is_err());

        let duplicate_digest = [
            binary_source(crate::TIMING_BINARY_ID, "01"),
            binary_source(crate::ATTRIBUTION_BINARY_ID, "01"),
            binary_source(crate::ORACLE_BINARY_ID, "03"),
        ];
        assert!(validate_binary_sources(&duplicate_digest).is_err());

        let mut uppercase = valid.clone();
        uppercase[0].sha256 = "AB".repeat(32);
        assert!(validate_binary_sources(&uppercase).is_err());
    }

    #[test]
    fn published_raw_rejects_unbound_formal_and_candidate_sources() {
        let raw_path =
            repository_root().join("docs/reference/v0.10-compiler-budget-calibration-raw.json");
        let raw = fs::read(&raw_path).expect("published raw checkpoint");
        let mut checkpoint = parse_checkpoint(&raw_path, &raw).expect("valid raw checkpoint");
        let trusted = crate::load_repository_contract().expect("trusted contract");

        validate_formal_ladder_base_scales(&checkpoint).expect("base-scale bindings");
        let original_b = checkpoint.formal_ladders[0].b;
        checkpoint.formal_ladders[0].b = original_b.checked_mul(2).expect("tampered B");
        assert!(validate_formal_ladder_base_scales(&checkpoint).is_err());
        checkpoint.formal_ladders[0].b = original_b;

        let ladder = &checkpoint.formal_ladders[0];
        let experiment_ordinal = validate_formal_rotation(ladder).expect("formal rotation");
        let level = &ladder.levels[0];
        let mut invalid_run = level.formal_runs[0].clone();
        invalid_run.status = RunStatus::Invalid;
        invalid_run
            .invalidation_reasons
            .push(crate::InvalidationReason::MonitoringGap);
        let expected_digest = level
            .attribution_preflight
            .as_ref()
            .and_then(|run| run.child.as_ref())
            .and_then(|child| child.semantic_digest_sha256.as_deref())
            .expect("formal semantic digest");
        assert!(
            validate_formal_round_envelope(
                &checkpoint,
                ladder,
                level,
                &invalid_run,
                expected_digest,
                experiment_ordinal,
                0,
            )
            .is_err()
        );

        let mut invalid_rotation = ladder.clone();
        invalid_rotation.levels[0].formal_runs[0].execution_position =
            (invalid_rotation.levels[0].formal_runs[0].execution_position + 1)
                % u32::try_from(crate::FORMAL_LADDER_MINIMUM_LEVEL_COUNT)
                    .expect("formal level count fits u32");
        assert!(validate_formal_rotation(&invalid_rotation).is_err());

        let guard_planner =
            crate::ScalableGuardPlanner::from_trusted_contract(&trusted).expect("guard planner");
        let graph_profile = parse_graph_profile(&ladder.graph_profile).expect("graph profile");
        let first_oracle = ladder.levels[0]
            .oracle
            .as_ref()
            .and_then(|run| run.child.as_ref())
            .expect("formal oracle");
        let previous = validate_completed_guard_observation(&ladder.levels[0], first_oracle)
            .expect("completed guard observation");
        let mut invalid_preflight = ladder.levels[1].clone();
        invalid_preflight.guard_preflight.predicted_wall_time_ns = None;
        assert!(
            validate_formal_guard_preflight(
                &guard_planner,
                ladder,
                &invalid_preflight,
                graph_profile,
                Some(previous),
                checkpoint.environment.physical_memory_bytes,
            )
            .is_err()
        );

        let matrix = checkpoint
            .candidate_matrix
            .as_ref()
            .expect("candidate matrix");
        validate_candidate_matrix_envelope(&trusted, &checkpoint, matrix)
            .expect("candidate matrix envelope");
        let mut incomplete_matrix = matrix.clone();
        incomplete_matrix.executions.pop();
        assert!(
            validate_candidate_matrix_envelope(&trusted, &checkpoint, &incomplete_matrix).is_err()
        );
        let expected_safety =
            crate::candidate_matrix::expected_candidate_safety_assessments(&trusted)
                .expect("candidate safety");
        let constant_hash_qualifications =
            validate_constant_hash_qualifications(&checkpoint, matrix)
                .expect("constant-hash qualifications");
        let mut invalid_constant_hash = matrix.clone();
        invalid_constant_hash.constant_hash_qualifications[0]
            .qualification
            .passed = !invalid_constant_hash.constant_hash_qualifications[0]
            .qualification
            .passed;
        assert!(
            validate_constant_hash_qualifications(&checkpoint, &invalid_constant_hash).is_err()
        );
        let candidate = matrix.executions[0].clone();
        validate_candidate_execution(
            &trusted,
            &checkpoint,
            &expected_safety,
            &constant_hash_qualifications,
            &candidate,
        )
        .expect("candidate execution");

        let tampered_digest = "00".repeat(32);
        let mut invalid_qualification_oracle = candidate.clone();
        invalid_qualification_oracle
            .roster
            .oracle_run
            .child
            .as_mut()
            .expect("candidate qualification oracle")
            .semantic_digest_sha256 = Some(tampered_digest.clone());
        for run in &mut invalid_qualification_oracle.roster.candidate_runs {
            run.child
                .as_mut()
                .expect("candidate qualification child")
                .semantic_digest_sha256 = Some(tampered_digest.clone());
        }
        assert!(
            validate_candidate_execution(
                &trusted,
                &checkpoint,
                &expected_safety,
                &constant_hash_qualifications,
                &invalid_qualification_oracle,
            )
            .is_err()
        );

        let mut invalid_performance_binding = candidate.clone();
        for sample in &mut invalid_performance_binding.samples {
            sample.child.semantic_digest_sha256 = Some(tampered_digest.clone());
        }
        for attempt in &mut invalid_performance_binding.attempts {
            attempt
                .child
                .as_mut()
                .expect("candidate performance child")
                .semantic_digest_sha256 = Some(tampered_digest.clone());
        }
        assert!(
            validate_candidate_execution(
                &trusted,
                &checkpoint,
                &expected_safety,
                &constant_hash_qualifications,
                &invalid_performance_binding,
            )
            .is_err()
        );

        let mut invalid_performance_checksums = candidate.clone();
        for sample in &mut invalid_performance_checksums.samples {
            sample
                .child
                .candidate_pipeline_checksums
                .as_mut()
                .expect("candidate performance checksums")
                .external_string ^= 1;
        }
        for attempt in &mut invalid_performance_checksums.attempts {
            attempt
                .child
                .as_mut()
                .and_then(|child| child.candidate_pipeline_checksums.as_mut())
                .expect("candidate performance checksums")
                .external_string ^= 1;
        }
        assert!(
            validate_candidate_execution(
                &trusted,
                &checkpoint,
                &expected_safety,
                &constant_hash_qualifications,
                &invalid_performance_checksums,
            )
            .is_err()
        );

        let mut invalid_roster = candidate.clone();
        let baseline_index = invalid_roster
            .roster
            .entries
            .iter()
            .position(|entry| entry.disposition == crate::CandidateDisposition::BaselineParticipant)
            .expect("baseline entry");
        let performance_index = invalid_roster
            .roster
            .entries
            .iter()
            .position(|entry| {
                entry.disposition == crate::CandidateDisposition::PerformanceParticipant
            })
            .expect("performance entry");
        invalid_roster.roster.baseline_id = invalid_roster.roster.entries[performance_index]
            .candidate_id
            .clone();
        invalid_roster.roster.entries[baseline_index].disposition =
            crate::CandidateDisposition::PerformanceParticipant;
        invalid_roster.roster.entries[performance_index].disposition =
            crate::CandidateDisposition::BaselineParticipant;
        assert!(
            validate_candidate_execution(
                &trusted,
                &checkpoint,
                &expected_safety,
                &constant_hash_qualifications,
                &invalid_roster,
            )
            .is_err()
        );

        let mut invalid_process = candidate.clone();
        let process_run_id = invalid_process.samples[0].run_id.clone();
        invalid_process
            .attempts
            .iter_mut()
            .find(|attempt| attempt.run_id == process_run_id)
            .expect("candidate attempt")
            .process
            .binary_id = "tampered-binary".to_owned();
        assert!(
            validate_candidate_execution(
                &trusted,
                &checkpoint,
                &expected_safety,
                &constant_hash_qualifications,
                &invalid_process,
            )
            .is_err()
        );

        let mut invalid_semantics = candidate.clone();
        let semantic_run_id = invalid_semantics.samples[0].run_id.clone();
        invalid_semantics.samples[0].child.semantic_digest_sha256 = Some(tampered_digest.clone());
        invalid_semantics
            .attempts
            .iter_mut()
            .find(|attempt| attempt.run_id == semantic_run_id)
            .and_then(|attempt| attempt.child.as_mut())
            .expect("candidate child")
            .semantic_digest_sha256 = Some(tampered_digest);
        assert!(
            validate_candidate_execution(
                &trusted,
                &checkpoint,
                &expected_safety,
                &constant_hash_qualifications,
                &invalid_semantics,
            )
            .is_err()
        );

        let mut invalid_guard = candidate;
        let run_id = invalid_guard.samples[0].run_id.clone();
        invalid_guard
            .attempts
            .iter_mut()
            .find(|attempt| attempt.run_id == run_id)
            .expect("candidate attempt")
            .monitor
            .trigger = Some(crate::ChildMonitorTrigger::WallTime);
        invalid_guard.samples[0].monitor.trigger = Some(crate::ChildMonitorTrigger::WallTime);
        assert!(
            validate_candidate_execution(
                &trusted,
                &checkpoint,
                &expected_safety,
                &constant_hash_qualifications,
                &invalid_guard,
            )
            .is_err()
        );
    }
}
