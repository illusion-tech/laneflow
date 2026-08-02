//! 编译器校准证据 v1 的原子写出与独立关系验证。
//!
//! 本模块把受信任契约描述符作为启动根：先认证描述符、证据 Schema 与工作负载清单，
//! 再读取待验证证据。写出器不覆盖既有文件；独立验证器除 JSON Schema 外还检查来源
//! 绑定、全局身份唯一性以及运行/汇总/候选之间的引用闭合。

use crate::{
    ATTRIBUTION_BINARY_ID, CandidateKeyDomain, CandidatePerformanceScopeContract,
    CandidateScaleRole, ContractError, CorridorContract, DIAGNOSTIC_LIMIT_ERROR_CODE,
    DUPLICATE_OWNER_ERROR_CODE, FORMAL_LADDER_MINIMUM_LEVEL_COUNT, GraphProfileId,
    GuardCompletedLevelObservation, GuardThresholds, LIMIT_EXCEEDED_ERROR_CODE, LimitDimensionId,
    LimitPairMode, LimitQualificationPlanner, LiveByteBaseline, LiveByteBaselineReplica,
    ORACLE_BINARY_ID, ScalableGuardPlanner, ScalableStagePlanFactory, ScalableWorkloadId,
    SystemMemoryObservation, TIMING_BINARY_ID, TrustedContract, UNKNOWN_REFERENCE_ERROR_CODE,
    build_current_fixture_summaries, load_repository_contract, repository_root,
    verify_current_fixtures_oracle,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

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
    pub run_count: usize,
    pub invalid_run_count: usize,
    pub guarded_run_count: usize,
    pub binary_count: usize,
    pub candidate_binding_count: usize,
    pub artifact_count: usize,
    pub referenced_run_count: usize,
    pub workload_count_check_count: usize,
    pub base_scale_check_count: usize,
    pub pilot_level_check_count: usize,
    pub round_metric_summary_check_count: usize,
    pub ladder_batch_summary_check_count: usize,
    pub adjacent_ratio_check_count: usize,
    pub knee_check_count: usize,
    pub reproducibility_envelope_check_count: usize,
    pub recommendation_check_count: usize,
    pub growth_slope_check_count: usize,
    pub constant_hash_qualification_check_count: usize,
    pub candidate_roster_check_count: usize,
    pub candidate_comparison_check_count: usize,
    pub guard_preflight_check_count: usize,
    pub limit_pair_check_count: usize,
    pub live_byte_baseline_check_count: usize,
    pub duplicate_owner_qualification_check_count: usize,
    pub cleanup_experiment_check_count: usize,
    pub cleanup_run_check_count: usize,
    pub failure_input_digest_check_count: usize,
    pub diagnostic_digest_check_count: usize,
}

pub fn write_evidence_v1(
    request: &EvidenceWriteRequest,
) -> Result<EvidenceWriteOutcome, EvidenceError> {
    let trusted = load_repository_contract()?;
    let context = VerificationContext::from_repository()?;
    write_evidence_document(request, &trusted, &context)
}

fn write_evidence_document(
    request: &EvidenceWriteRequest,
    trusted: &TrustedContract,
    context: &VerificationContext,
) -> Result<EvidenceWriteOutcome, EvidenceError> {
    let checkpoint_bytes =
        fs::read(&request.checkpoint_path).map_err(|source| EvidenceError::ReadEvidence {
            path: request.checkpoint_path.clone(),
            source,
        })?;
    let checkpoint: crate::FormalProtocolCheckpoint = serde_json::from_slice(&checkpoint_bytes)
        .map_err(|source| EvidenceError::InvalidEvidenceJson {
            path: request.checkpoint_path.clone(),
            source,
        })?;
    let document =
        crate::evidence_assembly::assemble_evidence_document(trusted, context, &checkpoint)?;
    publish_evidence_document(&request.output_path, &document, trusted, context)
}

fn publish_evidence_document(
    output_path: &Path,
    document: &Value,
    trusted: &TrustedContract,
    context: &VerificationContext,
) -> Result<EvidenceWriteOutcome, EvidenceError> {
    if output_path.exists() {
        return Err(EvidenceError::OutputAlreadyExists {
            path: output_path.to_path_buf(),
        });
    }
    let verification = verify_evidence_document(trusted, document, context)?;
    let bytes = serde_json::to_vec_pretty(&document)
        .map_err(|source| EvidenceError::SerializeEvidence { source })?;
    write_bytes_atomically(output_path, &bytes)?;
    Ok(EvidenceWriteOutcome {
        output_path: output_path.to_path_buf(),
        byte_length: u64::try_from(bytes.len() + 1)
            .expect("evidence byte length must fit into u64"),
        sha256: sha256_with_trailing_newline(&bytes),
        verification,
    })
}

pub fn verify_evidence_v1(path: &Path) -> Result<EvidenceVerificationReport, EvidenceError> {
    // 受信任描述符必须在读取、解析证据自报字段之前完成认证。
    let trusted = load_repository_contract()?;
    let bytes = fs::read(path).map_err(|source| EvidenceError::ReadEvidence {
        path: path.to_path_buf(),
        source,
    })?;
    let document: Value =
        serde_json::from_slice(&bytes).map_err(|source| EvidenceError::InvalidEvidenceJson {
            path: path.to_path_buf(),
            source,
        })?;
    let context = VerificationContext::from_repository()?;
    verify_evidence_document(&trusted, &document, &context)
}

#[derive(Clone, Debug)]
pub(crate) struct VerificationContext {
    pub(crate) repository_head: String,
    pub(crate) cargo_lock_sha256: String,
    pub(crate) direct_cargo_packages: BTreeMap<String, CargoPackageBinding>,
    pub(crate) binary_sha256: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub(crate) struct CargoPackageBinding {
    pub(crate) id: String,
    pub(crate) version: String,
    pub(crate) source: String,
    pub(crate) checksum: String,
    pub(crate) features: BTreeSet<String>,
    pub(crate) license: Option<String>,
    pub(crate) rust_version: Option<String>,
}

#[derive(Deserialize)]
struct CargoLockDocument {
    package: Vec<CargoLockPackage>,
}

#[derive(Deserialize)]
struct CargoLockPackage {
    name: String,
    version: String,
    source: Option<String>,
    checksum: Option<String>,
}

#[derive(Deserialize)]
struct CargoMetadataDocument {
    packages: Vec<CargoMetadataPackage>,
    resolve: Option<CargoMetadataResolve>,
    target_directory: PathBuf,
}

#[derive(Deserialize)]
struct CargoMetadataPackage {
    name: String,
    version: String,
    id: String,
    source: Option<String>,
    license: Option<String>,
    rust_version: Option<String>,
}

#[derive(Deserialize)]
struct CargoMetadataResolve {
    nodes: Vec<CargoMetadataNode>,
}

#[derive(Deserialize)]
struct CargoMetadataNode {
    id: String,
    deps: Vec<CargoMetadataDependency>,
    features: Vec<String>,
}

#[derive(Deserialize)]
struct CargoMetadataDependency {
    pkg: String,
}

impl VerificationContext {
    pub(crate) fn from_repository() -> Result<Self, EvidenceError> {
        let root = repository_root();
        let repository_head = command_stdout(
            Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&root),
            "git rev-parse HEAD",
        )?;
        let lock_bytes = fs::read(root.join("Cargo.lock"))
            .map_err(|source| EvidenceError::ReadCargoLock { source })?;
        let lock_text = std::str::from_utf8(&lock_bytes)
            .map_err(|source| EvidenceError::InvalidCargoLockUtf8 { source })?;
        let lock: CargoLockDocument =
            toml::from_str(lock_text).map_err(|source| EvidenceError::InvalidCargoLockToml {
                source: Box::new(source),
            })?;
        let metadata_text = command_stdout(
            Command::new("cargo")
                .args([
                    "+1.96.0",
                    "metadata",
                    "--locked",
                    "--format-version",
                    "1",
                    "--manifest-path",
                ])
                .arg(
                    root.join("research/issue-308-compiler-budget-calibration-research/Cargo.toml"),
                )
                .args([
                    "--no-default-features",
                    "--features",
                    "research-runner-full",
                ])
                .current_dir(&root),
            "cargo metadata",
        )?;
        let metadata: CargoMetadataDocument = serde_json::from_str(&metadata_text)
            .map_err(|source| EvidenceError::InvalidCargoMetadataJson { source })?;
        let direct_cargo_packages = direct_cargo_packages(&metadata, &lock)?;
        let binary_sha256 = repository_binary_sha256(&metadata.target_directory)?;
        Ok(Self {
            repository_head,
            cargo_lock_sha256: sha256_hex(&lock_bytes),
            direct_cargo_packages,
            binary_sha256,
        })
    }
}

fn repository_binary_sha256(
    target_directory: &Path,
) -> Result<BTreeMap<String, String>, EvidenceError> {
    let mut binaries = BTreeMap::new();
    for (binary_id, executable_name) in [
        (
            TIMING_BINARY_ID,
            "issue-308-compiler-budget-calibration-timing",
        ),
        (
            ATTRIBUTION_BINARY_ID,
            "issue-308-compiler-budget-calibration-attribution",
        ),
        (
            ORACLE_BINARY_ID,
            "issue-308-compiler-budget-calibration-oracle",
        ),
    ] {
        let path = target_directory
            .join("release")
            .join(format!("{executable_name}{}", std::env::consts::EXE_SUFFIX));
        if !path.is_file() {
            continue;
        }
        let bytes = fs::read(&path).map_err(|source| EvidenceError::ReadResearchBinary {
            path: path.clone(),
            source,
        })?;
        binaries.insert(binary_id.to_owned(), sha256_hex(&bytes));
    }
    Ok(binaries)
}

fn direct_cargo_packages(
    metadata: &CargoMetadataDocument,
    lock: &CargoLockDocument,
) -> Result<BTreeMap<String, CargoPackageBinding>, EvidenceError> {
    let package_by_id = metadata
        .packages
        .iter()
        .map(|package| (package.id.as_str(), package))
        .collect::<BTreeMap<_, _>>();
    let resolve = metadata
        .resolve
        .as_ref()
        .ok_or(EvidenceError::MissingCargoMetadataResolve)?;
    let harness = metadata
        .packages
        .iter()
        .find(|package| package.name == "issue-308-compiler-budget-calibration-research")
        .ok_or(EvidenceError::MissingHarnessMetadataPackage)?;
    let harness_node = resolve
        .nodes
        .iter()
        .find(|node| node.id == harness.id)
        .ok_or(EvidenceError::MissingHarnessMetadataNode)?;
    let node_by_id = resolve
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let mut packages = BTreeMap::new();
    for dependency in &harness_node.deps {
        let package = package_by_id.get(dependency.pkg.as_str()).ok_or_else(|| {
            EvidenceError::MissingCargoMetadataPackage {
                package_id: dependency.pkg.clone(),
            }
        })?;
        let Some(source) = package.source.as_deref() else {
            continue;
        };
        let locked = lock
            .package
            .iter()
            .find(|locked| {
                locked.name == package.name
                    && locked.version == package.version
                    && locked.source.as_deref() == Some(source)
            })
            .ok_or_else(|| EvidenceError::MissingCargoLockPackage {
                package_id: package.id.clone(),
            })?;
        let checksum =
            locked
                .checksum
                .clone()
                .ok_or_else(|| EvidenceError::MissingCargoLockChecksum {
                    package_id: package.id.clone(),
                })?;
        let node = node_by_id.get(package.id.as_str()).ok_or_else(|| {
            EvidenceError::MissingCargoMetadataNode {
                package_id: package.id.clone(),
            }
        })?;
        packages.insert(
            package.name.clone(),
            CargoPackageBinding {
                id: package.id.clone(),
                version: package.version.clone(),
                source: source.to_owned(),
                checksum,
                features: node.features.iter().cloned().collect(),
                license: package.license.clone(),
                rust_version: package.rust_version.clone(),
            },
        );
    }
    Ok(packages)
}

fn verify_evidence_document(
    trusted: &TrustedContract,
    document: &Value,
    context: &VerificationContext,
) -> Result<EvidenceVerificationReport, EvidenceError> {
    jsonschema::draft202012::validate(&trusted.evidence_schema, document).map_err(|error| {
        EvidenceError::SchemaValidation {
            detail: error.to_string(),
        }
    })?;
    verify_source_bindings(trusted, document, context)?;
    let binaries = unique_object_index(document, "/binaries", "id")?;
    verify_binary_bindings(&binaries, context)?;
    let candidate_bindings = unique_candidate_binding_index(document)?;
    verify_candidate_registry_bindings(trusted, document, &candidate_bindings, context)?;
    let runs = unique_object_index(document, "/runs", "runId")?;
    let artifacts = unique_object_index(document, "/artifacts", "path")?;
    let mut referenced_run_ids = BTreeSet::new();
    let workload_count_check_count = verify_workload_counts(trusted, &runs)?;
    let diagnostic_digest_check_count = verify_diagnostic_digests(trusted, &runs)?;
    let failure_input_digest_check_count = verify_failure_input_digests(trusted, &runs)?;
    let guard_preflight_check_count = verify_guard_preflights(trusted, document, &runs)?;
    let (base_scale_check_count, pilot_level_check_count) =
        verify_base_scales(document, &runs, &mut referenced_run_ids)?;
    let (round_metric_summary_check_count, ladder_batch_summary_check_count) =
        verify_metric_summaries(document, &runs, &mut referenced_run_ids)?;
    let (adjacent_ratio_check_count, knee_check_count) =
        verify_adjacent_ratios_and_knees(document, &runs)?;
    let selected_scales = recompute_selected_scales(trusted, document)?;
    let (
        limit_pair_check_count,
        live_byte_baseline_check_count,
        duplicate_owner_qualification_check_count,
    ) = verify_limit_qualifications(
        trusted,
        document,
        &runs,
        &selected_scales,
        &mut referenced_run_ids,
    )?;
    let (cleanup_experiment_check_count, cleanup_run_check_count) =
        verify_cleanup_experiments(document, &runs, &selected_scales, &mut referenced_run_ids)?;
    let constant_hash_qualification_check_count =
        verify_constant_hash_qualifications(trusted, document, &runs, &mut referenced_run_ids)?;
    let (candidate_roster_check_count, verified_candidate_rosters) = verify_candidate_rosters(
        trusted,
        document,
        &candidate_bindings,
        &runs,
        &mut referenced_run_ids,
    )?;
    let (reproducibility_envelope_check_count, recommendation_check_count) =
        verify_reproducibility_and_recommendations(document)?;
    let growth_slope_check_count = verify_growth_slopes(document, &runs)?;
    let candidate_comparison_check_count = verify_candidate_comparisons(
        trusted,
        document,
        &verified_candidate_rosters,
        &runs,
        &mut referenced_run_ids,
    )?;
    verify_candidate_performance_scope(
        trusted,
        document,
        &verified_candidate_rosters,
        &runs,
        &selected_scales,
    )?;
    verify_selected_scale_role_bindings(document, &runs, &selected_scales)?;

    for (run_id, run) in &runs {
        let binary_id = required_string(run, "/process/binaryId")?;
        if !binaries.contains_key(binary_id) {
            return Err(EvidenceError::UnknownReference {
                owner: format!("run {run_id}"),
                field: "process.binaryId".to_owned(),
                target: binary_id.to_owned(),
            });
        }
        let candidate = required_object(run, "/candidate")?;
        let candidate_key = candidate_binding_key(candidate)?;
        let Some(binding) = candidate_bindings.get(&candidate_key) else {
            return Err(EvidenceError::UnknownReference {
                owner: format!("run {run_id}"),
                field: "candidate".to_owned(),
                target: candidate_key,
            });
        };
        if *binding != candidate {
            return Err(EvidenceError::CandidateSnapshotMismatch {
                run_id: run_id.clone(),
            });
        }
        let status = required_string(run, "/status")?;
        let reasons = required_array(run, "/invalidationReasons")?;
        if status == "valid" && !reasons.is_empty() {
            return Err(EvidenceError::ValidRunHasInvalidationReasons {
                run_id: run_id.clone(),
            });
        }
        verify_external_state(document, run_id, run)?;
    }

    verify_derived_identities_and_references(document, &runs, &mut referenced_run_ids)?;

    let invalid_run_count = runs
        .values()
        .filter(|run| required_string(run, "/status").is_ok_and(|status| status == "invalid"))
        .count();
    let guarded_run_count = runs
        .values()
        .filter(|run| required_string(run, "/status").is_ok_and(|status| status == "guarded"))
        .count();
    Ok(EvidenceVerificationReport {
        schema: required_string(document, "/schema")?.to_owned(),
        schema_version: required_u64(document, "/schemaVersion")?,
        source_commit: required_string(document, "/source/sourceCommit")?.to_owned(),
        run_count: runs.len(),
        invalid_run_count,
        guarded_run_count,
        binary_count: binaries.len(),
        candidate_binding_count: candidate_bindings.len(),
        artifact_count: artifacts.len(),
        referenced_run_count: referenced_run_ids.len(),
        workload_count_check_count,
        base_scale_check_count,
        pilot_level_check_count,
        round_metric_summary_check_count,
        ladder_batch_summary_check_count,
        adjacent_ratio_check_count,
        knee_check_count,
        reproducibility_envelope_check_count,
        recommendation_check_count,
        growth_slope_check_count,
        constant_hash_qualification_check_count,
        candidate_roster_check_count,
        candidate_comparison_check_count,
        guard_preflight_check_count,
        limit_pair_check_count,
        live_byte_baseline_check_count,
        duplicate_owner_qualification_check_count,
        cleanup_experiment_check_count,
        cleanup_run_check_count,
        failure_input_digest_check_count,
        diagnostic_digest_check_count,
    })
}

fn verify_binary_bindings(
    binaries: &BTreeMap<String, &Value>,
    context: &VerificationContext,
) -> Result<(), EvidenceError> {
    let expected_modes = BTreeMap::from([
        (TIMING_BINARY_ID, "timing"),
        (ATTRIBUTION_BINARY_ID, "attribution"),
        (ORACLE_BINARY_ID, "oracle"),
    ]);
    if binaries.keys().map(String::as_str).collect::<BTreeSet<_>>()
        != expected_modes.keys().copied().collect::<BTreeSet<_>>()
    {
        return Err(EvidenceError::BinaryBindingRecomputation {
            detail: "证据必须精确绑定 timing、attribution、oracle 三种研究二进制".to_owned(),
        });
    }
    let expected_features = BTreeSet::from(["research-runner-full"]);
    for (binary_id, binary) in binaries {
        let expected_mode = expected_modes.get(binary_id.as_str()).ok_or_else(|| {
            EvidenceError::BinaryBindingRecomputation {
                detail: format!("证据登记未知研究二进制 {binary_id}"),
            }
        })?;
        expect_string(binary, "/mode", expected_mode)?;
        expect_string(binary, "/cargoProfile", "release")?;
        let features = required_array(binary, "/features")?
            .iter()
            .map(|feature| {
                feature
                    .as_str()
                    .ok_or_else(|| EvidenceError::BinaryBindingRecomputation {
                        detail: format!("二进制 {binary_id} 含非字符串 feature"),
                    })
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        if features != expected_features {
            return Err(EvidenceError::BinaryBindingRecomputation {
                detail: format!("二进制 {binary_id} 的构建 features 不是冻结集合"),
            });
        }
        let expected_sha256 = context.binary_sha256.get(binary_id).ok_or_else(|| {
            EvidenceError::MissingResearchBinary {
                binary_id: binary_id.clone(),
            }
        })?;
        expect_string(binary, "/sha256", expected_sha256)?;
    }
    Ok(())
}

fn verify_candidate_registry_bindings(
    trusted: &TrustedContract,
    document: &Value,
    bindings: &BTreeMap<String, &Value>,
    context: &VerificationContext,
) -> Result<(), EvidenceError> {
    let registry = required_object(&trusted.workload_manifest, "/candidateRegistry")?;
    let candidates = required_array(registry, "/candidates")?;
    let rustc = required_string(document, "/environment/rustc")?;
    let mut expected_keys = BTreeSet::new();
    for candidate in candidates {
        let candidate_id = required_string(candidate, "/id")?;
        for key_domain in required_array(candidate, "/allowedKeyDomains")? {
            let key_domain = key_domain.as_str().ok_or_else(|| {
                EvidenceError::CandidateRegistryRecomputation {
                    detail: format!("候选 {candidate_id} 含非字符串 key domain"),
                }
            })?;
            let key = format!("{candidate_id}/{key_domain}");
            expected_keys.insert(key.clone());
            let binding = bindings
                .get(&key)
                .ok_or_else(|| EvidenceError::UnknownReference {
                    owner: "trusted candidate registry".to_owned(),
                    field: "candidateBindings".to_owned(),
                    target: key.clone(),
                })?;
            expect_u64(binding, "/registryRevision", 1)?;
            expect_string(binding, "/id", candidate_id)?;
            expect_string(binding, "/keyDomain", key_domain)?;
            expect_string(
                binding,
                "/hasherSeedPolicy",
                required_string(candidate, "/hasherSeedPolicy")?,
            )?;
            if binding.pointer("/hasherSeedHexU64/value")
                != candidate.pointer("/fixedHasherSeedHexU64")
            {
                let policy = required_string(candidate, "/hasherSeedPolicy")?;
                if policy == "fixed-u64" {
                    return Err(EvidenceError::CandidateRegistryRecomputation {
                        detail: format!("候选 {key} 的固定种子与注册表不一致"),
                    });
                }
            }
            let expected_components = required_array(candidate, "/components")?;
            let actual_components = required_array(binding, "/components")?;
            if expected_components.len() != actual_components.len() {
                return Err(EvidenceError::CandidateRegistryRecomputation {
                    detail: format!("候选 {key} 的组件数量与注册表不一致"),
                });
            }
            for (expected, actual) in expected_components.iter().zip(actual_components) {
                for field in [
                    "role",
                    "implementationId",
                    "dependencyKind",
                    "dependencySource",
                ] {
                    if expected.get(field) != actual.get(field) {
                        return Err(EvidenceError::CandidateRegistryRecomputation {
                            detail: format!("候选 {key} 的组件字段 {field} 与注册表不一致"),
                        });
                    }
                }
                expect_string(
                    actual,
                    "/dependencyAudit/cargoLockSha256",
                    &context.cargo_lock_sha256,
                )?;
                match required_string(actual, "/dependencyKind")? {
                    "standard-library" => expect_string(actual, "/version", rustc)?,
                    "local-workspace" => {
                        expect_string(actual, "/version", &context.repository_head)?
                    }
                    "crates-io" | "git" => {
                        verify_external_component_binding(actual, context, &key)?
                    }
                    other => {
                        return Err(EvidenceError::CandidateRegistryRecomputation {
                            detail: format!("候选 {key} 使用未知依赖类型 {other}"),
                        });
                    }
                }
            }
        }
    }
    if bindings.keys().cloned().collect::<BTreeSet<_>>() != expected_keys {
        return Err(EvidenceError::CandidateRegistryRecomputation {
            detail: "candidateBindings 含注册表外自然身份".to_owned(),
        });
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct VerifiedCandidateRoster {
    stratum: Value,
    baseline_id: String,
    participant_ids: Vec<String>,
    performance_ids: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CandidateSafetyEvidence {
    Passed,
    Rejected,
    Unavailable,
}

fn verify_candidate_rosters(
    trusted: &TrustedContract,
    document: &Value,
    candidate_bindings: &BTreeMap<String, &Value>,
    runs: &BTreeMap<String, &Value>,
    referenced_run_ids: &mut BTreeSet<String>,
) -> Result<(usize, BTreeMap<String, VerifiedCandidateRoster>), EvidenceError> {
    let registry = required_object(&trusted.workload_manifest, "/candidateRegistry")?;
    let registered = required_array(registry, "/candidates")?;
    let qualifications = unique_object_index(
        document,
        "/derived/constantHashQualifications",
        "qualificationId",
    )?;
    let rosters = unique_object_index(document, "/derived/candidateRosters", "rosterId")?;
    let mut verified = BTreeMap::new();
    for (roster_id, roster) in &rosters {
        let stratum = required_object(roster, "/stratum")?;
        let key_domain = required_string(stratum, "/keyDomain")?;
        let expected_candidate_ids = registered
            .iter()
            .filter(|candidate| {
                candidate
                    .pointer("/allowedKeyDomains")
                    .and_then(Value::as_array)
                    .is_some_and(|domains| domains.iter().any(|domain| domain == key_domain))
            })
            .map(|candidate| required_string(candidate, "/id").map(str::to_owned))
            .collect::<Result<Vec<_>, _>>()?;
        let entries = required_array(roster, "/entries")?;
        let actual_candidate_ids = entries
            .iter()
            .map(|entry| required_string(entry, "/candidateId").map(str::to_owned))
            .collect::<Result<Vec<_>, _>>()?;
        if actual_candidate_ids != expected_candidate_ids {
            return Err(EvidenceError::CandidateRosterRecomputation {
                detail: format!(
                    "候选名单 {roster_id} 未按注册表过滤顺序完整登记：期望 {expected_candidate_ids:?}，实际 {actual_candidate_ids:?}"
                ),
            });
        }
        let expected_baseline =
            required_string(registry, &format!("/baselineByKeyDomain/{key_domain}"))?;
        expect_string(roster, "/baselineId", expected_baseline)?;
        let mut participant_ids = Vec::new();
        let mut performance_ids = BTreeSet::new();
        let mut baseline_participant_count = 0;
        for entry in entries {
            let candidate_id = required_string(entry, "/candidateId")?;
            let disposition = required_string(entry, "/disposition")?;
            if disposition == "baseline-participant" {
                if candidate_id != expected_baseline {
                    return Err(EvidenceError::CandidateRosterRecomputation {
                        detail: format!(
                            "候选名单 {roster_id} 的非基线 {candidate_id} 冒充基线参与者"
                        ),
                    });
                }
                baseline_participant_count += 1;
            } else if candidate_id == expected_baseline && disposition == "performance-participant"
            {
                return Err(EvidenceError::CandidateRosterRecomputation {
                    detail: format!("候选名单 {roster_id} 把基线登记为普通性能参与者"),
                });
            }
            if matches!(
                disposition,
                "baseline-participant" | "performance-participant"
            ) {
                participant_ids.push(candidate_id.to_owned());
            }
            if disposition == "performance-participant" {
                performance_ids.insert(candidate_id.to_owned());
            }

            let binding_key = format!("{candidate_id}/{key_domain}");
            let binding = candidate_bindings.get(&binding_key).ok_or_else(|| {
                EvidenceError::UnknownReference {
                    owner: format!("candidate roster {roster_id}"),
                    field: "candidate binding".to_owned(),
                    target: binding_key,
                }
            })?;
            match candidate_safety_evidence(binding)? {
                CandidateSafetyEvidence::Rejected if disposition != "rejected-safety" => {
                    return Err(EvidenceError::CandidateRosterRecomputation {
                        detail: format!("候选名单 {roster_id}/{candidate_id} 忽略已发现安全公告"),
                    });
                }
                CandidateSafetyEvidence::Unavailable
                    if disposition != "insufficient-qualification-evidence" =>
                {
                    return Err(EvidenceError::CandidateRosterRecomputation {
                        detail: format!(
                            "候选名单 {roster_id}/{candidate_id} 在安全审计不可用时仍作确定性分类"
                        ),
                    });
                }
                CandidateSafetyEvidence::Passed if disposition == "rejected-safety" => {
                    return Err(EvidenceError::CandidateRosterRecomputation {
                        detail: format!(
                            "候选名单 {roster_id}/{candidate_id} 没有支持安全拒绝的绑定证据"
                        ),
                    });
                }
                _ => {}
            }

            verify_roster_correctness_pair(
                roster_id,
                stratum,
                candidate_id,
                disposition,
                entry,
                runs,
                referenced_run_ids,
            )?;
            verify_roster_constant_hash_reference(
                trusted,
                roster_id,
                candidate_id,
                disposition,
                entry,
                &qualifications,
            )?;
        }
        if baseline_participant_count != 1 {
            return Err(EvidenceError::CandidateRosterRecomputation {
                detail: format!(
                    "候选名单 {roster_id} 必须恰有一个通过安全与正确性资格的基线参与者，实际 {baseline_participant_count}"
                ),
            });
        }
        verified.insert(
            roster_id.clone(),
            VerifiedCandidateRoster {
                stratum: stratum.clone(),
                baseline_id: expected_baseline.to_owned(),
                participant_ids,
                performance_ids,
            },
        );
    }
    Ok((rosters.len(), verified))
}

fn candidate_safety_evidence(binding: &Value) -> Result<CandidateSafetyEvidence, EvidenceError> {
    let mut state = CandidateSafetyEvidence::Passed;
    for component in required_array(binding, "/components")? {
        if !matches!(
            required_string(component, "/dependencyKind")?,
            "crates-io" | "git"
        ) {
            continue;
        }
        match required_string(component, "/dependencyAudit/securityAudit/status")? {
            "advisories-present" => return Ok(CandidateSafetyEvidence::Rejected),
            "audit-unavailable" => state = CandidateSafetyEvidence::Unavailable,
            "no-known-advisories" => {
                if component
                    .pointer("/dependencyAudit/licenseSpdxExpression/value")
                    .and_then(Value::as_str)
                    .is_none()
                    || component
                        .pointer("/dependencyAudit/msrvRustVersion/value")
                        .and_then(Value::as_str)
                        .is_none()
                {
                    state = CandidateSafetyEvidence::Unavailable;
                }
            }
            other => {
                return Err(EvidenceError::CandidateRosterRecomputation {
                    detail: format!("第三方候选组件使用非法安全审计状态 {other}"),
                });
            }
        }
    }
    Ok(state)
}

#[allow(clippy::too_many_arguments)]
fn verify_roster_correctness_pair(
    roster_id: &str,
    stratum: &Value,
    candidate_id: &str,
    disposition: &str,
    entry: &Value,
    runs: &BTreeMap<String, &Value>,
    referenced_run_ids: &mut BTreeSet<String>,
) -> Result<(), EvidenceError> {
    let run_ids = required_array(entry, "/correctnessEvidenceRunIds")?;
    let requires_pair = matches!(
        disposition,
        "baseline-participant" | "performance-participant" | "rejected-correctness"
    );
    if requires_pair && run_ids.len() != 2
        || disposition == "rejected-safety" && !run_ids.is_empty()
        || disposition == "insufficient-qualification-evidence" && !matches!(run_ids.len(), 0 | 2)
    {
        return Err(EvidenceError::CandidateRosterRecomputation {
            detail: format!("候选名单 {roster_id}/{candidate_id} 的正确性运行对数量不合法"),
        });
    }
    if run_ids.is_empty() {
        return Ok(());
    }
    let candidate_run_id =
        run_ids[0]
            .as_str()
            .ok_or_else(|| EvidenceError::CandidateRosterRecomputation {
                detail: format!("候选名单 {roster_id}/{candidate_id} 含非字符串候选运行 ID"),
            })?;
    let oracle_run_id =
        run_ids[1]
            .as_str()
            .ok_or_else(|| EvidenceError::CandidateRosterRecomputation {
                detail: format!("候选名单 {roster_id}/{candidate_id} 含非字符串预言机运行 ID"),
            })?;
    if candidate_run_id == oracle_run_id {
        return Err(EvidenceError::CandidateRosterRecomputation {
            detail: format!("候选名单 {roster_id}/{candidate_id} 的候选与预言机运行相同"),
        });
    }
    let candidate_run =
        runs.get(candidate_run_id)
            .copied()
            .ok_or_else(|| EvidenceError::UnknownReference {
                owner: format!("candidate roster {roster_id}/{candidate_id}"),
                field: "candidate-under-test run".to_owned(),
                target: candidate_run_id.to_owned(),
            })?;
    let oracle_run =
        runs.get(oracle_run_id)
            .copied()
            .ok_or_else(|| EvidenceError::UnknownReference {
                owner: format!("candidate roster {roster_id}/{candidate_id}"),
                field: "exact-research-oracle run".to_owned(),
                target: oracle_run_id.to_owned(),
            })?;
    referenced_run_ids.insert(candidate_run_id.to_owned());
    referenced_run_ids.insert(oracle_run_id.to_owned());
    expect_string(candidate_run, "/sampleKind", "candidate-qualification")?;
    expect_string(candidate_run, "/roundAttempt/scope", "single-experiment")?;
    expect_string(candidate_run, "/process/binaryId", TIMING_BINARY_ID)?;
    expect_string(candidate_run, "/candidate/id", candidate_id)?;
    expect_string(
        candidate_run,
        "/candidate/keyDomain",
        required_string(stratum, "/keyDomain")?,
    )?;
    expect_string(
        oracle_run,
        "/candidate/id",
        "baseline-std-randomstate-stable-vec-v1",
    )?;
    expect_string(oracle_run, "/candidate/keyDomain", "full-pipeline-baseline")?;
    expect_string(oracle_run, "/sampleKind", "candidate-qualification")?;
    expect_string(oracle_run, "/roundAttempt/scope", "single-experiment")?;
    expect_string(oracle_run, "/process/binaryId", ORACLE_BINARY_ID)?;
    for run in [candidate_run, oracle_run] {
        if required_string(run, "/status")? != "valid"
            || !run_workload_matches_roster_stratum(run, stratum)?
        {
            return Err(EvidenceError::CandidateRosterRecomputation {
                detail: format!("候选名单 {roster_id}/{candidate_id} 的正确性运行无效或跨分层"),
            });
        }
    }
    let matches_oracle = observed_string(candidate_run, "/metrics/semanticDigest")?
        == observed_string(oracle_run, "/metrics/semanticDigest")?;
    if matches!(
        disposition,
        "baseline-participant" | "performance-participant"
    ) && !matches_oracle
        || disposition == "rejected-correctness" && matches_oracle
    {
        return Err(EvidenceError::CandidateRosterRecomputation {
            detail: format!("候选名单 {roster_id}/{candidate_id} 的处置与语义摘要证据相反"),
        });
    }
    Ok(())
}

fn run_workload_matches_roster_stratum(
    run: &Value,
    stratum: &Value,
) -> Result<bool, EvidenceError> {
    for (run_pointer, stratum_pointer) in [
        ("/workload/id", "/workloadId"),
        ("/workload/graphProfile", "/graphProfile"),
        ("/workload/stringProfile", "/stringProfile"),
        ("/workload/scaleRole", "/scaleRole"),
        ("/workload/caseId", "/caseId"),
        ("/workload/inputVariantId", "/inputVariantId"),
    ] {
        if required_string(run, run_pointer)? != required_string(stratum, stratum_pointer)? {
            return Ok(false);
        }
    }
    for (run_pointer, stratum_pointer) in [
        ("/workload/revision", "/workloadRevision"),
        ("/workload/generatorVersion", "/generatorVersion"),
        ("/workload/n", "/n"),
    ] {
        if required_u64(run, run_pointer)? != required_u64(stratum, stratum_pointer)? {
            return Ok(false);
        }
    }
    Ok(run.pointer("/workload/b") == stratum.pointer("/b"))
}

fn verify_roster_constant_hash_reference(
    trusted: &TrustedContract,
    roster_id: &str,
    candidate_id: &str,
    disposition: &str,
    entry: &Value,
    qualifications: &BTreeMap<String, &Value>,
) -> Result<(), EvidenceError> {
    let fast_candidates = required_array(
        &trusted.workload_manifest,
        "/constantHashQualificationContract/candidateIds",
    )?;
    let is_fast = fast_candidates
        .iter()
        .any(|candidate| candidate == candidate_id);
    let value = entry
        .pointer("/constantHashQualificationId/value")
        .and_then(Value::as_str);
    let reason = entry
        .pointer("/constantHashQualificationId/reason")
        .and_then(Value::as_str);
    if !is_fast {
        if value.is_some() || reason != Some("not-applicable-non-fast-hash-candidate") {
            return Err(EvidenceError::CandidateRosterRecomputation {
                detail: format!("候选名单 {roster_id}/{candidate_id} 的恒定哈希资格应为不适用"),
            });
        }
        return Ok(());
    }
    match disposition {
        "performance-participant" | "rejected-correctness" => {
            let qualification_id =
                value.ok_or_else(|| EvidenceError::CandidateRosterRecomputation {
                    detail: format!("候选名单 {roster_id}/{candidate_id} 缺少必需恒定哈希资格"),
                })?;
            let qualification = qualifications.get(qualification_id).ok_or_else(|| {
                EvidenceError::UnknownReference {
                    owner: format!("candidate roster {roster_id}/{candidate_id}"),
                    field: "constantHashQualificationId".to_owned(),
                    target: qualification_id.to_owned(),
                }
            })?;
            expect_string(qualification, "/candidateId", candidate_id)?;
            if disposition == "performance-participant" && !required_bool(qualification, "/passed")?
            {
                return Err(EvidenceError::CandidateRosterRecomputation {
                    detail: format!("候选名单 {roster_id}/{candidate_id} 以失败资格参加性能比较"),
                });
            }
        }
        "rejected-safety" => {
            if value.is_some() || reason != Some("qualification-not-run-safety-pre-rejection") {
                return Err(EvidenceError::CandidateRosterRecomputation {
                    detail: format!(
                        "候选名单 {roster_id}/{candidate_id} 的安全预拒绝资格原因不匹配"
                    ),
                });
            }
        }
        "insufficient-qualification-evidence" => {
            if let Some(qualification_id) = value {
                let qualification = qualifications.get(qualification_id).ok_or_else(|| {
                    EvidenceError::UnknownReference {
                        owner: format!("candidate roster {roster_id}/{candidate_id}"),
                        field: "constantHashQualificationId".to_owned(),
                        target: qualification_id.to_owned(),
                    }
                })?;
                expect_string(qualification, "/candidateId", candidate_id)?;
            } else if reason != Some("qualification-not-run-insufficient-evidence") {
                return Err(EvidenceError::CandidateRosterRecomputation {
                    detail: format!("候选名单 {roster_id}/{candidate_id} 的资格缺失原因不匹配"),
                });
            }
        }
        other => {
            return Err(EvidenceError::CandidateRosterRecomputation {
                detail: format!("快速哈希候选 {roster_id}/{candidate_id} 使用非法处置 {other}"),
            });
        }
    }
    Ok(())
}

fn verify_candidate_comparisons(
    trusted: &TrustedContract,
    document: &Value,
    rosters: &BTreeMap<String, VerifiedCandidateRoster>,
    runs: &BTreeMap<String, &Value>,
    referenced_run_ids: &mut BTreeSet<String>,
) -> Result<usize, EvidenceError> {
    let round_summaries =
        unique_object_index(document, "/derived/roundMetricSummaries", "summaryId")?;
    let candidate_round_summary_ids = round_summaries
        .iter()
        .filter_map(|(summary_id, summary)| {
            (required_string(summary, "/purpose").ok() == Some("candidate-comparison"))
                .then_some(summary_id.clone())
        })
        .collect::<BTreeSet<_>>();
    let eligible_metrics = required_array(
        &trusted.workload_manifest,
        "/candidateDecisionContract/eligibleMetrics",
    )?
    .iter()
    .filter_map(Value::as_str)
    .collect::<BTreeSet<_>>();
    let incomplete_reason = required_string(
        &trusted.workload_manifest,
        "/candidateDecisionContract/batchEvidenceRules/incompleteBatchMedianRule",
    )?;
    let empty_reason = required_string(
        &trusted.workload_manifest,
        "/candidateDecisionContract/batchEvidenceRules/emptyBatchMedianRule",
    )?;
    let mut envelope_by_metric = BTreeMap::new();
    for envelope in required_array(document, "/derived/reproducibilityEnvelopes")? {
        envelope_by_metric.insert(required_string(envelope, "/metric")?, envelope);
    }
    let comparisons = required_array(document, "/derived/candidateComparisons")?;
    let mut identities = BTreeSet::new();
    let mut consumed_round_summaries = BTreeSet::new();
    for comparison in comparisons {
        let roster_id = required_string(comparison, "/rosterId")?;
        let roster = rosters
            .get(roster_id)
            .ok_or_else(|| EvidenceError::UnknownReference {
                owner: "candidate comparison".to_owned(),
                field: "rosterId".to_owned(),
                target: roster_id.to_owned(),
            })?;
        let candidate_id = required_string(comparison, "/candidateId")?;
        let baseline_id = required_string(comparison, "/baselineId")?;
        let metric = required_string(comparison, "/metric")?;
        let stratum = required_object(comparison, "/stratum")?;
        let identity = format!("{roster_id}/{candidate_id}/{metric}/{stratum}");
        if !identities.insert(identity.clone()) {
            return Err(EvidenceError::DuplicateIdentity {
                collection: "derived.candidateComparisons".to_owned(),
                id: identity,
            });
        }
        if baseline_id != roster.baseline_id
            || !roster.performance_ids.contains(candidate_id)
            || !roster.participant_ids.iter().any(|id| id == baseline_id)
            || !roster_stratum_matches_comparison(&roster.stratum, stratum)?
        {
            return Err(EvidenceError::CandidateComparisonRecomputation {
                detail: format!("候选比较 {identity} 与名单基线、参与者或分层不一致"),
            });
        }
        if !eligible_metrics.contains(metric) {
            return Err(EvidenceError::CandidateComparisonRecomputation {
                detail: format!("候选比较 {identity} 使用非性能资格指标 {metric}"),
            });
        }
        if roster.participant_ids.len() < 2 {
            return Err(EvidenceError::CandidateComparisonRecomputation {
                detail: format!("候选比较 {identity} 的名单少于两个性能参与者"),
            });
        }
        let batch_zero = verify_candidate_comparison_batch(
            comparison,
            "/batch0",
            0,
            roster,
            &round_summaries,
            runs,
            referenced_run_ids,
            &mut consumed_round_summaries,
            incomplete_reason,
            empty_reason,
        )?;
        let batch_one = verify_candidate_comparison_batch(
            comparison,
            "/batch1",
            1,
            roster,
            &round_summaries,
            runs,
            referenced_run_ids,
            &mut consumed_round_summaries,
            incomplete_reason,
            empty_reason,
        )?;
        let envelope = envelope_by_metric.get(metric).copied();
        let expected_decision = match (batch_zero, batch_one, envelope) {
            (Some(batch_zero), Some(batch_one), Some(envelope)) => classify_candidate_decision(
                batch_zero,
                batch_one,
                read_ratio(envelope, "/repeatRatio")?,
            ),
            _ => "insufficient-evidence",
        };
        expect_string(comparison, "/decision", expected_decision)?;
    }
    if consumed_round_summaries != candidate_round_summary_ids {
        return Err(EvidenceError::CandidateComparisonRecomputation {
            detail: "candidate-comparison roundMetricSummary 没有被候选比较完整消费".to_owned(),
        });
    }
    Ok(comparisons.len())
}

#[derive(Clone, Copy, Debug)]
struct ExpectedCandidateStratum {
    key_domain: CandidateKeyDomain,
    scale_role: CandidateScaleRole,
    n: u32,
    b: u32,
}

fn verify_candidate_performance_scope(
    trusted: &TrustedContract,
    document: &Value,
    rosters: &BTreeMap<String, VerifiedCandidateRoster>,
    runs: &BTreeMap<String, &Value>,
    selected_scales: &SelectedScaleMap,
) -> Result<(), EvidenceError> {
    let scope =
        CandidatePerformanceScopeContract::from_trusted_contract(trusted).map_err(|error| {
            EvidenceError::CandidateComparisonRecomputation {
                detail: format!("冻结候选性能范围无效：{error}"),
            }
        })?;
    if required_string(document, "/derived/formalStudyDisposition")? != "formal-analysis-available"
    {
        let has_candidate_summaries = required_array(document, "/derived/roundMetricSummaries")?
            .iter()
            .any(|summary| summary["purpose"] == "candidate-comparison");
        let has_candidate_runs = runs.values().any(|run| {
            run.pointer("/roundAttempt/scope").and_then(Value::as_str)
                == Some("candidate-comparison-round")
                || run.pointer("/sampleKind").and_then(Value::as_str)
                    == Some("candidate-qualification")
        });
        if !rosters.is_empty()
            || !required_array(document, "/derived/candidateComparisons")?.is_empty()
            || has_candidate_summaries
            || has_candidate_runs
        {
            return Err(EvidenceError::CandidateComparisonRecomputation {
                detail: "正式阶梯不足时不得发布候选性能分层、运行或分类".to_owned(),
            });
        }
        return Ok(());
    }
    let calibration_key = (
        scope.workload_id,
        scope.graph_profile,
        "calibration".to_owned(),
    );
    let stress_key = (scope.workload_id, scope.graph_profile, "stress".to_owned());
    let calibration = selected_scales.get(&calibration_key).ok_or_else(|| {
        EvidenceError::CandidateComparisonRecomputation {
            detail: "候选性能范围缺少冻结工作负载的校准规模".to_owned(),
        }
    })?;
    let stress = selected_scales.get(&stress_key).ok_or_else(|| {
        EvidenceError::CandidateComparisonRecomputation {
            detail: "候选性能范围缺少冻结工作负载的压力规模".to_owned(),
        }
    })?;
    if calibration.b != stress.b {
        return Err(EvidenceError::CandidateComparisonRecomputation {
            detail: "候选性能范围的校准规模与压力规模没有绑定同一个 B".to_owned(),
        });
    }
    let b = u32::try_from(calibration.b).map_err(|_| {
        EvidenceError::CandidateComparisonRecomputation {
            detail: "候选性能范围的 B 超出 u32".to_owned(),
        }
    })?;
    let scales = [
        (CandidateScaleRole::Base, b),
        (CandidateScaleRole::Calibration, calibration.n),
        (CandidateScaleRole::Stress, stress.n),
    ];
    let key_domains = [
        CandidateKeyDomain::ExternalString,
        CandidateKeyDomain::ValidatedFixedKey,
        CandidateKeyDomain::CanonicalOutputOrder,
    ];
    let mut expected_rosters = BTreeMap::new();
    for (scale_role, n) in scales {
        for key_domain in key_domains {
            let roster_id = expected_candidate_roster_id(&scope, key_domain, scale_role, n);
            expected_rosters.insert(
                roster_id,
                ExpectedCandidateStratum {
                    key_domain,
                    scale_role,
                    n,
                    b,
                },
            );
        }
    }
    if rosters.keys().collect::<BTreeSet<_>>() != expected_rosters.keys().collect::<BTreeSet<_>>() {
        return Err(EvidenceError::CandidateRosterRecomputation {
            detail: "候选名单没有精确覆盖冻结范围的三个规模角色和三个键域".to_owned(),
        });
    }
    for (roster_id, roster) in rosters {
        let expected = expected_rosters
            .get(roster_id)
            .expect("roster key set was checked above");
        verify_candidate_scope_stratum(&scope, &roster.stratum, *expected, false)?;
    }

    let mut expected_comparisons = BTreeSet::new();
    for (roster_id, roster) in rosters {
        for candidate_id in &roster.performance_ids {
            expected_comparisons.insert((roster_id.clone(), candidate_id.clone()));
        }
    }
    let mut actual_comparisons = BTreeSet::new();
    for comparison in required_array(document, "/derived/candidateComparisons")? {
        let roster_id = required_string(comparison, "/rosterId")?;
        let candidate_id = required_string(comparison, "/candidateId")?;
        let expected = expected_rosters.get(roster_id).ok_or_else(|| {
            EvidenceError::CandidateComparisonRecomputation {
                detail: format!("候选比较引用冻结范围外名单 {roster_id}"),
            }
        })?;
        verify_candidate_scope_stratum(
            &scope,
            required_object(comparison, "/stratum")?,
            *expected,
            true,
        )?;
        expect_string(comparison, "/metric", "wall-time-ns")?;
        actual_comparisons.insert((roster_id.to_owned(), candidate_id.to_owned()));
    }
    if actual_comparisons != expected_comparisons {
        return Err(EvidenceError::CandidateComparisonRecomputation {
            detail: "候选比较没有精确覆盖每个冻结名单的全部性能参与者".to_owned(),
        });
    }

    for summary in required_array(document, "/derived/roundMetricSummaries")? {
        if required_string(summary, "/purpose")? != "candidate-comparison" {
            continue;
        }
        expect_string(summary, "/metric", "wall-time-ns")?;
        let stratum = required_object(summary, "/stratum")?;
        let expected = expected_candidate_stratum_from_value(&scope, stratum, &expected_rosters)?;
        verify_candidate_scope_stratum(&scope, stratum, expected, true)?;
    }

    for (run_id, run) in runs {
        if required_string(run, "/roundAttempt/scope")? != "candidate-comparison-round" {
            continue;
        }
        expect_string(run, "/sampleKind", &scope.sample_kind)?;
        expect_string(run, "/process/binaryId", TIMING_BINARY_ID)?;
        let workload = required_object(run, "/workload")?;
        let key_domain = required_string(run, "/candidate/keyDomain")?;
        let roster_id = format!(
            "candidate-roster/{}/{}/{}/{}/{}/n-{}",
            scope.scope_id,
            required_string(workload, "/scaleRole")?,
            key_domain,
            scope.workload_id.as_str(),
            scope.graph_profile.as_str(),
            required_u64(workload, "/n")?,
        );
        let expected = expected_rosters.get(&roster_id).ok_or_else(|| {
            EvidenceError::CandidateComparisonRecomputation {
                detail: format!("候选性能运行 {run_id} 位于冻结范围外"),
            }
        })?;
        verify_candidate_scope_workload(&scope, workload, *expected)?;
        let candidate_id = required_string(run, "/candidate/id")?;
        if !rosters[&roster_id]
            .participant_ids
            .iter()
            .any(|participant| participant == candidate_id)
        {
            return Err(EvidenceError::CandidateComparisonRecomputation {
                detail: format!("候选性能运行 {run_id} 不属于其冻结名单"),
            });
        }
    }
    Ok(())
}

fn expected_candidate_roster_id(
    scope: &CandidatePerformanceScopeContract,
    key_domain: CandidateKeyDomain,
    scale_role: CandidateScaleRole,
    n: u32,
) -> String {
    format!(
        "candidate-roster/{}/{}/{}/{}/{}/n-{n}",
        scope.scope_id,
        scale_role.as_str(),
        key_domain.as_str(),
        scope.workload_id.as_str(),
        scope.graph_profile.as_str(),
    )
}

fn expected_candidate_stratum_from_value(
    scope: &CandidatePerformanceScopeContract,
    stratum: &Value,
    expected_rosters: &BTreeMap<String, ExpectedCandidateStratum>,
) -> Result<ExpectedCandidateStratum, EvidenceError> {
    let roster_id = format!(
        "candidate-roster/{}/{}/{}/{}/{}/n-{}",
        scope.scope_id,
        required_string(stratum, "/scaleRole")?,
        required_string(stratum, "/keyDomain")?,
        scope.workload_id.as_str(),
        scope.graph_profile.as_str(),
        required_u64(stratum, "/n")?,
    );
    expected_rosters.get(&roster_id).copied().ok_or_else(|| {
        EvidenceError::CandidateComparisonRecomputation {
            detail: format!("候选分层 {roster_id} 位于冻结范围外"),
        }
    })
}

fn verify_candidate_scope_stratum(
    scope: &CandidatePerformanceScopeContract,
    stratum: &Value,
    expected: ExpectedCandidateStratum,
    measurement: bool,
) -> Result<(), EvidenceError> {
    expect_string(stratum, "/keyDomain", expected.key_domain.as_str())?;
    expect_string(stratum, "/workloadId", scope.workload_id.as_str())?;
    expect_u64(
        stratum,
        "/workloadRevision",
        u64::from(scope.workload_revision),
    )?;
    expect_string(stratum, "/graphProfile", scope.graph_profile.as_str())?;
    expect_string(stratum, "/stringProfile", &scope.string_profile)?;
    expect_u64(
        stratum,
        "/generatorVersion",
        u64::from(scope.generator_version),
    )?;
    expect_u64(stratum, "/n", u64::from(expected.n))?;
    if observed_u64(stratum, "/b")? != u64::from(expected.b) {
        return Err(EvidenceError::CandidateComparisonRecomputation {
            detail: "候选分层的 B 与正式阶梯选择不一致".to_owned(),
        });
    }
    expect_string(stratum, "/scaleRole", expected.scale_role.as_str())?;
    expect_string(stratum, "/caseId", &scope.case_id)?;
    expect_string(stratum, "/inputVariantId", &scope.input_variant_id)?;
    if measurement {
        expect_string(stratum, "/sampleKind", &scope.sample_kind)?;
        expect_string(stratum, "/binaryMode", &scope.binary_mode)?;
    }
    Ok(())
}

fn verify_candidate_scope_workload(
    scope: &CandidatePerformanceScopeContract,
    workload: &Value,
    expected: ExpectedCandidateStratum,
) -> Result<(), EvidenceError> {
    expect_string(workload, "/id", scope.workload_id.as_str())?;
    expect_u64(workload, "/revision", u64::from(scope.workload_revision))?;
    expect_string(workload, "/graphProfile", scope.graph_profile.as_str())?;
    expect_string(workload, "/stringProfile", &scope.string_profile)?;
    expect_u64(
        workload,
        "/generatorVersion",
        u64::from(scope.generator_version),
    )?;
    expect_u64(workload, "/n", u64::from(expected.n))?;
    if observed_u64(workload, "/b")? != u64::from(expected.b) {
        return Err(EvidenceError::CandidateComparisonRecomputation {
            detail: "候选性能运行的 B 与正式阶梯选择不一致".to_owned(),
        });
    }
    expect_string(workload, "/scaleRole", expected.scale_role.as_str())?;
    expect_string(workload, "/caseId", &scope.case_id)?;
    expect_string(workload, "/inputVariantId", &scope.input_variant_id)?;
    Ok(())
}

fn roster_stratum_matches_comparison(
    roster: &Value,
    comparison: &Value,
) -> Result<bool, EvidenceError> {
    for pointer in [
        "/keyDomain",
        "/workloadId",
        "/workloadRevision",
        "/graphProfile",
        "/stringProfile",
        "/generatorVersion",
        "/n",
        "/b",
        "/scaleRole",
        "/caseId",
        "/inputVariantId",
    ] {
        if roster.pointer(pointer) != comparison.pointer(pointer) {
            return Ok(false);
        }
    }
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
fn verify_candidate_comparison_batch(
    comparison: &Value,
    pointer: &str,
    expected_batch: u64,
    roster: &VerifiedCandidateRoster,
    round_summaries: &BTreeMap<String, &Value>,
    runs: &BTreeMap<String, &Value>,
    referenced_run_ids: &mut BTreeSet<String>,
    consumed_round_summaries: &mut BTreeSet<String>,
    incomplete_reason: &str,
    empty_reason: &str,
) -> Result<Option<PositiveRatio>, EvidenceError> {
    let batch = required_object(comparison, pointer)?;
    let pairs = required_array(batch, "/roundPairs")?;
    let expected_round_count = roster.participant_ids.len().checked_mul(2).ok_or_else(|| {
        EvidenceError::CandidateComparisonRecomputation {
            detail: "候选比较轮次数溢出".to_owned(),
        }
    })?;
    let mut rounds = BTreeSet::new();
    let mut ratios = Vec::new();
    let candidate_id = required_string(comparison, "/candidateId")?;
    let baseline_id = required_string(comparison, "/baselineId")?;
    for pair in pairs {
        let round = required_u64(pair, "/round")?;
        if !rounds.insert(round) {
            return Err(EvidenceError::CandidateComparisonRecomputation {
                detail: format!("候选比较 {pointer} 重复登记 round={round}"),
            });
        }
        let baseline_summary_id = required_string(pair, "/baselineRoundSummaryId")?;
        let candidate_summary_id = required_string(pair, "/candidateRoundSummaryId")?;
        let baseline = round_summaries
            .get(baseline_summary_id)
            .copied()
            .ok_or_else(|| EvidenceError::UnknownReference {
                owner: format!("candidate comparison {pointer}"),
                field: "baselineRoundSummaryId".to_owned(),
                target: baseline_summary_id.to_owned(),
            })?;
        let candidate = round_summaries
            .get(candidate_summary_id)
            .copied()
            .ok_or_else(|| EvidenceError::UnknownReference {
                owner: format!("candidate comparison {pointer}"),
                field: "candidateRoundSummaryId".to_owned(),
                target: candidate_summary_id.to_owned(),
            })?;
        for (summary_id, summary, expected_candidate) in [
            (baseline_summary_id, baseline, baseline_id),
            (candidate_summary_id, candidate, candidate_id),
        ] {
            if required_string(summary, "/purpose")? != "candidate-comparison"
                || required_string(summary, "/candidateId")? != expected_candidate
                || required_string(summary, "/metric")? != required_string(comparison, "/metric")?
                || required_u64(summary, "/batch")? != expected_batch
                || required_u64(summary, "/round")? != round
                || summary.pointer("/stratum") != comparison.pointer("/stratum")
            {
                return Err(EvidenceError::CandidateComparisonRecomputation {
                    detail: format!("候选比较轮次汇总 {summary_id} 身份不闭合"),
                });
            }
            verify_balanced_candidate_position(summary, roster, expected_candidate, round, runs)?;
            for run_id in required_array(summary, "/contributingRunIds")? {
                referenced_run_ids.insert(
                    run_id
                        .as_str()
                        .ok_or_else(|| EvidenceError::CandidateComparisonRecomputation {
                            detail: format!("候选比较汇总 {summary_id} 含非字符串运行 ID"),
                        })?
                        .to_owned(),
                );
            }
            consumed_round_summaries.insert(summary_id.to_owned());
        }
        if required_string(baseline, "/roundAttemptId")?
            != required_string(candidate, "/roundAttemptId")?
        {
            return Err(EvidenceError::CandidateComparisonRecomputation {
                detail: format!("候选比较 {pointer}/round={round} 跨 round attempt 配对"),
            });
        }
        let baseline_median = required_u64(baseline, "/median")?;
        let candidate_median = required_u64(candidate, "/median")?;
        if baseline_median == 0 || candidate_median == 0 {
            expect_null_ratio(pair, "/ratio", None)?;
        } else {
            let ratio = exact_ratio(u128::from(candidate_median), u128::from(baseline_median))?;
            expect_present_ratio(pair, "/ratio", ratio)?;
            ratios.push(ratio);
        }
    }
    let complete_rounds = rounds
        == (0..u64::try_from(expected_round_count).expect("candidate count must fit u64"))
            .collect::<BTreeSet<_>>();
    let complete = complete_rounds && ratios.len() == expected_round_count;
    if complete {
        let median = exact_even_ratio_median(&ratios)?;
        expect_present_ratio(batch, "/medianRatio", median)?;
        Ok(Some(median))
    } else {
        let reason = if pairs.is_empty() {
            empty_reason
        } else {
            incomplete_reason
        };
        expect_null_ratio(batch, "/medianRatio", Some(reason))?;
        Ok(None)
    }
}

fn verify_balanced_candidate_position(
    summary: &Value,
    roster: &VerifiedCandidateRoster,
    candidate_id: &str,
    round: u64,
    runs: &BTreeMap<String, &Value>,
) -> Result<(), EvidenceError> {
    let count = roster.participant_ids.len();
    let index = roster
        .participant_ids
        .iter()
        .position(|candidate| candidate == candidate_id)
        .ok_or_else(|| EvidenceError::CandidateComparisonRecomputation {
            detail: format!("候选 {candidate_id} 不在性能参与者顺序中"),
        })?;
    let round =
        usize::try_from(round).map_err(|_| EvidenceError::CandidateComparisonRecomputation {
            detail: "候选比较 round 超出 usize".to_owned(),
        })?;
    let expected_position = if round < count {
        (index + count - round) % count
    } else if round < 2 * count {
        ((round - count) + count - index) % count
    } else {
        return Err(EvidenceError::CandidateComparisonRecomputation {
            detail: format!("候选比较 round={round} 超出 2C 平衡顺序"),
        });
    };
    let expected_position = u64::try_from(expected_position).expect("position must fit u64");
    for run_id in required_array(summary, "/contributingRunIds")? {
        let run_id =
            run_id
                .as_str()
                .ok_or_else(|| EvidenceError::CandidateComparisonRecomputation {
                    detail: "候选比较含非字符串运行 ID".to_owned(),
                })?;
        let run = runs
            .get(run_id)
            .copied()
            .ok_or_else(|| EvidenceError::UnknownReference {
                owner: "candidate comparison position".to_owned(),
                field: "contributingRunIds".to_owned(),
                target: run_id.to_owned(),
            })?;
        if required_u64(run, "/position")? != expected_position {
            return Err(EvidenceError::CandidateComparisonRecomputation {
                detail: format!(
                    "候选比较运行 {run_id} 的位置不符合 forward/reverse cyclic 2C 顺序"
                ),
            });
        }
    }
    Ok(())
}

fn exact_even_ratio_median(ratios: &[PositiveRatio]) -> Result<PositiveRatio, EvidenceError> {
    if ratios.is_empty() || !ratios.len().is_multiple_of(2) {
        return Err(EvidenceError::CandidateComparisonRecomputation {
            detail: format!("候选比较中位数需要非空偶数样本，实际 {}", ratios.len()),
        });
    }
    let mut sorted = ratios.to_vec();
    sorted.sort_by(|left, right| compare_positive_fractions(*left, *right));
    let upper = sorted.len() / 2;
    let left = sorted[upper - 1];
    let right = sorted[upper];
    let numerator = left
        .numerator
        .checked_mul(right.denominator)
        .and_then(|value| {
            right
                .numerator
                .checked_mul(left.denominator)
                .and_then(|right_value| value.checked_add(right_value))
        })
        .ok_or_else(candidate_ratio_overflow)?;
    let denominator = left
        .denominator
        .checked_mul(right.denominator)
        .and_then(|value| value.checked_mul(2))
        .ok_or_else(candidate_ratio_overflow)?;
    exact_ratio(numerator, denominator)
}

fn classify_candidate_decision(
    batch_zero: PositiveRatio,
    batch_one: PositiveRatio,
    envelope: PositiveRatio,
) -> &'static str {
    let inverse_envelope = PositiveRatio {
        numerator: envelope.denominator,
        denominator: envelope.numerator,
    };
    let ratios = [batch_zero, batch_one];
    if ratios
        .iter()
        .all(|ratio| compare_positive_fractions(*ratio, inverse_envelope) == Ordering::Less)
    {
        "repeatable-improvement"
    } else if ratios
        .iter()
        .all(|ratio| compare_positive_fractions(*ratio, envelope) == Ordering::Greater)
    {
        "repeatable-regression"
    } else if ratios.iter().all(|ratio| {
        compare_positive_fractions(*ratio, inverse_envelope) != Ordering::Less
            && compare_positive_fractions(*ratio, envelope) != Ordering::Greater
    }) {
        "noise-no-difference"
    } else {
        "insufficient-evidence"
    }
}

fn expect_present_ratio(
    value: &Value,
    pointer: &str,
    expected: PositiveRatio,
) -> Result<(), EvidenceError> {
    if value.pointer(&format!("{pointer}/reason")) != Some(&Value::Null) {
        return Err(EvidenceError::CandidateComparisonRecomputation {
            detail: format!("{pointer} 有比值时 reason 必须为空"),
        });
    }
    expect_ratio(value, &format!("{pointer}/value"), expected)
}

fn expect_null_ratio(
    value: &Value,
    pointer: &str,
    expected_reason: Option<&str>,
) -> Result<(), EvidenceError> {
    if value.pointer(&format!("{pointer}/value")) != Some(&Value::Null) {
        return Err(EvidenceError::CandidateComparisonRecomputation {
            detail: format!("{pointer} 应为不可比较的空比值"),
        });
    }
    if let Some(expected_reason) = expected_reason {
        expect_string(value, &format!("{pointer}/reason"), expected_reason)?;
    }
    Ok(())
}

fn candidate_ratio_overflow() -> EvidenceError {
    EvidenceError::CandidateComparisonRecomputation {
        detail: "候选比较精确比值算术溢出".to_owned(),
    }
}

fn verify_external_state(document: &Value, run_id: &str, run: &Value) -> Result<(), EvidenceError> {
    let external = required_object(run, "/externalState")?;
    let mut expected = BTreeSet::new();
    if required_string(external, "/powerSource")?
        != required_string(document, "/environment/powerSource")?
    {
        expected.insert("power-source-change");
    }
    if required_string(external, "/powerPlan")?
        != required_string(document, "/environment/powerPlan")?
    {
        expected.insert("power-plan-change");
    }
    if required_string(external, "/vendorPerformanceMode")?
        != required_string(document, "/environment/vendorPerformanceMode")?
    {
        expected.insert("vendor-mode-change");
    }
    if required_bool(external, "/sleepOrSessionLock")? {
        expected.insert("sleep-or-session-lock");
    }
    if required_bool(external, "/thermalOrPowerThrottling")? {
        expected.insert("thermal-or-power-throttling");
    }
    let _background_cpu_time_ns = nullable_u64(external, "/backgroundCpuTimeNs")?;
    let _background_write_bytes = nullable_u64(external, "/backgroundWriteBytes")?;
    if required_bool(external, "/monitoringGap")? {
        expected.insert("monitoring-gap");
    }

    let external_reason_names = BTreeSet::from([
        "power-source-change",
        "power-plan-change",
        "vendor-mode-change",
        "sleep-or-session-lock",
        "thermal-or-power-throttling",
        "monitoring-gap",
    ]);
    let actual = required_array(run, "/invalidationReasons")?
        .iter()
        .filter_map(Value::as_str)
        .filter(|reason| external_reason_names.contains(reason))
        .collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(EvidenceError::ExternalStateRecomputation {
            run_id: run_id.to_owned(),
            detail: format!("期望 {expected:?}，实际 {actual:?}"),
        });
    }
    if !expected.is_empty() && required_string(run, "/status")? != "invalid" {
        return Err(EvidenceError::ExternalStateRecomputation {
            run_id: run_id.to_owned(),
            detail: "存在外部状态作废原因但运行不是 invalid".to_owned(),
        });
    }
    Ok(())
}

fn verify_guard_preflights(
    trusted: &TrustedContract,
    document: &Value,
    runs: &BTreeMap<String, &Value>,
) -> Result<usize, EvidenceError> {
    let physical_memory_bytes = required_u64(document, "/environment/physicalMemoryBytes")?;
    let expected_thresholds = GuardThresholds::from_physical_memory_bytes(physical_memory_bytes)
        .map_err(|error| EvidenceError::GuardRecomputation {
            detail: error.to_string(),
        })?;
    let expected_thresholds_value =
        serde_json::to_value(expected_thresholds).expect("guard thresholds must serialize");
    if document.pointer("/protocol/guardThresholds") != Some(&expected_thresholds_value) {
        return Err(EvidenceError::GuardRecomputation {
            detail: "protocol.guardThresholds 不是由冻结物理内存规则重算的值".to_owned(),
        });
    }
    let planner = ScalableGuardPlanner::from_trusted_contract(trusted).map_err(|error| {
        EvidenceError::GuardRecomputation {
            detail: error.to_string(),
        }
    })?;
    let mut checked = 0;
    for (run_id, run) in runs {
        if required_string(run, "/sampleKind")? != "guard-preflight" {
            continue;
        }
        let workload_id = required_string(run, "/workload/id")?
            .parse::<ScalableWorkloadId>()
            .map_err(|error| EvidenceError::GuardRecomputation {
                detail: error.to_string(),
            })?;
        let graph_profile = parse_graph_profile(required_string(run, "/workload/graphProfile")?)?;
        let n = u32::try_from(required_u64(run, "/workload/n")?).map_err(|_| {
            EvidenceError::GuardRecomputation {
                detail: format!("guard run {run_id} 的 N 超出 u32"),
            }
        })?;
        let previous = guard_previous_observation(run_id, run)?;
        let report = planner
            .evaluate(
                workload_id,
                graph_profile,
                n,
                SystemMemoryObservation {
                    physical_memory_bytes,
                    available_physical_memory_bytes: required_u64(
                        run,
                        "/guard/lastAvailablePhysicalMemoryBytes",
                    )?,
                },
                previous,
            )
            .map_err(|error| EvidenceError::GuardRecomputation {
                detail: format!("guard run {run_id}: {error}"),
            })?;
        let expected_report =
            serde_json::to_value(&report).expect("guard report must serialize to JSON");
        for (evidence_pointer, report_pointer) in [
            (
                "/guard/compilerControlledPredictionBasis",
                "/compilerControlledPredictionBasis",
            ),
            (
                "/guard/privateBytesPredictionBasis",
                "/privateBytesPredictionBasis",
            ),
            ("/guard/wallTimePredictionBasis", "/wallTimePredictionBasis"),
        ] {
            expect_string(
                run,
                evidence_pointer,
                required_string(&expected_report, report_pointer)?,
            )?;
        }
        expect_u64(
            run,
            "/guard/nextPrimaryRecordCount",
            report.primary_record_count,
        )?;
        expect_u64(
            run,
            "/guard/logicalBytesLowerBound",
            report.logical_bytes_lower_bound,
        )?;
        if nullable_u64(run, "/guard/predictedCompilerControlledBytes")?
            != Some(report.predicted_compiler_controlled_bytes)
            || nullable_u64(run, "/guard/predictedPrivateBytes")? != report.predicted_private_bytes
            || nullable_u64(run, "/guard/predictedWallTimeNs")? != report.predicted_wall_time_ns
        {
            return Err(EvidenceError::GuardRecomputation {
                detail: format!("guard run {run_id} 的预测值与独立重算不一致"),
            });
        }
        let expected_trigger = report
            .triggers
            .first()
            .map(|trigger| {
                serde_json::to_value(trigger)
                    .expect("guard trigger must serialize")
                    .as_str()
                    .expect("guard trigger serializes to string")
                    .to_owned()
            })
            .unwrap_or_else(|| "none".to_owned());
        expect_string(run, "/guard/trigger", &expected_trigger)?;
        if report.allows_child_start
            || required_u64(run, "/guard/reservedBytesBeforeFailure")? != 0
            || nullable_u64(run, "/guard/lastPrivateBytes")?.is_some()
        {
            return Err(EvidenceError::GuardRecomputation {
                detail: format!("guard-preflight run {run_id} 不满足启动前拒绝语义"),
            });
        }
        checked += 1;
    }
    Ok(checked)
}

fn guard_previous_observation(
    run_id: &str,
    run: &Value,
) -> Result<Option<GuardCompletedLevelObservation>, EvidenceError> {
    let fields = [
        nullable_u64(run, "/guard/previousCompletedN")?,
        nullable_u64(run, "/guard/previousPrimaryRecordCount")?,
        nullable_u64(run, "/guard/previousPeakLiveRequestedBytes")?,
        nullable_u64(run, "/guard/previousPrivateBytes")?,
        nullable_u64(run, "/guard/previousWallTimeNs")?,
    ];
    if fields.iter().all(Option::is_none) {
        return Ok(None);
    }
    let [
        Some(n),
        Some(primary_record_count),
        Some(peak_live_requested_bytes),
        Some(private_bytes),
        Some(wall_time_ns),
    ] = fields
    else {
        return Err(EvidenceError::GuardRecomputation {
            detail: format!("guard run {run_id} 的前一级观察不是全空或全量"),
        });
    };
    Ok(Some(GuardCompletedLevelObservation {
        n: u32::try_from(n).map_err(|_| EvidenceError::GuardRecomputation {
            detail: format!("guard run {run_id} 的 previousCompletedN 超出 u32"),
        })?,
        primary_record_count,
        peak_live_requested_bytes,
        private_bytes,
        wall_time_ns,
    }))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct IndependentDiagnosticRecord {
    code: String,
    source_document_key: String,
    start_line: u32,
    start_column: u32,
    end_line: u32,
    end_column: u32,
    severity: u8,
    typed_payload: Vec<u8>,
}

type IndependentDiagnosticCanonicalKey<'a> = (&'a [u8], u32, u32, u32, u32, &'a [u8], u8, &'a [u8]);

impl IndependentDiagnosticRecord {
    fn canonical_key(&self) -> IndependentDiagnosticCanonicalKey<'_> {
        (
            self.source_document_key.as_bytes(),
            self.start_line,
            self.start_column,
            self.end_line,
            self.end_column,
            self.code.as_bytes(),
            self.severity,
            self.typed_payload.as_slice(),
        )
    }
}

fn verify_diagnostic_digests(
    trusted: &TrustedContract,
    runs: &BTreeMap<String, &Value>,
) -> Result<usize, EvidenceError> {
    let mut checked = 0;
    for (run_id, run) in runs {
        let actual = nullable_string(run, "/metrics/diagnosticDigest")?;
        if actual.is_none() {
            if required_string(run, "/status")? == "valid" {
                return Err(EvidenceError::DiagnosticRecomputation {
                    detail: format!("有效运行 {run_id} 缺少诊断摘要"),
                });
            }
            continue;
        }
        let expected = if run.get("correctnessQualification").is_some()
            && !run["correctnessQualification"].is_null()
        {
            match required_string(run, "/correctnessQualification/inputVariantId")? {
                "constant-hash-canonical-valid-v1" => {
                    independent_diagnostic_digest(trusted, Vec::new())?
                }
                "constant-hash-missing-reference-v1" => {
                    independent_unknown_reference_diagnostic_digest(trusted, run)?
                }
                other => {
                    return Err(EvidenceError::DiagnosticRecomputation {
                        detail: format!("运行 {run_id} 使用未知恒定哈希输入变体 {other}"),
                    });
                }
            }
        } else if run.get("failure").is_none()
            || nullable_string(run, "/failure/stableCompilerErrorCode")?.is_none()
        {
            independent_diagnostic_digest(trusted, Vec::new())?
        } else {
            let error_code = nullable_string(run, "/failure/stableCompilerErrorCode")?
                .expect("checked diagnostic error code");
            match error_code {
                LIMIT_EXCEEDED_ERROR_CODE => independent_limit_diagnostic_digest(trusted, run)?,
                UNKNOWN_REFERENCE_ERROR_CODE | DIAGNOSTIC_LIMIT_ERROR_CODE => {
                    independent_unknown_reference_diagnostic_digest(trusted, run)?
                }
                DUPLICATE_OWNER_ERROR_CODE => {
                    independent_duplicate_owner_diagnostic_digest(trusted, run)?
                }
                other => {
                    return Err(EvidenceError::DiagnosticRecomputation {
                        detail: format!("运行 {run_id} 使用尚未实现独立重建的诊断码 {other}"),
                    });
                }
            }
        };
        if actual != Some(expected.as_str()) {
            return Err(EvidenceError::DiagnosticRecomputation {
                detail: format!("运行 {run_id} 的诊断摘要不一致"),
            });
        }
        checked += 1;
    }
    Ok(checked)
}

fn verify_failure_input_digests(
    trusted: &TrustedContract,
    runs: &BTreeMap<String, &Value>,
) -> Result<usize, EvidenceError> {
    let mut checked = 0;
    for (run_id, run) in runs {
        let Some(failure) = run.get("failure") else {
            continue;
        };
        if failure.is_null() {
            continue;
        }
        let workload_id = required_string(run, "/workload/id")?
            .parse::<ScalableWorkloadId>()
            .map_err(|error| EvidenceError::FailureInputRecomputation {
                detail: format!("运行 {run_id} 的工作负载无法解析：{error}"),
            })?;
        let graph_profile = parse_graph_profile(required_string(run, "/workload/graphProfile")?)?;
        let n = u32::try_from(required_u64(run, "/workload/n")?).map_err(|_| {
            EvidenceError::FailureInputRecomputation {
                detail: format!("运行 {run_id} 的 N 超出 u32"),
            }
        })?;
        let plan = ScalableStagePlanFactory::from_trusted_contract(trusted)
            .map_err(|error| EvidenceError::FailureInputRecomputation {
                detail: error.to_string(),
            })?
            .plan(workload_id, graph_profile, n)
            .map_err(|error| EvidenceError::FailureInputRecomputation {
                detail: error.to_string(),
            })?;
        let case_id = required_string(run, "/failure/caseId")?;
        let input_variant_id = required_string(run, "/failure/inputVariantId")?;
        let (value_basis, basis_run_ids) = if run.pointer("/failure/limitSelection").is_some() {
            let mut basis_run_ids = required_array(run, "/failure/limitSelection/basisRunIds")?
                .iter()
                .map(|value| {
                    value.as_str().map(str::to_owned).ok_or_else(|| {
                        EvidenceError::FailureInputRecomputation {
                            detail: format!("运行 {run_id} 的失败输入基准运行 ID 非字符串"),
                        }
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            basis_run_ids.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
            (
                required_string(run, "/failure/limitSelection/valueBasis")?,
                basis_run_ids,
            )
        } else {
            let value_basis = if case_id == "limit/source-byte-count/plus-one" {
                "canonical-level-exact-value"
            } else {
                "not-applicable"
            };
            (value_basis, Vec::new())
        };
        let parameters =
            if case_id.starts_with("limit/") && run.pointer("/failure/limitSelection").is_some() {
                let dimension_name = required_string(run, "/failure/dimensionId")?;
                let dimension = LimitDimensionId::ALL
                    .into_iter()
                    .find(|candidate| candidate.as_str() == dimension_name)
                    .ok_or_else(|| EvidenceError::FailureInputRecomputation {
                        detail: format!("运行 {run_id} 使用未知限制维度 {dimension_name}"),
                    })?;
                let exact = required_u64(run, "/failure/limitSelection/exactDimensionValue")?;
                let selected = required_u64(run, "/failure/limitSelection/selectedLimitValue")?;
                independent_complete_private_limit_parameters(
                    Some((dimension, selected)),
                    &[
                        ("exact-dimension-value", exact),
                        ("selected-limit-value", selected),
                    ],
                )
            } else {
                match case_id {
                    "limit/source-byte-count/plus-one" => {
                        let exact = plan.counts.source_byte_count;
                        independent_complete_private_limit_parameters(
                            Some((LimitDimensionId::SourceByteCount, exact.saturating_sub(1))),
                            &[
                                ("exact-dimension-value", exact),
                                ("selected-limit-value", exact.saturating_sub(1)),
                            ],
                        )
                    }
                    "semantic/missing-reference-per-unit" | "semantic/duplicate-owner-per-unit" => {
                        independent_complete_private_limit_parameters(None, &[])
                    }
                    "diagnostic/cap-plus-one" => independent_complete_private_limit_parameters(
                        Some((LimitDimensionId::DiagnosticCount, u64::from(n))),
                        &[("maximum-diagnostics", u64::from(n))],
                    ),
                    other => {
                        return Err(EvidenceError::FailureInputRecomputation {
                            detail: format!("运行 {run_id} 使用未知失败用例 {other}"),
                        });
                    }
                }
            };
        let expected = independent_failure_input_digest(
            &trusted.descriptor.workload_manifest.sha256,
            workload_id,
            required_u64(run, "/workload/revision")?,
            graph_profile,
            required_string(run, "/workload/stringProfile")?,
            required_u64(run, "/workload/generatorVersion")?,
            u64::from(n),
            observed_u64(run, "/workload/b")?,
            required_string(run, "/workload/scaleRole")?,
            case_id,
            input_variant_id,
            &plan.counts,
            value_basis,
            &basis_run_ids,
            &parameters,
        );
        let actual = required_string(run, "/failure/inputDigest")?;
        if actual != expected {
            return Err(EvidenceError::FailureInputRecomputation {
                detail: format!("运行 {run_id} 的失败输入摘要与独立重算不一致"),
            });
        }
        checked += 1;
    }
    Ok(checked)
}

#[allow(clippy::too_many_arguments)]
fn independent_failure_input_digest(
    workload_manifest_sha256: &str,
    workload_id: ScalableWorkloadId,
    workload_revision: u64,
    graph_profile: GraphProfileId,
    string_profile: &str,
    generator_version: u64,
    n: u64,
    b: u64,
    scale_role: &str,
    case_id: &str,
    input_variant_id: &str,
    counts: &crate::IdentityAggregateCounts,
    value_basis: &str,
    basis_run_ids: &[String],
    parameters: &[(String, u64)],
) -> String {
    let mut parameters = parameters.to_vec();
    parameters.sort_unstable_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    let mut basis_run_ids = basis_run_ids.to_vec();
    basis_run_ids.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    let mut bytes = b"LANEFLOW-COMPILER-CALIBRATION-INPUT-V1\0".to_vec();
    independent_failure_input_string(&mut bytes, workload_manifest_sha256);
    independent_failure_input_string(&mut bytes, workload_id.as_str());
    bytes.extend_from_slice(&workload_revision.to_le_bytes());
    independent_failure_input_string(&mut bytes, graph_profile.as_str());
    independent_failure_input_string(&mut bytes, string_profile);
    bytes.extend_from_slice(&generator_version.to_le_bytes());
    bytes.extend_from_slice(&n.to_le_bytes());
    bytes.extend_from_slice(&b.to_le_bytes());
    independent_failure_input_string(&mut bytes, scale_role);
    independent_failure_input_string(&mut bytes, case_id);
    independent_failure_input_string(&mut bytes, input_variant_id);
    independent_failure_input_string(
        &mut bytes,
        "trusted-manifest+full-scale-identity+canonical-counts+input-variant-v1",
    );
    independent_failure_input_counts(&mut bytes, counts);
    independent_failure_input_string(&mut bytes, value_basis);
    bytes.extend_from_slice(
        &u32::try_from(basis_run_ids.len())
            .expect("失败输入摘要基准运行数量适合 u32")
            .to_le_bytes(),
    );
    for run_id in basis_run_ids {
        independent_failure_input_string(&mut bytes, &run_id);
    }
    bytes.extend_from_slice(
        &u32::try_from(parameters.len())
            .expect("失败输入摘要参数数量适合 u32")
            .to_le_bytes(),
    );
    for (name, value) in parameters {
        independent_failure_input_string(&mut bytes, &name);
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    sha256_hex(&bytes)
}

fn independent_failure_input_counts(bytes: &mut Vec<u8>, counts: &crate::IdentityAggregateCounts) {
    let fields = [
        ("module-count", counts.module_count),
        ("import-edge-count", counts.import_edge_count),
        (
            "cross-module-reference-count",
            counts.cross_module_reference_count,
        ),
        ("maximum-import-depth", counts.maximum_import_depth),
        ("source-document-count", counts.source_document_count),
        ("source-byte-count", counts.source_byte_count),
        (
            "identity-declaration-count",
            counts.identity_declaration_count,
        ),
        ("source-declaration-count", counts.source_declaration_count),
        ("source-span-count", counts.source_span_count),
        (
            "identity-field-occurrence-count",
            counts.identity_field_occurrence_count,
        ),
        (
            "profiled-key-occurrence-count",
            counts.profiled_key_occurrence_count,
        ),
        ("source-reference-count", counts.source_reference_count),
        ("source-relation-count", counts.source_relation_count),
        ("source-geometry-count", counts.source_geometry_count),
        ("symbol-count", counts.symbol_count),
        ("string-item-count", counts.string_item_count),
        ("maximum-string-bytes", counts.maximum_string_bytes),
        ("total-string-bytes", counts.total_string_bytes),
        ("diagnostic-count", counts.diagnostic_count),
        ("semantic-output-record", counts.semantic_output_record),
        (
            "semantic-payload-byte-count",
            counts.semantic_payload_byte_count,
        ),
        ("logical-byte-count", counts.logical_byte_count),
        ("output-byte-count", counts.output_byte_count),
    ];
    bytes.extend_from_slice(
        &u32::try_from(fields.len())
            .expect("独立规范计数字段数量适合 u32")
            .to_le_bytes(),
    );
    for (name, value) in fields {
        independent_failure_input_string(bytes, name);
        bytes.extend_from_slice(&value.to_le_bytes());
    }
}

fn independent_complete_private_limit_parameters(
    selected_limit: Option<(LimitDimensionId, u64)>,
    additional_parameters: &[(&str, u64)],
) -> Vec<(String, u64)> {
    let mut parameters = Vec::with_capacity(
        LimitDimensionId::ALL
            .len()
            .checked_add(additional_parameters.len())
            .expect("独立失败输入摘要参数数量不会溢出"),
    );
    for dimension in LimitDimensionId::ALL {
        let value = selected_limit
            .filter(|(selected, _)| *selected == dimension)
            .map_or(u64::MAX, |(_, value)| value);
        parameters.push((format!("private-limit/{}", dimension.as_str()), value));
    }
    parameters.extend(
        additional_parameters
            .iter()
            .map(|(name, value)| ((*name).to_owned(), *value)),
    );
    parameters
}

fn independent_failure_input_string(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(
        &u32::try_from(value.len())
            .expect("失败输入摘要字段长度适合 u32")
            .to_le_bytes(),
    );
    bytes.extend_from_slice(value.as_bytes());
}

fn independent_limit_diagnostic_digest(
    trusted: &TrustedContract,
    run: &Value,
) -> Result<String, EvidenceError> {
    let dimension = parse_limit_dimension(required_string(run, "/failure/dimensionId")?)?;
    let observed = if let Ok(value) =
        required_u64(run, "/failure/limitSelection/exactDimensionValue")
    {
        value
    } else {
        let workload_id = required_string(run, "/workload/id")?
            .parse::<ScalableWorkloadId>()
            .map_err(|error| EvidenceError::DiagnosticRecomputation {
                detail: error.to_string(),
            })?;
        let graph_profile = parse_graph_profile(required_string(run, "/workload/graphProfile")?)?;
        let n = u32::try_from(required_u64(run, "/workload/n")?).map_err(|_| {
            EvidenceError::DiagnosticRecomputation {
                detail: "限制诊断 N 超出 u32".to_owned(),
            }
        })?;
        let plan = ScalableStagePlanFactory::from_trusted_contract(trusted)
            .map_err(|error| EvidenceError::DiagnosticRecomputation {
                detail: error.to_string(),
            })?
            .plan(workload_id, graph_profile, n)
            .map_err(|error| EvidenceError::DiagnosticRecomputation {
                detail: error.to_string(),
            })?;
        match dimension {
            LimitDimensionId::SourceByteCount => plan.counts.source_byte_count,
            _ => {
                return Err(EvidenceError::DiagnosticRecomputation {
                    detail: format!(
                        "运行 {} 缺少限制选择且维度不是清理实验支持的 source-byte-count",
                        required_string(run, "/runId")?
                    ),
                });
            }
        }
    };
    let selected = required_u64(run, "/failure/limitSelection/selectedLimitValue")
        .unwrap_or_else(|_| observed.saturating_sub(1));
    let mut payload = Vec::with_capacity(17);
    payload.push(dimension.one_based_code_u8());
    payload.extend_from_slice(&selected.to_le_bytes());
    payload.extend_from_slice(&observed.to_le_bytes());
    independent_diagnostic_digest(
        trusted,
        vec![IndependentDiagnosticRecord {
            code: LIMIT_EXCEEDED_ERROR_CODE.to_owned(),
            source_document_key: String::new(),
            start_line: 0,
            start_column: 0,
            end_line: 0,
            end_column: 0,
            severity: 1,
            typed_payload: payload,
        }],
    )
}

fn independent_unknown_reference_diagnostic_digest(
    trusted: &TrustedContract,
    run: &Value,
) -> Result<String, EvidenceError> {
    let workload_id = required_string(run, "/workload/id")?;
    if workload_id != ScalableWorkloadId::Corridor.as_str() {
        return Err(EvidenceError::DiagnosticRecomputation {
            detail: format!("未知引用诊断使用非走廊工作负载 {workload_id}"),
        });
    }
    let graph_profile = parse_graph_profile(required_string(run, "/workload/graphProfile")?)?;
    let n = u32::try_from(required_u64(run, "/workload/n")?).map_err(|_| {
        EvidenceError::DiagnosticRecomputation {
            detail: "未知引用诊断 N 超出 u32".to_owned(),
        }
    })?;
    let is_constant_hash =
        run.get("correctnessQualification").is_some() && !run["correctnessQualification"].is_null();
    let (variant, retained_count, diagnostics_truncated) = if is_constant_hash {
        (
            required_string(run, "/correctnessQualification/inputVariantId")?,
            required_u64(run, "/correctnessQualification/actualDiagnosticCount")?,
            required_bool(run, "/correctnessQualification/actualDiagnosticsTruncated")?,
        )
    } else {
        (
            required_string(run, "/failure/inputVariantId")?,
            required_u64(run, "/failure/diagnosticCount")?,
            required_bool(run, "/failure/diagnosticsTruncated")?,
        )
    };
    let contract =
        CorridorContract::from_manifest(&trusted.workload_manifest).map_err(|error| {
            EvidenceError::DiagnosticRecomputation {
                detail: error.to_string(),
            }
        })?;
    let template = contract
        .load_template(&repository_root())
        .map_err(|error| EvidenceError::DiagnosticRecomputation {
            detail: error.to_string(),
        })?;
    let oracle = crate::corridor_oracle::build_template_oracle_records(
        &trusted.workload_manifest,
        workload_id,
        &template,
        graph_profile,
        n,
    )
    .map_err(|error| EvidenceError::DiagnosticRecomputation {
        detail: error.to_string(),
    })?;
    let mut selected = match variant {
        "corridor-missing-reference-per-unit-v1" | "constant-hash-missing-reference-v1" => oracle
            .route_occurrences
            .iter()
            .copied()
            .filter(|route| route.route_ordinal_within_unit == 0)
            .collect::<Vec<_>>(),
        "corridor-diagnostic-cap-plus-one-v1" => oracle
            .route_occurrences
            .iter()
            .copied()
            .take(usize::try_from(u64::from(n) + 1).map_err(|_| {
                EvidenceError::DiagnosticRecomputation {
                    detail: "诊断 cap 候选数量超出 usize".to_owned(),
                }
            })?)
            .collect::<Vec<_>>(),
        other => {
            return Err(EvidenceError::DiagnosticRecomputation {
                detail: format!("未知的引用失败输入变体 {other}"),
            });
        }
    };
    let expected_full_count = match variant {
        "corridor-missing-reference-per-unit-v1" | "constant-hash-missing-reference-v1" => {
            u64::from(n)
        }
        "corridor-diagnostic-cap-plus-one-v1" => u64::from(n) + 1,
        _ => unreachable!("variant validated above"),
    };
    if u64::try_from(selected.len()).ok() != Some(expected_full_count) {
        return Err(EvidenceError::DiagnosticRecomputation {
            detail: format!("输入变体 {variant} 的完整诊断候选数量不一致"),
        });
    }

    let stage_inputs = template.stage_input_counts();
    let source_reference_count = stage_inputs
        .get("sourceReferenceCount")
        .copied()
        .ok_or_else(|| EvidenceError::DiagnosticRecomputation {
            detail: "走廊模板缺少 sourceReferenceCount".to_owned(),
        })?;
    let source_declaration_count = u64::try_from(template.entities.len()).map_err(|_| {
        EvidenceError::DiagnosticRecomputation {
            detail: "走廊声明数量超出 u64".to_owned(),
        }
    })?;
    let mut diagnostics = Vec::with_capacity(selected.len());
    for route in selected.drain(..) {
        let source_document_key = format!(
            "source/{}/unit/{:08x}.lfsynthetic",
            graph_profile.as_str(),
            route.unit
        );
        let start_line = source_declaration_count
            .checked_add(source_reference_count)
            .and_then(|value| value.checked_add(u64::from(route.relation_sequence_ordinal)))
            .and_then(|value| value.checked_add(1))
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| EvidenceError::DiagnosticRecomputation {
                detail: "未知引用诊断来源行溢出".to_owned(),
            })?;
        let unknown_local = 0x8000_0000_u32
            .checked_add(route.route_ordinal_within_unit)
            .ok_or_else(|| EvidenceError::DiagnosticRecomputation {
                detail: "未知 LaneEdge 局部序号溢出".to_owned(),
            })?;
        let unknown_key = format!("04/{:08x}/{unknown_local:08x}", route.unit);
        let mut payload = Vec::with_capacity(34);
        payload.extend_from_slice(&4_u16.to_le_bytes());
        payload.extend_from_slice(&route.reference_ordinal.to_le_bytes());
        payload.extend_from_slice(&20_u32.to_le_bytes());
        payload.extend_from_slice(unknown_key.as_bytes());
        diagnostics.push(IndependentDiagnosticRecord {
            code: UNKNOWN_REFERENCE_ERROR_CODE.to_owned(),
            source_document_key,
            start_line,
            start_column: 1,
            end_line: start_line,
            end_column: 18,
            severity: 1,
            typed_payload: payload,
        });
    }
    diagnostics.sort_unstable_by(|left, right| left.canonical_key().cmp(&right.canonical_key()));
    diagnostics.truncate(usize::try_from(retained_count).map_err(|_| {
        EvidenceError::DiagnosticRecomputation {
            detail: "保留诊断数量超出 usize".to_owned(),
        }
    })?);
    let expected_truncated = expected_full_count > retained_count;
    if diagnostics_truncated != expected_truncated {
        return Err(EvidenceError::DiagnosticRecomputation {
            detail: format!(
                "运行 {} 的诊断截断标记不一致",
                required_string(run, "/runId")?
            ),
        });
    }
    independent_diagnostic_digest(trusted, diagnostics)
}

fn independent_duplicate_owner_diagnostic_digest(
    trusted: &TrustedContract,
    run: &Value,
) -> Result<String, EvidenceError> {
    if required_string(run, "/workload/id")? != ScalableWorkloadId::Corridor.as_str()
        || required_string(run, "/failure/inputVariantId")?
            != "corridor-duplicate-owner-per-unit-v1"
    {
        return Err(EvidenceError::DiagnosticRecomputation {
            detail: "重复所有者诊断没有绑定冻结走廊变体".to_owned(),
        });
    }
    let graph_profile = parse_graph_profile(required_string(run, "/workload/graphProfile")?)?;
    let n = u32::try_from(required_u64(run, "/workload/n")?).map_err(|_| {
        EvidenceError::DiagnosticRecomputation {
            detail: "重复所有者诊断 N 超出 u32".to_owned(),
        }
    })?;
    if required_u64(run, "/failure/diagnosticCount")? != u64::from(n)
        || required_bool(run, "/failure/diagnosticsTruncated")?
    {
        return Err(EvidenceError::DiagnosticRecomputation {
            detail: "重复所有者诊断数量或截断状态不一致".to_owned(),
        });
    }
    let contract =
        CorridorContract::from_manifest(&trusted.workload_manifest).map_err(|error| {
            EvidenceError::DiagnosticRecomputation {
                detail: error.to_string(),
            }
        })?;
    let template = contract
        .load_template(&repository_root())
        .map_err(|error| EvidenceError::DiagnosticRecomputation {
            detail: error.to_string(),
        })?;
    let child = template
        .entities
        .iter()
        .find(|entity| entity.reference.kind == 17)
        .ok_or_else(|| EvidenceError::DiagnosticRecomputation {
            detail: "走廊模板缺少 FacilityBand".to_owned(),
        })?;
    let first_owner = child
        .identity_references
        .values()
        .copied()
        .find(|target| target.kind == 1)
        .ok_or_else(|| EvidenceError::DiagnosticRecomputation {
            detail: "FacilityBand 缺少首个 RoadCorridor 所有者".to_owned(),
        })?;
    let second_owner = template
        .entities
        .iter()
        .filter(|entity| entity.reference.kind == 1)
        .nth(1)
        .map(|entity| entity.reference)
        .ok_or_else(|| EvidenceError::DiagnosticRecomputation {
            detail: "走廊模板缺少第二个 RoadCorridor".to_owned(),
        })?;
    if first_owner == second_owner {
        return Err(EvidenceError::DiagnosticRecomputation {
            detail: "重复所有者变体的两个 RoadCorridor 不独立".to_owned(),
        });
    }
    let relation_sequence_ordinal = template
        .relations
        .iter()
        .position(|relation| {
            matches!(
                relation,
                crate::corridor::TemplateRelation::Owner { child: relation_child, parent }
                    if *relation_child == child.reference && *parent == first_owner
            )
        })
        .and_then(|ordinal| u32::try_from(ordinal).ok())
        .ok_or_else(|| EvidenceError::DiagnosticRecomputation {
            detail: "重复所有者变体缺少原所有者关系来源".to_owned(),
        })?;
    let oracle = crate::corridor_oracle::build_template_oracle_records(
        &trusted.workload_manifest,
        ScalableWorkloadId::Corridor.as_str(),
        &template,
        graph_profile,
        n,
    )
    .map_err(|error| EvidenceError::DiagnosticRecomputation {
        detail: error.to_string(),
    })?;
    let stage_inputs = template.stage_input_counts();
    let source_reference_count = stage_inputs
        .get("sourceReferenceCount")
        .copied()
        .ok_or_else(|| EvidenceError::DiagnosticRecomputation {
            detail: "走廊模板缺少 sourceReferenceCount".to_owned(),
        })?;
    let source_declaration_count = u64::try_from(template.entities.len()).map_err(|_| {
        EvidenceError::DiagnosticRecomputation {
            detail: "走廊声明数量超出 u64".to_owned(),
        }
    })?;
    let start_line = source_declaration_count
        .checked_add(source_reference_count)
        .and_then(|value| value.checked_add(u64::from(relation_sequence_ordinal)))
        .and_then(|value| value.checked_add(1))
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| EvidenceError::DiagnosticRecomputation {
            detail: "重复所有者诊断来源行溢出".to_owned(),
        })?;
    let declaration_id = |unit: u32, entity| {
        oracle
            .declarations
            .iter()
            .find(|declaration| declaration.unit == unit && declaration.entity == entity)
            .map(|declaration| declaration.stable_id)
            .ok_or_else(|| EvidenceError::DiagnosticRecomputation {
                detail: format!(
                    "重复所有者诊断缺少 unit={unit} kind={} local={} 声明",
                    entity.kind, entity.local
                ),
            })
    };
    let mut diagnostics = Vec::with_capacity(usize::try_from(n).map_err(|_| {
        EvidenceError::DiagnosticRecomputation {
            detail: "重复所有者诊断数量超出 usize".to_owned(),
        }
    })?);
    for unit in 0..n {
        let mut payload = Vec::with_capacity(50);
        payload.extend_from_slice(&child.reference.kind.to_le_bytes());
        payload.extend_from_slice(&declaration_id(unit, child.reference)?);
        payload.extend_from_slice(&declaration_id(unit, first_owner)?);
        payload.extend_from_slice(&declaration_id(unit, second_owner)?);
        diagnostics.push(IndependentDiagnosticRecord {
            code: DUPLICATE_OWNER_ERROR_CODE.to_owned(),
            source_document_key: format!(
                "source/{}/unit/{unit:08x}.lfsynthetic",
                graph_profile.as_str()
            ),
            start_line,
            start_column: 1,
            end_line: start_line,
            end_column: 18,
            severity: 1,
            typed_payload: payload,
        });
    }
    independent_diagnostic_digest(trusted, diagnostics)
}

fn independent_diagnostic_digest(
    trusted: &TrustedContract,
    mut diagnostics: Vec<IndependentDiagnosticRecord>,
) -> Result<String, EvidenceError> {
    diagnostics.sort_unstable_by(|left, right| left.canonical_key().cmp(&right.canonical_key()));
    let manifest_stream = required_object(&trusted.workload_manifest, "/diagnosticStream")?;
    let domain = required_string(manifest_stream, "/domainUtf8NulTerminated")?;
    let version = u32::try_from(required_u64(
        &trusted.workload_manifest,
        "/diagnosticStreamVersion",
    )?)
    .map_err(|_| EvidenceError::DiagnosticRecomputation {
        detail: "诊断流版本超出 u32".to_owned(),
    })?;
    let severity =
        u8::try_from(required_u64(manifest_stream, "/severityCodeU8/error")?).map_err(|_| {
            EvidenceError::DiagnosticRecomputation {
                detail: "诊断严重性代码超出 u8".to_owned(),
            }
        })?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(domain.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&version.to_le_bytes());
    bytes.extend_from_slice(
        &u64::try_from(diagnostics.len())
            .map_err(|_| EvidenceError::DiagnosticRecomputation {
                detail: "诊断数量超出 u64".to_owned(),
            })?
            .to_le_bytes(),
    );
    for diagnostic in diagnostics {
        if diagnostic.severity != severity {
            return Err(EvidenceError::DiagnosticRecomputation {
                detail: "诊断严重性与受信任清单不一致".to_owned(),
            });
        }
        append_independent_length_prefixed(&mut bytes, diagnostic.code.as_bytes())?;
        bytes.push(diagnostic.severity);
        append_independent_length_prefixed(&mut bytes, diagnostic.source_document_key.as_bytes())?;
        bytes.extend_from_slice(&diagnostic.start_line.to_le_bytes());
        bytes.extend_from_slice(&diagnostic.start_column.to_le_bytes());
        bytes.extend_from_slice(&diagnostic.end_line.to_le_bytes());
        bytes.extend_from_slice(&diagnostic.end_column.to_le_bytes());
        append_independent_length_prefixed(&mut bytes, &diagnostic.typed_payload)?;
    }
    Ok(sha256_hex(&bytes))
}

fn append_independent_length_prefixed(
    output: &mut Vec<u8>,
    bytes: &[u8],
) -> Result<(), EvidenceError> {
    output.extend_from_slice(
        &u32::try_from(bytes.len())
            .map_err(|_| EvidenceError::DiagnosticRecomputation {
                detail: "诊断字段长度超出 u32".to_owned(),
            })?
            .to_le_bytes(),
    );
    output.extend_from_slice(bytes);
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct LimitEvidenceScale {
    workload_id: ScalableWorkloadId,
    graph_profile: GraphProfileId,
    scale_role: String,
    n: u32,
    b: u64,
}

type SelectedScaleMap = BTreeMap<(ScalableWorkloadId, GraphProfileId, String), LimitEvidenceScale>;

#[derive(Default)]
struct LimitPairSides<'a> {
    at_bound: Option<(&'a str, &'a Value)>,
    plus_one: Option<(&'a str, &'a Value)>,
}

fn verify_limit_qualifications(
    trusted: &TrustedContract,
    document: &Value,
    runs: &BTreeMap<String, &Value>,
    selected_scales: &SelectedScaleMap,
    referenced_run_ids: &mut BTreeSet<String>,
) -> Result<(usize, usize, usize), EvidenceError> {
    let planner = LimitQualificationPlanner::from_trusted_contract(trusted).map_err(|error| {
        EvidenceError::LimitRecomputation {
            detail: error.to_string(),
        }
    })?;
    let mut baselines = BTreeMap::<LimitEvidenceScale, Vec<(u64, &str, &Value)>>::new();
    let mut pairs = BTreeMap::<(LimitEvidenceScale, LimitDimensionId), LimitPairSides<'_>>::new();
    let mut duplicate_owners = BTreeMap::<LimitEvidenceScale, (&str, &Value)>::new();

    for (run_id, run) in runs {
        if required_string(run, "/sampleKind")? == "limit-baseline" {
            let scale = limit_evidence_scale(run)?;
            if scale.workload_id != ScalableWorkloadId::Identity
                || required_string(run, "/candidate/id")?
                    != "baseline-std-randomstate-stable-vec-v1"
                || required_string(run, "/candidate/keyDomain")? != "full-pipeline-baseline"
                || required_string(run, "/process/binaryId")? != ATTRIBUTION_BINARY_ID
                || required_string(run, "/workload/stringProfile")? != "short-unique-v1"
                || required_string(run, "/workload/caseId")? != "not-applicable"
                || required_string(run, "/cleanup/phase")? != "not-applicable"
                || required_string(run, "/limitBaseline/measurementId")?
                    != "compiler-controlled-live-byte-baseline-v1"
                || required_string(run, "/limitBaseline/dimensionId")?
                    != "compiler-controlled-live-byte-count"
                || required_string(run, "/limitBaseline/privateLimitMode")?
                    != "operational-hard-ceiling-only"
            {
                return Err(EvidenceError::LimitRecomputation {
                    detail: format!("limit baseline {run_id} 的身份或角色不满足冻结契约"),
                });
            }
            if required_string(run, "/status")? != "valid" {
                continue;
            }
            if required_string(run, "/process/exitKind")? != "success" {
                return Err(EvidenceError::LimitRecomputation {
                    detail: format!("有效 limit baseline {run_id} 没有成功退出"),
                });
            }
            let replica = required_u64(run, "/limitBaseline/replicaIndex")?;
            let peak = observed_u64(run, "/metrics/peakLiveRequestedBytes")?;
            if replica > 1
                || peak == 0
                || nullable_string(run, "/compilerInstanceId")?.is_none()
                || nullable_u64(run, "/process/childPid")?.is_none()
            {
                return Err(EvidenceError::LimitRecomputation {
                    detail: format!("limit baseline {run_id} 缺少独立进程、实例或正峰值"),
                });
            }
            baselines
                .entry(scale)
                .or_default()
                .push((replica, run_id.as_str(), run));
            continue;
        }

        if required_string(run, "/sampleKind")? != "failure"
            || required_string(run, "/cleanup/phase")? != "not-applicable"
        {
            continue;
        }
        if required_string(run, "/status")? != "valid" {
            continue;
        }
        let case_id = required_string(run, "/failure/caseId")?;
        if case_id == "semantic/duplicate-owner-per-unit" {
            let scale = limit_evidence_scale(run)?;
            if scale.workload_id != ScalableWorkloadId::Corridor
                || required_string(run, "/failure/dimensionId")? != "not-applicable"
                || required_string(run, "/failure/inputVariantId")?
                    != "corridor-duplicate-owner-per-unit-v1"
                || required_string(run, "/candidate/id")?
                    != "baseline-std-randomstate-stable-vec-v1"
                || required_string(run, "/candidate/keyDomain")? != "full-pipeline-baseline"
                || required_string(run, "/process/binaryId")? != TIMING_BINARY_ID
                || required_string(run, "/process/exitKind")? != "success"
                || required_string(run, "/failure/expectedOutcome")? != "compiler-error"
                || required_string(run, "/failure/actualOutcome")? != "compiler-error"
                || nullable_string(run, "/failure/stableCompilerErrorCode")?
                    != Some(DUPLICATE_OWNER_ERROR_CODE)
                || required_u64(run, "/failure/diagnosticCount")? != u64::from(scale.n)
                || required_bool(run, "/failure/diagnosticsTruncated")?
                || required_u64(run, "/failure/partialOutputRecordCount")? != 0
                || nullable_string(run, "/metrics/semanticDigest")?.is_some()
                || observed_u64(run, "/metrics/liveRequestedBytes")? != 0
            {
                return Err(EvidenceError::LimitRecomputation {
                    detail: format!("重复所有者资格 {run_id} 的身份或失败观察不一致"),
                });
            }
            if duplicate_owners
                .insert(scale, (run_id.as_str(), run))
                .is_some()
            {
                return Err(EvidenceError::LimitRecomputation {
                    detail: format!("重复所有者资格 {run_id} 的分层身份重复"),
                });
            }
            continue;
        }
        let Some(remainder) = case_id.strip_prefix("limit/") else {
            continue;
        };
        let (dimension_name, side) =
            remainder
                .rsplit_once('/')
                .ok_or_else(|| EvidenceError::LimitRecomputation {
                    detail: format!("限制运行 {run_id} 的 caseId 无法拆分"),
                })?;
        let dimension_id = parse_limit_dimension(dimension_name)?;
        let scale = limit_evidence_scale(run)?;
        let binding = planner
            .bindings()
            .iter()
            .find(|binding| binding.dimension_id == dimension_id)
            .ok_or_else(|| EvidenceError::LimitRecomputation {
                detail: format!("限制维度 {dimension_name} 没有清单绑定"),
            })?;
        if binding.workload_id != scale.workload_id
            || required_string(run, "/failure/dimensionId")? != dimension_name
            || required_string(run, "/failure/inputVariantId")? != binding.input_variant_id
            || required_string(run, "/candidate/id")? != "baseline-std-randomstate-stable-vec-v1"
            || required_string(run, "/candidate/keyDomain")? != "full-pipeline-baseline"
            || required_string(run, "/process/exitKind")? != "success"
        {
            return Err(EvidenceError::LimitRecomputation {
                detail: format!("限制运行 {run_id} 的维度、工作负载、候选或结果身份不一致"),
            });
        }
        let entry = pairs.entry((scale, dimension_id)).or_default();
        let target = match side {
            "at-bound" => &mut entry.at_bound,
            "plus-one" => &mut entry.plus_one,
            _ => {
                return Err(EvidenceError::LimitRecomputation {
                    detail: format!("限制运行 {run_id} 使用未知配对侧 {side}"),
                });
            }
        };
        if target.replace((run_id.as_str(), run)).is_some() {
            return Err(EvidenceError::LimitRecomputation {
                detail: format!("限制配对 {case_id} 在同一分层重复"),
            });
        }
    }

    let formal_available = required_string(document, "/derived/formalStudyDisposition")?
        == "formal-analysis-available";
    if !formal_available {
        if !baselines.is_empty() || !pairs.is_empty() || !duplicate_owners.is_empty() {
            return Err(EvidenceError::LimitRecomputation {
                detail: "没有正式分析时不得形成限制资格证据".to_owned(),
            });
        }
        return Ok((0, 0, 0));
    }

    if selected_scales.len() != 18 {
        return Err(EvidenceError::LimitRecomputation {
            detail: format!(
                "正式阶梯必须独立派生十八个限制规模身份，实际 {}",
                selected_scales.len()
            ),
        });
    }
    let expected_baseline_scales = selected_scales
        .values()
        .filter(|scale| scale.workload_id == ScalableWorkloadId::Identity)
        .cloned()
        .collect::<BTreeSet<_>>();
    if baselines.keys().cloned().collect::<BTreeSet<_>>() != expected_baseline_scales {
        return Err(EvidenceError::LimitRecomputation {
            detail: "双归因基线没有精确覆盖六个 LF-COMP-ID 校准/压力分层".to_owned(),
        });
    }

    let mut verified_baselines = BTreeMap::<LimitEvidenceScale, LiveByteBaseline>::new();
    for (scale, replicas) in &baselines {
        let baseline = verify_live_byte_baseline_group(scale, replicas, referenced_run_ids)?;
        verified_baselines.insert(scale.clone(), baseline);
    }

    let expected_pair_keys = selected_scales
        .values()
        .flat_map(|scale| {
            planner
                .bindings()
                .iter()
                .filter(move |binding| binding.workload_id == scale.workload_id)
                .map(move |binding| (scale.clone(), binding.dimension_id))
        })
        .collect::<BTreeSet<_>>();
    if pairs.keys().cloned().collect::<BTreeSet<_>>() != expected_pair_keys {
        return Err(EvidenceError::LimitRecomputation {
            detail: format!(
                "限制配对覆盖不完整：期望 {}，实际 {}",
                expected_pair_keys.len(),
                pairs.len()
            ),
        });
    }

    for ((scale, dimension_id), sides) in &pairs {
        let baseline = if *dimension_id == LimitDimensionId::CompilerControlledLiveByteCount {
            Some(
                verified_baselines
                    .get(scale)
                    .ok_or_else(|| EvidenceError::LimitRecomputation {
                        detail: format!("{scale:?} 缺少存续字节预扫描"),
                    })?
                    .clone(),
            )
        } else {
            None
        };
        let plan = planner
            .plan_pair(*dimension_id, scale.graph_profile, scale.n, baseline)
            .map_err(|error| EvidenceError::LimitRecomputation {
                detail: error.to_string(),
            })?;
        let (at_bound_id, at_bound) =
            sides
                .at_bound
                .ok_or_else(|| EvidenceError::LimitRecomputation {
                    detail: format!("{scale:?}/{dimension_id:?} 缺少 at-bound"),
                })?;
        let (plus_one_id, plus_one) =
            sides
                .plus_one
                .ok_or_else(|| EvidenceError::LimitRecomputation {
                    detail: format!("{scale:?}/{dimension_id:?} 缺少 plus-one"),
                })?;
        verify_limit_pair_run(at_bound_id, at_bound, &plan, true)?;
        verify_limit_pair_run(plus_one_id, plus_one, &plan, false)?;
        referenced_run_ids.insert(at_bound_id.to_owned());
        referenced_run_ids.insert(plus_one_id.to_owned());
    }
    let expected_duplicate_owner_scales = selected_scales
        .values()
        .filter(|scale| scale.workload_id == ScalableWorkloadId::Corridor)
        .cloned()
        .collect::<BTreeSet<_>>();
    if duplicate_owners.keys().cloned().collect::<BTreeSet<_>>() != expected_duplicate_owner_scales
    {
        return Err(EvidenceError::LimitRecomputation {
            detail: "重复所有者资格没有精确覆盖走廊的三个模块图 × 校准/压力规模".to_owned(),
        });
    }
    for (run_id, _) in duplicate_owners.values() {
        referenced_run_ids.insert((*run_id).to_owned());
    }
    Ok((
        pairs.len(),
        baselines.values().map(Vec::len).sum(),
        duplicate_owners.len(),
    ))
}

fn verify_live_byte_baseline_group(
    scale: &LimitEvidenceScale,
    replicas: &[(u64, &str, &Value)],
    referenced_run_ids: &mut BTreeSet<String>,
) -> Result<LiveByteBaseline, EvidenceError> {
    if replicas.len() != 2 {
        return Err(EvidenceError::LimitRecomputation {
            detail: format!("{scale:?} 的存续字节基线不是两个副本"),
        });
    }
    let mut replicas = replicas.to_vec();
    replicas.sort_by_key(|(replica, _, _)| *replica);
    if replicas[0].0 != 0 || replicas[1].0 != 1 {
        return Err(EvidenceError::LimitRecomputation {
            detail: format!("{scale:?} 的基线副本索引不是 0/1"),
        });
    }
    let left_instance = nullable_string(replicas[0].2, "/compilerInstanceId")?;
    let right_instance = nullable_string(replicas[1].2, "/compilerInstanceId")?;
    let left_pid = nullable_u64(replicas[0].2, "/process/childPid")?;
    let right_pid = nullable_u64(replicas[1].2, "/process/childPid")?;
    let left_peak = observed_u64(replicas[0].2, "/metrics/peakLiveRequestedBytes")?;
    let right_peak = observed_u64(replicas[1].2, "/metrics/peakLiveRequestedBytes")?;
    if left_instance == right_instance || left_pid == right_pid || left_peak != right_peak {
        return Err(EvidenceError::LimitRecomputation {
            detail: format!("{scale:?} 的两个 attribution 副本不独立或峰值不一致"),
        });
    }
    for (_, run_id, _) in &replicas {
        referenced_run_ids.insert((*run_id).to_owned());
    }
    Ok(LiveByteBaseline {
        replicas: [
            LiveByteBaselineReplica {
                run_id: replicas[0].1.to_owned(),
                workload_id: scale.workload_id,
                graph_profile: scale.graph_profile,
                n: scale.n,
                peak_live_requested_bytes: left_peak,
            },
            LiveByteBaselineReplica {
                run_id: replicas[1].1.to_owned(),
                workload_id: scale.workload_id,
                graph_profile: scale.graph_profile,
                n: scale.n,
                peak_live_requested_bytes: right_peak,
            },
        ],
    })
}

fn verify_limit_pair_run(
    run_id: &str,
    run: &Value,
    plan: &crate::LimitPairPlan,
    at_bound: bool,
) -> Result<(), EvidenceError> {
    let expected_binary_id = if plan.binding.pair_mode == LimitPairMode::BaselineLiveBytePrescanV1 {
        ATTRIBUTION_BINARY_ID
    } else {
        TIMING_BINARY_ID
    };
    expect_string(run, "/process/binaryId", expected_binary_id)?;
    let expected_side = if at_bound { "at-bound" } else { "plus-one" };
    expect_string(
        run,
        "/failure/caseId",
        &format!(
            "limit/{}/{expected_side}",
            plan.binding.dimension_id.as_str()
        ),
    )?;
    expect_u64(
        run,
        "/failure/limitSelection/exactDimensionValue",
        plan.exact_dimension_value,
    )?;
    expect_u64(
        run,
        "/failure/limitSelection/selectedLimitValue",
        if at_bound {
            plan.at_bound_limit_value
        } else {
            plan.plus_one_limit_value
        },
    )?;
    let expected_basis = match plan.binding.pair_mode {
        LimitPairMode::SuccessAtBound => "canonical-level-exact-value",
        LimitPairMode::DiagnosticCapOnSemanticFailure => "diagnostic-input-count",
        LimitPairMode::BaselineLiveBytePrescanV1 => "baseline-live-byte-prescan-v1",
    };
    expect_string(run, "/failure/limitSelection/valueBasis", expected_basis)?;
    let actual_basis_ids = required_array(run, "/failure/limitSelection/basisRunIds")?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| EvidenceError::LimitRecomputation {
                    detail: format!("限制运行 {run_id} 含非字符串 basisRunId"),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if actual_basis_ids != plan.basis_run_ids {
        return Err(EvidenceError::LimitRecomputation {
            detail: format!("限制运行 {run_id} 的基线运行引用不一致"),
        });
    }

    let n = required_u64(run, "/workload/n")?;
    let (expected_outcome, error_code, diagnostic_count, truncated) = match plan.binding.pair_mode {
        LimitPairMode::SuccessAtBound | LimitPairMode::BaselineLiveBytePrescanV1 if at_bound => {
            ("success", None, 0, false)
        }
        LimitPairMode::SuccessAtBound | LimitPairMode::BaselineLiveBytePrescanV1 => {
            ("compiler-error", Some(LIMIT_EXCEEDED_ERROR_CODE), 1, false)
        }
        LimitPairMode::DiagnosticCapOnSemanticFailure if at_bound => (
            "compiler-error",
            Some(UNKNOWN_REFERENCE_ERROR_CODE),
            n,
            false,
        ),
        LimitPairMode::DiagnosticCapOnSemanticFailure => (
            "compiler-error",
            Some(DIAGNOSTIC_LIMIT_ERROR_CODE),
            n.checked_sub(1)
                .ok_or_else(|| EvidenceError::LimitRecomputation {
                    detail: format!("限制运行 {run_id} 的诊断 N 不能减一"),
                })?,
            true,
        ),
    };
    expect_string(run, "/failure/expectedOutcome", expected_outcome)?;
    expect_string(run, "/failure/actualOutcome", expected_outcome)?;
    expect_optional_string(run, "/failure/stableCompilerErrorCode", error_code)?;
    expect_u64(run, "/failure/diagnosticCount", diagnostic_count)?;
    expect_bool(run, "/failure/diagnosticsTruncated", truncated)?;
    expect_u64(run, "/failure/partialOutputRecordCount", 0)?;
    Ok(())
}

fn limit_evidence_scale(run: &Value) -> Result<LimitEvidenceScale, EvidenceError> {
    let workload_id = required_string(run, "/workload/id")?
        .parse::<ScalableWorkloadId>()
        .map_err(|error| EvidenceError::LimitRecomputation {
            detail: error.to_string(),
        })?;
    let graph_profile = parse_graph_profile(required_string(run, "/workload/graphProfile")?)?;
    let scale_role = required_string(run, "/workload/scaleRole")?.to_owned();
    if !matches!(scale_role.as_str(), "calibration" | "stress") {
        return Err(EvidenceError::LimitRecomputation {
            detail: format!("限制运行使用非法规模角色 {scale_role}"),
        });
    }
    Ok(LimitEvidenceScale {
        workload_id,
        graph_profile,
        scale_role,
        n: u32::try_from(required_u64(run, "/workload/n")?).map_err(|_| {
            EvidenceError::LimitRecomputation {
                detail: "限制运行 N 超出 u32".to_owned(),
            }
        })?,
        b: observed_u64(run, "/workload/b")?,
    })
}

fn parse_limit_dimension(value: &str) -> Result<LimitDimensionId, EvidenceError> {
    LimitDimensionId::ALL
        .into_iter()
        .find(|dimension| dimension.as_str() == value)
        .ok_or_else(|| EvidenceError::LimitRecomputation {
            detail: format!("未知限制维度 {value}"),
        })
}

fn verify_cleanup_experiments(
    document: &Value,
    runs: &BTreeMap<String, &Value>,
    selected_scales: &SelectedScaleMap,
    referenced_run_ids: &mut BTreeSet<String>,
) -> Result<(usize, usize), EvidenceError> {
    let mut groups = BTreeMap::<String, Vec<(&str, &Value)>>::new();
    for (run_id, run) in runs {
        let phase = required_string(run, "/cleanup/phase")?;
        if phase == "not-applicable" {
            continue;
        }
        let experiment_id = nullable_string(run, "/cleanup/experimentId")?.ok_or_else(|| {
            EvidenceError::CleanupRecomputation {
                detail: format!("清理运行 {run_id} 缺少 experimentId"),
            }
        })?;
        groups
            .entry(experiment_id.to_owned())
            .or_default()
            .push((run_id.as_str(), run));
    }
    let formal_available = required_string(document, "/derived/formalStudyDisposition")?
        == "formal-analysis-available";
    if !formal_available {
        if !groups.is_empty() {
            return Err(EvidenceError::CleanupRecomputation {
                detail: "没有正式分析时不得形成清理实验".to_owned(),
            });
        }
        return Ok((0, 0));
    }
    if groups.len() != 6 {
        return Err(EvidenceError::CleanupRecomputation {
            detail: format!("清理实验必须恰有六组，实际 {}", groups.len()),
        });
    }
    let mut identities = BTreeSet::new();
    let mut checked_runs = 0;
    for (experiment_id, group) in &groups {
        let (case_id, scale) = verify_cleanup_group(experiment_id, group)?;
        let identity = (case_id, scale.scale_role.clone());
        if !identities.insert(identity) {
            return Err(EvidenceError::CleanupRecomputation {
                detail: format!("清理实验 {experiment_id} 与另一组身份重复"),
            });
        }
        let scale_key = (
            scale.workload_id,
            scale.graph_profile,
            scale.scale_role.clone(),
        );
        if selected_scales.get(&scale_key) != Some(&scale) {
            return Err(EvidenceError::CleanupRecomputation {
                detail: format!("清理实验 {experiment_id} 没有使用正式阶梯派生的规模"),
            });
        }
        for (run_id, _) in group {
            referenced_run_ids.insert((*run_id).to_owned());
        }
        checked_runs += group.len();
    }
    let expected = [
        "limit/source-byte-count/plus-one",
        "semantic/missing-reference-per-unit",
        "diagnostic/cap-plus-one",
    ]
    .into_iter()
    .flat_map(|case_id| {
        ["calibration", "stress"]
            .into_iter()
            .map(move |scale_role| (case_id.to_owned(), scale_role.to_owned()))
    })
    .collect::<BTreeSet<_>>();
    if identities != expected {
        return Err(EvidenceError::CleanupRecomputation {
            detail: "六组清理实验没有按三种失败 × 校准/压力精确覆盖".to_owned(),
        });
    }
    Ok((groups.len(), checked_runs))
}

fn verify_cleanup_group(
    experiment_id: &str,
    runs: &[(&str, &Value)],
) -> Result<(String, LimitEvidenceScale), EvidenceError> {
    if runs.len() != 35 {
        return Err(EvidenceError::CleanupRecomputation {
            detail: format!("清理实验 {experiment_id} 必须有三十五条运行"),
        });
    }
    let mut by_sequence = BTreeMap::new();
    for (run_id, run) in runs {
        let sequence = observed_u64(run, "/cleanup/sequenceIndex")?;
        if by_sequence.insert(sequence, (*run_id, *run)).is_some() {
            return Err(EvidenceError::CleanupRecomputation {
                detail: format!("清理实验 {experiment_id} 重复序号 {sequence}"),
            });
        }
        if nullable_string(run, "/cleanup/experimentId")? != Some(experiment_id)
            || required_string(run, "/roundAttempt/scope")? != "single-experiment"
            || required_string(run, "/candidate/id")? != "baseline-std-randomstate-stable-vec-v1"
            || required_string(run, "/candidate/keyDomain")? != "full-pipeline-baseline"
            || required_string(run, "/process/binaryId")? != ATTRIBUTION_BINARY_ID
            || required_string(run, "/status")? != "valid"
            || required_string(run, "/process/exitKind")? != "success"
            || required_string(run, "/workload/graphProfile")? != "shared-fanin-dag-v1"
            || required_string(run, "/workload/stringProfile")? != "short-unique-v1"
            || required_string(run, "/workload/caseId")? != "not-applicable"
        {
            return Err(EvidenceError::CleanupRecomputation {
                detail: format!("清理运行 {run_id} 的固定身份、角色或结果不一致"),
            });
        }
    }
    if by_sequence.keys().copied().collect::<BTreeSet<_>>() != (0..=34).collect() {
        return Err(EvidenceError::CleanupRecomputation {
            detail: format!("清理实验 {experiment_id} 没有完整覆盖序号 0..=34"),
        });
    }
    let baseline = by_sequence[&0].1;
    let recovery = by_sequence[&33].1;
    let fresh = by_sequence[&34].1;
    let workload_id = required_string(baseline, "/workload/id")?.to_owned();
    let scale_role = required_string(baseline, "/workload/scaleRole")?.to_owned();
    let n = required_u64(baseline, "/workload/n")?;
    if !matches!(scale_role.as_str(), "calibration" | "stress") {
        return Err(EvidenceError::CleanupRecomputation {
            detail: format!("清理实验 {experiment_id} 使用非法规模角色 {scale_role}"),
        });
    }
    for (_, run) in by_sequence.values() {
        if run.pointer("/workload") != baseline.pointer("/workload")
            || run.pointer("/candidate") != baseline.pointer("/candidate")
        {
            return Err(EvidenceError::CleanupRecomputation {
                detail: format!("清理实验 {experiment_id} 内工作负载或候选发生漂移"),
            });
        }
    }
    let case_id = required_string(by_sequence[&1].1, "/failure/caseId")?.to_owned();
    let expected_workload = match case_id.as_str() {
        "limit/source-byte-count/plus-one" => "LF-COMP-ID-v1",
        "semantic/missing-reference-per-unit" | "diagnostic/cap-plus-one" => "LF-COMP-CORRIDOR-v1",
        _ => {
            return Err(EvidenceError::CleanupRecomputation {
                detail: format!("清理实验 {experiment_id} 使用未知失败用例 {case_id}"),
            });
        }
    };
    if workload_id != expected_workload {
        return Err(EvidenceError::CleanupRecomputation {
            detail: format!("清理实验 {experiment_id} 的失败用例与工作负载不匹配"),
        });
    }
    let expected_experiment_id = format!("cleanup/{case_id}/{scale_role}/n-{n}");
    if experiment_id != expected_experiment_id {
        return Err(EvidenceError::CleanupRecomputation {
            detail: format!("清理实验标识符应为 {expected_experiment_id}"),
        });
    }

    let primary_instance = nullable_string(baseline, "/compilerInstanceId")?.ok_or_else(|| {
        EvidenceError::CleanupRecomputation {
            detail: format!("清理实验 {experiment_id} 缺少主实例身份"),
        }
    })?;
    for sequence in 0..=33 {
        if nullable_string(by_sequence[&sequence].1, "/compilerInstanceId")?
            != Some(primary_instance)
        {
            return Err(EvidenceError::CleanupRecomputation {
                detail: format!("清理实验 {experiment_id} 的序号 {sequence} 未复用主实例"),
            });
        }
    }
    let fresh_instance = nullable_string(fresh, "/compilerInstanceId")?.ok_or_else(|| {
        EvidenceError::CleanupRecomputation {
            detail: format!("清理实验 {experiment_id} 缺少新实例身份"),
        }
    })?;
    if fresh_instance == primary_instance {
        return Err(EvidenceError::CleanupRecomputation {
            detail: format!("清理实验 {experiment_id} 的判定基准没有使用新实例"),
        });
    }

    verify_cleanup_success_run(
        experiment_id,
        0,
        baseline,
        "baseline-success",
        "cold-instance",
    )?;
    let first_retained = observed_u64(by_sequence[&1].1, "/metrics/retainedCapacityBytes")?;
    let mut failure_digest = None;
    for sequence in 1..=32 {
        let run = by_sequence[&sequence].1;
        verify_cleanup_failure_run(experiment_id, sequence, run, &case_id, n)?;
        let retained = observed_u64(run, "/metrics/retainedCapacityBytes")?;
        if sequence > 1 && retained > first_retained {
            return Err(EvidenceError::CleanupRecomputation {
                detail: format!("清理实验 {experiment_id} 的序号 {sequence} 保留容量超过首轮上界"),
            });
        }
        let digest = required_string(run, "/failure/inputDigest")?;
        if failure_digest
            .replace(digest)
            .is_some_and(|previous| previous != digest)
        {
            return Err(EvidenceError::CleanupRecomputation {
                detail: format!("清理实验 {experiment_id} 的失败输入摘要不稳定"),
            });
        }
    }
    verify_cleanup_success_run(
        experiment_id,
        33,
        recovery,
        "post-recovery-success",
        "stable-capacity-reuse",
    )?;
    verify_cleanup_success_run(
        experiment_id,
        34,
        fresh,
        "fresh-instance-oracle",
        "cold-instance",
    )?;
    for pointer in [
        "/metrics/stageBreakdown",
        "/metrics/semanticDigest",
        "/metrics/diagnosticDigest",
    ] {
        if recovery.pointer(pointer) != fresh.pointer(pointer) {
            return Err(EvidenceError::CleanupRecomputation {
                detail: format!("清理实验 {experiment_id} 的恢复与新实例 {pointer} 不一致"),
            });
        }
    }
    if baseline.pointer("/metrics/semanticDigest") != recovery.pointer("/metrics/semanticDigest") {
        return Err(EvidenceError::CleanupRecomputation {
            detail: format!("清理实验 {experiment_id} 恢复后的语义摘要没有回到基线"),
        });
    }
    let workload_id = workload_id.parse::<ScalableWorkloadId>().map_err(|error| {
        EvidenceError::CleanupRecomputation {
            detail: error.to_string(),
        }
    })?;
    let graph_profile = parse_graph_profile(required_string(baseline, "/workload/graphProfile")?)?;
    Ok((
        case_id,
        LimitEvidenceScale {
            workload_id,
            graph_profile,
            scale_role,
            n: u32::try_from(n).map_err(|_| EvidenceError::CleanupRecomputation {
                detail: format!("清理实验 {experiment_id} 的 N 超出 u32"),
            })?,
            b: observed_u64(baseline, "/workload/b")?,
        },
    ))
}

fn verify_cleanup_success_run(
    experiment_id: &str,
    sequence: u64,
    run: &Value,
    phase: &str,
    sample_kind: &str,
) -> Result<(), EvidenceError> {
    expect_string(run, "/cleanup/phase", phase)?;
    expect_string(run, "/sampleKind", sample_kind)?;
    if nullable_string(run, "/metrics/semanticDigest")?.is_none()
        || nullable_string(run, "/metrics/diagnosticDigest")?.is_none()
        || run.get("failure").is_some()
    {
        return Err(EvidenceError::CleanupRecomputation {
            detail: format!("清理实验 {experiment_id} 的成功序号 {sequence} 结果不完整"),
        });
    }
    Ok(())
}

fn verify_cleanup_failure_run(
    experiment_id: &str,
    sequence: u64,
    run: &Value,
    case_id: &str,
    n: u64,
) -> Result<(), EvidenceError> {
    expect_string(run, "/cleanup/phase", "failure-iteration")?;
    expect_string(run, "/sampleKind", "failure")?;
    expect_string(run, "/failure/caseId", case_id)?;
    expect_string(run, "/failure/expectedOutcome", "compiler-error")?;
    expect_string(run, "/failure/actualOutcome", "compiler-error")?;
    let (dimension, variant, error, diagnostic_count, truncated) = match case_id {
        "limit/source-byte-count/plus-one" => (
            "source-byte-count",
            "canonical-valid-v1",
            LIMIT_EXCEEDED_ERROR_CODE,
            1,
            false,
        ),
        "semantic/missing-reference-per-unit" => (
            "not-applicable",
            "corridor-missing-reference-per-unit-v1",
            UNKNOWN_REFERENCE_ERROR_CODE,
            n,
            false,
        ),
        "diagnostic/cap-plus-one" => (
            "not-applicable",
            "corridor-diagnostic-cap-plus-one-v1",
            DIAGNOSTIC_LIMIT_ERROR_CODE,
            n,
            true,
        ),
        _ => unreachable!("cleanup case was validated before iteration checks"),
    };
    expect_string(run, "/failure/dimensionId", dimension)?;
    expect_string(run, "/failure/inputVariantId", variant)?;
    expect_optional_string(run, "/failure/stableCompilerErrorCode", Some(error))?;
    expect_u64(run, "/failure/diagnosticCount", diagnostic_count)?;
    expect_bool(run, "/failure/diagnosticsTruncated", truncated)?;
    expect_u64(run, "/failure/partialOutputRecordCount", 0)?;
    if nullable_string(run, "/metrics/semanticDigest")?.is_some()
        || nullable_string(run, "/metrics/diagnosticDigest")?.is_none()
        || observed_u64(run, "/metrics/liveRequestedBytes")? != 0
    {
        return Err(EvidenceError::CleanupRecomputation {
            detail: format!(
                "清理实验 {experiment_id} 的失败序号 {sequence} 有语义输出、缺诊断或存续字节未清零"
            ),
        });
    }
    Ok(())
}

fn verify_external_component_binding(
    component: &Value,
    context: &VerificationContext,
    candidate_key: &str,
) -> Result<(), EvidenceError> {
    let dependency_source = required_string(component, "/dependencySource")?;
    let package_name = dependency_source.rsplit('/').next().ok_or_else(|| {
        EvidenceError::CandidateRegistryRecomputation {
            detail: format!("候选 {candidate_key} 的外部组件来源无法解析包名"),
        }
    })?;
    let locked = context
        .direct_cargo_packages
        .get(package_name)
        .ok_or_else(|| EvidenceError::CandidateRegistryRecomputation {
            detail: format!("候选 {candidate_key} 的外部组件 {package_name} 不是研究包直接依赖"),
        })?;
    match required_string(component, "/dependencyKind")? {
        "crates-io" if !locked.source.starts_with("registry+") => {
            return Err(EvidenceError::CandidateRegistryRecomputation {
                detail: format!("候选 {candidate_key} 的 {package_name} 不是 registry 依赖"),
            });
        }
        "git" if !locked.source.starts_with("git+") => {
            return Err(EvidenceError::CandidateRegistryRecomputation {
                detail: format!("候选 {candidate_key} 的 {package_name} 不是 git 依赖"),
            });
        }
        _ => {}
    }
    expect_string(component, "/version", &locked.version)?;
    expect_string(
        component,
        "/dependencyAudit/cargoPackageId/value",
        &locked.id,
    )?;
    expect_string(
        component,
        "/dependencyAudit/cargoPackageChecksumSha256/value",
        &locked.checksum,
    )?;
    let security_status = required_string(component, "/dependencyAudit/securityAudit/status")?;
    let advisory_ids = required_array(component, "/dependencyAudit/securityAudit/advisoryIds")?;
    match security_status {
        "no-known-advisories" | "advisories-present" => {
            let license = locked.license.as_deref().ok_or_else(|| {
                EvidenceError::CandidateRegistryRecomputation {
                    detail: format!("候选 {candidate_key} 的 {package_name} 没有 Cargo 许可证声明"),
                }
            })?;
            expect_string(
                component,
                "/dependencyAudit/licenseSpdxExpression/value",
                license,
            )?;
            let expected_msrv = locked
                .rust_version
                .as_deref()
                .unwrap_or("not-declared;research-toolchain-1.96-all-features-build-validated");
            expect_string(
                component,
                "/dependencyAudit/msrvRustVersion/value",
                expected_msrv,
            )?;
            expect_string(
                component,
                "/dependencyAudit/securityAudit/tool/value",
                "cargo-deny 0.20.2",
            )?;
            let audit_snapshot = required_string(
                component,
                "/dependencyAudit/securityAudit/databaseSnapshot/value",
            )?;
            let audit_digest = audit_snapshot
                .strip_prefix("cargo-deny-output-sha256:")
                .ok_or_else(|| EvidenceError::CandidateRegistryRecomputation {
                    detail: format!(
                        "候选 {candidate_key} 的 {package_name} 审计输出未绑定 SHA-256"
                    ),
                })?;
            if audit_digest.len() != 64
                || !audit_digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
            {
                return Err(EvidenceError::CandidateRegistryRecomputation {
                    detail: format!(
                        "候选 {candidate_key} 的 {package_name} 审计输出 SHA-256 非规范"
                    ),
                });
            }
            if security_status == "no-known-advisories" && !advisory_ids.is_empty()
                || security_status == "advisories-present" && advisory_ids.is_empty()
            {
                return Err(EvidenceError::CandidateRegistryRecomputation {
                    detail: format!(
                        "候选 {candidate_key} 的 {package_name} 审计状态与公告集合不一致"
                    ),
                });
            }
        }
        "audit-unavailable" => {
            if nullable_string(component, "/dependencyAudit/licenseSpdxExpression")?.is_some()
                || nullable_string(component, "/dependencyAudit/msrvRustVersion")?.is_some()
                || nullable_string(component, "/dependencyAudit/securityAudit/tool")?.is_some()
                || nullable_string(component, "/dependencyAudit/securityAudit/databaseSnapshot")?
                    .is_some()
                || !advisory_ids.is_empty()
            {
                return Err(EvidenceError::CandidateRegistryRecomputation {
                    detail: format!(
                        "候选 {candidate_key} 的 {package_name} 审计不可用状态含伪造观测"
                    ),
                });
            }
        }
        other => {
            return Err(EvidenceError::CandidateRegistryRecomputation {
                detail: format!(
                    "候选 {candidate_key} 的 {package_name} 使用非法外部安全审计状态 {other}"
                ),
            });
        }
    }
    let features = required_array(component, "/features")?
        .iter()
        .map(|feature| {
            feature.as_str().map(str::to_owned).ok_or_else(|| {
                EvidenceError::CandidateRegistryRecomputation {
                    detail: format!("候选 {candidate_key} 的 {package_name} 含非字符串 feature"),
                }
            })
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if features != locked.features {
        return Err(EvidenceError::CandidateRegistryRecomputation {
            detail: format!(
                "候选 {candidate_key} 的 {package_name} features 与 research-runner-full 锁定解析不一致"
            ),
        });
    }
    Ok(())
}

fn verify_constant_hash_qualifications(
    trusted: &TrustedContract,
    document: &Value,
    runs: &BTreeMap<String, &Value>,
    referenced_run_ids: &mut BTreeSet<String>,
) -> Result<usize, EvidenceError> {
    let contract = required_object(
        &trusted.workload_manifest,
        "/constantHashQualificationContract",
    )?;
    let expected_candidates = required_array(contract, "/candidateIds")?
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| EvidenceError::ConstantHashQualificationRecomputation {
                    detail: "candidateIds 含非字符串".to_owned(),
                })
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    let qualifications = unique_object_index(
        document,
        "/derived/constantHashQualifications",
        "qualificationId",
    )?;
    let mut candidate_ids = BTreeSet::new();
    for (qualification_id, qualification) in &qualifications {
        let candidate_id = required_string(qualification, "/candidateId")?;
        if !expected_candidates.contains(candidate_id) {
            return Err(EvidenceError::ConstantHashQualificationRecomputation {
                detail: format!("资格 {qualification_id} 使用未登记快速哈希候选 {candidate_id}"),
            });
        }
        if !candidate_ids.insert(candidate_id) {
            return Err(EvidenceError::ConstantHashQualificationRecomputation {
                detail: format!("候选 {candidate_id} 出现多个恒定哈希资格"),
            });
        }
        expect_string(
            qualification,
            "/protocol",
            required_string(contract, "/protocolId")?,
        )?;
        expect_string(
            qualification,
            "/candidateBuilder",
            required_string(contract, "/candidateBuilderId")?,
        )?;
        expect_string(
            qualification,
            "/oracleBuilder",
            required_string(contract, "/oracleBuilderId")?,
        )?;

        let canonical = verify_constant_hash_variant(
            qualification_id,
            candidate_id,
            "constant-hash-canonical-valid-v1",
            qualification,
            "/canonicalValidCandidateRunIds",
            "/canonicalValidOracleRunId",
            runs,
            referenced_run_ids,
        )?;
        let missing = verify_constant_hash_variant(
            qualification_id,
            candidate_id,
            "constant-hash-missing-reference-v1",
            qualification,
            "/missingReferenceCandidateRunIds",
            "/missingReferenceOracleRunId",
            runs,
            referenced_run_ids,
        )?;
        let all_stage_counts = canonical.all_stage_counts && missing.all_stage_counts;
        let semantic_digests = canonical.semantic_digests && missing.semantic_digests;
        let diagnostic_digests = canonical.diagnostic_digests && missing.diagnostic_digests;
        let repeats_deterministic =
            canonical.repeats_deterministic && missing.repeats_deterministic;
        let stable_outcomes = canonical.stable_outcomes && missing.stable_outcomes;
        let partial_outputs = canonical.partial_outputs && missing.partial_outputs;
        let all_runs_valid = canonical.all_runs_valid && missing.all_runs_valid;
        expect_bool(
            qualification,
            "/allStageCountsMatchOracle",
            all_stage_counts,
        )?;
        expect_bool(
            qualification,
            "/semanticDigestsMatchOracle",
            semantic_digests,
        )?;
        expect_bool(
            qualification,
            "/diagnosticDigestsMatchOracle",
            diagnostic_digests,
        )?;
        expect_bool(
            qualification,
            "/candidateRepeatsDeterministic",
            repeats_deterministic,
        )?;
        expect_bool(qualification, "/stableOutcomesMatchOracle", stable_outcomes)?;
        expect_bool(
            qualification,
            "/partialOutputCountsMatchOracle",
            partial_outputs,
        )?;
        expect_bool(
            qualification,
            "/passed",
            all_runs_valid
                && all_stage_counts
                && semantic_digests
                && diagnostic_digests
                && repeats_deterministic
                && stable_outcomes
                && partial_outputs,
        )?;
    }
    Ok(qualifications.len())
}

#[derive(Clone, Copy)]
struct ConstantHashVariantChecks {
    all_stage_counts: bool,
    semantic_digests: bool,
    diagnostic_digests: bool,
    repeats_deterministic: bool,
    stable_outcomes: bool,
    partial_outputs: bool,
    all_runs_valid: bool,
}

#[allow(clippy::too_many_arguments)]
fn verify_constant_hash_variant(
    qualification_id: &str,
    candidate_id: &str,
    input_variant_id: &str,
    qualification: &Value,
    candidate_pointer: &str,
    oracle_pointer: &str,
    runs: &BTreeMap<String, &Value>,
    referenced_run_ids: &mut BTreeSet<String>,
) -> Result<ConstantHashVariantChecks, EvidenceError> {
    let candidate_ids = required_array(qualification, candidate_pointer)?;
    if candidate_ids.len() != 2 {
        return Err(EvidenceError::ConstantHashQualificationRecomputation {
            detail: format!("资格 {qualification_id}/{input_variant_id} 必须引用两个候选运行"),
        });
    }
    let mut candidate_runs = Vec::with_capacity(2);
    let mut repeats = BTreeSet::new();
    for run_id in candidate_ids {
        let run_id = run_id.as_str().ok_or_else(|| {
            EvidenceError::ConstantHashQualificationRecomputation {
                detail: format!("资格 {qualification_id} 含非字符串候选运行 ID"),
            }
        })?;
        let run = constant_hash_run(
            qualification_id,
            candidate_id,
            input_variant_id,
            "candidate-collision-builder",
            run_id,
            runs,
            referenced_run_ids,
        )?;
        repeats.insert(required_u64(run, "/correctnessQualification/repeatIndex")?);
        candidate_runs.push(run);
    }
    if repeats != BTreeSet::from([0, 1]) {
        return Err(EvidenceError::ConstantHashQualificationRecomputation {
            detail: format!("资格 {qualification_id}/{input_variant_id} 候选重复序号不是 0、1"),
        });
    }
    let oracle_id = required_string(qualification, oracle_pointer)?;
    let oracle = constant_hash_run(
        qualification_id,
        candidate_id,
        input_variant_id,
        "exact-oracle",
        oracle_id,
        runs,
        referenced_run_ids,
    )?;
    if required_u64(oracle, "/correctnessQualification/repeatIndex")? != 0 {
        return Err(EvidenceError::ConstantHashQualificationRecomputation {
            detail: format!("资格 {qualification_id}/{input_variant_id} 预言机重复序号必须为零"),
        });
    }
    let repeated = candidate_runs[0];
    let repeat = candidate_runs[1];
    let all_stage_counts = candidate_runs.iter().try_fold(true, |matches, candidate| {
        Ok::<_, EvidenceError>(matches && stage_record_counts_equal(candidate, oracle)?)
    })?;
    let oracle_partial_output = required_u64(
        oracle,
        "/correctnessQualification/actualPartialOutputRecordCount",
    )?;
    let partial_outputs = candidate_runs.iter().try_fold(true, |matches, candidate| {
        Ok::<_, EvidenceError>(
            matches
                && required_u64(
                    candidate,
                    "/correctnessQualification/actualPartialOutputRecordCount",
                )? == oracle_partial_output,
        )
    })?;
    Ok(ConstantHashVariantChecks {
        all_stage_counts,
        semantic_digests: candidate_runs
            .iter()
            .all(|candidate| observed_values_equal(candidate, oracle, "/metrics/semanticDigest")),
        diagnostic_digests: candidate_runs
            .iter()
            .all(|candidate| observed_values_equal(candidate, oracle, "/metrics/diagnosticDigest")),
        repeats_deterministic: constant_hash_repeat_projection(repeated)
            == constant_hash_repeat_projection(repeat),
        stable_outcomes: candidate_runs
            .iter()
            .all(|candidate| constant_hash_outcome_matches(candidate, oracle))
            && constant_hash_actual_matches_expected(oracle)
            && candidate_runs
                .iter()
                .all(|candidate| constant_hash_actual_matches_expected(candidate)),
        partial_outputs,
        all_runs_valid: candidate_runs
            .iter()
            .chain(std::iter::once(&oracle))
            .all(|run| required_string(run, "/status").is_ok_and(|status| status == "valid")),
    })
}

fn constant_hash_run<'a>(
    qualification_id: &str,
    candidate_id: &str,
    input_variant_id: &str,
    role: &str,
    run_id: &str,
    runs: &'a BTreeMap<String, &Value>,
    referenced_run_ids: &mut BTreeSet<String>,
) -> Result<&'a Value, EvidenceError> {
    let run = runs
        .get(run_id)
        .copied()
        .ok_or_else(|| EvidenceError::UnknownReference {
            owner: format!("constant hash qualification {qualification_id}"),
            field: "runId".to_owned(),
            target: run_id.to_owned(),
        })?;
    referenced_run_ids.insert(run_id.to_owned());
    for (pointer, expected) in [
        ("/sampleKind", "correctness"),
        (
            "/correctnessQualification/qualificationId",
            qualification_id,
        ),
        (
            "/correctnessQualification/protocol",
            "constant-hash-full-key-equality-v1",
        ),
        ("/correctnessQualification/role", role),
        (
            "/correctnessQualification/candidateUnderTestId",
            candidate_id,
        ),
        ("/correctnessQualification/inputVariantId", input_variant_id),
    ] {
        expect_string(run, pointer, expected)?;
    }
    let expected_builder = if role == "candidate-collision-builder" {
        "all-keys-u64-zero-v1"
    } else {
        "exact-research-oracle-v1"
    };
    expect_string(run, "/correctnessQualification/builder", expected_builder)?;
    let (expected_candidate, expected_domain) = if role == "candidate-collision-builder" {
        (candidate_id, "validated-fixed-key")
    } else {
        (
            "baseline-std-randomstate-stable-vec-v1",
            "full-pipeline-baseline",
        )
    };
    expect_string(run, "/candidate/id", expected_candidate)?;
    expect_string(run, "/candidate/keyDomain", expected_domain)?;
    for (pointer, expected) in [
        ("/workload/id", "LF-COMP-CORRIDOR-v1"),
        ("/workload/graphProfile", "wide-star-v1"),
        ("/workload/stringProfile", "short-unique-v1"),
        ("/workload/scaleRole", "known-vector"),
        ("/workload/caseId", "not-applicable"),
    ] {
        expect_string(run, pointer, expected)?;
    }
    expect_u64(run, "/workload/revision", 1)?;
    expect_u64(run, "/workload/generatorVersion", 1)?;
    expect_u64(run, "/workload/n", 1)?;
    if nullable_u64(run, "/workload/b")?.is_some() {
        return Err(EvidenceError::ConstantHashQualificationRecomputation {
            detail: format!("正确性运行 {run_id} 不得绑定 B"),
        });
    }
    let actual_outcome = required_string(run, "/correctnessQualification/actualOutcome")?;
    if actual_outcome != "abnormal-termination" && required_string(run, "/status")? != "valid" {
        return Err(EvidenceError::ConstantHashQualificationRecomputation {
            detail: format!("正常返回的正确性运行 {run_id} 非 valid，必须重试而不是进入六运行资格"),
        });
    }
    Ok(run)
}

fn stage_record_counts_equal(left: &Value, right: &Value) -> Result<bool, EvidenceError> {
    for stage in [
        "sourceInput",
        "typedAst",
        "hir",
        "mir",
        "canonicalLir",
        "diagnostics",
        "scratch",
        "outputConstruction",
    ] {
        if required_u64(
            left,
            &format!("/metrics/stageBreakdown/{stage}/recordCount"),
        )? != required_u64(
            right,
            &format!("/metrics/stageBreakdown/{stage}/recordCount"),
        )? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn observed_values_equal(left: &Value, right: &Value, pointer: &str) -> bool {
    left.pointer(&format!("{pointer}/value")) == right.pointer(&format!("{pointer}/value"))
}

fn constant_hash_repeat_projection(run: &Value) -> Vec<Option<&Value>> {
    let mut projection = [
        "/correctnessQualification/actualOutcome",
        "/correctnessQualification/actualStableCompilerErrorCode/value",
        "/correctnessQualification/actualDiagnosticCount",
        "/correctnessQualification/actualDiagnosticsTruncated",
        "/correctnessQualification/actualPartialOutputRecordCount",
        "/metrics/semanticDigest/value",
        "/metrics/diagnosticDigest/value",
    ]
    .into_iter()
    .map(|pointer| run.pointer(pointer))
    .collect::<Vec<_>>();
    for stage in [
        "sourceInput",
        "typedAst",
        "hir",
        "mir",
        "canonicalLir",
        "diagnostics",
        "scratch",
        "outputConstruction",
    ] {
        projection.push(run.pointer(&format!("/metrics/stageBreakdown/{stage}/recordCount")));
    }
    projection
}

fn constant_hash_outcome_matches(candidate: &Value, oracle: &Value) -> bool {
    [
        "/correctnessQualification/actualOutcome",
        "/correctnessQualification/actualStableCompilerErrorCode/value",
        "/correctnessQualification/actualDiagnosticCount",
        "/correctnessQualification/actualDiagnosticsTruncated",
    ]
    .into_iter()
    .all(|pointer| candidate.pointer(pointer) == oracle.pointer(pointer))
}

fn constant_hash_actual_matches_expected(run: &Value) -> bool {
    [
        ("expectedOutcome", "actualOutcome"),
        (
            "expectedStableCompilerErrorCode/value",
            "actualStableCompilerErrorCode/value",
        ),
        ("expectedDiagnosticCount", "actualDiagnosticCount"),
        ("expectedDiagnosticsTruncated", "actualDiagnosticsTruncated"),
        (
            "expectedPartialOutputRecordCount",
            "actualPartialOutputRecordCount",
        ),
    ]
    .into_iter()
    .all(|(expected, actual)| {
        run.pointer(&format!("/correctnessQualification/{expected}"))
            == run.pointer(&format!("/correctnessQualification/{actual}"))
    })
}

fn verify_growth_slopes(
    document: &Value,
    runs: &BTreeMap<String, &Value>,
) -> Result<usize, EvidenceError> {
    let batch_summaries =
        unique_object_index(document, "/derived/ladderBatchSummaries", "summaryId")?;
    let knees = required_array(document, "/derived/knees")?;
    let mut identities = BTreeSet::new();
    for growth in required_array(document, "/derived/growthSlopes")? {
        let identity = format!(
            "{}/{}/{}",
            required_string(growth, "/candidateId")?,
            serde_json::to_string(required_object(growth, "/series")?)
                .expect("growth series serializes"),
            required_string(growth, "/metric")?
        );
        if !identities.insert(identity.clone()) {
            return Err(EvidenceError::DuplicateIdentity {
                collection: "derived.growthSlopes".to_owned(),
                id: identity,
            });
        }
        let batch_zero =
            verify_growth_slope_batch(growth, "/batch0", 0, &batch_summaries, knees, runs)?;
        let batch_one =
            verify_growth_slope_batch(growth, "/batch1", 1, &batch_summaries, knees, runs)?;
        let suggested = upper_slope_bound(batch_zero, batch_one)?;
        expect_signed_ratio(growth, "/suggestedUpperSlope", suggested)?;
    }
    let expected_identities = expected_growth_identities(&batch_summaries, knees)?;
    if identities != expected_identities {
        return Err(EvidenceError::GrowthSlopeRecomputation {
            detail: format!(
                "增长斜率自然身份集合不完整：期望 {expected_identities:?}，实际 {identities:?}"
            ),
        });
    }
    Ok(identities.len())
}

fn expected_growth_identities(
    batch_summaries: &BTreeMap<String, &Value>,
    knees: &[Value],
) -> Result<BTreeSet<String>, EvidenceError> {
    let mut series_by_identity = BTreeMap::<String, (Value, String)>::new();
    for summary in batch_summaries.values() {
        if required_string(summary, "/candidateId")? != crate::BASELINE_CANDIDATE_ID
            || required_string(summary, "/stratum/keyDomain")? != "full-pipeline-baseline"
        {
            continue;
        }
        let series = growth_series_from_stratum(required_object(summary, "/stratum")?);
        let metric = required_string(summary, "/metric")?.to_owned();
        let identity = format!(
            "{}/{}/{}",
            required_string(summary, "/candidateId")?,
            serde_json::to_string(&series).expect("growth series serializes"),
            metric
        );
        series_by_identity
            .entry(identity)
            .or_insert((series, metric));
    }
    let mut expected = BTreeSet::new();
    for (identity, (series, metric)) in series_by_identity {
        let cutoff_n = knees
            .iter()
            .filter(|knee| {
                knee.pointer("/candidateId").and_then(Value::as_str)
                    == Some(crate::BASELINE_CANDIDATE_ID)
                    && knee.pointer("/metric").and_then(Value::as_str) == Some(metric.as_str())
                    && knee
                        .pointer("/confirmedKnee")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    && stratum_matches_growth_series(knee.pointer("/lowerStratum"), Some(&series))
            })
            .map(|knee| required_u64(knee, "/lowerStratum/n"))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .min();
        let mut counts = [0_usize; 2];
        for summary in batch_summaries.values().filter(|summary| {
            summary.pointer("/candidateId").and_then(Value::as_str)
                == Some(crate::BASELINE_CANDIDATE_ID)
                && summary.pointer("/metric").and_then(Value::as_str) == Some(metric.as_str())
                && stratum_matches_growth_series(summary.pointer("/stratum"), Some(&series))
                && cutoff_n.is_none_or(|cutoff| {
                    required_u64(summary, "/stratum/n").is_ok_and(|n| n <= cutoff)
                })
        }) {
            let batch = usize::try_from(required_u64(summary, "/batch")?).map_err(|_| {
                EvidenceError::GrowthSlopeRecomputation {
                    detail: "增长斜率 batch 超出 usize".to_owned(),
                }
            })?;
            if batch > 1 {
                return Err(EvidenceError::GrowthSlopeRecomputation {
                    detail: format!("增长斜率包含非法 batch {batch}"),
                });
            }
            counts[batch] += 1;
        }
        if counts[0] >= 3 && counts[1] >= 3 {
            expected.insert(identity);
        }
    }
    Ok(expected)
}

fn growth_series_from_stratum(stratum: &Value) -> Value {
    let stratum = stratum
        .as_object()
        .expect("required_object 已验证增长斜率分层是对象");
    let mut series = serde_json::Map::new();
    for field in [
        "keyDomain",
        "workloadId",
        "workloadRevision",
        "graphProfile",
        "stringProfile",
        "generatorVersion",
        "b",
        "caseId",
        "sampleKind",
        "binaryMode",
    ] {
        series.insert(
            field.to_owned(),
            stratum
                .get(field)
                .expect("schema-validated stratum field")
                .clone(),
        );
    }
    Value::Object(series)
}

fn verify_growth_slope_batch(
    growth: &Value,
    pointer: &str,
    expected_batch: u64,
    batch_summaries: &BTreeMap<String, &Value>,
    knees: &[Value],
    runs: &BTreeMap<String, &Value>,
) -> Result<SignedRatio, EvidenceError> {
    let batch = required_object(growth, pointer)?;
    let ids = required_array(batch, "/levelBatchSummaryIds")?;
    if ids.len() < 3 {
        return Err(EvidenceError::GrowthSlopeRecomputation {
            detail: format!("{pointer} 少于三个线性区间级别"),
        });
    }
    let expected_ids =
        expected_growth_summary_ids(growth, expected_batch, batch_summaries, knees, runs)?;
    let actual_ids = ids
        .iter()
        .map(|id| {
            id.as_str()
                .map(str::to_owned)
                .ok_or_else(|| EvidenceError::GrowthSlopeRecomputation {
                    detail: format!("{pointer} 含非字符串批次汇总引用"),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if actual_ids != expected_ids {
        return Err(EvidenceError::GrowthSlopeRecomputation {
            detail: format!(
                "{pointer} 未引用确认拐点前的全部且仅全部级别：期望 {expected_ids:?}，实际 {actual_ids:?}"
            ),
        });
    }
    let mut points = Vec::with_capacity(ids.len());
    let mut previous_x = None;
    for id in ids {
        let id = id
            .as_str()
            .ok_or_else(|| EvidenceError::GrowthSlopeRecomputation {
                detail: format!("{pointer} 含非字符串批次汇总引用"),
            })?;
        let summary = batch_summaries
            .get(id)
            .ok_or_else(|| EvidenceError::UnknownReference {
                owner: format!("growth slope {pointer}"),
                field: "levelBatchSummaryIds".to_owned(),
                target: id.to_owned(),
            })?;
        if required_u64(summary, "/batch")? != expected_batch
            || summary.pointer("/candidateId") != growth.pointer("/candidateId")
            || summary.pointer("/metric") != growth.pointer("/metric")
            || !summary_matches_growth_series(summary, growth)
        {
            return Err(EvidenceError::GrowthSlopeRecomputation {
                detail: format!("{pointer} 的批次、候选、指标或增长序列引用不一致：{id}"),
            });
        }
        let x = summary_normalizer(
            summary,
            runs,
            normalizer_for_metric(required_string(growth, "/metric")?),
        )?;
        let y = required_u64(summary, "/median")?;
        if y == 0 || previous_x.is_some_and(|previous| previous >= x) {
            return Err(EvidenceError::GrowthSlopeRecomputation {
                detail: format!("{pointer} 的点必须按严格递增主记录数排列且指标为正"),
            });
        }
        previous_x = Some(x);
        points.push((id, x, y));
    }
    let expected_pair_count = points.len() * (points.len() - 1) / 2;
    let pairwise = required_array(batch, "/pairwiseSlopes")?;
    if pairwise.len() != expected_pair_count {
        return Err(EvidenceError::GrowthSlopeRecomputation {
            detail: format!(
                "{pointer} 的 pairwiseSlopes 数量应为 {expected_pair_count}，实际 {}",
                pairwise.len()
            ),
        });
    }
    let mut recorded_pairs = BTreeMap::new();
    for pair in pairwise {
        let lower = required_string(pair, "/lowerBatchSummaryId")?;
        let upper = required_string(pair, "/upperBatchSummaryId")?;
        if recorded_pairs
            .insert(
                (lower.to_owned(), upper.to_owned()),
                required_i64(pair, "/slopeQ16_16")?,
            )
            .is_some()
        {
            return Err(EvidenceError::GrowthSlopeRecomputation {
                detail: format!("{pointer} 重复两两斜率 {lower}/{upper}"),
            });
        }
    }
    let mut slopes = Vec::with_capacity(expected_pair_count);
    for lower_index in 0..points.len() {
        for upper_index in (lower_index + 1)..points.len() {
            let (lower_id, lower_x, lower_y) = points[lower_index];
            let (upper_id, upper_x, upper_y) = points[upper_index];
            let slope = exact_slope_q16_16(lower_x, lower_y, upper_x, upper_y)?;
            let recorded = recorded_pairs
                .get(&(lower_id.to_owned(), upper_id.to_owned()))
                .ok_or_else(|| EvidenceError::GrowthSlopeRecomputation {
                    detail: format!("{pointer} 缺少两两斜率 {lower_id}/{upper_id}"),
                })?;
            if *recorded != i64::from(slope) {
                return Err(EvidenceError::GrowthSlopeRecomputation {
                    detail: format!(
                        "{pointer} 的 {lower_id}/{upper_id} 斜率不匹配：期望 {slope}，实际 {recorded}"
                    ),
                });
            }
            slopes.push(slope);
        }
    }
    let theil_sen = median_signed_slopes(&slopes)?;
    expect_signed_ratio(batch, "/theilSenSlope", theil_sen)?;
    Ok(theil_sen)
}

fn expected_growth_summary_ids(
    growth: &Value,
    expected_batch: u64,
    batch_summaries: &BTreeMap<String, &Value>,
    knees: &[Value],
    runs: &BTreeMap<String, &Value>,
) -> Result<Vec<String>, EvidenceError> {
    let cutoff_n = knees
        .iter()
        .filter(|knee| {
            knee.pointer("/candidateId") == growth.pointer("/candidateId")
                && knee.pointer("/metric") == growth.pointer("/metric")
                && knee
                    .pointer("/confirmedKnee")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                && stratum_matches_growth_series(
                    knee.pointer("/lowerStratum"),
                    growth.pointer("/series"),
                )
        })
        .map(|knee| required_u64(knee, "/lowerStratum/n"))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .min();
    let mut matching = batch_summaries
        .values()
        .filter(|summary| {
            required_u64(summary, "/batch").ok() == Some(expected_batch)
                && summary.pointer("/candidateId") == growth.pointer("/candidateId")
                && summary.pointer("/metric") == growth.pointer("/metric")
                && summary_matches_growth_series(summary, growth)
                && cutoff_n.is_none_or(|cutoff| {
                    required_u64(summary, "/stratum/n").is_ok_and(|n| n <= cutoff)
                })
        })
        .map(|summary| {
            Ok((
                summary_normalizer(
                    summary,
                    runs,
                    normalizer_for_metric(required_string(growth, "/metric")?),
                )?,
                required_string(summary, "/summaryId")?.to_owned(),
            ))
        })
        .collect::<Result<Vec<_>, EvidenceError>>()?;
    matching.sort_by_key(|(normalizer, _)| *normalizer);
    Ok(matching.into_iter().map(|(_, id)| id).collect())
}

fn stratum_matches_growth_series(stratum: Option<&Value>, series: Option<&Value>) -> bool {
    let (Some(stratum), Some(series)) = (stratum, series) else {
        return false;
    };
    [
        "keyDomain",
        "workloadId",
        "workloadRevision",
        "graphProfile",
        "stringProfile",
        "generatorVersion",
        "b",
        "caseId",
        "sampleKind",
        "binaryMode",
    ]
    .into_iter()
    .all(|field| stratum.get(field) == series.get(field))
}

fn summary_matches_growth_series(summary: &Value, growth: &Value) -> bool {
    [
        "keyDomain",
        "workloadId",
        "workloadRevision",
        "graphProfile",
        "stringProfile",
        "generatorVersion",
        "b",
        "caseId",
        "sampleKind",
        "binaryMode",
    ]
    .into_iter()
    .all(|field| {
        summary.pointer(&format!("/stratum/{field}")) == growth.pointer(&format!("/series/{field}"))
    })
}

fn normalizer_for_metric(metric: &str) -> &'static str {
    if metric == "wall-time-ns" {
        "primary-record-count"
    } else {
        "canonical-lir-shape-output-record-count"
    }
}

fn exact_slope_q16_16(
    lower_x: u64,
    lower_y: u64,
    upper_x: u64,
    upper_y: u64,
) -> Result<i32, EvidenceError> {
    use num_bigint::BigUint;

    if lower_x == 0 || lower_y == 0 || upper_x <= lower_x || upper_y == 0 {
        return Err(EvidenceError::GrowthSlopeRecomputation {
            detail: "增长斜率点必须满足 0 < x_l < x_u 且 y_l,y_u > 0".to_owned(),
        });
    }
    if lower_y == upper_y {
        return Ok(0);
    }
    let (a, b, sign) = if upper_y > lower_y {
        (upper_y, lower_y, 1_i64)
    } else {
        (lower_y, upper_y, -1_i64)
    };
    const FRACTIONAL_DENOMINATOR: u32 = 65_536;
    let a_pow = BigUint::from(a).pow(FRACTIONAL_DENOMINATOR);
    let b_pow = BigUint::from(b).pow(FRACTIONAL_DENOMINATOR);
    let lower_x = BigUint::from(lower_x);
    let upper_x = BigUint::from(upper_x);
    let at_least = |k: u32| &a_pow * lower_x.pow(k) >= &b_pow * upper_x.pow(k);
    let mut high = 1_u32;
    while at_least(high) {
        high = high
            .checked_mul(2)
            .ok_or_else(|| EvidenceError::GrowthSlopeRecomputation {
                detail: "增长斜率超出 Q16.16 i32 范围".to_owned(),
            })?;
        if high > i32::MAX as u32 {
            return Err(EvidenceError::GrowthSlopeRecomputation {
                detail: "增长斜率超出 Q16.16 i32 范围".to_owned(),
            });
        }
    }
    let mut low = high / 2;
    while low + 1 < high {
        let middle = low + (high - low) / 2;
        if at_least(middle) {
            low = middle;
        } else {
            high = middle;
        }
    }
    let midpoint_exponent = low
        .checked_mul(2)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| EvidenceError::GrowthSlopeRecomputation {
            detail: "增长斜率中点指数溢出".to_owned(),
        })?;
    let left = BigUint::from(a).pow(FRACTIONAL_DENOMINATOR * 2) * lower_x.pow(midpoint_exponent);
    let right = BigUint::from(b).pow(FRACTIONAL_DENOMINATOR * 2) * upper_x.pow(midpoint_exponent);
    let rounded = match left.cmp(&right) {
        Ordering::Less => low,
        Ordering::Greater => low + 1,
        Ordering::Equal if low.is_multiple_of(2) => low,
        Ordering::Equal => low + 1,
    };
    let signed = sign
        .checked_mul(i64::from(rounded))
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| EvidenceError::GrowthSlopeRecomputation {
            detail: "增长斜率超出 Q16.16 i32 范围".to_owned(),
        })?;
    Ok(signed)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SignedRatio {
    numerator: i64,
    denominator: i64,
}

fn median_signed_slopes(slopes: &[i32]) -> Result<SignedRatio, EvidenceError> {
    if slopes.is_empty() {
        return Err(EvidenceError::GrowthSlopeRecomputation {
            detail: "不能对空斜率集合计算泰尔－森中位数".to_owned(),
        });
    }
    let mut sorted = slopes.to_vec();
    sorted.sort_unstable();
    if sorted.len() % 2 == 1 {
        Ok(SignedRatio {
            numerator: i64::from(sorted[sorted.len() / 2]),
            denominator: 1,
        })
    } else {
        reduce_signed_ratio(
            i64::from(sorted[sorted.len() / 2 - 1]) + i64::from(sorted[sorted.len() / 2]),
            2,
        )
    }
}

fn upper_slope_bound(left: SignedRatio, right: SignedRatio) -> Result<SignedRatio, EvidenceError> {
    let left_twice = i128::from(left.numerator)
        .checked_mul(2)
        .and_then(|value| value.checked_div(i128::from(left.denominator)))
        .ok_or_else(slope_overflow)?;
    let right_twice = i128::from(right.numerator)
        .checked_mul(2)
        .and_then(|value| value.checked_div(i128::from(right.denominator)))
        .ok_or_else(slope_overflow)?;
    let upper_twice = left_twice
        .max(right_twice)
        .checked_add((left_twice - right_twice).abs())
        .and_then(|value| i64::try_from(value).ok())
        .ok_or_else(slope_overflow)?;
    reduce_signed_ratio(upper_twice, 2)
}

fn reduce_signed_ratio(numerator: i64, denominator: i64) -> Result<SignedRatio, EvidenceError> {
    if denominator <= 0 {
        return Err(slope_overflow());
    }
    let divisor = gcd(numerator.unsigned_abs().into(), denominator as u128) as i64;
    Ok(SignedRatio {
        numerator: numerator / divisor,
        denominator: denominator / divisor,
    })
}

fn expect_signed_ratio(
    value: &Value,
    pointer: &str,
    expected: SignedRatio,
) -> Result<(), EvidenceError> {
    let actual = SignedRatio {
        numerator: required_i64(value, &format!("{pointer}/numerator"))?,
        denominator: required_i64(value, &format!("{pointer}/denominator"))?,
    };
    expect_u64(value, &format!("{pointer}/fractionalBits"), 16)?;
    if actual != expected {
        return Err(EvidenceError::GrowthSlopeRecomputation {
            detail: format!("{pointer} 不匹配：期望 {expected:?}，实际 {actual:?}"),
        });
    }
    Ok(())
}

fn slope_overflow() -> EvidenceError {
    EvidenceError::GrowthSlopeRecomputation {
        detail: "增长斜率有理数算术溢出".to_owned(),
    }
}

fn verify_reproducibility_and_recommendations(
    document: &Value,
) -> Result<(usize, usize), EvidenceError> {
    let round_summaries =
        unique_object_index(document, "/derived/roundMetricSummaries", "summaryId")?;
    let batch_summaries =
        unique_object_index(document, "/derived/ladderBatchSummaries", "summaryId")?;
    let mut pairs_by_key: BTreeMap<String, (Option<&Value>, Option<&Value>)> = BTreeMap::new();
    for summary in batch_summaries.values() {
        let key = batch_pair_key(summary)?;
        let entry = pairs_by_key.entry(key).or_insert((None, None));
        match required_u64(summary, "/batch")? {
            0 if entry.0.is_none() => entry.0 = Some(summary),
            1 if entry.1.is_none() => entry.1 = Some(summary),
            batch => {
                return Err(EvidenceError::FormalRecomputation {
                    detail: format!("批次汇总配对出现重复或非法 batch={batch}"),
                });
            }
        }
    }
    for (key, (batch_zero, batch_one)) in &pairs_by_key {
        if batch_zero.is_none() || batch_one.is_none() {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!("批次汇总 {key} 缺少 batch 0 或 batch 1"),
            });
        }
    }

    let mut expected_envelope_by_metric: BTreeMap<String, (&Value, &Value, PositiveRatio)> =
        BTreeMap::new();
    for (batch_zero, batch_one) in pairs_by_key.values().copied() {
        let batch_zero = batch_zero.expect("pair completeness was checked");
        let batch_one = batch_one.expect("pair completeness was checked");
        let metric = required_string(batch_zero, "/metric")?.to_owned();
        let ratio = bidirectional_repeat_ratio(
            required_u64(batch_zero, "/median")?,
            required_u64(batch_one, "/median")?,
        )?;
        let replace = expected_envelope_by_metric
            .get(&metric)
            .is_none_or(|(_, _, current)| {
                compare_positive_fractions(ratio, *current) == Ordering::Greater
            });
        if replace {
            expected_envelope_by_metric.insert(metric, (batch_zero, batch_one, ratio));
        }
    }

    let mut envelope_by_metric = BTreeMap::new();
    for envelope in required_array(document, "/derived/reproducibilityEnvelopes")? {
        let metric = required_string(envelope, "/metric")?.to_owned();
        if envelope_by_metric
            .insert(metric.clone(), envelope)
            .is_some()
        {
            return Err(EvidenceError::DuplicateIdentity {
                collection: "derived.reproducibilityEnvelopes".to_owned(),
                id: metric,
            });
        }
    }
    if envelope_by_metric.len() != expected_envelope_by_metric.len()
        || envelope_by_metric
            .keys()
            .ne(expected_envelope_by_metric.keys())
    {
        return Err(EvidenceError::FormalRecomputation {
            detail: "重复性包络的指标自然身份集合不完整".to_owned(),
        });
    }
    for (metric, (expected_zero, expected_one, expected_ratio)) in &expected_envelope_by_metric {
        let envelope = envelope_by_metric
            .get(metric)
            .expect("matching envelope key was checked");
        let recorded_zero = required_string(envelope, "/maximizingBatch0LadderBatchSummaryId")?;
        let recorded_one = required_string(envelope, "/maximizingBatch1LadderBatchSummaryId")?;
        let recorded_pair = (
            batch_summaries.get(recorded_zero),
            batch_summaries.get(recorded_one),
        );
        let (Some(recorded_zero_summary), Some(recorded_one_summary)) = recorded_pair else {
            return Err(EvidenceError::UnknownReference {
                owner: format!("reproducibility envelope {metric}"),
                field: "maximizing batch summaries".to_owned(),
                target: format!("{recorded_zero}/{recorded_one}"),
            });
        };
        if batch_pair_key(recorded_zero_summary)? != batch_pair_key(recorded_one_summary)?
            || required_string(recorded_zero_summary, "/metric")? != metric
        {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!("重复性包络 {metric} 的来源批次不是同一分层"),
            });
        }
        let recorded_ratio = bidirectional_repeat_ratio(
            required_u64(recorded_zero_summary, "/median")?,
            required_u64(recorded_one_summary, "/median")?,
        )?;
        if compare_positive_fractions(recorded_ratio, *expected_ratio) != Ordering::Equal {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!("重复性包络 {metric} 没有引用全局最大双向比值"),
            });
        }
        expect_ratio(envelope, "/repeatRatio", *expected_ratio)?;
        // 最大值并列时允许任一真实最大来源；这两个值只用于保证生产者没有错误跨指标。
        debug_assert_eq!(
            required_string(expected_zero, "/metric").ok(),
            Some(metric.as_str())
        );
        debug_assert_eq!(
            required_string(expected_one, "/metric").ok(),
            Some(metric.as_str())
        );
    }

    let mut recommendation_keys = BTreeSet::new();
    for recommendation in required_array(document, "/derived/recommendations")? {
        let metric = required_string(recommendation, "/metric")?;
        if required_string(recommendation, "/reproducibilityEnvelopeMetric")? != metric {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!("预算建议的包络指标与自身指标不一致：{metric}"),
            });
        }
        let batch_zero_id = required_string(recommendation, "/batch0LadderBatchSummaryId")?;
        let batch_one_id = required_string(recommendation, "/batch1LadderBatchSummaryId")?;
        let batch_zero =
            batch_summaries
                .get(batch_zero_id)
                .ok_or_else(|| EvidenceError::UnknownReference {
                    owner: "recommendation".to_owned(),
                    field: "batch0LadderBatchSummaryId".to_owned(),
                    target: batch_zero_id.to_owned(),
                })?;
        let batch_one =
            batch_summaries
                .get(batch_one_id)
                .ok_or_else(|| EvidenceError::UnknownReference {
                    owner: "recommendation".to_owned(),
                    field: "batch1LadderBatchSummaryId".to_owned(),
                    target: batch_one_id.to_owned(),
                })?;
        if required_u64(batch_zero, "/batch")? != 0
            || required_u64(batch_one, "/batch")? != 1
            || batch_pair_key(batch_zero)? != batch_pair_key(batch_one)?
            || recommendation.pointer("/stratum") != batch_zero.pointer("/stratum")
            || recommendation.pointer("/metric") != batch_zero.pointer("/metric")
        {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!("预算建议 {batch_zero_id}/{batch_one_id} 的分层或批次不闭合"),
            });
        }
        let key = batch_pair_key(batch_zero)?;
        if !recommendation_keys.insert(key.clone()) {
            return Err(EvidenceError::DuplicateIdentity {
                collection: "derived.recommendations".to_owned(),
                id: key,
            });
        }
        let observed_upper = maximum_round_median(batch_zero, batch_one, &round_summaries)?;
        expect_u64(recommendation, "/observedUpper", observed_upper)?;
        let envelope =
            envelope_by_metric
                .get(metric)
                .ok_or_else(|| EvidenceError::FormalRecomputation {
                    detail: format!("预算建议缺少指标 {metric} 的重复性包络"),
                })?;
        let ratio = read_ratio(envelope, "/repeatRatio")?;
        let quantum = if metric == "wall-time-ns" {
            required_u64(document, "/protocol/clockQuantumNs")?
        } else {
            1
        };
        expect_u64(recommendation, "/roundingQuantum", quantum)?;
        let value = ceil_ratio_to_quantum(observed_upper, ratio, quantum)?;
        expect_u64(recommendation, "/value", value)?;
    }
    if recommendation_keys.len() != pairs_by_key.len()
        || recommendation_keys.iter().ne(pairs_by_key.keys())
    {
        return Err(EvidenceError::FormalRecomputation {
            detail: "预算建议没有与全部可配对正式分层一一对应".to_owned(),
        });
    }
    Ok((envelope_by_metric.len(), recommendation_keys.len()))
}

fn batch_pair_key(summary: &Value) -> Result<String, EvidenceError> {
    Ok(format!(
        "{}/{}/{}",
        required_string(summary, "/candidateId")?,
        stratum_key(required_object(summary, "/stratum")?)?,
        required_string(summary, "/metric")?,
    ))
}

fn bidirectional_repeat_ratio(left: u64, right: u64) -> Result<PositiveRatio, EvidenceError> {
    let direct = exact_ratio(u128::from(left), u128::from(right))?;
    let inverse = exact_ratio(u128::from(right), u128::from(left))?;
    Ok(
        if compare_positive_fractions(direct, inverse) == Ordering::Greater {
            direct
        } else {
            inverse
        },
    )
}

fn maximum_round_median(
    batch_zero: &Value,
    batch_one: &Value,
    round_summaries: &BTreeMap<String, &Value>,
) -> Result<u64, EvidenceError> {
    required_array(batch_zero, "/roundSummaryIds")?
        .iter()
        .chain(required_array(batch_one, "/roundSummaryIds")?)
        .map(|id| {
            let id = id
                .as_str()
                .ok_or_else(|| EvidenceError::FormalRecomputation {
                    detail: "batch summary 含非字符串 roundSummaryId".to_owned(),
                })?;
            let summary =
                round_summaries
                    .get(id)
                    .ok_or_else(|| EvidenceError::UnknownReference {
                        owner: "recommendation".to_owned(),
                        field: "roundSummaryIds".to_owned(),
                        target: id.to_owned(),
                    })?;
            required_u64(summary, "/median")
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .max()
        .ok_or_else(|| EvidenceError::FormalRecomputation {
            detail: "预算建议没有轮次中位数".to_owned(),
        })
}

fn read_ratio(value: &Value, pointer: &str) -> Result<PositiveRatio, EvidenceError> {
    exact_ratio(
        u128::from(required_u64(value, &format!("{pointer}/numerator"))?),
        u128::from(required_u64(value, &format!("{pointer}/denominator"))?),
    )
}

fn ceil_ratio_to_quantum(
    observed_upper: u64,
    ratio: PositiveRatio,
    quantum: u64,
) -> Result<u64, EvidenceError> {
    let numerator = u128::from(observed_upper)
        .checked_mul(ratio.numerator)
        .ok_or_else(ratio_overflow)?;
    let denominator = ratio
        .denominator
        .checked_mul(u128::from(quantum))
        .ok_or_else(ratio_overflow)?;
    let quanta = numerator
        .checked_add(denominator - 1)
        .ok_or_else(ratio_overflow)?
        / denominator;
    quanta
        .checked_mul(u128::from(quantum))
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(ratio_overflow)
}

fn verify_adjacent_ratios_and_knees(
    document: &Value,
    runs: &BTreeMap<String, &Value>,
) -> Result<(usize, usize), EvidenceError> {
    let round_summaries =
        unique_object_index(document, "/derived/roundMetricSummaries", "summaryId")?;
    let batch_summaries =
        unique_object_index(document, "/derived/ladderBatchSummaries", "summaryId")?;
    let mut adjacent_by_key = BTreeMap::new();
    for adjacent in required_array(document, "/derived/adjacentLevelRatios")? {
        let lower_id = required_string(adjacent, "/lowerLadderBatchSummaryId")?;
        let upper_id = required_string(adjacent, "/upperLadderBatchSummaryId")?;
        let lower =
            batch_summaries
                .get(lower_id)
                .ok_or_else(|| EvidenceError::UnknownReference {
                    owner: "adjacentLevelRatio".to_owned(),
                    field: "lowerLadderBatchSummaryId".to_owned(),
                    target: lower_id.to_owned(),
                })?;
        let upper =
            batch_summaries
                .get(upper_id)
                .ok_or_else(|| EvidenceError::UnknownReference {
                    owner: "adjacentLevelRatio".to_owned(),
                    field: "upperLadderBatchSummaryId".to_owned(),
                    target: upper_id.to_owned(),
                })?;
        verify_adjacent_batch_identity(adjacent, lower, upper)?;
        let normalization_basis = required_string(adjacent, "/normalizationBasis")?;
        let lower_normalizer = summary_normalizer(lower, runs, normalization_basis)?;
        let upper_normalizer = summary_normalizer(upper, runs, normalization_basis)?;
        let mut round_ratios = Vec::with_capacity(5);
        let mut rounds = BTreeSet::new();
        for pair in required_array(adjacent, "/roundPairs")? {
            let round = required_u64(pair, "/round")?;
            rounds.insert(round);
            let lower_round_id = required_string(pair, "/lowerRoundSummaryId")?;
            let upper_round_id = required_string(pair, "/upperRoundSummaryId")?;
            let lower_round = round_summaries.get(lower_round_id).ok_or_else(|| {
                EvidenceError::UnknownReference {
                    owner: "adjacentLevelRatio.roundPair".to_owned(),
                    field: "lowerRoundSummaryId".to_owned(),
                    target: lower_round_id.to_owned(),
                }
            })?;
            let upper_round = round_summaries.get(upper_round_id).ok_or_else(|| {
                EvidenceError::UnknownReference {
                    owner: "adjacentLevelRatio.roundPair".to_owned(),
                    field: "upperRoundSummaryId".to_owned(),
                    target: upper_round_id.to_owned(),
                }
            })?;
            if required_u64(lower_round, "/round")? != round
                || required_u64(upper_round, "/round")? != round
                || lower_round.pointer("/stratum") != lower.pointer("/stratum")
                || upper_round.pointer("/stratum") != upper.pointer("/stratum")
                || lower_round.pointer("/metric") != adjacent.pointer("/metric")
                || upper_round.pointer("/metric") != adjacent.pointer("/metric")
            {
                return Err(EvidenceError::FormalRecomputation {
                    detail: format!("相邻级别轮次对 {lower_round_id}/{upper_round_id} 身份不闭合"),
                });
            }
            let ratio = exact_ratio(
                u128::from(required_u64(upper_round, "/median")?)
                    .checked_mul(u128::from(lower_normalizer))
                    .ok_or_else(ratio_overflow)?,
                u128::from(required_u64(lower_round, "/median")?)
                    .checked_mul(u128::from(upper_normalizer))
                    .ok_or_else(ratio_overflow)?,
            )?;
            expect_ratio(pair, "/ratio", ratio)?;
            round_ratios.push(ratio);
        }
        if rounds != BTreeSet::from([0, 1, 2, 3, 4]) {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!("相邻级别 {lower_id}/{upper_id} 没有覆盖轮次 0..4"),
            });
        }
        let median_ratio = median_ratio(&round_ratios)?;
        expect_ratio(adjacent, "/medianRatio", median_ratio)?;
        let metric = required_string(adjacent, "/metric")?;
        let candidate_knee = match metric {
            "wall-time-ns" => {
                round_ratios
                    .iter()
                    .filter(|ratio| ratio_at_least(**ratio, 11, 10))
                    .count()
                    >= 4
                    && ratio_at_least(median_ratio, 6, 5)
            }
            "peak-live-requested-bytes" => {
                round_ratios
                    .iter()
                    .all(|ratio| ratio_at_least(*ratio, 21, 20))
                    && ratio_at_least(median_ratio, 11, 10)
            }
            "private-bytes" | "commit-peak-bytes" => false,
            other => {
                return Err(EvidenceError::FormalRecomputation {
                    detail: format!("相邻级别使用不支持的指标 {other}"),
                });
            }
        };
        expect_bool(adjacent, "/candidateKnee", candidate_knee)?;
        let key = adjacent_key(adjacent)?;
        if adjacent_by_key.insert(key.clone(), adjacent).is_some() {
            return Err(EvidenceError::DuplicateIdentity {
                collection: "derived.adjacentLevelRatios".to_owned(),
                id: key,
            });
        }
    }

    let mut knee_keys = BTreeSet::new();
    for knee in required_array(document, "/derived/knees")? {
        let batch_zero_key = knee_adjacent_key(knee, 0)?;
        let batch_one_key = knee_adjacent_key(knee, 1)?;
        let batch_zero = adjacent_by_key.get(&batch_zero_key).ok_or_else(|| {
            EvidenceError::UnknownReference {
                owner: "knee".to_owned(),
                field: "candidateBatch0Ratio".to_owned(),
                target: batch_zero_key.clone(),
            }
        })?;
        let batch_one =
            adjacent_by_key
                .get(&batch_one_key)
                .ok_or_else(|| EvidenceError::UnknownReference {
                    owner: "knee".to_owned(),
                    field: "confirmationBatch1Ratio".to_owned(),
                    target: batch_one_key.clone(),
                })?;
        verify_knee_reference_ids(knee, batch_zero, batch_one)?;
        let candidate = required_bool(batch_zero, "/candidateKnee")?;
        let confirmed = candidate && required_bool(batch_one, "/candidateKnee")?;
        expect_bool(knee, "/candidateKnee", candidate)?;
        expect_bool(knee, "/confirmedKnee", confirmed)?;
        let key = batch_zero_key.trim_end_matches("/batch-0").to_owned();
        if !knee_keys.insert(key.clone()) {
            return Err(EvidenceError::DuplicateIdentity {
                collection: "derived.knees".to_owned(),
                id: key,
            });
        }
        if let Some(artifact_sha) = knee
            .pointer("/profilerArtifactSha256/value")
            .and_then(Value::as_str)
        {
            let artifacts = required_array(document, "/artifacts")?;
            if !artifacts.iter().any(|artifact| {
                artifact.get("sha256").and_then(Value::as_str) == Some(artifact_sha)
            }) {
                return Err(EvidenceError::UnknownReference {
                    owner: "knee".to_owned(),
                    field: "profilerArtifactSha256".to_owned(),
                    target: artifact_sha.to_owned(),
                });
            }
        }
    }
    let expected_knee_keys = adjacent_by_key
        .iter()
        .filter(|(key, adjacent)| {
            key.ends_with("/batch-0")
                && required_string(adjacent, "/metric").is_ok_and(|metric| {
                    matches!(metric, "wall-time-ns" | "peak-live-requested-bytes")
                })
        })
        .map(|(key, _)| key.trim_end_matches("/batch-0").to_owned())
        .collect::<BTreeSet<_>>();
    if knee_keys != expected_knee_keys {
        return Err(EvidenceError::FormalRecomputation {
            detail: "knees 没有与 batch 0/1 的可判定相邻级别一一对应".to_owned(),
        });
    }
    Ok((adjacent_by_key.len(), knee_keys.len()))
}

fn recompute_selected_scales(
    trusted: &TrustedContract,
    document: &Value,
) -> Result<SelectedScaleMap, EvidenceError> {
    crate::validate_base_scale_contract(&trusted.workload_manifest).map_err(|error| {
        EvidenceError::FormalRecomputation {
            detail: error.to_string(),
        }
    })?;
    if required_string(document, "/derived/formalStudyDisposition")? != "formal-analysis-available"
    {
        return Ok(BTreeMap::new());
    }

    let mut base_by_source = BTreeMap::<(ScalableWorkloadId, GraphProfileId), u64>::new();
    for base in required_array(document, "/derived/baseScales")? {
        if required_string(base, "/candidateId")? != "baseline-std-randomstate-stable-vec-v1"
            || required_string(base, "/stringProfile")? != "short-unique-v1"
        {
            return Err(EvidenceError::FormalRecomputation {
                detail: "正式规模来源必须使用基线候选和 short-unique-v1".to_owned(),
            });
        }
        let workload_id = required_string(base, "/workloadId")?
            .parse::<ScalableWorkloadId>()
            .map_err(|error| EvidenceError::FormalRecomputation {
                detail: error.to_string(),
            })?;
        let graph_profile = parse_graph_profile(required_string(base, "/graphProfile")?)?;
        let b = observed_u64(base, "/b")?;
        if base_by_source
            .insert((workload_id, graph_profile), b)
            .is_some()
        {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!(
                    "正式规模来源 {}/{graph_profile:?} 重复",
                    workload_id.as_str()
                ),
            });
        }
    }
    let expected_sources = ScalableWorkloadId::ALL
        .into_iter()
        .flat_map(|workload_id| {
            GraphProfileId::ALL
                .into_iter()
                .map(move |graph_profile| (workload_id, graph_profile))
        })
        .collect::<BTreeSet<_>>();
    if base_by_source.keys().copied().collect::<BTreeSet<_>>() != expected_sources {
        return Err(EvidenceError::FormalRecomputation {
            detail: format!(
                "正式分析必须具有九个唯一 B 来源，实际 {}",
                base_by_source.len()
            ),
        });
    }

    let mut level_roles =
        BTreeMap::<(ScalableWorkloadId, GraphProfileId), BTreeMap<u32, String>>::new();
    for summary in required_array(document, "/derived/ladderBatchSummaries")? {
        if required_string(summary, "/candidateId")? != "baseline-std-randomstate-stable-vec-v1"
            || required_string(summary, "/stratum/keyDomain")? != "full-pipeline-baseline"
            || required_string(summary, "/stratum/stringProfile")? != "short-unique-v1"
        {
            return Err(EvidenceError::FormalRecomputation {
                detail: "正式阶梯汇总使用了非来源候选、键域或字符串配置档".to_owned(),
            });
        }
        let workload_id = required_string(summary, "/stratum/workloadId")?
            .parse::<ScalableWorkloadId>()
            .map_err(|error| EvidenceError::FormalRecomputation {
                detail: error.to_string(),
            })?;
        let graph_profile =
            parse_graph_profile(required_string(summary, "/stratum/graphProfile")?)?;
        let source = (workload_id, graph_profile);
        let expected_b = base_by_source[&source];
        if observed_u64(summary, "/stratum/b")? != expected_b {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!("正式阶梯 {source:?} 没有继承唯一 B={expected_b}"),
            });
        }
        let n = u32::try_from(required_u64(summary, "/stratum/n")?).map_err(|_| {
            EvidenceError::FormalRecomputation {
                detail: format!("正式阶梯 {source:?} 的 N 超出 u32"),
            }
        })?;
        let role = required_string(summary, "/stratum/scaleRole")?.to_owned();
        if !matches!(role.as_str(), "base" | "ladder" | "calibration" | "stress") {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!("正式阶梯 {source:?}/N={n} 使用非法规模角色 {role}"),
            });
        }
        let existing = level_roles
            .entry(source)
            .or_default()
            .entry(n)
            .or_insert(role.clone());
        if *existing != role {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!("正式阶梯 {source:?}/N={n} 的规模角色不一致"),
            });
        }
    }
    if level_roles.keys().copied().collect::<BTreeSet<_>>() != expected_sources {
        return Err(EvidenceError::FormalRecomputation {
            detail: "正式阶梯汇总没有精确覆盖九个规模来源".to_owned(),
        });
    }

    let mut first_confirmed_knee = BTreeMap::<(ScalableWorkloadId, GraphProfileId), u32>::new();
    for knee in required_array(document, "/derived/knees")? {
        if !required_bool(knee, "/confirmedKnee")? {
            continue;
        }
        let workload_id = required_string(knee, "/upperStratum/workloadId")?
            .parse::<ScalableWorkloadId>()
            .map_err(|error| EvidenceError::FormalRecomputation {
                detail: error.to_string(),
            })?;
        let graph_profile =
            parse_graph_profile(required_string(knee, "/upperStratum/graphProfile")?)?;
        let source = (workload_id, graph_profile);
        if required_string(knee, "/lowerStratum/workloadId")? != workload_id.as_str()
            || required_string(knee, "/lowerStratum/graphProfile")? != graph_profile.as_str()
        {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!("确认拐点 {source:?} 的上下级自然身份不一致"),
            });
        }
        let upper_n = u32::try_from(required_u64(knee, "/upperStratum/n")?).map_err(|_| {
            EvidenceError::FormalRecomputation {
                detail: format!("确认拐点 {source:?} 的 N 超出 u32"),
            }
        })?;
        first_confirmed_knee
            .entry(source)
            .and_modify(|current| *current = (*current).min(upper_n))
            .or_insert(upper_n);
    }

    let mut selected = BTreeMap::new();
    for source in expected_sources {
        let levels = &level_roles[&source];
        if levels.len() < FORMAL_LADDER_MINIMUM_LEVEL_COUNT {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!("正式阶梯 {source:?} 少于五个完整级别"),
            });
        }
        let ordered = levels.keys().copied().collect::<Vec<_>>();
        let b = base_by_source[&source];
        if u64::from(ordered[0]) != b {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!("正式阶梯 {source:?} 没有从 B={b} 开始"),
            });
        }
        for pair in ordered.windows(2) {
            if pair[0].checked_mul(2) != Some(pair[1]) {
                return Err(EvidenceError::FormalRecomputation {
                    detail: format!("正式阶梯 {source:?} 不是严格二倍序列"),
                });
            }
        }
        let stress_n = first_confirmed_knee
            .get(&source)
            .copied()
            .unwrap_or_else(|| *ordered.last().expect("five levels are non-empty"));
        let Some(calibration_n) = ordered.iter().rev().find(|n| **n < stress_n).copied() else {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!("正式阶梯 {source:?} 无法为压力规模 {stress_n} 选择前一级"),
            });
        };
        if !levels.contains_key(&stress_n) {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!("正式阶梯 {source:?} 的首个确认拐点不在完整级别中"),
            });
        }
        for (n, actual_role) in levels {
            let expected_role = selected_role_for_level(*n, b, calibration_n, stress_n);
            if actual_role != expected_role {
                return Err(EvidenceError::FormalRecomputation {
                    detail: format!(
                        "正式阶梯 {source:?}/N={n} 的规模角色应为 {expected_role}，实际 {actual_role}"
                    ),
                });
            }
        }
        for (role, n) in [("calibration", calibration_n), ("stress", stress_n)] {
            let scale = LimitEvidenceScale {
                workload_id: source.0,
                graph_profile: source.1,
                scale_role: role.to_owned(),
                n,
                b,
            };
            selected.insert((source.0, source.1, role.to_owned()), scale);
        }
    }
    Ok(selected)
}

fn selected_role_for_level(n: u32, b: u64, calibration_n: u32, stress_n: u32) -> &'static str {
    if n == calibration_n {
        "calibration"
    } else if n == stress_n {
        "stress"
    } else if u64::from(n) == b {
        "base"
    } else {
        "ladder"
    }
}

fn verify_selected_scale_role_bindings(
    document: &Value,
    runs: &BTreeMap<String, &Value>,
    selected_scales: &SelectedScaleMap,
) -> Result<(), EvidenceError> {
    for (run_id, run) in runs {
        verify_scale_binding(
            required_object(run, "/workload")?,
            "id",
            run_id,
            selected_scales,
        )?;
    }
    for array_pointer in [
        "/derived/roundMetricSummaries",
        "/derived/ladderBatchSummaries",
        "/derived/growthSlopes",
        "/derived/candidateRosters",
        "/derived/candidateComparisons",
        "/derived/recommendations",
    ] {
        for (index, object) in required_array(document, array_pointer)?.iter().enumerate() {
            if let Some(stratum) = object.get("stratum") {
                verify_scale_binding(
                    stratum,
                    "workloadId",
                    &format!("{array_pointer}[{index}].stratum"),
                    selected_scales,
                )?;
            }
        }
    }
    for array_pointer in ["/derived/adjacentLevelRatios", "/derived/knees"] {
        for (index, object) in required_array(document, array_pointer)?.iter().enumerate() {
            for field in ["lowerStratum", "upperStratum"] {
                verify_scale_binding(
                    required_object(object, &format!("/{field}"))?,
                    "workloadId",
                    &format!("{array_pointer}[{index}].{field}"),
                    selected_scales,
                )?;
            }
        }
    }
    Ok(())
}

fn verify_scale_binding(
    value: &Value,
    workload_field: &str,
    owner: &str,
    selected_scales: &SelectedScaleMap,
) -> Result<(), EvidenceError> {
    let workload_value = required_string(value, &format!("/{workload_field}"))?;
    let Ok(workload_id) = workload_value.parse::<ScalableWorkloadId>() else {
        if workload_value == "LF-COMP-RESEARCH-CURRENT-FIXTURES-v1" {
            return Ok(());
        }
        return Err(EvidenceError::FormalRecomputation {
            detail: format!("{owner} 使用未知工作负载 {workload_value}"),
        });
    };
    let role = required_string(value, "/scaleRole")?;
    if !matches!(role, "base" | "ladder" | "calibration" | "stress") {
        return Ok(());
    }
    let graph_profile = parse_graph_profile(required_string(value, "/graphProfile")?)?;
    let Some(calibration) =
        selected_scales.get(&(workload_id, graph_profile, "calibration".to_owned()))
    else {
        if matches!(role, "calibration" | "stress") {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!("{owner} 在正式规模不足时声明了 {role}"),
            });
        }
        return Ok(());
    };
    let stress = &selected_scales[&(workload_id, graph_profile, "stress".to_owned())];
    let n = u32::try_from(required_u64(value, "/n")?).map_err(|_| {
        EvidenceError::FormalRecomputation {
            detail: format!("{owner} 的 N 超出 u32"),
        }
    })?;
    if observed_u64(value, "/b")? != calibration.b || stress.b != calibration.b {
        return Err(EvidenceError::FormalRecomputation {
            detail: format!("{owner} 没有继承正式规模来源的唯一 B"),
        });
    }
    let expected_role = selected_role_for_level(n, calibration.b, calibration.n, stress.n);
    if role != expected_role {
        return Err(EvidenceError::FormalRecomputation {
            detail: format!("{owner} 的规模角色应为 {expected_role}，实际 {role}"),
        });
    }
    Ok(())
}

fn verify_adjacent_batch_identity(
    adjacent: &Value,
    lower: &Value,
    upper: &Value,
) -> Result<(), EvidenceError> {
    for (adjacent_pointer, lower_pointer, upper_pointer) in [
        ("/candidateId", "/candidateId", "/candidateId"),
        ("/metric", "/metric", "/metric"),
        ("/batch", "/batch", "/batch"),
        ("/lowerStratum", "/stratum", "/unused"),
        ("/upperStratum", "/unused", "/stratum"),
    ] {
        if lower_pointer != "/unused"
            && adjacent.pointer(adjacent_pointer) != lower.pointer(lower_pointer)
        {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!("相邻级别 lower batch 的 {adjacent_pointer} 不一致"),
            });
        }
        if upper_pointer != "/unused"
            && adjacent.pointer(adjacent_pointer) != upper.pointer(upper_pointer)
        {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!("相邻级别 upper batch 的 {adjacent_pointer} 不一致"),
            });
        }
    }
    let lower_n = required_u64(adjacent, "/lowerStratum/n")?;
    let upper_n = required_u64(adjacent, "/upperStratum/n")?;
    if lower_n.checked_mul(2) != Some(upper_n) {
        return Err(EvidenceError::FormalRecomputation {
            detail: format!("相邻级别 N 不是严格二倍：{lower_n} -> {upper_n}"),
        });
    }
    Ok(())
}

fn summary_normalizer(
    summary: &Value,
    runs: &BTreeMap<String, &Value>,
    basis: &str,
) -> Result<u64, EvidenceError> {
    let round_ids = required_array(summary, "/roundSummaryIds")?;
    let round_summary_id = round_ids.first().and_then(Value::as_str).ok_or_else(|| {
        EvidenceError::FormalRecomputation {
            detail: "batch summary 没有可解析的 roundSummaryId".to_owned(),
        }
    })?;
    // 这里通过 summaryId 回查派生数组会形成额外索引；更直接地由该 batch 的第一个
    // 正式运行按完整 stratum 唯一定位，计数本身已经由工作负载重算验证。
    let stratum = required_object(summary, "/stratum")?;
    let run = runs
        .values()
        .find(|run| run_matches_stratum(run, stratum))
        .ok_or_else(|| EvidenceError::FormalRecomputation {
            detail: format!("batch summary {round_summary_id} 找不到同 stratum 运行"),
        })?;
    match basis {
        "primary-record-count" => match required_string(run, "/workload/id")? {
            "LF-COMP-ID-v1" => required_u64(run, "/workload/counts/identityFieldOccurrenceCount"),
            "LF-COMP-CORRIDOR-v1" => required_u64(run, "/workload/counts/sourceRelationCount")?
                .checked_add(required_u64(
                    run,
                    "/workload/counts/canonicalGeometryPoint",
                )?)
                .ok_or_else(ratio_overflow),
            "LF-COMP-JUNCTION-GRID-v1" => [
                "/workload/counts/gateOccurrence",
                "/workload/counts/waitingZoneOccurrence",
                "/workload/counts/routeOccurrence",
            ]
            .into_iter()
            .try_fold(0_u64, |sum, pointer| {
                sum.checked_add(required_u64(run, pointer)?)
                    .ok_or_else(ratio_overflow)
            }),
            other => Err(EvidenceError::FormalRecomputation {
                detail: format!("{other} 没有 primary record 归一化规则"),
            }),
        },
        "canonical-lir-shape-output-record-count" => {
            required_u64(run, "/metrics/stageBreakdown/canonicalLir/recordCount")
        }
        other => Err(EvidenceError::FormalRecomputation {
            detail: format!("未知归一化规则 {other}"),
        }),
    }
}

fn run_matches_stratum(run: &Value, stratum: &Value) -> bool {
    [
        ("/candidate/keyDomain", "/keyDomain"),
        ("/workload/id", "/workloadId"),
        ("/workload/graphProfile", "/graphProfile"),
        ("/workload/stringProfile", "/stringProfile"),
        ("/workload/scaleRole", "/scaleRole"),
        ("/workload/caseId", "/caseId"),
        ("/sampleKind", "/sampleKind"),
    ]
    .into_iter()
    .all(|(run_pointer, stratum_pointer)| {
        run.pointer(run_pointer) == stratum.pointer(stratum_pointer)
    }) && [
        ("/workload/revision", "/workloadRevision"),
        ("/workload/generatorVersion", "/generatorVersion"),
        ("/workload/n", "/n"),
    ]
    .into_iter()
    .all(|(run_pointer, stratum_pointer)| {
        run.pointer(run_pointer) == stratum.pointer(stratum_pointer)
    }) && run.pointer("/workload/b/value") == stratum.pointer("/b/value")
}

fn adjacent_key(adjacent: &Value) -> Result<String, EvidenceError> {
    Ok(format!(
        "{}/{}/{}/{}/batch-{}",
        required_string(adjacent, "/candidateId")?,
        required_string(adjacent, "/metric")?,
        stratum_key(required_object(adjacent, "/lowerStratum")?)?,
        stratum_key(required_object(adjacent, "/upperStratum")?)?,
        required_u64(adjacent, "/batch")?
    ))
}

fn knee_adjacent_key(knee: &Value, batch: u64) -> Result<String, EvidenceError> {
    Ok(format!(
        "{}/{}/{}/{}/batch-{batch}",
        required_string(knee, "/candidateId")?,
        required_string(knee, "/metric")?,
        stratum_key(required_object(knee, "/lowerStratum")?)?,
        stratum_key(required_object(knee, "/upperStratum")?)?,
    ))
}

fn stratum_key(stratum: &Value) -> Result<String, EvidenceError> {
    Ok(format!(
        "{}/{}/n-{}/{}/{}/{}",
        required_string(stratum, "/workloadId")?,
        required_string(stratum, "/graphProfile")?,
        required_u64(stratum, "/n")?,
        required_string(stratum, "/sampleKind")?,
        required_string(stratum, "/binaryMode")?,
        required_string(stratum, "/scaleRole")?,
    ))
}

fn verify_knee_reference_ids(
    knee: &Value,
    batch_zero: &Value,
    batch_one: &Value,
) -> Result<(), EvidenceError> {
    for (reference, adjacent) in [
        ("/candidateBatch0Ratio", batch_zero),
        ("/confirmationBatch1Ratio", batch_one),
    ] {
        if required_string(knee, &format!("{reference}/lowerLadderBatchSummaryId"))?
            != required_string(adjacent, "/lowerLadderBatchSummaryId")?
            || required_string(knee, &format!("{reference}/upperLadderBatchSummaryId"))?
                != required_string(adjacent, "/upperLadderBatchSummaryId")?
        {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!("knee 的 {reference} 没有引用匹配相邻级别"),
            });
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PositiveRatio {
    numerator: u128,
    denominator: u128,
}

fn exact_ratio(numerator: u128, denominator: u128) -> Result<PositiveRatio, EvidenceError> {
    if numerator == 0 || denominator == 0 {
        return Err(EvidenceError::FormalRecomputation {
            detail: "比值分子与分母必须为正".to_owned(),
        });
    }
    let divisor = gcd(numerator, denominator);
    Ok(PositiveRatio {
        numerator: numerator / divisor,
        denominator: denominator / divisor,
    })
}

fn median_ratio(ratios: &[PositiveRatio]) -> Result<PositiveRatio, EvidenceError> {
    if ratios.len() != 5 {
        return Err(EvidenceError::FormalRecomputation {
            detail: "相邻级别必须恰有五个轮次比值".to_owned(),
        });
    }
    let mut sorted = ratios.to_vec();
    sorted.sort_by(|left, right| compare_positive_fractions(*left, *right));
    Ok(sorted[2])
}

fn compare_positive_fractions(mut left: PositiveRatio, mut right: PositiveRatio) -> Ordering {
    let mut reversed = false;
    loop {
        let left_quotient = left.numerator / left.denominator;
        let right_quotient = right.numerator / right.denominator;
        if left_quotient != right_quotient {
            let order = left_quotient.cmp(&right_quotient);
            return if reversed { order.reverse() } else { order };
        }
        let left_remainder = left.numerator % left.denominator;
        let right_remainder = right.numerator % right.denominator;
        match (left_remainder == 0, right_remainder == 0) {
            (true, true) => return Ordering::Equal,
            (true, false) => {
                return if reversed {
                    Ordering::Greater
                } else {
                    Ordering::Less
                };
            }
            (false, true) => {
                return if reversed {
                    Ordering::Less
                } else {
                    Ordering::Greater
                };
            }
            (false, false) => {}
        }
        left = PositiveRatio {
            numerator: left.denominator,
            denominator: left_remainder,
        };
        right = PositiveRatio {
            numerator: right.denominator,
            denominator: right_remainder,
        };
        reversed = !reversed;
    }
}

fn ratio_at_least(ratio: PositiveRatio, numerator: u128, denominator: u128) -> bool {
    compare_positive_fractions(
        ratio,
        PositiveRatio {
            numerator,
            denominator,
        },
    ) != Ordering::Less
}

fn expect_ratio(
    value: &Value,
    pointer: &str,
    expected: PositiveRatio,
) -> Result<(), EvidenceError> {
    let actual = PositiveRatio {
        numerator: u128::from(required_u64(value, &format!("{pointer}/numerator"))?),
        denominator: u128::from(required_u64(value, &format!("{pointer}/denominator"))?),
    };
    if actual != expected {
        return Err(EvidenceError::FormalRecomputation {
            detail: format!("{pointer} 比值不匹配：期望 {expected:?}，实际 {actual:?}"),
        });
    }
    Ok(())
}

fn gcd(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

fn ratio_overflow() -> EvidenceError {
    EvidenceError::FormalRecomputation {
        detail: "正式阶梯比值算术溢出".to_owned(),
    }
}

fn verify_metric_summaries(
    document: &Value,
    runs: &BTreeMap<String, &Value>,
    referenced_run_ids: &mut BTreeSet<String>,
) -> Result<(usize, usize), EvidenceError> {
    let round_summaries =
        unique_object_index(document, "/derived/roundMetricSummaries", "summaryId")?;
    for (summary_id, summary) in &round_summaries {
        let contributing = required_array(summary, "/contributingRunIds")?;
        let sample_kind = required_string(summary, "/stratum/sampleKind")?;
        let expected_sample_count = match sample_kind {
            "cold-instance" => 1,
            "stable-capacity-reuse" => 7,
            other => {
                return Err(EvidenceError::FormalRecomputation {
                    detail: format!("round summary {summary_id} 使用不支持的 sampleKind {other}"),
                });
            }
        };
        if contributing.len() != expected_sample_count {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!(
                    "round summary {summary_id} 的贡献运行数应为 {expected_sample_count}"
                ),
            });
        }
        let mut values = Vec::with_capacity(contributing.len());
        for run_id in contributing {
            let run_id = run_id
                .as_str()
                .ok_or_else(|| EvidenceError::FormalRecomputation {
                    detail: format!("round summary {summary_id} 含非字符串运行引用"),
                })?;
            let run = runs
                .get(run_id)
                .ok_or_else(|| EvidenceError::UnknownReference {
                    owner: format!("round summary {summary_id}"),
                    field: "contributingRunIds".to_owned(),
                    target: run_id.to_owned(),
                })?;
            referenced_run_ids.insert(run_id.to_owned());
            verify_summary_run_identity(summary_id, summary, run_id, run)?;
            values.push(run_metric_value(run, required_string(summary, "/metric")?)?);
        }
        let (median, mad) = median_and_mad(&values)?;
        expect_u64(summary, "/median", median)?;
        expect_u64(summary, "/medianAbsoluteDeviation", mad)?;
    }

    let batch_summaries =
        unique_object_index(document, "/derived/ladderBatchSummaries", "summaryId")?;
    let mut referenced_round_summaries = BTreeSet::new();
    for (summary_id, summary) in &batch_summaries {
        let round_ids = required_array(summary, "/roundSummaryIds")?;
        if round_ids.len() != 5 {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!("ladder batch summary {summary_id} 必须引用五个轮次汇总"),
            });
        }
        let mut rounds = BTreeSet::new();
        let mut medians = Vec::with_capacity(5);
        for round_id in round_ids {
            let round_id = round_id
                .as_str()
                .ok_or_else(|| EvidenceError::FormalRecomputation {
                    detail: format!("ladder batch summary {summary_id} 含非字符串汇总引用"),
                })?;
            let round_summary =
                round_summaries
                    .get(round_id)
                    .ok_or_else(|| EvidenceError::UnknownReference {
                        owner: format!("ladder batch summary {summary_id}"),
                        field: "roundSummaryIds".to_owned(),
                        target: round_id.to_owned(),
                    })?;
            verify_batch_round_identity(summary_id, summary, round_id, round_summary)?;
            rounds.insert(required_u64(round_summary, "/round")?);
            medians.push(required_u64(round_summary, "/median")?);
            if !referenced_round_summaries.insert(round_id.to_owned()) {
                return Err(EvidenceError::FormalRecomputation {
                    detail: format!("round summary {round_id} 被多个 batch summary 重复消费"),
                });
            }
        }
        if rounds != BTreeSet::from([0, 1, 2, 3, 4]) {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!("ladder batch summary {summary_id} 没有完整覆盖轮次 0..4"),
            });
        }
        let (median, mad) = median_and_mad(&medians)?;
        expect_u64(summary, "/median", median)?;
        expect_u64(summary, "/medianAbsoluteDeviation", mad)?;
    }
    let formal_round_summaries = round_summaries
        .iter()
        .filter_map(|(summary_id, summary)| {
            (required_string(summary, "/stratum/keyDomain").ok() == Some("full-pipeline-baseline"))
                .then_some(summary_id.clone())
        })
        .collect::<BTreeSet<_>>();
    if referenced_round_summaries != formal_round_summaries {
        return Err(EvidenceError::FormalRecomputation {
            detail: "正式阶梯 roundMetricSummary 与 ladderBatchSummaries 消费集合不一致".to_owned(),
        });
    }
    Ok((round_summaries.len(), batch_summaries.len()))
}

fn verify_summary_run_identity(
    summary_id: &str,
    summary: &Value,
    run_id: &str,
    run: &Value,
) -> Result<(), EvidenceError> {
    if required_string(run, "/status")? != "valid" {
        return Err(EvidenceError::FormalRecomputation {
            detail: format!("round summary {summary_id} 引用了非有效运行 {run_id}"),
        });
    }
    for (summary_pointer, run_pointer) in [
        ("/candidateId", "/candidate/id"),
        ("/stratum/keyDomain", "/candidate/keyDomain"),
        ("/stratum/workloadId", "/workload/id"),
        ("/stratum/graphProfile", "/workload/graphProfile"),
        ("/stratum/stringProfile", "/workload/stringProfile"),
        ("/stratum/scaleRole", "/workload/scaleRole"),
        ("/stratum/caseId", "/workload/caseId"),
        ("/stratum/sampleKind", "/sampleKind"),
        ("/roundAttemptId", "/roundAttempt/id"),
    ] {
        if required_string(summary, summary_pointer)? != required_string(run, run_pointer)? {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!(
                    "round summary {summary_id} 与运行 {run_id} 的 {summary_pointer}/{run_pointer} 不一致"
                ),
            });
        }
    }
    for (summary_pointer, run_pointer) in [
        ("/stratum/workloadRevision", "/workload/revision"),
        ("/stratum/generatorVersion", "/workload/generatorVersion"),
        ("/stratum/n", "/workload/n"),
        ("/batch", "/batch"),
        ("/round", "/round"),
    ] {
        if required_u64(summary, summary_pointer)? != required_u64(run, run_pointer)? {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!(
                    "round summary {summary_id} 与运行 {run_id} 的 {summary_pointer}/{run_pointer} 不一致"
                ),
            });
        }
    }
    if nullable_u64(summary, "/stratum/b")? != nullable_u64(run, "/workload/b")? {
        return Err(EvidenceError::FormalRecomputation {
            detail: format!("round summary {summary_id} 与运行 {run_id} 的 B 不一致"),
        });
    }
    Ok(())
}

fn verify_batch_round_identity(
    batch_summary_id: &str,
    batch_summary: &Value,
    round_summary_id: &str,
    round_summary: &Value,
) -> Result<(), EvidenceError> {
    for pointer in ["/candidateId", "/stratum", "/metric", "/batch"] {
        if batch_summary.pointer(pointer) != round_summary.pointer(pointer) {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!(
                    "batch summary {batch_summary_id} 与 round summary {round_summary_id} 的 {pointer} 不一致"
                ),
            });
        }
    }
    Ok(())
}

fn run_metric_value(run: &Value, metric: &str) -> Result<u64, EvidenceError> {
    let pointer = match metric {
        "wall-time-ns" => "/metrics/wallTimeNs",
        "allocated-bytes" => "/metrics/allocatedBytes",
        "peak-live-requested-bytes" => "/metrics/peakLiveRequestedBytes",
        "retained-capacity-bytes" => "/metrics/retainedCapacityBytes",
        "working-set-bytes" => "/metrics/workingSetBytes",
        "private-bytes" => "/metrics/privateBytes",
        "commit-peak-bytes" => "/metrics/commitPeakBytes",
        other => {
            return Err(EvidenceError::FormalRecomputation {
                detail: format!("不支持的正式指标 {other}"),
            });
        }
    };
    observed_u64(run, pointer)
}

fn verify_base_scales(
    document: &Value,
    runs: &BTreeMap<String, &Value>,
    referenced_run_ids: &mut BTreeSet<String>,
) -> Result<(usize, usize), EvidenceError> {
    let clock_quantum_ns = required_u64(document, "/protocol/clockQuantumNs")?;
    let minimum_reliable_wall_time_ns = clock_quantum_ns.checked_mul(10_000).ok_or_else(|| {
        EvidenceError::BaseScaleRecomputation {
            detail: "clockQuantumNs × 10000 溢出".to_owned(),
        }
    })?;
    let base_scales = required_array(document, "/derived/baseScales")?;
    let mut identities = BTreeSet::new();
    let mut checked_levels = 0;
    for base_scale in base_scales {
        let identity = format!(
            "{}/{}/{}",
            required_string(base_scale, "/workloadId")?,
            required_string(base_scale, "/graphProfile")?,
            required_string(base_scale, "/stringProfile")?
        );
        if !identities.insert(identity.clone()) {
            return Err(EvidenceError::DuplicateIdentity {
                collection: "derived.baseScales".to_owned(),
                id: identity,
            });
        }
        let mut expected_n = 1_u64;
        let mut first_qualifying_n = None;
        for pilot_level in required_array(base_scale, "/pilotLevels")? {
            let n = required_u64(pilot_level, "/n")?;
            if n != expected_n {
                return Err(EvidenceError::BaseScaleRecomputation {
                    detail: format!(
                        "base scale {identity} 的试运行级别不是从 N=1 严格二倍：期望 {expected_n}，实际 {n}"
                    ),
                });
            }
            expected_n =
                expected_n
                    .checked_mul(2)
                    .ok_or_else(|| EvidenceError::BaseScaleRecomputation {
                        detail: format!("base scale {identity} 的 N 二倍溢出"),
                    })?;
            let contributing = required_array(pilot_level, "/contributingRunIds")?;
            if contributing.len() != 7 {
                return Err(EvidenceError::BaseScaleRecomputation {
                    detail: format!("base scale {identity} N={n} 必须恰有七个计时运行"),
                });
            }
            let mut wall_times = Vec::with_capacity(7);
            let mut semantic_digests = BTreeSet::new();
            let mut all_guards_clear = true;
            let mut seen_contributing = BTreeSet::new();
            for run_id in contributing {
                let run_id =
                    run_id
                        .as_str()
                        .ok_or_else(|| EvidenceError::BaseScaleRecomputation {
                            detail: format!("base scale {identity} N={n} 含非字符串 runId"),
                        })?;
                if !seen_contributing.insert(run_id) {
                    return Err(EvidenceError::BaseScaleRecomputation {
                        detail: format!("base scale {identity} N={n} 重复引用运行 {run_id}"),
                    });
                }
                let run = runs
                    .get(run_id)
                    .ok_or_else(|| EvidenceError::UnknownReference {
                        owner: format!("baseScale {identity} N={n}"),
                        field: "contributingRunIds".to_owned(),
                        target: run_id.to_owned(),
                    })?;
                referenced_run_ids.insert(run_id.to_owned());
                verify_pilot_run_identity(base_scale, n, run_id, run)?;
                wall_times.push(observed_u64(run, "/metrics/wallTimeNs")?);
                semantic_digests.insert(observed_string(run, "/metrics/semanticDigest")?);
                all_guards_clear &= required_string(run, "/guard/trigger")? == "none";
            }
            let (median, mad) = median_and_mad(&wall_times)?;
            expect_u64(pilot_level, "/wallTimeMedianNs", median)?;
            expect_u64(pilot_level, "/wallTimeMedianAbsoluteDeviationNs", mad)?;
            expect_u64(
                pilot_level,
                "/minimumReliableWallTimeNs",
                minimum_reliable_wall_time_ns,
            )?;
            let all_semantic_digests_equal = semantic_digests.len() == 1;
            expect_bool(
                pilot_level,
                "/allSemanticDigestsEqual",
                all_semantic_digests_equal,
            )?;
            expect_bool(pilot_level, "/allGuardsClear", all_guards_clear)?;
            if all_semantic_digests_equal {
                expect_string(
                    pilot_level,
                    "/semanticDigest",
                    semantic_digests
                        .first()
                        .expect("one semantic digest exists"),
                )?;
            }
            verify_unique_pilot_oracle(base_scale, n, pilot_level, runs, referenced_run_ids)?;
            let qualifies = median >= minimum_reliable_wall_time_ns
                && all_semantic_digests_equal
                && all_guards_clear;
            expect_bool(pilot_level, "/qualifies", qualifies)?;
            if qualifies && first_qualifying_n.is_none() {
                first_qualifying_n = Some(n);
            }
            checked_levels += 1;
        }

        let recorded_b = nullable_u64(base_scale, "/b")?;
        if recorded_b != first_qualifying_n {
            return Err(EvidenceError::BaseScaleRecomputation {
                detail: format!(
                    "base scale {identity} 的 B 不等于首个合格级别：记录 {recorded_b:?}，重算 {first_qualifying_n:?}"
                ),
            });
        }
        if let Some(b) = recorded_b {
            let levels = required_array(base_scale, "/pilotLevels")?;
            if levels
                .last()
                .and_then(|level| level.get("n"))
                .and_then(Value::as_u64)
                != Some(b)
            {
                return Err(EvidenceError::BaseScaleRecomputation {
                    detail: format!("base scale {identity} 在首个合格 B={b} 后仍保存额外完成级别"),
                });
            }
        }
    }
    Ok((base_scales.len(), checked_levels))
}

fn verify_pilot_run_identity(
    base_scale: &Value,
    n: u64,
    run_id: &str,
    run: &Value,
) -> Result<(), EvidenceError> {
    for (run_pointer, scale_pointer) in [
        ("/workload/id", "/workloadId"),
        ("/workload/graphProfile", "/graphProfile"),
        ("/workload/stringProfile", "/stringProfile"),
        ("/candidate/id", "/candidateId"),
    ] {
        let expected = required_string(base_scale, scale_pointer)?;
        if required_string(run, run_pointer)? != expected {
            return Err(EvidenceError::BaseScaleRecomputation {
                detail: format!("试运行 {run_id} 的 {run_pointer} 与 base scale 不一致"),
            });
        }
    }
    if required_u64(run, "/workload/n")? != n
        || required_string(run, "/workload/scaleRole")? != "pilot"
        || required_string(run, "/sampleKind")? != "cold-instance"
        || required_string(run, "/status")? != "valid"
    {
        return Err(EvidenceError::BaseScaleRecomputation {
            detail: format!("试运行 {run_id} 的 N、scaleRole、sampleKind 或 status 不合格"),
        });
    }
    Ok(())
}

fn verify_unique_pilot_oracle(
    base_scale: &Value,
    n: u64,
    pilot_level: &Value,
    runs: &BTreeMap<String, &Value>,
    referenced_run_ids: &mut BTreeSet<String>,
) -> Result<(), EvidenceError> {
    let expected_workload = required_string(base_scale, "/workloadId")?;
    let expected_graph = required_string(base_scale, "/graphProfile")?;
    let expected_string = required_string(base_scale, "/stringProfile")?;
    let mut matching = runs.iter().filter(|(_, run)| {
        required_string(run, "/sampleKind").is_ok_and(|value| value == "oracle")
            && required_string(run, "/workload/id").is_ok_and(|value| value == expected_workload)
            && required_string(run, "/workload/graphProfile")
                .is_ok_and(|value| value == expected_graph)
            && required_string(run, "/workload/stringProfile")
                .is_ok_and(|value| value == expected_string)
            && required_u64(run, "/workload/n").is_ok_and(|value| value == n)
    });
    let Some((run_id, run)) = matching.next() else {
        return Err(EvidenceError::BaseScaleRecomputation {
            detail: format!("{expected_workload}/{expected_graph}/N={n} 缺少独立 oracle"),
        });
    };
    if matching.next().is_some()
        || required_string(run, "/status")? != "valid"
        || observed_string(run, "/metrics/semanticDigest")?
            != required_string(pilot_level, "/semanticDigest")?
    {
        return Err(EvidenceError::BaseScaleRecomputation {
            detail: format!("{expected_workload}/{expected_graph}/N={n} 的 oracle 不唯一或不一致"),
        });
    }
    referenced_run_ids.insert(run_id.clone());
    Ok(())
}

fn median_and_mad(values: &[u64]) -> Result<(u64, u64), EvidenceError> {
    if values.is_empty() {
        return Err(EvidenceError::BaseScaleRecomputation {
            detail: "不能对空样本重算中位数".to_owned(),
        });
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let median = sorted[sorted.len() / 2];
    let mut deviations = values
        .iter()
        .map(|value| value.abs_diff(median))
        .collect::<Vec<_>>();
    deviations.sort_unstable();
    Ok((median, deviations[deviations.len() / 2]))
}

fn verify_workload_counts(
    trusted: &TrustedContract,
    runs: &BTreeMap<String, &Value>,
) -> Result<usize, EvidenceError> {
    let factory = ScalableStagePlanFactory::from_trusted_contract(trusted).map_err(|error| {
        EvidenceError::WorkloadRecomputation {
            detail: error.to_string(),
        }
    })?;
    let fixture_summaries = build_current_fixture_summaries(trusted).map_err(|error| {
        EvidenceError::FixtureRecomputation {
            detail: error.to_string(),
        }
    })?;
    let fixture_summary_by_case = fixture_summaries
        .iter()
        .map(|summary| (summary.case_id.as_str(), summary))
        .collect::<BTreeMap<_, _>>();
    let mut observed_fixture_cases = BTreeSet::new();
    let mut checked = 0;
    for (run_id, run) in runs {
        let workload = required_object(run, "/workload")?;
        expect_string(
            workload,
            "/manifestDigest",
            &trusted.descriptor.workload_manifest.sha256,
        )?;
        let workload_id = required_string(workload, "/id")?;
        if workload_id == "LF-COMP-RESEARCH-CURRENT-FIXTURES-v1" {
            let case_id = required_string(workload, "/caseId")?;
            if !observed_fixture_cases.insert(case_id.to_owned()) {
                return Err(EvidenceError::FixtureRecomputation {
                    detail: format!("固定夹具 case {case_id} 出现多次"),
                });
            }
            let summary = fixture_summary_by_case
                .get(case_id)
                .copied()
                .ok_or_else(|| EvidenceError::FixtureRecomputation {
                    detail: format!("固定夹具清单缺少 case {case_id}"),
                })?;
            verify_fixture_inputs(trusted, run_id, case_id, workload)?;
            let mut expected_counts = serde_json::to_value(&summary.counts)
                .expect("fixture counts must serialize")
                .as_object()
                .expect("fixture counts serialize to object")
                .clone();
            expected_counts.extend(
                summary
                    .entity_counts
                    .iter()
                    .map(|(name, value)| (name.clone(), Value::from(*value))),
            );
            expected_counts.extend(
                summary
                    .relation_record_counts
                    .iter()
                    .map(|(name, value)| (name.clone(), Value::from(*value))),
            );
            if required_object(workload, "/counts")? != &Value::Object(expected_counts) {
                return Err(EvidenceError::WorkloadCountsMismatch {
                    run_id: run_id.clone(),
                });
            }
            verify_stage_shape_counts(run_id, run, &summary.stages)?;
            if required_string(run, "/status")? != "valid"
                || observed_string(run, "/metrics/semanticDigest")?
                    != summary.semantic_digest_sha256
            {
                return Err(EvidenceError::FixtureRecomputation {
                    detail: format!("固定夹具运行 {run_id} 的状态或语义摘要与独立投影不一致"),
                });
            }
            checked += 1;
            continue;
        }
        let workload_id = workload_id.parse::<ScalableWorkloadId>().map_err(|error| {
            EvidenceError::WorkloadRecomputation {
                detail: error.to_string(),
            }
        })?;
        let graph_profile = parse_graph_profile(required_string(workload, "/graphProfile")?)?;
        let n = u32::try_from(required_u64(workload, "/n")?).map_err(|_| {
            EvidenceError::WorkloadRecomputation {
                detail: format!("run {run_id} 的 N 超出 u32"),
            }
        })?;
        let plan = factory
            .plan(workload_id, graph_profile, n)
            .map_err(|error| EvidenceError::WorkloadRecomputation {
                detail: error.to_string(),
            })?;
        let mut expected_counts = serde_json::to_value(&plan.counts)
            .expect("serializing exact plan counts cannot fail")
            .as_object()
            .expect("plan counts serialize to an object")
            .clone();
        merge_manifest_per_unit_counts(
            trusted,
            workload_id.as_str(),
            u64::from(n),
            &mut expected_counts,
        )?;
        let actual_counts = required_object(workload, "/counts")?;
        if actual_counts != &Value::Object(expected_counts) {
            return Err(EvidenceError::WorkloadCountsMismatch {
                run_id: run_id.clone(),
            });
        }
        verify_stage_shape_counts(run_id, run, &plan.stages)?;
        checked += 1;
    }
    if !observed_fixture_cases.is_empty() {
        let expected_cases = fixture_summary_by_case
            .keys()
            .map(|case_id| (*case_id).to_owned())
            .collect::<BTreeSet<_>>();
        if observed_fixture_cases != expected_cases {
            return Err(EvidenceError::FixtureRecomputation {
                detail: format!(
                    "固定夹具运行集合不完整：期望 {expected_cases:?}，实际 {observed_fixture_cases:?}"
                ),
            });
        }
        let oracle = verify_current_fixtures_oracle(trusted).map_err(|error| {
            EvidenceError::FixtureRecomputation {
                detail: error.to_string(),
            }
        })?;
        if usize::try_from(oracle.checked_cases).ok() != Some(expected_cases.len())
            || !oracle.independent_identity_and_stream_checked
            || !oracle.scenario_manifest_emits_no_records
            || !oracle.excluded_from_budget_and_candidate_ranking
        {
            return Err(EvidenceError::FixtureRecomputation {
                detail: "固定夹具独立预言机没有闭合全部契约".to_owned(),
            });
        }
    }
    Ok(checked)
}

fn verify_fixture_inputs(
    trusted: &TrustedContract,
    run_id: &str,
    case_id: &str,
    workload: &Value,
) -> Result<(), EvidenceError> {
    let fixture_workload = required_array(&trusted.workload_manifest, "/workloads")?
        .iter()
        .find(|candidate| {
            candidate.pointer("/id").and_then(Value::as_str)
                == Some("LF-COMP-RESEARCH-CURRENT-FIXTURES-v1")
        })
        .ok_or_else(|| EvidenceError::FixtureRecomputation {
            detail: "工作负载清单缺少当前固定夹具投影".to_owned(),
        })?;
    let case = required_array(fixture_workload, "/cases")?
        .iter()
        .find(|candidate| candidate.pointer("/id").and_then(Value::as_str) == Some(case_id))
        .ok_or_else(|| EvidenceError::FixtureRecomputation {
            detail: format!("工作负载清单缺少固定夹具 case {case_id}"),
        })?;
    let expected_files = required_array(case, "/files")?;
    let actual_files = required_array(workload, "/fixtureInputs")?;
    if actual_files != expected_files {
        return Err(EvidenceError::FixtureRecomputation {
            detail: format!("固定夹具运行 {run_id} 的输入文件绑定与清单不一致"),
        });
    }
    let root = repository_root();
    for file in expected_files {
        let relative_path = required_string(file, "/path")?;
        let path = root.join(relative_path);
        let bytes = fs::read(&path).map_err(|source| EvidenceError::ReadFixtureInput {
            path: path.clone(),
            source,
        })?;
        if u64::try_from(bytes.len()).ok() != Some(required_u64(file, "/byteLength")?)
            || sha256_hex(&bytes) != required_string(file, "/sha256")?
        {
            return Err(EvidenceError::FixtureRecomputation {
                detail: format!("固定夹具文件 {relative_path} 的实际长度或 SHA-256 漂移"),
            });
        }
    }
    Ok(())
}

fn merge_manifest_per_unit_counts(
    trusted: &TrustedContract,
    workload_id: &str,
    n: u64,
    counts: &mut serde_json::Map<String, Value>,
) -> Result<(), EvidenceError> {
    let workloads = trusted
        .workload_manifest
        .get("workloads")
        .and_then(Value::as_array)
        .ok_or_else(|| EvidenceError::WorkloadRecomputation {
            detail: "workload manifest 缺少 workloads".to_owned(),
        })?;
    let workload = workloads
        .iter()
        .find(|workload| workload.get("id").and_then(Value::as_str) == Some(workload_id))
        .ok_or_else(|| EvidenceError::WorkloadRecomputation {
            detail: format!("workload manifest 缺少 {workload_id}"),
        })?;
    let per_unit = workload
        .get("perUnitCounts")
        .and_then(Value::as_object)
        .ok_or_else(|| EvidenceError::WorkloadRecomputation {
            detail: format!("workload {workload_id} 缺少 perUnitCounts"),
        })?;
    for (field, value) in per_unit {
        let value = value
            .as_u64()
            .and_then(|value| value.checked_mul(n))
            .ok_or_else(|| EvidenceError::WorkloadRecomputation {
                detail: format!("workload {workload_id} 的 {field} 计数溢出"),
            })?;
        counts.insert(field.clone(), Value::from(value));
    }
    Ok(())
}

fn verify_stage_shape_counts(
    run_id: &str,
    run: &Value,
    expected: &crate::StageBreakdown,
) -> Result<(), EvidenceError> {
    let expected =
        serde_json::to_value(expected).expect("serializing exact stage plan cannot fail");
    for stage in [
        "sourceInput",
        "typedAst",
        "hir",
        "mir",
        "canonicalLir",
        "diagnostics",
        "scratch",
        "outputConstruction",
    ] {
        for field in ["recordCount", "logicalBytes"] {
            let pointer = format!("/metrics/stageBreakdown/{stage}/{field}");
            if run.pointer(&pointer) != expected.pointer(&format!("/{stage}/{field}")) {
                return Err(EvidenceError::StageShapeMismatch {
                    run_id: run_id.to_owned(),
                    stage: stage.to_owned(),
                    field: field.to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn parse_graph_profile(value: &str) -> Result<GraphProfileId, EvidenceError> {
    match value {
        "wide-star-v1" => Ok(GraphProfileId::WideStar),
        "deep-chain-v1" => Ok(GraphProfileId::DeepChain),
        "shared-fanin-dag-v1" => Ok(GraphProfileId::SharedFaninDag),
        _ => Err(EvidenceError::WorkloadRecomputation {
            detail: format!("未知模块图配置档 {value}"),
        }),
    }
}

fn verify_source_bindings(
    trusted: &TrustedContract,
    document: &Value,
    context: &VerificationContext,
) -> Result<(), EvidenceError> {
    expect_string(document, "/schema", EVIDENCE_SCHEMA_ID)?;
    expect_u64(document, "/schemaVersion", EVIDENCE_SCHEMA_VERSION)?;
    expect_string(document, "/source/sourceCommit", &context.repository_head)?;
    expect_string(document, "/source/harnessCommit", &context.repository_head)?;
    expect_string(
        document,
        "/source/cargoLockSha256",
        &context.cargo_lock_sha256,
    )?;
    expect_string(
        document,
        "/source/contractDescriptorId",
        &trusted.descriptor.schema,
    )?;
    expect_u64(
        document,
        "/source/contractDescriptorVersion",
        u64::from(trusted.descriptor.schema_version),
    )?;
    expect_string(
        document,
        "/source/contractDescriptorSha256",
        &trusted.descriptor_sha256,
    )?;
    expect_string(
        document,
        "/source/workloadManifestSha256",
        &trusted.descriptor.workload_manifest.sha256,
    )?;
    expect_string(
        document,
        "/source/evidenceSchemaSha256",
        &trusted.descriptor.evidence_schema.sha256,
    )?;
    if document.pointer("/source/dirty").and_then(Value::as_bool) != Some(false) {
        return Err(EvidenceError::BindingMismatch {
            pointer: "/source/dirty".to_owned(),
            expected: "false".to_owned(),
            actual: render_pointer(document, "/source/dirty"),
        });
    }
    Ok(())
}

fn verify_derived_identities_and_references(
    document: &Value,
    runs: &BTreeMap<String, &Value>,
    referenced_run_ids: &mut BTreeSet<String>,
) -> Result<(), EvidenceError> {
    let mut summary_ids = BTreeSet::new();
    for (array_pointer, id_field) in [
        ("/derived/roundMetricSummaries", "summaryId"),
        ("/derived/ladderBatchSummaries", "summaryId"),
        ("/derived/constantHashQualifications", "qualificationId"),
        ("/derived/candidateRosters", "rosterId"),
    ] {
        for object in required_array(document, array_pointer)? {
            let id = required_string(object, &format!("/{id_field}"))?;
            if !summary_ids.insert(id.to_owned()) {
                return Err(EvidenceError::DuplicateIdentity {
                    collection: "derived identities".to_owned(),
                    id: id.to_owned(),
                });
            }
        }
    }

    for summary in required_array(document, "/derived/roundMetricSummaries")? {
        verify_run_id_array(
            summary,
            "/contributingRunIds",
            "roundMetricSummary",
            runs,
            referenced_run_ids,
        )?;
    }
    for base_scale in required_array(document, "/derived/baseScales")? {
        for pilot_level in required_array(base_scale, "/pilotLevels")? {
            verify_run_id_array(
                pilot_level,
                "/contributingRunIds",
                "baseScalePilotLevel",
                runs,
                referenced_run_ids,
            )?;
        }
        if let Some(terminal_run_id) = base_scale
            .pointer("/terminalGuardRunId/value")
            .and_then(Value::as_str)
        {
            verify_run_reference(
                "baseScale.terminalGuardRunId",
                terminal_run_id,
                runs,
                referenced_run_ids,
            )?;
        }
    }
    for qualification in required_array(document, "/derived/constantHashQualifications")? {
        for pointer in [
            "/canonicalValidCandidateRunIds",
            "/missingReferenceCandidateRunIds",
        ] {
            verify_run_id_array(
                qualification,
                pointer,
                "constantHashQualification",
                runs,
                referenced_run_ids,
            )?;
        }
        for pointer in ["/canonicalValidOracleRunId", "/missingReferenceOracleRunId"] {
            let run_id = required_string(qualification, pointer)?;
            verify_run_reference(
                "constantHashQualification",
                run_id,
                runs,
                referenced_run_ids,
            )?;
        }
    }
    Ok(())
}

fn verify_run_id_array(
    owner: &Value,
    pointer: &str,
    owner_kind: &str,
    runs: &BTreeMap<String, &Value>,
    referenced_run_ids: &mut BTreeSet<String>,
) -> Result<(), EvidenceError> {
    for value in required_array(owner, pointer)? {
        let run_id = value
            .as_str()
            .ok_or_else(|| EvidenceError::MissingOrInvalidField {
                pointer: pointer.to_owned(),
            })?;
        verify_run_reference(owner_kind, run_id, runs, referenced_run_ids)?;
    }
    Ok(())
}

fn verify_run_reference(
    owner: &str,
    run_id: &str,
    runs: &BTreeMap<String, &Value>,
    referenced_run_ids: &mut BTreeSet<String>,
) -> Result<(), EvidenceError> {
    if !runs.contains_key(run_id) {
        return Err(EvidenceError::UnknownReference {
            owner: owner.to_owned(),
            field: "runId".to_owned(),
            target: run_id.to_owned(),
        });
    }
    referenced_run_ids.insert(run_id.to_owned());
    Ok(())
}

fn unique_candidate_binding_index(
    document: &Value,
) -> Result<BTreeMap<String, &Value>, EvidenceError> {
    let mut output = BTreeMap::new();
    for candidate in required_array(document, "/candidateBindings")? {
        let key = candidate_binding_key(candidate)?;
        if output.insert(key.clone(), candidate).is_some() {
            return Err(EvidenceError::DuplicateIdentity {
                collection: "candidateBindings".to_owned(),
                id: key,
            });
        }
    }
    Ok(output)
}

fn candidate_binding_key(candidate: &Value) -> Result<String, EvidenceError> {
    Ok(format!(
        "{}/{}",
        required_string(candidate, "/id")?,
        required_string(candidate, "/keyDomain")?
    ))
}

fn unique_object_index<'a>(
    document: &'a Value,
    array_pointer: &str,
    id_field: &str,
) -> Result<BTreeMap<String, &'a Value>, EvidenceError> {
    let mut output = BTreeMap::new();
    for object in required_array(document, array_pointer)? {
        let id = required_string(object, &format!("/{id_field}"))?.to_owned();
        if output.insert(id.clone(), object).is_some() {
            return Err(EvidenceError::DuplicateIdentity {
                collection: array_pointer.to_owned(),
                id,
            });
        }
    }
    Ok(output)
}

fn required_object<'a>(value: &'a Value, pointer: &str) -> Result<&'a Value, EvidenceError> {
    value
        .pointer(pointer)
        .filter(|value| value.is_object())
        .ok_or_else(|| EvidenceError::MissingOrInvalidField {
            pointer: pointer.to_owned(),
        })
}

fn required_array<'a>(value: &'a Value, pointer: &str) -> Result<&'a [Value], EvidenceError> {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| EvidenceError::MissingOrInvalidField {
            pointer: pointer.to_owned(),
        })
}

fn required_string<'a>(value: &'a Value, pointer: &str) -> Result<&'a str, EvidenceError> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| EvidenceError::MissingOrInvalidField {
            pointer: pointer.to_owned(),
        })
}

fn required_u64(value: &Value, pointer: &str) -> Result<u64, EvidenceError> {
    value
        .pointer(pointer)
        .and_then(Value::as_u64)
        .ok_or_else(|| EvidenceError::MissingOrInvalidField {
            pointer: pointer.to_owned(),
        })
}

fn required_i64(value: &Value, pointer: &str) -> Result<i64, EvidenceError> {
    value
        .pointer(pointer)
        .and_then(Value::as_i64)
        .ok_or_else(|| EvidenceError::MissingOrInvalidField {
            pointer: pointer.to_owned(),
        })
}

fn required_bool(value: &Value, pointer: &str) -> Result<bool, EvidenceError> {
    value
        .pointer(pointer)
        .and_then(Value::as_bool)
        .ok_or_else(|| EvidenceError::MissingOrInvalidField {
            pointer: pointer.to_owned(),
        })
}

fn observed_u64(value: &Value, pointer: &str) -> Result<u64, EvidenceError> {
    required_u64(value, &format!("{pointer}/value"))
}

fn observed_string<'a>(value: &'a Value, pointer: &str) -> Result<&'a str, EvidenceError> {
    required_string(value, &format!("{pointer}/value"))
}

fn nullable_u64(value: &Value, pointer: &str) -> Result<Option<u64>, EvidenceError> {
    let observation = required_object(value, pointer)?;
    let observed =
        observation
            .get("value")
            .ok_or_else(|| EvidenceError::MissingOrInvalidField {
                pointer: format!("{pointer}/value"),
            })?;
    if observed.is_null() {
        Ok(None)
    } else {
        observed
            .as_u64()
            .map(Some)
            .ok_or_else(|| EvidenceError::MissingOrInvalidField {
                pointer: format!("{pointer}/value"),
            })
    }
}

fn nullable_string<'a>(value: &'a Value, pointer: &str) -> Result<Option<&'a str>, EvidenceError> {
    let observation = required_object(value, pointer)?;
    let observed =
        observation
            .get("value")
            .ok_or_else(|| EvidenceError::MissingOrInvalidField {
                pointer: format!("{pointer}/value"),
            })?;
    if observed.is_null() {
        Ok(None)
    } else {
        observed
            .as_str()
            .map(Some)
            .ok_or_else(|| EvidenceError::MissingOrInvalidField {
                pointer: format!("{pointer}/value"),
            })
    }
}

fn expect_optional_string(
    value: &Value,
    pointer: &str,
    expected: Option<&str>,
) -> Result<(), EvidenceError> {
    let actual = nullable_string(value, pointer)?;
    if actual != expected {
        return Err(EvidenceError::BindingMismatch {
            pointer: format!("{pointer}/value"),
            expected: expected.unwrap_or("null").to_owned(),
            actual: actual.unwrap_or("null").to_owned(),
        });
    }
    Ok(())
}

fn expect_string(value: &Value, pointer: &str, expected: &str) -> Result<(), EvidenceError> {
    let actual = required_string(value, pointer)?;
    if actual != expected {
        return Err(EvidenceError::BindingMismatch {
            pointer: pointer.to_owned(),
            expected: expected.to_owned(),
            actual: actual.to_owned(),
        });
    }
    Ok(())
}

fn expect_u64(value: &Value, pointer: &str, expected: u64) -> Result<(), EvidenceError> {
    let actual = required_u64(value, pointer)?;
    if actual != expected {
        return Err(EvidenceError::BindingMismatch {
            pointer: pointer.to_owned(),
            expected: expected.to_string(),
            actual: actual.to_string(),
        });
    }
    Ok(())
}

fn expect_bool(value: &Value, pointer: &str, expected: bool) -> Result<(), EvidenceError> {
    let actual = value
        .pointer(pointer)
        .and_then(Value::as_bool)
        .ok_or_else(|| EvidenceError::MissingOrInvalidField {
            pointer: pointer.to_owned(),
        })?;
    if actual != expected {
        return Err(EvidenceError::BindingMismatch {
            pointer: pointer.to_owned(),
            expected: expected.to_string(),
            actual: actual.to_string(),
        });
    }
    Ok(())
}

fn render_pointer(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .map(Value::to_string)
        .unwrap_or_else(|| "<missing>".to_owned())
}

fn command_stdout(command: &mut Command, label: &'static str) -> Result<String, EvidenceError> {
    let output = command
        .output()
        .map_err(|source| EvidenceError::CommandLaunch { label, source })?;
    if !output.status.success() {
        return Err(EvidenceError::CommandFailed {
            label,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let value = String::from_utf8(output.stdout)
        .map_err(|_| EvidenceError::CommandOutputNotUtf8 { label })?;
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err(EvidenceError::CommandOutputEmpty { label });
    }
    Ok(value)
}

fn write_bytes_atomically(destination: &Path, bytes: &[u8]) -> Result<(), EvidenceError> {
    let parent = destination
        .parent()
        .ok_or_else(|| EvidenceError::MissingOutputParent {
            path: destination.to_path_buf(),
        })?;
    if !parent.is_dir() {
        return Err(EvidenceError::OutputParentNotDirectory {
            path: parent.to_path_buf(),
        });
    }
    let file_name =
        destination
            .file_name()
            .ok_or_else(|| EvidenceError::MissingOutputFileName {
                path: destination.to_path_buf(),
            })?;
    let mut temporary_name = OsString::from(".");
    temporary_name.push(file_name);
    temporary_name.push(format!(".{}.tmp", std::process::id()));
    let temporary = destination.with_file_name(temporary_name);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|source| EvidenceError::WriteEvidence {
            path: temporary.clone(),
            source,
        })?;
    let write_result = file
        .write_all(bytes)
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.sync_all());
    if let Err(source) = write_result {
        drop(file);
        let _ = fs::remove_file(&temporary);
        return Err(EvidenceError::WriteEvidence {
            path: temporary,
            source,
        });
    }
    drop(file);
    fs::rename(&temporary, destination).map_err(|source| {
        let _ = fs::remove_file(&temporary);
        EvidenceError::PublishEvidence {
            source_path: temporary,
            destination_path: destination.to_path_buf(),
            source,
        }
    })
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

fn sha256_with_trailing_newline(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.update(b"\n");
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

#[derive(Debug, thiserror::Error)]
pub enum EvidenceError {
    #[error(transparent)]
    Contract(#[from] ContractError),
    #[error("无法读取证据文件 {path}: {source}")]
    ReadEvidence {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("证据文件 {path} 不是有效 JSON: {source}")]
    InvalidEvidenceJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("证据不满足冻结的 Draft 2020-12 Schema：{detail}")]
    SchemaValidation { detail: String },
    #[error("证据字段 {pointer} 绑定不匹配：期望 {expected}，实际 {actual}")]
    BindingMismatch {
        pointer: String,
        expected: String,
        actual: String,
    },
    #[error("证据字段缺失或类型错误：{pointer}")]
    MissingOrInvalidField { pointer: String },
    #[error("{collection} 存在重复身份 {id}")]
    DuplicateIdentity { collection: String, id: String },
    #[error("{owner} 的 {field} 引用了不存在的目标 {target}")]
    UnknownReference {
        owner: String,
        field: String,
        target: String,
    },
    #[error("运行 {run_id} 的候选快照与 candidateBindings 不一致")]
    CandidateSnapshotMismatch { run_id: String },
    #[error("有效运行 {run_id} 不得包含失效原因")]
    ValidRunHasInvalidationReasons { run_id: String },
    #[error("运行 {run_id} 的外部状态作废原因无法重算：{detail}")]
    ExternalStateRecomputation { run_id: String, detail: String },
    #[error("无法独立重算工作负载：{detail}")]
    WorkloadRecomputation { detail: String },
    #[error("运行 {run_id} 的工作负载计数与受信任清单重算结果不一致")]
    WorkloadCountsMismatch { run_id: String },
    #[error("运行 {run_id} 的阶段 {stage}.{field} 与受信任清单重算结果不一致")]
    StageShapeMismatch {
        run_id: String,
        stage: String,
        field: String,
    },
    #[error("无法重算基础规模：{detail}")]
    BaseScaleRecomputation { detail: String },
    #[error("无法重算正式阶梯：{detail}")]
    FormalRecomputation { detail: String },
    #[error("无法重算增长斜率：{detail}")]
    GrowthSlopeRecomputation { detail: String },
    #[error("无法重算候选注册表绑定：{detail}")]
    CandidateRegistryRecomputation { detail: String },
    #[error("无法重算恒定哈希资格：{detail}")]
    ConstantHashQualificationRecomputation { detail: String },
    #[error("无法重算候选名单：{detail}")]
    CandidateRosterRecomputation { detail: String },
    #[error("无法重算候选比较：{detail}")]
    CandidateComparisonRecomputation { detail: String },
    #[error("无法重算研究二进制绑定：{detail}")]
    BinaryBindingRecomputation { detail: String },
    #[error("找不到证据引用的 release 研究二进制 {binary_id}")]
    MissingResearchBinary { binary_id: String },
    #[error("无法读取研究二进制 {path}: {source}")]
    ReadResearchBinary {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("无法重算当前固定夹具投影：{detail}")]
    FixtureRecomputation { detail: String },
    #[error("无法重算停止护栏：{detail}")]
    GuardRecomputation { detail: String },
    #[error("无法重算限制资格：{detail}")]
    LimitRecomputation { detail: String },
    #[error("无法重算失败清理实验：{detail}")]
    CleanupRecomputation { detail: String },
    #[error("无法独立重算失败输入摘要：{detail}")]
    FailureInputRecomputation { detail: String },
    #[error("无法独立重算诊断摘要：{detail}")]
    DiagnosticRecomputation { detail: String },
    #[error("无法读取固定夹具输入 {path}: {source}")]
    ReadFixtureInput {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("无法读取 Cargo.lock: {source}")]
    ReadCargoLock {
        #[source]
        source: std::io::Error,
    },
    #[error("Cargo.lock 不是有效 UTF-8: {source}")]
    InvalidCargoLockUtf8 {
        #[source]
        source: std::str::Utf8Error,
    },
    #[error("Cargo.lock 不是有效 TOML: {source}")]
    InvalidCargoLockToml {
        #[source]
        source: Box<toml::de::Error>,
    },
    #[error("cargo metadata 输出不是有效 JSON: {source}")]
    InvalidCargoMetadataJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("cargo metadata 缺少 resolve 图")]
    MissingCargoMetadataResolve,
    #[error("cargo metadata 缺少研究包")]
    MissingHarnessMetadataPackage,
    #[error("cargo metadata resolve 图缺少研究包节点")]
    MissingHarnessMetadataNode,
    #[error("cargo metadata 缺少包 {package_id}")]
    MissingCargoMetadataPackage { package_id: String },
    #[error("cargo metadata resolve 图缺少包节点 {package_id}")]
    MissingCargoMetadataNode { package_id: String },
    #[error("Cargo.lock 缺少 cargo metadata 包 {package_id}")]
    MissingCargoLockPackage { package_id: String },
    #[error("Cargo.lock 包 {package_id} 缺少 checksum")]
    MissingCargoLockChecksum { package_id: String },
    #[error("无法启动 {label}: {source}")]
    CommandLaunch {
        label: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("{label} 执行失败：{stderr}")]
    CommandFailed { label: &'static str, stderr: String },
    #[error("{label} 输出不是有效 UTF-8")]
    CommandOutputNotUtf8 { label: &'static str },
    #[error("{label} 没有输出")]
    CommandOutputEmpty { label: &'static str },
    #[error("无法序列化证据 JSON: {source}")]
    SerializeEvidence {
        #[source]
        source: serde_json::Error,
    },
    #[error("证据输出文件已存在，拒绝覆盖：{path}")]
    OutputAlreadyExists { path: PathBuf },
    #[error("证据输出路径没有父目录：{path}")]
    MissingOutputParent { path: PathBuf },
    #[error("证据输出父路径不是目录：{path}")]
    OutputParentNotDirectory { path: PathBuf },
    #[error("证据输出路径没有文件名：{path}")]
    MissingOutputFileName { path: PathBuf },
    #[error("无法写入证据临时文件 {path}: {source}")]
    WriteEvidence {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("无法把证据临时文件 {source_path} 原子发布为 {destination_path}: {source}")]
    PublishEvidence {
        source_path: PathBuf,
        destination_path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timing::ScalableFailureInput;
    #[cfg(feature = "research-runner-full")]
    use crate::{ConstantHashOutcome, ConstantHashRole, qualify_constant_hash_candidate};
    use crate::{
        GraphProfileId, ScalableCompilerInstance, ScalableStagePlanFactory, ScalableWorkloadId,
    };
    use serde_json::{Map, json};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn trusted_schema_and_global_references_accept_a_minimal_guarded_study() {
        let trusted = load_repository_contract().expect("trusted contract");
        let context = test_context();
        let document = minimal_guarded_evidence(&trusted, &context);
        let report = verify_evidence_document(&trusted, &document, &context)
            .expect("minimal guarded evidence must be valid");
        assert_eq!(report.run_count, 1);
        assert_eq!(report.guarded_run_count, 1);
        assert_eq!(report.guard_preflight_check_count, 1);
        assert_eq!(report.referenced_run_count, 1);
    }

    #[test]
    fn guard_preflight_is_recomputed_from_the_frozen_contract() {
        let trusted = load_repository_contract().expect("trusted contract");
        let context = test_context();
        let mut document = minimal_guarded_evidence(&trusted, &context);
        document["runs"][0]["guard"]["nextPrimaryRecordCount"] = json!(2);
        assert!(matches!(
            verify_evidence_document(&trusted, &document, &context),
            Err(EvidenceError::BindingMismatch { pointer, .. })
                if pointer == "/guard/nextPrimaryRecordCount"
        ));
    }

    #[test]
    fn source_binding_is_recomputed_instead_of_trusting_the_document() {
        let trusted = load_repository_contract().expect("trusted contract");
        let context = test_context();
        let mut document = minimal_guarded_evidence(&trusted, &context);
        document["source"]["cargoLockSha256"] = json!("f".repeat(64));
        assert!(matches!(
            verify_evidence_document(&trusted, &document, &context),
            Err(EvidenceError::BindingMismatch { pointer, .. })
                if pointer == "/source/cargoLockSha256"
        ));
    }

    #[test]
    fn duplicate_owner_diagnostic_is_rebuilt_independently_from_the_frozen_template() {
        let trusted = load_repository_contract().expect("trusted contract");
        let mut compiler = ScalableCompilerInstance::<false>::from_trusted_contract_with_id(
            &trusted,
            "duplicate-owner-known-vector".to_owned(),
            ScalableWorkloadId::Corridor,
        )
        .expect("corridor compiler");
        let actual = compiler
            .run_failure(
                GraphProfileId::WideStar,
                2,
                ScalableFailureInput::DuplicateOwnerPerUnit,
            )
            .expect("duplicate-owner failure");
        assert_eq!(
            actual.stable_compiler_error_code,
            DUPLICATE_OWNER_ERROR_CODE
        );
        assert_eq!(actual.diagnostic_count, 2);
        assert!(!actual.diagnostics_truncated);
        assert_eq!(actual.partial_output_record_count, 0);
        assert_eq!(actual.output_record_count, 0);
        assert_eq!(actual.live_requested_bytes_after_run, 0);

        let run = json!({
            "workload": {
                "id": "LF-COMP-CORRIDOR-v1",
                "graphProfile": "wide-star-v1",
                "n": 2
            },
            "failure": {
                "inputVariantId": "corridor-duplicate-owner-per-unit-v1",
                "diagnosticCount": 2,
                "diagnosticsTruncated": false
            }
        });
        let independent = independent_duplicate_owner_diagnostic_digest(&trusted, &run)
            .expect("independent duplicate-owner digest");
        assert_eq!(actual.diagnostic_digest_sha256, independent);
    }

    #[test]
    fn binary_digest_and_role_are_bound_to_the_release_artifact() {
        let trusted = load_repository_contract().expect("trusted contract");
        let context = test_context();
        let mut digest_tampered = minimal_guarded_evidence(&trusted, &context);
        digest_tampered["binaries"][0]["sha256"] = json!("f".repeat(64));
        assert!(matches!(
            verify_evidence_document(&trusted, &digest_tampered, &context),
            Err(EvidenceError::BindingMismatch { pointer, .. }) if pointer == "/sha256"
        ));

        let mut role_tampered = minimal_guarded_evidence(&trusted, &context);
        role_tampered["binaries"][0]["mode"] = json!("memory");
        assert!(matches!(
            verify_evidence_document(&trusted, &role_tampered, &context),
            Err(EvidenceError::BindingMismatch { pointer, .. }) if pointer == "/mode"
        ));

        let mut missing_role = minimal_guarded_evidence(&trusted, &context);
        missing_role["binaries"]
            .as_array_mut()
            .expect("binaries")
            .retain(|binary| binary["id"] != ATTRIBUTION_BINARY_ID);
        assert!(matches!(
            verify_evidence_document(&trusted, &missing_role, &context),
            Err(EvidenceError::BinaryBindingRecomputation { .. })
        ));
    }

    #[cfg(feature = "fixture-oracle")]
    #[test]
    fn current_fixture_runs_bind_real_files_counts_stages_and_independent_oracle() {
        let trusted = load_repository_contract().expect("trusted contract");
        let context = test_context();
        let document = current_fixture_evidence(&trusted, &context);
        let report = verify_evidence_document(&trusted, &document, &context)
            .expect("current fixture evidence");
        assert_eq!(report.run_count, 4);
        assert_eq!(report.workload_count_check_count, 4);

        let mut input_tampered = current_fixture_evidence(&trusted, &context);
        input_tampered["runs"][1]["workload"]["fixtureInputs"][0]["sha256"] = json!("f".repeat(64));
        let runs = unique_object_index(&input_tampered, "/runs", "runId").expect("runs");
        assert!(matches!(
            verify_workload_counts(&trusted, &runs),
            Err(EvidenceError::FixtureRecomputation { .. })
        ));
    }

    #[test]
    fn schema_rejects_a_dropped_required_observation_before_relation_checks() {
        let trusted = load_repository_contract().expect("trusted contract");
        let context = test_context();
        let mut document = minimal_guarded_evidence(&trusted, &context);
        document["runs"][0]
            .as_object_mut()
            .expect("run object")
            .remove("externalState");
        assert!(matches!(
            verify_evidence_document(&trusted, &document, &context),
            Err(EvidenceError::SchemaValidation { .. })
        ));
    }

    #[test]
    fn verifier_keeps_high_background_totals_as_diagnostics() {
        let trusted = load_repository_contract().expect("trusted contract");
        let context = test_context();
        let mut document = minimal_guarded_evidence(&trusted, &context);
        document["runs"][0]["externalState"]["backgroundCpuTimeNs"]["value"] =
            json!(9_000_000_000_u64);
        document["runs"][0]["externalState"]["backgroundWriteBytes"]["value"] =
            json!(900 * 1_048_576_u64);

        verify_evidence_document(&trusted, &document, &context)
            .expect("high background totals are diagnostic observations");
    }

    #[test]
    fn writer_publishes_once_and_verifies_the_exact_written_document() {
        let trusted = load_repository_contract().expect("trusted contract");
        let context = test_context();
        let document = minimal_guarded_evidence(&trusted, &context);
        let directory = temporary_directory("writer");
        let output = directory.join("evidence.json");
        let outcome = publish_evidence_document(&output, &document, &trusted, &context)
            .expect("write evidence");
        assert_eq!(outcome.verification.run_count, 1);
        let written: Value =
            serde_json::from_slice(&fs::read(&output).expect("read output")).expect("parse output");
        assert_eq!(
            verify_evidence_document(&trusted, &written, &context)
                .expect("verify exact output")
                .run_count,
            1
        );
        assert!(matches!(
            publish_evidence_document(&output, &document, &trusted, &context),
            Err(EvidenceError::OutputAlreadyExists { .. })
        ));
        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn qualified_base_scale_recomputes_high_mad_without_using_it_as_a_scale_gate() {
        let trusted = load_repository_contract().expect("trusted contract");
        let context = test_context();
        let document = qualified_base_evidence(&trusted, &context);
        let report = verify_evidence_document(&trusted, &document, &context)
            .expect("qualified base evidence");
        assert_eq!(report.run_count, 8);
        assert_eq!(report.pilot_level_check_count, 1);
        assert_eq!(report.referenced_run_count, 8);
    }

    #[test]
    fn base_scale_tampering_cannot_change_mad_or_selected_b() {
        let trusted = load_repository_contract().expect("trusted contract");
        let context = test_context();
        let mut mad_tampered = qualified_base_evidence(&trusted, &context);
        mad_tampered["derived"]["baseScales"][0]["pilotLevels"][0]["wallTimeMedianAbsoluteDeviationNs"] =
            json!(3);
        assert!(matches!(
            verify_evidence_document(&trusted, &mad_tampered, &context),
            Err(EvidenceError::BindingMismatch { pointer, .. })
                if pointer == "/wallTimeMedianAbsoluteDeviationNs"
        ));

        let mut b_tampered = qualified_base_evidence(&trusted, &context);
        b_tampered["derived"]["baseScales"][0]["b"]["value"] = json!(2);
        assert!(matches!(
            verify_evidence_document(&trusted, &b_tampered, &context),
            Err(EvidenceError::BaseScaleRecomputation { .. })
        ));
    }

    #[test]
    fn workload_count_tampering_is_rejected() {
        let trusted = load_repository_contract().expect("trusted contract");
        let context = test_context();
        let mut document = minimal_guarded_evidence(&trusted, &context);
        document["runs"][0]["workload"]["counts"]["RoadCorridor"] = json!(2);
        assert!(matches!(
            verify_evidence_document(&trusted, &document, &context),
            Err(EvidenceError::SchemaValidation { .. })
                | Err(EvidenceError::WorkloadCountsMismatch { .. })
        ));
    }

    #[test]
    fn external_candidate_binding_must_match_the_resolved_lock_package() {
        let trusted = load_repository_contract().expect("trusted contract");
        let context = test_context();
        let mut document = minimal_guarded_evidence(&trusted, &context);
        let binding = document["candidateBindings"]
            .as_array_mut()
            .expect("candidate bindings")
            .iter_mut()
            .find(|binding| binding["id"] == "hashbrown-xxh3-fixed-v1")
            .expect("hashbrown xxh3 binding");
        binding["components"][0]["version"] = json!("0.17.0");
        assert!(matches!(
            verify_evidence_document(&trusted, &document, &context),
            Err(EvidenceError::BindingMismatch { pointer, .. }) if pointer == "/version"
        ));

        let mut document = minimal_guarded_evidence(&trusted, &context);
        let binding = document["candidateBindings"]
            .as_array_mut()
            .expect("candidate bindings")
            .iter_mut()
            .find(|binding| binding["id"] == "hashbrown-xxh3-fixed-v1")
            .expect("hashbrown xxh3 binding");
        binding["components"][0]["features"] = json!(["inline-more"]);
        assert!(matches!(
            verify_evidence_document(&trusted, &document, &context),
            Err(EvidenceError::CandidateRegistryRecomputation { .. })
        ));
    }

    #[cfg(feature = "research-runner-full")]
    #[test]
    fn constant_hash_qualification_recomputes_all_six_linked_runs() {
        let trusted = load_repository_contract().expect("trusted contract");
        let context = test_context();
        let document = constant_hash_qualification_evidence(&trusted, &context);
        let report = verify_evidence_document(&trusted, &document, &context)
            .expect("constant hash evidence");
        assert_eq!(report.run_count, 7);
        assert_eq!(report.constant_hash_qualification_check_count, 1);
        assert_eq!(report.referenced_run_count, 7);
    }

    #[cfg(feature = "research-runner-full")]
    #[test]
    fn constant_hash_qualification_rejects_digest_and_run_set_tampering() {
        let trusted = load_repository_contract().expect("trusted contract");
        let context = test_context();
        let mut digest_tampered = constant_hash_qualification_evidence(&trusted, &context);
        let run_id = digest_tampered["derived"]["constantHashQualifications"][0]
            ["canonicalValidCandidateRunIds"][0]
            .as_str()
            .expect("candidate run id")
            .to_owned();
        let run = digest_tampered["runs"]
            .as_array_mut()
            .expect("runs")
            .iter_mut()
            .find(|run| run["runId"] == run_id)
            .expect("candidate run");
        run["metrics"]["semanticDigest"]["value"] = json!("f".repeat(64));
        assert!(matches!(
            verify_evidence_document(&trusted, &digest_tampered, &context),
            Err(EvidenceError::BindingMismatch { pointer, .. })
                if pointer == "/semanticDigestsMatchOracle"
        ));

        let mut run_set_tampered = constant_hash_qualification_evidence(&trusted, &context);
        let repeated_run_id = run_set_tampered["derived"]["constantHashQualifications"][0]
            ["missingReferenceCandidateRunIds"][1]
            .as_str()
            .expect("second candidate run id")
            .to_owned();
        let repeated_run = run_set_tampered["runs"]
            .as_array_mut()
            .expect("runs")
            .iter_mut()
            .find(|run| run["runId"] == repeated_run_id)
            .expect("second candidate run");
        repeated_run["correctnessQualification"]["repeatIndex"] = json!(0);
        assert!(matches!(
            verify_evidence_document(&trusted, &run_set_tampered, &context),
            Err(EvidenceError::ConstantHashQualificationRecomputation { .. })
        ));
    }

    #[test]
    fn candidate_roster_recomputes_registry_order_safety_and_correctness_pairs() {
        let trusted = load_repository_contract().expect("trusted contract");
        let context = test_context();
        let document = candidate_roster_evidence(&trusted, &context);
        let bindings = unique_candidate_binding_index(&document).expect("candidate bindings");
        let runs = unique_object_index(&document, "/runs", "runId").expect("runs");
        let mut referenced = BTreeSet::new();
        let (count, rosters) =
            verify_candidate_rosters(&trusted, &document, &bindings, &runs, &mut referenced)
                .expect("candidate roster");
        assert_eq!(count, 1);
        assert_eq!(
            rosters["roster/external-string"].participant_ids,
            [
                "std-hashmap-randomstate-v1".to_owned(),
                "sorted-vec-binary-search-v1".to_owned()
            ]
        );
        assert_eq!(referenced.len(), 3);

        let mut wrong_input_variant = candidate_roster_evidence(&trusted, &context);
        let run = wrong_input_variant["runs"]
            .as_array_mut()
            .expect("runs")
            .iter_mut()
            .find(|run| run["runId"] == "correctness/sorted")
            .expect("candidate correctness run");
        run["workload"]["inputVariantId"] = json!("different-input-v1");
        let bindings =
            unique_candidate_binding_index(&wrong_input_variant).expect("candidate bindings");
        let runs = unique_object_index(&wrong_input_variant, "/runs", "runId").expect("runs");
        assert!(matches!(
            verify_candidate_rosters(
                &trusted,
                &wrong_input_variant,
                &bindings,
                &runs,
                &mut BTreeSet::new()
            ),
            Err(EvidenceError::CandidateRosterRecomputation { .. })
        ));

        let mut wrong_oracle_role = candidate_roster_evidence(&trusted, &context);
        let run = wrong_oracle_role["runs"]
            .as_array_mut()
            .expect("runs")
            .iter_mut()
            .find(|run| run["runId"] == "correctness/oracle")
            .expect("oracle correctness run");
        run["process"]["binaryId"] = json!(TIMING_BINARY_ID);
        let bindings =
            unique_candidate_binding_index(&wrong_oracle_role).expect("candidate bindings");
        let runs = unique_object_index(&wrong_oracle_role, "/runs", "runId").expect("runs");
        assert!(matches!(
            verify_candidate_rosters(
                &trusted,
                &wrong_oracle_role,
                &bindings,
                &runs,
                &mut BTreeSet::new()
            ),
            Err(EvidenceError::BindingMismatch { pointer, .. })
                if pointer == "/process/binaryId"
        ));

        let mut reordered = candidate_roster_evidence(&trusted, &context);
        reordered["derived"]["candidateRosters"][0]["entries"]
            .as_array_mut()
            .expect("entries")
            .swap(0, 1);
        let bindings = unique_candidate_binding_index(&reordered).expect("candidate bindings");
        let runs = unique_object_index(&reordered, "/runs", "runId").expect("runs");
        assert!(matches!(
            verify_candidate_rosters(&trusted, &reordered, &bindings, &runs, &mut BTreeSet::new()),
            Err(EvidenceError::CandidateRosterRecomputation { .. })
        ));

        let mut unsafe_participant = candidate_roster_evidence(&trusted, &context);
        unsafe_participant["derived"]["candidateRosters"][0]["entries"][1]["disposition"] =
            json!("performance-participant");
        unsafe_participant["derived"]["candidateRosters"][0]["entries"][1]["correctnessEvidenceRunIds"] =
            json!(["correctness/std", "correctness/oracle"]);
        let bindings =
            unique_candidate_binding_index(&unsafe_participant).expect("candidate bindings");
        let runs = unique_object_index(&unsafe_participant, "/runs", "runId").expect("runs");
        assert!(matches!(
            verify_candidate_rosters(
                &trusted,
                &unsafe_participant,
                &bindings,
                &runs,
                &mut BTreeSet::new()
            ),
            Err(EvidenceError::CandidateRosterRecomputation { .. })
        ));
    }

    #[test]
    fn candidate_comparison_recomputes_balanced_rounds_ratios_and_decision() {
        let trusted = load_repository_contract().expect("trusted contract");
        let context = test_context();
        let document = candidate_comparison_evidence(&trusted, &context);
        let bindings = unique_candidate_binding_index(&document).expect("candidate bindings");
        let runs = unique_object_index(&document, "/runs", "runId").expect("runs");
        let mut referenced = BTreeSet::new();
        let (_, rosters) =
            verify_candidate_rosters(&trusted, &document, &bindings, &runs, &mut referenced)
                .expect("candidate roster");
        verify_metric_summaries(&document, &runs, &mut referenced)
            .expect("candidate round summaries");
        assert_eq!(
            verify_candidate_comparisons(&trusted, &document, &rosters, &runs, &mut referenced)
                .expect("candidate comparison"),
            1
        );

        let mut position_tampered = candidate_comparison_evidence(&trusted, &context);
        let run = position_tampered["runs"]
            .as_array_mut()
            .expect("runs")
            .iter_mut()
            .find(|run| run["runId"] == "candidate/batch-0/round-0/sorted")
            .expect("candidate performance run");
        run["position"] = json!(0);
        let bindings =
            unique_candidate_binding_index(&position_tampered).expect("candidate bindings");
        let runs = unique_object_index(&position_tampered, "/runs", "runId").expect("runs");
        let (_, rosters) = verify_candidate_rosters(
            &trusted,
            &position_tampered,
            &bindings,
            &runs,
            &mut BTreeSet::new(),
        )
        .expect("candidate roster");
        assert!(matches!(
            verify_candidate_comparisons(
                &trusted,
                &position_tampered,
                &rosters,
                &runs,
                &mut BTreeSet::new()
            ),
            Err(EvidenceError::CandidateComparisonRecomputation { .. })
        ));

        let mut decision_tampered = candidate_comparison_evidence(&trusted, &context);
        decision_tampered["derived"]["candidateComparisons"][0]["decision"] =
            json!("noise-no-difference");
        let bindings =
            unique_candidate_binding_index(&decision_tampered).expect("candidate bindings");
        let runs = unique_object_index(&decision_tampered, "/runs", "runId").expect("runs");
        let (_, rosters) = verify_candidate_rosters(
            &trusted,
            &decision_tampered,
            &bindings,
            &runs,
            &mut BTreeSet::new(),
        )
        .expect("candidate roster");
        assert!(matches!(
            verify_candidate_comparisons(
                &trusted,
                &decision_tampered,
                &rosters,
                &runs,
                &mut BTreeSet::new()
            ),
            Err(EvidenceError::BindingMismatch { pointer, .. }) if pointer == "/decision"
        ));
    }

    #[test]
    fn candidate_performance_scope_is_exact_and_binds_the_input_variant() {
        let trusted = load_repository_contract().expect("trusted contract");
        let scope = CandidatePerformanceScopeContract::from_trusted_contract(&trusted)
            .expect("candidate performance scope");
        let selected_scales = test_selected_scales();
        let mut rosters = BTreeMap::new();
        for scale_role in CandidateScaleRole::ALL {
            let n = 1;
            for key_domain in [
                CandidateKeyDomain::ExternalString,
                CandidateKeyDomain::ValidatedFixedKey,
                CandidateKeyDomain::CanonicalOutputOrder,
            ] {
                let roster_id = expected_candidate_roster_id(&scope, key_domain, scale_role, n);
                rosters.insert(
                    roster_id,
                    VerifiedCandidateRoster {
                        stratum: json!({
                            "keyDomain": key_domain.as_str(),
                            "workloadId": scope.workload_id.as_str(),
                            "workloadRevision": scope.workload_revision,
                            "graphProfile": scope.graph_profile.as_str(),
                            "stringProfile": scope.string_profile,
                            "generatorVersion": scope.generator_version,
                            "n": n,
                            "b": {"value": 1, "reason": null},
                            "scaleRole": scale_role.as_str(),
                            "caseId": scope.case_id,
                            "inputVariantId": scope.input_variant_id
                        }),
                        baseline_id: "unused-test-baseline".to_owned(),
                        participant_ids: Vec::new(),
                        performance_ids: BTreeSet::new(),
                    },
                );
            }
        }
        let document = json!({
            "derived": {
                "formalStudyDisposition": "formal-analysis-available",
                "roundMetricSummaries": [],
                "candidateComparisons": []
            }
        });
        verify_candidate_performance_scope(
            &trusted,
            &document,
            &rosters,
            &BTreeMap::new(),
            &selected_scales,
        )
        .expect("exact candidate scope");

        let mut tampered = rosters;
        tampered
            .values_mut()
            .next()
            .expect("candidate roster")
            .stratum["inputVariantId"] = json!("different-input-v1");
        assert!(
            verify_candidate_performance_scope(
                &trusted,
                &document,
                &tampered,
                &BTreeMap::new(),
                &selected_scales,
            )
            .is_err()
        );
    }

    #[test]
    fn failure_input_verifier_rejects_an_unselected_private_limit_change() {
        let trusted = load_repository_contract().expect("trusted contract");
        let workload_id = ScalableWorkloadId::Identity;
        let graph_profile = GraphProfileId::WideStar;
        let n = 1;
        let pair = LimitQualificationPlanner::from_trusted_contract(&trusted)
            .expect("limit planner")
            .plan_pair(LimitDimensionId::SourceByteCount, graph_profile, n, None)
            .expect("source-byte pair");
        let plan = ScalableStagePlanFactory::from_trusted_contract(&trusted)
            .expect("stage plans")
            .plan(workload_id, graph_profile, n)
            .expect("stage plan");
        let case_id = "limit/source-byte-count/plus-one";
        let binding = crate::FailureInputDigestBinding {
            workload_manifest_sha256: &trusted.descriptor.workload_manifest.sha256,
            workload_id,
            workload_revision: crate::WORKLOAD_REVISION_V1,
            graph_profile,
            string_profile: crate::BASE_SCALE_STRING_PROFILE,
            generator_version: crate::GENERATOR_VERSION_V1,
            n,
            b: 1,
            scale_role: "calibration",
            case_id,
            input_variant_id: &pair.binding.input_variant_id,
            counts: &plan.counts,
            value_basis: "canonical-level-exact-value",
            basis_run_ids: &[],
        };
        let correct_digest = crate::input_digest::failure_input_digest_with_private_limits(
            &binding,
            Some((LimitDimensionId::SourceByteCount, pair.plus_one_limit_value)),
            &[
                ("exact-dimension-value", pair.exact_dimension_value),
                ("selected-limit-value", pair.plus_one_limit_value),
            ],
        );
        let mut run = json!({
            "runId": "failure-input/private-limit-binding",
            "workload": {
                "id": workload_id.as_str(),
                "revision": crate::WORKLOAD_REVISION_V1,
                "graphProfile": graph_profile.as_str(),
                "stringProfile": crate::BASE_SCALE_STRING_PROFILE,
                "generatorVersion": crate::GENERATOR_VERSION_V1,
                "n": n,
                "b": {"value": 1, "reason": null},
                "scaleRole": "calibration"
            },
            "failure": {
                "caseId": case_id,
                "dimensionId": LimitDimensionId::SourceByteCount.as_str(),
                "inputVariantId": pair.binding.input_variant_id,
                "inputDigest": correct_digest,
                "limitSelection": {
                    "exactDimensionValue": pair.exact_dimension_value,
                    "selectedLimitValue": pair.plus_one_limit_value,
                    "valueBasis": "canonical-level-exact-value",
                    "basisRunIds": []
                }
            }
        });
        let run_id = required_string(&run, "/runId").expect("run id").to_owned();
        let indexed = BTreeMap::from([(run_id.clone(), &run)]);
        assert_eq!(
            verify_failure_input_digests(&trusted, &indexed).expect("complete private limits"),
            1
        );

        let mut tampered_parameters = LimitDimensionId::ALL
            .into_iter()
            .map(|dimension| {
                let value = if dimension == LimitDimensionId::SourceByteCount {
                    pair.plus_one_limit_value
                } else if dimension == LimitDimensionId::ModuleCount {
                    u64::MAX - 1
                } else {
                    u64::MAX
                };
                (format!("private-limit/{}", dimension.as_str()), value)
            })
            .collect::<Vec<_>>();
        tampered_parameters.extend([
            (
                "exact-dimension-value".to_owned(),
                pair.exact_dimension_value,
            ),
            ("selected-limit-value".to_owned(), pair.plus_one_limit_value),
        ]);
        let borrowed = tampered_parameters
            .iter()
            .map(|(name, value)| (name.as_str(), *value))
            .collect::<Vec<_>>();
        run["failure"]["inputDigest"] =
            json!(crate::failure_input_digest_sha256(&binding, &borrowed,));
        let tampered = BTreeMap::from([(run_id, &run)]);
        assert!(matches!(
            verify_failure_input_digests(&trusted, &tampered),
            Err(EvidenceError::FailureInputRecomputation { .. })
        ));
    }

    #[test]
    fn limit_qualification_recomputes_all_pairs_and_independent_live_byte_replicas() {
        let trusted = load_repository_contract().expect("trusted contract");
        let document = json!({
            "derived": {"formalStudyDisposition": "formal-analysis-available"}
        });
        let runs = limit_qualification_runs(&trusted);
        let indexed = runs
            .iter()
            .map(|run| {
                (
                    required_string(run, "/runId").expect("run id").to_owned(),
                    run,
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut referenced = BTreeSet::new();
        let selected_scales = test_selected_scales();
        let (pair_count, baseline_count, duplicate_owner_count) = verify_limit_qualifications(
            &trusted,
            &document,
            &indexed,
            &selected_scales,
            &mut referenced,
        )
        .expect("limit qualification evidence");
        assert_eq!(pair_count, 138);
        assert_eq!(baseline_count, 12);
        assert_eq!(duplicate_owner_count, 6);
        assert_eq!(referenced.len(), 294);

        let mut wrong_scale = runs.clone();
        wrong_scale[0]["workload"]["n"] = json!(2);
        let wrong_scale_index = wrong_scale
            .iter()
            .map(|run| {
                (
                    required_string(run, "/runId").expect("run id").to_owned(),
                    run,
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert!(matches!(
            verify_limit_qualifications(
                &trusted,
                &document,
                &wrong_scale_index,
                &selected_scales,
                &mut BTreeSet::new()
            ),
            Err(EvidenceError::LimitRecomputation { .. })
        ));

        let mut tampered = runs;
        let plus_one = tampered
            .iter_mut()
            .find(|run| {
                required_string(run, "/failure/caseId")
                    .is_ok_and(|case_id| case_id.ends_with("/plus-one"))
            })
            .expect("plus-one run");
        plus_one["failure"]["limitSelection"]["selectedLimitValue"] = json!(u64::MAX);
        let indexed = tampered
            .iter()
            .map(|run| {
                (
                    required_string(run, "/runId").expect("run id").to_owned(),
                    run,
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert!(matches!(
            verify_limit_qualifications(
                &trusted,
                &document,
                &indexed,
                &selected_scales,
                &mut BTreeSet::new()
            ),
            Err(EvidenceError::BindingMismatch { pointer, .. })
                if pointer == "/failure/limitSelection/selectedLimitValue"
        ));
    }

    #[test]
    fn cleanup_verifier_rejects_retained_capacity_growth_after_repeated_failures() {
        let document = json!({
            "derived": {"formalStudyDisposition": "formal-analysis-available"}
        });
        let runs = cleanup_evidence_runs();
        let indexed = runs
            .iter()
            .map(|run| {
                (
                    required_string(run, "/runId").expect("run id").to_owned(),
                    run,
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut referenced = BTreeSet::new();
        let selected_scales = test_selected_scales();
        let (experiment_count, run_count) =
            verify_cleanup_experiments(&document, &indexed, &selected_scales, &mut referenced)
                .expect("cleanup evidence");
        assert_eq!(experiment_count, 6);
        assert_eq!(run_count, 210);
        assert_eq!(referenced.len(), 210);

        let mut wrong_scale = runs.clone();
        for run in wrong_scale.iter_mut().filter(|run| {
            nullable_string(run, "/cleanup/experimentId").is_ok_and(|value| {
                value.is_some_and(|experiment_id| experiment_id.contains("/calibration/"))
            })
        }) {
            run["workload"]["n"] = json!(2);
        }
        let wrong_scale_index = wrong_scale
            .iter()
            .map(|run| {
                (
                    required_string(run, "/runId").expect("run id").to_owned(),
                    run,
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert!(matches!(
            verify_cleanup_experiments(
                &document,
                &wrong_scale_index,
                &selected_scales,
                &mut BTreeSet::new()
            ),
            Err(EvidenceError::CleanupRecomputation { .. })
        ));

        let mut tampered = runs;
        let failure = tampered
            .iter_mut()
            .find(|run| {
                observed_u64(run, "/cleanup/sequenceIndex").is_ok_and(|value| value == 2)
                    && required_string(run, "/failure/caseId")
                        .is_ok_and(|value| value == "limit/source-byte-count/plus-one")
                    && required_string(run, "/workload/scaleRole")
                        .is_ok_and(|value| value == "calibration")
            })
            .expect("second failure");
        failure["metrics"]["retainedCapacityBytes"]["value"] = json!(101);
        let indexed = tampered
            .iter()
            .map(|run| {
                (
                    required_string(run, "/runId").expect("run id").to_owned(),
                    run,
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert!(matches!(
            verify_cleanup_experiments(&document, &indexed, &selected_scales, &mut BTreeSet::new()),
            Err(EvidenceError::CleanupRecomputation { .. })
        ));
    }

    #[test]
    fn scale_roles_are_recomputed_from_formal_levels_and_confirmed_knees() {
        let trusted = load_repository_contract().expect("trusted contract");
        let mut base_scales = Vec::new();
        let mut ladder_summaries = Vec::new();
        for workload_id in ScalableWorkloadId::ALL {
            for graph_profile in GraphProfileId::ALL {
                base_scales.push(json!({
                    "candidateId": "baseline-std-randomstate-stable-vec-v1",
                    "workloadId": workload_id.as_str(),
                    "graphProfile": graph_profile.as_str(),
                    "stringProfile": "short-unique-v1",
                    "b": {"value": 1, "reason": null}
                }));
                let knee_source = workload_id == ScalableWorkloadId::Identity
                    && graph_profile == GraphProfileId::WideStar;
                for n in [1_u32, 2, 4, 8, 16] {
                    let role = if knee_source {
                        match n {
                            1 => "base",
                            4 => "calibration",
                            8 => "stress",
                            _ => "ladder",
                        }
                    } else {
                        match n {
                            1 => "base",
                            8 => "calibration",
                            16 => "stress",
                            _ => "ladder",
                        }
                    };
                    for batch in 0..=1_u32 {
                        ladder_summaries.push(json!({
                            "candidateId": "baseline-std-randomstate-stable-vec-v1",
                            "batch": batch,
                            "stratum": {
                                "keyDomain": "full-pipeline-baseline",
                                "workloadId": workload_id.as_str(),
                                "graphProfile": graph_profile.as_str(),
                                "stringProfile": "short-unique-v1",
                                "n": n,
                                "b": {"value": 1, "reason": null},
                                "scaleRole": role
                            }
                        }));
                    }
                }
            }
        }
        let document = json!({
            "derived": {
                "formalStudyDisposition": "formal-analysis-available",
                "baseScales": base_scales,
                "ladderBatchSummaries": ladder_summaries,
                "knees": [{
                    "confirmedKnee": true,
                    "lowerStratum": {
                        "workloadId": "LF-COMP-ID-v1",
                        "graphProfile": "wide-star-v1",
                        "n": 4
                    },
                    "upperStratum": {
                        "workloadId": "LF-COMP-ID-v1",
                        "graphProfile": "wide-star-v1",
                        "n": 8
                    }
                }]
            }
        });
        let selected = recompute_selected_scales(&trusted, &document).expect("selected scales");
        assert_eq!(selected.len(), 18);
        assert_eq!(
            selected[&(
                ScalableWorkloadId::Identity,
                GraphProfileId::WideStar,
                "calibration".to_owned()
            )]
                .n,
            4
        );
        assert_eq!(
            selected[&(
                ScalableWorkloadId::Identity,
                GraphProfileId::WideStar,
                "stress".to_owned()
            )]
                .n,
            8
        );
        assert_eq!(
            selected[&(
                ScalableWorkloadId::Corridor,
                GraphProfileId::DeepChain,
                "calibration".to_owned()
            )]
                .n,
            8
        );
    }

    #[test]
    fn q16_growth_slope_uses_exact_integer_relationships() {
        assert_eq!(exact_slope_q16_16(1, 1, 2, 2).unwrap(), 65_536);
        assert_eq!(exact_slope_q16_16(1, 2, 2, 1).unwrap(), -65_536);
        assert_eq!(exact_slope_q16_16(1, 7, 8, 7).unwrap(), 0);
        assert_eq!(
            median_signed_slopes(&[0, 65_536, 131_072, 196_608]).unwrap(),
            SignedRatio {
                numerator: 98_304,
                denominator: 1,
            }
        );
        assert_eq!(
            upper_slope_bound(
                SignedRatio {
                    numerator: 1,
                    denominator: 1,
                },
                SignedRatio {
                    numerator: 3,
                    denominator: 2,
                },
            )
            .unwrap(),
            SignedRatio {
                numerator: 2,
                denominator: 1,
            }
        );
    }

    fn test_selected_scales() -> SelectedScaleMap {
        ScalableWorkloadId::ALL
            .into_iter()
            .flat_map(|workload_id| {
                GraphProfileId::ALL
                    .into_iter()
                    .flat_map(move |graph_profile| {
                        ["calibration", "stress"].into_iter().map(move |role| {
                            let scale = LimitEvidenceScale {
                                workload_id,
                                graph_profile,
                                scale_role: role.to_owned(),
                                n: 1,
                                b: 1,
                            };
                            ((workload_id, graph_profile, role.to_owned()), scale)
                        })
                    })
            })
            .collect()
    }

    fn limit_qualification_runs(trusted: &TrustedContract) -> Vec<Value> {
        let planner =
            LimitQualificationPlanner::from_trusted_contract(trusted).expect("limit planner");
        let mut runs = Vec::new();
        for workload_id in ScalableWorkloadId::ALL {
            for graph_profile in GraphProfileId::ALL {
                for scale_role in ["calibration", "stress"] {
                    let n = 1_u32;
                    let b = 1_u64;
                    let baseline = if workload_id == ScalableWorkloadId::Identity {
                        let baseline_ids = [0_u64, 1_u64].map(|replica| {
                            format!(
                                "limit-baseline/{}/{}/{scale_role}/n-{n}/replica-{replica}",
                                workload_id.as_str(),
                                graph_profile.as_str()
                            )
                        });
                        for (replica, run_id) in baseline_ids.iter().enumerate() {
                            runs.push(json!({
                                "runId": run_id,
                                "sampleKind": "limit-baseline",
                                "status": "valid",
                                "compilerInstanceId": {
                                    "value": format!("{run_id}/instance"),
                                    "reason": null
                                },
                                "candidate": {
                                    "id": "baseline-std-randomstate-stable-vec-v1",
                                    "keyDomain": "full-pipeline-baseline"
                                },
                                "workload": {
                                    "id": workload_id.as_str(),
                                    "graphProfile": graph_profile.as_str(),
                                    "stringProfile": "short-unique-v1",
                                    "caseId": "not-applicable",
                                    "scaleRole": scale_role,
                                    "n": n,
                                    "b": {"value": b, "reason": null}
                                },
                                "process": {
                                    "binaryId": ATTRIBUTION_BINARY_ID,
                                    "exitKind": "success",
                                    "childPid": {"value": 10_000 + replica, "reason": null}
                                },
                                "metrics": {
                                    "peakLiveRequestedBytes": {"value": 1_000_000, "reason": null}
                                },
                                "cleanup": {"phase": "not-applicable"},
                                "limitBaseline": {
                                    "measurementId": "compiler-controlled-live-byte-baseline-v1",
                                    "dimensionId": "compiler-controlled-live-byte-count",
                                    "privateLimitMode": "operational-hard-ceiling-only",
                                    "replicaIndex": replica
                                }
                            }));
                        }
                        Some(LiveByteBaseline {
                            replicas: [
                                LiveByteBaselineReplica {
                                    run_id: baseline_ids[0].clone(),
                                    workload_id,
                                    graph_profile,
                                    n,
                                    peak_live_requested_bytes: 1_000_000,
                                },
                                LiveByteBaselineReplica {
                                    run_id: baseline_ids[1].clone(),
                                    workload_id,
                                    graph_profile,
                                    n,
                                    peak_live_requested_bytes: 1_000_000,
                                },
                            ],
                        })
                    } else {
                        None
                    };

                    for binding in planner
                        .bindings()
                        .iter()
                        .filter(|binding| binding.workload_id == workload_id)
                    {
                        let pair = planner
                            .plan_pair(
                                binding.dimension_id,
                                graph_profile,
                                n,
                                if binding.dimension_id
                                    == LimitDimensionId::CompilerControlledLiveByteCount
                                {
                                    baseline.clone()
                                } else {
                                    None
                                },
                            )
                            .expect("limit pair");
                        for at_bound in [true, false] {
                            runs.push(limit_pair_evidence_run(
                                trusted,
                                workload_id,
                                graph_profile,
                                scale_role,
                                b,
                                &pair,
                                at_bound,
                            ));
                        }
                    }
                    if workload_id == ScalableWorkloadId::Corridor {
                        let case_id = "semantic/duplicate-owner-per-unit";
                        let input_variant_id = "corridor-duplicate-owner-per-unit-v1";
                        let run_id = format!(
                            "failure/semantic-duplicate-owner/{}/{scale_role}/n-{n}",
                            graph_profile.as_str()
                        );
                        let plan = ScalableStagePlanFactory::from_trusted_contract(trusted)
                            .expect("stage plans")
                            .plan(workload_id, graph_profile, n)
                            .expect("duplicate owner plan");
                        let binding = crate::FailureInputDigestBinding {
                            workload_manifest_sha256: &trusted.descriptor.workload_manifest.sha256,
                            workload_id,
                            workload_revision: crate::WORKLOAD_REVISION_V1,
                            graph_profile,
                            string_profile: crate::BASE_SCALE_STRING_PROFILE,
                            generator_version: crate::GENERATOR_VERSION_V1,
                            n,
                            b: u32::try_from(b).expect("test B fits u32"),
                            scale_role,
                            case_id,
                            input_variant_id,
                            counts: &plan.counts,
                            value_basis: "not-applicable",
                            basis_run_ids: &[],
                        };
                        runs.push(json!({
                            "runId": run_id,
                            "sampleKind": "failure",
                            "status": "valid",
                            "candidate": {
                                "id": "baseline-std-randomstate-stable-vec-v1",
                                "keyDomain": "full-pipeline-baseline"
                            },
                            "workload": {
                                "id": workload_id.as_str(),
                                "revision": crate::WORKLOAD_REVISION_V1,
                                "graphProfile": graph_profile.as_str(),
                                "stringProfile": "short-unique-v1",
                                "generatorVersion": crate::GENERATOR_VERSION_V1,
                                "caseId": "not-applicable",
                                "scaleRole": scale_role,
                                "n": n,
                                "b": {"value": b, "reason": null}
                            },
                            "process": {"binaryId": TIMING_BINARY_ID, "exitKind": "success"},
                            "cleanup": {"phase": "not-applicable"},
                            "metrics": {
                                "semanticDigest": {"value": null, "reason": "compiler-error"},
                                "liveRequestedBytes": {"value": 0, "reason": null}
                            },
                            "failure": {
                                "caseId": case_id,
                                "dimensionId": "not-applicable",
                                "inputVariantId": input_variant_id,
                                "inputDigest": crate::input_digest::failure_input_digest_with_private_limits(
                                    &binding,
                                    None,
                                    &[],
                                ),
                                "expectedOutcome": "compiler-error",
                                "actualOutcome": "compiler-error",
                                "stableCompilerErrorCode": {
                                    "value": DUPLICATE_OWNER_ERROR_CODE,
                                    "reason": null
                                },
                                "diagnosticCount": n,
                                "diagnosticsTruncated": false,
                                "partialOutputRecordCount": 0
                            }
                        }));
                    }
                }
            }
        }
        runs
    }

    fn limit_pair_evidence_run(
        trusted: &TrustedContract,
        workload_id: ScalableWorkloadId,
        graph_profile: GraphProfileId,
        scale_role: &str,
        b: u64,
        pair: &crate::LimitPairPlan,
        at_bound: bool,
    ) -> Value {
        let side = if at_bound { "at-bound" } else { "plus-one" };
        let binary_id = if pair.binding.pair_mode == LimitPairMode::BaselineLiveBytePrescanV1 {
            ATTRIBUTION_BINARY_ID
        } else {
            TIMING_BINARY_ID
        };
        let (outcome, error, diagnostic_count, truncated) = match pair.binding.pair_mode {
            LimitPairMode::SuccessAtBound | LimitPairMode::BaselineLiveBytePrescanV1
                if at_bound =>
            {
                ("success", None, 0, false)
            }
            LimitPairMode::SuccessAtBound | LimitPairMode::BaselineLiveBytePrescanV1 => {
                ("compiler-error", Some(LIMIT_EXCEEDED_ERROR_CODE), 1, false)
            }
            LimitPairMode::DiagnosticCapOnSemanticFailure if at_bound => (
                "compiler-error",
                Some(UNKNOWN_REFERENCE_ERROR_CODE),
                u64::from(pair.n),
                false,
            ),
            LimitPairMode::DiagnosticCapOnSemanticFailure => (
                "compiler-error",
                Some(DIAGNOSTIC_LIMIT_ERROR_CODE),
                u64::from(pair.n) - 1,
                true,
            ),
        };
        let value_basis = match pair.binding.pair_mode {
            LimitPairMode::SuccessAtBound => "canonical-level-exact-value",
            LimitPairMode::DiagnosticCapOnSemanticFailure => "diagnostic-input-count",
            LimitPairMode::BaselineLiveBytePrescanV1 => "baseline-live-byte-prescan-v1",
        };
        let run_id = format!(
            "limit/{}/{}/{scale_role}/n-{}/{}/{side}",
            workload_id.as_str(),
            graph_profile.as_str(),
            pair.n,
            pair.binding.dimension_id.as_str()
        );
        let selected_limit_value = if at_bound {
            pair.at_bound_limit_value
        } else {
            pair.plus_one_limit_value
        };
        let case_id = format!("limit/{}/{side}", pair.binding.dimension_id.as_str());
        let plan = ScalableStagePlanFactory::from_trusted_contract(trusted)
            .expect("stage plans")
            .plan(workload_id, graph_profile, pair.n)
            .expect("limit pair plan");
        let binding = crate::FailureInputDigestBinding {
            workload_manifest_sha256: &trusted.descriptor.workload_manifest.sha256,
            workload_id,
            workload_revision: crate::WORKLOAD_REVISION_V1,
            graph_profile,
            string_profile: crate::BASE_SCALE_STRING_PROFILE,
            generator_version: crate::GENERATOR_VERSION_V1,
            n: pair.n,
            b: u32::try_from(b).expect("test B fits u32"),
            scale_role,
            case_id: &case_id,
            input_variant_id: &pair.binding.input_variant_id,
            counts: &plan.counts,
            value_basis,
            basis_run_ids: &pair.basis_run_ids,
        };
        json!({
            "runId": run_id,
            "sampleKind": "failure",
            "status": "valid",
            "candidate": {
                "id": "baseline-std-randomstate-stable-vec-v1",
                "keyDomain": "full-pipeline-baseline"
            },
            "workload": {
                "id": workload_id.as_str(),
                "revision": crate::WORKLOAD_REVISION_V1,
                "graphProfile": graph_profile.as_str(),
                "stringProfile": "short-unique-v1",
                "generatorVersion": crate::GENERATOR_VERSION_V1,
                "caseId": "not-applicable",
                "scaleRole": scale_role,
                "n": pair.n,
                "b": {"value": b, "reason": null}
            },
            "process": {"binaryId": binary_id, "exitKind": "success"},
            "cleanup": {"phase": "not-applicable"},
            "failure": {
                "caseId": case_id,
                "dimensionId": pair.binding.dimension_id.as_str(),
                "inputVariantId": pair.binding.input_variant_id,
                "inputDigest": crate::input_digest::failure_input_digest_with_private_limits(
                    &binding,
                    Some((pair.binding.dimension_id, selected_limit_value)),
                    &[
                        ("exact-dimension-value", pair.exact_dimension_value),
                        ("selected-limit-value", selected_limit_value),
                    ],
                ),
                "expectedOutcome": outcome,
                "actualOutcome": outcome,
                "stableCompilerErrorCode": {"value": error, "reason": null},
                "diagnosticCount": diagnostic_count,
                "diagnosticsTruncated": truncated,
                "partialOutputRecordCount": 0,
                "limitSelection": {
                    "exactDimensionValue": pair.exact_dimension_value,
                    "selectedLimitValue": selected_limit_value,
                    "valueBasis": value_basis,
                    "basisRunIds": pair.basis_run_ids
                }
            }
        })
    }

    fn cleanup_evidence_runs() -> Vec<Value> {
        let mut runs = Vec::new();
        for case_id in [
            "limit/source-byte-count/plus-one",
            "semantic/missing-reference-per-unit",
            "diagnostic/cap-plus-one",
        ] {
            for scale_role in ["calibration", "stress"] {
                let workload_id = if case_id == "limit/source-byte-count/plus-one" {
                    "LF-COMP-ID-v1"
                } else {
                    "LF-COMP-CORRIDOR-v1"
                };
                let n = 1_u64;
                let experiment_id = format!("cleanup/{case_id}/{scale_role}/n-{n}");
                for sequence in 0..=34_u64 {
                    let phase = match sequence {
                        0 => "baseline-success",
                        1..=32 => "failure-iteration",
                        33 => "post-recovery-success",
                        34 => "fresh-instance-oracle",
                        _ => unreachable!(),
                    };
                    let sample_kind = match sequence {
                        0 | 34 => "cold-instance",
                        1..=32 => "failure",
                        33 => "stable-capacity-reuse",
                        _ => unreachable!(),
                    };
                    let instance = if sequence == 34 {
                        format!("{experiment_id}/fresh-instance")
                    } else {
                        format!("{experiment_id}/primary-instance")
                    };
                    let mut run = json!({
                        "runId": format!("{experiment_id}/sequence-{sequence}"),
                        "roundAttempt": {"scope": "single-experiment"},
                        "compilerInstanceId": {"value": instance, "reason": null},
                        "sampleKind": sample_kind,
                        "status": "valid",
                        "candidate": {
                            "id": "baseline-std-randomstate-stable-vec-v1",
                            "keyDomain": "full-pipeline-baseline"
                        },
                        "workload": {
                            "id": workload_id,
                            "graphProfile": "shared-fanin-dag-v1",
                            "stringProfile": "short-unique-v1",
                            "caseId": "not-applicable",
                            "scaleRole": scale_role,
                            "n": n,
                            "b": {"value": 1, "reason": null}
                        },
                        "process": {"binaryId": ATTRIBUTION_BINARY_ID, "exitKind": "success"},
                        "cleanup": {
                            "experimentId": {"value": experiment_id, "reason": null},
                            "sequenceIndex": {"value": sequence, "reason": null},
                            "phase": phase
                        },
                        "metrics": {
                            "stageBreakdown": {},
                            "semanticDigest": {"value": "a".repeat(64), "reason": null},
                            "diagnosticDigest": {"value": "b".repeat(64), "reason": null},
                            "retainedCapacityBytes": {"value": 100, "reason": null},
                            "liveRequestedBytes": {"value": 0, "reason": null}
                        }
                    });
                    if (1..=32).contains(&sequence) {
                        let (dimension, variant, error, diagnostic_count, truncated) = match case_id
                        {
                            "limit/source-byte-count/plus-one" => (
                                "source-byte-count",
                                "canonical-valid-v1",
                                LIMIT_EXCEEDED_ERROR_CODE,
                                1,
                                false,
                            ),
                            "semantic/missing-reference-per-unit" => (
                                "not-applicable",
                                "corridor-missing-reference-per-unit-v1",
                                UNKNOWN_REFERENCE_ERROR_CODE,
                                n,
                                false,
                            ),
                            "diagnostic/cap-plus-one" => (
                                "not-applicable",
                                "corridor-diagnostic-cap-plus-one-v1",
                                DIAGNOSTIC_LIMIT_ERROR_CODE,
                                n,
                                true,
                            ),
                            _ => unreachable!(),
                        };
                        run["metrics"]["semanticDigest"] =
                            json!({"value": null, "reason": "compiler-error"});
                        run["failure"] = json!({
                            "caseId": case_id,
                            "dimensionId": dimension,
                            "inputVariantId": variant,
                            "expectedOutcome": "compiler-error",
                            "actualOutcome": "compiler-error",
                            "stableCompilerErrorCode": {"value": error, "reason": null},
                            "diagnosticCount": diagnostic_count,
                            "diagnosticsTruncated": truncated,
                            "partialOutputRecordCount": 0,
                            "inputDigest": "c".repeat(64)
                        });
                    }
                    runs.push(run);
                }
            }
        }
        runs
    }

    fn minimal_guarded_evidence(trusted: &TrustedContract, context: &VerificationContext) -> Value {
        let physical_memory_bytes = 4 * 1_073_741_824_u64;
        let available_physical_memory_bytes = 1_073_741_824_u64 - 1;
        let plan = ScalableStagePlanFactory::from_trusted_contract(trusted)
            .expect("plan factory")
            .plan(ScalableWorkloadId::Identity, GraphProfileId::WideStar, 1)
            .expect("identity plan");
        let mut counts = serde_json::to_value(&plan.counts)
            .expect("serialize counts")
            .as_object()
            .expect("counts object")
            .clone();
        merge_per_unit_counts(trusted, "LF-COMP-ID-v1", 1, &mut counts);
        let stage_breakdown = stage_breakdown_json(&plan.stages, "child-not-started");
        let guard_report = ScalableGuardPlanner::from_trusted_contract(trusted)
            .expect("guard planner")
            .evaluate(
                ScalableWorkloadId::Identity,
                GraphProfileId::WideStar,
                1,
                SystemMemoryObservation {
                    physical_memory_bytes,
                    available_physical_memory_bytes,
                },
                None,
            )
            .expect("guard report");
        assert!(!guard_report.allows_child_start);
        assert_eq!(
            guard_report.triggers,
            vec![crate::GuardTrigger::AvailablePhysicalMemory]
        );
        let candidate_bindings = all_candidate_bindings(trusted, context, "rustc-test");
        let candidate = candidate_bindings
            .iter()
            .find(|candidate| {
                candidate["id"] == "baseline-std-randomstate-stable-vec-v1"
                    && candidate["keyDomain"] == "full-pipeline-baseline"
            })
            .expect("baseline candidate binding")
            .clone();
        let run_id = "guard/LF-COMP-ID-v1/wide-star-v1/n-1";
        json!({
            "schema": EVIDENCE_SCHEMA_ID,
            "schemaVersion": EVIDENCE_SCHEMA_VERSION,
            "source": {
                "sourceCommit": context.repository_head,
                "harnessCommit": context.repository_head,
                "dirty": false,
                "cargoLockSha256": context.cargo_lock_sha256,
                "contractDescriptorId": trusted.descriptor.schema,
                "contractDescriptorVersion": trusted.descriptor.schema_version,
                "contractDescriptorSha256": trusted.descriptor_sha256,
                "workloadManifestSha256": trusted.descriptor.workload_manifest.sha256,
                "evidenceSchemaSha256": trusted.descriptor.evidence_schema.sha256
            },
            "environment": {
                "os": "test-os",
                "osBuild": "test-build",
                "cpu": "test-cpu",
                "logicalProcessorCount": 1,
                "physicalMemoryBytes": physical_memory_bytes,
                "targetTriple": "x86_64-pc-windows-msvc",
                "rustc": "rustc-test",
                "llvm": "llvm-test",
                "powerSource": "ac",
                "vendorPerformanceMode": "test-performance-mode",
                "powerPlan": "test-power-plan",
                "biosFirmware": "test-bios",
                "monitoringProvider": "test-monitor-v1",
                "backgroundProcessAudit": []
            },
            "protocol": {
                "id": "compiler-calibration-v1",
                "workloadSeedHexU64": "4c46434f4d500001",
                "clockQuantumNs": 1,
                "batchCount": 2,
                "candidateOrderDesign": "forward-reverse-cyclic-2c-v1",
                "guardThresholds": guard_report.thresholds
            },
            "binaries": [
                {
                    "id": TIMING_BINARY_ID,
                    "mode": "timing",
                    "sha256": "1".repeat(64),
                    "cargoProfile": "release",
                    "features": ["research-runner-full"]
                },
                {
                    "id": ATTRIBUTION_BINARY_ID,
                    "mode": "attribution",
                    "sha256": "2".repeat(64),
                    "cargoProfile": "release",
                    "features": ["research-runner-full"]
                },
                {
                    "id": ORACLE_BINARY_ID,
                    "mode": "oracle",
                    "sha256": "3".repeat(64),
                    "cargoProfile": "release",
                    "features": ["research-runner-full"]
                }
            ],
            "candidateBindings": candidate_bindings,
            "runs": [{
                "runId": run_id,
                "batch": 0,
                "round": 0,
                "position": 0,
                "roundAttempt": {"id": "attempt/guard-0", "ordinal": 0, "scope": "single-experiment"},
                "compilerInstanceId": {"value": null, "reason": "compiler-instance-not-created"},
                "sampleOrdinal": 0,
                "sampleKind": "guard-preflight",
                "status": "guarded",
                "invalidationReasons": [],
                "workload": {
                    "id": "LF-COMP-ID-v1",
                    "revision": 1,
                    "graphProfile": "wide-star-v1",
                    "stringProfile": "short-unique-v1",
                    "generatorVersion": 1,
                    "n": 1,
                    "b": {"value": null, "reason": "no-reliable-base-scale-before-guard"},
                    "scaleRole": "pilot",
                    "caseId": "not-applicable",
                    "manifestDigest": trusted.descriptor.workload_manifest.sha256,
                    "counts": counts,
                    "fixtureInputs": []
                },
                "candidate": candidate,
                "process": {
                    "coordinatorPid": 1,
                    "childPid": {"value": null, "reason": "child-not-started"},
                    "binaryId": TIMING_BINARY_ID,
                    "exitKind": "guarded-before-start",
                    "exitCode": {"value": null, "reason": "child-not-started"},
                    "termination": {
                        "kind": "not-started",
                        "signalNumber": {"value": null, "reason": "child-not-started"},
                        "rawPlatformStatus": {"value": null, "reason": "child-not-started"}
                    }
                },
                "metrics": {
                    "wallTimeNs": null_observation("child-not-started"),
                    "allocationCount": null_observation("child-not-started"),
                    "reallocationCount": null_observation("child-not-started"),
                    "allocatedBytes": null_observation("child-not-started"),
                    "freedBytes": null_observation("child-not-started"),
                    "liveRequestedBytes": null_observation("child-not-started"),
                    "peakLiveRequestedBytes": null_observation("child-not-started"),
                    "retainedCapacityBytes": null_observation("child-not-started"),
                    "workingSetBytes": null_observation("child-not-started"),
                    "privateBytes": null_observation("child-not-started"),
                    "commitPeakBytes": null_observation("child-not-started"),
                    "semanticDigest": null_observation("child-not-started"),
                    "diagnosticDigest": null_observation("child-not-started"),
                    "stageBreakdown": stage_breakdown
                },
                "guard": {
                    "compilerControlledPredictionBasis": guard_report.compiler_controlled_prediction_basis,
                    "privateBytesPredictionBasis": guard_report.private_bytes_prediction_basis,
                    "wallTimePredictionBasis": guard_report.wall_time_prediction_basis,
                    "previousCompletedN": null_observation("first-level-no-completed-level"),
                    "previousPrimaryRecordCount": null_observation("first-level-no-completed-level"),
                    "nextPrimaryRecordCount": guard_report.primary_record_count,
                    "previousPeakLiveRequestedBytes": null_observation("first-level-no-completed-level"),
                    "predictedCompilerControlledBytes": {"value": guard_report.predicted_compiler_controlled_bytes, "reason": null},
                    "previousPrivateBytes": null_observation("first-level-no-completed-level"),
                    "predictedPrivateBytes": null_observation("first-level-monitor-only"),
                    "previousWallTimeNs": null_observation("first-level-no-completed-level"),
                    "predictedWallTimeNs": null_observation("first-level-monitor-only"),
                    "logicalBytesLowerBound": guard_report.logical_bytes_lower_bound,
                    "reservedBytesBeforeFailure": 0,
                    "trigger": "available-physical-memory",
                    "lastAvailablePhysicalMemoryBytes": available_physical_memory_bytes,
                    "lastPrivateBytes": null_observation("child-not-started")
                },
                "cleanup": {
                    "experimentId": null_observation("not-applicable"),
                    "phase": "not-applicable",
                    "sequenceIndex": null_observation("not-applicable")
                },
                "externalState": {
                    "powerSource": "ac",
                    "vendorPerformanceMode": "test-performance-mode",
                    "powerPlan": "test-power-plan",
                    "sleepOrSessionLock": false,
                    "thermalOrPowerThrottling": false,
                    "backgroundCpuTimeNs": {"value": 0, "reason": null},
                    "backgroundWriteBytes": {"value": 0, "reason": null},
                    "monitoringGap": false,
                    "backgroundProcessDeltas": []
                }
            }],
            "derived": {
                "formalStudyDisposition": "no-reliable-base-scale",
                "baseScales": [{
                    "candidateId": "baseline-std-randomstate-stable-vec-v1",
                    "workloadId": "LF-COMP-ID-v1",
                    "workloadRevision": 1,
                    "graphProfile": "wide-star-v1",
                    "stringProfile": "short-unique-v1",
                    "generatorVersion": 1,
                    "selectionRule": "first-power-of-two-clock-qualified-seven-pilot-runs-v2",
                    "pilotLevels": [],
                    "b": {"value": null, "reason": "no-reliable-base-scale-before-guard"},
                    "terminalGuardRunId": {"value": run_id, "reason": null}
                }],
                "constantHashQualifications": [],
                "roundMetricSummaries": [],
                "ladderBatchSummaries": [],
                "adjacentLevelRatios": [],
                "knees": [],
                "reproducibilityEnvelopes": [],
                "growthSlopes": [],
                "candidateRosters": [],
                "candidateComparisons": [],
                "recommendations": []
            },
            "artifacts": []
        })
    }

    fn all_candidate_bindings(
        trusted: &TrustedContract,
        context: &VerificationContext,
        rustc: &str,
    ) -> Vec<Value> {
        let candidates = trusted.workload_manifest["candidateRegistry"]["candidates"]
            .as_array()
            .expect("candidate registry");
        let mut bindings = Vec::new();
        for candidate in candidates {
            let candidate_id = candidate["id"].as_str().expect("candidate id");
            let components = candidate["components"]
                .as_array()
                .expect("candidate components")
                .iter()
                .map(|component| {
                    let dependency_kind = component["dependencyKind"]
                        .as_str()
                        .expect("dependency kind");
                    let (version, features, audit) = match dependency_kind {
                        "standard-library" => (
                            rustc.to_owned(),
                            Vec::new(),
                            not_applicable_dependency_audit(&context.cargo_lock_sha256),
                        ),
                        "local-workspace" => (
                            context.repository_head.clone(),
                            Vec::new(),
                            not_applicable_dependency_audit(&context.cargo_lock_sha256),
                        ),
                        "crates-io" | "git" => {
                            let dependency_source = component["dependencySource"]
                                .as_str()
                                .expect("dependency source");
                            let package_name = dependency_source
                                .rsplit('/')
                                .next()
                                .expect("dependency package name");
                            let locked = context
                                .direct_cargo_packages
                                .get(package_name)
                                .expect("direct cargo package");
                            (
                                locked.version.clone(),
                                locked.features.iter().cloned().collect(),
                                unavailable_dependency_audit(&context.cargo_lock_sha256, locked),
                            )
                        }
                        other => panic!("unexpected dependency kind {other}"),
                    };
                    json!({
                        "role": component["role"],
                        "implementationId": component["implementationId"],
                        "version": version,
                        "features": features,
                        "dependencyKind": dependency_kind,
                        "dependencySource": component["dependencySource"],
                        "dependencyAudit": audit
                    })
                })
                .collect::<Vec<_>>();
            for key_domain in candidate["allowedKeyDomains"]
                .as_array()
                .expect("allowed key domains")
            {
                let policy = candidate["hasherSeedPolicy"]
                    .as_str()
                    .expect("hasher seed policy");
                let hasher_seed = match policy {
                    "fixed-u64" => json!({
                        "value": candidate["fixedHasherSeedHexU64"],
                        "reason": null
                    }),
                    "random-state-process-random" => {
                        null_observation("process-random-not-recorded")
                    }
                    "not-applicable" => null_observation("not-applicable"),
                    other => panic!("unexpected hasher seed policy {other}"),
                };
                bindings.push(json!({
                    "registryRevision": 1,
                    "id": candidate_id,
                    "keyDomain": key_domain,
                    "components": components,
                    "hasherSeedPolicy": policy,
                    "hasherSeedHexU64": hasher_seed
                }));
            }
        }
        bindings
    }

    fn qualified_base_evidence(trusted: &TrustedContract, context: &VerificationContext) -> Value {
        let mut document = minimal_guarded_evidence(trusted, context);
        let template = document["runs"][0].clone();
        let digest = "2".repeat(64);
        let wall_times = [1_000_u64, 2_000, 3_000, 10_003, 20_000, 30_000, 40_000];
        let attempt_id = "attempt/pilot/LF-COMP-ID-v1/wide-star-v1/n-1/0";
        let mut contributing = Vec::new();
        let mut runs = Vec::new();
        for (position, wall_time) in wall_times.into_iter().enumerate() {
            let run_id = format!("pilot/LF-COMP-ID-v1/wide-star-v1/n-1/timing-{position}");
            contributing.push(run_id.clone());
            runs.push(started_run(
                template.clone(),
                &run_id,
                attempt_id,
                position as u64,
                TIMING_BINARY_ID,
                "cold-instance",
                Some(wall_time),
                &digest,
            ));
        }
        let oracle_id = "pilot/LF-COMP-ID-v1/wide-star-v1/n-1/oracle";
        let mut oracle = started_run(
            template,
            oracle_id,
            "attempt/pilot/LF-COMP-ID-v1/wide-star-v1/n-1/oracle",
            0,
            ORACLE_BINARY_ID,
            "oracle",
            None,
            &digest,
        );
        oracle["compilerInstanceId"] = null_observation("not-applicable-oracle");
        runs.push(oracle);
        document["runs"] = Value::Array(runs);
        document["derived"]["formalStudyDisposition"] = json!("insufficient-formal-ladder");
        document["derived"]["baseScales"][0]["pilotLevels"] = json!([{
            "n": 1,
            "contributingRunIds": contributing,
            "aggregationMethod": "median-and-mad-of-seven-exact-integers-v1",
            "wallTimeMedianNs": 10_003,
            "wallTimeMedianAbsoluteDeviationNs": 9_003,
            "minimumReliableWallTimeNs": 10_000,
            "semanticDigest": digest,
            "allSemanticDigestsEqual": true,
            "allGuardsClear": true,
            "qualifies": true
        }]);
        document["derived"]["baseScales"][0]["b"] = json!({"value": 1, "reason": null});
        document["derived"]["baseScales"][0]["terminalGuardRunId"] =
            null_observation("base-scale-selected");
        document
    }

    #[cfg(feature = "research-runner-full")]
    fn constant_hash_qualification_evidence(
        trusted: &TrustedContract,
        context: &VerificationContext,
    ) -> Value {
        let mut document = minimal_guarded_evidence(trusted, context);
        let template = document["runs"][0].clone();
        let candidate_id = "hashbrown-xxh3-fixed-v1";
        let qualification =
            qualify_constant_hash_candidate(trusted, candidate_id).expect("qualification");
        let plan = ScalableStagePlanFactory::from_trusted_contract(trusted)
            .expect("plan factory")
            .plan(ScalableWorkloadId::Corridor, GraphProfileId::WideStar, 1)
            .expect("corridor plan");
        let mut counts = serde_json::to_value(&plan.counts)
            .expect("serialize counts")
            .as_object()
            .expect("counts object")
            .clone();
        merge_per_unit_counts(trusted, "LF-COMP-CORRIDOR-v1", 1, &mut counts);
        let candidate = document["candidateBindings"]
            .as_array()
            .expect("candidate bindings")
            .iter()
            .find(|binding| {
                binding["id"] == candidate_id && binding["keyDomain"] == "validated-fixed-key"
            })
            .expect("constant hash candidate binding")
            .clone();
        let oracle_candidate = document["candidateBindings"]
            .as_array()
            .expect("candidate bindings")
            .iter()
            .find(|binding| {
                binding["id"] == "baseline-std-randomstate-stable-vec-v1"
                    && binding["keyDomain"] == "full-pipeline-baseline"
            })
            .expect("oracle candidate binding")
            .clone();
        let mut canonical_candidates = Vec::new();
        let mut canonical_oracle = None;
        let mut missing_candidates = Vec::new();
        let mut missing_oracle = None;
        let mut runs = vec![template.clone()];
        for observation in &qualification.observations {
            let role = match observation.role {
                ConstantHashRole::CandidateUnderTest => "candidate-collision-builder",
                ConstantHashRole::ExactResearchOracle => "exact-oracle",
            };
            let builder = match observation.role {
                ConstantHashRole::CandidateUnderTest => "all-keys-u64-zero-v1",
                ConstantHashRole::ExactResearchOracle => "exact-research-oracle-v1",
            };
            let run_id = format!("correctness/{}", observation.observation_id);
            match (observation.input_variant_id.as_str(), observation.role) {
                ("constant-hash-canonical-valid-v1", ConstantHashRole::CandidateUnderTest) => {
                    canonical_candidates.push(run_id.clone());
                }
                ("constant-hash-canonical-valid-v1", ConstantHashRole::ExactResearchOracle) => {
                    canonical_oracle = Some(run_id.clone());
                }
                ("constant-hash-missing-reference-v1", ConstantHashRole::CandidateUnderTest) => {
                    missing_candidates.push(run_id.clone());
                }
                ("constant-hash-missing-reference-v1", ConstantHashRole::ExactResearchOracle) => {
                    missing_oracle = Some(run_id.clone());
                }
                (variant, _) => panic!("unexpected constant hash variant {variant}"),
            }
            let mut run = started_run(
                template.clone(),
                &run_id,
                &format!("attempt/{run_id}"),
                u64::from(observation.repeat),
                ORACLE_BINARY_ID,
                "correctness",
                None,
                &observation.semantic_digest_sha256,
            );
            run["workload"] = json!({
                "id": "LF-COMP-CORRIDOR-v1",
                "revision": 1,
                "graphProfile": "wide-star-v1",
                "stringProfile": "short-unique-v1",
                "generatorVersion": 1,
                "n": 1,
                "b": null_observation("not-applicable-correctness-qualification"),
                "scaleRole": "known-vector",
                "caseId": "not-applicable",
                "manifestDigest": trusted.descriptor.workload_manifest.sha256,
                "counts": counts,
                "fixtureInputs": []
            });
            run["candidate"] = if observation.role == ConstantHashRole::CandidateUnderTest {
                candidate.clone()
            } else {
                oracle_candidate.clone()
            };
            run["metrics"]["diagnosticDigest"] =
                json!({"value": observation.diagnostic_digest_sha256, "reason": null});
            run["metrics"]["stageBreakdown"] =
                stage_breakdown_json(&plan.stages, "not-measured-by-correctness-run");
            let (expected_outcome, stable_error, diagnostic_count) =
                if observation.outcome == ConstantHashOutcome::Success {
                    ("success", null_observation("no-compiler-error-expected"), 0)
                } else {
                    (
                        "compiler-error",
                        json!({"value": "LF-COMP-RESEARCH-E-UNKNOWN-REFERENCE", "reason": null}),
                        1,
                    )
                };
            run["correctnessQualification"] = json!({
                "qualificationId": qualification.qualification_id,
                "protocol": qualification.protocol_id,
                "role": role,
                "builder": builder,
                "candidateUnderTestId": candidate_id,
                "inputVariantId": observation.input_variant_id,
                "repeatIndex": observation.repeat,
                "expectedOutcome": expected_outcome,
                "actualOutcome": expected_outcome,
                "expectedStableCompilerErrorCode": stable_error,
                "actualStableCompilerErrorCode": stable_error,
                "expectedDiagnosticCount": diagnostic_count,
                "actualDiagnosticCount": diagnostic_count,
                "expectedDiagnosticsTruncated": false,
                "actualDiagnosticsTruncated": false,
                "expectedPartialOutputRecordCount": 0,
                "actualPartialOutputRecordCount": observation.partial_output_record_count
            });
            runs.push(run);
        }
        document["runs"] = Value::Array(runs);
        document["derived"]["constantHashQualifications"] = json!([{
            "qualificationId": qualification.qualification_id,
            "candidateId": candidate_id,
            "protocol": qualification.protocol_id,
            "candidateBuilder": qualification.candidate_builder_id,
            "oracleBuilder": qualification.oracle_builder_id,
            "canonicalValidCandidateRunIds": canonical_candidates,
            "canonicalValidOracleRunId": canonical_oracle.expect("canonical oracle"),
            "missingReferenceCandidateRunIds": missing_candidates,
            "missingReferenceOracleRunId": missing_oracle.expect("missing oracle"),
            "allStageCountsMatchOracle": true,
            "semanticDigestsMatchOracle": true,
            "diagnosticDigestsMatchOracle": true,
            "candidateRepeatsDeterministic": true,
            "stableOutcomesMatchOracle": true,
            "partialOutputCountsMatchOracle": true,
            "passed": true
        }]);
        document
    }

    fn candidate_roster_evidence(
        trusted: &TrustedContract,
        context: &VerificationContext,
    ) -> Value {
        let mut document = minimal_guarded_evidence(trusted, context);
        let template = document["runs"][0].clone();
        let candidate_bindings = document["candidateBindings"]
            .as_array()
            .expect("candidate bindings");
        let candidate_snapshot = |candidate_id: &str, key_domain: &str| {
            candidate_bindings
                .iter()
                .find(|binding| binding["id"] == candidate_id && binding["keyDomain"] == key_domain)
                .unwrap_or_else(|| panic!("missing binding {candidate_id}/{key_domain}"))
                .clone()
        };
        let digest = "a".repeat(64);
        let mut std_run = started_run(
            template.clone(),
            "correctness/std",
            "attempt/correctness/std",
            0,
            TIMING_BINARY_ID,
            "candidate-qualification",
            Some(10_000),
            &digest,
        );
        std_run["candidate"] = candidate_snapshot("std-hashmap-randomstate-v1", "external-string");
        let mut sorted_run = started_run(
            template.clone(),
            "correctness/sorted",
            "attempt/correctness/sorted",
            1,
            TIMING_BINARY_ID,
            "candidate-qualification",
            Some(10_000),
            &digest,
        );
        sorted_run["candidate"] =
            candidate_snapshot("sorted-vec-binary-search-v1", "external-string");
        let mut oracle_run = started_run(
            template,
            "correctness/oracle",
            "attempt/correctness/oracle",
            0,
            ORACLE_BINARY_ID,
            "candidate-qualification",
            None,
            &digest,
        );
        oracle_run["candidate"] = candidate_snapshot(
            "baseline-std-randomstate-stable-vec-v1",
            "full-pipeline-baseline",
        );
        for run in [&mut std_run, &mut sorted_run, &mut oracle_run] {
            run["workload"]["b"] = json!({"value": 1, "reason": null});
            run["workload"]["scaleRole"] = json!("calibration");
            run["workload"]["inputVariantId"] = json!("canonical-valid-v1");
        }
        document["runs"]
            .as_array_mut()
            .expect("runs")
            .extend([std_run, sorted_run, oracle_run]);
        let stratum = json!({
            "keyDomain": "external-string",
            "workloadId": "LF-COMP-ID-v1",
            "workloadRevision": 1,
            "graphProfile": "wide-star-v1",
            "stringProfile": "short-unique-v1",
            "generatorVersion": 1,
            "n": 1,
            "b": {"value": 1, "reason": null},
            "scaleRole": "calibration",
            "caseId": "not-applicable",
            "inputVariantId": "canonical-valid-v1"
        });
        document["derived"]["candidateRosters"] = json!([{
            "rosterId": "roster/external-string",
            "stratum": stratum,
            "baselineId": "std-hashmap-randomstate-v1",
            "entries": [
                {
                    "candidateId": "std-hashmap-randomstate-v1",
                    "disposition": "baseline-participant",
                    "correctnessEvidenceRunIds": ["correctness/std", "correctness/oracle"],
                    "constantHashQualificationId": null_observation("not-applicable-non-fast-hash-candidate")
                },
                {
                    "candidateId": "hashbrown-randomstate-v1",
                    "disposition": "insufficient-qualification-evidence",
                    "correctnessEvidenceRunIds": [],
                    "constantHashQualificationId": null_observation("not-applicable-non-fast-hash-candidate")
                },
                {
                    "candidateId": "sorted-vec-binary-search-v1",
                    "disposition": "performance-participant",
                    "correctnessEvidenceRunIds": ["correctness/sorted", "correctness/oracle"],
                    "constantHashQualificationId": null_observation("not-applicable-non-fast-hash-candidate")
                }
            ]
        }]);
        document
    }

    #[cfg(feature = "fixture-oracle")]
    fn current_fixture_evidence(trusted: &TrustedContract, context: &VerificationContext) -> Value {
        let mut document = minimal_guarded_evidence(trusted, context);
        let template = document["runs"][0].clone();
        let baseline_candidate = document["candidateBindings"]
            .as_array()
            .expect("candidate bindings")
            .iter()
            .find(|binding| {
                binding["id"] == "baseline-std-randomstate-stable-vec-v1"
                    && binding["keyDomain"] == "full-pipeline-baseline"
            })
            .expect("baseline binding")
            .clone();
        let summaries = build_current_fixture_summaries(trusted).expect("fixture summaries");
        let fixture_workload = trusted.workload_manifest["workloads"]
            .as_array()
            .expect("workloads")
            .iter()
            .find(|workload| workload["id"] == "LF-COMP-RESEARCH-CURRENT-FIXTURES-v1")
            .expect("fixture workload");
        for (position, summary) in summaries.iter().enumerate() {
            let case = fixture_workload["cases"]
                .as_array()
                .expect("fixture cases")
                .iter()
                .find(|case| case["id"] == summary.case_id)
                .expect("fixture case");
            let mut counts = serde_json::to_value(&summary.counts)
                .expect("serialize fixture counts")
                .as_object()
                .expect("fixture count object")
                .clone();
            counts.extend(
                summary
                    .entity_counts
                    .iter()
                    .map(|(name, value)| (name.clone(), json!(value))),
            );
            counts.extend(
                summary
                    .relation_record_counts
                    .iter()
                    .map(|(name, value)| (name.clone(), json!(value))),
            );
            let run_id = format!("fixture/{}", summary.case_id);
            let mut run = started_run(
                template.clone(),
                &run_id,
                &format!("attempt/{run_id}"),
                u64::try_from(position).expect("fixture position"),
                ORACLE_BINARY_ID,
                "oracle",
                None,
                &summary.semantic_digest_sha256,
            );
            run["candidate"] = baseline_candidate.clone();
            run["workload"] = json!({
                "id": "LF-COMP-RESEARCH-CURRENT-FIXTURES-v1",
                "revision": 1,
                "graphProfile": "not-applicable",
                "stringProfile": "not-applicable",
                "generatorVersion": 1,
                "n": 1,
                "b": null_observation("not-applicable-current-fixture"),
                "scaleRole": "current-fixture",
                "caseId": summary.case_id,
                "manifestDigest": trusted.descriptor.workload_manifest.sha256,
                "counts": counts,
                "fixtureInputs": case["files"]
            });
            run["metrics"]["stageBreakdown"] =
                stage_breakdown_json(&summary.stages, "not-measured-by-fixture-oracle");
            document["runs"].as_array_mut().expect("runs").push(run);
        }
        document
    }

    fn candidate_comparison_evidence(
        trusted: &TrustedContract,
        context: &VerificationContext,
    ) -> Value {
        let mut document = candidate_roster_evidence(trusted, context);
        let template = document["runs"][0].clone();
        let candidate_bindings = document["candidateBindings"]
            .as_array()
            .expect("candidate bindings");
        let std_candidate = candidate_bindings
            .iter()
            .find(|binding| {
                binding["id"] == "std-hashmap-randomstate-v1"
                    && binding["keyDomain"] == "external-string"
            })
            .expect("std candidate")
            .clone();
        let sorted_candidate = candidate_bindings
            .iter()
            .find(|binding| {
                binding["id"] == "sorted-vec-binary-search-v1"
                    && binding["keyDomain"] == "external-string"
            })
            .expect("sorted candidate")
            .clone();
        let stratum = json!({
            "keyDomain": "external-string",
            "workloadId": "LF-COMP-ID-v1",
            "workloadRevision": 1,
            "graphProfile": "wide-star-v1",
            "stringProfile": "short-unique-v1",
            "generatorVersion": 1,
            "n": 1,
            "b": {"value": 1, "reason": null},
            "scaleRole": "calibration",
            "caseId": "not-applicable",
            "inputVariantId": "canonical-valid-v1",
            "sampleKind": "cold-instance",
            "binaryMode": "timing"
        });
        let mut summaries = Vec::new();
        let mut batch_pairs = [Vec::new(), Vec::new()];
        for batch in 0..2_u64 {
            for round in 0..4_u64 {
                let attempt_id = format!("attempt/candidate/batch-{batch}/round-{round}");
                let (std_position, sorted_position) = if round.is_multiple_of(2) {
                    (0, 1)
                } else {
                    (1, 0)
                };
                let std_run_id = format!("candidate/batch-{batch}/round-{round}/std");
                let sorted_run_id = format!("candidate/batch-{batch}/round-{round}/sorted");
                for (run_id, position, candidate, wall_time) in [
                    (&std_run_id, std_position, std_candidate.clone(), 100_u64),
                    (
                        &sorted_run_id,
                        sorted_position,
                        sorted_candidate.clone(),
                        90_u64,
                    ),
                ] {
                    let mut run = started_run(
                        template.clone(),
                        run_id,
                        &attempt_id,
                        position,
                        TIMING_BINARY_ID,
                        "cold-instance",
                        Some(wall_time),
                        &"a".repeat(64),
                    );
                    run["batch"] = json!(batch);
                    run["round"] = json!(round);
                    run["candidate"] = candidate;
                    run["workload"]["b"] = json!({"value": 1, "reason": null});
                    run["workload"]["scaleRole"] = json!("calibration");
                    run["workload"]["inputVariantId"] = json!("canonical-valid-v1");
                    document["runs"].as_array_mut().expect("runs").push(run);
                }
                let std_summary_id = format!("summary/{std_run_id}");
                let sorted_summary_id = format!("summary/{sorted_run_id}");
                summaries.push(candidate_round_summary(
                    &std_summary_id,
                    "std-hashmap-randomstate-v1",
                    &stratum,
                    batch,
                    round,
                    &attempt_id,
                    &std_run_id,
                    100,
                ));
                summaries.push(candidate_round_summary(
                    &sorted_summary_id,
                    "sorted-vec-binary-search-v1",
                    &stratum,
                    batch,
                    round,
                    &attempt_id,
                    &sorted_run_id,
                    90,
                ));
                batch_pairs[usize::try_from(batch).expect("batch index")].push(json!({
                    "round": round,
                    "baselineRoundSummaryId": std_summary_id,
                    "candidateRoundSummaryId": sorted_summary_id,
                    "ratio": {"value": {"numerator": 9, "denominator": 10}, "reason": null}
                }));
            }
        }
        document["derived"]["roundMetricSummaries"] = Value::Array(summaries);
        document["derived"]["reproducibilityEnvelopes"] = json!([{
            "candidateId": "baseline-std-randomstate-stable-vec-v1",
            "metric": "wall-time-ns",
            "aggregationScope": "all-completed-non-guard-baseline-ladder-strata-v1",
            "maximizingBatch0LadderBatchSummaryId": "formal/batch-0",
            "maximizingBatch1LadderBatchSummaryId": "formal/batch-1",
            "repeatRatio": {"numerator": 11, "denominator": 10}
        }]);
        document["derived"]["candidateComparisons"] = json!([{
            "candidateId": "sorted-vec-binary-search-v1",
            "baselineId": "std-hashmap-randomstate-v1",
            "rosterId": "roster/external-string",
            "stratum": stratum,
            "metric": "wall-time-ns",
            "batch0": {
                "pairingMethod": "same-batch-same-round-v1",
                "aggregationMethod": "median-of-exact-round-ratios-v1",
                "roundPairs": batch_pairs[0],
                "medianRatio": {"value": {"numerator": 9, "denominator": 10}, "reason": null}
            },
            "batch1": {
                "pairingMethod": "same-batch-same-round-v1",
                "aggregationMethod": "median-of-exact-round-ratios-v1",
                "roundPairs": batch_pairs[1],
                "medianRatio": {"value": {"numerator": 9, "denominator": 10}, "reason": null}
            },
            "decision": "repeatable-improvement"
        }]);
        document
    }

    #[allow(clippy::too_many_arguments)]
    fn candidate_round_summary(
        summary_id: &str,
        candidate_id: &str,
        stratum: &Value,
        batch: u64,
        round: u64,
        attempt_id: &str,
        run_id: &str,
        median: u64,
    ) -> Value {
        json!({
            "summaryId": summary_id,
            "purpose": "candidate-comparison",
            "candidateId": candidate_id,
            "stratum": stratum,
            "metric": "wall-time-ns",
            "batch": batch,
            "round": round,
            "roundAttemptId": attempt_id,
            "aggregationMethod": "median-and-mad-of-exact-integers-v1",
            "contributingRunIds": [run_id],
            "median": median,
            "medianAbsoluteDeviation": 0
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn started_run(
        mut run: Value,
        run_id: &str,
        attempt_id: &str,
        position: u64,
        binary_id: &str,
        sample_kind: &str,
        wall_time_ns: Option<u64>,
        semantic_digest: &str,
    ) -> Value {
        run["runId"] = json!(run_id);
        run["position"] = json!(position);
        run["roundAttempt"] = json!({"id": attempt_id, "ordinal": 0, "scope": "single-experiment"});
        run["compilerInstanceId"] = json!({"value": format!("instance/{run_id}"), "reason": null});
        run["sampleKind"] = json!(sample_kind);
        run["status"] = json!("valid");
        run["process"] = json!({
            "coordinatorPid": 1,
            "childPid": {"value": 2, "reason": null},
            "binaryId": binary_id,
            "exitKind": "success",
            "exitCode": {"value": 0, "reason": null},
            "termination": {
                "kind": "exit-code",
                "signalNumber": {"value": null, "reason": "not-signal-termination"},
                "rawPlatformStatus": {"value": null, "reason": "exit-code-is-authoritative"}
            }
        });
        run["metrics"]["wallTimeNs"] = wall_time_ns
            .map(|value| json!({"value": value, "reason": null}))
            .unwrap_or_else(|| null_observation("not-measured-by-oracle"));
        run["metrics"]["semanticDigest"] = json!({"value": semantic_digest, "reason": null});
        run["metrics"]["diagnosticDigest"] =
            json!({"value": crate::diagnostic::empty_diagnostic_digest(), "reason": null});
        run["metrics"]["privateBytes"] = json!({"value": 1, "reason": null});
        run["guard"]["trigger"] = json!("none");
        run["guard"]["lastPrivateBytes"] = json!({"value": 1, "reason": null});
        run
    }

    fn not_applicable_dependency_audit(cargo_lock_sha256: &str) -> Value {
        json!({
            "licenseSpdxExpression": null_observation("not-applicable"),
            "msrvRustVersion": null_observation("not-applicable"),
            "securityAudit": {
                "tool": null_observation("not-applicable"),
                "databaseSnapshot": null_observation("not-applicable"),
                "observedAtUtc": null_observation("not-applicable"),
                "status": "not-applicable",
                "advisoryIds": []
            },
            "cargoPackageId": null_observation("not-applicable"),
            "cargoPackageChecksumSha256": null_observation("not-applicable"),
            "cargoLockSha256": cargo_lock_sha256
        })
    }

    fn unavailable_dependency_audit(
        cargo_lock_sha256: &str,
        package: &CargoPackageBinding,
    ) -> Value {
        json!({
            "licenseSpdxExpression": null_observation("audit-unavailable"),
            "msrvRustVersion": null_observation("audit-unavailable"),
            "securityAudit": {
                "tool": null_observation("audit-unavailable"),
                "databaseSnapshot": null_observation("audit-unavailable"),
                "observedAtUtc": null_observation("audit-unavailable"),
                "status": "audit-unavailable",
                "advisoryIds": []
            },
            "cargoPackageId": {"value": package.id, "reason": null},
            "cargoPackageChecksumSha256": {"value": package.checksum, "reason": null},
            "cargoLockSha256": cargo_lock_sha256
        })
    }

    fn merge_per_unit_counts(
        trusted: &TrustedContract,
        workload_id: &str,
        n: u64,
        counts: &mut Map<String, Value>,
    ) {
        let workload = trusted.workload_manifest["workloads"]
            .as_array()
            .expect("workloads")
            .iter()
            .find(|workload| workload["id"] == workload_id)
            .expect("workload");
        for (key, value) in workload["perUnitCounts"]
            .as_object()
            .expect("perUnitCounts")
        {
            counts.insert(
                key.clone(),
                json!(value.as_u64().expect("per-unit count") * n),
            );
        }
    }

    fn stage_breakdown_json(stages: &crate::StageBreakdown, reason: &str) -> Value {
        let stages = serde_json::to_value(stages).expect("serialize stages");
        let mut output = Map::new();
        for key in [
            "sourceInput",
            "typedAst",
            "hir",
            "mir",
            "canonicalLir",
            "diagnostics",
            "scratch",
            "outputConstruction",
        ] {
            output.insert(
                key.to_owned(),
                json!({
                    "recordCount": stages[key]["recordCount"],
                    "logicalBytes": stages[key]["logicalBytes"],
                    "attributionTimeNs": null_observation(reason),
                    "liveRequestedBytes": null_observation(reason),
                    "peakLiveRequestedBytes": null_observation(reason)
                }),
            );
        }
        Value::Object(output)
    }

    fn null_observation(reason: &str) -> Value {
        json!({"value": null, "reason": reason})
    }

    fn test_context() -> VerificationContext {
        let direct_cargo_packages = [
            test_cargo_package(
                "hashbrown",
                "0.17.1",
                "ed5909b6e89a2db4456e54cd5f673791d7eca6732202bbf2a9cc504fe2f9b84a",
                &[
                    "allocator-api2",
                    "default",
                    "default-hasher",
                    "equivalent",
                    "inline-more",
                    "raw-entry",
                ],
            ),
            test_cargo_package(
                "indexmap",
                "2.14.0",
                "d466e9454f08e4a911e14806c24e16fba1b4c121d1ea474396f396069cf949d9",
                &["default", "std"],
            ),
            test_cargo_package(
                "xxhash-rust",
                "0.8.18",
                "aee1b19627c7c60102ab80d3a9cbe18de90bfe03bfa6c3715447681f0e8c8af6",
                &["xxh3", "xxh64"],
            ),
        ]
        .into_iter()
        .collect();
        VerificationContext {
            repository_head: "a".repeat(40),
            cargo_lock_sha256: "b".repeat(64),
            direct_cargo_packages,
            binary_sha256: BTreeMap::from([
                (TIMING_BINARY_ID.to_owned(), "1".repeat(64)),
                (ATTRIBUTION_BINARY_ID.to_owned(), "2".repeat(64)),
                (ORACLE_BINARY_ID.to_owned(), "3".repeat(64)),
            ]),
        }
    }

    fn test_cargo_package(
        name: &str,
        version: &str,
        checksum: &str,
        features: &[&str],
    ) -> (String, CargoPackageBinding) {
        let source = "registry+https://github.com/rust-lang/crates.io-index";
        (
            name.to_owned(),
            CargoPackageBinding {
                id: format!("{source}#{name}@{version}"),
                version: version.to_owned(),
                source: source.to_owned(),
                checksum: checksum.to_owned(),
                features: features
                    .iter()
                    .map(|feature| (*feature).to_owned())
                    .collect(),
                license: Some("MIT OR Apache-2.0".to_owned()),
                rust_version: Some("1.85".to_owned()),
            },
        )
    }

    fn temporary_directory(label: &str) -> PathBuf {
        let ordinal = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "laneflow-evidence-{label}-{}-{ordinal}",
            std::process::id()
        ));
        fs::create_dir(&directory).expect("create temporary directory");
        directory
    }
}
