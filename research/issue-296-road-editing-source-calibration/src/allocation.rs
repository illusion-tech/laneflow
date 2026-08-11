//! #296 G3 的独立 heap instrumentation 角色。
//!
//! 该角色在独立 release 进程中运行，不参与 80 个正式计时样本。来源 buffer、旧 accepted
//! revision 等共存对象在 profiler 启动前构造；因此 `operation_heap` 只观察候选操作新增的
//! heap。production `CompilationMetrics` 仍是 compiler-controlled 账本权威，allocator 数据
//! 只用于证明成功编译路径的实际新增 heap 没有超过该保守账本。

use std::hint::black_box;
use std::path::Path;
use std::process::Command;

use laneflow_compiler::{CompileLimits, GeometryAccuracyProfile, GeometryDirectionProfile};
use serde::{Deserialize, Serialize};

use crate::evidence::metrics_record;
use crate::{
    EvidenceMetrics, GeneratorError, build_base_modules_from_seed,
    build_regularity_probe_modules_from_seed, build_rewrite_candidate_module_from_seed,
    compile_encoded_modules, compile_rewrite_candidate_modules, encode_module, encode_modules,
    load_p100_seed,
};

pub const ALLOCATOR_PROBE_SCHEMA: &str = "laneflow.road-editing-source-calibration-allocator-probe";
pub const ALLOCATOR_PROBE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AllocatorProbeRole {
    BaseCompleteCompile,
    RegularityCompleteCompile,
    RewriteCandidateBuildEncode,
    RewriteCandidateCompleteCompile,
}

