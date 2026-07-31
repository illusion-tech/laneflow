//! #308 编译器预算校准的非生产研究内核。
//!
//! 本 crate 不属于 LaneFlow 生产依赖图，也不定义公共编译器契约。

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Component, Path, PathBuf};

mod bounded_oracle;
mod bounded_template;
mod controlled_alloc;
mod corridor;
#[cfg(feature = "fixture-oracle")]
mod corridor_fixture_oracle;
mod corridor_oracle;
mod current_fixtures;
mod generator;
mod guard;
mod identity;
mod junction_grid;
mod junction_grid_oracle;
mod ladder;
mod ladder_runner;
mod manifest;
mod oracle;
mod pilot;
mod pilot_budget;
mod pipeline;
mod process_containment;
mod process_protocol;
mod protocol;
mod roles;
mod scale_plan;
mod stage;
mod stage_oracle;
mod timing;
mod workload;

pub use corridor::{
    CORRIDOR_KNOWN_VECTOR_SCHEMA, CORRIDOR_WORKLOAD_ID, CorridorContract, CorridorError,
    CorridorKnownVectorDocument, CorridorStageSummary, build_corridor_known_vectors,
    build_corridor_stage_summary,
};
pub use corridor_oracle::{
    CorridorOracleError, CorridorOracleVerificationReport, verify_corridor_oracle_matrix,
};
pub use current_fixtures::{
    CURRENT_FIXTURES_KNOWN_VECTOR_SCHEMA, CURRENT_FIXTURES_WORKLOAD_ID, CurrentFixtureCaseSummary,
    CurrentFixturesContract, CurrentFixturesError, CurrentFixturesKnownVectorDocument,
    CurrentFixturesOracleError, CurrentFixturesOracleVerificationReport,
    build_current_fixture_summaries, build_current_fixtures_known_vectors,
    verify_current_fixtures_oracle,
};
pub use generator::{
    ExpandedModule, ExpandedModuleGraph, GeneratorError, GraphProfileId,
    ModuleGraphKnownVectorDocument, SequenceKind, build_module_graph_known_vectors,
    expand_module_graph, permute_in_place,
};
pub use guard::{
    COMPILER_CONTROLLED_HARD_CEILING_BYTES, ChildProcessMemoryMonitor,
    ChildProcessMemoryObservation, GuardCompletedLevelObservation, GuardError,
    GuardPredictionBasis, GuardPreflightReport, GuardThresholds, GuardTrigger,
    PRIVATE_MEMORY_HARD_CEILING_BYTES, ScalableGuardPlanner, SystemMemoryMonitor,
    SystemMemoryObservation, WALL_TIME_HARD_CEILING_NS, evaluate_identity_guard_preflight,
};
pub use identity::{
    IdentityContract, IdentityContractError, IdentityDeclarationVector, IdentityFieldVector,
    IdentityGenerationError, IdentityKnownVector, IdentityKnownVectorDocument,
    SemanticRecordVector, build_identity_known_vectors,
};
pub use junction_grid::{
    JUNCTION_GRID_KNOWN_VECTOR_SCHEMA, JUNCTION_GRID_WORKLOAD_ID, JunctionGridContract,
    JunctionGridError, JunctionGridKnownVectorDocument, build_junction_grid_known_vectors,
    build_junction_grid_stage_summary,
};
pub use junction_grid_oracle::{
    JunctionGridOracleError, JunctionGridOracleVerificationReport,
    verify_junction_grid_oracle_matrix,
};
pub use ladder::{
    ExactPositiveRatio, FORMAL_LADDER_AGGREGATION_METHOD, FORMAL_LADDER_BATCH_COUNT,
    FORMAL_LADDER_MINIMUM_LEVEL_COUNT, FORMAL_LADDER_ROUND_COUNT,
    FORMAL_LADDER_SCALE_SELECTION_RULE, FormalAdjacentLevelRatio, FormalKneeAssessment,
    FormalLadderAnalysis, FormalLadderBatchSummary, FormalLadderCompletedLevel, FormalLadderError,
    FormalLadderMetric, FormalLadderRoundMetricSummary, FormalLadderRoundRun,
    FormalLadderSampleKind, FormalScaleSelection, FormalScaleSelectionDisposition,
    analyze_formal_ladder,
};
pub use ladder_runner::{
    FORMAL_LADDER_EXECUTION_SCHEMA, FORMAL_LADDER_EXECUTION_SCHEMA_VERSION,
    FormalAttributionPreflightRun, FormalLadderExecution, FormalLadderExecutionDisposition,
    FormalLadderLevelExecution, FormalLadderProcessRun, FormalLadderRunnerError, FormalOracleRun,
    run_formal_ladders,
};
pub use manifest::{GeneratorContract, ManifestContractError};
pub use oracle::{
    ExactOracleError, OracleVerificationError, OracleVerificationReport,
    verify_identity_oracle_matrix,
};
pub use pilot::{
    BASE_SCALE_AGGREGATION_METHOD, BASE_SCALE_PILOT_CHECKPOINT_SCHEMA,
    BASE_SCALE_PILOT_CHECKPOINT_SCHEMA_VERSION, BASE_SCALE_SELECTION_RULE,
    BaseScaleOracleInvalidationReason, BaseScaleOracleRun, BaseScalePilotCheckpoint,
    BaseScalePilotLevel, BaseScalePilotRun, BaseScalePilotRunKind, BaseScaleSelection,
    CLOCK_QUANTUM_MULTIPLIER, ChildMonitorTrigger, ChildProcessMonitorReport, FORMAL_PROTOCOL_ID,
    FRESH_PROCESS_PILOT_SAMPLE_COUNT, IdentityFreshProcessPilot, IdentityFreshProcessPilotOutcome,
    IdentityFreshProcessPilotStop, IdentityMonitoredChildSample, MAXIMUM_RELATIVE_MAD_PERCENT,
    PilotError, run_base_scale_pilot_discovery,
    run_base_scale_pilot_discovery_with_checkpoint_sink, run_identity_fresh_process_pilot,
    wait_for_parent_start_signal,
};
pub use pilot_budget::{
    PILOT_BUDGET_REPORT_SCHEMA, PilotBudgetError, PilotBudgetOutcome, PilotBudgetRequest,
    recompute_pilot_budget,
};
pub use pipeline::IdentityAllocationSnapshot;
pub use process_protocol::{
    InvalidationReason, NullableObservation, ProcessExitKind, ProcessObservation,
    ProcessProtocolError, RunStatus, TerminationKind, TerminationObservation,
};
pub use protocol::{
    FORMAL_PROTOCOL_CHECKPOINT_SCHEMA, FORMAL_PROTOCOL_CHECKPOINT_SCHEMA_VERSION,
    FormalProtocolCheckpoint, FormalProtocolError, FormalProtocolOutcome, FormalProtocolRequest,
    parse_formal_protocol_arguments, run_formal_protocol,
};
pub use roles::{
    ATTRIBUTION_BINARY_ID, ControlledAllocationGuardReport, IDENTITY_ATTRIBUTION_CHILD_SCHEMA,
    IDENTITY_ATTRIBUTION_CHILD_SCHEMA_VERSION, IDENTITY_ORACLE_CHILD_SCHEMA,
    IDENTITY_ORACLE_CHILD_SCHEMA_VERSION, IDENTITY_TIMING_CHILD_SCHEMA,
    IDENTITY_TIMING_CHILD_SCHEMA_VERSION, IdentityAttributionChildReport,
    IdentityAttributionOutcome, IdentityOracleChildReport, IdentityTimingChildReport,
    ORACLE_BINARY_ID, RUNNER_BINARY_ID, ResearchBinaryDescriptor, ResearchBinaryRole,
    RoleExecutionError, SCALABLE_ATTRIBUTION_CHILD_SCHEMA,
    SCALABLE_ATTRIBUTION_CHILD_SCHEMA_VERSION, SCALABLE_LADDER_CHILD_SCHEMA,
    SCALABLE_LADDER_CHILD_SCHEMA_VERSION, SCALABLE_ORACLE_CHILD_SCHEMA,
    SCALABLE_ORACLE_CHILD_SCHEMA_VERSION, SCALABLE_TIMING_CHILD_SCHEMA,
    SCALABLE_TIMING_CHILD_SCHEMA_VERSION, ScalableAttributionChildReport,
    ScalableAttributionOutcome, ScalableLadderBinaryMode, ScalableLadderChildReport,
    ScalableLadderOutcome, ScalableLadderSample, ScalableOracleChildReport, ScalableOracleOutcome,
    ScalableTimingChildReport, ScalableTimingOutcome, TIMING_BINARY_ID,
    attribution_binary_descriptor, build_identity_oracle_child, measure_identity_attribution_child,
    measure_identity_timing_child, measure_scalable_attribution_child,
    measure_scalable_attribution_ladder_child, measure_scalable_timing_child,
    measure_scalable_timing_ladder_child, oracle_binary_descriptor, runner_binary_descriptor,
    timing_binary_descriptor, verify_scalable_oracle_child,
};
pub use scale_plan::{ScalableStagePlanFactory, ScalableStagePlanSummary, ScalePlanError};
pub use stage::{
    IdentityAggregateCounts, IdentityStagePlanSummary, IdentityStageSummary, StageBreakdown,
    StageContract, StageContractError, StageGenerationError, StageRetainedCapacityBytes,
    StageShape, build_identity_stage_plan_summary, build_identity_stage_summary,
};
pub use stage_oracle::StageOracleError;
pub use timing::{
    CLOCK_QUANTUM_OBSERVATION_COUNT, IdentityAttributionCompilerInstance, IdentityCompilerInstance,
    IdentityStableCapacitySequence, IdentityTimingCompilerInstance, IdentityTimingSample,
    STABLE_CAPACITY_SAMPLE_COUNT, STABLE_CAPACITY_WARMUP_COUNT,
    ScalableAttributionCompilerInstance, ScalableCompilerInstance, ScalableExecutedMeasurement,
    ScalablePreparedMeasurement, ScalableStableCapacitySequence, ScalableTimingCompilerInstance,
    ScalableTimingSample, TimingError, measure_identity_stage_once, observe_clock_quantum_ns,
};
pub use workload::{
    BASE_SCALE_STRING_PROFILE, BASELINE_CANDIDATE_ID, GENERATOR_VERSION_V1,
    ScalableWorkloadContractError, ScalableWorkloadId, ScalableWorkloadParseError,
    WORKLOAD_REVISION_V1, validate_base_scale_contract,
};

