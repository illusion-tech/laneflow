//! #308 私有容器与哈希候选的非生产资格和性能矩阵。
//!
//! 机制内核只用于诊断归因；可发布的候选分类来自真实完整研究管线的新进程执行。
//! 两条路径都不替代 #292 的生产实现选择。

use crate::corridor::CorridorContract;
use crate::ladder_runner::decode_child_execution;
use crate::pilot::{run_monitored_command_child, run_monitored_scalable_oracle};
use crate::timing::{ScalableFailureInput, TimingError};
use crate::{
    ChildProcessMonitorReport, ControlledAllocationGuardReport, ExternalStateObservation,
    GraphProfileId, GuardThresholds, InvalidationReason, ORACLE_BINARY_ID, ProcessObservation,
    RunStatus, SCALABLE_ORACLE_CHILD_SCHEMA, SCALABLE_ORACLE_CHILD_SCHEMA_VERSION,
    ScalableCompilerInstance, ScalableOracleChildReport, ScalableOracleOutcome,
    ScalableStagePlanFactory, ScalableWorkloadId, SystemMemoryMonitor, TIMING_BINARY_ID,
    TrustedContract, build_corridor_stage_summary, repository_root,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};
#[cfg(any(
    feature = "candidate-hashbrown-randomstate",
    feature = "candidate-hashbrown-xxh3",
    feature = "candidate-hashbrown-xxh64",
    feature = "candidate-hashbrown-fnv1a64"
))]
use std::hash::BuildHasher;
#[cfg(any(
    feature = "candidate-hashbrown-xxh3",
    feature = "candidate-hashbrown-xxh64",
    feature = "candidate-hashbrown-fnv1a64"
))]
use std::hash::Hasher;
#[cfg(any(
    feature = "candidate-hashbrown-randomstate",
    feature = "candidate-indexmap-randomstate"
))]
use std::hash::RandomState;
use std::hint::black_box;
use std::path::Path;
use std::process::Command;
use std::str::FromStr;
use std::time::Instant;

pub const CANDIDATE_MATRIX_SCOPE: &str = "mechanism-only-not-production-selection";
pub const CANDIDATE_KERNEL_CHILD_SCHEMA: &str =
    "laneflow.compiler-calibration-candidate-kernel-child";
pub const CANDIDATE_KERNEL_CHILD_SCHEMA_VERSION: u32 = 1;
pub const CANDIDATE_PIPELINE_CHILD_SCHEMA: &str =
    "laneflow.compiler-calibration-candidate-pipeline-child";
pub const CANDIDATE_PIPELINE_CHILD_SCHEMA_VERSION: u32 = 1;
pub const CONSTANT_HASH_CHILD_SCHEMA: &str = "laneflow.compiler-calibration-constant-hash-child";
pub const CONSTANT_HASH_CHILD_SCHEMA_VERSION: u32 = 1;
pub const CANDIDATE_MATRIX_CHECKPOINT_SCHEMA: &str =
    "laneflow.compiler-calibration-candidate-matrix-checkpoint";
pub const CANDIDATE_MATRIX_CHECKPOINT_SCHEMA_VERSION: u32 = 1;
const CANDIDATE_REGISTRY_REVISION: u32 = 1;
const CANDIDATE_PERFORMANCE_SCOPE_REVISION: u32 = 1;
const MAX_CANDIDATE_ROUND_RETRY_ORDINAL: u32 = 2;
pub(crate) const FIXED_HASHER_SEED: u64 = 0x4c46_434f_4d50_0001;
#[cfg(any(test, feature = "candidate-hashbrown-fnv1a64"))]
pub(crate) const FNV1A64_OFFSET_BASIS: u64 = 14_695_981_039_346_656_037;
#[cfg(any(test, feature = "candidate-hashbrown-fnv1a64"))]
pub(crate) const FNV1A64_PRIME: u64 = 1_099_511_628_211;
const UNKNOWN_REFERENCE_ERROR_CODE: &str = "LF-COMP-RESEARCH-E-UNKNOWN-REFERENCE";
pub(crate) const FAST_HASH_CANDIDATES: [&str; 3] = [
    "hashbrown-xxh3-fixed-v1",
    "hashbrown-xxh64-fixed-v1",
    "hashbrown-fnv1a64-v1",
];
const EXPECTED_CANDIDATE_IDS: [&str; 11] = [
    "baseline-std-randomstate-stable-vec-v1",
    "std-hashmap-randomstate-v1",
    "hashbrown-randomstate-v1",
    "sorted-vec-binary-search-v1",
    "hashbrown-xxh3-fixed-v1",
    "hashbrown-xxh64-fixed-v1",
    "hashbrown-fnv1a64-v1",
    "indexmap-randomstate-v1",
    "stable-vec-sort-v1",
    "deterministic-radix-sort-v1",
    "deterministic-bucket-sort-v1",
];

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum CandidateKeyDomain {
    ExternalString,
    ValidatedFixedKey,
    CanonicalOutputOrder,
    FullPipelineBaseline,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum CandidateScaleRole {
    Base,
    Calibration,
    Stress,
}

impl CandidateScaleRole {
    pub const ALL: [Self; 3] = [Self::Base, Self::Calibration, Self::Stress];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Calibration => "calibration",
            Self::Stress => "stress",
        }
    }
}

/// 冻结的完整管线候选比较范围；它只控制研究执行量，不构成生产选择。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidatePerformanceScopeContract {
    pub revision: u32,
    pub scope_id: String,
    pub workload_id: ScalableWorkloadId,
    pub workload_revision: u32,
    pub graph_profile: GraphProfileId,
    pub string_profile: String,
    pub generator_version: u32,
    pub case_id: String,
    pub input_variant_id: String,
    pub scale_roles: Vec<CandidateScaleRole>,
    pub sample_kind: String,
    pub binary_mode: String,
    pub comparison_metrics: Vec<String>,
    pub metric_source_rules: BTreeMap<String, String>,
    pub raw_diagnostic_metric_rule: String,
    pub controlled_allocation_metrics_rule: String,
    pub selection_boundary: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidatePerformanceScalePlan {
    pub scale_role: CandidateScaleRole,
    pub n: u32,
    pub b: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidatePipelineStratum {
    pub scope_id: String,
    pub key_domain: CandidateKeyDomain,
    pub workload_id: ScalableWorkloadId,
    pub workload_revision: u32,
    pub graph_profile: GraphProfileId,
    pub string_profile: String,
    pub generator_version: u32,
    pub n: u32,
    pub b: u32,
    pub scale_role: CandidateScaleRole,
    pub case_id: String,
    pub input_variant_id: String,
    pub sample_kind: String,
    pub binary_mode: String,
}

impl CandidatePipelineStratum {
    pub fn from_scope(
        scope: &CandidatePerformanceScopeContract,
        key_domain: CandidateKeyDomain,
        scale: CandidatePerformanceScalePlan,
    ) -> Result<Self, CandidateMatrixError> {
        scope.validate()?;
        if key_domain == CandidateKeyDomain::FullPipelineBaseline
            || scale.n == 0
            || scale.b == 0
            || (scale.scale_role == CandidateScaleRole::Base && scale.n != scale.b)
        {
            return Err(CandidateMatrixError::InvalidPerformanceStratum);
        }
        Ok(Self {
            scope_id: scope.scope_id.clone(),
            key_domain,
            workload_id: scope.workload_id,
            workload_revision: scope.workload_revision,
            graph_profile: scope.graph_profile,
            string_profile: scope.string_profile.clone(),
            generator_version: scope.generator_version,
            n: scale.n,
            b: scale.b,
            scale_role: scale.scale_role,
            case_id: scope.case_id.clone(),
            input_variant_id: scope.input_variant_id.clone(),
            sample_kind: scope.sample_kind.clone(),
            binary_mode: scope.binary_mode.clone(),
        })
    }
}

impl CandidatePerformanceScopeContract {
    pub fn from_trusted_contract(trusted: &TrustedContract) -> Result<Self, CandidateMatrixError> {
        let value = trusted
            .workload_manifest
            .get("candidatePerformanceScopeContract")
            .cloned()
            .ok_or(CandidateMatrixError::MissingPerformanceScope)?;
        let contract: Self = serde_json::from_value(value)
            .map_err(CandidateMatrixError::InvalidPerformanceScopeShape)?;
        contract.validate()?;
        Ok(contract)
    }

    fn validate(&self) -> Result<(), CandidateMatrixError> {
        let expected_metric_sources = BTreeMap::from([(
            "wall-time-ns".to_owned(),
            "candidate-pipeline-child.wallDurationNs".to_owned(),
        )]);
        if self.revision != CANDIDATE_PERFORMANCE_SCOPE_REVISION
            || self.scope_id != "junction-grid-wide-star-full-pipeline-v1"
            || self.workload_id != ScalableWorkloadId::JunctionGrid
            || self.workload_revision != crate::WORKLOAD_REVISION_V1
            || self.graph_profile != GraphProfileId::WideStar
            || self.string_profile != crate::BASE_SCALE_STRING_PROFILE
            || self.generator_version != crate::GENERATOR_VERSION_V1
            || self.case_id != "not-applicable"
            || self.input_variant_id != "canonical-valid-v1"
            || self.scale_roles != CandidateScaleRole::ALL
            || self.sample_kind != "cold-instance"
            || self.binary_mode != "timing"
            || self.comparison_metrics != ["wall-time-ns"]
            || self.metric_source_rules != expected_metric_sources
            || self.raw_diagnostic_metric_rule
                != "parent-process-monitor.peakPrivateBytes-is-retained-in-runs-but-not-classified-because-formal-baseline-has-no-same-stratum-private-bytes-envelope-v1"
            || self.controlled_allocation_metrics_rule
                != "excluded-from-candidate-classification-because-third-party-and-standard-library-containers-are-not-exactly-attributed-v1"
            || self.selection_boundary
                != "representative-structurally-rich-research-scope-not-production-implementation-selection-v1"
        {
            return Err(CandidateMatrixError::PerformanceScopeMismatch);
        }
        Ok(())
    }
}

pub fn resolve_candidate_performance_scales(
    scope: &CandidatePerformanceScopeContract,
    formal_ladders: &[crate::FormalLadderExecution],
) -> Result<Vec<CandidatePerformanceScalePlan>, CandidateMatrixError> {
    scope.validate()?;
    let mut matching = formal_ladders.iter().filter(|ladder| {
        ladder.workload_id == scope.workload_id
            && ladder.workload_revision == scope.workload_revision
            && ladder.graph_profile == scope.graph_profile.as_str()
            && ladder.string_profile == scope.string_profile
            && ladder.generator_version == scope.generator_version
    });
    let ladder = matching
        .next()
        .ok_or(CandidateMatrixError::MissingPerformanceScopeLadder)?;
    if matching.next().is_some() {
        return Err(CandidateMatrixError::DuplicatePerformanceScopeLadder);
    }
    let analysis = ladder
        .analysis
        .as_ref()
        .ok_or(CandidateMatrixError::MissingPerformanceScaleSelection)?;
    let calibration_n = analysis
        .scale_selection
        .calibration_n
        .ok_or(CandidateMatrixError::MissingPerformanceScaleSelection)?;
    let stress_n = analysis
        .scale_selection
        .stress_n
        .ok_or(CandidateMatrixError::MissingPerformanceScaleSelection)?;
    let plans = [
        CandidatePerformanceScalePlan {
            scale_role: CandidateScaleRole::Base,
            n: ladder.b,
            b: ladder.b,
        },
        CandidatePerformanceScalePlan {
            scale_role: CandidateScaleRole::Calibration,
            n: calibration_n,
            b: ladder.b,
        },
        CandidatePerformanceScalePlan {
            scale_role: CandidateScaleRole::Stress,
            n: stress_n,
            b: ladder.b,
        },
    ];
    for plan in plans {
        if !ladder
            .levels
            .iter()
            .any(|level| level.n == plan.n && level.complete)
        {
            return Err(CandidateMatrixError::IncompletePerformanceScale {
                scale_role: plan.scale_role,
                n: plan.n,
            });
        }
    }
    Ok(plans.to_vec())
}

/// 完整研究管线中三个可替换组件的精确配置。
///
/// 每次候选比较只替换 `under_test_key_domain` 对应的一项；另外两项始终取冻结注册表
/// 基线。普通规模阶梯使用 `baseline()`，不会把单项候选身份误写成完整管线基线。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidatePipelineConfiguration {
    pub under_test_key_domain: Option<CandidateKeyDomain>,
    pub under_test_candidate_id: Option<String>,
    external_string_candidate_id: String,
    validated_fixed_key_candidate_id: String,
    canonical_output_order_candidate_id: String,
}

impl CandidatePipelineConfiguration {
    pub fn baseline(trusted: &TrustedContract) -> Result<Self, CandidateMatrixError> {
        let registry = CandidateRegistry::from_trusted_contract(trusted)?;
        Ok(Self {
            under_test_key_domain: None,
            under_test_candidate_id: None,
            external_string_candidate_id: registry
                .baseline_id(CandidateKeyDomain::ExternalString)?
                .to_owned(),
            validated_fixed_key_candidate_id: registry
                .baseline_id(CandidateKeyDomain::ValidatedFixedKey)?
                .to_owned(),
            canonical_output_order_candidate_id: registry
                .baseline_id(CandidateKeyDomain::CanonicalOutputOrder)?
                .to_owned(),
        })
    }

    pub fn single_candidate(
        trusted: &TrustedContract,
        key_domain: CandidateKeyDomain,
        candidate_id: &str,
    ) -> Result<Self, CandidateMatrixError> {
        if key_domain == CandidateKeyDomain::FullPipelineBaseline {
            return Err(CandidateMatrixError::UnsupportedMechanismDomain(key_domain));
        }
        let registry = CandidateRegistry::from_trusted_contract(trusted)?;
        let candidate = registry.candidate(candidate_id)?;
        if !candidate.allowed_key_domains.contains(&key_domain) {
            return Err(CandidateMatrixError::CandidateDomainMismatch {
                candidate_id: candidate_id.to_owned(),
                key_domain,
            });
        }
        if !candidate_feature_available(candidate_id) {
            return Err(CandidateMatrixError::UnavailableCandidateFeature(
                candidate_id.to_owned(),
            ));
        }
        let mut configuration = Self::baseline(trusted)?;
        match key_domain {
            CandidateKeyDomain::ExternalString => {
                configuration.external_string_candidate_id = candidate_id.to_owned();
            }
            CandidateKeyDomain::ValidatedFixedKey => {
                configuration.validated_fixed_key_candidate_id = candidate_id.to_owned();
            }
            CandidateKeyDomain::CanonicalOutputOrder => {
                configuration.canonical_output_order_candidate_id = candidate_id.to_owned();
            }
            CandidateKeyDomain::FullPipelineBaseline => unreachable!("rejected above"),
        }
        configuration.under_test_key_domain = Some(key_domain);
        configuration.under_test_candidate_id = Some(candidate_id.to_owned());
        Ok(configuration)
    }

