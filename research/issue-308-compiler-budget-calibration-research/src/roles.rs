//! #308 正式研究二进制角色的闭合协议。
//!
//! 四个角色共享同一受信任契约入口，但不共享运行职责：runner 只编排与监控，
//! timing 只产生无逐分配记账的端到端墙钟，attribution 只产生受控分配与存续内存
//! 归因，oracle 只从受信任清单独立重建正确性结果。

use crate::bounded_oracle::{BoundedOracleError, verify_bounded_scalable_oracle};
use crate::stage_oracle::build_identity_stage_oracle;
use crate::{
    GraphProfileId, IdentityAllocationSnapshot, IdentityAttributionCompilerInstance,
    IdentityStageSummary, IdentityTimingCompilerInstance, ScalableTimingCompilerInstance,
    ScalableWorkloadId, StageGenerationError, StageRetainedCapacityBytes, TimingError,
    TrustedContract,
};
use serde::{Deserialize, Serialize};

pub const RUNNER_BINARY_ID: &str = "compiler-calibration-runner-v1";
pub const TIMING_BINARY_ID: &str = "compiler-calibration-timing-v1";
pub const ATTRIBUTION_BINARY_ID: &str = "compiler-calibration-attribution-v1";
pub const ORACLE_BINARY_ID: &str = "compiler-calibration-oracle-v1";

pub const IDENTITY_TIMING_CHILD_SCHEMA: &str =
    "laneflow.compiler-calibration-identity-timing-child";
pub const IDENTITY_TIMING_CHILD_SCHEMA_VERSION: u32 = 3;
pub const SCALABLE_TIMING_CHILD_SCHEMA: &str =
    "laneflow.compiler-calibration-scalable-timing-child";
pub const SCALABLE_TIMING_CHILD_SCHEMA_VERSION: u32 = 2;
pub const IDENTITY_ATTRIBUTION_CHILD_SCHEMA: &str =
    "laneflow.compiler-calibration-identity-attribution-child";
pub const IDENTITY_ATTRIBUTION_CHILD_SCHEMA_VERSION: u32 = 1;
pub const IDENTITY_ORACLE_CHILD_SCHEMA: &str =
    "laneflow.compiler-calibration-identity-oracle-child";
pub const IDENTITY_ORACLE_CHILD_SCHEMA_VERSION: u32 = 1;
pub const SCALABLE_ORACLE_CHILD_SCHEMA: &str =
    "laneflow.compiler-calibration-scalable-oracle-child";
pub const SCALABLE_ORACLE_CHILD_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResearchBinaryRole {
    Runner,
    Timing,
    Attribution,
    Oracle,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchBinaryDescriptor {
    pub binary_id: &'static str,
    pub role: ResearchBinaryRole,
    pub evidence_mode: Option<&'static str>,
    pub allocation_instrumentation_enabled: bool,
    pub responsibilities: &'static [&'static str],
}

pub fn runner_binary_descriptor() -> ResearchBinaryDescriptor {
    ResearchBinaryDescriptor {
        binary_id: RUNNER_BINARY_ID,
        role: ResearchBinaryRole::Runner,
        evidence_mode: None,
        allocation_instrumentation_enabled: false,
        responsibilities: &["orchestration", "monitoring", "evidence-assembly"],
    }
}

pub fn timing_binary_descriptor() -> ResearchBinaryDescriptor {
    ResearchBinaryDescriptor {
        binary_id: TIMING_BINARY_ID,
        role: ResearchBinaryRole::Timing,
        evidence_mode: Some("timing"),
        allocation_instrumentation_enabled:
            IdentityTimingCompilerInstance::ALLOCATION_INSTRUMENTATION_ENABLED,
        responsibilities: &["single-outer-wall-clock"],
    }
}

pub fn attribution_binary_descriptor() -> ResearchBinaryDescriptor {
    ResearchBinaryDescriptor {
        binary_id: ATTRIBUTION_BINARY_ID,
        role: ResearchBinaryRole::Attribution,
        evidence_mode: Some("attribution"),
        allocation_instrumentation_enabled:
            IdentityAttributionCompilerInstance::ALLOCATION_INSTRUMENTATION_ENABLED,
        responsibilities: &[
            "controlled-allocation",
            "live-requested-bytes",
            "peak-live-requested-bytes",
            "retained-capacity-bytes",
        ],
    }
}