pub const CONTRACT_DESCRIPTOR_PATH: &str = "docs/reference/compiler-calibration-contract-v1.json";
pub const CONTRACT_DESCRIPTOR_BYTE_LENGTH: u64 = 1_322;
pub const CONTRACT_DESCRIPTOR_SHA256: &str =
    "5df3d5029f16de863af1a2fce04b736cf42595ffae1bcf8ce2b0a6edd90991d0";

const CONTRACT_SCHEMA: &str = "laneflow.compiler-calibration-contract";
const WORKLOAD_MANIFEST_SCHEMA: &str = "laneflow.compiler-calibration-workload-manifest";
const JSON_SCHEMA_DRAFT_2020_12: &str = "https://json-schema.org/draft/2020-12/schema";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContractDescriptor {
    pub schema: String,
    pub schema_version: u32,
    pub evidence_schema: ArtifactBinding,
    pub workload_manifest: ArtifactBinding,
    pub candidate_registry_revision: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactBinding {
    pub schema: Option<String>,
    pub id: Option<String>,
    pub schema_version: u32,
    pub path: String,
    pub byte_length: u64,
    pub sha256: String,
}

#[derive(Clone, Debug)]
pub struct TrustedContract {
    pub descriptor: ContractDescriptor,
    pub descriptor_sha256: String,
    pub workload_manifest: serde_json::Value,
    pub evidence_schema: serde_json::Value,
}

impl TrustedContract {
    pub fn generator_contract(&self) -> Result<GeneratorContract, ManifestContractError> {
        GeneratorContract::from_manifest(&self.workload_manifest)
    }

    pub fn identity_contract(&self) -> Result<IdentityContract, IdentityContractError> {
        IdentityContract::from_manifest(&self.workload_manifest)
    }

    pub fn stage_contract(&self) -> Result<StageContract, StageContractError> {
        StageContract::from_manifest(&self.workload_manifest)
    }
}

pub fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("research crate must remain two levels below the repository root")
        .to_path_buf()
}

