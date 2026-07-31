//! #308 私有容器与哈希候选的非生产机制级资格和性能矩阵。
//!
//! 输入构造和语义摘要在计时区外执行；单一计时区只覆盖候选容器、哈希或排序操作。
//! 本模块的分类只用于机制归因，不是 #292 的生产实现选择，也不替代完整研究管线证据。

use crate::corridor::CorridorContract;
use crate::{GraphProfileId, TrustedContract, build_corridor_stage_summary, repository_root};
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
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::time::Instant;

pub const CANDIDATE_MATRIX_SCOPE: &str = "mechanism-only-not-production-selection";
pub const CANDIDATE_KERNEL_CHILD_SCHEMA: &str =
    "laneflow.compiler-calibration-candidate-kernel-child";
pub const CANDIDATE_KERNEL_CHILD_SCHEMA_VERSION: u32 = 1;
const CANDIDATE_REGISTRY_REVISION: u32 = 1;
const FIXED_HASHER_SEED: u64 = 0x4c46_434f_4d50_0001;
#[cfg(any(test, feature = "candidate-hashbrown-fnv1a64"))]
const FNV1A64_OFFSET_BASIS: u64 = 14_695_981_039_346_656_037;
#[cfg(any(test, feature = "candidate-hashbrown-fnv1a64"))]
const FNV1A64_PRIME: u64 = 1_099_511_628_211;
const UNKNOWN_REFERENCE_ERROR_CODE: &str = "LF-COMP-RESEARCH-E-UNKNOWN-REFERENCE";
const FAST_HASH_CANDIDATES: [&str; 3] = [
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

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CandidateSafetyStatus {
    Passed,
    Rejected,
    Unavailable,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidateSafetyAssessment {
    pub candidate_id: String,
    pub status: CandidateSafetyStatus,
    pub evidence: String,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
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

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ConstantHashOutcome {
    Success,
    CompilerError,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ConstantHashRole {
    CandidateUnderTest,
    ExactResearchOracle,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
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

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConstantHashQualification {
    pub qualification_id: String,
    pub candidate_id: String,
    pub protocol_id: &'static str,
    pub candidate_builder_id: &'static str,
    pub oracle_builder_id: &'static str,
    pub observations: Vec<ConstantHashObservation>,
    pub passed: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
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
                        if measurement.semantic_digest_sha256
                            != baseline_measurement.semantic_digest_sha256
                            || measurement.lookup_checksum != baseline_measurement.lookup_checksum
                        {
                            entry.disposition = CandidateDisposition::RejectedCorrectness;
                            entry.reason = Some("mechanism-semantic-mismatch".to_owned());
                        } else if FAST_HASH_CANDIDATES.contains(&candidate.id.as_str()) {
                            let qualification =
                                qualify_constant_hash_candidate(trusted, &candidate.id)?;
                            entry.constant_hash_qualification_id =
                                Some(qualification.qualification_id.clone());
                            if qualification.passed {
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
                            constant_hash_qualifications.push(qualification);
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
    let participants = roster.participant_ids();
    let schedule = build_two_batch_balanced_schedule(&participants)?;
    let mut samples = Vec::new();
    for round in &schedule {
        for (position, candidate_id) in round.participant_order.iter().enumerate() {
            let child =
                run_candidate_kernel_child(timing_binary, candidate_id, key_domain, item_count)?;
            samples.push(CandidatePerformanceSample {
                batch: round.batch,
                round: round.round,
                position: u32::try_from(position).expect("candidate position must fit u32"),
                child_pid: Some(child.child_pid),
                binary_id: Some(child.binary_id),
                measurement: child.measurement,
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
        execution_mode: CandidateExecutionMode::FreshProcessTiming,
        roster,
        schedule,
        samples,
        comparisons,
    })
}

fn run_candidate_kernel_child(
    timing_binary: &Path,
    candidate_id: &str,
    key_domain: CandidateKeyDomain,
    item_count: u32,
) -> Result<CandidateKernelChildReport, CandidateMatrixError> {
    let mut child = Command::new(timing_binary)
        .arg("run-candidate-kernel")
        .arg(candidate_id)
        .arg(key_domain.as_str())
        .arg(item_count.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| CandidateMatrixError::SpawnCandidateChild {
            path: timing_binary.to_path_buf(),
            source,
        })?;
    let child_pid = child.id();
    let release_result = child
        .stdin
        .take()
        .ok_or(CandidateMatrixError::MissingCandidateChildStdin)
        .and_then(|mut stdin| {
            stdin
                .write_all(b"G")
                .map_err(CandidateMatrixError::ReleaseCandidateChild)
        });
    if let Err(error) = release_result {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let output = child
        .wait_with_output()
        .map_err(CandidateMatrixError::WaitCandidateChild)?;
    if !output.status.success() {
        return Err(CandidateMatrixError::CandidateChildExit {
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let report: CandidateKernelChildReport = serde_json::from_slice(&output.stdout)
        .map_err(CandidateMatrixError::InvalidCandidateChildJson)?;
    if report.schema != CANDIDATE_KERNEL_CHILD_SCHEMA
        || report.schema_version != CANDIDATE_KERNEL_CHILD_SCHEMA_VERSION
        || report.binary_id != crate::roles::TIMING_BINARY_ID
        || report.child_pid != child_pid
        || report.measurement.scope != CANDIDATE_MATRIX_SCOPE
        || report.measurement.candidate_id != candidate_id
        || report.measurement.key_domain != key_domain
        || report.measurement.item_count != item_count
    {
        return Err(CandidateMatrixError::CandidateChildProtocol);
    }
    Ok(report)
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
    let contract = CorridorContract::from_manifest(&trusted.workload_manifest)
        .map_err(|error| CandidateMatrixError::Corridor(error.to_string()))?;
    let template = contract
        .load_template(&repository_root())
        .map_err(|error| CandidateMatrixError::Corridor(error.to_string()))?;
    let summary = build_corridor_stage_summary(trusted, GraphProfileId::WideStar, 1)
        .map_err(|error| CandidateMatrixError::Corridor(error.to_string()))?;
    let stage_counts_digest = sha256_json(&(&summary.counts, &summary.stages))?;
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
    let input = ConstantHashInput {
        declarations,
        references,
        canonical_semantic_digest: summary.semantic_digest_sha256,
        stage_counts_digest,
    };

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
    let passed = constant_hash_observations_match(&observations);
    Ok(ConstantHashQualification {
        qualification_id,
        candidate_id: candidate_id.to_owned(),
        protocol_id: "constant-hash-full-key-equality-v1",
        candidate_builder_id: "all-keys-u64-zero-v1",
        oracle_builder_id: "exact-research-oracle-v1",
        observations,
        passed,
    })
}

struct ConstantHashInput {
    declarations: BTreeMap<u64, u64>,
    references: Vec<u64>,
    canonical_semantic_digest: String,
    stage_counts_digest: String,
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
                diagnostic_digest_sha256: sha256_hex(&[]),
                partial_output_record_count: u64::try_from(values.len())
                    .expect("reference count must fit u64"),
            }
        }
        Err((missing_key, partial)) => {
            let diagnostic = format!("{UNKNOWN_REFERENCE_ERROR_CODE}:{missing_key:016x}");
            ConstantHashObservation {
                observation_id: format!("{qualification_id}/{variant_id}/{role_id}/{repeat}"),
                role,
                input_variant_id: variant_id.to_owned(),
                repeat,
                outcome: ConstantHashOutcome::CompilerError,
                error_code: Some(UNKNOWN_REFERENCE_ERROR_CODE.to_owned()),
                stage_counts_digest_sha256: input.stage_counts_digest.clone(),
                semantic_digest_sha256: sha256_hex(&[]),
                diagnostic_digest_sha256: sha256_hex(diagnostic.as_bytes()),
                partial_output_record_count: u64::try_from(partial.len())
                    .expect("partial reference count must fit u64"),
            }
        }
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
struct Fnv1a64BuildHasher;

#[cfg(feature = "candidate-hashbrown-fnv1a64")]
impl BuildHasher for Fnv1a64BuildHasher {
    type Hasher = Fnv1a64Hasher;

    fn build_hasher(&self) -> Self::Hasher {
        Fnv1a64Hasher(FNV1A64_OFFSET_BASIS)
    }
}

#[cfg(feature = "candidate-hashbrown-fnv1a64")]
#[derive(Clone, Copy, Debug)]
struct Fnv1a64Hasher(u64);

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
    #[error("候选 {0} 不适用恒定哈希资格")]
    QualificationNotApplicable(String),
    #[error("候选机制项数必须大于零")]
    ZeroItemCount,
    #[error("机制计时结果为零纳秒：{0}")]
    ZeroDuration(String),
    #[error("机制计时纳秒数溢出 u64")]
    DurationOverflow,
    #[error("机制内核不支持键域 {0:?}")]
    UnsupportedMechanismDomain(CandidateKeyDomain),
    #[error("未知候选键域 {0}")]
    UnknownKeyDomain(String),
    #[error("未找到候选计时角色二进制 {0}")]
    MissingTimingBinary(std::path::PathBuf),
    #[error("无法启动候选计时角色 {path}: {source}")]
    SpawnCandidateChild {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("候选计时角色没有可用 stdin")]
    MissingCandidateChildStdin,
    #[error("无法释放候选计时角色: {0}")]
    ReleaseCandidateChild(std::io::Error),
    #[error("无法等待候选计时角色: {0}")]
    WaitCandidateChild(std::io::Error),
    #[error("候选计时角色异常退出：code={code:?}, stderr={stderr}")]
    CandidateChildExit { code: Option<i32>, stderr: String },
    #[error("候选计时角色没有返回有效 JSON: {0}")]
    InvalidCandidateChildJson(serde_json::Error),
    #[error("候选计时角色返回的身份或测量分层不匹配")]
    CandidateChildProtocol,
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