    pub(crate) fn candidate_id(&self, key_domain: CandidateKeyDomain) -> &str {
        match key_domain {
            CandidateKeyDomain::ExternalString => &self.external_string_candidate_id,
            CandidateKeyDomain::ValidatedFixedKey => &self.validated_fixed_key_candidate_id,
            CandidateKeyDomain::CanonicalOutputOrder => &self.canonical_output_order_candidate_id,
            CandidateKeyDomain::FullPipelineBaseline => "baseline-std-randomstate-stable-vec-v1",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidatePipelineChecksums {
    pub external_string: u64,
    pub validated_fixed_key: u64,
    pub canonical_output_order: u64,
}

impl CandidateKeyDomain {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExternalString => "external-string",
            Self::ValidatedFixedKey => "validated-fixed-key",
            Self::CanonicalOutputOrder => "canonical-output-order",
            Self::FullPipelineBaseline => "full-pipeline-baseline",
        }
    }
}

impl FromStr for CandidateKeyDomain {
    type Err = CandidateMatrixError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "external-string" => Ok(Self::ExternalString),
            "validated-fixed-key" => Ok(Self::ValidatedFixedKey),
            "canonical-output-order" => Ok(Self::CanonicalOutputOrder),
            "full-pipeline-baseline" => Ok(Self::FullPipelineBaseline),
            _ => Err(CandidateMatrixError::UnknownKeyDomain(value.to_owned())),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidateAlgorithmConstant {
    pub name: String,
    pub decimal_value: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidateComponent {
    pub role: String,
    pub implementation_id: String,
    pub dependency_kind: String,
    pub dependency_source: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidateRegistration {
    pub id: String,
    pub allowed_key_domains: Vec<CandidateKeyDomain>,
    pub hasher_seed_policy: String,
    pub fixed_hasher_seed_hex_u64: Option<String>,
    pub algorithm_constants: Vec<CandidateAlgorithmConstant>,
    pub components: Vec<CandidateComponent>,
}

impl CandidateRegistration {
    fn requires_third_party_safety_evidence(&self) -> bool {
        self.components
            .iter()
            .any(|component| matches!(component.dependency_kind.as_str(), "crates-io" | "git"))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidateRegistry {
    pub revision: u32,
    pub baseline_by_key_domain: BTreeMap<String, String>,
    pub candidates: Vec<CandidateRegistration>,
}

impl CandidateRegistry {
    pub fn from_trusted_contract(trusted: &TrustedContract) -> Result<Self, CandidateMatrixError> {
        let value = trusted
            .workload_manifest
            .get("candidateRegistry")
            .cloned()
            .ok_or(CandidateMatrixError::MissingRegistry)?;
        let registry: Self =
            serde_json::from_value(value).map_err(CandidateMatrixError::InvalidRegistryShape)?;
        registry.validate()?;
        Ok(registry)
    }

    pub fn baseline_id(
        &self,
        key_domain: CandidateKeyDomain,
    ) -> Result<&str, CandidateMatrixError> {
        self.baseline_by_key_domain
            .get(key_domain.as_str())
            .map(String::as_str)
            .ok_or(CandidateMatrixError::MissingBaseline(key_domain))
    }

    pub fn candidates_for(
        &self,
        key_domain: CandidateKeyDomain,
    ) -> impl Iterator<Item = &CandidateRegistration> {
        self.candidates
            .iter()
            .filter(move |candidate| candidate.allowed_key_domains.contains(&key_domain))
    }

    fn validate(&self) -> Result<(), CandidateMatrixError> {
        if self.revision != CANDIDATE_REGISTRY_REVISION {
            return Err(CandidateMatrixError::RegistryRevision {
                actual: self.revision,
            });
        }
        let actual_ids = self
            .candidates
            .iter()
            .map(|candidate| candidate.id.as_str())
            .collect::<Vec<_>>();
        if actual_ids != EXPECTED_CANDIDATE_IDS {
            return Err(CandidateMatrixError::CandidateIdentityOrder);
        }
        let mut unique_ids = BTreeSet::new();
        for candidate in &self.candidates {
            if !unique_ids.insert(candidate.id.as_str()) || candidate.components.is_empty() {
                return Err(CandidateMatrixError::CandidateIdentityOrder);
            }
        }
        for (domain, baseline) in [
            (
                CandidateKeyDomain::ExternalString,
                "std-hashmap-randomstate-v1",
            ),
            (
                CandidateKeyDomain::ValidatedFixedKey,
                "std-hashmap-randomstate-v1",
            ),
            (
                CandidateKeyDomain::CanonicalOutputOrder,
                "stable-vec-sort-v1",
            ),
        ] {
            if self.baseline_id(domain)? != baseline {
                return Err(CandidateMatrixError::BaselineMismatch(domain));
            }
        }
        for candidate_id in FAST_HASH_CANDIDATES {
            let candidate = self
                .candidates
                .iter()
                .find(|candidate| candidate.id == candidate_id)
                .ok_or(CandidateMatrixError::CandidateIdentityOrder)?;
            if candidate.allowed_key_domains != [CandidateKeyDomain::ValidatedFixedKey] {
                return Err(CandidateMatrixError::UnsafeFastHashDomain(
                    candidate.id.clone(),
                ));
            }
        }
        self.validate_fast_hash_constants()
    }

    fn validate_fast_hash_constants(&self) -> Result<(), CandidateMatrixError> {
        for candidate_id in ["hashbrown-xxh3-fixed-v1", "hashbrown-xxh64-fixed-v1"] {
            let candidate = self.candidate(candidate_id)?;
            if candidate.hasher_seed_policy != "fixed-u64"
                || candidate.fixed_hasher_seed_hex_u64.as_deref() != Some("4c46434f4d500001")
                || !candidate.algorithm_constants.is_empty()
            {
                return Err(CandidateMatrixError::HasherContract(
                    candidate_id.to_owned(),
                ));
            }
        }
        let fnv = self.candidate("hashbrown-fnv1a64-v1")?;
        let constants = fnv
            .algorithm_constants
            .iter()
            .map(|constant| (constant.name.as_str(), constant.decimal_value.as_str()))
            .collect::<Vec<_>>();
        if fnv.hasher_seed_policy != "not-applicable"
            || fnv.fixed_hasher_seed_hex_u64.is_some()
            || constants
                != [
                    ("offset-basis-u64", "14695981039346656037"),
                    ("prime-u64", "1099511628211"),
                ]
        {
            return Err(CandidateMatrixError::HasherContract(fnv.id.clone()));
        }
        Ok(())
    }

    fn candidate(&self, id: &str) -> Result<&CandidateRegistration, CandidateMatrixError> {
        self.candidates
            .iter()
            .find(|candidate| candidate.id == id)
            .ok_or_else(|| CandidateMatrixError::UnregisteredCandidate(id.to_owned()))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CandidateSafetyStatus {
    Passed,
    Rejected,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidateSafetyAssessment {
    pub candidate_id: String,
    pub status: CandidateSafetyStatus,
    pub evidence: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidatePackageSafetyAudit {
    pub package_name: String,
    pub cargo_package_id: String,
    pub version: String,
    pub checksum_sha256: String,
    pub license_spdx_expression: String,
    pub msrv_rust_version: String,
    pub enabled_features: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidateSafetyAuditSnapshot {
    pub tool: String,
    pub command: String,
    pub observed_at_utc: String,
    pub output_sha256: String,
    pub cargo_lock_sha256: String,
    pub advisory_error_count: u64,
    pub license_error_count: u64,
    pub source_error_count: u64,
    pub ban_error_count: u64,
    pub package_audits: Vec<CandidatePackageSafetyAudit>,
    pub assessments: Vec<CandidateSafetyAssessment>,
}

pub fn audit_candidate_safety(
    trusted: &TrustedContract,
) -> Result<CandidateSafetyAuditSnapshot, CandidateMatrixError> {
    let registry = CandidateRegistry::from_trusted_contract(trusted)?;
    let context = crate::evidence::VerificationContext::from_repository()
        .map_err(|error| CandidateMatrixError::CandidateSafetyContext(error.to_string()))?;
    let root = repository_root();
    let version_output = Command::new("cargo")
        .args(["deny", "--version"])
        .current_dir(&root)
        .output()
        .map_err(|error| CandidateMatrixError::CandidateSafetyCommand(error.to_string()))?;
    if !version_output.status.success() {
        return Err(CandidateMatrixError::CandidateSafetyCommand(
            String::from_utf8_lossy(&version_output.stderr)
                .trim()
                .to_owned(),
        ));
    }
    let tool = String::from_utf8(version_output.stdout)
        .map_err(|error| CandidateMatrixError::CandidateSafetyCommand(error.to_string()))?
        .trim()
        .to_owned();
    if tool != "cargo-deny 0.20.2" {
        return Err(CandidateMatrixError::CandidateSafetyToolVersion(tool));
    }
    let command =
        "cargo deny --format json --locked --all-features check advisories bans licenses sources";
    let audit_output = Command::new("cargo")
        .args([
            "deny",
            "--format",
            "json",
            "--locked",
            "--all-features",
            "check",
            "advisories",
            "bans",
            "licenses",
            "sources",
        ])
        .current_dir(&root)
        .output()
        .map_err(|error| CandidateMatrixError::CandidateSafetyCommand(error.to_string()))?;
    let mut audit_bytes = audit_output.stdout.clone();
    audit_bytes.extend_from_slice(&audit_output.stderr);
    let summary = parse_cargo_deny_summary(&audit_bytes)?;
    if !audit_output.status.success()
        || summary.advisory_errors != 0
        || summary.license_errors != 0
        || summary.source_errors != 0
        || summary.ban_errors != 0
    {
        return Err(CandidateMatrixError::CandidateSafetyAuditFailed {
            advisory_errors: summary.advisory_errors,
            license_errors: summary.license_errors,
            source_errors: summary.source_errors,
            ban_errors: summary.ban_errors,
        });
    }
    let package_names = registry
        .candidates
        .iter()
        .flat_map(|candidate| &candidate.components)
        .filter(|component| matches!(component.dependency_kind.as_str(), "crates-io" | "git"))
        .map(|component| {
            component
                .dependency_source
                .rsplit('/')
                .next()
                .unwrap_or(&component.dependency_source)
        })
        .collect::<BTreeSet<_>>();
    let mut package_audits = Vec::new();
    for package_name in package_names {
        let package = context
            .direct_cargo_packages
            .get(package_name)
            .ok_or_else(|| {
                CandidateMatrixError::MissingCandidateAuditPackage(package_name.to_owned())
            })?;
        let license = package.license.clone().ok_or_else(|| {
            CandidateMatrixError::MissingCandidatePackageLicense(package_name.to_owned())
        })?;
        let rust_version = package.rust_version.clone().unwrap_or_else(|| {
            "not-declared;research-toolchain-1.96-all-features-build-validated".to_owned()
        });
        if package.rust_version.is_some() && !rust_version_at_most(&rust_version, 1, 96)? {
            return Err(CandidateMatrixError::CandidatePackageMsrvTooNew {
                package: package_name.to_owned(),
                rust_version,
            });
        }
        package_audits.push(CandidatePackageSafetyAudit {
            package_name: package_name.to_owned(),
            cargo_package_id: package.id.clone(),
            version: package.version.clone(),
            checksum_sha256: package.checksum.clone(),
            license_spdx_expression: license,
            msrv_rust_version: rust_version,
            enabled_features: package.features.iter().cloned().collect(),
        });
    }
    let assessments = registry
        .candidates
        .iter()
        .map(|candidate| CandidateSafetyAssessment {
            candidate_id: candidate.id.clone(),
            status: CandidateSafetyStatus::Passed,
            evidence: if candidate.requires_third_party_safety_evidence() {
                "cargo-deny-and-direct-package-metadata-passed-v1"
            } else {
                "not-applicable-no-third-party-component-v1"
            }
            .to_owned(),
        })
        .collect();
    Ok(CandidateSafetyAuditSnapshot {
        tool,
        command: command.to_owned(),
        observed_at_utc: current_utc_string()?,
        output_sha256: sha256_hex(&audit_bytes),
        cargo_lock_sha256: context.cargo_lock_sha256,
        advisory_error_count: summary.advisory_errors,
        license_error_count: summary.license_errors,
        source_error_count: summary.source_errors,
        ban_error_count: summary.ban_errors,
        package_audits,
        assessments,
    })
}

#[derive(Deserialize)]
struct CargoDenySummaryEnvelope {
    #[serde(rename = "type")]
    kind: String,
    fields: CargoDenySummaryFields,
}

#[derive(Deserialize)]
struct CargoDenySummaryFields {
    advisories: CargoDenyCheckCounts,
    bans: CargoDenyCheckCounts,
    licenses: CargoDenyCheckCounts,
    sources: CargoDenyCheckCounts,
}

#[derive(Deserialize)]
struct CargoDenyCheckCounts {
    errors: u64,
}

struct ParsedCargoDenySummary {
    advisory_errors: u64,
    license_errors: u64,
    source_errors: u64,
    ban_errors: u64,
}

fn parse_cargo_deny_summary(bytes: &[u8]) -> Result<ParsedCargoDenySummary, CandidateMatrixError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| CandidateMatrixError::CandidateSafetyCommand(error.to_string()))?;
    let envelope = text
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<CargoDenySummaryEnvelope>(line).ok())
        .filter(|envelope| envelope.kind == "summary")
        .ok_or(CandidateMatrixError::MissingCandidateSafetySummary)?;
    Ok(ParsedCargoDenySummary {
        advisory_errors: envelope.fields.advisories.errors,
        license_errors: envelope.fields.licenses.errors,
        source_errors: envelope.fields.sources.errors,
        ban_errors: envelope.fields.bans.errors,
    })
}

fn rust_version_at_most(
    value: &str,
    maximum_major: u64,
    maximum_minor: u64,
) -> Result<bool, CandidateMatrixError> {
    let mut parts = value.split('.');
    let major = parts
        .next()
        .and_then(|part| part.parse::<u64>().ok())
        .ok_or_else(|| CandidateMatrixError::InvalidCandidatePackageMsrv(value.to_owned()))?;
    let minor = parts
        .next()
        .and_then(|part| part.parse::<u64>().ok())
        .ok_or_else(|| CandidateMatrixError::InvalidCandidatePackageMsrv(value.to_owned()))?;
    Ok((major, minor) <= (maximum_major, maximum_minor))
}

fn current_utc_string() -> Result<String, CandidateMatrixError> {
    #[cfg(windows)]
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffffffZ')",
        ])
        .output();
    #[cfg(not(windows))]
    let output = Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output();
    let output =
        output.map_err(|error| CandidateMatrixError::CandidateSafetyCommand(error.to_string()))?;
    if !output.status.success() {
        return Err(CandidateMatrixError::CandidateSafetyCommand(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|error| CandidateMatrixError::CandidateSafetyCommand(error.to_string()))
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CandidateDisposition {
    BaselineParticipant,
    PerformanceParticipant,
    RejectedSafety,
    RejectedCorrectness,
    InsufficientQualificationEvidence,
}

impl CandidateDisposition {
    const fn is_participant(self) -> bool {
        matches!(
            self,
            Self::BaselineParticipant | Self::PerformanceParticipant
        )
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidateMechanismRosterEntry {
    pub candidate_id: String,
    pub disposition: CandidateDisposition,
    pub semantic_digest_sha256: Option<String>,
    pub constant_hash_qualification_id: Option<String>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidateMechanismRoster {
    pub scope: String,
    pub key_domain: CandidateKeyDomain,
    pub item_count: u32,
    pub baseline_id: String,
    pub safety_assessments: Vec<CandidateSafetyAssessment>,
    pub correctness_measurements: Vec<CandidateKernelMeasurement>,
    pub constant_hash_qualifications: Vec<ConstantHashQualification>,
    pub entries: Vec<CandidateMechanismRosterEntry>,
}

impl CandidateMechanismRoster {
    pub fn participant_ids(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|entry| entry.disposition.is_participant())
            .map(|entry| entry.candidate_id.clone())
            .collect()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidateKernelMeasurement {
    pub scope: String,
    pub candidate_id: String,
    pub key_domain: CandidateKeyDomain,
    pub item_count: u32,
    pub wall_time_ns: u64,
    pub semantic_digest_sha256: String,
    pub lookup_checksum: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidateKernelChildReport {
    pub schema: String,
    pub schema_version: u32,
    pub binary_id: String,
    pub child_pid: u32,
    pub measurement: CandidateKernelMeasurement,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CandidatePipelineOutcome {
    Success,
    GuardedInChild,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidatePipelineChildReport {
    pub schema: String,
    pub schema_version: u32,
    pub binary_id: String,
    pub child_pid: u32,
    pub compiler_instance_id: String,
    pub candidate_id: String,
    pub key_domain: CandidateKeyDomain,
    pub workload_id: ScalableWorkloadId,
    pub workload_revision: u32,
    pub graph_profile: String,
    pub string_profile: String,
    pub generator_version: u32,
    pub n: u32,
    pub controlled_allocation_hard_ceiling_bytes: u64,
    pub outcome: CandidatePipelineOutcome,
    pub wall_time_ns: Option<u64>,
    pub semantic_digest_sha256: Option<String>,
    pub candidate_pipeline_checksums: Option<CandidatePipelineChecksums>,
    pub guard_peak_live_requested_bytes: Option<u64>,
    pub controlled_allocation_guard: Option<ControlledAllocationGuardReport>,
}

#[allow(clippy::too_many_arguments)]
pub fn measure_candidate_pipeline_child(
    trusted: &TrustedContract,
    compiler_instance_id: String,
    candidate_id: &str,
    key_domain: CandidateKeyDomain,
    workload_id: ScalableWorkloadId,
    graph_profile: GraphProfileId,
    n: u32,
    controlled_allocation_hard_ceiling_bytes: u64,
) -> Result<CandidatePipelineChildReport, CandidateMatrixError> {
    let configuration =
        CandidatePipelineConfiguration::single_candidate(trusted, key_domain, candidate_id)?;
    let mut compiler = ScalableCompilerInstance::<false>::from_trusted_contract_with_candidate_and_allocation_ceiling(
        trusted,
        compiler_instance_id,
        workload_id,
        controlled_allocation_hard_ceiling_bytes,
        configuration,
    )
    .map_err(|error| CandidateMatrixError::PipelineTiming(error.to_string()))?;
    let compiler_instance_id = compiler.compiler_instance_id().to_owned();
    let base = || CandidatePipelineChildReport {
        schema: CANDIDATE_PIPELINE_CHILD_SCHEMA.to_owned(),
        schema_version: CANDIDATE_PIPELINE_CHILD_SCHEMA_VERSION,
        binary_id: TIMING_BINARY_ID.to_owned(),
        child_pid: std::process::id(),
        compiler_instance_id: compiler_instance_id.clone(),
        candidate_id: candidate_id.to_owned(),
        key_domain,
        workload_id,
        workload_revision: crate::WORKLOAD_REVISION_V1,
        graph_profile: graph_profile.as_str().to_owned(),
        string_profile: crate::BASE_SCALE_STRING_PROFILE.to_owned(),
        generator_version: crate::GENERATOR_VERSION_V1,
        n,
        controlled_allocation_hard_ceiling_bytes,
        outcome: CandidatePipelineOutcome::Success,
        wall_time_ns: None,
        semantic_digest_sha256: None,
        candidate_pipeline_checksums: None,
        guard_peak_live_requested_bytes: None,
        controlled_allocation_guard: None,
    };
    match compiler.measure(graph_profile, n) {
        Ok(sample) => Ok(CandidatePipelineChildReport {
            wall_time_ns: Some(sample.wall_time_ns),
            semantic_digest_sha256: Some(sample.semantic_digest_sha256),
            candidate_pipeline_checksums: Some(sample.candidate_pipeline_checksums),
            guard_peak_live_requested_bytes: Some(sample.guard_peak_live_requested_bytes),
            ..base()
        }),
        Err(TimingError::StageGeneration(
            crate::StageGenerationError::ControlledAllocationHardCeiling {
                field,
                hard_ceiling_bytes,
                live_requested_bytes,
                requested_bytes,
            },
        )) => Ok(CandidatePipelineChildReport {
            outcome: CandidatePipelineOutcome::GuardedInChild,
            controlled_allocation_guard: Some(ControlledAllocationGuardReport {
                field: field.to_owned(),
                hard_ceiling_bytes,
                live_requested_bytes,
                requested_bytes,
            }),
            ..base()
        }),
        Err(error) => Err(CandidateMatrixError::PipelineTiming(error.to_string())),
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ConstantHashOutcome {
    Success,
    CompilerError,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ConstantHashRole {
    CandidateUnderTest,
    ExactResearchOracle,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConstantHashObservation {
    pub observation_id: String,
    pub role: ConstantHashRole,
    pub input_variant_id: String,
    pub repeat: u32,
    pub outcome: ConstantHashOutcome,
    pub error_code: Option<String>,
    pub stage_counts_digest_sha256: String,
    pub semantic_digest_sha256: String,
    pub diagnostic_digest_sha256: String,
    pub partial_output_record_count: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConstantHashQualification {
    pub qualification_id: String,
    pub candidate_id: String,
    pub protocol_id: String,
    pub candidate_builder_id: String,
    pub oracle_builder_id: String,
    pub observations: Vec<ConstantHashObservation>,
    pub passed: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConstantHashChildReport {
    pub schema: String,
    pub schema_version: u32,
    pub binary_id: String,
    pub child_pid: u32,
    pub candidate_id: String,
    pub observation: ConstantHashObservation,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConstantHashProcessRun {
    pub run_id: String,
    pub status: RunStatus,
    pub invalidation_reasons: Vec<InvalidationReason>,
    pub process: ProcessObservation,
    pub child: Option<ConstantHashChildReport>,
    pub monitor: ChildProcessMonitorReport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_state: Option<ExternalStateObservation>,
    pub kill_error: Option<String>,
    pub monitor_error: Option<String>,
    pub stderr: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConstantHashQualificationExecution {
    pub qualification: ConstantHashQualification,
    pub runs: Vec<ConstantHashProcessRun>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MechanismBalancedRound {
    pub batch: u32,
    pub round: u32,
    pub participant_order: Vec<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidatePerformanceSample {
    pub batch: u32,
    pub round: u32,
    pub position: u32,
    pub child_pid: Option<u32>,
    pub binary_id: Option<String>,
    pub measurement: CandidateKernelMeasurement,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidatePerformanceAttempt {
    pub run_id: String,
    pub round_attempt_id: String,
    pub retry_ordinal: u32,
    pub batch: u32,
    pub round: u32,
    pub position: u32,
    pub candidate_id: String,
    pub status: RunStatus,
    pub invalidation_reasons: Vec<InvalidationReason>,
    pub process: ProcessObservation,
    pub child: Option<CandidateKernelChildReport>,
    pub monitor: ChildProcessMonitorReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_state: Option<ExternalStateObservation>,
    pub kill_error: Option<String>,
    pub monitor_error: Option<String>,
    pub stderr: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidatePipelinePerformanceAttempt {
    pub run_id: String,
    pub round_attempt_id: String,
    pub retry_ordinal: u32,
    pub batch: u32,
    pub round: u32,
    pub position: u32,
    pub candidate_id: String,
    pub status: RunStatus,
    pub invalidation_reasons: Vec<InvalidationReason>,
    pub process: ProcessObservation,
    pub child: Option<CandidatePipelineChildReport>,
    pub monitor: ChildProcessMonitorReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_state: Option<ExternalStateObservation>,
    pub kill_error: Option<String>,
    pub monitor_error: Option<String>,
    pub stderr: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidatePipelinePerformanceSample {
    pub run_id: String,
    pub batch: u32,
    pub round: u32,
    pub position: u32,
    pub candidate_id: String,
    pub child: CandidatePipelineChildReport,
    pub monitor: ChildProcessMonitorReport,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidatePipelineExecution {
    pub stratum: CandidatePipelineStratum,
    pub roster: CandidatePipelineQualifiedRoster,
    pub schedule: Vec<MechanismBalancedRound>,
    pub complete: bool,
    pub attempts: Vec<CandidatePipelinePerformanceAttempt>,
    pub samples: Vec<CandidatePipelinePerformanceSample>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidateMatrixExecutionBundle {
    pub schema: String,
    pub schema_version: u32,
    pub scope: CandidatePerformanceScopeContract,
    pub scales: Vec<CandidatePerformanceScalePlan>,
    pub safety_audit: CandidateSafetyAuditSnapshot,
    pub constant_hash_qualifications: Vec<ConstantHashQualificationExecution>,
    pub executions: Vec<CandidatePipelineExecution>,
    pub active_execution: Option<CandidatePipelineExecution>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidatePipelineQualificationRun {
    pub run_id: String,
    pub candidate_id: String,
    pub status: RunStatus,
    pub invalidation_reasons: Vec<InvalidationReason>,
    pub process: ProcessObservation,
    pub child: Option<CandidatePipelineChildReport>,
    pub monitor: ChildProcessMonitorReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_state: Option<ExternalStateObservation>,
    pub kill_error: Option<String>,
    pub monitor_error: Option<String>,
    pub stderr: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidatePipelineQualificationOracleRun {
    pub run_id: String,
    pub status: RunStatus,
    pub invalidation_reasons: Vec<InvalidationReason>,
    pub process: ProcessObservation,
    pub child: Option<ScalableOracleChildReport>,
    pub monitor: ChildProcessMonitorReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_state: Option<ExternalStateObservation>,
    pub kill_error: Option<String>,
    pub monitor_error: Option<String>,
    pub stderr: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidatePipelineQualifiedRosterEntry {
    pub candidate_id: String,
    pub disposition: CandidateDisposition,
    pub correctness_evidence_run_ids: Vec<String>,
    pub constant_hash_qualification_id: Option<String>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidatePipelineQualifiedRoster {
    pub stratum: CandidatePipelineStratum,
    pub baseline_id: String,
    pub safety_assessments: Vec<CandidateSafetyAssessment>,
    pub oracle_run: CandidatePipelineQualificationOracleRun,
    pub candidate_runs: Vec<CandidatePipelineQualificationRun>,
    pub entries: Vec<CandidatePipelineQualifiedRosterEntry>,
}

impl CandidatePipelineQualifiedRoster {
    pub fn participant_ids(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|entry| entry.disposition.is_participant())
            .map(|entry| entry.candidate_id.clone())
            .collect()
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CandidateExecutionMode {
    InProcessDiagnostic,
    FreshProcessTiming,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CandidateDecision {
    RepeatableImprovement,
    RepeatableRegression,
    NoiseNoDifference,
    InsufficientEvidence,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExactRatio {
    pub numerator: u128,
    pub denominator: u128,
}

impl ExactRatio {
    pub fn new(numerator: u128, denominator: u128) -> Result<Self, CandidateMatrixError> {
        if numerator == 0 || denominator == 0 {
            return Err(CandidateMatrixError::NonPositiveRatio);
        }
        let divisor = gcd(numerator, denominator);
        Ok(Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidateMechanismComparison {
    pub scope: String,
    pub candidate_id: String,
    pub baseline_id: String,
    pub metric: &'static str,
    pub envelope: ExactRatio,
    pub batch_median_ratios: [ExactRatio; 2],
    pub decision: CandidateDecision,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidateMechanismExecution {
    pub scope: String,
    pub execution_mode: CandidateExecutionMode,
    pub roster: CandidateMechanismRoster,
    pub schedule: Vec<MechanismBalancedRound>,
    pub complete: bool,
    pub attempts: Vec<CandidatePerformanceAttempt>,
    pub samples: Vec<CandidatePerformanceSample>,
    pub comparisons: Vec<CandidateMechanismComparison>,
}

pub fn build_mechanism_candidate_roster(
    trusted: &TrustedContract,
    key_domain: CandidateKeyDomain,
    item_count: u32,
    safety_assessments: &[CandidateSafetyAssessment],
) -> Result<CandidateMechanismRoster, CandidateMatrixError> {
    if item_count == 0 {
        return Err(CandidateMatrixError::ZeroItemCount);
    }
    let registry = CandidateRegistry::from_trusted_contract(trusted)?;
    let baseline_id = registry.baseline_id(key_domain)?.to_owned();
    let mut safety_by_candidate = BTreeMap::new();
    for assessment in safety_assessments {
        registry.candidate(&assessment.candidate_id)?;
        if assessment.evidence.trim().is_empty() {
            return Err(CandidateMatrixError::MissingSafetyEvidence(
                assessment.candidate_id.clone(),
            ));
        }
        if safety_by_candidate
            .insert(assessment.candidate_id.as_str(), assessment.status)
            .is_some()
        {
            return Err(CandidateMatrixError::DuplicateSafetyAssessment(
                assessment.candidate_id.clone(),
            ));
        }
    }
    let baseline_measurement =
        run_candidate_mechanism_kernel(&baseline_id, key_domain, item_count)?;
    let mut entries = Vec::new();
    let mut correctness_measurements = Vec::new();
    let mut constant_hash_qualifications = Vec::new();

    for candidate in registry.candidates_for(key_domain) {
        let safety = safety_by_candidate
            .get(candidate.id.as_str())
            .copied()
            .unwrap_or_else(|| {
                if candidate.requires_third_party_safety_evidence() {
                    CandidateSafetyStatus::Unavailable
                } else {
                    CandidateSafetyStatus::Passed
                }
            });
        let mut entry = CandidateMechanismRosterEntry {
            candidate_id: candidate.id.clone(),
            disposition: CandidateDisposition::InsufficientQualificationEvidence,
            semantic_digest_sha256: None,
            constant_hash_qualification_id: None,
            reason: None,
        };
        match safety {
            CandidateSafetyStatus::Rejected => {
                entry.disposition = CandidateDisposition::RejectedSafety;
                entry.reason = Some("safety-qualification-rejected".to_owned());
            }
            CandidateSafetyStatus::Unavailable => {
                entry.reason = Some("safety-qualification-unavailable".to_owned());
            }
            CandidateSafetyStatus::Passed => {
                let measurement = if candidate.id == baseline_id {
                    Ok(baseline_measurement.clone())
                } else {
                    run_candidate_mechanism_kernel(&candidate.id, key_domain, item_count)
                };
                match measurement {
                    Err(CandidateMatrixError::UnavailableCandidateFeature(_)) => {
                        entry.reason = Some("candidate-feature-unavailable".to_owned());
                    }
                    Err(error) => return Err(error),
                    Ok(measurement) => {
                        entry.semantic_digest_sha256 =
                            Some(measurement.semantic_digest_sha256.clone());
                        let constant_hash_passed =
                            if FAST_HASH_CANDIDATES.contains(&candidate.id.as_str()) {
                                let qualification =
                                    qualify_constant_hash_candidate(trusted, &candidate.id)?;
                                let passed = qualification.passed;
                                entry.constant_hash_qualification_id =
                                    Some(qualification.qualification_id.clone());
                                constant_hash_qualifications.push(qualification);
                                Some(passed)
                            } else {
                                None
                            };
                        if measurement.semantic_digest_sha256
                            != baseline_measurement.semantic_digest_sha256
                            || measurement.lookup_checksum != baseline_measurement.lookup_checksum
                        {
                            entry.disposition = CandidateDisposition::RejectedCorrectness;
                            entry.reason = Some("mechanism-semantic-mismatch".to_owned());
                        } else if let Some(passed) = constant_hash_passed {
                            if passed {
                                entry.disposition = if candidate.id == baseline_id {
                                    CandidateDisposition::BaselineParticipant
                                } else {
                                    CandidateDisposition::PerformanceParticipant
                                };
                            } else {
                                entry.disposition = CandidateDisposition::RejectedCorrectness;
                                entry.reason =
                                    Some("constant-hash-qualification-failed".to_owned());
                            }
                        } else {
                            entry.disposition = if candidate.id == baseline_id {
                                CandidateDisposition::BaselineParticipant
                            } else {
                                CandidateDisposition::PerformanceParticipant
                            };
                        }
                        correctness_measurements.push(measurement);
                    }
                }
            }
        }
        entries.push(entry);
    }

    Ok(CandidateMechanismRoster {
        scope: CANDIDATE_MATRIX_SCOPE.to_owned(),
        key_domain,
        item_count,
        baseline_id,
        safety_assessments: registry
            .candidates_for(key_domain)
            .filter_map(|candidate| {
                safety_assessments
                    .iter()
                    .find(|assessment| assessment.candidate_id == candidate.id)
                    .cloned()
            })
            .collect(),
        correctness_measurements,
        constant_hash_qualifications,
        entries,
    })
}

fn validate_roster_for_performance(
    roster: &CandidateMechanismRoster,
) -> Result<(), CandidateMatrixError> {
    let baseline_count = roster
        .entries
        .iter()
        .filter(|entry| entry.disposition == CandidateDisposition::BaselineParticipant)
        .count();
    if baseline_count != 1 {
        return Err(CandidateMatrixError::InvalidParticipantBaseline {
            count: baseline_count,
        });
    }
    let participant_count = roster.participant_ids().len();
    if participant_count < 2 {
        return Err(CandidateMatrixError::InsufficientParticipants {
            count: participant_count,
        });
    }
    Ok(())
}

pub fn build_two_batch_balanced_schedule(
    participants: &[String],
) -> Result<Vec<MechanismBalancedRound>, CandidateMatrixError> {
    let count = participants.len();
    if count < 2 {
        return Err(CandidateMatrixError::InsufficientParticipants { count });
    }
    let mut rounds = Vec::with_capacity(4 * count);
    for batch in 0..2_u32 {
        for round in 0..count {
            let participant_order = (0..count)
                .map(|position| participants[(position + round) % count].clone())
                .collect();
            rounds.push(MechanismBalancedRound {
                batch,
                round: u32::try_from(round).expect("candidate count must fit u32"),
                participant_order,
            });
        }
        for reverse_round in 0..count {
            let participant_order = (0..count)
                .map(|position| {
                    let index = (reverse_round + count - (position % count)) % count;
                    participants[index].clone()
                })
                .collect();
            rounds.push(MechanismBalancedRound {
                batch,
                round: u32::try_from(count + reverse_round).expect("candidate round must fit u32"),
                participant_order,
            });
        }
    }
    Ok(rounds)
}

pub fn run_mechanism_candidate_matrix(
    trusted: &TrustedContract,
    key_domain: CandidateKeyDomain,
    item_count: u32,
    safety_assessments: &[CandidateSafetyAssessment],
    reproducibility_envelope: ExactRatio,
) -> Result<CandidateMechanismExecution, CandidateMatrixError> {
    if reproducibility_envelope.numerator < reproducibility_envelope.denominator {
        return Err(CandidateMatrixError::InvalidEnvelope);
    }
    let roster =
        build_mechanism_candidate_roster(trusted, key_domain, item_count, safety_assessments)?;
    validate_roster_for_performance(&roster)?;
    let participants = roster.participant_ids();
    let schedule = build_two_batch_balanced_schedule(&participants)?;
    let mut samples = Vec::new();
    for round in &schedule {
        for (position, candidate_id) in round.participant_order.iter().enumerate() {
            samples.push(CandidatePerformanceSample {
                batch: round.batch,
                round: round.round,
                position: u32::try_from(position).expect("candidate position must fit u32"),
                child_pid: None,
                binary_id: None,
                measurement: run_candidate_mechanism_kernel(candidate_id, key_domain, item_count)?,
            });
        }
    }
    let comparisons = participants
        .iter()
        .filter(|candidate_id| **candidate_id != roster.baseline_id)
        .map(|candidate_id| {
            build_mechanism_comparison(
                candidate_id,
                &roster.baseline_id,
                &schedule,
                &samples,
                reproducibility_envelope,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CandidateMechanismExecution {
        scope: CANDIDATE_MATRIX_SCOPE.to_owned(),
        execution_mode: CandidateExecutionMode::InProcessDiagnostic,
        roster,
        schedule,
        complete: true,
        attempts: Vec::new(),
        samples,
        comparisons,
    })
}

pub fn run_mechanism_candidate_matrix_fresh_process(
    trusted: &TrustedContract,
    timing_binary: &Path,
    key_domain: CandidateKeyDomain,
    item_count: u32,
    safety_assessments: &[CandidateSafetyAssessment],
    reproducibility_envelope: ExactRatio,
) -> Result<CandidateMechanismExecution, CandidateMatrixError> {
    if reproducibility_envelope.numerator < reproducibility_envelope.denominator {
        return Err(CandidateMatrixError::InvalidEnvelope);
    }
    if !timing_binary.is_file() {
        return Err(CandidateMatrixError::MissingTimingBinary(
            timing_binary.to_path_buf(),
        ));
    }
    let roster =
        build_mechanism_candidate_roster(trusted, key_domain, item_count, safety_assessments)?;
    validate_roster_for_performance(&roster)?;
    let mut system_memory_monitor = SystemMemoryMonitor::new()
        .map_err(|error| CandidateMatrixError::Guard(error.to_string()))?;
    let system_memory = system_memory_monitor
        .observe()
        .map_err(|error| CandidateMatrixError::Guard(error.to_string()))?;
    let thresholds =
        GuardThresholds::from_physical_memory_bytes(system_memory.physical_memory_bytes)
            .map_err(|error| CandidateMatrixError::Guard(error.to_string()))?;
    let participants = roster.participant_ids();
    let schedule = build_two_batch_balanced_schedule(&participants)?;
    let mut attempts = Vec::new();
    let mut samples = Vec::new();
    let mut complete = true;
    'rounds: for (round_ordinal, round) in schedule.iter().enumerate() {
        for retry_ordinal in 0..=MAX_CANDIDATE_ROUND_RETRY_ORDINAL {
            let round_attempt_id = format!(
                "candidate/{}/{}/batch-{}/round-{}/attempt-{retry_ordinal}",
                key_domain.as_str(),
                item_count,
                round.batch,
                round.round
            );
            let attempt_start = attempts.len();
            let mut round_samples = Vec::with_capacity(participants.len());
            for (position, candidate_id) in round.participant_order.iter().enumerate() {
                let child_ordinal = round_ordinal
                    .checked_mul(participants.len())
                    .and_then(|value| value.checked_add(position))
                    .and_then(|value| {
                        value.checked_add(
                            usize::try_from(retry_ordinal)
                                .expect("retry ordinal fits usize")
                                .checked_mul(schedule.len().checked_mul(participants.len())?)?,
                        )
                    })
                    .ok_or(CandidateMatrixError::ChildOrdinalOverflow)?;
                let run_id =
                    format!("{round_attempt_id}/position-{position}/candidate-{candidate_id}");
                let attempt = run_candidate_kernel_child(CandidateChildAttemptRequest {
                    timing_binary,
                    ordinal: child_ordinal,
                    run_id,
                    round_attempt_id: round_attempt_id.clone(),
                    retry_ordinal,
                    round,
                    position,
                    candidate_id,
                    key_domain,
                    item_count,
                    thresholds,
                })?;
                if attempt.status == RunStatus::Valid {
                    let child = attempt
                        .child
                        .as_ref()
                        .expect("a valid candidate attempt has a child report");
                    validate_candidate_child_measurement(
                        &roster,
                        candidate_id,
                        &child.measurement,
                    )?;
                    round_samples.push(CandidatePerformanceSample {
                        batch: round.batch,
                        round: round.round,
                        position: u32::try_from(position).expect("candidate position must fit u32"),
                        child_pid: Some(child.child_pid),
                        binary_id: Some(child.binary_id.clone()),
                        measurement: child.measurement.clone(),
                    });
                }
                attempts.push(attempt);
            }
            let attempt_end = attempts.len();
            let mut group_reasons = attempts[attempt_start..attempt_end]
                .iter()
                .flat_map(|attempt| attempt.invalidation_reasons.iter().copied())
                .collect::<Vec<_>>();
            group_reasons.sort_unstable();
            group_reasons.dedup();
            if group_reasons.is_empty() {
                samples.extend(round_samples);
                continue 'rounds;
            }
            for attempt in &mut attempts[attempt_start..attempt_end] {
                attempt.status = RunStatus::Invalid;
                attempt.invalidation_reasons = group_reasons.clone();
            }
            if retry_ordinal == MAX_CANDIDATE_ROUND_RETRY_ORDINAL {
                complete = false;
                break 'rounds;
            }
        }
    }
    let comparisons = if complete {
        participants
            .iter()
            .filter(|candidate_id| **candidate_id != roster.baseline_id)
            .map(|candidate_id| {
                build_mechanism_comparison(
                    candidate_id,
                    &roster.baseline_id,
                    &schedule,
                    &samples,
                    reproducibility_envelope,
                )
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };
    Ok(CandidateMechanismExecution {
        scope: CANDIDATE_MATRIX_SCOPE.to_owned(),
        execution_mode: CandidateExecutionMode::FreshProcessTiming,
        roster,
        schedule,
        complete,
        attempts,
        samples,
        comparisons,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn run_pipeline_candidate_matrix_fresh_process(
    timing_binary: &Path,
    roster: CandidatePipelineQualifiedRoster,
) -> Result<CandidatePipelineExecution, CandidateMatrixError> {
    run_pipeline_candidate_matrix_with_checkpoint_sink(timing_binary, roster, |_| Ok(()))
}

pub fn run_pipeline_candidate_matrix_with_checkpoint_sink(
    timing_binary: &Path,
    roster: CandidatePipelineQualifiedRoster,
    mut persist: impl FnMut(&CandidatePipelineExecution) -> Result<(), String>,
) -> Result<CandidatePipelineExecution, CandidateMatrixError> {
    let stratum = roster.stratum.clone();
    let key_domain = stratum.key_domain;
    let workload_id = stratum.workload_id;
    let graph_profile = stratum.graph_profile;
    let n = stratum.n;
    if n == 0 {
        return Err(CandidateMatrixError::ZeroItemCount);
    }
    if workload_id == ScalableWorkloadId::Identity {
        return Err(CandidateMatrixError::UnsupportedCandidatePipelineWorkload(
            workload_id,
        ));
    }
    if !timing_binary.is_file() {
        return Err(CandidateMatrixError::MissingTimingBinary(
            timing_binary.to_path_buf(),
        ));
    }
    validate_qualified_roster_for_performance(&roster)?;
    let mut system_memory_monitor = SystemMemoryMonitor::new()
        .map_err(|error| CandidateMatrixError::Guard(error.to_string()))?;
    let system_memory = system_memory_monitor
        .observe()
        .map_err(|error| CandidateMatrixError::Guard(error.to_string()))?;
    let thresholds =
        GuardThresholds::from_physical_memory_bytes(system_memory.physical_memory_bytes)
            .map_err(|error| CandidateMatrixError::Guard(error.to_string()))?;
    let participants = roster.participant_ids();
    let schedule = build_two_batch_balanced_schedule(&participants)?;
    let mut execution = CandidatePipelineExecution {
        stratum: stratum.clone(),
        roster,
        schedule,
        complete: true,
        attempts: Vec::new(),
        samples: Vec::new(),
    };
    persist(&execution).map_err(CandidateMatrixError::CheckpointPersistence)?;
    let mut child_ordinal = 0_usize;
    'rounds: for round in execution.schedule.clone() {
        for retry_ordinal in 0..=MAX_CANDIDATE_ROUND_RETRY_ORDINAL {
            let round_attempt_id = format!(
                "candidate-pipeline/{}/{}/{}/{}/{}/batch-{}/round-{}/attempt-{retry_ordinal}",
                stratum.scale_role.as_str(),
                key_domain.as_str(),
                workload_id.as_str(),
                graph_profile.as_str(),
                n,
                round.batch,
                round.round,
            );
            let attempt_start = execution.attempts.len();
            let mut round_samples = Vec::with_capacity(participants.len());
            for (position, candidate_id) in round.participant_order.iter().enumerate() {
                let run_id =
                    format!("{round_attempt_id}/position-{position}/candidate-{candidate_id}");
                let attempt = run_candidate_pipeline_process(CandidatePipelineAttemptRequest {
                    timing_binary,
                    ordinal: child_ordinal,
                    run_id,
                    round_attempt_id: round_attempt_id.clone(),
                    retry_ordinal,
                    round: &round,
                    position,
                    candidate_id,
                    key_domain,
                    workload_id,
                    graph_profile,
                    n,
                    thresholds,
                })?;
                child_ordinal = child_ordinal
                    .checked_add(1)
                    .ok_or(CandidateMatrixError::ChildOrdinalOverflow)?;
                if attempt.status == RunStatus::Valid {
                    let child = attempt
                        .child
                        .as_ref()
                        .expect("valid candidate pipeline attempt has a report")
                        .clone();
                    round_samples.push(CandidatePipelinePerformanceSample {
                        run_id: attempt.run_id.clone(),
                        batch: round.batch,
                        round: round.round,
                        position: u32::try_from(position).expect("candidate position must fit u32"),
                        candidate_id: candidate_id.clone(),
                        child,
                        monitor: attempt.monitor.clone(),
                    });
                }
                execution.attempts.push(attempt);
            }
            let attempt_end = execution.attempts.len();
            let mut group_reasons = execution.attempts[attempt_start..attempt_end]
                .iter()
                .flat_map(|attempt| attempt.invalidation_reasons.iter().copied())
                .collect::<Vec<_>>();
            group_reasons.sort_unstable();
            group_reasons.dedup();
            if group_reasons.is_empty() {
                validate_pipeline_round_semantics(&round_samples, &execution.roster.baseline_id)?;
                execution.samples.extend(round_samples);
                persist(&execution).map_err(CandidateMatrixError::CheckpointPersistence)?;
                continue 'rounds;
            }
            for attempt in &mut execution.attempts[attempt_start..attempt_end] {
                attempt.status = RunStatus::Invalid;
                attempt.invalidation_reasons = group_reasons.clone();
            }
            if retry_ordinal == MAX_CANDIDATE_ROUND_RETRY_ORDINAL {
                execution.complete = false;
                persist(&execution).map_err(CandidateMatrixError::CheckpointPersistence)?;
                break 'rounds;
            }
            persist(&execution).map_err(CandidateMatrixError::CheckpointPersistence)?;
        }
    }
    Ok(execution)
}

pub fn run_candidate_matrix_bundle_with_checkpoint_sink(
    trusted: &TrustedContract,
    timing_binary: &Path,
    oracle_binary: &Path,
    formal_ladders: &[crate::FormalLadderExecution],
    mut persist: impl FnMut(&CandidateMatrixExecutionBundle) -> Result<(), String>,
) -> Result<CandidateMatrixExecutionBundle, CandidateMatrixError> {
    let scope = CandidatePerformanceScopeContract::from_trusted_contract(trusted)?;
    let scales = resolve_candidate_performance_scales(&scope, formal_ladders)?;
    let safety_audit = audit_candidate_safety(trusted)?;
    let mut bundle = CandidateMatrixExecutionBundle {
        schema: CANDIDATE_MATRIX_CHECKPOINT_SCHEMA.to_owned(),
        schema_version: CANDIDATE_MATRIX_CHECKPOINT_SCHEMA_VERSION,
        scope,
        scales,
        safety_audit,
        constant_hash_qualifications: Vec::new(),
        executions: Vec::new(),
        active_execution: None,
    };
    persist(&bundle).map_err(CandidateMatrixError::CheckpointPersistence)?;
    for (index, candidate_id) in FAST_HASH_CANDIDATES.into_iter().enumerate() {
        let ordinal_base = index
            .checked_mul(6)
            .ok_or(CandidateMatrixError::ChildOrdinalOverflow)?;
        let qualification = qualify_constant_hash_candidate_fresh_process(
            trusted,
            timing_binary,
            oracle_binary,
            candidate_id,
            ordinal_base,
        )?;
        bundle.constant_hash_qualifications.push(qualification);
        persist(&bundle).map_err(CandidateMatrixError::CheckpointPersistence)?;
    }
    let constant_hash_summaries = bundle
        .constant_hash_qualifications
        .iter()
        .map(|execution| execution.qualification.clone())
        .collect::<Vec<_>>();
    for scale in bundle.scales.clone() {
        for key_domain in [
            CandidateKeyDomain::ExternalString,
            CandidateKeyDomain::ValidatedFixedKey,
            CandidateKeyDomain::CanonicalOutputOrder,
        ] {
            let stratum = CandidatePipelineStratum::from_scope(&bundle.scope, key_domain, scale)?;
            let roster = qualify_pipeline_candidate_roster_fresh_process(
                trusted,
                timing_binary,
                oracle_binary,
                stratum,
                &bundle.safety_audit.assessments,
                &constant_hash_summaries,
            )?;
            let execution = run_pipeline_candidate_matrix_with_checkpoint_sink(
                timing_binary,
                roster,
                |active| {
                    bundle.active_execution = Some(active.clone());
                    persist(&bundle)
                },
            )?;
            bundle.active_execution = None;
            bundle.executions.push(execution);
            persist(&bundle).map_err(CandidateMatrixError::CheckpointPersistence)?;
        }
    }
    Ok(bundle)
}

fn validate_qualified_roster_for_performance(
    roster: &CandidatePipelineQualifiedRoster,
) -> Result<(), CandidateMatrixError> {
    let baseline_count = roster
        .entries
        .iter()
        .filter(|entry| entry.disposition == CandidateDisposition::BaselineParticipant)
        .count();
    if baseline_count != 1 {
        return Err(CandidateMatrixError::InvalidParticipantBaseline {
            count: baseline_count,
        });
    }
    let participant_count = roster.participant_ids().len();
    if participant_count < 2 {
        return Err(CandidateMatrixError::InsufficientParticipants {
            count: participant_count,
        });
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn qualify_pipeline_candidate_roster_fresh_process(
    trusted: &TrustedContract,
    timing_binary: &Path,
    oracle_binary: &Path,
    stratum: CandidatePipelineStratum,
    safety_assessments: &[CandidateSafetyAssessment],
    constant_hash_qualifications: &[ConstantHashQualification],
) -> Result<CandidatePipelineQualifiedRoster, CandidateMatrixError> {
    let key_domain = stratum.key_domain;
    let workload_id = stratum.workload_id;
    let graph_profile = stratum.graph_profile;
    let n = stratum.n;
    if n == 0 {
        return Err(CandidateMatrixError::ZeroItemCount);
    }
    if workload_id == ScalableWorkloadId::Identity {
        return Err(CandidateMatrixError::UnsupportedCandidatePipelineWorkload(
            workload_id,
        ));
    }
    for path in [timing_binary, oracle_binary] {
        if !path.is_file() {
            return Err(CandidateMatrixError::MissingCandidateBinary(
                path.to_path_buf(),
            ));
        }
    }
    let registry = CandidateRegistry::from_trusted_contract(trusted)?;
    let baseline_id = registry.baseline_id(key_domain)?.to_owned();
    let safety_by_candidate =
        resolved_safety_assessments(&registry, key_domain, safety_assessments)?;
    let mut system_memory_monitor = SystemMemoryMonitor::new()
        .map_err(|error| CandidateMatrixError::Guard(error.to_string()))?;
    let system_memory = system_memory_monitor
        .observe()
        .map_err(|error| CandidateMatrixError::Guard(error.to_string()))?;
    let thresholds =
        GuardThresholds::from_physical_memory_bytes(system_memory.physical_memory_bytes)
            .map_err(|error| CandidateMatrixError::Guard(error.to_string()))?;
    let plans = ScalableStagePlanFactory::from_trusted_contract(trusted)
        .map_err(|error| CandidateMatrixError::PipelineTiming(error.to_string()))?;
    let plan = plans
        .plan(workload_id, graph_profile, n)
        .map_err(|error| CandidateMatrixError::PipelineTiming(error.to_string()))?;
    let oracle_run_id = format!(
        "candidate-qualification/{}/{}/{}/{}/n-{n}/oracle",
        stratum.scale_role.as_str(),
        key_domain.as_str(),
        workload_id.as_str(),
        graph_profile.as_str(),
    );
    let oracle_execution = run_monitored_scalable_oracle(
        oracle_binary,
        0,
        &oracle_run_id,
        workload_id,
        graph_profile,
        n,
        thresholds,
    )
    .map_err(|error| CandidateMatrixError::MonitorCandidateChild(error.to_string()))?;
    let oracle = decode_child_execution(
        oracle_execution,
        ORACLE_BINARY_ID,
        |report: &ScalableOracleChildReport| {
            validate_candidate_oracle_report(
                report,
                &oracle_run_id,
                workload_id,
                graph_profile,
                n,
                thresholds.compiler_controlled_bytes,
                plan.primary_record_count,
            )
        },
        |report| report.outcome == ScalableOracleOutcome::GuardedInChild,
    )
    .map_err(|error| CandidateMatrixError::MonitorCandidateChild(error.to_string()))?;
    let oracle_run = CandidatePipelineQualificationOracleRun {
        run_id: oracle_run_id.clone(),
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

    let oracle_digest = oracle_run
        .child
        .as_ref()
        .and_then(|child| child.semantic_digest_sha256.clone())
        .filter(|_| oracle_run.status == RunStatus::Valid);
    let mut candidate_runs = Vec::new();
    if oracle_digest.is_some() {
        for (ordinal, candidate) in registry.candidates_for(key_domain).enumerate() {
            if safety_by_candidate[candidate.id.as_str()] != CandidateSafetyStatus::Passed
                || !candidate_feature_available(&candidate.id)
            {
                continue;
            }
            candidate_runs.push(run_candidate_pipeline_qualification_child(
                timing_binary,
                ordinal + 1,
                &candidate.id,
                &stratum,
                thresholds,
            )?);
        }
    }
    let baseline_checksums = candidate_runs
        .iter()
        .find(|run| run.candidate_id == baseline_id && run.status == RunStatus::Valid)
        .and_then(|run| run.child.as_ref())
        .and_then(|child| child.candidate_pipeline_checksums);
    let mut entries = Vec::new();
    for candidate in registry.candidates_for(key_domain) {
        let safety = safety_by_candidate[candidate.id.as_str()];
        let run = candidate_runs
            .iter()
            .find(|run| run.candidate_id == candidate.id);
        let mut entry = CandidatePipelineQualifiedRosterEntry {
            candidate_id: candidate.id.clone(),
            disposition: CandidateDisposition::InsufficientQualificationEvidence,
            correctness_evidence_run_ids: Vec::new(),
            constant_hash_qualification_id: None,
            reason: None,
        };
        match safety {
            CandidateSafetyStatus::Rejected => {
                entry.disposition = CandidateDisposition::RejectedSafety;
                entry.reason = Some("safety-qualification-rejected".to_owned());
            }
            CandidateSafetyStatus::Unavailable => {
                entry.reason = Some("safety-qualification-unavailable".to_owned());
            }
            CandidateSafetyStatus::Passed => {
                if !candidate_feature_available(&candidate.id) {
                    entry.reason = Some("candidate-feature-unavailable".to_owned());
                } else if oracle_digest.is_none() {
                    entry.reason = Some("exact-oracle-run-invalid".to_owned());
                } else if let Some(run) = run.filter(|run| run.status == RunStatus::Valid) {
                    let child = run
                        .child
                        .as_ref()
                        .expect("valid candidate qualification has a child report");
                    let semantic_matches = child.semantic_digest_sha256 == oracle_digest;
                    let checksums_match = baseline_checksums.is_some()
                        && child.candidate_pipeline_checksums == baseline_checksums;
                    entry.correctness_evidence_run_ids =
                        vec![run.run_id.clone(), oracle_run_id.clone()];
                    let constant_hash_qualification = if FAST_HASH_CANDIDATES
                        .contains(&candidate.id.as_str())
                    {
                        let mut matches = constant_hash_qualifications
                            .iter()
                            .filter(|qualification| qualification.candidate_id == candidate.id);
                        let qualification = matches.next();
                        if matches.next().is_some() {
                            return Err(CandidateMatrixError::DuplicateConstantHashQualification(
                                candidate.id.clone(),
                            ));
                        }
                        qualification
                    } else {
                        None
                    };
                    if FAST_HASH_CANDIDATES.contains(&candidate.id.as_str())
                        && constant_hash_qualification.is_none()
                    {
                        entry.reason = Some("constant-hash-qualification-missing".to_owned());
                        entries.push(entry);
                        continue;
                    }
                    let constant_hash_passed = constant_hash_qualification.map(|qualification| {
                        entry.constant_hash_qualification_id =
                            Some(qualification.qualification_id.clone());
                        qualification.passed
                    });
                    if !semantic_matches || !checksums_match || constant_hash_passed == Some(false)
                    {
                        entry.disposition = CandidateDisposition::RejectedCorrectness;
                        entry.reason = Some(
                            if !semantic_matches {
                                "full-pipeline-semantic-mismatch"
                            } else if !checksums_match {
                                "full-pipeline-operation-checksum-mismatch"
                            } else {
                                "constant-hash-qualification-failed"
                            }
                            .to_owned(),
                        );
                    } else {
                        entry.disposition = if candidate.id == baseline_id {
                            CandidateDisposition::BaselineParticipant
                        } else {
                            CandidateDisposition::PerformanceParticipant
                        };
                    }
                } else {
                    entry.reason = Some("candidate-correctness-run-invalid".to_owned());
                }
            }
        }
        entries.push(entry);
    }
    Ok(CandidatePipelineQualifiedRoster {
        stratum,
        baseline_id,
        safety_assessments: safety_assessments.to_vec(),
        oracle_run,
        candidate_runs,
        entries,
    })
}

fn resolved_safety_assessments<'a>(
    registry: &'a CandidateRegistry,
    key_domain: CandidateKeyDomain,
    safety_assessments: &'a [CandidateSafetyAssessment],
) -> Result<BTreeMap<&'a str, CandidateSafetyStatus>, CandidateMatrixError> {
    let mut explicit = BTreeMap::new();
    for assessment in safety_assessments {
        registry.candidate(&assessment.candidate_id)?;
        if assessment.evidence.trim().is_empty() {
            return Err(CandidateMatrixError::MissingSafetyEvidence(
                assessment.candidate_id.clone(),
            ));
        }
        if explicit
            .insert(assessment.candidate_id.as_str(), assessment.status)
            .is_some()
        {
            return Err(CandidateMatrixError::DuplicateSafetyAssessment(
                assessment.candidate_id.clone(),
            ));
        }
    }
    let mut resolved = BTreeMap::new();
    for candidate in registry.candidates_for(key_domain) {
        let status = explicit
            .get(candidate.id.as_str())
            .copied()
            .unwrap_or_else(|| {
                if candidate.requires_third_party_safety_evidence() {
                    CandidateSafetyStatus::Unavailable
                } else {
                    CandidateSafetyStatus::Passed
                }
            });
        resolved.insert(candidate.id.as_str(), status);
    }
    Ok(resolved)
}

#[allow(clippy::too_many_arguments)]
fn run_candidate_pipeline_qualification_child(
    timing_binary: &Path,
    ordinal: usize,
    candidate_id: &str,
    stratum: &CandidatePipelineStratum,
    thresholds: GuardThresholds,
) -> Result<CandidatePipelineQualificationRun, CandidateMatrixError> {
    let key_domain = stratum.key_domain;
    let workload_id = stratum.workload_id;
    let graph_profile = stratum.graph_profile;
    let n = stratum.n;
    let run_id = format!(
        "candidate-qualification/{}/{}/{}/{}/n-{n}/candidate-{candidate_id}",
        stratum.scale_role.as_str(),
        key_domain.as_str(),
        workload_id.as_str(),
        graph_profile.as_str(),
    );
    let compiler_instance_id = format!("{run_id}/compiler");
    let arguments = [
        "run-candidate-pipeline".to_owned(),
        compiler_instance_id.clone(),
        candidate_id.to_owned(),
        key_domain.as_str().to_owned(),
        workload_id.as_str().to_owned(),
        graph_profile.as_str().to_owned(),
        n.to_string(),
        thresholds.compiler_controlled_bytes.to_string(),
    ];
    let execution = run_monitored_command_child(timing_binary, ordinal, &arguments, thresholds)
        .map_err(|error| CandidateMatrixError::MonitorCandidateChild(error.to_string()))?;
    let decoded = decode_child_execution(
        execution,
        TIMING_BINARY_ID,
        |report: &CandidatePipelineChildReport| {
            validate_candidate_pipeline_report(
                report,
                &compiler_instance_id,
                candidate_id,
                key_domain,
                workload_id,
                graph_profile,
                n,
                thresholds.compiler_controlled_bytes,
            )
        },
        |report| report.outcome == CandidatePipelineOutcome::GuardedInChild,
    )
    .map_err(|error| CandidateMatrixError::MonitorCandidateChild(error.to_string()))?;
    Ok(CandidatePipelineQualificationRun {
        run_id,
        candidate_id: candidate_id.to_owned(),
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

#[allow(clippy::too_many_arguments)]
fn validate_candidate_oracle_report(
    report: &ScalableOracleChildReport,
    oracle_run_id: &str,
    workload_id: ScalableWorkloadId,
    graph_profile: GraphProfileId,
    n: u32,
    controlled_ceiling: u64,
    primary_record_count: u64,
) -> Result<(), String> {
    let envelope_matches = report.schema == SCALABLE_ORACLE_CHILD_SCHEMA
        && report.schema_version == SCALABLE_ORACLE_CHILD_SCHEMA_VERSION
        && report.binary_id == ORACLE_BINARY_ID
        && report.oracle_run_id == oracle_run_id
        && report.workload_id == workload_id
        && report.workload_revision == crate::WORKLOAD_REVISION_V1
        && report.graph_profile == graph_profile.as_str()
        && report.string_profile == crate::BASE_SCALE_STRING_PROFILE
        && report.generator_version == crate::GENERATOR_VERSION_V1
        && report.n == n
        && report.controlled_allocation_hard_ceiling_bytes == controlled_ceiling;
    let payload_matches = match report.outcome {
        ScalableOracleOutcome::Success => {
            report
                .guard_peak_live_requested_bytes
                .is_some_and(|value| value > 0)
                && report.primary_record_count == Some(primary_record_count)
                && report.semantic_digest_sha256.is_some()
                && report.complete_counts_equal
                && report.complete_typed_output_equal
                && report.controlled_allocation_guard.is_none()
        }
        ScalableOracleOutcome::GuardedInChild => {
            report.guard_peak_live_requested_bytes.is_none()
                && report.primary_record_count.is_none()
                && report.semantic_digest_sha256.is_none()
                && !report.complete_counts_equal
                && !report.complete_typed_output_equal
                && report.controlled_allocation_guard.is_some()
        }
    };
    if envelope_matches && payload_matches {
        Ok(())
    } else {
        Err("candidate-qualification-oracle-protocol".to_owned())
    }
}

struct CandidatePipelineAttemptRequest<'a> {
    timing_binary: &'a Path,
    ordinal: usize,
    run_id: String,
    round_attempt_id: String,
    retry_ordinal: u32,
    round: &'a MechanismBalancedRound,
    position: usize,
    candidate_id: &'a str,
    key_domain: CandidateKeyDomain,
    workload_id: ScalableWorkloadId,
    graph_profile: GraphProfileId,
    n: u32,
    thresholds: GuardThresholds,
}

fn run_candidate_pipeline_process(
    request: CandidatePipelineAttemptRequest<'_>,
) -> Result<CandidatePipelinePerformanceAttempt, CandidateMatrixError> {
    let compiler_instance_id = format!("{}/compiler", request.run_id);
    let arguments = [
        "run-candidate-pipeline".to_owned(),
        compiler_instance_id.clone(),
        request.candidate_id.to_owned(),
        request.key_domain.as_str().to_owned(),
        request.workload_id.as_str().to_owned(),
        request.graph_profile.as_str().to_owned(),
        request.n.to_string(),
        request.thresholds.compiler_controlled_bytes.to_string(),
    ];
    let execution = run_monitored_command_child(
        request.timing_binary,
        request.ordinal,
        &arguments,
        request.thresholds,
    )
    .map_err(|error| CandidateMatrixError::MonitorCandidateChild(error.to_string()))?;
    let decoded = decode_child_execution(
        execution,
        TIMING_BINARY_ID,
        |report: &CandidatePipelineChildReport| {
            validate_candidate_pipeline_report(
                report,
                &compiler_instance_id,
                request.candidate_id,
                request.key_domain,
                request.workload_id,
                request.graph_profile,
                request.n,
                request.thresholds.compiler_controlled_bytes,
            )
        },
        |report| report.outcome == CandidatePipelineOutcome::GuardedInChild,
    )
    .map_err(|error| CandidateMatrixError::MonitorCandidateChild(error.to_string()))?;
    if decoded
        .child
        .as_ref()
        .is_some_and(|report| decoded.process.child_pid.value != Some(u64::from(report.child_pid)))
    {
        return Err(CandidateMatrixError::CandidateChildProtocol);
    }
    Ok(CandidatePipelinePerformanceAttempt {
        run_id: request.run_id,
        round_attempt_id: request.round_attempt_id,
        retry_ordinal: request.retry_ordinal,
        batch: request.round.batch,
        round: request.round.round,
        position: u32::try_from(request.position).expect("candidate position fits u32"),
        candidate_id: request.candidate_id.to_owned(),
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

#[allow(clippy::too_many_arguments)]
fn validate_candidate_pipeline_report(
    report: &CandidatePipelineChildReport,
    compiler_instance_id: &str,
    candidate_id: &str,
    key_domain: CandidateKeyDomain,
    workload_id: ScalableWorkloadId,
    graph_profile: GraphProfileId,
    n: u32,
    controlled_ceiling: u64,
) -> Result<(), String> {
    if report.schema == CANDIDATE_PIPELINE_CHILD_SCHEMA
        && report.schema_version == CANDIDATE_PIPELINE_CHILD_SCHEMA_VERSION
        && report.binary_id == TIMING_BINARY_ID
        && report.compiler_instance_id == compiler_instance_id
        && report.candidate_id == candidate_id
        && report.key_domain == key_domain
        && report.workload_id == workload_id
        && report.workload_revision == crate::WORKLOAD_REVISION_V1
        && report.graph_profile == graph_profile.as_str()
        && report.string_profile == crate::BASE_SCALE_STRING_PROFILE
        && report.generator_version == crate::GENERATOR_VERSION_V1
        && report.n == n
        && report.controlled_allocation_hard_ceiling_bytes == controlled_ceiling
        && valid_candidate_pipeline_payload(report)
    {
        Ok(())
    } else {
        Err("candidate-pipeline-child-protocol".to_owned())
    }
}

fn valid_candidate_pipeline_payload(report: &CandidatePipelineChildReport) -> bool {
    match report.outcome {
        CandidatePipelineOutcome::Success => {
            report.wall_time_ns.is_some_and(|value| value > 0)
                && report.semantic_digest_sha256.is_some()
                && report.candidate_pipeline_checksums.is_some()
                && report
                    .guard_peak_live_requested_bytes
                    .is_some_and(|value| value > 0)
                && report.controlled_allocation_guard.is_none()
        }
        CandidatePipelineOutcome::GuardedInChild => {
            report.wall_time_ns.is_none()
                && report.semantic_digest_sha256.is_none()
                && report.candidate_pipeline_checksums.is_none()
                && report.guard_peak_live_requested_bytes.is_none()
                && report.controlled_allocation_guard.is_some()
        }
    }
}

fn validate_pipeline_round_semantics(
    samples: &[CandidatePipelinePerformanceSample],
    baseline_id: &str,
) -> Result<(), CandidateMatrixError> {
    let baseline = samples
        .iter()
        .find(|sample| sample.candidate_id == baseline_id)
        .ok_or_else(|| CandidateMatrixError::MissingQualifiedCandidate(baseline_id.to_owned()))?;
    for sample in samples {
        if sample.child.semantic_digest_sha256 != baseline.child.semantic_digest_sha256
            || sample.child.candidate_pipeline_checksums
                != baseline.child.candidate_pipeline_checksums
        {
            return Err(CandidateMatrixError::CandidateChildSemanticMismatch {
                candidate_id: sample.candidate_id.clone(),
            });
        }
    }
    Ok(())
}

struct CandidateChildAttemptRequest<'a> {
    timing_binary: &'a Path,
    ordinal: usize,
    run_id: String,
    round_attempt_id: String,
    retry_ordinal: u32,
    round: &'a MechanismBalancedRound,
    position: usize,
    candidate_id: &'a str,
    key_domain: CandidateKeyDomain,
    item_count: u32,
    thresholds: GuardThresholds,
}

fn run_candidate_kernel_child(
    request: CandidateChildAttemptRequest<'_>,
) -> Result<CandidatePerformanceAttempt, CandidateMatrixError> {
    let CandidateChildAttemptRequest {
        timing_binary,
        ordinal,
        run_id,
        round_attempt_id,
        retry_ordinal,
        round,
        position,
        candidate_id,
        key_domain,
        item_count,
        thresholds,
    } = request;
    let arguments = [
        "run-candidate-kernel".to_owned(),
        candidate_id.to_owned(),
        key_domain.as_str().to_owned(),
        item_count.to_string(),
    ];
    let execution = run_monitored_command_child(timing_binary, ordinal, &arguments, thresholds)
        .map_err(|error| CandidateMatrixError::MonitorCandidateChild(error.to_string()))?;
    let decoded = decode_child_execution(
        execution,
        TIMING_BINARY_ID,
        |report: &CandidateKernelChildReport| {
            if report.schema == CANDIDATE_KERNEL_CHILD_SCHEMA
                && report.schema_version == CANDIDATE_KERNEL_CHILD_SCHEMA_VERSION
                && report.binary_id == TIMING_BINARY_ID
                && report.measurement.scope == CANDIDATE_MATRIX_SCOPE
                && report.measurement.candidate_id == candidate_id
                && report.measurement.key_domain == key_domain
                && report.measurement.item_count == item_count
            {
                Ok(())
            } else {
                Err("candidate-kernel-child-protocol".to_owned())
            }
        },
        |_| false,
    )
    .map_err(|error| CandidateMatrixError::MonitorCandidateChild(error.to_string()))?;
    if decoded
        .child
        .as_ref()
        .is_some_and(|report| decoded.process.child_pid.value != Some(u64::from(report.child_pid)))
    {
        return Err(CandidateMatrixError::CandidateChildProtocol);
    }
    Ok(CandidatePerformanceAttempt {
        run_id,
        round_attempt_id,
        retry_ordinal,
        batch: round.batch,
        round: round.round,
        position: u32::try_from(position).expect("candidate position fits u32"),
        candidate_id: candidate_id.to_owned(),
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

fn validate_candidate_child_measurement(
    roster: &CandidateMechanismRoster,
    candidate_id: &str,
    measurement: &CandidateKernelMeasurement,
) -> Result<(), CandidateMatrixError> {
    let expected = roster
        .correctness_measurements
        .iter()
        .find(|candidate| candidate.candidate_id == candidate_id)
        .ok_or_else(|| CandidateMatrixError::MissingQualifiedCandidate(candidate_id.to_owned()))?;
    if measurement.semantic_digest_sha256 != expected.semantic_digest_sha256
        || measurement.lookup_checksum != expected.lookup_checksum
    {
        return Err(CandidateMatrixError::CandidateChildSemanticMismatch {
            candidate_id: candidate_id.to_owned(),
        });
    }
    Ok(())
}

fn build_mechanism_comparison(
    candidate_id: &str,
    baseline_id: &str,
    schedule: &[MechanismBalancedRound],
    samples: &[CandidatePerformanceSample],
    envelope: ExactRatio,
) -> Result<CandidateMechanismComparison, CandidateMatrixError> {
    let mut batch_medians = Vec::with_capacity(2);
    for batch in 0..2_u32 {
        let mut ratios = Vec::new();
        for round in schedule.iter().filter(|round| round.batch == batch) {
            let candidate = find_sample(samples, batch, round.round, candidate_id)?;
            let baseline = find_sample(samples, batch, round.round, baseline_id)?;
            ratios.push(ExactRatio::new(
                u128::from(candidate.measurement.wall_time_ns),
                u128::from(baseline.measurement.wall_time_ns),
            )?);
        }
        batch_medians.push(exact_even_median(&ratios)?);
    }
    let batch_median_ratios = [batch_medians[0], batch_medians[1]];
    let decision = classify_two_batch(batch_median_ratios, envelope)?;
    Ok(CandidateMechanismComparison {
        scope: CANDIDATE_MATRIX_SCOPE.to_owned(),
        candidate_id: candidate_id.to_owned(),
        baseline_id: baseline_id.to_owned(),
        metric: "wall-time-ns",
        envelope,
        batch_median_ratios,
        decision,
    })
}

fn find_sample<'a>(
    samples: &'a [CandidatePerformanceSample],
    batch: u32,
    round: u32,
    candidate_id: &str,
) -> Result<&'a CandidatePerformanceSample, CandidateMatrixError> {
    let mut matching = samples.iter().filter(|sample| {
        sample.batch == batch
            && sample.round == round
            && sample.measurement.candidate_id == candidate_id
    });
    let sample = matching
        .next()
        .ok_or_else(|| CandidateMatrixError::MissingPerformanceSample {
            batch,
            round,
            candidate_id: candidate_id.to_owned(),
        })?;
    if matching.next().is_some() {
        return Err(CandidateMatrixError::DuplicatePerformanceSample {
            batch,
            round,
            candidate_id: candidate_id.to_owned(),
        });
    }
    Ok(sample)
}

pub fn run_candidate_mechanism_kernel(
    candidate_id: &str,
    key_domain: CandidateKeyDomain,
    item_count: u32,
) -> Result<CandidateKernelMeasurement, CandidateMatrixError> {
    if item_count == 0 {
        return Err(CandidateMatrixError::ZeroItemCount);
    }
    let output = match key_domain {
        CandidateKeyDomain::ExternalString => {
            run_external_string_candidate(candidate_id, item_count)?
        }
        CandidateKeyDomain::ValidatedFixedKey => run_fixed_key_candidate(candidate_id, item_count)?,
        CandidateKeyDomain::CanonicalOutputOrder => {
            run_output_order_candidate(candidate_id, item_count)?
        }
        CandidateKeyDomain::FullPipelineBaseline => {
            return Err(CandidateMatrixError::UnsupportedMechanismDomain(key_domain));
        }
    };
    if output.elapsed_ns == 0 {
        return Err(CandidateMatrixError::ZeroDuration(candidate_id.to_owned()));
    }
    Ok(CandidateKernelMeasurement {
        scope: CANDIDATE_MATRIX_SCOPE.to_owned(),
        candidate_id: candidate_id.to_owned(),
        key_domain,
        item_count,
        wall_time_ns: output.elapsed_ns,
        semantic_digest_sha256: sha256_hex(&output.canonical_bytes),
        lookup_checksum: output.lookup_checksum,
    })
}

pub fn measure_candidate_kernel_child(
    candidate_id: &str,
    key_domain: CandidateKeyDomain,
    item_count: u32,
) -> Result<CandidateKernelChildReport, CandidateMatrixError> {
    Ok(CandidateKernelChildReport {
        schema: CANDIDATE_KERNEL_CHILD_SCHEMA.to_owned(),
        schema_version: CANDIDATE_KERNEL_CHILD_SCHEMA_VERSION,
        binary_id: crate::roles::TIMING_BINARY_ID.to_owned(),
        child_pid: std::process::id(),
        measurement: run_candidate_mechanism_kernel(candidate_id, key_domain, item_count)?,
    })
}

struct TimedKernelOutput {
    elapsed_ns: u64,
    canonical_bytes: Vec<u8>,
    lookup_checksum: u64,
}

fn run_external_string_candidate(
    candidate_id: &str,
    item_count: u32,
) -> Result<TimedKernelOutput, CandidateMatrixError> {
    let pairs = external_string_input(item_count);
    let lookup_keys = pairs.iter().map(|(key, _)| key.clone()).collect::<Vec<_>>();
    match candidate_id {
        "std-hashmap-randomstate-v1" => {
            let start = Instant::now();
            let mut map = HashMap::with_capacity(pairs.len());
            for (key, value) in pairs {
                map.insert(key, value);
            }
            let checksum = lookup_keys.iter().fold(0_u64, |checksum, key| {
                checksum.wrapping_add(*map.get(key).expect("generated key must resolve"))
            });
            black_box(checksum);
            let elapsed_ns = elapsed_ns(start)?;
            let mut canonical = map.into_iter().collect::<Vec<_>>();
            canonical.sort_by(|left, right| left.0.cmp(&right.0));
            Ok(TimedKernelOutput {
                elapsed_ns,
                canonical_bytes: encode_string_pairs(&canonical),
                lookup_checksum: checksum,
            })
        }
        "hashbrown-randomstate-v1" => {
            #[cfg(feature = "candidate-hashbrown-randomstate")]
            {
                let start = Instant::now();
                let mut map =
                    hashbrown::HashMap::with_capacity_and_hasher(pairs.len(), RandomState::new());
                for (key, value) in pairs {
                    map.insert(key, value);
                }
                let checksum = lookup_keys.iter().fold(0_u64, |checksum, key| {
                    checksum.wrapping_add(*map.get(key).expect("generated key must resolve"))
                });
                black_box(checksum);
                let elapsed_ns = elapsed_ns(start)?;
                let mut canonical = map.into_iter().collect::<Vec<_>>();
                canonical.sort_by(|left, right| left.0.cmp(&right.0));
                Ok(TimedKernelOutput {
                    elapsed_ns,
                    canonical_bytes: encode_string_pairs(&canonical),
                    lookup_checksum: checksum,
                })
            }
            #[cfg(not(feature = "candidate-hashbrown-randomstate"))]
            Err(CandidateMatrixError::UnavailableCandidateFeature(
                candidate_id.to_owned(),
            ))
        }
        "sorted-vec-binary-search-v1" => {
            let start = Instant::now();
            let mut sorted = pairs;
            sorted.sort_by(|left, right| left.0.cmp(&right.0));
            let checksum = lookup_keys.iter().fold(0_u64, |checksum, key| {
                let index = sorted
                    .binary_search_by(|(candidate, _)| candidate.cmp(key))
                    .expect("generated key must resolve");
                checksum.wrapping_add(sorted[index].1)
            });
            black_box(checksum);
            let elapsed_ns = elapsed_ns(start)?;
            Ok(TimedKernelOutput {
                elapsed_ns,
                canonical_bytes: encode_string_pairs(&sorted),
                lookup_checksum: checksum,
            })
        }
        _ => Err(CandidateMatrixError::CandidateDomainMismatch {
            candidate_id: candidate_id.to_owned(),
            key_domain: CandidateKeyDomain::ExternalString,
        }),
    }
}

fn run_fixed_key_candidate(
    candidate_id: &str,
    item_count: u32,
) -> Result<TimedKernelOutput, CandidateMatrixError> {
    let pairs = fixed_key_input(item_count);
    let lookup_keys = pairs.iter().map(|(key, _)| *key).collect::<Vec<_>>();
    match candidate_id {
        "std-hashmap-randomstate-v1" => run_std_fixed_map(pairs, &lookup_keys),
        "sorted-vec-binary-search-v1" => run_sorted_fixed_vec(pairs, &lookup_keys),
        "hashbrown-randomstate-v1" => {
            #[cfg(feature = "candidate-hashbrown-randomstate")]
            {
                run_hashbrown_fixed_map(pairs, &lookup_keys, RandomState::new)
            }
            #[cfg(not(feature = "candidate-hashbrown-randomstate"))]
            Err(CandidateMatrixError::UnavailableCandidateFeature(
                candidate_id.to_owned(),
            ))
        }
        "hashbrown-xxh3-fixed-v1" => {
            #[cfg(feature = "candidate-hashbrown-xxh3")]
            {
                run_hashbrown_fixed_map(pairs, &lookup_keys, || {
                    xxhash_rust::xxh3::Xxh3Builder::new().with_seed(FIXED_HASHER_SEED)
                })
            }
            #[cfg(not(feature = "candidate-hashbrown-xxh3"))]
            Err(CandidateMatrixError::UnavailableCandidateFeature(
                candidate_id.to_owned(),
            ))
        }
        "hashbrown-xxh64-fixed-v1" => {
            #[cfg(feature = "candidate-hashbrown-xxh64")]
            {
                run_hashbrown_fixed_map(pairs, &lookup_keys, || {
                    xxhash_rust::xxh64::Xxh64Builder::new(FIXED_HASHER_SEED)
                })
            }
            #[cfg(not(feature = "candidate-hashbrown-xxh64"))]
            Err(CandidateMatrixError::UnavailableCandidateFeature(
                candidate_id.to_owned(),
            ))
        }
        "hashbrown-fnv1a64-v1" => {
            #[cfg(feature = "candidate-hashbrown-fnv1a64")]
            {
                run_hashbrown_fixed_map(pairs, &lookup_keys, || Fnv1a64BuildHasher)
            }
            #[cfg(not(feature = "candidate-hashbrown-fnv1a64"))]
            Err(CandidateMatrixError::UnavailableCandidateFeature(
                candidate_id.to_owned(),
            ))
        }
        "indexmap-randomstate-v1" => {
            #[cfg(feature = "candidate-indexmap-randomstate")]
            {
                let start = Instant::now();
                let mut map =
                    indexmap::IndexMap::with_capacity_and_hasher(pairs.len(), RandomState::new());
                for (key, value) in pairs {
                    map.insert(key, value);
                }
                let checksum = lookup_keys.iter().fold(0_u64, |checksum, key| {
                    checksum.wrapping_add(*map.get(key).expect("generated key must resolve"))
                });
                black_box(checksum);
                let elapsed_ns = elapsed_ns(start)?;
                let mut canonical = map.into_iter().collect::<Vec<_>>();
                canonical.sort_by_key(|(key, _)| *key);
                Ok(TimedKernelOutput {
                    elapsed_ns,
                    canonical_bytes: encode_fixed_pairs(&canonical),
                    lookup_checksum: checksum,
                })
            }
            #[cfg(not(feature = "candidate-indexmap-randomstate"))]
            Err(CandidateMatrixError::UnavailableCandidateFeature(
                candidate_id.to_owned(),
            ))
        }
        _ => Err(CandidateMatrixError::CandidateDomainMismatch {
            candidate_id: candidate_id.to_owned(),
            key_domain: CandidateKeyDomain::ValidatedFixedKey,
        }),
    }
}

fn run_std_fixed_map(
    pairs: Vec<(u128, u64)>,
    lookup_keys: &[u128],
) -> Result<TimedKernelOutput, CandidateMatrixError> {
    let start = Instant::now();
    let mut map = HashMap::with_capacity(pairs.len());
    for (key, value) in pairs {
        map.insert(key, value);
    }
    let checksum = lookup_keys.iter().fold(0_u64, |checksum, key| {
        checksum.wrapping_add(*map.get(key).expect("generated key must resolve"))
    });
    black_box(checksum);
    let elapsed_ns = elapsed_ns(start)?;
    let mut canonical = map.into_iter().collect::<Vec<_>>();
    canonical.sort_by_key(|(key, _)| *key);
    Ok(TimedKernelOutput {
        elapsed_ns,
        canonical_bytes: encode_fixed_pairs(&canonical),
        lookup_checksum: checksum,
    })
}

#[cfg(any(
    feature = "candidate-hashbrown-randomstate",
    feature = "candidate-hashbrown-xxh3",
    feature = "candidate-hashbrown-xxh64",
    feature = "candidate-hashbrown-fnv1a64"
))]
fn run_hashbrown_fixed_map<S, F>(
    pairs: Vec<(u128, u64)>,
    lookup_keys: &[u128],
    build_hasher: F,
) -> Result<TimedKernelOutput, CandidateMatrixError>
where
    S: BuildHasher,
    F: FnOnce() -> S,
{
    let start = Instant::now();
    let hasher = build_hasher();
    let mut map = hashbrown::HashMap::with_capacity_and_hasher(pairs.len(), hasher);
    for (key, value) in pairs {
        map.insert(key, value);
    }
    let checksum = lookup_keys.iter().fold(0_u64, |checksum, key| {
        checksum.wrapping_add(*map.get(key).expect("generated key must resolve"))
    });
    black_box(checksum);
    let elapsed_ns = elapsed_ns(start)?;
    let mut canonical = map.into_iter().collect::<Vec<_>>();
    canonical.sort_by_key(|(key, _)| *key);
    Ok(TimedKernelOutput {
        elapsed_ns,
        canonical_bytes: encode_fixed_pairs(&canonical),
        lookup_checksum: checksum,
    })
}

fn run_sorted_fixed_vec(
    pairs: Vec<(u128, u64)>,
    lookup_keys: &[u128],
) -> Result<TimedKernelOutput, CandidateMatrixError> {
    let start = Instant::now();
    let mut sorted = pairs;
    sorted.sort_by_key(|(key, _)| *key);
    let checksum = lookup_keys.iter().fold(0_u64, |checksum, key| {
        let index = sorted
            .binary_search_by_key(key, |(candidate, _)| *candidate)
            .expect("generated key must resolve");
        checksum.wrapping_add(sorted[index].1)
    });
    black_box(checksum);
    let elapsed_ns = elapsed_ns(start)?;
    Ok(TimedKernelOutput {
        elapsed_ns,
        canonical_bytes: encode_fixed_pairs(&sorted),
        lookup_checksum: checksum,
    })
}

fn run_output_order_candidate(
    candidate_id: &str,
    item_count: u32,
) -> Result<TimedKernelOutput, CandidateMatrixError> {
    let mut values = canonical_order_input(item_count);
    let start = Instant::now();
    match candidate_id {
        "stable-vec-sort-v1" => values.sort(),
        "deterministic-radix-sort-v1" => values = radix_sort_u128(values),
        "deterministic-bucket-sort-v1" => values = bucket_sort_u128(values),
        _ => {
            return Err(CandidateMatrixError::CandidateDomainMismatch {
                candidate_id: candidate_id.to_owned(),
                key_domain: CandidateKeyDomain::CanonicalOutputOrder,
            });
        }
    }
    let checksum = values.iter().fold(0_u64, |checksum, value| {
        checksum.wrapping_add(*value as u64)
    });
    black_box(checksum);
    let elapsed_ns = elapsed_ns(start)?;
    Ok(TimedKernelOutput {
        elapsed_ns,
        canonical_bytes: encode_ordered_values(&values),
        lookup_checksum: checksum,
    })
}

pub fn qualify_constant_hash_candidate(
    trusted: &TrustedContract,
    candidate_id: &str,
) -> Result<ConstantHashQualification, CandidateMatrixError> {
    validate_constant_hash_candidate(candidate_id)?;
    let input = build_constant_hash_input(trusted)?;
    let qualification_id = format!("constant-hash/{candidate_id}/v1");
    let mut observations = Vec::with_capacity(6);
    for (variant_id, missing_reference) in [
        ("constant-hash-canonical-valid-v1", false),
        ("constant-hash-missing-reference-v1", true),
    ] {
        for repeat in 0..2_u32 {
            observations.push(resolve_constant_hash_candidate(
                &qualification_id,
                variant_id,
                repeat,
                missing_reference,
                &input,
            )?);
        }
        observations.push(resolve_constant_hash_oracle(
            &qualification_id,
            variant_id,
            missing_reference,
            &input,
        ));
    }
    Ok(build_constant_hash_qualification(
        candidate_id,
        observations,
    ))
}

pub fn qualify_constant_hash_candidate_fresh_process(
    trusted: &TrustedContract,
    timing_binary: &Path,
    oracle_binary: &Path,
    candidate_id: &str,
    ordinal_base: usize,
) -> Result<ConstantHashQualificationExecution, CandidateMatrixError> {
    validate_constant_hash_candidate(candidate_id)?;
    CandidateRegistry::from_trusted_contract(trusted)?.candidate(candidate_id)?;
    for path in [timing_binary, oracle_binary] {
        if !path.is_file() {
            return Err(CandidateMatrixError::MissingCandidateBinary(
                path.to_path_buf(),
            ));
        }
    }
    let mut system_memory_monitor = SystemMemoryMonitor::new()
        .map_err(|error| CandidateMatrixError::Guard(error.to_string()))?;
    let system_memory = system_memory_monitor
        .observe()
        .map_err(|error| CandidateMatrixError::Guard(error.to_string()))?;
    let thresholds =
        GuardThresholds::from_physical_memory_bytes(system_memory.physical_memory_bytes)
            .map_err(|error| CandidateMatrixError::Guard(error.to_string()))?;
    let mut runs = Vec::with_capacity(6);
    let mut ordinal = ordinal_base;
    for input_variant_id in [
        "constant-hash-canonical-valid-v1",
        "constant-hash-missing-reference-v1",
    ] {
        for repeat in 0..2_u32 {
            runs.push(run_constant_hash_process(
                timing_binary,
                ordinal,
                candidate_id,
                input_variant_id,
                ConstantHashRole::CandidateUnderTest,
                repeat,
                thresholds,
            )?);
            ordinal = ordinal
                .checked_add(1)
                .ok_or(CandidateMatrixError::ChildOrdinalOverflow)?;
        }
        runs.push(run_constant_hash_process(
            oracle_binary,
            ordinal,
            candidate_id,
            input_variant_id,
            ConstantHashRole::ExactResearchOracle,
            0,
            thresholds,
        )?);
        ordinal = ordinal
            .checked_add(1)
            .ok_or(CandidateMatrixError::ChildOrdinalOverflow)?;
    }
    let observations = runs
        .iter()
        .filter(|run| run.status == RunStatus::Valid)
        .filter_map(|run| run.child.as_ref())
        .map(|child| child.observation.clone())
        .collect::<Vec<_>>();
    Ok(ConstantHashQualificationExecution {
        qualification: build_constant_hash_qualification(candidate_id, observations),
        runs,
    })
}

fn run_constant_hash_process(
    executable: &Path,
    ordinal: usize,
    candidate_id: &str,
    input_variant_id: &str,
    role: ConstantHashRole,
    repeat: u32,
    thresholds: GuardThresholds,
) -> Result<ConstantHashProcessRun, CandidateMatrixError> {
    let role_id = match role {
        ConstantHashRole::CandidateUnderTest => "candidate",
        ConstantHashRole::ExactResearchOracle => "oracle",
    };
    let run_id =
        format!("constant-hash/{candidate_id}/{input_variant_id}/{role_id}/repeat-{repeat}");
    let mut arguments = vec![
        "run-constant-hash".to_owned(),
        candidate_id.to_owned(),
        input_variant_id.to_owned(),
    ];
    if role == ConstantHashRole::CandidateUnderTest {
        arguments.push(repeat.to_string());
    }
    let expected_binary = match role {
        ConstantHashRole::CandidateUnderTest => TIMING_BINARY_ID,
        ConstantHashRole::ExactResearchOracle => ORACLE_BINARY_ID,
    };
    let execution = run_monitored_command_child(executable, ordinal, &arguments, thresholds)
        .map_err(|error| CandidateMatrixError::MonitorCandidateChild(error.to_string()))?;
    let decoded = decode_child_execution(
        execution,
        expected_binary,
        |report: &ConstantHashChildReport| {
            validate_constant_hash_child_report(
                report,
                expected_binary,
                candidate_id,
                input_variant_id,
                role,
                repeat,
            )
        },
        |_| false,
    )
    .map_err(|error| CandidateMatrixError::MonitorCandidateChild(error.to_string()))?;
    Ok(ConstantHashProcessRun {
        run_id,
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

fn validate_constant_hash_child_report(
    report: &ConstantHashChildReport,
    expected_binary: &str,
    candidate_id: &str,
    input_variant_id: &str,
    role: ConstantHashRole,
    repeat: u32,
) -> Result<(), String> {
    let role_id = match role {
        ConstantHashRole::CandidateUnderTest => "candidate",
        ConstantHashRole::ExactResearchOracle => "oracle",
    };
    if report.schema == CONSTANT_HASH_CHILD_SCHEMA
        && report.schema_version == CONSTANT_HASH_CHILD_SCHEMA_VERSION
        && report.binary_id == expected_binary
        && report.child_pid != 0
        && report.candidate_id == candidate_id
        && report.observation.role == role
        && report.observation.input_variant_id == input_variant_id
        && report.observation.repeat == repeat
        && report.observation.observation_id
            == format!("constant-hash/{candidate_id}/v1/{input_variant_id}/{role_id}/{repeat}")
    {
        Ok(())
    } else {
        Err("constant-hash-child-protocol".to_owned())
    }
}

pub fn measure_constant_hash_observation(
    trusted: &TrustedContract,
    candidate_id: &str,
    role: ConstantHashRole,
    input_variant_id: &str,
    repeat: u32,
    binary_id: &str,
) -> Result<ConstantHashChildReport, CandidateMatrixError> {
    validate_constant_hash_candidate(candidate_id)?;
    let missing_reference = match input_variant_id {
        "constant-hash-canonical-valid-v1" => false,
        "constant-hash-missing-reference-v1" => true,
        _ => {
            return Err(CandidateMatrixError::UnknownConstantHashInputVariant(
                input_variant_id.to_owned(),
            ));
        }
    };
    match role {
        ConstantHashRole::CandidateUnderTest if repeat > 1 => {
            return Err(CandidateMatrixError::InvalidConstantHashRepeat { role, repeat });
        }
        ConstantHashRole::ExactResearchOracle if repeat != 0 => {
            return Err(CandidateMatrixError::InvalidConstantHashRepeat { role, repeat });
        }
        _ => {}
    }
    let input = build_constant_hash_input(trusted)?;
    let qualification_id = format!("constant-hash/{candidate_id}/v1");
    let observation = match role {
        ConstantHashRole::CandidateUnderTest => resolve_constant_hash_candidate(
            &qualification_id,
            input_variant_id,
            repeat,
            missing_reference,
            &input,
        )?,
        ConstantHashRole::ExactResearchOracle => resolve_constant_hash_oracle(
            &qualification_id,
            input_variant_id,
            missing_reference,
            &input,
        ),
    };
    Ok(ConstantHashChildReport {
        schema: CONSTANT_HASH_CHILD_SCHEMA.to_owned(),
        schema_version: CONSTANT_HASH_CHILD_SCHEMA_VERSION,
        binary_id: binary_id.to_owned(),
        child_pid: std::process::id(),
        candidate_id: candidate_id.to_owned(),
        observation,
    })
}

fn validate_constant_hash_candidate(candidate_id: &str) -> Result<(), CandidateMatrixError> {
    if !FAST_HASH_CANDIDATES.contains(&candidate_id) {
        return Err(CandidateMatrixError::QualificationNotApplicable(
            candidate_id.to_owned(),
        ));
    }
    if !candidate_feature_available(candidate_id) {
        return Err(CandidateMatrixError::UnavailableCandidateFeature(
            candidate_id.to_owned(),
        ));
    }
    Ok(())
}

fn build_constant_hash_qualification(
    candidate_id: &str,
    observations: Vec<ConstantHashObservation>,
) -> ConstantHashQualification {
    let passed = constant_hash_observations_match(&observations);
    ConstantHashQualification {
        qualification_id: format!("constant-hash/{candidate_id}/v1"),
        candidate_id: candidate_id.to_owned(),
        protocol_id: "constant-hash-full-key-equality-v1".to_owned(),
        candidate_builder_id: "all-keys-u64-zero-v1".to_owned(),
        oracle_builder_id: "exact-research-oracle-v1".to_owned(),
        observations,
        passed,
    }
}

fn build_constant_hash_input(
    trusted: &TrustedContract,
) -> Result<ConstantHashInput, CandidateMatrixError> {
    let contract = CorridorContract::from_manifest(&trusted.workload_manifest)
        .map_err(|error| CandidateMatrixError::Corridor(error.to_string()))?;
    let template = contract
        .load_template(&repository_root())
        .map_err(|error| CandidateMatrixError::Corridor(error.to_string()))?;
    let summary = build_corridor_stage_summary(trusted, GraphProfileId::WideStar, 1)
        .map_err(|error| CandidateMatrixError::Corridor(error.to_string()))?;
    let stage_counts_digest = sha256_json(&(&summary.counts, &summary.stages))?;
    let mut diagnostic_compiler = ScalableCompilerInstance::<false>::from_trusted_contract_with_id(
        trusted,
        "constant-hash-diagnostic-reference".to_owned(),
        ScalableWorkloadId::Corridor,
    )
    .map_err(|error| CandidateMatrixError::Timing(error.to_string()))?;
    let missing_reference_diagnostic_digest = diagnostic_compiler
        .run_failure(
            GraphProfileId::WideStar,
            1,
            ScalableFailureInput::MissingReferencePerUnit,
        )
        .map_err(|error| CandidateMatrixError::Timing(error.to_string()))?
        .diagnostic_digest_sha256;
    let mut declarations = BTreeMap::new();
    for (ordinal, entity) in template.entities.iter().enumerate() {
        declarations.insert(
            encode_entity_reference(entity.reference.kind, entity.reference.local),
            u64::try_from(ordinal).expect("corridor entity count must fit u64"),
        );
    }
    let mut references = Vec::new();
    for entity in &template.entities {
        references.extend(
            entity
                .identity_references
                .values()
                .map(|target| encode_entity_reference(target.kind, target.local)),
        );
    }
    for relation in &template.relations {
        let mut targets = Vec::new();
        relation.append_stable_references(&mut targets);
        references.extend(
            targets
                .into_iter()
                .map(|target| encode_entity_reference(target.kind, target.local)),
        );
    }
    for point in &template.geometry {
        references.push(encode_entity_reference(point.frame.kind, point.frame.local));
    }
    Ok(ConstantHashInput {
        declarations,
        references,
        canonical_semantic_digest: summary.semantic_digest_sha256,
        stage_counts_digest,
        empty_diagnostic_digest: crate::diagnostic::empty_diagnostic_digest(),
        missing_reference_diagnostic_digest,
    })
}

struct ConstantHashInput {
    declarations: BTreeMap<u64, u64>,
    references: Vec<u64>,
    canonical_semantic_digest: String,
    stage_counts_digest: String,
    empty_diagnostic_digest: String,
    missing_reference_diagnostic_digest: String,
}

fn resolve_constant_hash_candidate(
    qualification_id: &str,
    variant_id: &str,
    repeat: u32,
    missing_reference: bool,
    input: &ConstantHashInput,
) -> Result<ConstantHashObservation, CandidateMatrixError> {
    #[cfg(any(
        feature = "candidate-hashbrown-xxh3",
        feature = "candidate-hashbrown-xxh64",
        feature = "candidate-hashbrown-fnv1a64"
    ))]
    {
        let mut table = hashbrown::HashMap::with_capacity_and_hasher(
            input.declarations.len(),
            ConstantBuildHasher,
        );
        for (key, value) in &input.declarations {
            table.insert(*key, *value);
        }
        let references = qualification_references(input, missing_reference);
        let resolved = resolve_references(&references, |key| table.get(key).copied());
        Ok(build_constant_hash_observation(
            qualification_id,
            ConstantHashRole::CandidateUnderTest,
            variant_id,
            repeat,
            input,
            resolved,
        ))
    }
    #[cfg(not(any(
        feature = "candidate-hashbrown-xxh3",
        feature = "candidate-hashbrown-xxh64",
        feature = "candidate-hashbrown-fnv1a64"
    )))]
    {
        let _ = (
            qualification_id,
            variant_id,
            repeat,
            missing_reference,
            input,
        );
        Err(CandidateMatrixError::UnavailableCandidateFeature(
            "constant-hash-qualification".to_owned(),
        ))
    }
}

fn resolve_constant_hash_oracle(
    qualification_id: &str,
    variant_id: &str,
    missing_reference: bool,
    input: &ConstantHashInput,
) -> ConstantHashObservation {
    let references = qualification_references(input, missing_reference);
    let resolved = resolve_references(&references, |key| input.declarations.get(key).copied());
    build_constant_hash_observation(
        qualification_id,
        ConstantHashRole::ExactResearchOracle,
        variant_id,
        0,
        input,
        resolved,
    )
}

fn qualification_references(input: &ConstantHashInput, missing_reference: bool) -> Vec<u64> {
    let mut references = input.references.clone();
    if missing_reference {
        references[0] = u64::MAX;
    }
    references
}

fn resolve_references(
    references: &[u64],
    mut lookup: impl FnMut(&u64) -> Option<u64>,
) -> Result<Vec<u64>, (u64, Vec<u64>)> {
    let mut resolved = Vec::with_capacity(references.len());
    for key in references {
        if let Some(value) = lookup(key) {
            resolved.push(value);
        } else {
            return Err((*key, resolved));
        }
    }
    Ok(resolved)
}

fn build_constant_hash_observation(
    qualification_id: &str,
    role: ConstantHashRole,
    variant_id: &str,
    repeat: u32,
    input: &ConstantHashInput,
    resolved: Result<Vec<u64>, (u64, Vec<u64>)>,
) -> ConstantHashObservation {
    let role_id = match role {
        ConstantHashRole::CandidateUnderTest => "candidate",
        ConstantHashRole::ExactResearchOracle => "oracle",
    };
    match resolved {
        Ok(values) => {
            let mut semantic_preimage = input.canonical_semantic_digest.as_bytes().to_vec();
            for value in &values {
                semantic_preimage.extend_from_slice(&value.to_le_bytes());
            }
            ConstantHashObservation {
                observation_id: format!("{qualification_id}/{variant_id}/{role_id}/{repeat}"),
                role,
                input_variant_id: variant_id.to_owned(),
                repeat,
                outcome: ConstantHashOutcome::Success,
                error_code: None,
                stage_counts_digest_sha256: input.stage_counts_digest.clone(),
                semantic_digest_sha256: sha256_hex(&semantic_preimage),
                diagnostic_digest_sha256: input.empty_diagnostic_digest.clone(),
                // This field records output published before a failed compile, not the
                // number of references resolved by a successful qualification run.
                partial_output_record_count: 0,
            }
        }
        Err((_missing_key, partial)) => ConstantHashObservation {
            observation_id: format!("{qualification_id}/{variant_id}/{role_id}/{repeat}"),
            role,
            input_variant_id: variant_id.to_owned(),
            repeat,
            outcome: ConstantHashOutcome::CompilerError,
            error_code: Some(UNKNOWN_REFERENCE_ERROR_CODE.to_owned()),
            stage_counts_digest_sha256: input.stage_counts_digest.clone(),
            semantic_digest_sha256: sha256_hex(&[]),
            diagnostic_digest_sha256: input.missing_reference_diagnostic_digest.clone(),
            partial_output_record_count: u64::try_from(partial.len())
                .expect("partial reference count must fit u64"),
        },
    }
}

fn constant_hash_observations_match(observations: &[ConstantHashObservation]) -> bool {
    if observations.len() != 6 {
        return false;
    }
    for variant in [
        "constant-hash-canonical-valid-v1",
        "constant-hash-missing-reference-v1",
    ] {
        let candidates = observations
            .iter()
            .filter(|observation| {
                observation.input_variant_id == variant
                    && observation.role == ConstantHashRole::CandidateUnderTest
            })
            .collect::<Vec<_>>();
        let oracle = observations.iter().find(|observation| {
            observation.input_variant_id == variant
                && observation.role == ConstantHashRole::ExactResearchOracle
        });
        if candidates.len() != 2 || oracle.is_none() {
            return false;
        }
        let oracle = oracle.expect("oracle presence was checked");
        let expected_outcome = if variant == "constant-hash-canonical-valid-v1" {
            ConstantHashOutcome::Success
        } else {
            ConstantHashOutcome::CompilerError
        };
        let expected_error_code = if expected_outcome == ConstantHashOutcome::CompilerError {
            Some(UNKNOWN_REFERENCE_ERROR_CODE)
        } else {
            None
        };
        if oracle.outcome != expected_outcome || oracle.error_code.as_deref() != expected_error_code
        {
            return false;
        }
        if candidates.iter().any(|candidate| {
            candidate.outcome != oracle.outcome
                || candidate.error_code != oracle.error_code
                || candidate.stage_counts_digest_sha256 != oracle.stage_counts_digest_sha256
                || candidate.semantic_digest_sha256 != oracle.semantic_digest_sha256
                || candidate.diagnostic_digest_sha256 != oracle.diagnostic_digest_sha256
                || candidate.partial_output_record_count != oracle.partial_output_record_count
        }) {
            return false;
        }
    }
    true
}

fn classify_two_batch(
    batch_ratios: [ExactRatio; 2],
    envelope: ExactRatio,
) -> Result<CandidateDecision, CandidateMatrixError> {
    let mut improvements = true;
    let mut regressions = true;
    let mut noise = true;
    for ratio in batch_ratios {
        let improvement_left = checked_product(ratio.numerator, envelope.numerator)?;
        let improvement_right = checked_product(ratio.denominator, envelope.denominator)?;
        improvements &= improvement_left < improvement_right;

        let regression_left = checked_product(ratio.numerator, envelope.denominator)?;
        let regression_right = checked_product(ratio.denominator, envelope.numerator)?;
        regressions &= regression_left > regression_right;
        noise &= improvement_left >= improvement_right && regression_left <= regression_right;
    }
    Ok(if improvements {
        CandidateDecision::RepeatableImprovement
    } else if regressions {
        CandidateDecision::RepeatableRegression
    } else if noise {
        CandidateDecision::NoiseNoDifference
    } else {
        CandidateDecision::InsufficientEvidence
    })
}

fn exact_even_median(ratios: &[ExactRatio]) -> Result<ExactRatio, CandidateMatrixError> {
    if ratios.is_empty() || !ratios.len().is_multiple_of(2) {
        return Err(CandidateMatrixError::InvalidMedianSampleCount(ratios.len()));
    }
    let mut sorted = ratios.to_vec();
    for index in 1..sorted.len() {
        let mut cursor = index;
        while cursor > 0 && compare_ratio(sorted[cursor], sorted[cursor - 1])? == Ordering::Less {
            sorted.swap(cursor, cursor - 1);
            cursor -= 1;
        }
    }
    let upper = sorted.len() / 2;
    let left = sorted[upper - 1];
    let right = sorted[upper];
    let numerator = checked_product(left.numerator, right.denominator)?
        .checked_add(checked_product(right.numerator, left.denominator)?)
        .ok_or(CandidateMatrixError::ExactArithmeticOverflow)?;
    let denominator = checked_product(left.denominator, right.denominator)?
        .checked_mul(2)
        .ok_or(CandidateMatrixError::ExactArithmeticOverflow)?;
    ExactRatio::new(numerator, denominator)
}

fn compare_ratio(left: ExactRatio, right: ExactRatio) -> Result<Ordering, CandidateMatrixError> {
    Ok(checked_product(left.numerator, right.denominator)?
        .cmp(&checked_product(right.numerator, left.denominator)?))
}

fn checked_product(left: u128, right: u128) -> Result<u128, CandidateMatrixError> {
    left.checked_mul(right)
        .ok_or(CandidateMatrixError::ExactArithmeticOverflow)
}

const fn gcd(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

fn candidate_feature_available(candidate_id: &str) -> bool {
    if candidate_id == "hashbrown-randomstate-v1" {
        return cfg!(feature = "candidate-hashbrown-randomstate");
    }
    if candidate_id == "hashbrown-xxh3-fixed-v1" {
        return cfg!(feature = "candidate-hashbrown-xxh3");
    }
    if candidate_id == "hashbrown-xxh64-fixed-v1" {
        return cfg!(feature = "candidate-hashbrown-xxh64");
    }
    if candidate_id == "hashbrown-fnv1a64-v1" {
        return cfg!(feature = "candidate-hashbrown-fnv1a64");
    }
    if candidate_id == "indexmap-randomstate-v1" {
        return cfg!(feature = "candidate-indexmap-randomstate");
    }
    true
}

fn external_string_input(item_count: u32) -> Vec<(String, u64)> {
    (0..item_count)
        .map(|ordinal| {
            (
                format!(
                    "external/{ordinal:08x}/{:016x}",
                    splitmix64(u64::from(ordinal))
                ),
                u64::from(ordinal),
            )
        })
        .collect()
}

fn fixed_key_input(item_count: u32) -> Vec<(u128, u64)> {
    (0..item_count)
        .map(|ordinal| {
            let low = u64::from(ordinal);
            let high = splitmix64(low ^ FIXED_HASHER_SEED);
            ((u128::from(high) << 64) | u128::from(low), low)
        })
        .collect()
}

fn canonical_order_input(item_count: u32) -> Vec<u128> {
    let mut values = fixed_key_input(item_count)
        .into_iter()
        .map(|(key, _)| key)
        .collect::<Vec<_>>();
    let mut state = FIXED_HASHER_SEED;
    for index in (1..values.len()).rev() {
        state = splitmix64(state);
        let target = usize::try_from(state % u64::try_from(index + 1).expect("index must fit u64"))
            .expect("target index must fit usize");
        values.swap(index, target);
    }
    values
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn radix_sort_u128(mut values: Vec<u128>) -> Vec<u128> {
    let mut scratch = vec![0_u128; values.len()];
    for byte_index in 0..16_u32 {
        let mut counts = [0_usize; 256];
        let shift = byte_index * 8;
        for value in &values {
            counts[usize::from((value >> shift) as u8)] += 1;
        }
        let mut offsets = [0_usize; 256];
        let mut total = 0_usize;
        for (offset, count) in offsets.iter_mut().zip(counts) {
            *offset = total;
            total += count;
        }
        for value in &values {
            let bucket = usize::from((value >> shift) as u8);
            scratch[offsets[bucket]] = *value;
            offsets[bucket] += 1;
        }
        std::mem::swap(&mut values, &mut scratch);
    }
    values
}

fn bucket_sort_u128(values: Vec<u128>) -> Vec<u128> {
    let mut buckets = (0..256).map(|_| Vec::new()).collect::<Vec<Vec<u128>>>();
    for value in values {
        buckets[usize::from((value >> 120) as u8)].push(value);
    }
    let total = buckets.iter().map(Vec::len).sum();
    let mut sorted = Vec::with_capacity(total);
    for bucket in &mut buckets {
        bucket.sort_unstable();
        sorted.append(bucket);
    }
    sorted
}

fn encode_string_pairs(pairs: &[(String, u64)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (key, value) in pairs {
        bytes.extend_from_slice(
            &u64::try_from(key.len())
                .expect("string length must fit u64")
                .to_le_bytes(),
        );
        bytes.extend_from_slice(key.as_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn encode_fixed_pairs(pairs: &[(u128, u64)]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(pairs.len() * 24);
    for (key, value) in pairs {
        bytes.extend_from_slice(&key.to_le_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn encode_ordered_values(values: &[u128]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * 16);
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn elapsed_ns(start: Instant) -> Result<u64, CandidateMatrixError> {
    u64::try_from(start.elapsed().as_nanos()).map_err(|_| CandidateMatrixError::DurationOverflow)
}

fn sha256_json(value: &impl Serialize) -> Result<String, CandidateMatrixError> {
    let bytes = serde_json::to_vec(value).map_err(CandidateMatrixError::Serialize)?;
    Ok(sha256_hex(&bytes))
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

const fn encode_entity_reference(kind: u16, local: u32) -> u64 {
    (kind as u64) << 32 | local as u64
}

#[cfg(any(
    feature = "candidate-hashbrown-xxh3",
    feature = "candidate-hashbrown-xxh64",
    feature = "candidate-hashbrown-fnv1a64"
))]
#[derive(Clone, Copy, Debug, Default)]
struct ConstantBuildHasher;

#[cfg(any(
    feature = "candidate-hashbrown-xxh3",
    feature = "candidate-hashbrown-xxh64",
    feature = "candidate-hashbrown-fnv1a64"
))]
impl BuildHasher for ConstantBuildHasher {
    type Hasher = ConstantHasher;

    fn build_hasher(&self) -> Self::Hasher {
        ConstantHasher
    }
}

#[cfg(any(
    feature = "candidate-hashbrown-xxh3",
    feature = "candidate-hashbrown-xxh64",
    feature = "candidate-hashbrown-fnv1a64"
))]
#[derive(Clone, Copy, Debug, Default)]
struct ConstantHasher;

#[cfg(any(
    feature = "candidate-hashbrown-xxh3",
    feature = "candidate-hashbrown-xxh64",
    feature = "candidate-hashbrown-fnv1a64"
))]
impl Hasher for ConstantHasher {
    fn finish(&self) -> u64 {
        0
    }

    fn write(&mut self, _bytes: &[u8]) {}
}

#[cfg(feature = "candidate-hashbrown-fnv1a64")]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Fnv1a64BuildHasher;

#[cfg(feature = "candidate-hashbrown-fnv1a64")]
impl BuildHasher for Fnv1a64BuildHasher {
    type Hasher = Fnv1a64Hasher;

    fn build_hasher(&self) -> Self::Hasher {
        Fnv1a64Hasher(FNV1A64_OFFSET_BASIS)
    }
}

#[cfg(feature = "candidate-hashbrown-fnv1a64")]
#[derive(Clone, Copy, Debug)]
pub(crate) struct Fnv1a64Hasher(u64);

#[cfg(feature = "candidate-hashbrown-fnv1a64")]
impl Hasher for Fnv1a64Hasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(FNV1A64_PRIME);
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CandidateMatrixError {
    #[error("研究工作负载清单缺少 candidateRegistry")]
    MissingRegistry,
    #[error("candidateRegistry 形状无效: {0}")]
    InvalidRegistryShape(serde_json::Error),
    #[error("研究工作负载清单缺少 candidatePerformanceScopeContract")]
    MissingPerformanceScope,
    #[error("candidatePerformanceScopeContract 形状无效: {0}")]
    InvalidPerformanceScopeShape(serde_json::Error),
    #[error("candidatePerformanceScopeContract 不等于冻结的完整管线候选范围")]
    PerformanceScopeMismatch,
    #[error("候选性能分层不满足冻结范围")]
    InvalidPerformanceStratum,
    #[error("正式阶梯缺少候选性能范围对应的自然身份")]
    MissingPerformanceScopeLadder,
    #[error("正式阶梯重复包含候选性能范围对应的自然身份")]
    DuplicatePerformanceScopeLadder,
    #[error("候选性能范围缺少可用的校准规模或压力规模选择")]
    MissingPerformanceScaleSelection,
    #[error("候选性能规模 {scale_role:?}/N={n} 不是完整正式阶梯级别")]
    IncompletePerformanceScale {
        scale_role: CandidateScaleRole,
        n: u32,
    },
    #[error("candidateRegistry revision 必须为 1，实际 {actual}")]
    RegistryRevision { actual: u32 },
    #[error("candidateRegistry 候选身份或顺序不等于冻结注册表")]
    CandidateIdentityOrder,
    #[error("键域 {0:?} 缺少冻结基线")]
    MissingBaseline(CandidateKeyDomain),
    #[error("键域 {0:?} 的冻结基线不匹配")]
    BaselineMismatch(CandidateKeyDomain),
    #[error("快速哈希候选 {0} 错误进入外部可控键域")]
    UnsafeFastHashDomain(String),
    #[error("候选 {0} 的哈希 seed 或算法常量不匹配")]
    HasherContract(String),
    #[error("未注册候选 {0}")]
    UnregisteredCandidate(String),
    #[error("候选 {candidate_id} 不允许用于键域 {key_domain:?}")]
    CandidateDomainMismatch {
        candidate_id: String,
        key_domain: CandidateKeyDomain,
    },
    #[error("候选 {0} 的 Cargo feature 未启用")]
    UnavailableCandidateFeature(String),
    #[error("候选 {0} 的安全资格缺少非空证据说明")]
    MissingSafetyEvidence(String),
    #[error("候选 {0} 的安全资格重复")]
    DuplicateSafetyAssessment(String),
    #[error("无法建立候选安全资格上下文: {0}")]
    CandidateSafetyContext(String),
    #[error("无法执行候选安全资格命令: {0}")]
    CandidateSafetyCommand(String),
    #[error("候选安全资格要求 cargo-deny 0.20.2，实际 {0}")]
    CandidateSafetyToolVersion(String),
    #[error("cargo-deny 输出缺少唯一 summary")]
    MissingCandidateSafetySummary,
    #[error(
        "候选安全资格未通过：advisories={advisory_errors}, licenses={license_errors}, sources={source_errors}, bans={ban_errors}"
    )]
    CandidateSafetyAuditFailed {
        advisory_errors: u64,
        license_errors: u64,
        source_errors: u64,
        ban_errors: u64,
    },
    #[error("Cargo 元数据缺少候选依赖 {0}")]
    MissingCandidateAuditPackage(String),
    #[error("候选依赖 {0} 缺少 SPDX 许可证表达式")]
    MissingCandidatePackageLicense(String),
    #[error("候选依赖 MSRV 不是合法 Rust 版本 {0}")]
    InvalidCandidatePackageMsrv(String),
    #[error("候选依赖 {package} 的 MSRV {rust_version} 高于研究工具链 1.96")]
    CandidatePackageMsrvTooNew {
        package: String,
        rust_version: String,
    },
    #[error("候选 {0} 不适用恒定哈希资格")]
    QualificationNotApplicable(String),
    #[error("候选 {0} 存在重复恒定哈希资格")]
    DuplicateConstantHashQualification(String),
    #[error("未知恒定哈希输入变体 {0}")]
    UnknownConstantHashInputVariant(String),
    #[error("恒定哈希角色 {role:?} 不允许 repeat={repeat}")]
    InvalidConstantHashRepeat { role: ConstantHashRole, repeat: u32 },
    #[error("恒定哈希资格无法生成冻结诊断参考：{0}")]
    Timing(String),
    #[error("候选机制项数必须大于零")]
    ZeroItemCount,
    #[error("机制计时结果为零纳秒：{0}")]
    ZeroDuration(String),
    #[error("机制计时纳秒数溢出 u64")]
    DurationOverflow,
    #[error("机制内核不支持键域 {0:?}")]
    UnsupportedMechanismDomain(CandidateKeyDomain),
    #[error("完整候选管线不支持工作负载 {0:?}")]
    UnsupportedCandidatePipelineWorkload(ScalableWorkloadId),
    #[error("未知候选键域 {0}")]
    UnknownKeyDomain(String),
    #[error("未找到候选计时角色二进制 {0}")]
    MissingTimingBinary(std::path::PathBuf),
    #[error("未找到候选资格所需二进制 {0}")]
    MissingCandidateBinary(std::path::PathBuf),
    #[error("无法建立候选计时角色停止护栏: {0}")]
    Guard(String),
    #[error("候选计时角色序号溢出")]
    ChildOrdinalOverflow,
    #[error("无法监控候选计时角色: {0}")]
    MonitorCandidateChild(String),
    #[error(
        "候选计时角色被停止护栏作废：pid={child_pid}, trigger={trigger}, killError={kill_error:?}, monitorError={monitor_error:?}"
    )]
    CandidateChildInvalidated {
        child_pid: u32,
        trigger: String,
        kill_error: Option<String>,
        monitor_error: Option<String>,
    },
    #[error("候选计时角色异常退出：code={code:?}, stderr={stderr}")]
    CandidateChildExit { code: Option<i32>, stderr: String },
    #[error("候选计时角色没有返回有效 JSON: {0}")]
    InvalidCandidateChildJson(serde_json::Error),
    #[error("候选计时角色返回的身份或测量分层不匹配")]
    CandidateChildProtocol,
    #[error("候选 {0} 缺少已认证正确性测量")]
    MissingQualifiedCandidate(String),
    #[error("候选 {candidate_id} 的新进程语义输出与已认证测量不一致")]
    CandidateChildSemanticMismatch { candidate_id: String },
    #[error("候选参赛名单必须恰有一个基线参与者，实际 {count}")]
    InvalidParticipantBaseline { count: usize },
    #[error("平衡候选顺序至少需要两个参赛者，实际 {count}")]
    InsufficientParticipants { count: usize },
    #[error("缺少 batch={batch} round={round} candidate={candidate_id} 的性能样本")]
    MissingPerformanceSample {
        batch: u32,
        round: u32,
        candidate_id: String,
    },
    #[error("重复 batch={batch} round={round} candidate={candidate_id} 的性能样本")]
    DuplicatePerformanceSample {
        batch: u32,
        round: u32,
        candidate_id: String,
    },
    #[error("精确比值的分子和分母必须都大于零")]
    NonPositiveRatio,
    #[error("重复性包络必须大于等于 1")]
    InvalidEnvelope,
    #[error("精确有理数运算溢出 u128")]
    ExactArithmeticOverflow,
    #[error("精确中位数需要非空偶数样本，实际 {0}")]
    InvalidMedianSampleCount(usize),
    #[error("无法序列化候选资格摘要: {0}")]
    Serialize(serde_json::Error),
    #[error("无法执行 corridor 恒定哈希资格: {0}")]
    Corridor(String),
    #[error("无法执行完整候选研究管线: {0}")]
    PipelineTiming(String),
    #[error("无法持久化候选完整管线检查点: {0}")]
    CheckpointPersistence(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exact_frozen_candidate_registry() {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        let registry = CandidateRegistry::from_trusted_contract(&trusted).expect("registry");

        assert_eq!(
            registry
                .candidates_for(CandidateKeyDomain::ExternalString)
                .count(),
            3
        );
        assert_eq!(
            registry
                .candidates_for(CandidateKeyDomain::ValidatedFixedKey)
                .count(),
            7
        );
        assert_eq!(
            registry
                .candidates_for(CandidateKeyDomain::CanonicalOutputOrder)
                .count(),
            3
        );
        assert_eq!(FIXED_HASHER_SEED, 0x4c46_434f_4d50_0001);
        assert_eq!(FNV1A64_OFFSET_BASIS, 14_695_981_039_346_656_037);
        assert_eq!(FNV1A64_PRIME, 1_099_511_628_211);
    }

    #[test]
    fn parses_exact_frozen_full_pipeline_performance_scope() {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        let scope = CandidatePerformanceScopeContract::from_trusted_contract(&trusted)
            .expect("performance scope");
        assert_eq!(scope.workload_id, ScalableWorkloadId::JunctionGrid);
        assert_eq!(scope.graph_profile, GraphProfileId::WideStar);
        assert_eq!(scope.scale_roles, CandidateScaleRole::ALL);
        assert_eq!(scope.comparison_metrics, ["wall-time-ns"]);
        assert_eq!(
            scope.raw_diagnostic_metric_rule,
            "parent-process-monitor.peakPrivateBytes-is-retained-in-runs-but-not-classified-because-formal-baseline-has-no-same-stratum-private-bytes-envelope-v1"
        );
    }

    #[test]
    #[cfg(feature = "research-runner-full")]
    fn candidate_safety_uses_frozen_cargo_deny_and_direct_package_metadata() {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        let audit = audit_candidate_safety(&trusted).expect("candidate safety audit");
        assert_eq!(audit.tool, "cargo-deny 0.20.2");
        assert_eq!(audit.advisory_error_count, 0);
        assert_eq!(audit.license_error_count, 0);
        assert_eq!(audit.source_error_count, 0);
        assert_eq!(audit.ban_error_count, 0);
        assert_eq!(
            audit
                .package_audits
                .iter()
                .map(|package| package.package_name.as_str())
                .collect::<Vec<_>>(),
            ["hashbrown", "indexmap", "xxhash-rust"]
        );
        assert!(
            audit
                .assessments
                .iter()
                .all(|assessment| assessment.status == CandidateSafetyStatus::Passed)
        );
    }

    #[test]
    fn balanced_schedule_places_every_candidate_twice_at_every_position() {
        let participants = ["a", "b", "c", "d"].map(str::to_owned).to_vec();
        let schedule = build_two_batch_balanced_schedule(&participants).expect("schedule");
        assert_eq!(schedule.len(), 16);
        for batch in 0..2_u32 {
            for candidate in &participants {
                for position in 0..participants.len() {
                    let occurrences = schedule
                        .iter()
                        .filter(|round| round.batch == batch)
                        .filter(|round| round.participant_order[position] == *candidate)
                        .count();
                    assert_eq!(occurrences, 2);
                }
            }
        }
    }

    #[test]
    fn exact_median_and_envelope_classification_have_no_float_boundary() {
        let median = exact_even_median(&[
            ExactRatio::new(1, 2).expect("ratio"),
            ExactRatio::new(3, 4).expect("ratio"),
            ExactRatio::new(5, 4).expect("ratio"),
            ExactRatio::new(3, 2).expect("ratio"),
        ])
        .expect("median");
        assert_eq!(median, ExactRatio::new(1, 1).expect("ratio"));

        let envelope = ExactRatio::new(21, 20).expect("envelope");
        assert_eq!(
            classify_two_batch(
                [
                    ExactRatio::new(20, 21).expect("ratio"),
                    ExactRatio::new(21, 20).expect("ratio")
                ],
                envelope,
            )
            .expect("classification"),
            CandidateDecision::NoiseNoDifference
        );
        assert_eq!(
            classify_two_batch(
                [
                    ExactRatio::new(9, 10).expect("ratio"),
                    ExactRatio::new(19, 20).expect("ratio")
                ],
                envelope,
            )
            .expect("classification"),
            CandidateDecision::RepeatableImprovement
        );
        assert_eq!(
            classify_two_batch(
                [
                    ExactRatio::new(11, 10).expect("ratio"),
                    ExactRatio::new(6, 5).expect("ratio")
                ],
                envelope,
            )
            .expect("classification"),
            CandidateDecision::RepeatableRegression
        );
    }

    #[test]
    fn available_candidates_match_domain_baselines() {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        let registry = CandidateRegistry::from_trusted_contract(&trusted).expect("registry");
        for domain in [
            CandidateKeyDomain::ExternalString,
            CandidateKeyDomain::ValidatedFixedKey,
            CandidateKeyDomain::CanonicalOutputOrder,
        ] {
            let baseline = run_candidate_mechanism_kernel(
                registry.baseline_id(domain).expect("baseline"),
                domain,
                512,
            )
            .expect("baseline run");
            for candidate in registry.candidates_for(domain) {
                match run_candidate_mechanism_kernel(&candidate.id, domain, 512) {
                    Ok(measurement) => {
                        assert_eq!(
                            measurement.semantic_digest_sha256, baseline.semantic_digest_sha256,
                            "{}",
                            candidate.id
                        );
                        assert_eq!(
                            measurement.lookup_checksum, baseline.lookup_checksum,
                            "{}",
                            candidate.id
                        );
                    }
                    Err(CandidateMatrixError::UnavailableCandidateFeature(_)) => {}
                    Err(error) => panic!("{} failed: {error}", candidate.id),
                }
            }
        }
    }

    #[cfg(feature = "research-runner-full")]
    #[test]
    fn every_candidate_replaces_one_component_in_the_real_corridor_pipeline() {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        let registry = CandidateRegistry::from_trusted_contract(&trusted).expect("registry");
        let mut baseline = ScalableCompilerInstance::<false>::from_trusted_contract_with_id(
            &trusted,
            "candidate-full-pipeline-baseline".to_owned(),
            ScalableWorkloadId::Corridor,
        )
        .expect("baseline compiler");
        let baseline = baseline
            .measure(GraphProfileId::WideStar, 1)
            .expect("baseline corridor pipeline");
        for key_domain in [
            CandidateKeyDomain::ExternalString,
            CandidateKeyDomain::ValidatedFixedKey,
            CandidateKeyDomain::CanonicalOutputOrder,
        ] {
            for candidate in registry.candidates_for(key_domain) {
                let configuration = CandidatePipelineConfiguration::single_candidate(
                    &trusted,
                    key_domain,
                    &candidate.id,
                )
                .expect("candidate configuration");
                let mut compiler = ScalableCompilerInstance::<false>::from_trusted_contract_with_candidate_and_allocation_ceiling(
                    &trusted,
                    format!("candidate-full-pipeline/{}/{candidate}", key_domain.as_str(), candidate = candidate.id),
                    ScalableWorkloadId::Corridor,
                    u64::MAX,
                    configuration,
                )
                .expect("candidate compiler");
                let sample = compiler
                    .measure(GraphProfileId::WideStar, 1)
                    .expect("candidate corridor pipeline");
                assert_eq!(
                    sample.semantic_digest_sha256, baseline.semantic_digest_sha256,
                    "{}",
                    candidate.id
                );
                assert_eq!(
                    sample.candidate_pipeline_checksums, baseline.candidate_pipeline_checksums,
                    "{}",
                    candidate.id
                );
            }
        }
    }

    #[test]
    fn fresh_process_measurements_must_match_the_qualified_roster() {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        let roster = build_mechanism_candidate_roster(
            &trusted,
            CandidateKeyDomain::CanonicalOutputOrder,
            128,
            &[],
        )
        .expect("qualified roster");
        let expected = roster
            .correctness_measurements
            .iter()
            .find(|measurement| measurement.candidate_id == roster.baseline_id)
            .expect("baseline measurement")
            .clone();
        validate_candidate_child_measurement(&roster, &roster.baseline_id, &expected)
            .expect("matching measurement");

        let mut divergent_digest = expected.clone();
        divergent_digest.semantic_digest_sha256 = "00".repeat(32);
        assert!(matches!(
            validate_candidate_child_measurement(&roster, &roster.baseline_id, &divergent_digest),
            Err(CandidateMatrixError::CandidateChildSemanticMismatch { .. })
        ));

        let mut divergent_lookup = expected;
        divergent_lookup.lookup_checksum ^= 1;
        assert!(matches!(
            validate_candidate_child_measurement(&roster, &roster.baseline_id, &divergent_lookup),
            Err(CandidateMatrixError::CandidateChildSemanticMismatch { .. })
        ));
    }

    #[test]
    fn malformed_safety_assessments_never_silently_override_each_other() {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        let duplicate = CandidateSafetyAssessment {
            candidate_id: "stable-vec-sort-v1".to_owned(),
            status: CandidateSafetyStatus::Passed,
            evidence: "unit-test".to_owned(),
        };
        assert!(matches!(
            build_mechanism_candidate_roster(
                &trusted,
                CandidateKeyDomain::CanonicalOutputOrder,
                128,
                &[duplicate.clone(), duplicate],
            ),
            Err(CandidateMatrixError::DuplicateSafetyAssessment(_))
        ));
        assert!(matches!(
            build_mechanism_candidate_roster(
                &trusted,
                CandidateKeyDomain::CanonicalOutputOrder,
                128,
                &[CandidateSafetyAssessment {
                    candidate_id: "stable-vec-sort-v1".to_owned(),
                    status: CandidateSafetyStatus::Passed,
                    evidence: String::new(),
                }],
            ),
            Err(CandidateMatrixError::MissingSafetyEvidence(_))
        ));
    }

    #[test]
    fn roster_preserves_an_explicit_baseline_safety_rejection() {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        let roster = build_mechanism_candidate_roster(
            &trusted,
            CandidateKeyDomain::CanonicalOutputOrder,
            128,
            &[CandidateSafetyAssessment {
                candidate_id: "stable-vec-sort-v1".to_owned(),
                status: CandidateSafetyStatus::Rejected,
                evidence: "unit-test-rejection".to_owned(),
            }],
        )
        .expect("rejected roster remains evidence");
        assert_eq!(
            roster
                .entries
                .iter()
                .find(|entry| entry.candidate_id == "stable-vec-sort-v1")
                .expect("baseline entry")
                .disposition,
            CandidateDisposition::RejectedSafety
        );
        assert!(matches!(
            validate_roster_for_performance(&roster),
            Err(CandidateMatrixError::InvalidParticipantBaseline { count: 0 })
        ));
    }

    #[cfg(feature = "research-runner-full")]
    #[test]
    fn all_fast_hashes_pass_six_run_constant_hash_qualification() {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        for candidate_id in FAST_HASH_CANDIDATES {
            let qualification =
                qualify_constant_hash_candidate(&trusted, candidate_id).expect("qualification");
            assert!(qualification.passed, "{candidate_id}");
            assert_eq!(qualification.observations.len(), 6);
            assert_eq!(
                qualification
                    .observations
                    .iter()
                    .filter(|observation| {
                        observation.role == ConstantHashRole::CandidateUnderTest
                    })
                    .count(),
                4
            );
            assert_eq!(
                qualification
                    .observations
                    .iter()
                    .filter(|observation| {
                        observation.role == ConstantHashRole::ExactResearchOracle
                    })
                    .count(),
                2
            );
            for observation in &qualification.observations {
                if observation.input_variant_id == "constant-hash-canonical-valid-v1" {
                    assert_eq!(observation.outcome, ConstantHashOutcome::Success);
                    assert_eq!(observation.error_code, None);
                    assert_eq!(observation.partial_output_record_count, 0);
                } else {
                    assert_eq!(
                        observation.input_variant_id,
                        "constant-hash-missing-reference-v1"
                    );
                    assert_eq!(observation.outcome, ConstantHashOutcome::CompilerError);
                    assert_eq!(
                        observation.error_code.as_deref(),
                        Some(UNKNOWN_REFERENCE_ERROR_CODE)
                    );
                }
            }
        }
    }

    #[test]
    fn mechanism_matrix_is_balanced_and_never_claims_production_selection() {
        let trusted = crate::load_repository_contract().expect("frozen contract");
        let registry = CandidateRegistry::from_trusted_contract(&trusted).expect("registry");
        let safety = registry
            .candidates_for(CandidateKeyDomain::CanonicalOutputOrder)
            .map(|candidate| CandidateSafetyAssessment {
                candidate_id: candidate.id.clone(),
                status: CandidateSafetyStatus::Passed,
                evidence: "unit-test".to_owned(),
            })
            .collect::<Vec<_>>();
        let execution = run_mechanism_candidate_matrix(
            &trusted,
            CandidateKeyDomain::CanonicalOutputOrder,
            512,
            &safety,
            ExactRatio::new(11, 10).expect("envelope"),
        )
        .expect("matrix");
        let candidate_count = execution.roster.participant_ids().len();
        assert_eq!(execution.scope, CANDIDATE_MATRIX_SCOPE);
        assert_eq!(execution.schedule.len(), 4 * candidate_count);
        assert_eq!(
            execution.samples.len(),
            4 * candidate_count * candidate_count
        );
        assert_eq!(execution.comparisons.len(), candidate_count - 1);
        assert!(
            execution
                .comparisons
                .iter()
                .all(|comparison| comparison.scope == CANDIDATE_MATRIX_SCOPE)
        );
    }
}