pub fn load_repository_contract() -> Result<TrustedContract, ContractError> {
    load_contract(&repository_root())
}

pub fn load_contract(repository_root: &Path) -> Result<TrustedContract, ContractError> {
    let descriptor_path = resolve_repository_path(repository_root, CONTRACT_DESCRIPTOR_PATH)?;
    let descriptor_bytes = read_exact_artifact(
        &descriptor_path,
        CONTRACT_DESCRIPTOR_BYTE_LENGTH,
        CONTRACT_DESCRIPTOR_SHA256,
    )?;
    let descriptor_sha256 = sha256_hex(&descriptor_bytes);
    let descriptor: ContractDescriptor =
        serde_json::from_slice(&descriptor_bytes).map_err(|source| ContractError::InvalidJson {
            path: descriptor_path.clone(),
            source,
        })?;

    if descriptor.schema != CONTRACT_SCHEMA || descriptor.schema_version != 1 {
        return Err(ContractError::UnexpectedDescriptorIdentity {
            schema: descriptor.schema,
            schema_version: descriptor.schema_version,
        });
    }
    if descriptor.candidate_registry_revision != 1 {
        return Err(ContractError::UnexpectedCandidateRegistryRevision {
            actual: descriptor.candidate_registry_revision,
        });
    }

    let workload_manifest = load_bound_json(repository_root, &descriptor.workload_manifest)?;
    require_json_string(
        &workload_manifest,
        "schema",
        WORKLOAD_MANIFEST_SCHEMA,
        &descriptor.workload_manifest.path,
    )?;
    require_json_u64(
        &workload_manifest,
        "schemaVersion",
        u64::from(descriptor.workload_manifest.schema_version),
        &descriptor.workload_manifest.path,
    )?;
    require_json_u64(
        &workload_manifest,
        "candidateRegistry/revision",
        u64::from(descriptor.candidate_registry_revision),
        &descriptor.workload_manifest.path,
    )?;
    if descriptor.workload_manifest.schema.as_deref() != Some(WORKLOAD_MANIFEST_SCHEMA) {
        return Err(ContractError::UnexpectedBindingIdentity {
            path: descriptor.workload_manifest.path.clone(),
            field: "schema",
        });
    }

    let evidence_schema = load_bound_json(repository_root, &descriptor.evidence_schema)?;
    if descriptor.evidence_schema.schema_version != 1 {
        return Err(ContractError::UnexpectedBindingIdentity {
            path: descriptor.evidence_schema.path.clone(),
            field: "schemaVersion",
        });
    }
    require_json_string(
        &evidence_schema,
        "$schema",
        JSON_SCHEMA_DRAFT_2020_12,
        &descriptor.evidence_schema.path,
    )?;
    let expected_evidence_id = descriptor.evidence_schema.id.as_deref().ok_or_else(|| {
        ContractError::UnexpectedBindingIdentity {
            path: descriptor.evidence_schema.path.clone(),
            field: "id",
        }
    })?;
    require_json_string(
        &evidence_schema,
        "$id",
        expected_evidence_id,
        &descriptor.evidence_schema.path,
    )?;
    require_json_u64(
        &evidence_schema,
        "properties/schemaVersion/const",
        u64::from(descriptor.evidence_schema.schema_version),
        &descriptor.evidence_schema.path,
    )?;

    Ok(TrustedContract {
        descriptor,
        descriptor_sha256,
        workload_manifest,
        evidence_schema,
    })
}

