//! #296 道路编辑正式校准的 fresh-process 编排与原始记录。

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use laneflow_compiler::{GeometryAccuracyProfile, GeometryDirectionProfile};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    ALLOCATOR_PROBE_SCHEMA, ALLOCATOR_PROBE_SCHEMA_VERSION, AllocatorProbe, AllocatorProbeRole,
    EvidenceFixture, EvidenceMetrics, EvidenceSample, EvidenceSampleKind, EvidenceWorkload,
    GeometryObservation, P100_PROFILE_COMBINATIONS, SAMPLE_SCHEMA, SAMPLE_SCHEMA_VERSION,
    SingleModuleRewriteEvidence,
};

pub const RAW_EVIDENCE_SCHEMA: &str = "laneflow.road-editing-source-calibration-raw";
pub const RAW_EVIDENCE_SCHEMA_VERSION: u32 = 1;
const FORMAL_SAMPLE_COUNT: u8 = 7;
const HARDWARE_ID: &str = "LF-P100-REF-01";
const EXPECTED_HARDWARE_IDENTITY: &str =
    "be3637be955f6c2c9e9e55b80419794adfac64b709d573602a37da9a8672fd20";
const EXPECTED_REFERENCE_MACHINE_SHA256: &str =
    "7e40530497dd57950cbe7b85d4f1a4510da1c1331ca6efe7e4b54dceebe98c2e";
const EXPECTED_SCHEMA_SHA256: &str =
    "173c25751659124e7a9c9835b59606aed0838b62a944db62fe99be9345f9a268";
const EXPECTED_SEED_SHA256: &str =
    "05a32c19f3fe4ab8f7ea176d996a505688f875197433ab7f83d629ef5d560ce2";
const EXPECTED_WORKLOAD_SHA256: &str =
    "0b7ef419af05e0eac08b67d551f207bed510b7bdaca079184cef12c386672f5a";
const EXPECTED_COMPACT_EVIDENCE_SCHEMA_SHA256: &str =
    "d741b2235de123fc3466d6917a0942472ade03cddd39b17740b1d59437f6240a";