impl AllocatorProbeRole {
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::BaseCompleteCompile => "base-complete-compile",
            Self::RegularityCompleteCompile => "regularity-complete-compile",
            Self::RewriteCandidateBuildEncode => "rewrite-candidate-build-encode",
            Self::RewriteCandidateCompleteCompile => "rewrite-candidate-complete-compile",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "base-complete-compile" => Some(Self::BaseCompleteCompile),
            "regularity-complete-compile" => Some(Self::RegularityCompleteCompile),
            "rewrite-candidate-build-encode" => Some(Self::RewriteCandidateBuildEncode),
            "rewrite-candidate-complete-compile" => Some(Self::RewriteCandidateCompleteCompile),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllocatorProbeRequest {
    pub role: AllocatorProbeRole,
    pub argv: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AllocatorProbe {
    pub schema: String,
    pub schema_version: u32,
    pub role: AllocatorProbeRole,
    pub workload: String,
    pub accuracy_profile_code: u8,
    pub direction_profile_code: u8,
    pub measurement_commit: String,
    pub argv: Vec<String>,
    pub preloaded_revision: PreloadedRevisionObservation,
    pub operation_heap: AllocatorHeapObservation,
    pub candidate_source_buffer_retained_capacity_bytes: u64,
    pub production_metrics: Option<EvidenceMetrics>,
    pub compiler_ledger_covers_observed_operation_peak: Option<bool>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreloadedRevisionObservation {
    pub source_buffer_retained_capacity_bytes: u64,
    pub output_logical_bytes: u64,
    pub candidate_source_buffer_retained_capacity_bytes: u64,
    pub conservative_coexisting_bytes: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AllocatorHeapObservation {
    pub total_allocation_count: u64,
    pub total_allocated_bytes: u64,
    pub peak_live_allocation_count: u64,
    pub peak_live_bytes: u64,
    pub end_live_allocation_count: u64,
    pub end_live_bytes: u64,
}

pub fn run_allocator_probe(
    repository_root: &Path,
    request: &AllocatorProbeRequest,
) -> Result<AllocatorProbe, GeneratorError> {
    let measurement_commit = clean_measurement_commit(repository_root)?;
    match request.role {
        AllocatorProbeRole::BaseCompleteCompile => {
            run_complete_compile_probe(repository_root, request, measurement_commit, false)
        }
        AllocatorProbeRole::RegularityCompleteCompile => {
            run_complete_compile_probe(repository_root, request, measurement_commit, true)
        }
        AllocatorProbeRole::RewriteCandidateBuildEncode => {
            run_rewrite_build_encode_probe(repository_root, request, measurement_commit)
        }
        AllocatorProbeRole::RewriteCandidateCompleteCompile => {
            run_rewrite_compile_probe(repository_root, request, measurement_commit)
        }
    }
}

fn run_complete_compile_probe(
    repository_root: &Path,
    request: &AllocatorProbeRequest,
    measurement_commit: String,
    regularity: bool,
) -> Result<AllocatorProbe, GeneratorError> {
    let accuracy = GeometryAccuracyProfile::Fine2Cm;
    let direction = GeometryDirectionProfile::Smooth1Deg;
    let limits = CompileLimits::p100_initial_v2();
    let seed = load_p100_seed(repository_root)?;
    let modules = if regularity {
        build_regularity_probe_modules_from_seed(seed, &limits)?
    } else {
        build_base_modules_from_seed(seed, accuracy, direction, &limits)?
    };
    let encoded = encode_modules(modules, &limits)?;
    let source_retained = retained_capacity_sum(&encoded)?;
    let preloaded = preloaded_revision(source_retained, 0, 0)?;

    let profiler = heap_profiler();
    let output = compile_encoded_modules(&encoded, limits)?;
    black_box((&encoded, &output));
    let heap = heap_observation();
    let production = output.metrics();
    let covered = heap.peak_live_bytes <= production.compiler_controlled_peak_bytes();
    drop(profiler);
    if !covered {
        return Err(contract(format!(
            "{} allocator peak {} exceeds compiler-controlled ledger {}",
            request.role.id(),
            heap.peak_live_bytes,
            production.compiler_controlled_peak_bytes()
        )));
    }
    Ok(AllocatorProbe {
        schema: ALLOCATOR_PROBE_SCHEMA.to_owned(),
        schema_version: ALLOCATOR_PROBE_SCHEMA_VERSION,
        role: request.role,
        workload: if regularity {
            "LF-ROAD-EDITING-P100-REGULARITY-v1"
        } else {
            "LF-ROAD-EDITING-P100-v1"
        }
        .to_owned(),
        accuracy_profile_code: accuracy as u8,
        direction_profile_code: direction as u8,
        measurement_commit,
        argv: request.argv.clone(),
        preloaded_revision: preloaded,
        operation_heap: heap,
        candidate_source_buffer_retained_capacity_bytes: 0,
        production_metrics: Some(metrics_record(production)),
        compiler_ledger_covers_observed_operation_peak: Some(true),
    })
}

fn run_rewrite_build_encode_probe(
    repository_root: &Path,
    request: &AllocatorProbeRequest,
    measurement_commit: String,
) -> Result<AllocatorProbe, GeneratorError> {
    let accuracy = GeometryAccuracyProfile::Balanced5Cm;
    let direction = GeometryDirectionProfile::Balanced2Deg;
    let limits = CompileLimits::p100_initial_v2();
    let accepted_seed = load_p100_seed(repository_root)?;
    let candidate_seed = load_p100_seed(repository_root)?;
    let accepted_modules =
        build_base_modules_from_seed(accepted_seed, accuracy, direction, &limits)?;
    let accepted_encoded = encode_modules(accepted_modules, &limits)?;
    let accepted_output = compile_encoded_modules(&accepted_encoded, limits.clone())?;
    let source_retained = retained_capacity_sum(&accepted_encoded)?;
    let output_logical = accepted_output.metrics().output_logical_bytes();
    let preloaded = preloaded_revision(source_retained, output_logical, 0)?;

    let profiler = heap_profiler();
    let candidate_module = build_rewrite_candidate_module_from_seed(candidate_seed, &limits)?;
    let candidate_encoded = encode_module(candidate_module, &limits)?;
    black_box((&accepted_encoded, &accepted_output, &candidate_encoded));
    let heap = heap_observation();
    let candidate_retained = usize_u64(candidate_encoded.retained_capacity_bytes());
    drop(profiler);

    Ok(AllocatorProbe {
        schema: ALLOCATOR_PROBE_SCHEMA.to_owned(),
        schema_version: ALLOCATOR_PROBE_SCHEMA_VERSION,
        role: request.role,
        workload: "LF-ROAD-EDITING-P100-v1".to_owned(),
        accuracy_profile_code: accuracy as u8,
        direction_profile_code: direction as u8,
        measurement_commit,
        argv: request.argv.clone(),
        preloaded_revision: preloaded,
        operation_heap: heap,
        candidate_source_buffer_retained_capacity_bytes: candidate_retained,
        production_metrics: None,
        compiler_ledger_covers_observed_operation_peak: None,
    })
}

fn run_rewrite_compile_probe(
    repository_root: &Path,
    request: &AllocatorProbeRequest,
    measurement_commit: String,
) -> Result<AllocatorProbe, GeneratorError> {
    let accuracy = GeometryAccuracyProfile::Balanced5Cm;
    let direction = GeometryDirectionProfile::Balanced2Deg;
    let limits = CompileLimits::p100_initial_v2();
    let accepted_seed = load_p100_seed(repository_root)?;
    let candidate_seed = load_p100_seed(repository_root)?;
    let accepted_modules =
        build_base_modules_from_seed(accepted_seed, accuracy, direction, &limits)?;
    let accepted_encoded = encode_modules(accepted_modules, &limits)?;
    let accepted_output = compile_encoded_modules(&accepted_encoded, limits.clone())?;
    let candidate_module = build_rewrite_candidate_module_from_seed(candidate_seed, &limits)?;
    let candidate_encoded = encode_module(candidate_module, &limits)?;
    let source_retained = retained_capacity_sum(&accepted_encoded)?;
    let output_logical = accepted_output.metrics().output_logical_bytes();
    let candidate_retained = usize_u64(candidate_encoded.retained_capacity_bytes());
    let preloaded = preloaded_revision(source_retained, output_logical, candidate_retained)?;

    let profiler = heap_profiler();
    let candidate_output =
        compile_rewrite_candidate_modules(&accepted_encoded, &candidate_encoded, limits)?;
    black_box((
        &accepted_encoded,
        &accepted_output,
        &candidate_encoded,
        &candidate_output,
    ));
    let heap = heap_observation();
    let production = candidate_output.metrics();
    let covered = heap.peak_live_bytes <= production.compiler_controlled_peak_bytes();
    drop(profiler);
    if !covered {
        return Err(contract(format!(
            "rewrite candidate allocator peak {} exceeds compiler-controlled ledger {}",
            heap.peak_live_bytes,
            production.compiler_controlled_peak_bytes()
        )));
    }

    Ok(AllocatorProbe {
        schema: ALLOCATOR_PROBE_SCHEMA.to_owned(),
        schema_version: ALLOCATOR_PROBE_SCHEMA_VERSION,
        role: request.role,
        workload: "LF-ROAD-EDITING-P100-v1".to_owned(),
        accuracy_profile_code: accuracy as u8,
        direction_profile_code: direction as u8,
        measurement_commit,
        argv: request.argv.clone(),
        preloaded_revision: preloaded,
        operation_heap: heap,
        candidate_source_buffer_retained_capacity_bytes: candidate_retained,
        production_metrics: Some(metrics_record(production)),
        compiler_ledger_covers_observed_operation_peak: Some(true),
    })
}

fn heap_profiler() -> dhat::Profiler {
    dhat::Profiler::builder()
        .testing()
        .trim_backtraces(Some(4))
        .build()
}

fn heap_observation() -> AllocatorHeapObservation {
    let stats = dhat::HeapStats::get();
    AllocatorHeapObservation {
        total_allocation_count: stats.total_blocks,
        total_allocated_bytes: stats.total_bytes,
        peak_live_allocation_count: usize_u64(stats.max_blocks),
        peak_live_bytes: usize_u64(stats.max_bytes),
        end_live_allocation_count: usize_u64(stats.curr_blocks),
        end_live_bytes: usize_u64(stats.curr_bytes),
    }
}

fn retained_capacity_sum(modules: &[crate::EncodedP100Module]) -> Result<u64, GeneratorError> {
    modules.iter().try_fold(0_u64, |total, module| {
        total
            .checked_add(usize_u64(module.retained_capacity_bytes()))
            .ok_or_else(|| contract("source retained-capacity sum overflow"))
    })
}

fn preloaded_revision(
    source_buffer_retained_capacity_bytes: u64,
    output_logical_bytes: u64,
    candidate_source_buffer_retained_capacity_bytes: u64,
) -> Result<PreloadedRevisionObservation, GeneratorError> {
    let conservative_coexisting_bytes = source_buffer_retained_capacity_bytes
        .checked_add(output_logical_bytes)
        .and_then(|value| value.checked_add(candidate_source_buffer_retained_capacity_bytes))
        .ok_or_else(|| contract("preloaded revision byte sum overflow"))?;
    Ok(PreloadedRevisionObservation {
        source_buffer_retained_capacity_bytes,
        output_logical_bytes,
        candidate_source_buffer_retained_capacity_bytes,
        conservative_coexisting_bytes,
    })
}

fn clean_measurement_commit(repository_root: &Path) -> Result<String, GeneratorError> {
    let commit = command_text(repository_root, ["rev-parse", "HEAD"], "git rev-parse HEAD")?;
    if commit.len() != 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(contract(
            "allocator probe commit is not a full Git object id",
        ));
    }
    let status = command_text(repository_root, ["status", "--porcelain=v1"], "git status")?;
    if !status.is_empty() {
        return Err(contract("allocator probe requires a clean worktree"));
    }
    Ok(commit)
}

fn command_text<const N: usize>(
    repository_root: &Path,
    arguments: [&str; N],
    label: &str,
) -> Result<String, GeneratorError> {
    let output = Command::new("git")
        .current_dir(repository_root)
        .args(arguments)
        .output()
        .map_err(|error| contract(format!("cannot run {label}: {error}")))?;
    if !output.status.success() {
        return Err(contract(format!(
            "{label} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|error| contract(format!("{label} returned non-UTF-8 output: {error}")))
}

fn usize_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn contract(message: impl Into<String>) -> GeneratorError {
    GeneratorError::Contract(message.into())
}