fn load_bound_json(
    repository_root: &Path,
    binding: &ArtifactBinding,
) -> Result<serde_json::Value, ContractError> {
    let path = resolve_repository_path(repository_root, &binding.path)?;
    let bytes = read_exact_artifact(&path, binding.byte_length, &binding.sha256)?;
    serde_json::from_slice(&bytes).map_err(|source| ContractError::InvalidJson { path, source })
}

fn resolve_repository_path(
    repository_root: &Path,
    relative_path: &str,
) -> Result<PathBuf, ContractError> {
    let candidate = Path::new(relative_path);
    if candidate.is_absolute()
        || candidate
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ContractError::UnsafeArtifactPath {
            path: relative_path.to_owned(),
        });
    }

    Ok(repository_root.join(candidate))
}

fn read_exact_artifact(
    path: &Path,
    expected_length: u64,
    expected_sha256: &str,
) -> Result<Vec<u8>, ContractError> {
    let bytes = fs::read(path).map_err(|source| ContractError::ReadArtifact {
        path: path.to_path_buf(),
        source,
    })?;
    let actual_length = u64::try_from(bytes.len()).expect("artifact byte length must fit into u64");
    if actual_length != expected_length {
        return Err(ContractError::ByteLengthMismatch {
            path: path.to_path_buf(),
            expected: expected_length,
            actual: actual_length,
        });
    }

    let actual_sha256 = sha256_hex(&bytes);
    if actual_sha256 != expected_sha256 {
        return Err(ContractError::Sha256Mismatch {
            path: path.to_path_buf(),
            expected: expected_sha256.to_owned(),
            actual: actual_sha256,
        });
    }

    Ok(bytes)
}