pub fn oracle_binary_descriptor() -> ResearchBinaryDescriptor {
    ResearchBinaryDescriptor {
        binary_id: ORACLE_BINARY_ID,
        role: ResearchBinaryRole::Oracle,
        evidence_mode: Some("oracle"),
        allocation_instrumentation_enabled: false,
        responsibilities: &["independent-exact-correctness"],
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityTimingChildReport {
    pub schema: String,
    pub schema_version: u32,
    pub binary_id: String,
    pub allocation_instrumentation_enabled: bool,
    pub compiler_instance_id: String,
    pub child_pid: u32,
    pub graph_profile: String,
    pub n: u32,
    pub wall_time_ns: u64,
    pub semantic_digest_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScalableTimingChildReport {
    pub schema: String,
    pub schema_version: u32,
    pub binary_id: String,
    pub allocation_instrumentation_enabled: bool,
    pub compiler_instance_id: String,
    pub child_pid: u32,
    pub workload_id: ScalableWorkloadId,
    pub workload_revision: u32,
    pub graph_profile: String,
    pub string_profile: String,
    pub generator_version: u32,
    pub n: u32,
    pub outcome: ScalableTimingOutcome,
    pub controlled_allocation_hard_ceiling_bytes: u64,
    pub guard_peak_live_requested_bytes: Option<u64>,
    pub wall_time_ns: Option<u64>,
    pub semantic_digest_sha256: Option<String>,
    pub controlled_allocation_guard: Option<ControlledAllocationGuardReport>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScalableTimingOutcome {
    Success,
    GuardedInChild,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IdentityAttributionOutcome {
    Success,
    GuardedInChild,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityAttributionChildReport {
    pub schema: String,
    pub schema_version: u32,
    pub binary_id: String,
    pub allocation_instrumentation_enabled: bool,
    pub compiler_instance_id: String,
    pub child_pid: u32,
    pub graph_profile: String,
    pub n: u32,
    pub outcome: IdentityAttributionOutcome,
    pub controlled_allocation_hard_ceiling_bytes: u64,
    pub allocation: IdentityAllocationSnapshot,
    pub attribution_wall_time_ns_diagnostic: Option<u64>,
    pub retained_capacity_bytes: Option<StageRetainedCapacityBytes>,
    pub semantic_digest_sha256: Option<String>,
    pub controlled_allocation_guard: Option<ControlledAllocationGuardReport>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlledAllocationGuardReport {
    pub field: String,
    pub hard_ceiling_bytes: u64,
    pub live_requested_bytes: u64,
    pub requested_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityOracleChildReport {
    pub schema: String,
    pub schema_version: u32,
    pub binary_id: String,
    pub child_pid: u32,
    pub graph_profile: String,
    pub n: u32,
    pub stage_summary: IdentityStageSummary,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScalableOracleChildReport {
    pub schema: String,
    pub schema_version: u32,
    pub binary_id: String,
    pub oracle_run_id: String,
    pub child_pid: u32,
    pub workload_id: ScalableWorkloadId,
    pub workload_revision: u32,
    pub graph_profile: String,
    pub string_profile: String,
    pub generator_version: u32,
    pub n: u32,
    pub outcome: ScalableOracleOutcome,
    pub controlled_allocation_hard_ceiling_bytes: u64,
    pub guard_peak_live_requested_bytes: Option<u64>,
    pub primary_record_count: Option<u64>,
    pub semantic_digest_sha256: Option<String>,
    pub complete_counts_equal: bool,
    pub complete_typed_output_equal: bool,
    pub controlled_allocation_guard: Option<ControlledAllocationGuardReport>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScalableOracleOutcome {
    Success,
    GuardedInChild,
}

pub fn measure_identity_timing_child(
    trusted: &TrustedContract,
    compiler_instance_id: String,
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<IdentityTimingChildReport, RoleExecutionError> {
    let mut instance = IdentityTimingCompilerInstance::from_trusted_contract_with_id(
        trusted,
        compiler_instance_id.clone(),
    )?;
    let sample = instance.measure(graph_profile, n)?;
    Ok(IdentityTimingChildReport {
        schema: IDENTITY_TIMING_CHILD_SCHEMA.to_owned(),
        schema_version: IDENTITY_TIMING_CHILD_SCHEMA_VERSION,
        binary_id: TIMING_BINARY_ID.to_owned(),
        allocation_instrumentation_enabled:
            IdentityTimingCompilerInstance::ALLOCATION_INSTRUMENTATION_ENABLED,
        compiler_instance_id,
        child_pid: std::process::id(),
        graph_profile: graph_profile.as_str().to_owned(),
        n,
        wall_time_ns: sample.wall_time_ns,
        semantic_digest_sha256: sample.stage_summary.semantic_digest_sha256,
    })
}

pub fn measure_scalable_timing_child(
    trusted: &TrustedContract,
    compiler_instance_id: String,
    workload_id: ScalableWorkloadId,
    graph_profile: GraphProfileId,
    n: u32,
    controlled_allocation_hard_ceiling_bytes: u64,
) -> Result<ScalableTimingChildReport, RoleExecutionError> {
    let mut instance =
        ScalableTimingCompilerInstance::from_trusted_contract_with_id_and_allocation_ceiling(
            trusted,
            compiler_instance_id.clone(),
            workload_id,
            controlled_allocation_hard_ceiling_bytes,
        )?;
    let base = || ScalableTimingChildReport {
        schema: SCALABLE_TIMING_CHILD_SCHEMA.to_owned(),
        schema_version: SCALABLE_TIMING_CHILD_SCHEMA_VERSION,
        binary_id: TIMING_BINARY_ID.to_owned(),
        allocation_instrumentation_enabled:
            IdentityTimingCompilerInstance::ALLOCATION_INSTRUMENTATION_ENABLED,
        compiler_instance_id: compiler_instance_id.clone(),
        child_pid: std::process::id(),
        workload_id,
        workload_revision: crate::WORKLOAD_REVISION_V1,
        graph_profile: graph_profile.as_str().to_owned(),
        string_profile: crate::BASE_SCALE_STRING_PROFILE.to_owned(),
        generator_version: crate::GENERATOR_VERSION_V1,
        n,
        outcome: ScalableTimingOutcome::Success,
        controlled_allocation_hard_ceiling_bytes,
        guard_peak_live_requested_bytes: None,
        wall_time_ns: None,
        semantic_digest_sha256: None,
        controlled_allocation_guard: None,
    };
    match instance.measure(graph_profile, n) {
        Ok(sample) => Ok(ScalableTimingChildReport {
            workload_id: sample.workload_id,
            graph_profile: sample.graph_profile.as_str().to_owned(),
            n: sample.n,
            guard_peak_live_requested_bytes: Some(sample.guard_peak_live_requested_bytes),
            wall_time_ns: Some(sample.wall_time_ns),
            semantic_digest_sha256: Some(sample.semantic_digest_sha256),
            ..base()
        }),
        Err(TimingError::StageGeneration(
            StageGenerationError::ControlledAllocationHardCeiling {
                field,
                hard_ceiling_bytes,
                live_requested_bytes,
                requested_bytes,
            },
        )) => Ok(ScalableTimingChildReport {
            outcome: ScalableTimingOutcome::GuardedInChild,
            controlled_allocation_guard: Some(ControlledAllocationGuardReport {
                field: field.to_owned(),
                hard_ceiling_bytes,
                live_requested_bytes,
                requested_bytes,
            }),
            ..base()
        }),
        Err(error) => Err(error.into()),
    }
}

pub fn measure_identity_attribution_child(
    trusted: &TrustedContract,
    compiler_instance_id: String,
    graph_profile: GraphProfileId,
    n: u32,
    controlled_allocation_hard_ceiling_bytes: u64,
) -> Result<IdentityAttributionChildReport, RoleExecutionError> {
    let mut instance =
        IdentityAttributionCompilerInstance::from_trusted_contract_with_id_and_allocation_ceiling(
            trusted,
            compiler_instance_id.clone(),
            controlled_allocation_hard_ceiling_bytes,
        )?;
    let result = instance.measure(graph_profile, n);
    let allocation = instance.allocation_snapshot();
    let base = || IdentityAttributionChildReport {
        schema: IDENTITY_ATTRIBUTION_CHILD_SCHEMA.to_owned(),
        schema_version: IDENTITY_ATTRIBUTION_CHILD_SCHEMA_VERSION,
        binary_id: ATTRIBUTION_BINARY_ID.to_owned(),
        allocation_instrumentation_enabled:
            IdentityAttributionCompilerInstance::ALLOCATION_INSTRUMENTATION_ENABLED,
        compiler_instance_id: compiler_instance_id.clone(),
        child_pid: std::process::id(),
        graph_profile: graph_profile.as_str().to_owned(),
        n,
        outcome: IdentityAttributionOutcome::Success,
        controlled_allocation_hard_ceiling_bytes,
        allocation,
        attribution_wall_time_ns_diagnostic: None,
        retained_capacity_bytes: None,
        semantic_digest_sha256: None,
        controlled_allocation_guard: None,
    };
    match result {
        Ok(sample) => Ok(IdentityAttributionChildReport {
            attribution_wall_time_ns_diagnostic: Some(sample.wall_time_ns),
            retained_capacity_bytes: Some(instance.retained_capacity_bytes()?),
            semantic_digest_sha256: Some(sample.stage_summary.semantic_digest_sha256),
            ..base()
        }),
        Err(TimingError::StageGeneration(
            StageGenerationError::ControlledAllocationHardCeiling {
                field,
                hard_ceiling_bytes,
                live_requested_bytes,
                requested_bytes,
            },
        )) => Ok(IdentityAttributionChildReport {
            outcome: IdentityAttributionOutcome::GuardedInChild,
            controlled_allocation_guard: Some(ControlledAllocationGuardReport {
                field: field.to_owned(),
                hard_ceiling_bytes,
                live_requested_bytes,
                requested_bytes,
            }),
            ..base()
        }),
        Err(error) => Err(error.into()),
    }
}

pub fn build_identity_oracle_child(
    trusted: &TrustedContract,
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<IdentityOracleChildReport, RoleExecutionError> {
    let stage_summary = build_identity_stage_oracle(&trusted.workload_manifest, graph_profile, n)?;
    Ok(IdentityOracleChildReport {
        schema: IDENTITY_ORACLE_CHILD_SCHEMA.to_owned(),
        schema_version: IDENTITY_ORACLE_CHILD_SCHEMA_VERSION,
        binary_id: ORACLE_BINARY_ID.to_owned(),
        child_pid: std::process::id(),
        graph_profile: graph_profile.as_str().to_owned(),
        n,
        stage_summary,
    })
}

pub fn verify_scalable_oracle_child(
    trusted: &TrustedContract,
    oracle_run_id: String,
    workload_id: ScalableWorkloadId,
    graph_profile: GraphProfileId,
    n: u32,
    controlled_allocation_hard_ceiling_bytes: u64,
) -> Result<ScalableOracleChildReport, RoleExecutionError> {
    let base = || ScalableOracleChildReport {
        schema: SCALABLE_ORACLE_CHILD_SCHEMA.to_owned(),
        schema_version: SCALABLE_ORACLE_CHILD_SCHEMA_VERSION,
        binary_id: ORACLE_BINARY_ID.to_owned(),
        oracle_run_id: oracle_run_id.clone(),
        child_pid: std::process::id(),
        workload_id,
        workload_revision: crate::WORKLOAD_REVISION_V1,
        graph_profile: graph_profile.as_str().to_owned(),
        string_profile: crate::BASE_SCALE_STRING_PROFILE.to_owned(),
        generator_version: crate::GENERATOR_VERSION_V1,
        n,
        outcome: ScalableOracleOutcome::Success,
        controlled_allocation_hard_ceiling_bytes,
        guard_peak_live_requested_bytes: None,
        primary_record_count: None,
        semantic_digest_sha256: None,
        complete_counts_equal: false,
        complete_typed_output_equal: false,
        controlled_allocation_guard: None,
    };
    match verify_bounded_scalable_oracle(
        trusted,
        workload_id,
        graph_profile,
        n,
        controlled_allocation_hard_ceiling_bytes,
    ) {
        Ok(verification) => Ok(ScalableOracleChildReport {
            guard_peak_live_requested_bytes: Some(
                verification.allocation.peak_live_requested_bytes,
            ),
            primary_record_count: Some(verification.primary_record_count),
            semantic_digest_sha256: Some(verification.semantic_digest_sha256),
            complete_counts_equal: verification.complete_counts_equal,
            complete_typed_output_equal: verification.complete_typed_output_equal,
            ..base()
        }),
        Err(BoundedOracleError::Generation(
            StageGenerationError::ControlledAllocationHardCeiling {
                field,
                hard_ceiling_bytes,
                live_requested_bytes,
                requested_bytes,
            },
        )) => Ok(ScalableOracleChildReport {
            outcome: ScalableOracleOutcome::GuardedInChild,
            controlled_allocation_guard: Some(ControlledAllocationGuardReport {
                field: field.to_owned(),
                hard_ceiling_bytes,
                live_requested_bytes,
                requested_bytes,
            }),
            ..base()
        }),
        Err(error) => Err(RoleExecutionError::BoundedOracle(error.to_string())),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RoleExecutionError {
    #[error(transparent)]
    Timing(#[from] TimingError),
    #[error(transparent)]
    Oracle(#[from] crate::StageOracleError),
    #[error(transparent)]
    ScalePlan(#[from] crate::ScalePlanError),
    #[error(transparent)]
    IdentityOracle(#[from] crate::OracleVerificationError),
    #[error(transparent)]
    CorridorOracle(#[from] crate::CorridorOracleError),
    #[error(transparent)]
    JunctionGridOracle(#[from] crate::JunctionGridOracleError),
    #[error("受控独立预言机失败：{0}")]
    BoundedOracle(String),
}
