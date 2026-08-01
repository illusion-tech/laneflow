//! `compiler-calibration-v1` 正式研究入口与执行检查点写出。
//!
//! runner 先完成基础规模试运行，再执行正式规模阶梯并保存原始子进程、精确汇总、
//! 拐点和规模选择。该检查点不冒充编译器校准证据 v1；证据写出器只能把它作为原始
//! 输入，并在发布前通过独立 Evidence v1 验证器。

use crate::ladder_runner::decode_child_execution;
use crate::pilot::run_monitored_command_child;
use crate::{
    ATTRIBUTION_BINARY_ID, BaseScalePilotCheckpoint, CandidateMatrixError,
    CandidateMatrixExecutionBundle, ChildProcessMonitorReport, ContractError,
    CurrentFixturesChildReport, ExternalStateObservation, FORMAL_PROTOCOL_ID,
    FormalEnvironmentSnapshot, FormalLadderExecution, FormalLadderRunnerError, GuardThresholds,
    InvalidationReason, LimitQualificationBundle, LimitQualificationExecutionError,
    ORACLE_BINARY_ID, PilotError, ProcessObservation, RunStatus, TIMING_BINARY_ID,
    load_and_install_formal_environment, load_repository_contract, repository_root,
    run_base_scale_pilot_discovery_with_checkpoint_sink,
    run_candidate_matrix_bundle_with_checkpoint_sink, run_formal_ladders,
    run_limit_qualification_bundle, validate_limit_qualification_bundle,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormalProtocolRequest {
    pub output_path: PathBuf,
    pub environment_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormalProtocolOutcome {
    pub protocol_id: String,
    pub artifact_kind: String,
    pub output_path: PathBuf,
    pub checkpoint_directory: PathBuf,
    pub completed_base_scale_selections: usize,
    pub completed_formal_ladders: usize,
    pub recorded_base_scale_runs: usize,
    pub recorded_base_scale_oracle_runs: usize,
    pub recorded_formal_process_runs: usize,
    pub recorded_formal_oracle_runs: usize,
    pub recorded_attribution_preflight_runs: usize,
    pub recorded_timing_guard_runs: usize,
    pub current_fixture_case_count: usize,
    pub limit_pair_count: usize,
    pub cleanup_experiment_count: usize,
    pub limit_qualification_valid: bool,
    pub candidate_scale_count: usize,
    pub candidate_roster_count: usize,
    pub constant_hash_qualification_count: usize,
    pub candidate_performance_attempt_count: usize,
}

pub const FORMAL_PROTOCOL_CHECKPOINT_SCHEMA: &str =
    "laneflow.compiler-calibration-formal-execution-checkpoint";
pub const FORMAL_PROTOCOL_CHECKPOINT_SCHEMA_VERSION: u32 = 5;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormalCurrentFixtureProjection {
    pub status: RunStatus,
    pub invalidation_reasons: Vec<InvalidationReason>,
    pub process: ProcessObservation,
    pub child: Option<CurrentFixturesChildReport>,
    pub monitor: ChildProcessMonitorReport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_state: Option<ExternalStateObservation>,
    pub kill_error: Option<String>,
    pub monitor_error: Option<String>,
    pub stderr: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormalBinarySourceSnapshot {
    pub binary_id: String,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormalSourceSnapshot {
    pub source_commit: String,
    pub harness_commit: String,
    pub dirty: bool,
    pub cargo_lock_sha256: String,
    pub binaries: Vec<FormalBinarySourceSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormalProtocolCheckpoint {
    pub schema: String,
    pub schema_version: u32,
    pub protocol_id: String,
    pub source: FormalSourceSnapshot,
    pub environment: FormalEnvironmentSnapshot,
    pub current_fixtures: FormalCurrentFixtureProjection,
    pub base_scale_pilot: BaseScalePilotCheckpoint,
    pub formal_ladders: Vec<FormalLadderExecution>,
    pub active_formal_ladder: Option<FormalLadderExecution>,
    pub limit_qualification: Option<LimitQualificationBundle>,
    pub limit_qualification_validation_error: Option<String>,
    pub candidate_matrix: Option<CandidateMatrixExecutionBundle>,
}

pub fn parse_formal_protocol_arguments(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<FormalProtocolRequest, FormalProtocolError> {
    let mut protocol = None;
    let mut output_path = None;
    let mut environment_path = None;
    let mut arguments = arguments.into_iter();

    while let Some(flag) = arguments.next() {
        let flag = flag
            .into_string()
            .map_err(|_| FormalProtocolError::NonUtf8Option)?;
        let value = arguments
            .next()
            .ok_or_else(|| FormalProtocolError::MissingOptionValue {
                option: flag.clone(),
            })?;
        match flag.as_str() {
            "--protocol" => {
                if protocol.is_some() {
                    return Err(FormalProtocolError::DuplicateOption { option: flag });
                }
                let value = value
                    .into_string()
                    .map_err(|_| FormalProtocolError::NonUtf8Protocol)?;
                protocol = Some(value);
            }
            "--output" => {
                if output_path.is_some() {
                    return Err(FormalProtocolError::DuplicateOption { option: flag });
                }
                if value.is_empty() {
                    return Err(FormalProtocolError::EmptyOutputPath);
                }
                output_path = Some(PathBuf::from(value));
            }
            "--environment" => {
                if environment_path.is_some() {
                    return Err(FormalProtocolError::DuplicateOption { option: flag });
                }
                if value.is_empty() {
                    return Err(FormalProtocolError::EmptyEnvironmentPath);
                }
                environment_path = Some(PathBuf::from(value));
            }
            _ => return Err(FormalProtocolError::UnknownOption { option: flag }),
        }
    }

    let protocol = protocol.ok_or(FormalProtocolError::MissingRequiredOption {
        option: "--protocol",
    })?;
    if protocol != FORMAL_PROTOCOL_ID {
        return Err(FormalProtocolError::UnsupportedProtocol { actual: protocol });
    }
    let output_path =
        output_path.ok_or(FormalProtocolError::MissingRequiredOption { option: "--output" })?;
    let environment_path = environment_path.ok_or(FormalProtocolError::MissingRequiredOption {
        option: "--environment",
    })?;
    Ok(FormalProtocolRequest {
        output_path,
        environment_path,
    })
}

pub fn run_formal_protocol(
    request: &FormalProtocolRequest,
) -> Result<FormalProtocolOutcome, FormalProtocolError> {
    verify_formal_build_mode(
        cfg!(debug_assertions),
        cfg!(feature = "research-runner-full"),
    )?;

    let repository_root = repository_root();
    verify_clean_worktree(&repository_root)?;
    build_formal_child_binaries(&repository_root)?;
    verify_clean_worktree(&repository_root)?;
    let trusted = load_repository_contract()?;
    let environment = load_and_install_formal_environment(&request.environment_path)?.clone();
    let timing_executable = resolve_sibling_timing_binary()?;
    let attribution_executable = resolve_sibling_attribution_binary()?;
    let oracle_executable = resolve_sibling_oracle_binary()?;
    verify_timing_binary_role(&timing_executable)?;
    verify_attribution_binary_role(&attribution_executable)?;
    verify_oracle_binary_role(&oracle_executable)?;
    let source = capture_formal_source(
        &repository_root,
        &timing_executable,
        &attribution_executable,
        &oracle_executable,
    )?;
    let current_fixtures = run_current_fixtures_process(&oracle_executable, &environment)?;
    let mut writer = FormalCheckpointWriter::prepare(&request.output_path)?;
    let base_scale_pilot = run_base_scale_pilot_discovery_with_checkpoint_sink(
        &trusted,
        &timing_executable,
        &oracle_executable,
        |base_scale_pilot| {
            writer
                .persist(&formal_checkpoint(
                    &source,
                    &environment,
                    &current_fixtures,
                    base_scale_pilot,
                    &[],
                    None,
                    None,
                    None,
                    None,
                ))
                .map_err(|error| PilotError::CheckpointPersistence {
                    detail: error.to_string(),
                })
        },
    )?;
    let formal_ladders = run_formal_ladders(
        &trusted,
        &timing_executable,
        &attribution_executable,
        &oracle_executable,
        &base_scale_pilot,
        |completed, active| {
            writer
                .persist(&formal_checkpoint(
                    &source,
                    &environment,
                    &current_fixtures,
                    &base_scale_pilot,
                    completed,
                    active,
                    None,
                    None,
                    None,
                ))
                .map_err(|error| FormalLadderRunnerError::CheckpointPersistence {
                    detail: error.to_string(),
                })
        },
    )?;
    let limit_qualification = run_limit_qualification_bundle(
        &trusted,
        &timing_executable,
        &attribution_executable,
        &formal_ladders,
    )?;
    let limit_qualification_validation_error =
        validate_limit_qualification_bundle(&limit_qualification)
            .err()
            .map(|error| error.to_string());
    writer.persist(&formal_checkpoint(
        &source,
        &environment,
        &current_fixtures,
        &base_scale_pilot,
        &formal_ladders,
        None,
        Some(&limit_qualification),
        limit_qualification_validation_error.as_deref(),
        None,
    ))?;
    let candidate_matrix = run_candidate_matrix_bundle_with_checkpoint_sink(
        &trusted,
        &timing_executable,
        &oracle_executable,
        &formal_ladders,
        |candidate_matrix| {
            writer
                .persist(&formal_checkpoint(
                    &source,
                    &environment,
                    &current_fixtures,
                    &base_scale_pilot,
                    &formal_ladders,
                    None,
                    Some(&limit_qualification),
                    limit_qualification_validation_error.as_deref(),
                    Some(candidate_matrix),
                ))
                .map_err(|error| error.to_string())
        },
    )?;
    let checkpoint = formal_checkpoint(
        &source,
        &environment,
        &current_fixtures,
        &base_scale_pilot,
        &formal_ladders,
        None,
        Some(&limit_qualification),
        limit_qualification_validation_error.as_deref(),
        Some(&candidate_matrix),
    );
    writer.finish(&checkpoint)?;

    Ok(FormalProtocolOutcome {
        protocol_id: FORMAL_PROTOCOL_ID.to_owned(),
        artifact_kind: "formal-execution-checkpoint".to_owned(),
        output_path: writer.output_path.clone(),
        checkpoint_directory: writer.checkpoint_directory.clone(),
        completed_base_scale_selections: base_scale_pilot.selections.len(),
        completed_formal_ladders: formal_ladders.len(),
        recorded_base_scale_runs: base_scale_pilot.runs.len(),
        recorded_base_scale_oracle_runs: base_scale_pilot.oracle_runs.len(),
        recorded_formal_process_runs: formal_ladders
            .iter()
            .flat_map(|ladder| &ladder.levels)
            .map(|level| level.formal_runs.len())
            .sum(),
        recorded_formal_oracle_runs: formal_ladders
            .iter()
            .flat_map(|ladder| &ladder.levels)
            .filter(|level| level.oracle.is_some())
            .count(),
        recorded_attribution_preflight_runs: formal_ladders
            .iter()
            .flat_map(|ladder| &ladder.levels)
            .filter(|level| level.attribution_preflight.is_some())
            .count(),
        recorded_timing_guard_runs: formal_ladders
            .iter()
            .flat_map(|ladder| &ladder.levels)
            .filter(|level| level.timing_guard_run.is_some())
            .count(),
        current_fixture_case_count: current_fixtures
            .child
            .as_ref()
            .map_or(0, |child| child.cases.len()),
        limit_pair_count: limit_qualification.limit_pairs.len(),
        cleanup_experiment_count: limit_qualification.cleanup_experiments.len(),
        limit_qualification_valid: limit_qualification_validation_error.is_none(),
        candidate_scale_count: candidate_matrix.scales.len(),
        candidate_roster_count: candidate_matrix.executions.len(),
        constant_hash_qualification_count: candidate_matrix.constant_hash_qualifications.len(),
        candidate_performance_attempt_count: candidate_matrix
            .executions
            .iter()
            .map(|execution| execution.attempts.len())
            .sum(),
    })
}

#[allow(clippy::too_many_arguments)]
fn formal_checkpoint(
    source: &FormalSourceSnapshot,
    environment: &FormalEnvironmentSnapshot,
    current_fixtures: &FormalCurrentFixtureProjection,
    base_scale_pilot: &BaseScalePilotCheckpoint,
    formal_ladders: &[FormalLadderExecution],
    active_formal_ladder: Option<&FormalLadderExecution>,
    limit_qualification: Option<&LimitQualificationBundle>,
    limit_qualification_validation_error: Option<&str>,
    candidate_matrix: Option<&CandidateMatrixExecutionBundle>,
) -> FormalProtocolCheckpoint {
    FormalProtocolCheckpoint {
        schema: FORMAL_PROTOCOL_CHECKPOINT_SCHEMA.to_owned(),
        schema_version: FORMAL_PROTOCOL_CHECKPOINT_SCHEMA_VERSION,
        protocol_id: FORMAL_PROTOCOL_ID.to_owned(),
        source: source.clone(),
        environment: environment.clone(),
        current_fixtures: current_fixtures.clone(),
        base_scale_pilot: base_scale_pilot.clone(),
        formal_ladders: formal_ladders.to_vec(),
        active_formal_ladder: active_formal_ladder.cloned(),
        limit_qualification: limit_qualification.cloned(),
        limit_qualification_validation_error: limit_qualification_validation_error
            .map(str::to_owned),
        candidate_matrix: candidate_matrix.cloned(),
    }
}

fn capture_formal_source(
    repository_root: &Path,
    timing_executable: &Path,
    attribution_executable: &Path,
    oracle_executable: &Path,
) -> Result<FormalSourceSnapshot, FormalProtocolError> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repository_root)
        .output()
        .map_err(|source| FormalProtocolError::SourceCommitLaunch { source })?;
    if !output.status.success() {
        return Err(FormalProtocolError::SourceCommitFailed {
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let source_commit = String::from_utf8(output.stdout)
        .map_err(|_| FormalProtocolError::SourceCommitNotUtf8)?
        .trim()
        .to_owned();
    if source_commit.len() != 40 || !source_commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(FormalProtocolError::InvalidSourceCommit(source_commit));
    }
    let cargo_lock_path = repository_root.join("Cargo.lock");
    let cargo_lock_sha256 = sha256_file(&cargo_lock_path)?;
    let binaries = [
        (TIMING_BINARY_ID, timing_executable),
        (ATTRIBUTION_BINARY_ID, attribution_executable),
        (ORACLE_BINARY_ID, oracle_executable),
    ]
    .into_iter()
    .map(|(binary_id, path)| {
        Ok(FormalBinarySourceSnapshot {
            binary_id: binary_id.to_owned(),
            sha256: sha256_file(path)?,
        })
    })
    .collect::<Result<Vec<_>, FormalProtocolError>>()?;
    Ok(FormalSourceSnapshot {
        source_commit: source_commit.clone(),
        harness_commit: source_commit,
        dirty: false,
        cargo_lock_sha256,
        binaries,
    })
}

fn run_current_fixtures_process(
    oracle_executable: &Path,
    environment: &FormalEnvironmentSnapshot,
) -> Result<FormalCurrentFixtureProjection, FormalProtocolError> {
    let thresholds =
        GuardThresholds::from_physical_memory_bytes(environment.physical_memory_bytes)?;
    let execution = run_monitored_command_child(
        oracle_executable,
        0,
        &["run-current-fixtures".to_owned()],
        thresholds,
    )?;
    let decoded = decode_child_execution(
        execution,
        ORACLE_BINARY_ID,
        |report: &CurrentFixturesChildReport| {
            if report.schema == crate::CURRENT_FIXTURES_CHILD_SCHEMA
                && report.schema_version == crate::CURRENT_FIXTURES_CHILD_SCHEMA_VERSION
                && report.binary_id == ORACLE_BINARY_ID
                && report.verification.checked_cases
                    == u32::try_from(report.cases.len()).unwrap_or(u32::MAX)
                && report.verification.production_loader_cases
                    == u32::try_from(report.cases.len()).unwrap_or(u32::MAX)
                && report.verification.independent_identity_and_stream_checked
                && report.verification.scenario_manifest_emits_no_records
                && report
                    .verification
                    .excluded_from_budget_and_candidate_ranking
            {
                Ok(())
            } else {
                Err("current-fixtures-child-protocol".to_owned())
            }
        },
        |_| false,
    )?;
    if decoded
        .child
        .as_ref()
        .is_some_and(|report| decoded.process.child_pid.value != Some(u64::from(report.child_pid)))
    {
        return Err(FormalProtocolError::CurrentFixturesChildPidMismatch);
    }
    Ok(FormalCurrentFixtureProjection {
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

fn sha256_file(path: &Path) -> Result<String, FormalProtocolError> {
    let bytes = fs::read(path).map_err(|source| FormalProtocolError::ReadSourceArtifact {
        path: path.to_path_buf(),
        source,
    })?;
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(encoded)
}

fn verify_formal_build_mode(
    debug_assertions_enabled: bool,
    full_runner_feature_enabled: bool,
) -> Result<(), FormalProtocolError> {
    if debug_assertions_enabled {
        return Err(FormalProtocolError::DebugBuild);
    }
    if !full_runner_feature_enabled {
        return Err(FormalProtocolError::MissingFullRunnerFeature);
    }
    Ok(())
}

fn verify_clean_worktree(repository_root: &Path) -> Result<(), FormalProtocolError> {
    let output = Command::new("git")
        .args(["status", "--porcelain=v1", "--untracked-files=all"])
        .current_dir(repository_root)
        .output()
        .map_err(|source| FormalProtocolError::GitStatusLaunch { source })?;
    validate_git_status_result(output.status.success(), &output.stdout, &output.stderr)
}

fn validate_git_status_result(
    success: bool,
    stdout: &[u8],
    stderr: &[u8],
) -> Result<(), FormalProtocolError> {
    if !success {
        return Err(FormalProtocolError::GitStatusFailed {
            stderr: String::from_utf8_lossy(stderr).trim().to_owned(),
        });
    }
    if !stdout.is_empty() {
        return Err(FormalProtocolError::DirtyWorktree {
            entries: String::from_utf8_lossy(stdout).trim().to_owned(),
        });
    }
    Ok(())
}

fn resolve_sibling_timing_binary() -> Result<PathBuf, FormalProtocolError> {
    let runner = std::env::current_exe()
        .map_err(|source| FormalProtocolError::CurrentExecutable { source })?;
    let directory =
        runner
            .parent()
            .ok_or_else(|| FormalProtocolError::MissingExecutableParent {
                executable: runner.clone(),
            })?;
    let timing = directory.join(format!(
        "issue-308-compiler-budget-calibration-timing{}",
        std::env::consts::EXE_SUFFIX
    ));
    if !timing.is_file() {
        return Err(FormalProtocolError::MissingTimingBinary { path: timing });
    }
    Ok(timing)
}

fn resolve_sibling_attribution_binary() -> Result<PathBuf, FormalProtocolError> {
    let runner = std::env::current_exe()
        .map_err(|source| FormalProtocolError::CurrentExecutable { source })?;
    let directory =
        runner
            .parent()
            .ok_or_else(|| FormalProtocolError::MissingExecutableParent {
                executable: runner.clone(),
            })?;
    let attribution = directory.join(format!(
        "issue-308-compiler-budget-calibration-attribution{}",
        std::env::consts::EXE_SUFFIX
    ));
    if !attribution.is_file() {
        return Err(FormalProtocolError::MissingAttributionBinary { path: attribution });
    }
    Ok(attribution)
}

fn resolve_sibling_oracle_binary() -> Result<PathBuf, FormalProtocolError> {
    let runner = std::env::current_exe()
        .map_err(|source| FormalProtocolError::CurrentExecutable { source })?;
    let directory =
        runner
            .parent()
            .ok_or_else(|| FormalProtocolError::MissingExecutableParent {
                executable: runner.clone(),
            })?;
    let oracle = directory.join(format!(
        "issue-308-compiler-budget-calibration-oracle{}",
        std::env::consts::EXE_SUFFIX
    ));
    if !oracle.is_file() {
        return Err(FormalProtocolError::MissingOracleBinary { path: oracle });
    }
    Ok(oracle)
}

fn build_formal_child_binaries(repository_root: &Path) -> Result<(), FormalProtocolError> {
    let output = Command::new("cargo")
        .args([
            "+1.96.0",
            "build",
            "--release",
            "--locked",
            "-p",
            "issue-308-compiler-budget-calibration-research",
            "--no-default-features",
            "--features",
            "research-runner-full",
            "--bin",
            "issue-308-compiler-budget-calibration-timing",
            "--bin",
            "issue-308-compiler-budget-calibration-attribution",
            "--bin",
            "issue-308-compiler-budget-calibration-oracle",
        ])
        .current_dir(repository_root)
        .output()
        .map_err(|source| FormalProtocolError::ChildRoleBuildLaunch { source })?;
    if !output.status.success() {
        return Err(FormalProtocolError::ChildRoleBuildFailed {
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(())
}

fn verify_timing_binary_role(timing_executable: &Path) -> Result<(), FormalProtocolError> {
    let output = Command::new(timing_executable)
        .arg("describe-role")
        .output()
        .map_err(|source| FormalProtocolError::TimingDescriptorLaunch {
            path: timing_executable.to_path_buf(),
            source,
        })?;
    if !output.status.success() {
        return Err(FormalProtocolError::TimingDescriptorFailed {
            path: timing_executable.to_path_buf(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let descriptor: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|source| {
            FormalProtocolError::InvalidTimingDescriptor {
                path: timing_executable.to_path_buf(),
                source,
            }
        })?;
    let responsibilities_match = descriptor
        .get("responsibilities")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|responsibilities| {
            responsibilities.as_slice() == [serde_json::json!("single-outer-wall-clock")]
        });
    if descriptor
        .get("binaryId")
        .and_then(serde_json::Value::as_str)
        != Some(TIMING_BINARY_ID)
        || descriptor.get("role").and_then(serde_json::Value::as_str) != Some("timing")
        || descriptor
            .get("evidenceMode")
            .and_then(serde_json::Value::as_str)
            != Some("timing")
        || descriptor
            .get("allocationInstrumentationEnabled")
            .and_then(serde_json::Value::as_bool)
            != Some(false)
        || !responsibilities_match
    {
        return Err(FormalProtocolError::UnexpectedTimingDescriptor {
            path: timing_executable.to_path_buf(),
        });
    }
    Ok(())
}

fn verify_attribution_binary_role(
    attribution_executable: &Path,
) -> Result<(), FormalProtocolError> {
    let output = Command::new(attribution_executable)
        .arg("describe-role")
        .output()
        .map_err(|source| FormalProtocolError::AttributionDescriptorLaunch {
            path: attribution_executable.to_path_buf(),
            source,
        })?;
    if !output.status.success() {
        return Err(FormalProtocolError::AttributionDescriptorFailed {
            path: attribution_executable.to_path_buf(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let descriptor: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|source| {
            FormalProtocolError::InvalidAttributionDescriptor {
                path: attribution_executable.to_path_buf(),
                source,
            }
        })?;
    let responsibilities_match = descriptor
        .get("responsibilities")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|responsibilities| {
            responsibilities.as_slice()
                == [
                    serde_json::json!("controlled-allocation"),
                    serde_json::json!("live-requested-bytes"),
                    serde_json::json!("peak-live-requested-bytes"),
                    serde_json::json!("retained-capacity-bytes"),
                ]
        });
    if descriptor
        .get("binaryId")
        .and_then(serde_json::Value::as_str)
        != Some(ATTRIBUTION_BINARY_ID)
        || descriptor.get("role").and_then(serde_json::Value::as_str) != Some("attribution")
        || descriptor
            .get("evidenceMode")
            .and_then(serde_json::Value::as_str)
            != Some("attribution")
        || descriptor
            .get("allocationInstrumentationEnabled")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || !responsibilities_match
    {
        return Err(FormalProtocolError::UnexpectedAttributionDescriptor {
            path: attribution_executable.to_path_buf(),
        });
    }
    Ok(())
}

fn verify_oracle_binary_role(oracle_executable: &Path) -> Result<(), FormalProtocolError> {
    let output = Command::new(oracle_executable)
        .arg("describe-role")
        .output()
        .map_err(|source| FormalProtocolError::OracleDescriptorLaunch {
            path: oracle_executable.to_path_buf(),
            source,
        })?;
    if !output.status.success() {
        return Err(FormalProtocolError::OracleDescriptorFailed {
            path: oracle_executable.to_path_buf(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let descriptor: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|source| {
            FormalProtocolError::InvalidOracleDescriptor {
                path: oracle_executable.to_path_buf(),
                source,
            }
        })?;
    let responsibilities_match = descriptor
        .get("responsibilities")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|responsibilities| {
            responsibilities.as_slice() == [serde_json::json!("independent-exact-correctness")]
        });
    if descriptor
        .get("binaryId")
        .and_then(serde_json::Value::as_str)
        != Some(ORACLE_BINARY_ID)
        || descriptor.get("role").and_then(serde_json::Value::as_str) != Some("oracle")
        || descriptor
            .get("evidenceMode")
            .and_then(serde_json::Value::as_str)
            != Some("oracle")
        || descriptor
            .get("allocationInstrumentationEnabled")
            .and_then(serde_json::Value::as_bool)
            != Some(false)
        || !responsibilities_match
    {
        return Err(FormalProtocolError::UnexpectedOracleDescriptor {
            path: oracle_executable.to_path_buf(),
        });
    }
    Ok(())
}

struct FormalCheckpointWriter {
    output_path: PathBuf,
    checkpoint_directory: PathBuf,
    next_sequence: u64,
}

impl FormalCheckpointWriter {
    fn prepare(output_path: &Path) -> Result<Self, FormalProtocolError> {
        let output_path = absolute_output_path(output_path)?;
        let parent =
            output_path
                .parent()
                .ok_or_else(|| FormalProtocolError::MissingOutputParent {
                    path: output_path.clone(),
                })?;
        if !parent.is_dir() {
            return Err(FormalProtocolError::OutputParentNotDirectory {
                path: parent.to_path_buf(),
            });
        }
        if output_path.exists() {
            return Err(FormalProtocolError::OutputAlreadyExists {
                path: output_path.clone(),
            });
        }
        let checkpoint_directory = checkpoint_directory_for(&output_path)?;
        if checkpoint_directory.exists() {
            return Err(FormalProtocolError::CheckpointDirectoryAlreadyExists {
                path: checkpoint_directory,
            });
        }
        fs::create_dir(&checkpoint_directory).map_err(|source| {
            FormalProtocolError::CreateCheckpointDirectory {
                path: checkpoint_directory.clone(),
                source,
            }
        })?;
        Ok(Self {
            output_path,
            checkpoint_directory,
            next_sequence: 0,
        })
    }

    fn persist(
        &mut self,
        checkpoint: &FormalProtocolCheckpoint,
    ) -> Result<(), FormalProtocolError> {
        let path = self
            .checkpoint_directory
            .join(format!("checkpoint-{:08}.json", self.next_sequence));
        write_json_atomically(&path, checkpoint)?;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(FormalProtocolError::CheckpointSequenceOverflow)?;
        Ok(())
    }

    fn finish(&self, checkpoint: &FormalProtocolCheckpoint) -> Result<(), FormalProtocolError> {
        write_json_atomically(&self.output_path, checkpoint)
    }
}

fn absolute_output_path(path: &Path) -> Result<PathBuf, FormalProtocolError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    std::env::current_dir()
        .map(|directory| directory.join(path))
        .map_err(|source| FormalProtocolError::CurrentDirectory { source })
}

fn checkpoint_directory_for(output_path: &Path) -> Result<PathBuf, FormalProtocolError> {
    let file_name =
        output_path
            .file_name()
            .ok_or_else(|| FormalProtocolError::MissingOutputFileName {
                path: output_path.to_path_buf(),
            })?;
    let mut checkpoint_name = file_name.to_os_string();
    checkpoint_name.push(".checkpoints");
    Ok(output_path.with_file_name(checkpoint_name))
}

fn write_json_atomically(
    destination: &Path,
    value: &impl Serialize,
) -> Result<(), FormalProtocolError> {
    if destination.exists() {
        return Err(FormalProtocolError::OutputAlreadyExists {
            path: destination.to_path_buf(),
        });
    }
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|source| FormalProtocolError::SerializeCheckpoint { source })?;
    let temporary = temporary_path_for(destination)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|source| FormalProtocolError::WriteCheckpoint {
            path: temporary.clone(),
            source,
        })?;
    write_checkpoint_bytes(&mut file, &temporary, &bytes)?;
    drop(file);
    fs::rename(&temporary, destination).map_err(|source| {
        let _ = fs::remove_file(&temporary);
        FormalProtocolError::PublishCheckpoint {
            source_path: temporary,
            destination_path: destination.to_path_buf(),
            source,
        }
    })
}

fn write_checkpoint_bytes(
    file: &mut File,
    path: &Path,
    bytes: &[u8],
) -> Result<(), FormalProtocolError> {
    file.write_all(bytes)
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.sync_all())
        .map_err(|source| FormalProtocolError::WriteCheckpoint {
            path: path.to_path_buf(),
            source,
        })
}

fn temporary_path_for(destination: &Path) -> Result<PathBuf, FormalProtocolError> {
    let file_name =
        destination
            .file_name()
            .ok_or_else(|| FormalProtocolError::MissingOutputFileName {
                path: destination.to_path_buf(),
            })?;
    let mut temporary_name = OsString::from(".");
    temporary_name.push(file_name);
    temporary_name.push(format!(".{}.tmp", std::process::id()));
    Ok(destination.with_file_name(temporary_name))
}

#[derive(Debug, thiserror::Error)]
pub enum FormalProtocolError {
    #[error("正式研究入口只接受 release 二进制；debug assertions 当前已启用")]
    DebugBuild,
    #[error("正式研究入口要求启用封闭总特性 research-runner-full")]
    MissingFullRunnerFeature,
    #[error("正式研究入口无法启动 git status")]
    GitStatusLaunch {
        #[source]
        source: std::io::Error,
    },
    #[error("正式研究入口无法确认工作树状态：{stderr}")]
    GitStatusFailed { stderr: String },
    #[error("正式研究入口拒绝脏工作树；以下条目尚未提交：\n{entries}")]
    DirtyWorktree { entries: String },
    #[error("正式研究入口无法定位当前执行器")]
    CurrentExecutable {
        #[source]
        source: std::io::Error,
    },
    #[error("正式研究执行器没有父目录：{executable}")]
    MissingExecutableParent { executable: PathBuf },
    #[error("未找到同目录的非插桩计时角色二进制：{path}")]
    MissingTimingBinary { path: PathBuf },
    #[error("未找到同目录的分配归因角色二进制：{path}")]
    MissingAttributionBinary { path: PathBuf },
    #[error("未找到同目录的独立预言机角色二进制：{path}")]
    MissingOracleBinary { path: PathBuf },
    #[error("正式研究入口无法启动锁定的 release 子角色构建")]
    ChildRoleBuildLaunch {
        #[source]
        source: std::io::Error,
    },
    #[error("锁定的 release 子角色构建失败：{stderr}")]
    ChildRoleBuildFailed { stderr: String },
    #[error("无法执行计时角色二进制 {path} 的角色描述")]
    TimingDescriptorLaunch {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("计时角色二进制 {path} 无法返回角色描述：{stderr}")]
    TimingDescriptorFailed { path: PathBuf, stderr: String },
    #[error("计时角色二进制 {path} 返回无效 JSON 角色描述")]
    InvalidTimingDescriptor {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("计时角色二进制 {path} 的角色、模式、记账状态或职责不符合正式协议")]
    UnexpectedTimingDescriptor { path: PathBuf },
    #[error("无法执行分配归因角色二进制 {path} 的角色描述")]
    AttributionDescriptorLaunch {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("分配归因角色二进制 {path} 无法返回角色描述：{stderr}")]
    AttributionDescriptorFailed { path: PathBuf, stderr: String },
    #[error("分配归因角色二进制 {path} 返回无效 JSON 角色描述")]
    InvalidAttributionDescriptor {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("分配归因角色二进制 {path} 的角色、模式、记账状态或职责不符合正式协议")]
    UnexpectedAttributionDescriptor { path: PathBuf },
    #[error("无法执行预言机角色二进制 {path} 的角色描述")]
    OracleDescriptorLaunch {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("预言机角色二进制 {path} 无法返回角色描述：{stderr}")]
    OracleDescriptorFailed { path: PathBuf, stderr: String },
    #[error("预言机角色二进制 {path} 返回无效 JSON 角色描述")]
    InvalidOracleDescriptor {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("预言机角色二进制 {path} 的角色、模式、记账状态或职责不符合正式协议")]
    UnexpectedOracleDescriptor { path: PathBuf },
    #[error("正式研究入口无法取得当前目录")]
    CurrentDirectory {
        #[source]
        source: std::io::Error,
    },
    #[error("输出路径没有父目录：{path}")]
    MissingOutputParent { path: PathBuf },
    #[error("输出父路径不是已存在目录：{path}")]
    OutputParentNotDirectory { path: PathBuf },
    #[error("输出路径没有文件名：{path}")]
    MissingOutputFileName { path: PathBuf },
    #[error("输出或检查点文件已存在，正式研究入口拒绝覆盖：{path}")]
    OutputAlreadyExists { path: PathBuf },
    #[error("分代检查点目录已存在，正式研究入口拒绝混用既有运行：{path}")]
    CheckpointDirectoryAlreadyExists { path: PathBuf },
    #[error("无法创建分代检查点目录 {path}")]
    CreateCheckpointDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("无法序列化正式研究执行检查点")]
    SerializeCheckpoint {
        #[source]
        source: serde_json::Error,
    },
    #[error("无法写入正式研究执行检查点 {path}")]
    WriteCheckpoint {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("无法把临时检查点 {source_path} 原子发布为 {destination_path}")]
    PublishCheckpoint {
        source_path: PathBuf,
        destination_path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("正式研究执行检查点序号溢出")]
    CheckpointSequenceOverflow,
    #[error("无法取得正式执行源提交")]
    SourceCommitLaunch {
        #[source]
        source: std::io::Error,
    },
    #[error("无法取得正式执行源提交：{stderr}")]
    SourceCommitFailed { stderr: String },
    #[error("正式执行源提交不是有效 UTF-8")]
    SourceCommitNotUtf8,
    #[error("正式执行源提交不是四十位十六进制 Git 对象名：{0}")]
    InvalidSourceCommit(String),
    #[error("无法读取正式执行来源制品 {path}")]
    ReadSourceArtifact {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("当前固定样例子进程报告的 pid 与父进程观察不一致")]
    CurrentFixturesChildPidMismatch,
    #[error("命令行选项必须为有效 UTF-8")]
    NonUtf8Option,
    #[error("--protocol 的值必须为有效 UTF-8")]
    NonUtf8Protocol,
    #[error("命令行选项 {option} 缺少值")]
    MissingOptionValue { option: String },
    #[error("命令行选项重复：{option}")]
    DuplicateOption { option: String },
    #[error("未知命令行选项：{option}")]
    UnknownOption { option: String },
    #[error("缺少必需命令行选项：{option}")]
    MissingRequiredOption { option: &'static str },
    #[error("不支持的正式研究协议 {actual:?}；只接受 {FORMAL_PROTOCOL_ID}")]
    UnsupportedProtocol { actual: String },
    #[error("--output 路径不能为空")]
    EmptyOutputPath,
    #[error("--environment 路径不能为空")]
    EmptyEnvironmentPath,
    #[error(transparent)]
    Environment(#[from] crate::EnvironmentError),
    #[error(transparent)]
    Contract(#[from] ContractError),
    #[error(transparent)]
    Pilot(#[from] PilotError),
    #[error(transparent)]
    FormalLadder(#[from] FormalLadderRunnerError),
    #[error(transparent)]
    CurrentFixtures(#[from] crate::CurrentFixturesError),
    #[error(transparent)]
    CurrentFixturesOracle(#[from] crate::CurrentFixturesOracleError),
    #[error(transparent)]
    Guard(#[from] crate::GuardError),
    #[error(transparent)]
    LimitQualification(#[from] LimitQualificationExecutionError),
    #[error(transparent)]
    CandidateMatrix(#[from] CandidateMatrixError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BASE_SCALE_PILOT_CHECKPOINT_SCHEMA, BASE_SCALE_PILOT_CHECKPOINT_SCHEMA_VERSION};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn exact_formal_arguments_are_required() {
        let request = parse_formal_protocol_arguments([
            OsString::from("--protocol"),
            OsString::from(FORMAL_PROTOCOL_ID),
            OsString::from("--output"),
            OsString::from("pilot.json"),
            OsString::from("--environment"),
            OsString::from("environment.json"),
        ])
        .expect("exact formal arguments");
        assert_eq!(request.output_path, PathBuf::from("pilot.json"));
        assert_eq!(request.environment_path, PathBuf::from("environment.json"));

        assert!(matches!(
            parse_formal_protocol_arguments([
                OsString::from("--protocol"),
                OsString::from("other"),
                OsString::from("--output"),
                OsString::from("pilot.json"),
                OsString::from("--environment"),
                OsString::from("environment.json"),
            ]),
            Err(FormalProtocolError::UnsupportedProtocol { .. })
        ));
        assert!(matches!(
            parse_formal_protocol_arguments([
                OsString::from("--protocol"),
                OsString::from(FORMAL_PROTOCOL_ID),
            ]),
            Err(FormalProtocolError::MissingRequiredOption { option: "--output" })
        ));
        assert!(matches!(
            parse_formal_protocol_arguments([
                OsString::from("--protocol"),
                OsString::from(FORMAL_PROTOCOL_ID),
                OsString::from("--output"),
                OsString::from("a.json"),
                OsString::from("--output"),
                OsString::from("b.json"),
                OsString::from("--environment"),
                OsString::from("environment.json"),
            ]),
            Err(FormalProtocolError::DuplicateOption { .. })
        ));
    }

    #[test]
    fn formal_mode_fails_closed_for_debug_or_incomplete_features() {
        assert!(matches!(
            verify_formal_build_mode(true, true),
            Err(FormalProtocolError::DebugBuild)
        ));
        assert!(matches!(
            verify_formal_build_mode(false, false),
            Err(FormalProtocolError::MissingFullRunnerFeature)
        ));
        verify_formal_build_mode(false, true).expect("release full runner");
    }

    #[test]
    fn clean_worktree_status_requires_success_and_empty_porcelain_output() {
        validate_git_status_result(true, b"", b"").expect("clean status");
        assert!(matches!(
            validate_git_status_result(true, b" M src/lib.rs\n", b""),
            Err(FormalProtocolError::DirtyWorktree { .. })
        ));
        assert!(matches!(
            validate_git_status_result(false, b"", b"fatal"),
            Err(FormalProtocolError::GitStatusFailed { .. })
        ));
    }

    #[test]
    fn checkpoints_are_immutable_and_final_output_is_atomic() {
        let directory = temporary_directory("checkpoint-writer");
        let output = directory.join("formal-execution.json");
        let mut writer = FormalCheckpointWriter::prepare(&output).expect("prepare writer");
        let checkpoint = empty_checkpoint();

        writer.persist(&checkpoint).expect("persist checkpoint");
        writer.finish(&checkpoint).expect("publish final output");

        assert!(output.is_file());
        let published: serde_json::Value =
            serde_json::from_slice(&fs::read(&output).expect("read published formal checkpoint"))
                .expect("parse published formal checkpoint");
        assert_eq!(
            published["schema"],
            serde_json::json!(FORMAL_PROTOCOL_CHECKPOINT_SCHEMA)
        );
        assert_eq!(
            published["baseScalePilot"]["schema"],
            serde_json::json!(BASE_SCALE_PILOT_CHECKPOINT_SCHEMA)
        );
        assert_eq!(published["formalLadders"], serde_json::json!([]));
        assert!(
            writer
                .checkpoint_directory
                .join("checkpoint-00000000.json")
                .is_file()
        );
        assert!(matches!(
            FormalCheckpointWriter::prepare(&output),
            Err(FormalProtocolError::OutputAlreadyExists { .. })
        ));

        fs::remove_dir_all(directory).expect("remove test directory");
    }

    fn empty_checkpoint() -> FormalProtocolCheckpoint {
        formal_checkpoint(
            &test_source(),
            &test_environment(),
            &FormalCurrentFixtureProjection {
                status: RunStatus::Valid,
                invalidation_reasons: Vec::new(),
                process: ProcessObservation::guarded_before_start(1, ORACLE_BINARY_ID),
                child: None,
                monitor: ChildProcessMonitorReport {
                    observation_count: 0,
                    last_private_bytes: crate::NullableObservation::unavailable(
                        "child-not-started",
                    ),
                    peak_private_bytes: crate::NullableObservation::unavailable(
                        "child-not-started",
                    ),
                    last_available_physical_memory_bytes: crate::NullableObservation::unavailable(
                        "child-not-started",
                    ),
                    elapsed_wall_time_ns: 0,
                    trigger: None,
                },
                external_state: None,
                kill_error: None,
                monitor_error: None,
                stderr: String::new(),
            },
            &BaseScalePilotCheckpoint {
                schema: BASE_SCALE_PILOT_CHECKPOINT_SCHEMA.to_owned(),
                schema_version: BASE_SCALE_PILOT_CHECKPOINT_SCHEMA_VERSION,
                protocol_id: FORMAL_PROTOCOL_ID.to_owned(),
                clock_quantum_ns: 1,
                required_median_wall_time_ns: 10_000,
                selections: Vec::new(),
                active_selection: None,
                runs: Vec::new(),
                oracle_runs: Vec::new(),
            },
            &[],
            None,
            None,
            None,
            None,
        )
    }

    fn test_source() -> FormalSourceSnapshot {
        FormalSourceSnapshot {
            source_commit: "1".repeat(40),
            harness_commit: "1".repeat(40),
            dirty: false,
            cargo_lock_sha256: "2".repeat(64),
            binaries: [TIMING_BINARY_ID, ATTRIBUTION_BINARY_ID, ORACLE_BINARY_ID]
                .into_iter()
                .map(|binary_id| FormalBinarySourceSnapshot {
                    binary_id: binary_id.to_owned(),
                    sha256: "3".repeat(64),
                })
                .collect(),
        }
    }

    fn test_environment() -> FormalEnvironmentSnapshot {
        FormalEnvironmentSnapshot {
            os: "test-os".to_owned(),
            os_build: "test-build".to_owned(),
            cpu: "test-cpu".to_owned(),
            logical_processor_count: 1,
            physical_memory_bytes: 1,
            target_triple: "test-target".to_owned(),
            rustc: "test-rustc".to_owned(),
            llvm: "test-llvm".to_owned(),
            power_source: "ac".to_owned(),
            vendor_performance_mode: "test-mode".to_owned(),
            power_plan: "test-plan".to_owned(),
            bios_firmware: "test-bios".to_owned(),
            monitoring_provider: "test-monitor".to_owned(),
            background_process_audit: Vec::new(),
            operator_declaration: crate::FormalEnvironmentDeclaration {
                vendor_performance_mode: "test-mode".to_owned(),
                bios_firmware: "test-bios".to_owned(),
                sleep_or_session_lock_observed: false,
                thermal_or_power_throttling_observed: false,
            },
        }
    }

    fn temporary_directory(label: &str) -> PathBuf {
        let ordinal = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "laneflow-issue-308-{label}-{}-{ordinal}",
            std::process::id()
        ));
        fs::create_dir(&directory).expect("create test directory");
        directory
    }
}