fn require_json_string(
    value: &serde_json::Value,
    pointer: &str,
    expected: &str,
    artifact_path: &str,
) -> Result<(), ContractError> {
    let actual = lookup_json(value, pointer).and_then(serde_json::Value::as_str);
    if actual != Some(expected) {
        return Err(ContractError::UnexpectedArtifactField {
            path: artifact_path.to_owned(),
            field: pointer.to_owned(),
            expected: expected.to_owned(),
        });
    }
    Ok(())
}

fn require_json_u64(
    value: &serde_json::Value,
    pointer: &str,
    expected: u64,
    artifact_path: &str,
) -> Result<(), ContractError> {
    let actual = lookup_json(value, pointer).and_then(serde_json::Value::as_u64);
    if actual != Some(expected) {
        return Err(ContractError::UnexpectedArtifactField {
            path: artifact_path.to_owned(),
            field: pointer.to_owned(),
            expected: expected.to_string(),
        });
    }
    Ok(())
}

fn lookup_json<'a>(
    mut value: &'a serde_json::Value,
    slash_separated_path: &str,
) -> Option<&'a serde_json::Value> {
    for segment in slash_separated_path.split('/') {
        value = value.get(segment)?;
    }
    Some(value)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

#[derive(Debug, thiserror::Error)]
pub enum ContractError {
    #[error("无法读取研究契约制品 {path}: {source}")]
    ReadArtifact {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("研究契约制品 {path} 的精确字节长度不匹配：期望 {expected}，实际 {actual}")]
    ByteLengthMismatch {
        path: PathBuf,
        expected: u64,
        actual: u64,
    },
    #[error("研究契约制品 {path} 的 SHA-256 不匹配：期望 {expected}，实际 {actual}")]
    Sha256Mismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error("研究契约制品 {path} 不是有效 JSON: {source}")]
    InvalidJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("研究契约描述符身份不匹配：schema={schema}, schemaVersion={schema_version}")]
    UnexpectedDescriptorIdentity { schema: String, schema_version: u32 },
    #[error("研究契约描述符的候选注册表修订不是 1：实际 {actual}")]
    UnexpectedCandidateRegistryRevision { actual: u32 },
    #[error("研究契约绑定 {path} 缺少或误报字段 {field}")]
    UnexpectedBindingIdentity { path: String, field: &'static str },
    #[error("研究契约制品 {path} 的字段 {field} 不等于 {expected}")]
    UnexpectedArtifactField {
        path: String,
        field: String,
        expected: String,
    },
    #[error("研究契约描述符包含不安全的仓库相对路径：{path}")]
    UnsafeArtifactPath { path: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn loads_frozen_repository_contract_after_exact_byte_verification() {
        let contract = load_repository_contract().expect("frozen G1 contract must verify");

        assert_eq!(contract.descriptor.schema, CONTRACT_SCHEMA);
        assert_eq!(contract.descriptor.schema_version, 1);
        assert_eq!(contract.descriptor_sha256, CONTRACT_DESCRIPTOR_SHA256);
        assert_eq!(
            contract
                .workload_manifest
                .get("generatorVersion")
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
        assert_eq!(
            contract
                .evidence_schema
                .get("$schema")
                .and_then(serde_json::Value::as_str),
            Some(JSON_SCHEMA_DRAFT_2020_12)
        );
    }

    #[test]
    fn rejects_changed_descriptor_before_parsing_it() {
        let temporary_root = copy_contract_fixture("descriptor-changed");
        let descriptor_path = temporary_root.join(CONTRACT_DESCRIPTOR_PATH);
        let mut bytes = fs::read(&descriptor_path).expect("copied descriptor");
        bytes[0] = b'[';
        fs::write(&descriptor_path, bytes).expect("mutate copied descriptor");

        assert!(matches!(
            load_contract(&temporary_root),
            Err(ContractError::Sha256Mismatch { .. })
        ));

        fs::remove_dir_all(temporary_root).expect("remove test directory");
    }

    #[test]
    fn rejects_changed_bound_artifact_before_using_it() {
        let temporary_root = copy_contract_fixture("manifest-changed");
        let manifest_path =
            temporary_root.join("docs/reference/compiler-calibration-workloads-v1.json");
        let mut bytes = fs::read(&manifest_path).expect("copied workload manifest");
        let last = bytes.len() - 1;
        bytes[last] = if bytes[last] == b'\n' { b' ' } else { b'\n' };
        fs::write(&manifest_path, bytes).expect("mutate copied workload manifest");

        assert!(matches!(
            load_contract(&temporary_root),
            Err(ContractError::Sha256Mismatch { .. })
                | Err(ContractError::ByteLengthMismatch { .. })
        ));

        fs::remove_dir_all(temporary_root).expect("remove test directory");
    }

    #[test]
    fn rejects_paths_that_can_escape_the_repository_root() {
        assert!(matches!(
            resolve_repository_path(Path::new("repo"), "../outside.json"),
            Err(ContractError::UnsafeArtifactPath { .. })
        ));
        assert!(matches!(
            resolve_repository_path(Path::new("repo"), "./inside.json"),
            Err(ContractError::UnsafeArtifactPath { .. })
        ));
    }

    fn copy_contract_fixture(label: &str) -> PathBuf {
        let ordinal = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let temporary_root = std::env::temp_dir().join(format!(
            "laneflow-issue-308-{label}-{}-{ordinal}",
            std::process::id()
        ));
        let target_reference = temporary_root.join("docs/reference");
        fs::create_dir_all(&target_reference).expect("create test directory");

        for relative_path in [
            CONTRACT_DESCRIPTOR_PATH,
            "docs/reference/compiler-calibration-workloads-v1.json",
            "docs/reference/compiler-calibration-evidence-v1.schema.json",
        ] {
            let source = repository_root().join(relative_path);
            let target = temporary_root.join(relative_path);
            fs::copy(source, target).expect("copy frozen contract artifact");
        }

        temporary_root
    }
}
