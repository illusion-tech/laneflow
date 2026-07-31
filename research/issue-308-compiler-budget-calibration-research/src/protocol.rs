//! `compiler-calibration-v1` 正式研究入口与执行检查点写出。
//!
//! runner 先完成基础规模试运行，再执行正式规模阶梯并保存原始子进程、精确汇总、
//! 拐点和规模选择。该检查点不冒充编译器校准证据 v1；完整证据封套、来源摘要和
//! 独立 Evidence 验证器由后续切片建立。

use crate::{
    ATTRIBUTION_BINARY_ID, BaseScalePilotCheckpoint, ContractError, FORMAL_PROTOCOL_ID,
    FormalLadderExecution, FormalLadderRunnerError, ORACLE_BINARY_ID, PilotError, TIMING_BINARY_ID,
    load_repository_contract, repository_root, run_base_scale_pilot_discovery_with_checkpoint_sink,
    run_formal_ladders,
};
use serde::Serialize;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormalProtocolRequest {
    pub output_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
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
}

pub const FORMAL_PROTOCOL_CHECKPOINT_SCHEMA: &str =
    "laneflow.compiler-calibration-formal-execution-checkpoint";
pub const FORMAL_PROTOCOL_CHECKPOINT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormalProtocolCheckpoint {
    pub schema: String,
    pub schema_version: u32,
    pub protocol_id: String,
    pub base_scale_pilot: BaseScalePilotCheckpoint,
    pub formal_ladders: Vec<FormalLadderExecution>,
    pub active_formal_ladder: Option<FormalLadderExecution>,
}

pub fn parse_formal_protocol_arguments(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<FormalProtocolRequest, FormalProtocolError> {
    let mut protocol = None;
    let mut output_path = None;
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
    Ok(FormalProtocolRequest { output_path })
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
    let timing_executable = resolve_sibling_timing_binary()?;
    let attribution_executable = resolve_sibling_attribution_binary()?;
    let oracle_executable = resolve_sibling_oracle_binary()?;
    verify_timing_binary_role(&timing_executable)?;
    verify_attribution_binary_role(&attribution_executable)?;
    verify_oracle_binary_role(&oracle_executable)?;
    let mut writer = FormalCheckpointWriter::prepare(&request.output_path)?;
    let base_scale_pilot = run_base_scale_pilot_discovery_with_checkpoint_sink(
        &trusted,
        &timing_executable,
        &oracle_executable,
        |base_scale_pilot| {
            writer
                .persist(&formal_checkpoint(base_scale_pilot, &[], None))
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
                .persist(&formal_checkpoint(&base_scale_pilot, completed, active))
                .map_err(|error| FormalLadderRunnerError::CheckpointPersistence {
                    detail: error.to_string(),
                })
        },
    )?;
    let checkpoint = formal_checkpoint(&base_scale_pilot, &formal_ladders, None);
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
    })
}

fn formal_checkpoint(
    base_scale_pilot: &BaseScalePilotCheckpoint,
    formal_ladders: &[FormalLadderExecution],
    active_formal_ladder: Option<&FormalLadderExecution>,
) -> FormalProtocolCheckpoint {
    FormalProtocolCheckpoint {
        schema: FORMAL_PROTOCOL_CHECKPOINT_SCHEMA.to_owned(),
        schema_version: FORMAL_PROTOCOL_CHECKPOINT_SCHEMA_VERSION,
        protocol_id: FORMAL_PROTOCOL_ID.to_owned(),
        base_scale_pilot: base_scale_pilot.clone(),
        formal_ladders: formal_ladders.to_vec(),
        active_formal_ladder: active_formal_ladder.cloned(),
    }
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
    #[error(transparent)]
    Contract(#[from] ContractError),
    #[error(transparent)]
    Pilot(#[from] PilotError),
    #[error(transparent)]
    FormalLadder(#[from] FormalLadderRunnerError),
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
        ])
        .expect("exact formal arguments");
        assert_eq!(request.output_path, PathBuf::from("pilot.json"));

        assert!(matches!(
            parse_formal_protocol_arguments([
                OsString::from("--protocol"),
                OsString::from("other"),
                OsString::from("--output"),
                OsString::from("pilot.json"),
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
        )
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