const BALANCED_POWER_PLAN_GUID: &str = "381b4222-f694-41f0-9685-ff5bb260df2e";
const SAMPLE_OUTPUT_ROOT: &str = "target/road-editing-evidence";
const PACKAGE: &str = "issue-296-road-editing-source-calibration";
const RUSTUP: &str = "rustup";
const RUST_TOOLCHAIN: &str = "1.96.0";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawEvidence {
    pub schema: String,
    pub schema_version: u32,
    pub source: EvidenceSource,
    pub environment: EvidenceEnvironment,
    pub protocol: EvidenceProtocol,
    pub bindings: Vec<EvidenceFileBinding>,
    pub invocations: Vec<EvidenceInvocation>,
    pub samples: Vec<EvidenceSample>,
    pub allocator_probe_invocations: Vec<AllocatorProbeInvocation>,
    pub allocator_probes: Vec<AllocatorProbe>,
    pub summaries: Vec<EvidenceSummary>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceSource {
    pub measurement_commit: String,
    pub generator_contract_revision: String,
    pub writer_implementation_revision: String,
    pub compiler_implementation_revision: String,
    pub worktree_clean: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceEnvironment {
    pub hardware_id: String,
    pub hardware_identity_sha256: String,
    pub cpu: String,
    pub physical_core_count: u32,
    pub logical_processor_count: u32,
    pub physical_memory_bytes: u64,
    pub operating_system: String,
    pub operating_system_build: String,
    pub bios_firmware: String,
    pub power_source: String,
    pub power_plan_guid: String,
    pub power_plan_description: String,
    pub rustc_verbose_version: String,
    pub cargo_version: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceProtocol {
    pub release: bool,
    pub locked: bool,
    pub worker_thread_count: u8,
    pub warmup_count_per_workload_profile: u8,
    pub formal_sample_count_per_workload_profile: u8,
    pub outlier_deletion: String,
    pub primary_statistic: String,
    pub dispersion_statistic: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceFileBinding {
    pub path: String,
    pub byte_length: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceInvocation {
    pub workload: String,
    pub accuracy_profile_code: u8,
    pub direction_profile_code: u8,
    pub sample_kind: EvidenceSampleKind,
    pub sample_index: u8,
    pub argv: Vec<String>,
    pub output_path: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AllocatorProbeInvocation {
    pub role: AllocatorProbeRole,
    pub argv: Vec<String>,
    pub output_path: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceSummary {
    pub workload: String,
    pub accuracy_profile_code: u8,
    pub direction_profile_code: u8,
    pub formal_sample_count: u8,
    pub timings_ns: EvidenceTimingSummary,
    pub fixtures: Vec<EvidenceFixture>,
    pub metrics: EvidenceMetrics,
    pub geometry_observation: GeometryObservation,
    pub single_module_rewrite: Option<SingleModuleRewriteSummary>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SingleModuleRewriteSummary {
    pub edited_module: String,
    pub target_kind: String,
    pub target_local_key: String,
    pub initial_start_width_f64_bits: String,
    pub initial_end_width_f64_bits: String,
    pub candidate_start_width_f64_bits: String,
    pub candidate_end_width_f64_bits: String,
    pub old_fixture: EvidenceFixture,
    pub candidate_fixture: EvidenceFixture,
    pub unmodified_modules: Vec<EvidenceFixture>,
    pub unmodified_module_byte_identity: bool,
    pub old_source_buffer_retained_capacity_bytes: u64,
    pub candidate_source_buffer_retained_capacity_bytes: u64,
    pub post_commit_retained_capacity_bytes: u64,
    pub timings_ns: SingleModuleRewriteTimingSummary,
    pub old_metrics: EvidenceMetrics,
    pub candidate_metrics: EvidenceMetrics,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SingleModuleRewriteTimingSummary {
    pub candidate_typed_model_build: MedianMad,
    pub candidate_encode: MedianMad,
    pub candidate_complete_compile: MedianMad,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceTimingSummary {
    pub typed_model_build: MedianMad,
    pub encode: MedianMad,
    pub size_prefix_and_identifier_preflight: MedianMad,
    pub flatbuffers_verifier: MedianMad,
    pub semantic_preflight_and_typed_ast_lowering: MedianMad,
    pub complete_compile: MedianMad,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MedianMad {
    pub median: u64,
    pub median_absolute_deviation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WorkloadProfile {
    workload: EvidenceWorkload,
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
}

/// 在冻结参考机上串行启动 80 个 fresh process，并返回完整原始记录。
///
/// 每个 workload/profile 有一个 warmup 和七个正式样本；不并发、不删异常值。调用方在
/// 本函数返回后以 create-new 语义写出 raw evidence，不能覆盖已有记录。
pub fn run_evidence_protocol(repository_root: &Path) -> Result<RawEvidence, String> {
    let source = source_identity(repository_root)?;
    let environment = capture_environment(repository_root)?;
    let bindings = bindings(repository_root)?;
    let sample_root = repository_root
        .join(SAMPLE_OUTPUT_ROOT)
        .join(&source.measurement_commit)
        .join("samples");
    if sample_root.exists() {
        return Err(format!(
            "fresh-process sample directory already exists: {}",
            sample_root.display()
        ));
    }
    fs::create_dir_all(&sample_root).map_err(|error| {
        format!(
            "cannot create sample directory {}: {error}",
            sample_root.display()
        )
    })?;

    let mut invocations = Vec::with_capacity(80);
    let mut samples = Vec::with_capacity(80);
    for profile in workload_profiles() {
        run_one_sample(
            repository_root,
            &sample_root,
            &profile,
            EvidenceSampleKind::Warmup,
            0,
            &mut invocations,
            &mut samples,
        )?;
        for sample_index in 1..=FORMAL_SAMPLE_COUNT {
            run_one_sample(
                repository_root,
                &sample_root,
                &profile,
                EvidenceSampleKind::Formal,
                sample_index,
                &mut invocations,
                &mut samples,
            )?;
        }
    }
    let probe_root = sample_root
        .parent()
        .expect("sample root always has a measurement-commit parent")
        .join("allocator-probes");
    let (allocator_probe_invocations, allocator_probes) =
        run_all_allocator_probes(repository_root, &probe_root, &source.measurement_commit)?;
    let summaries = summarize(&samples)?;
    let evidence = RawEvidence {
        schema: RAW_EVIDENCE_SCHEMA.to_owned(),
        schema_version: RAW_EVIDENCE_SCHEMA_VERSION,
        source,
        environment,
        protocol: EvidenceProtocol {
            release: true,
            locked: true,
            worker_thread_count: 1,
            warmup_count_per_workload_profile: 1,
            formal_sample_count_per_workload_profile: FORMAL_SAMPLE_COUNT,
            outlier_deletion: "forbidden".to_owned(),
            primary_statistic: "median-of-seven-formal-samples".to_owned(),
            dispersion_statistic: "median-absolute-deviation-of-same-seven".to_owned(),
        },
        bindings,
        invocations,
        samples,
        allocator_probe_invocations,
        allocator_probes,
        summaries,
    };
    validate_raw_evidence(repository_root, &evidence)?;
    Ok(evidence)
}

/// 从 exact measurement commit 独立重读全部绑定并重算 raw evidence 的跨记录关系。
///
/// 验证不信任当前工作树中的 schema/workload bytes；所有冻结输入均通过
/// `git show <measurement-commit>:<path>` 取得。
pub fn validate_raw_evidence(repository_root: &Path, evidence: &RawEvidence) -> Result<(), String> {
    if evidence.schema != RAW_EVIDENCE_SCHEMA
        || evidence.schema_version != RAW_EVIDENCE_SCHEMA_VERSION
        || evidence.source.measurement_commit.len() != 40
        || !evidence
            .source
            .measurement_commit
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || evidence.source.generator_contract_revision != evidence.source.measurement_commit
        || evidence.source.writer_implementation_revision != evidence.source.measurement_commit
        || evidence.source.compiler_implementation_revision != evidence.source.measurement_commit
        || !evidence.source.worktree_clean
    {
        return Err("raw evidence source identity is invalid".to_owned());
    }
    if evidence.environment.hardware_id != HARDWARE_ID
        || evidence.environment.hardware_identity_sha256 != EXPECTED_HARDWARE_IDENTITY
        || evidence.environment.power_source != "AC"
        || evidence.environment.power_plan_guid != BALANCED_POWER_PLAN_GUID
        || !evidence
            .environment
            .rustc_verbose_version
            .starts_with("rustc 1.96.0 (ac68faa20 2026-05-25)")
        || !evidence
            .environment
            .cargo_version
            .starts_with("cargo 1.96.0 ")
    {
        return Err("raw evidence reference environment is invalid".to_owned());
    }
    if !evidence.protocol.release
        || !evidence.protocol.locked
        || evidence.protocol.worker_thread_count != 1
        || evidence.protocol.warmup_count_per_workload_profile != 1
        || evidence.protocol.formal_sample_count_per_workload_profile != FORMAL_SAMPLE_COUNT
        || evidence.protocol.outlier_deletion != "forbidden"
        || evidence.protocol.primary_statistic != "median-of-seven-formal-samples"
        || evidence.protocol.dispersion_statistic != "median-absolute-deviation-of-same-seven"
    {
        return Err("raw evidence protocol does not match the frozen contract".to_owned());
    }
    validate_bindings_at_commit(repository_root, evidence)?;

    let profiles = workload_profiles();
    let expected_sample_count = profiles
        .len()
        .checked_mul(usize::from(FORMAL_SAMPLE_COUNT + 1))
        .ok_or_else(|| "raw evidence sample count overflow".to_owned())?;
    if evidence.invocations.len() != expected_sample_count
        || evidence.samples.len() != expected_sample_count
        || evidence.summaries.len() != profiles.len()
    {
        return Err("raw evidence does not contain the exact 80-process matrix".to_owned());
    }
    let mut ordinal = 0_usize;
    let mut output_paths = std::collections::BTreeSet::new();
    for profile in &profiles {
        for sample_index in 0..=FORMAL_SAMPLE_COUNT {
            let sample_kind = if sample_index == 0 {
                EvidenceSampleKind::Warmup
            } else {
                EvidenceSampleKind::Formal
            };
            let invocation = evidence
                .invocations
                .get(ordinal)
                .ok_or_else(|| "raw evidence invocation is missing".to_owned())?;
            let sample = evidence
                .samples
                .get(ordinal)
                .ok_or_else(|| "raw evidence sample is missing".to_owned())?;
            validate_sample(sample, profile, sample_kind, sample_index)?;
            if invocation.workload != profile.workload.id()
                || invocation.accuracy_profile_code != profile.accuracy as u8
                || invocation.direction_profile_code != profile.direction as u8
                || invocation.sample_kind != sample_kind
                || invocation.sample_index != sample_index
                || invocation.argv.last() != Some(&invocation.output_path)
                || sample.argv.last() != Some(&invocation.output_path)
                || !output_paths.insert(invocation.output_path.clone())
            {
                return Err("raw invocation does not bind its unique fresh sample".to_owned());
            }
            validate_repository_relative_json_path(&invocation.output_path)?;
            ordinal = ordinal
                .checked_add(1)
                .ok_or_else(|| "raw evidence ordinal overflow".to_owned())?;
        }
    }
    let roles = allocator_probe_roles();
    if evidence.allocator_probe_invocations.len() != roles.len()
        || evidence.allocator_probes.len() != roles.len()
    {
        return Err("raw evidence does not contain the exact four allocator probes".to_owned());
    }
    for ((invocation, probe), role) in evidence
        .allocator_probe_invocations
        .iter()
        .zip(&evidence.allocator_probes)
        .zip(roles)
    {
        if invocation.role != role
            || invocation.argv.last() != Some(&invocation.output_path)
            || probe.argv.last() != Some(&invocation.output_path)
            || !output_paths.insert(invocation.output_path.clone())
        {
            return Err("allocator invocation does not bind its unique fresh probe".to_owned());
        }
        validate_repository_relative_json_path(&invocation.output_path)?;
        validate_allocator_probe(probe, role, &evidence.source.measurement_commit)?;
    }
    let recomputed = summarize(&evidence.samples)?;
    if recomputed != evidence.summaries {
        return Err("raw evidence summaries do not match independent recomputation".to_owned());
    }
    Ok(())
}

fn validate_bindings_at_commit(
    repository_root: &Path,
    evidence: &RawEvidence,
) -> Result<(), String> {
    const PATHS: [&str; 6] = [
        "schemas/road-editing/v1/road-editing.fbs",
        "docs/reference/road-editing-source-semantic-seed-v1.json",
        "docs/reference/road-editing-source-workload-definition-v1.json",
        "docs/reference/road-editing-source-reference-machine-v1.json",
        "docs/reference/road-editing-source-calibration-evidence-v1.schema.json",
        "Cargo.lock",
    ];
    if evidence.bindings.len() != PATHS.len() {
        return Err("raw evidence binding count is not six".to_owned());
    }
    for (binding, expected_path) in evidence.bindings.iter().zip(PATHS) {
        if binding.path != expected_path {
            return Err("raw evidence binding order/path mismatch".to_owned());
        }
        let object = Command::new("git")
            .current_dir(repository_root)
            .args([
                "show",
                &format!("{}:{expected_path}", evidence.source.measurement_commit),
            ])
            .output()
            .map_err(|error| format!("cannot read measured Git object {expected_path}: {error}"))?;
        require_success("git show evidence binding", &object)?;
        if binding.byte_length != u64::try_from(object.stdout.len()).unwrap_or(u64::MAX)
            || binding.sha256 != sha256_hex(&object.stdout)
        {
            return Err(format!(
                "raw evidence binding does not match measurement commit: {expected_path}"
            ));
        }
    }
    Ok(())
}

fn validate_repository_relative_json_path(path: &str) -> Result<(), String> {
    let path = Path::new(path);
    if path.is_absolute()
        || path.extension().and_then(|value| value.to_str()) != Some("json")
        || path.components().any(|component| {
            !matches!(
                component,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
    {
        return Err(
            "raw evidence output path is not a closed repository-relative JSON path".to_owned(),
        );
    }
    Ok(())
}

fn workload_profiles() -> Vec<WorkloadProfile> {
    let mut profiles = P100_PROFILE_COMBINATIONS
        .into_iter()
        .map(|combination| WorkloadProfile {
            workload: EvidenceWorkload::Base,
            accuracy: combination.accuracy(),
            direction: combination.direction(),
        })
        .collect::<Vec<_>>();
    profiles.push(WorkloadProfile {
        workload: EvidenceWorkload::Regularity,
        accuracy: GeometryAccuracyProfile::Fine2Cm,
        direction: GeometryDirectionProfile::Smooth1Deg,
    });
    profiles
}

fn allocator_probe_roles() -> [AllocatorProbeRole; 4] {
    [
        AllocatorProbeRole::BaseCompleteCompile,
        AllocatorProbeRole::RegularityCompleteCompile,
        AllocatorProbeRole::RewriteCandidateBuildEncode,
        AllocatorProbeRole::RewriteCandidateCompleteCompile,
    ]
}

fn run_all_allocator_probes(
    repository_root: &Path,
    probe_root: &Path,
    measurement_commit: &str,
) -> Result<(Vec<AllocatorProbeInvocation>, Vec<AllocatorProbe>), String> {
    if probe_root.exists() {
        return Err(format!(
            "fresh allocator-probe directory already exists: {}",
            probe_root.display()
        ));
    }
    fs::create_dir_all(probe_root).map_err(|error| {
        format!(
            "cannot create allocator-probe directory {}: {error}",
            probe_root.display()
        )
    })?;
    let mut invocations = Vec::with_capacity(4);
    let mut probes = Vec::with_capacity(4);
    for role in allocator_probe_roles() {
        let output = probe_root.join(format!("{}.json", role.id()));
        let relative_output = repository_relative(repository_root, &output)?;
        let arguments = vec![
            "run".to_owned(),
            RUST_TOOLCHAIN.to_owned(),
            "cargo".to_owned(),
            "run".to_owned(),
            "--release".to_owned(),
            "--locked".to_owned(),
            "-p".to_owned(),
            PACKAGE.to_owned(),
            "--bin".to_owned(),
            "calibrate-alloc".to_owned(),
            "--".to_owned(),
            role.id().to_owned(),
            relative_output.clone(),
        ];
        let status = Command::new(RUSTUP)
            .current_dir(repository_root)
            .args(&arguments)
            .output()
            .map_err(|error| format!("cannot start allocator probe {}: {error}", role.id()))?;
        require_success("road-editing allocator probe", &status)?;
        let bytes = fs::read(&output)
            .map_err(|error| format!("cannot read probe {}: {error}", output.display()))?;
        let probe: AllocatorProbe = serde_json::from_slice(&bytes)
            .map_err(|error| format!("invalid probe {}: {error}", output.display()))?;
        validate_allocator_probe(&probe, role, measurement_commit)?;
        let mut argv = Vec::with_capacity(arguments.len() + 1);
        argv.push(RUSTUP.to_owned());
        argv.extend(arguments);
        invocations.push(AllocatorProbeInvocation {
            role,
            argv,
            output_path: relative_output,
        });
        probes.push(probe);
    }
    Ok((invocations, probes))
}

#[allow(clippy::too_many_arguments)]
fn run_one_sample(
    repository_root: &Path,
    sample_root: &Path,
    profile: &WorkloadProfile,
    sample_kind: EvidenceSampleKind,
    sample_index: u8,
    invocations: &mut Vec<EvidenceInvocation>,
    samples: &mut Vec<EvidenceSample>,
) -> Result<(), String> {
    let workload_arg = match profile.workload {
        EvidenceWorkload::Base => "base",
        EvidenceWorkload::Regularity => "regularity",
    };
    let kind_arg = match sample_kind {
        EvidenceSampleKind::Warmup => "warmup",
        EvidenceSampleKind::Formal => "formal",
    };
    let file_name = format!(
        "{workload_arg}-a{}-d{}-{kind_arg}-{sample_index}.json",
        profile.accuracy as u8, profile.direction as u8
    );
    let output = sample_root.join(file_name);
    let relative_output = repository_relative(repository_root, &output)?;
    let arguments = vec![
        "run".to_owned(),
        RUST_TOOLCHAIN.to_owned(),
        "cargo".to_owned(),
        "run".to_owned(),
        "--release".to_owned(),
        "--locked".to_owned(),
        "-p".to_owned(),
        PACKAGE.to_owned(),
        "--bin".to_owned(),
        "calibrate".to_owned(),
        "--".to_owned(),
        "road-editing-evidence-sample".to_owned(),
        workload_arg.to_owned(),
        (profile.accuracy as u8).to_string(),
        (profile.direction as u8).to_string(),
        kind_arg.to_owned(),
        sample_index.to_string(),
        relative_output.clone(),
    ];
    let output_status = Command::new(RUSTUP)
        .current_dir(repository_root)
        .args(&arguments)
        .output()
        .map_err(|error| format!("cannot start fresh cargo sample: {error}"))?;
    require_success("fresh road-editing evidence sample", &output_status)?;
    let bytes = fs::read(&output)
        .map_err(|error| format!("cannot read sample {}: {error}", output.display()))?;
    let sample: EvidenceSample = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid sample {}: {error}", output.display()))?;
    validate_sample(&sample, profile, sample_kind, sample_index)?;
    let mut argv = Vec::with_capacity(arguments.len() + 1);
    argv.push(RUSTUP.to_owned());
    argv.extend(arguments);
    invocations.push(EvidenceInvocation {
        workload: profile.workload.id().to_owned(),
        accuracy_profile_code: profile.accuracy as u8,
        direction_profile_code: profile.direction as u8,
        sample_kind,
        sample_index,
        argv,
        output_path: relative_output,
    });
    samples.push(sample);
    Ok(())
}

fn validate_sample(
    sample: &EvidenceSample,
    profile: &WorkloadProfile,
    sample_kind: EvidenceSampleKind,
    sample_index: u8,
) -> Result<(), String> {
    if sample.schema != SAMPLE_SCHEMA
        || sample.schema_version != SAMPLE_SCHEMA_VERSION
        || sample.workload != profile.workload.id()
        || sample.accuracy_profile_code != profile.accuracy as u8
        || sample.direction_profile_code != profile.direction as u8
        || sample.sample_kind != sample_kind
        || sample.sample_index != sample_index
        || sample.fixtures.len() != 5
        || sample.metrics.verified_table_occurrence_count != 3_165
    {
        return Err("fresh sample identity or exact P100 counts do not match protocol".to_owned());
    }
    validate_geometry_observation(&sample.geometry_observation, profile.direction)?;
    if rewrite_is_required(profile) != sample.single_module_rewrite.is_some() {
        return Err(
            "single-module rewrite evidence must appear only for base Balanced5Cm/Balanced2Deg"
                .to_owned(),
        );
    }
    match profile.workload {
        EvidenceWorkload::Base
            if sample.metrics.total_horizontal_regularity_node_visits != 0
                || sample
                    .metrics
                    .maximum_horizontal_regularity_node_visits_per_offset_bearing_source_segment
                    != 0 =>
        {
            Err("base P100 must report zero cubic regularity visits".to_owned())
        }
        EvidenceWorkload::Regularity
            if sample.metrics.total_horizontal_regularity_node_visits != 3
                || sample
                    .metrics
                    .maximum_horizontal_regularity_node_visits_per_offset_bearing_source_segment
                    != 3 =>
        {
            Err("regularity companion must report exactly three visits".to_owned())
        }
        _ => Ok(()),
    }
}

fn validate_allocator_probe(
    probe: &AllocatorProbe,
    role: AllocatorProbeRole,
    measurement_commit: &str,
) -> Result<(), String> {
    let (workload, accuracy, direction, expected_visits, expects_compiler_metrics) = match role {
        AllocatorProbeRole::BaseCompleteCompile => (
            "LF-ROAD-EDITING-P100-v1",
            GeometryAccuracyProfile::Fine2Cm as u8,
            GeometryDirectionProfile::Smooth1Deg as u8,
            0,
            true,
        ),
        AllocatorProbeRole::RegularityCompleteCompile => (
            "LF-ROAD-EDITING-P100-REGULARITY-v1",
            GeometryAccuracyProfile::Fine2Cm as u8,
            GeometryDirectionProfile::Smooth1Deg as u8,
            3,
            true,
        ),
        AllocatorProbeRole::RewriteCandidateBuildEncode
        | AllocatorProbeRole::RewriteCandidateCompleteCompile => (
            "LF-ROAD-EDITING-P100-v1",
            GeometryAccuracyProfile::Balanced5Cm as u8,
            GeometryDirectionProfile::Balanced2Deg as u8,
            0,
            role == AllocatorProbeRole::RewriteCandidateCompleteCompile,
        ),
    };
    let expected_preloaded = probe
        .preloaded_revision
        .source_buffer_retained_capacity_bytes
        .checked_add(probe.preloaded_revision.output_logical_bytes)
        .and_then(|value| {
            value.checked_add(
                probe
                    .preloaded_revision
                    .candidate_source_buffer_retained_capacity_bytes,
            )
        })
        .ok_or_else(|| "allocator probe preloaded byte sum overflow".to_owned())?;
    if probe.schema != ALLOCATOR_PROBE_SCHEMA
        || probe.schema_version != ALLOCATOR_PROBE_SCHEMA_VERSION
        || probe.role != role
        || probe.workload != workload
        || probe.accuracy_profile_code != accuracy
        || probe.direction_profile_code != direction
        || probe.measurement_commit != measurement_commit
        || probe
            .preloaded_revision
            .source_buffer_retained_capacity_bytes
            == 0
        || probe.preloaded_revision.conservative_coexisting_bytes != expected_preloaded
        || probe.operation_heap.total_allocated_bytes < probe.operation_heap.peak_live_bytes
        || probe.operation_heap.peak_live_bytes < probe.operation_heap.end_live_bytes
        || probe.operation_heap.peak_live_allocation_count
            < probe.operation_heap.end_live_allocation_count
        || probe.operation_heap.peak_live_bytes == 0
    {
        return Err(format!(
            "allocator probe identity or heap relationships are invalid: {}",
            role.id()
        ));
    }
    match role {
        AllocatorProbeRole::BaseCompleteCompile | AllocatorProbeRole::RegularityCompleteCompile => {
            if probe.preloaded_revision.output_logical_bytes != 0
                || probe
                    .preloaded_revision
                    .candidate_source_buffer_retained_capacity_bytes
                    != 0
                || probe.candidate_source_buffer_retained_capacity_bytes != 0
            {
                return Err("complete-compile probe preloaded identity is invalid".to_owned());
            }
        }
        AllocatorProbeRole::RewriteCandidateBuildEncode => {
            if probe.preloaded_revision.output_logical_bytes == 0
                || probe
                    .preloaded_revision
                    .candidate_source_buffer_retained_capacity_bytes
                    != 0
                || probe.candidate_source_buffer_retained_capacity_bytes == 0
            {
                return Err("rewrite build/encode preloaded identity is invalid".to_owned());
            }
        }
        AllocatorProbeRole::RewriteCandidateCompleteCompile => {
            if probe.preloaded_revision.output_logical_bytes == 0
                || probe
                    .preloaded_revision
                    .candidate_source_buffer_retained_capacity_bytes
                    != probe.candidate_source_buffer_retained_capacity_bytes
                || probe.candidate_source_buffer_retained_capacity_bytes == 0
            {
                return Err("rewrite compile preloaded identity is invalid".to_owned());
            }
        }
    }
    match (&probe.production_metrics, expects_compiler_metrics) {
        (Some(metrics), true)
            if metrics.verified_table_occurrence_count == 3_165
                && metrics.total_horizontal_regularity_node_visits == expected_visits
                && metrics.compiler_controlled_peak_bytes
                    >= probe.operation_heap.peak_live_bytes
                && probe.compiler_ledger_covers_observed_operation_peak == Some(true) =>
        {
            Ok(())
        }
        (None, false)
            if probe
                .compiler_ledger_covers_observed_operation_peak
                .is_none() =>
        {
            Ok(())
        }
        _ => Err(format!(
            "allocator probe production-ledger proof is invalid: {}",
            role.id()
        )),
    }
}

fn validate_geometry_observation(
    observation: &GeometryObservation,
    direction: GeometryDirectionProfile,
) -> Result<(), String> {
    let expected_samples = observation
        .evaluator_interval_count
        .checked_mul(4_097)
        .ok_or_else(|| "geometry observation sample count overflow".to_owned())?;
    if observation.evaluator_interval_count == 0
        || observation.observed_sample_count != expected_samples
        || observation.evaluator_interval_identity_sha256.len() != 64
        || !observation
            .evaluator_interval_identity_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        || observation.worst_observed_error.module.is_empty()
        || observation.worst_observed_error.source_address.is_empty()
        || observation.worst_observed_error.evaluator_interval_ordinal
            >= observation.evaluator_interval_count
    {
        return Err("geometry observation identity/completeness is invalid".to_owned());
    }
    let p50 = parse_f64_bits(&observation.position_error.p50_meters_bits)?;
    let p95 = parse_f64_bits(&observation.position_error.p95_meters_bits)?;
    let p99 = parse_f64_bits(&observation.position_error.p99_meters_bits)?;
    let maximum = parse_f64_bits(&observation.position_error.maximum_meters_bits)?;
    let parameter = parse_f64_bits(&observation.worst_observed_error.parameter_bits)?;
    let direction_cosine_squared =
        parse_f64_bits(&observation.final_f32_direction_minimum_cosine_squared_bits)?;
    if !p50.is_finite()
        || !p95.is_finite()
        || !p99.is_finite()
        || !maximum.is_finite()
        || !parameter.is_finite()
        || !direction_cosine_squared.is_finite()
        || p50.is_sign_negative()
        || p95 < p50
        || p99 < p95
        || maximum < p99
        || !(0.0 < direction_cosine_squared && direction_cosine_squared <= 1.0)
        || direction_cosine_squared < full_angle_cosine_squared(direction)
    {
        return Err("geometry observation statistics are not finite ordered values".to_owned());
    }
    Ok(())
}

fn full_angle_cosine_squared(profile: GeometryDirectionProfile) -> f64 {
    f64::from_bits(match profile {
        GeometryDirectionProfile::Smooth1Deg => 0x3fef_fd81_3c5f_82b4,
        GeometryDirectionProfile::Balanced2Deg => 0x3fef_f605_b8b8_7ffc,
        GeometryDirectionProfile::Compact5Deg => 0x3fef_c1c5_c640_8e0c,
    })
}

fn parse_f64_bits(value: &str) -> Result<f64, String> {
    let digits = value
        .strip_prefix("0x")
        .filter(|digits| digits.len() == 16)
        .ok_or_else(|| "binary64 evidence value is not canonical hexadecimal".to_owned())?;
    let bits = u64::from_str_radix(digits, 16)
        .map_err(|_| "binary64 evidence value is not hexadecimal".to_owned())?;
    Ok(f64::from_bits(bits))
}

fn summarize(samples: &[EvidenceSample]) -> Result<Vec<EvidenceSummary>, String> {
    let mut summaries = Vec::with_capacity(10);
    for profile in workload_profiles() {
        let all = samples
            .iter()
            .filter(|sample| {
                sample.workload == profile.workload.id()
                    && sample.accuracy_profile_code == profile.accuracy as u8
                    && sample.direction_profile_code == profile.direction as u8
            })
            .collect::<Vec<_>>();
        let formal = all
            .iter()
            .copied()
            .filter(|sample| sample.sample_kind == EvidenceSampleKind::Formal)
            .collect::<Vec<_>>();
        if all.len() != usize::from(FORMAL_SAMPLE_COUNT + 1)
            || formal.len() != usize::from(FORMAL_SAMPLE_COUNT)
        {
            return Err(
                "workload/profile does not contain one warmup and seven formal samples".to_owned(),
            );
        }
        let identity = all[0];
        if all.iter().any(|sample| {
            sample.fixtures != identity.fixtures
                || sample.metrics != identity.metrics
                || sample.geometry_observation != identity.geometry_observation
                || !same_optional_rewrite_identity(
                    sample.single_module_rewrite.as_ref(),
                    identity.single_module_rewrite.as_ref(),
                )
        }) {
            return Err(
                "fixture identities, production metrics or geometry observation changed between samples"
                    .to_owned(),
            );
        }
        summaries.push(EvidenceSummary {
            workload: profile.workload.id().to_owned(),
            accuracy_profile_code: profile.accuracy as u8,
            direction_profile_code: profile.direction as u8,
            formal_sample_count: FORMAL_SAMPLE_COUNT,
            timings_ns: EvidenceTimingSummary {
                typed_model_build: median_mad(
                    formal
                        .iter()
                        .map(|sample| sample.timings_ns.typed_model_build),
                )?,
                encode: median_mad(formal.iter().map(|sample| sample.timings_ns.encode))?,
                size_prefix_and_identifier_preflight: median_mad(
                    formal
                        .iter()
                        .map(|sample| sample.timings_ns.size_prefix_and_identifier_preflight),
                )?,
                flatbuffers_verifier: median_mad(
                    formal
                        .iter()
                        .map(|sample| sample.timings_ns.flatbuffers_verifier),
                )?,
                semantic_preflight_and_typed_ast_lowering: median_mad(
                    formal
                        .iter()
                        .map(|sample| sample.timings_ns.semantic_preflight_and_typed_ast_lowering),
                )?,
                complete_compile: median_mad(
                    formal
                        .iter()
                        .map(|sample| sample.timings_ns.complete_compile),
                )?,
            },
            fixtures: identity.fixtures.clone(),
            metrics: identity.metrics.clone(),
            geometry_observation: identity.geometry_observation.clone(),
            single_module_rewrite: summarize_rewrite(identity, &formal)?,
        });
    }
    Ok(summaries)
}

fn rewrite_is_required(profile: &WorkloadProfile) -> bool {
    profile.workload == EvidenceWorkload::Base
        && profile.accuracy == GeometryAccuracyProfile::Balanced5Cm
        && profile.direction == GeometryDirectionProfile::Balanced2Deg
}

fn same_optional_rewrite_identity(
    left: Option<&SingleModuleRewriteEvidence>,
    right: Option<&SingleModuleRewriteEvidence>,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => same_rewrite_identity(left, right),
        (None, Some(_)) | (Some(_), None) => false,
    }
}

fn same_rewrite_identity(
    left: &SingleModuleRewriteEvidence,
    right: &SingleModuleRewriteEvidence,
) -> bool {
    left.edited_module == right.edited_module
        && left.target_kind == right.target_kind
        && left.target_local_key == right.target_local_key
        && left.initial_start_width_f64_bits == right.initial_start_width_f64_bits
        && left.initial_end_width_f64_bits == right.initial_end_width_f64_bits
        && left.candidate_start_width_f64_bits == right.candidate_start_width_f64_bits
        && left.candidate_end_width_f64_bits == right.candidate_end_width_f64_bits
        && left.old_fixture == right.old_fixture
        && left.candidate_fixture == right.candidate_fixture
        && left.unmodified_modules == right.unmodified_modules
        && left.unmodified_module_byte_identity == right.unmodified_module_byte_identity
        && left.old_source_buffer_retained_capacity_bytes
            == right.old_source_buffer_retained_capacity_bytes
        && left.candidate_source_buffer_retained_capacity_bytes
            == right.candidate_source_buffer_retained_capacity_bytes
        && left.post_commit_retained_capacity_bytes == right.post_commit_retained_capacity_bytes
        && left.old_metrics == right.old_metrics
        && left.candidate_metrics == right.candidate_metrics
}

fn summarize_rewrite(
    identity_sample: &EvidenceSample,
    formal: &[&EvidenceSample],
) -> Result<Option<SingleModuleRewriteSummary>, String> {
    let Some(identity) = identity_sample.single_module_rewrite.as_ref() else {
        return Ok(None);
    };
    let formal = formal
        .iter()
        .map(|sample| {
            sample.single_module_rewrite.as_ref().ok_or_else(|| {
                "formal rewrite sample is missing from the rewrite profile".to_owned()
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(SingleModuleRewriteSummary {
        edited_module: identity.edited_module.clone(),
        target_kind: identity.target_kind.clone(),
        target_local_key: identity.target_local_key.clone(),
        initial_start_width_f64_bits: identity.initial_start_width_f64_bits.clone(),
        initial_end_width_f64_bits: identity.initial_end_width_f64_bits.clone(),
        candidate_start_width_f64_bits: identity.candidate_start_width_f64_bits.clone(),
        candidate_end_width_f64_bits: identity.candidate_end_width_f64_bits.clone(),
        old_fixture: identity.old_fixture.clone(),
        candidate_fixture: identity.candidate_fixture.clone(),
        unmodified_modules: identity.unmodified_modules.clone(),
        unmodified_module_byte_identity: identity.unmodified_module_byte_identity,
        old_source_buffer_retained_capacity_bytes: identity
            .old_source_buffer_retained_capacity_bytes,
        candidate_source_buffer_retained_capacity_bytes: identity
            .candidate_source_buffer_retained_capacity_bytes,
        post_commit_retained_capacity_bytes: identity.post_commit_retained_capacity_bytes,
        timings_ns: SingleModuleRewriteTimingSummary {
            candidate_typed_model_build: median_mad(
                formal
                    .iter()
                    .map(|rewrite| rewrite.timings_ns.candidate_typed_model_build),
            )?,
            candidate_encode: median_mad(
                formal
                    .iter()
                    .map(|rewrite| rewrite.timings_ns.candidate_encode),
            )?,
            candidate_complete_compile: median_mad(
                formal
                    .iter()
                    .map(|rewrite| rewrite.timings_ns.candidate_complete_compile),
            )?,
        },
        old_metrics: identity.old_metrics.clone(),
        candidate_metrics: identity.candidate_metrics.clone(),
    }))
}

fn median_mad(values: impl Iterator<Item = u64>) -> Result<MedianMad, String> {
    let mut values = values.collect::<Vec<_>>();
    if values.len() != usize::from(FORMAL_SAMPLE_COUNT) {
        return Err("median/MAD requires exactly seven values".to_owned());
    }
    values.sort_unstable();
    let median = values[values.len() / 2];
    let mut deviations = values
        .into_iter()
        .map(|value| value.abs_diff(median))
        .collect::<Vec<_>>();
    deviations.sort_unstable();
    Ok(MedianMad {
        median,
        median_absolute_deviation: deviations[deviations.len() / 2],
    })
}

fn source_identity(repository_root: &Path) -> Result<EvidenceSource, String> {
    let commit = command_text(
        Command::new("git")
            .current_dir(repository_root)
            .args(["rev-parse", "HEAD"]),
        "git rev-parse HEAD",
    )?;
    if commit.len() != 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("measurement commit is not a full Git object id".to_owned());
    }
    let status = command_text(
        Command::new("git")
            .current_dir(repository_root)
            .args(["status", "--porcelain=v1"]),
        "git status",
    )?;
    if !status.is_empty() {
        return Err("formal evidence requires a clean worktree".to_owned());
    }
    Ok(EvidenceSource {
        measurement_commit: commit.clone(),
        generator_contract_revision: commit.clone(),
        writer_implementation_revision: commit.clone(),
        compiler_implementation_revision: commit,
        worktree_clean: true,
    })
}

fn bindings(repository_root: &Path) -> Result<Vec<EvidenceFileBinding>, String> {
    [
        "schemas/road-editing/v1/road-editing.fbs",
        "docs/reference/road-editing-source-semantic-seed-v1.json",
        "docs/reference/road-editing-source-workload-definition-v1.json",
        "docs/reference/road-editing-source-reference-machine-v1.json",
        "docs/reference/road-editing-source-calibration-evidence-v1.schema.json",
        "Cargo.lock",
    ]
    .into_iter()
    .map(|relative| file_binding(repository_root, relative))
    .collect()
}

fn file_binding(repository_root: &Path, relative: &str) -> Result<EvidenceFileBinding, String> {
    let bytes = fs::read(repository_root.join(relative))
        .map_err(|error| format!("cannot read evidence binding {relative}: {error}"))?;
    let sha256 = sha256_hex(&bytes);
    let expected = match relative {
        "schemas/road-editing/v1/road-editing.fbs" => Some(EXPECTED_SCHEMA_SHA256),
        "docs/reference/road-editing-source-semantic-seed-v1.json" => Some(EXPECTED_SEED_SHA256),
        "docs/reference/road-editing-source-reference-machine-v1.json" => {
            Some(EXPECTED_REFERENCE_MACHINE_SHA256)
        }
        "docs/reference/road-editing-source-workload-definition-v1.json" => {
            Some(EXPECTED_WORKLOAD_SHA256)
        }
        "docs/reference/road-editing-source-calibration-evidence-v1.schema.json" => {
            Some(EXPECTED_COMPACT_EVIDENCE_SCHEMA_SHA256)
        }
        _ => None,
    };
    if expected.is_some_and(|expected| sha256 != expected) {
        return Err(format!("evidence binding digest mismatch: {relative}"));
    }
    Ok(EvidenceFileBinding {
        path: relative.to_owned(),
        byte_length: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        sha256,
    })
}

fn capture_environment(repository_root: &Path) -> Result<EvidenceEnvironment, String> {
    if !cfg!(windows) {
        return Err("formal LF-P100-REF-01 evidence must run on Windows".to_owned());
    }
    let output = Command::new("pwsh")
        .current_dir(repository_root)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            POWERSHELL_ENVIRONMENT,
        ])
        .output()
        .map_err(|error| format!("cannot capture Windows reference environment: {error}"))?;
    require_success("reference environment capture", &output)?;
    let captured: CapturedWindowsEnvironment = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("invalid reference environment JSON: {error}"))?;
    let hardware_identity_sha256 = captured.hardware_identity_sha256();
    if hardware_identity_sha256 != EXPECTED_HARDWARE_IDENTITY {
        return Err(format!(
            "reference machine identity mismatch: expected={EXPECTED_HARDWARE_IDENTITY} actual={hardware_identity_sha256}"
        ));
    }
    if captured.power_source != "AC"
        || !captured
            .power_plan_description
            .to_ascii_lowercase()
            .contains(BALANCED_POWER_PLAN_GUID)
    {
        return Err("formal evidence requires AC power and the Balanced power plan".to_owned());
    }
    let rustc_verbose_version = command_text(
        Command::new("rustc")
            .current_dir(repository_root)
            .args(["+1.96.0", "-Vv"]),
        "rustc +1.96.0 -Vv",
    )?;
    if !rustc_verbose_version.contains("release: 1.96.0")
        || !rustc_verbose_version.contains("host: x86_64-pc-windows-msvc")
        || !rustc_verbose_version.contains("LLVM version: 22.1.2")
    {
        return Err("rustc/target/LLVM does not match the frozen measurement protocol".to_owned());
    }
    let cargo_version = command_text(
        Command::new("cargo")
            .current_dir(repository_root)
            .args(["+1.96.0", "--version"]),
        "cargo +1.96.0 --version",
    )?;
    Ok(EvidenceEnvironment {
        hardware_id: HARDWARE_ID.to_owned(),
        hardware_identity_sha256,
        cpu: captured.cpu,
        physical_core_count: captured.physical_core_count,
        logical_processor_count: captured.logical_processor_count,
        physical_memory_bytes: captured.physical_memory_bytes,
        operating_system: captured.operating_system,
        operating_system_build: captured.operating_system_build,
        bios_firmware: captured.bios_firmware,
        power_source: captured.power_source,
        power_plan_guid: BALANCED_POWER_PLAN_GUID.to_owned(),
        power_plan_description: captured.power_plan_description,
        rustc_verbose_version,
        cargo_version,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CapturedWindowsEnvironment {
    cpu: String,
    physical_core_count: u32,
    logical_processor_count: u32,
    physical_memory_bytes: u64,
    operating_system: String,
    operating_system_build: String,
    bios_firmware: String,
    power_source: String,
    power_plan_description: String,
    system_uuid: String,
    bios_serial: String,
    baseboard_serial: String,
}

impl CapturedWindowsEnvironment {
    fn hardware_identity_sha256(&self) -> String {
        let identity = [&self.system_uuid, &self.bios_serial, &self.baseboard_serial]
            .map(|value| {
                value
                    .chars()
                    .filter(|character| !character.is_whitespace())
                    .flat_map(char::to_uppercase)
                    .collect::<String>()
            })
            .join("\n");
        sha256_hex(identity.as_bytes())
    }
}

fn command_text(command: &mut Command, context: &str) -> Result<String, String> {
    let output = command
        .output()
        .map_err(|error| format!("cannot run {context}: {error}"))?;
    require_success(context, &output)?;
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|error| format!("{context} output is not UTF-8: {error}"))
}

fn require_success(context: &str, output: &Output) -> Result<(), String> {
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{context} failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn repository_relative(repository_root: &Path, path: &Path) -> Result<String, String> {
    let relative = path.strip_prefix(repository_root).map_err(|_| {
        format!(
            "evidence output {} is outside repository {}",
            path.display(),
            repository_root.display()
        )
    })?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        write!(output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

const POWERSHELL_ENVIRONMENT: &str = r#"
$ErrorActionPreference = 'Stop'
$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$computer = Get-CimInstance Win32_ComputerSystem
$product = Get-CimInstance Win32_ComputerSystemProduct
$operatingSystem = Get-CimInstance Win32_OperatingSystem
$bios = Get-CimInstance Win32_BIOS
$baseboard = Get-CimInstance Win32_BaseBoard
$battery = Get-CimInstance Win32_Battery -ErrorAction SilentlyContinue | Select-Object -First 1
$acStatuses = @(2, 6, 7, 8, 9, 11)
$powerSource = if ($null -eq $battery -or $acStatuses -contains [int]$battery.BatteryStatus) { 'AC' } else { 'Battery' }
$powerPlan = (powercfg /getactivescheme | Out-String).Trim()
[ordered]@{
  cpu = [string]$cpu.Name.Trim()
  physicalCoreCount = [uint32]$cpu.NumberOfCores
  logicalProcessorCount = [uint32]$cpu.NumberOfLogicalProcessors
  physicalMemoryBytes = [uint64]$computer.TotalPhysicalMemory
  operatingSystem = [string]$operatingSystem.Caption.Trim()
  operatingSystemBuild = [string]$operatingSystem.BuildNumber
  biosFirmware = (([string]$bios.SMBIOSBIOSVersion).Trim() + ' (' + $bios.ReleaseDate.ToString('yyyy-MM-dd') + ')')
  powerSource = $powerSource
  powerPlanDescription = $powerPlan
  systemUuid = [string]$product.UUID
  biosSerial = [string]$bios.SerialNumber
  baseboardSerial = [string]$baseboard.SerialNumber
} | ConvertTo-Json -Compress
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workload_matrix_is_nine_base_combinations_plus_one_probe() {
        let profiles = workload_profiles();
        assert_eq!(profiles.len(), 10);
        assert_eq!(
            profiles
                .iter()
                .filter(|profile| profile.workload == EvidenceWorkload::Base)
                .count(),
            9
        );
        assert_eq!(profiles[9].workload, EvidenceWorkload::Regularity);
    }

    #[test]
    fn median_and_mad_use_all_seven_samples_without_deletion() {
        assert_eq!(
            median_mad([10, 11, 12, 13, 14, 100, 101].into_iter()),
            Ok(MedianMad {
                median: 13,
                median_absolute_deviation: 2,
            })
        );
        assert!(median_mad([1, 2, 3].into_iter()).is_err());
    }

    #[test]
    fn hardware_identity_normalization_matches_the_frozen_scheme() {
        let captured = CapturedWindowsEnvironment {
            cpu: String::new(),
            physical_core_count: 0,
            logical_processor_count: 0,
            physical_memory_bytes: 0,
            operating_system: String::new(),
            operating_system_build: String::new(),
            bios_firmware: String::new(),
            power_source: String::new(),
            power_plan_description: String::new(),
            system_uuid: " ab c ".to_owned(),
            bios_serial: "de\tf".to_owned(),
            baseboard_serial: "g h\n".to_owned(),
        };
        assert_eq!(
            captured.hardware_identity_sha256(),
            sha256_hex(b"ABC\nDEF\nGH")
        );
    }

    #[test]
    fn direction_observation_must_meet_the_selected_profile() {
        let smooth_threshold = full_angle_cosine_squared(GeometryDirectionProfile::Smooth1Deg);
        let mut observation = GeometryObservation {
            evaluator_interval_count: 1,
            observed_sample_count: 4_097,
            evaluator_interval_identity_sha256: "0".repeat(64),
            position_error: crate::PositionErrorStatistics {
                p50_meters_bits: "0x0000000000000000".to_owned(),
                p95_meters_bits: "0x0000000000000000".to_owned(),
                p99_meters_bits: "0x0000000000000000".to_owned(),
                maximum_meters_bits: "0x0000000000000000".to_owned(),
            },
            worst_observed_error: crate::WorstObservedError {
                module: "p100.m00".to_owned(),
                source_address: "LaneEdge/lane-0".to_owned(),
                source_segment_ordinal: 0,
                station_row_ordinal: None,
                evaluator_interval_ordinal: 0,
                parameter_bits: "0x0000000000000000".to_owned(),
            },
            final_f32_direction_minimum_cosine_squared_bits: format!(
                "0x{:016x}",
                smooth_threshold.to_bits()
            ),
        };

        assert!(
            validate_geometry_observation(&observation, GeometryDirectionProfile::Smooth1Deg,)
                .is_ok()
        );
        observation.final_f32_direction_minimum_cosine_squared_bits =
            format!("0x{:016x}", smooth_threshold.to_bits().saturating_sub(1));
        assert!(
            validate_geometry_observation(&observation, GeometryDirectionProfile::Smooth1Deg,)
                .is_err()
        );
    }

    #[cfg(windows)]
    #[test]
    fn current_reference_machine_environment_is_reproducible() {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap();
        let environment = capture_environment(repository_root).unwrap();

        assert_eq!(environment.hardware_id, HARDWARE_ID);
        assert_eq!(
            environment.hardware_identity_sha256,
            EXPECTED_HARDWARE_IDENTITY
        );
        assert_eq!(environment.power_source, "AC");
        assert_eq!(environment.power_plan_guid, BALANCED_POWER_PLAN_GUID);
    }
}
