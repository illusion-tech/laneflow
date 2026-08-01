//! 正式外层计时区的最小测量原语。
//!
//! 本模块建立 #308 协议要求的 prepare -> timed execute -> finalize 边界，并由
//! 同一个编译器实例只保留已清空的阶段容器容量。计时与归因实例通过编译期常量分别
//! 单态化：正式计时路径只执行停止护栏所必需的受控分配预占，不累计分配归因指标；
//! 归因路径额外记录分配、释放与存续峰值。独立正确性验证不进入这两个实例；正式轮次
//! 和证据写出仍由后续 runner 切片负责。

use crate::bounded_template::{
    BoundedTemplateBufferPool, BoundedTemplateExecution, BoundedTemplateFailureMode,
    execute_bounded_template_failure_case_with_pool,
    execute_bounded_template_stage_case_with_pool_and_candidate,
    finalize_bounded_template_stage_case_with_pool,
};
use crate::candidate_matrix::{CandidatePipelineChecksums, CandidatePipelineConfiguration};
use crate::controlled_alloc::ControlledAllocator;
use crate::corridor::{CorridorContract, CorridorTemplate};
use crate::junction_grid::{JunctionGridContract, build_junction_grid_template};
use crate::pipeline::{
    IdentityAllocationSnapshot, IdentityStageBufferPool, IdentityStageMaterialization,
    execute_identity_stage_case_with_buffers, finalize_identity_stage_case,
    prepare_identity_stage_case, recycle_identity_stage_case,
};
use crate::stage::{IdentityStagePlan, IdentityStageSummary, StageRetainedCapacityBytes};
use crate::stage_oracle::build_identity_stage_oracle;
use crate::{
    DIAGNOSTIC_LIMIT_ERROR_CODE, DUPLICATE_OWNER_ERROR_CODE, GeneratorContract, GraphProfileId,
    IdentityContract, LIMIT_EXCEEDED_ERROR_CODE, LimitDimensionId, ScalableWorkloadId,
    StageContract, TrustedContract, UNKNOWN_REFERENCE_ERROR_CODE,
};
use crate::{ScalableStagePlanFactory, ScalableStagePlanSummary};
use serde::Serialize;
use std::hint::black_box;
use std::time::Instant;

pub const CLOCK_QUANTUM_OBSERVATION_COUNT: u32 = 100_000;
pub const STABLE_CAPACITY_WARMUP_COUNT: u32 = 3;
pub const STABLE_CAPACITY_SAMPLE_COUNT: u32 = 7;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityTimingSample {
    pub wall_time_ns: u64,
    pub stage_summary: IdentityStageSummary,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityStableCapacitySequence {
    pub cold_instance: IdentityTimingSample,
    pub stable_capacity_reuse: Vec<IdentityTimingSample>,
    pub retained_capacity_bytes: StageRetainedCapacityBytes,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScalableTimingSample {
    pub workload_id: ScalableWorkloadId,
    pub graph_profile: GraphProfileId,
    pub n: u32,
    pub wall_time_ns: u64,
    pub semantic_digest_sha256: String,
    pub guard_peak_live_requested_bytes: u64,
    pub allocation: IdentityAllocationSnapshot,
    pub candidate_pipeline_checksums: CandidatePipelineChecksums,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScalableStableCapacitySequence {
    pub cold_instance: ScalableTimingSample,
    pub stable_capacity_reuse: Vec<ScalableTimingSample>,
    pub retained_capacity_bytes: StageRetainedCapacityBytes,
    pub post_warmup_allocation: IdentityAllocationSnapshot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScalableFailureInput {
    SourceByteLimitPlusOne { selected_limit_value: u64 },
    MissingReferencePerUnit,
    MissingReferencePerUnitWithMaximumDiagnostics { maximum_diagnostics: u64 },
    DuplicateOwnerPerUnit,
    DiagnosticCapPlusOne { maximum_diagnostics: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScalableFailureReport {
    pub(crate) stable_compiler_error_code: &'static str,
    pub(crate) diagnostic_count: u64,
    pub(crate) diagnostics_truncated: bool,
    pub(crate) partial_output_record_count: u64,
    pub(crate) output_record_count: u64,
    pub(crate) live_requested_bytes_after_run: u64,
    pub(crate) diagnostic_digest_sha256: String,
}

#[derive(Debug)]
pub struct ScalableCompilerInstance<const TRACK_ALLOCATIONS: bool> {
    compiler_instance_id: String,
    controlled_allocation_hard_ceiling_bytes: u64,
    limit_manifest: serde_json::Value,
    plans: ScalableStagePlanFactory,
    inner: ScalableCompilerInner<TRACK_ALLOCATIONS>,
    completed_compilations: u32,
}

#[derive(Debug)]
pub struct ScalablePreparedMeasurement {
    workload_id: ScalableWorkloadId,
    graph_profile: GraphProfileId,
    n: u32,
    inner: ScalablePreparedMeasurementInner,
}

#[derive(Debug)]
enum ScalablePreparedMeasurementInner {
    Identity(Box<IdentityStagePlan>),
    Corridor(Box<ScalableStagePlanSummary>),
    JunctionGrid(Box<ScalableStagePlanSummary>),
}

#[derive(Debug)]
pub struct ScalableExecutedMeasurement {
    workload_id: ScalableWorkloadId,
    graph_profile: GraphProfileId,
    n: u32,
    wall_time_ns: u64,
    guard_peak_live_requested_bytes: u64,
    inner: ScalableExecutedMeasurementInner,
}

#[derive(Debug)]
enum ScalableExecutedMeasurementInner {
    Identity {
        plan: Box<IdentityStagePlan>,
        materialized: IdentityStageMaterialization,
    },
    Corridor(BoundedTemplateExecution),
    JunctionGrid(BoundedTemplateExecution),
}

#[derive(Debug)]
enum ScalableCompilerInner<const TRACK_ALLOCATIONS: bool> {
    Identity(Box<IdentityCompilerInstance<TRACK_ALLOCATIONS>>),
    Corridor(TemplateTimingCompilerInstance<CorridorContract>),
    JunctionGrid(TemplateTimingCompilerInstance<JunctionGridContract>),
}

#[derive(Debug)]
struct TemplateTimingCompilerInstance<C> {
    generator: GeneratorContract,
    identity: IdentityContract,
    stage: StageContract,
    contract: C,
    template: CorridorTemplate,
    buffers: Option<BoundedTemplateBufferPool>,
    candidate_configuration: CandidatePipelineConfiguration,
}

#[derive(Debug)]
pub struct IdentityCompilerInstance<const TRACK_ALLOCATIONS: bool> {
    compiler_instance_id: Option<String>,
    generator: GeneratorContract,
    identity: IdentityContract,
    stage: StageContract,
    buffers: IdentityStageBufferPool<TRACK_ALLOCATIONS>,
    completed_compilations: u32,
}

pub type IdentityTimingCompilerInstance = IdentityCompilerInstance<false>;
pub type IdentityAttributionCompilerInstance = IdentityCompilerInstance<true>;
pub type ScalableTimingCompilerInstance = ScalableCompilerInstance<false>;
pub type ScalableAttributionCompilerInstance = ScalableCompilerInstance<true>;

impl IdentityCompilerInstance<false> {
    pub fn from_trusted_contract(trusted: &TrustedContract) -> Result<Self, TimingError> {
        Self::from_trusted_contract_with_optional_id_and_allocation_ceiling(trusted, None, u64::MAX)
    }

    pub fn from_trusted_contract_with_id(
        trusted: &TrustedContract,
        compiler_instance_id: String,
    ) -> Result<Self, TimingError> {
        if compiler_instance_id.is_empty() {
            return Err(TimingError::EmptyCompilerInstanceId);
        }
        Self::from_trusted_contract_with_optional_id_and_allocation_ceiling(
            trusted,
            Some(compiler_instance_id),
            u64::MAX,
        )
    }

    pub fn from_trusted_contract_with_id_and_allocation_ceiling(
        trusted: &TrustedContract,
        compiler_instance_id: String,
        controlled_allocation_hard_ceiling_bytes: u64,
    ) -> Result<Self, TimingError> {
        if compiler_instance_id.is_empty() {
            return Err(TimingError::EmptyCompilerInstanceId);
        }
        Self::from_trusted_contract_with_optional_id_and_allocation_ceiling(
            trusted,
            Some(compiler_instance_id),
            controlled_allocation_hard_ceiling_bytes,
        )
    }

    pub fn controlled_allocation_hard_ceiling_bytes(&self) -> u64 {
        self.buffers.controlled_allocation_hard_ceiling_bytes()
    }

    pub fn guard_peak_live_requested_bytes(&self) -> u64 {
        self.buffers.peak_live_requested_bytes()
    }
}

impl IdentityCompilerInstance<true> {
    pub fn from_trusted_contract_with_id_and_allocation_ceiling(
        trusted: &TrustedContract,
        compiler_instance_id: String,
        controlled_allocation_hard_ceiling_bytes: u64,
    ) -> Result<Self, TimingError> {
        if compiler_instance_id.is_empty() {
            return Err(TimingError::EmptyCompilerInstanceId);
        }
        Self::from_trusted_contract_with_optional_id_and_allocation_ceiling(
            trusted,
            Some(compiler_instance_id),
            controlled_allocation_hard_ceiling_bytes,
        )
    }

    pub fn controlled_allocation_hard_ceiling_bytes(&self) -> u64 {
        self.buffers.controlled_allocation_hard_ceiling_bytes()
    }

    pub fn peak_live_requested_bytes(&self) -> u64 {
        self.buffers.peak_live_requested_bytes()
    }
}

impl<const TRACK_ALLOCATIONS: bool> IdentityCompilerInstance<TRACK_ALLOCATIONS> {
    pub const ALLOCATION_INSTRUMENTATION_ENABLED: bool = TRACK_ALLOCATIONS;

    fn from_trusted_contract_with_optional_id_and_allocation_ceiling(
        trusted: &TrustedContract,
        compiler_instance_id: Option<String>,
        controlled_allocation_hard_ceiling_bytes: u64,
    ) -> Result<Self, TimingError> {
        Ok(Self {
            compiler_instance_id,
            generator: trusted.generator_contract()?,
            identity: trusted.identity_contract()?,
            stage: trusted.stage_contract()?,
            buffers: IdentityStageBufferPool::new_for_mode(
                controlled_allocation_hard_ceiling_bytes,
            ),
            completed_compilations: 0,
        })
    }

    pub fn compiler_instance_id(&self) -> Option<&str> {
        self.compiler_instance_id.as_deref()
    }

    pub fn allocation_snapshot(&self) -> IdentityAllocationSnapshot {
        self.buffers.allocation_snapshot()
    }

    pub fn run_stable_capacity_sequence(
        &mut self,
        graph_profile: GraphProfileId,
        n: u32,
    ) -> Result<IdentityStableCapacitySequence, TimingError> {
        if self.completed_compilations != 0 {
            return Err(TimingError::CompilerInstanceAlreadyUsed);
        }
        let cold_instance = self.measure(graph_profile, n)?;
        let retained_capacity_bytes = self.retained_capacity_bytes()?;
        for _ in 0..STABLE_CAPACITY_WARMUP_COUNT {
            let warmup = self.run_unmeasured(graph_profile, n)?;
            if warmup != cold_instance.stage_summary {
                return Err(TimingError::StableCapacitySummaryChanged);
            }
            if self.retained_capacity_bytes()? != retained_capacity_bytes {
                return Err(TimingError::StableCapacityChanged);
            }
        }
        let mut stable_capacity_reuse = Vec::with_capacity(STABLE_CAPACITY_SAMPLE_COUNT as usize);
        for _ in 0..STABLE_CAPACITY_SAMPLE_COUNT {
            let sample = self.measure(graph_profile, n)?;
            if sample.stage_summary != cold_instance.stage_summary {
                return Err(TimingError::StableCapacitySummaryChanged);
            }
            if self.retained_capacity_bytes()? != retained_capacity_bytes {
                return Err(TimingError::StableCapacityChanged);
            }
            stable_capacity_reuse.push(sample);
        }
        Ok(IdentityStableCapacitySequence {
            cold_instance,
            stable_capacity_reuse,
            retained_capacity_bytes,
        })
    }

    pub fn measure(
        &mut self,
        graph_profile: GraphProfileId,
        n: u32,
    ) -> Result<IdentityTimingSample, TimingError> {
        self.buffers.begin_request()?;
        let plan = prepare_identity_stage_case(&self.identity, &self.stage, graph_profile, n)?;

        // 正式墙钟样本只允许这一对外层时钟读取。
        let started = Instant::now();
        let materialized = self.execute_with_failure_recovery(&plan)?;
        black_box(materialized.output_construction.as_slice());
        let elapsed = started.elapsed();

        let stage_summary = self.finalize_and_recycle(&plan, materialized)?;
        Ok(IdentityTimingSample {
            wall_time_ns: u64::try_from(elapsed.as_nanos())
                .map_err(|_| TimingError::ClockDurationOverflow)?,
            stage_summary,
        })
    }

    pub fn run_unmeasured(
        &mut self,
        graph_profile: GraphProfileId,
        n: u32,
    ) -> Result<IdentityStageSummary, TimingError> {
        self.buffers.begin_request()?;
        let plan = prepare_identity_stage_case(&self.identity, &self.stage, graph_profile, n)?;
        let materialized = self.execute_with_failure_recovery(&plan)?;
        self.finalize_and_recycle(&plan, materialized)
    }

    pub fn retained_capacity_bytes(&self) -> Result<StageRetainedCapacityBytes, TimingError> {
        Ok(self.buffers.retained_capacity_bytes()?)
    }

    fn finalize_and_recycle(
        &mut self,
        plan: &crate::stage::IdentityStagePlan,
        materialized: crate::pipeline::IdentityStageMaterialization,
    ) -> Result<IdentityStageSummary, TimingError> {
        // 摘要、完整形状检查和容量回收均在停表后执行；独立预言机属于 oracle 角色。
        let produced = match finalize_identity_stage_case(plan, materialized) {
            Ok(produced) => produced,
            Err(source) => {
                self.buffers.reconcile_dropped_allocations_after_failure()?;
                return Err(source.into());
            }
        };
        let summary = recycle_identity_stage_case(&mut self.buffers, produced);
        if !self.buffers.all_lengths_are_zero() {
            return Err(TimingError::RetainedSemanticState);
        }
        self.completed_compilations = self
            .completed_compilations
            .checked_add(1)
            .ok_or(TimingError::CompilerInstanceCompilationOverflow)?;
        Ok(summary)
    }

    fn execute_with_failure_recovery(
        &mut self,
        plan: &crate::stage::IdentityStagePlan,
    ) -> Result<crate::pipeline::IdentityStageMaterialization, TimingError> {
        match execute_identity_stage_case_with_buffers(
            &self.generator,
            &self.identity,
            &self.stage,
            plan,
            &mut self.buffers,
        ) {
            Ok(materialized) => Ok(materialized),
            Err(source) => {
                self.buffers.reconcile_dropped_allocations_after_failure()?;
                Err(source.into())
            }
        }
    }
}

impl<const TRACK_ALLOCATIONS: bool> ScalableCompilerInstance<TRACK_ALLOCATIONS> {
    pub fn from_trusted_contract_with_id(
        trusted: &TrustedContract,
        compiler_instance_id: String,
        workload_id: ScalableWorkloadId,
    ) -> Result<Self, TimingError> {
        Self::from_trusted_contract_with_id_and_allocation_ceiling(
            trusted,
            compiler_instance_id,
            workload_id,
            u64::MAX,
        )
    }

    pub fn from_trusted_contract_with_id_and_allocation_ceiling(
        trusted: &TrustedContract,
        compiler_instance_id: String,
        workload_id: ScalableWorkloadId,
        controlled_allocation_hard_ceiling_bytes: u64,
    ) -> Result<Self, TimingError> {
        let candidate_configuration = CandidatePipelineConfiguration::baseline(trusted)?;
        Self::from_trusted_contract_with_configuration(
            trusted,
            compiler_instance_id,
            workload_id,
            controlled_allocation_hard_ceiling_bytes,
            candidate_configuration,
        )
    }

    pub fn from_trusted_contract_with_candidate_and_allocation_ceiling(
        trusted: &TrustedContract,
        compiler_instance_id: String,
        workload_id: ScalableWorkloadId,
        controlled_allocation_hard_ceiling_bytes: u64,
        candidate_configuration: CandidatePipelineConfiguration,
    ) -> Result<Self, TimingError> {
        if workload_id == ScalableWorkloadId::Identity {
            return Err(TimingError::CandidatePipelineRequiresTemplateWorkload);
        }
        Self::from_trusted_contract_with_configuration(
            trusted,
            compiler_instance_id,
            workload_id,
            controlled_allocation_hard_ceiling_bytes,
            candidate_configuration,
        )
    }

    fn from_trusted_contract_with_configuration(
        trusted: &TrustedContract,
        compiler_instance_id: String,
        workload_id: ScalableWorkloadId,
        controlled_allocation_hard_ceiling_bytes: u64,
        candidate_configuration: CandidatePipelineConfiguration,
    ) -> Result<Self, TimingError> {
        if compiler_instance_id.is_empty() {
            return Err(TimingError::EmptyCompilerInstanceId);
        }
        let (inner, plans) = match workload_id {
            ScalableWorkloadId::Identity => (
                ScalableCompilerInner::Identity(Box::new(
                    IdentityCompilerInstance::from_trusted_contract_with_optional_id_and_allocation_ceiling(
                        trusted,
                        Some(compiler_instance_id.clone()),
                        controlled_allocation_hard_ceiling_bytes,
                    )?,
                )),
                ScalableStagePlanFactory::from_trusted_contract_for_workload(trusted, workload_id)?,
            ),
            ScalableWorkloadId::Corridor => {
                let contract = CorridorContract::from_manifest(&trusted.workload_manifest)?;
                let template = contract.load_template(&crate::repository_root())?;
                let plans = ScalableStagePlanFactory::from_trusted_contract_for_template_workload(
                    trusted,
                    workload_id,
                    &template,
                )?;
                (
                    ScalableCompilerInner::Corridor(TemplateTimingCompilerInstance {
                        generator: trusted.generator_contract()?,
                        identity: trusted.identity_contract()?,
                        stage: trusted.stage_contract()?,
                        contract,
                        template,
                        buffers: Some(BoundedTemplateBufferPool::new(
                            ControlledAllocator::new_for_mode(
                                controlled_allocation_hard_ceiling_bytes,
                                TRACK_ALLOCATIONS,
                            ),
                        )),
                        candidate_configuration: candidate_configuration.clone(),
                    }),
                    plans,
                )
            }
            ScalableWorkloadId::JunctionGrid => {
                let contract = JunctionGridContract::from_manifest(&trusted.workload_manifest)?;
                let template = build_junction_grid_template();
                contract.validate_template(&template)?;
                let plans = ScalableStagePlanFactory::from_trusted_contract_for_template_workload(
                    trusted,
                    workload_id,
                    &template,
                )?;
                (
                    ScalableCompilerInner::JunctionGrid(TemplateTimingCompilerInstance {
                        generator: trusted.generator_contract()?,
                        identity: trusted.identity_contract()?,
                        stage: trusted.stage_contract()?,
                        contract,
                        template,
                        buffers: Some(BoundedTemplateBufferPool::new(
                            ControlledAllocator::new_for_mode(
                                controlled_allocation_hard_ceiling_bytes,
                                TRACK_ALLOCATIONS,
                            ),
                        )),
                        candidate_configuration: candidate_configuration.clone(),
                    }),
                    plans,
                )
            }
        };
        Ok(Self {
            compiler_instance_id,
            controlled_allocation_hard_ceiling_bytes,
            limit_manifest: trusted.workload_manifest.clone(),
            plans,
            inner,
            completed_compilations: 0,
        })
    }

    pub fn compiler_instance_id(&self) -> &str {
        &self.compiler_instance_id
    }

    pub fn controlled_allocation_hard_ceiling_bytes(&self) -> u64 {
        self.controlled_allocation_hard_ceiling_bytes
    }

    pub fn run_stable_capacity_sequence(
        &mut self,
        graph_profile: GraphProfileId,
        n: u32,
    ) -> Result<ScalableStableCapacitySequence, TimingError> {
        if self.completed_compilations != 0 {
            return Err(TimingError::CompilerInstanceAlreadyUsed);
        }
        let cold_instance = self.measure(graph_profile, n)?;
        let retained_capacity_bytes = self.retained_capacity_bytes()?;
        for _ in 0..STABLE_CAPACITY_WARMUP_COUNT {
            let warmup_digest = self.run_unmeasured(graph_profile, n)?;
            if warmup_digest != cold_instance.semantic_digest_sha256 {
                return Err(TimingError::StableCapacitySummaryChanged);
            }
            if self.retained_capacity_bytes()? != retained_capacity_bytes {
                return Err(TimingError::StableCapacityChanged);
            }
        }
        let post_warmup_allocation = self.allocation_snapshot()?;
        let mut stable_capacity_reuse = Vec::with_capacity(STABLE_CAPACITY_SAMPLE_COUNT as usize);
        for _ in 0..STABLE_CAPACITY_SAMPLE_COUNT {
            let sample = self.measure(graph_profile, n)?;
            if sample.semantic_digest_sha256 != cold_instance.semantic_digest_sha256 {
                return Err(TimingError::StableCapacitySummaryChanged);
            }
            if self.retained_capacity_bytes()? != retained_capacity_bytes {
                return Err(TimingError::StableCapacityChanged);
            }
            stable_capacity_reuse.push(sample);
        }
        Ok(ScalableStableCapacitySequence {
            cold_instance,
            stable_capacity_reuse,
            retained_capacity_bytes,
            post_warmup_allocation,
        })
    }

    pub fn measure(
        &mut self,
        graph_profile: GraphProfileId,
        n: u32,
    ) -> Result<ScalableTimingSample, TimingError> {
        let prepared = self.prepare(graph_profile, n)?;
        let executed = self.execute_prepared(prepared)?;
        self.finalize_executed(executed)
    }

    pub fn run_unmeasured(
        &mut self,
        graph_profile: GraphProfileId,
        n: u32,
    ) -> Result<String, TimingError> {
        let prepared = self.prepare(graph_profile, n)?;
        let executed = self.execute_prepared_internal(prepared, false)?;
        Ok(self.finalize_executed(executed)?.semantic_digest_sha256)
    }

    pub(crate) fn run_unmeasured_with_selected_limit(
        &mut self,
        graph_profile: GraphProfileId,
        n: u32,
        dimension_id: LimitDimensionId,
        selected_limit_value: u64,
    ) -> Result<String, TimingError> {
        let workload_id = match &self.inner {
            ScalableCompilerInner::Identity(_) => ScalableWorkloadId::Identity,
            ScalableCompilerInner::Corridor(_) => ScalableWorkloadId::Corridor,
            ScalableCompilerInner::JunctionGrid(_) => ScalableWorkloadId::JunctionGrid,
        };
        let plan = self.plans.plan(workload_id, graph_profile, n)?;
        let actual_value =
            crate::limits::exact_plan_value(&self.limit_manifest, dimension_id, &plan)?;
        if actual_value > selected_limit_value {
            return Err(TimingError::SelectedLimitExceeded {
                dimension_id,
                selected_limit_value,
                actual_value,
            });
        }
        self.run_unmeasured(graph_profile, n)
    }

    pub(crate) fn run_failure(
        &mut self,
        graph_profile: GraphProfileId,
        n: u32,
        failure_input: ScalableFailureInput,
    ) -> Result<ScalableFailureReport, TimingError> {
        let prepared = self.prepare(graph_profile, n)?;
        let ScalablePreparedMeasurement {
            workload_id,
            graph_profile,
            n,
            inner,
        } = prepared;
        match (&mut self.inner, inner, failure_input) {
            (
                ScalableCompilerInner::Identity(instance),
                ScalablePreparedMeasurementInner::Identity(plan),
                ScalableFailureInput::SourceByteLimitPlusOne {
                    selected_limit_value,
                },
            ) => {
                instance.buffers.begin_request()?;
                if plan
                    .counts
                    .source_byte_count
                    .checked_sub(selected_limit_value)
                    != Some(1)
                {
                    return Err(TimingError::InvalidFailureInput(
                        "source byte limit plus-one",
                    ));
                }
                Ok(ScalableFailureReport {
                    stable_compiler_error_code: LIMIT_EXCEEDED_ERROR_CODE,
                    diagnostic_count: 1,
                    diagnostics_truncated: false,
                    partial_output_record_count: 0,
                    output_record_count: 0,
                    live_requested_bytes_after_run: 0,
                    diagnostic_digest_sha256: failure_diagnostic_digest(
                        LIMIT_EXCEEDED_ERROR_CODE,
                        LimitDimensionId::SourceByteCount.one_based_code_u8(),
                        selected_limit_value,
                        plan.counts.source_byte_count,
                    ),
                })
            }
            (
                ScalableCompilerInner::Corridor(instance),
                ScalablePreparedMeasurementInner::Corridor(plan),
                failure_input @ (ScalableFailureInput::MissingReferencePerUnit
                | ScalableFailureInput::MissingReferencePerUnitWithMaximumDiagnostics { .. }
                | ScalableFailureInput::DuplicateOwnerPerUnit
                | ScalableFailureInput::DiagnosticCapPlusOne { .. }),
            ) => {
                let buffers = instance
                    .buffers
                    .take()
                    .ok_or(TimingError::MissingTemplateBufferPool)?;
                buffers.begin_request()?;
                let failure_mode = match failure_input {
                    ScalableFailureInput::MissingReferencePerUnit => {
                        BoundedTemplateFailureMode::MissingReferencePerUnit {
                            maximum_diagnostics: u64::from(n),
                        }
                    }
                    ScalableFailureInput::MissingReferencePerUnitWithMaximumDiagnostics {
                        maximum_diagnostics,
                    } => {
                        if maximum_diagnostics >= u64::from(n) {
                            instance.buffers = Some(buffers);
                            return Err(TimingError::InvalidFailureInput(
                                "missing-reference selected diagnostic limit",
                            ));
                        }
                        BoundedTemplateFailureMode::MissingReferencePerUnit {
                            maximum_diagnostics,
                        }
                    }
                    ScalableFailureInput::DuplicateOwnerPerUnit => {
                        BoundedTemplateFailureMode::DuplicateOwnerPerUnit
                    }
                    ScalableFailureInput::DiagnosticCapPlusOne {
                        maximum_diagnostics,
                    } => {
                        if maximum_diagnostics != u64::from(n) {
                            instance.buffers = Some(buffers);
                            return Err(TimingError::InvalidFailureInput(
                                "diagnostic cap plus-one",
                            ));
                        }
                        BoundedTemplateFailureMode::DiagnosticCapPlusOne {
                            maximum_diagnostics,
                        }
                    }
                    ScalableFailureInput::SourceByteLimitPlusOne { .. } => {
                        unreachable!("closed corridor failure input")
                    }
                };
                let (observation, buffers) = match execute_bounded_template_failure_case_with_pool(
                    &instance.generator,
                    &instance.identity,
                    &instance.stage,
                    workload_id,
                    &instance.template,
                    graph_profile,
                    n,
                    &plan,
                    failure_mode,
                    buffers,
                ) {
                    Ok(result) => result,
                    Err(failure) => {
                        let (source, buffers) = failure.into_parts();
                        instance.buffers = Some(buffers.reset_after_unexpected_failure()?);
                        return Err(source.into());
                    }
                };
                instance.buffers = Some(buffers);
                let (stable_compiler_error_code, diagnostics_truncated) = match failure_input {
                    ScalableFailureInput::MissingReferencePerUnit => {
                        (UNKNOWN_REFERENCE_ERROR_CODE, false)
                    }
                    ScalableFailureInput::MissingReferencePerUnitWithMaximumDiagnostics { .. } => {
                        (DIAGNOSTIC_LIMIT_ERROR_CODE, true)
                    }
                    ScalableFailureInput::DuplicateOwnerPerUnit => {
                        (DUPLICATE_OWNER_ERROR_CODE, false)
                    }
                    ScalableFailureInput::DiagnosticCapPlusOne { .. } => {
                        (DIAGNOSTIC_LIMIT_ERROR_CODE, true)
                    }
                    ScalableFailureInput::SourceByteLimitPlusOne { .. } => {
                        unreachable!("closed corridor failure input")
                    }
                };
                if observation.diagnostics_truncated != diagnostics_truncated {
                    return Err(TimingError::InvalidFailureObservation);
                }
                Ok(ScalableFailureReport {
                    stable_compiler_error_code,
                    diagnostic_count: observation.diagnostic_count,
                    diagnostics_truncated,
                    partial_output_record_count: 0,
                    output_record_count: 0,
                    live_requested_bytes_after_run: 0,
                    diagnostic_digest_sha256: observation.diagnostic_digest_sha256,
                })
            }
            _ => Err(TimingError::InvalidFailureInput(
                "workload and failure variant mismatch",
            )),
        }
    }

    pub fn retained_capacity_bytes(&self) -> Result<StageRetainedCapacityBytes, TimingError> {
        match &self.inner {
            ScalableCompilerInner::Identity(instance) => instance.retained_capacity_bytes(),
            ScalableCompilerInner::Corridor(instance) => instance
                .buffers
                .as_ref()
                .ok_or(TimingError::MissingTemplateBufferPool)?
                .retained_capacity_bytes()
                .map_err(Into::into),
            ScalableCompilerInner::JunctionGrid(instance) => instance
                .buffers
                .as_ref()
                .ok_or(TimingError::MissingTemplateBufferPool)?
                .retained_capacity_bytes()
                .map_err(Into::into),
        }
    }

    pub fn allocation_snapshot(&self) -> Result<IdentityAllocationSnapshot, TimingError> {
        match &self.inner {
            ScalableCompilerInner::Identity(instance) => Ok(instance.allocation_snapshot()),
            ScalableCompilerInner::Corridor(instance) => Ok(instance
                .buffers
                .as_ref()
                .ok_or(TimingError::MissingTemplateBufferPool)?
                .allocation_snapshot()),
            ScalableCompilerInner::JunctionGrid(instance) => Ok(instance
                .buffers
                .as_ref()
                .ok_or(TimingError::MissingTemplateBufferPool)?
                .allocation_snapshot()),
        }
    }

    pub fn prepare(
        &self,
        graph_profile: GraphProfileId,
        n: u32,
    ) -> Result<ScalablePreparedMeasurement, TimingError> {
        let (workload_id, inner) = match &self.inner {
            ScalableCompilerInner::Identity(instance) => (
                ScalableWorkloadId::Identity,
                ScalablePreparedMeasurementInner::Identity(Box::new(prepare_identity_stage_case(
                    &instance.identity,
                    &instance.stage,
                    graph_profile,
                    n,
                )?)),
            ),
            ScalableCompilerInner::Corridor(instance) => {
                if n == 0 {
                    return Err(TimingError::ScaleMustBePositive);
                }
                instance.contract.validate_template(&instance.template)?;
                (
                    ScalableWorkloadId::Corridor,
                    ScalablePreparedMeasurementInner::Corridor(Box::new(self.plans.plan(
                        ScalableWorkloadId::Corridor,
                        graph_profile,
                        n,
                    )?)),
                )
            }
            ScalableCompilerInner::JunctionGrid(instance) => {
                if n == 0 {
                    return Err(TimingError::ScaleMustBePositive);
                }
                instance.contract.validate_template(&instance.template)?;
                (
                    ScalableWorkloadId::JunctionGrid,
                    ScalablePreparedMeasurementInner::JunctionGrid(Box::new(self.plans.plan(
                        ScalableWorkloadId::JunctionGrid,
                        graph_profile,
                        n,
                    )?)),
                )
            }
        };
        Ok(ScalablePreparedMeasurement {
            workload_id,
            graph_profile,
            n,
            inner,
        })
    }

    pub fn execute_prepared(
        &mut self,
        prepared: ScalablePreparedMeasurement,
    ) -> Result<ScalableExecutedMeasurement, TimingError> {
        self.execute_prepared_internal(prepared, true)
    }

    fn execute_prepared_internal(
        &mut self,
        prepared: ScalablePreparedMeasurement,
        measure_wall_clock: bool,
    ) -> Result<ScalableExecutedMeasurement, TimingError> {
        let ScalablePreparedMeasurement {
            workload_id,
            graph_profile,
            n,
            inner,
        } = prepared;
        match &self.inner {
            ScalableCompilerInner::Identity(instance) => {
                instance.buffers.begin_request()?;
            }
            ScalableCompilerInner::Corridor(instance) => {
                instance
                    .buffers
                    .as_ref()
                    .ok_or(TimingError::MissingTemplateBufferPool)?
                    .begin_request()?;
            }
            ScalableCompilerInner::JunctionGrid(instance) => {
                instance
                    .buffers
                    .as_ref()
                    .ok_or(TimingError::MissingTemplateBufferPool)?
                    .begin_request()?;
            }
        }
        let started = measure_wall_clock.then(Instant::now);
        let executed_inner = match (&mut self.inner, inner) {
            (
                ScalableCompilerInner::Identity(instance),
                ScalablePreparedMeasurementInner::Identity(plan),
            ) => {
                let materialized = instance.execute_with_failure_recovery(&plan)?;
                black_box(materialized.output_construction.as_slice());
                ScalableExecutedMeasurementInner::Identity { plan, materialized }
            }
            (
                ScalableCompilerInner::Corridor(instance),
                ScalablePreparedMeasurementInner::Corridor(plan),
            ) => {
                let buffers = instance
                    .buffers
                    .take()
                    .ok_or(TimingError::MissingTemplateBufferPool)?;
                let execution = match execute_bounded_template_stage_case_with_pool_and_candidate(
                    &instance.generator,
                    &instance.identity,
                    &instance.stage,
                    ScalableWorkloadId::Corridor,
                    &instance.template,
                    graph_profile,
                    n,
                    &plan,
                    buffers,
                    Some(&instance.candidate_configuration),
                ) {
                    Ok(execution) => execution,
                    Err(failure) => {
                        let (source, buffers) = failure.into_parts();
                        instance.buffers = Some(buffers.reset_after_unexpected_failure()?);
                        return Err(source.into());
                    }
                };
                black_box(execution.output_construction());
                ScalableExecutedMeasurementInner::Corridor(execution)
            }
            (
                ScalableCompilerInner::JunctionGrid(instance),
                ScalablePreparedMeasurementInner::JunctionGrid(plan),
            ) => {
                let buffers = instance
                    .buffers
                    .take()
                    .ok_or(TimingError::MissingTemplateBufferPool)?;
                let execution = match execute_bounded_template_stage_case_with_pool_and_candidate(
                    &instance.generator,
                    &instance.identity,
                    &instance.stage,
                    ScalableWorkloadId::JunctionGrid,
                    &instance.template,
                    graph_profile,
                    n,
                    &plan,
                    buffers,
                    Some(&instance.candidate_configuration),
                ) {
                    Ok(execution) => execution,
                    Err(failure) => {
                        let (source, buffers) = failure.into_parts();
                        instance.buffers = Some(buffers.reset_after_unexpected_failure()?);
                        return Err(source.into());
                    }
                };
                black_box(execution.output_construction());
                ScalableExecutedMeasurementInner::JunctionGrid(execution)
            }
            _ => return Err(TimingError::ScalableMeasurementInstanceMismatch),
        };
        let wall_time_ns = match started {
            Some(started) => u64::try_from(started.elapsed().as_nanos())
                .map_err(|_| TimingError::ClockDurationOverflow)?,
            None => 0,
        };
        let guard_peak_live_requested_bytes = match (&self.inner, &executed_inner) {
            (
                ScalableCompilerInner::Identity(instance),
                ScalableExecutedMeasurementInner::Identity { .. },
            ) => instance.allocation_snapshot().peak_live_requested_bytes,
            (
                ScalableCompilerInner::Corridor(_),
                ScalableExecutedMeasurementInner::Corridor(execution),
            ) => execution.peak_live_requested_bytes(),
            (
                ScalableCompilerInner::JunctionGrid(_),
                ScalableExecutedMeasurementInner::JunctionGrid(execution),
            ) => execution.peak_live_requested_bytes(),
            _ => return Err(TimingError::ScalableMeasurementInstanceMismatch),
        };
        Ok(ScalableExecutedMeasurement {
            workload_id,
            graph_profile,
            n,
            wall_time_ns,
            guard_peak_live_requested_bytes,
            inner: executed_inner,
        })
    }

    pub fn finalize_executed(
        &mut self,
        executed: ScalableExecutedMeasurement,
    ) -> Result<ScalableTimingSample, TimingError> {
        let ScalableExecutedMeasurement {
            workload_id,
            graph_profile,
            n,
            wall_time_ns,
            guard_peak_live_requested_bytes,
            inner,
        } = executed;
        let (semantic_digest_sha256, candidate_pipeline_checksums) = match (&mut self.inner, inner)
        {
            (
                ScalableCompilerInner::Identity(instance),
                ScalableExecutedMeasurementInner::Identity { plan, materialized },
            ) => (
                instance
                    .finalize_and_recycle(&plan, materialized)?
                    .semantic_digest_sha256,
                CandidatePipelineChecksums::default(),
            ),
            (
                ScalableCompilerInner::Corridor(instance),
                ScalableExecutedMeasurementInner::Corridor(execution),
            ) => {
                let checksums = execution.candidate_pipeline_checksums();
                let (digest, buffers) = finalize_bounded_template_stage_case_with_pool(execution)?;
                instance.buffers = Some(buffers);
                (digest, checksums)
            }
            (
                ScalableCompilerInner::JunctionGrid(instance),
                ScalableExecutedMeasurementInner::JunctionGrid(execution),
            ) => {
                let checksums = execution.candidate_pipeline_checksums();
                let (digest, buffers) = finalize_bounded_template_stage_case_with_pool(execution)?;
                instance.buffers = Some(buffers);
                (digest, checksums)
            }
            _ => return Err(TimingError::ScalableMeasurementInstanceMismatch),
        };
        self.completed_compilations = self
            .completed_compilations
            .checked_add(1)
            .ok_or(TimingError::CompilerInstanceCompilationOverflow)?;
        let allocation = self.allocation_snapshot()?;
        Ok(ScalableTimingSample {
            workload_id,
            graph_profile,
            n,
            wall_time_ns,
            semantic_digest_sha256,
            guard_peak_live_requested_bytes,
            allocation,
            candidate_pipeline_checksums,
        })
    }
}

fn failure_diagnostic_digest(
    error_code: &'static str,
    dimension_code_u8: u8,
    selected_limit_value: u64,
    observed_value: u64,
) -> String {
    crate::diagnostic::limit_exceeded_diagnostic_digest(
        error_code,
        dimension_code_u8,
        selected_limit_value,
        observed_value,
    )
}

pub fn observe_clock_quantum_ns() -> Result<u64, TimingError> {
    let mut previous = Instant::now();
    let mut minimum_positive = None::<u64>;
    for _ in 0..CLOCK_QUANTUM_OBSERVATION_COUNT {
        let current = Instant::now();
        let delta = current.duration_since(previous);
        previous = current;
        let nanoseconds =
            u64::try_from(delta.as_nanos()).map_err(|_| TimingError::ClockDurationOverflow)?;
        if nanoseconds > 0 {
            minimum_positive =
                Some(minimum_positive.map_or(nanoseconds, |minimum| minimum.min(nanoseconds)));
        }
    }
    minimum_positive.ok_or(TimingError::NoPositiveClockDelta)
}

pub fn measure_identity_stage_once(
    trusted: &TrustedContract,
    graph_profile: GraphProfileId,
    n: u32,
) -> Result<IdentityTimingSample, TimingError> {
    let mut instance = IdentityCompilerInstance::from_trusted_contract(trusted)?;
    let sample = instance.measure(graph_profile, n)?;
    let oracle = build_identity_stage_oracle(&trusted.workload_manifest, graph_profile, n)?;
    if sample.stage_summary != oracle {
        return Err(TimingError::IndependentOracleSummaryMismatch);
    }
    Ok(sample)
}

#[derive(Debug, thiserror::Error)]
pub enum TimingError {
    #[error(transparent)]
    GeneratorContract(#[from] crate::ManifestContractError),
    #[error(transparent)]
    IdentityContract(#[from] crate::IdentityContractError),
    #[error(transparent)]
    StageContract(#[from] crate::StageContractError),
    #[error(transparent)]
    StageGeneration(#[from] crate::StageGenerationError),
    #[error(transparent)]
    StageOracle(#[from] crate::StageOracleError),
    #[error(transparent)]
    Corridor(#[from] crate::CorridorError),
    #[error(transparent)]
    JunctionGrid(#[from] crate::JunctionGridError),
    #[error(transparent)]
    ScalePlan(#[from] crate::ScalePlanError),
    #[error(transparent)]
    LimitQualification(#[from] crate::LimitQualificationError),
    #[error(transparent)]
    CandidateMatrix(#[from] crate::CandidateMatrixError),
    #[error("单调时钟观测未取得任何正差值")]
    NoPositiveClockDelta,
    #[error("单调时钟时长无法表示为 u64 纳秒")]
    ClockDurationOverflow,
    #[error("稳定容量复用实例在样本间保留了语义值")]
    RetainedSemanticState,
    #[error("稳定容量序列必须从未使用过的新编译器实例开始")]
    CompilerInstanceAlreadyUsed,
    #[error("编译器实例身份不能为空")]
    EmptyCompilerInstanceId,
    #[error("编译器实例的完成编译次数溢出")]
    CompilerInstanceCompilationOverflow,
    #[error("稳定容量复用期间阶段摘要发生变化")]
    StableCapacitySummaryChanged,
    #[error("稳定容量复用期间保留容量发生变化")]
    StableCapacityChanged,
    #[error("计时生产者阶段摘要与独立预言机摘要不一致")]
    IndependentOracleSummaryMismatch,
    #[error("可扩展工作负载规模 N 必须大于零")]
    ScaleMustBePositive,
    #[error("计时区外准备、受测执行与编译器实例的工作负载身份不一致")]
    ScalableMeasurementInstanceMismatch,
    #[error("模板工作负载编译器实例缺少可回收的阶段缓冲池")]
    MissingTemplateBufferPool,
    #[error("完整管线候选只适用于模板型 Corridor/Junction Grid 工作负载")]
    CandidatePipelineRequiresTemplateWorkload,
    #[error("失败输入与真实编译管线不匹配：{0}")]
    InvalidFailureInput(&'static str),
    #[error("真实编译失败观察与请求不一致")]
    InvalidFailureObservation,
    #[error(
        "私有限制 {dimension_id:?} 在进入比例分配前失败：限制 {selected_limit_value}，实际 {actual_value}"
    )]
    SelectedLimitExceeded {
        dimension_id: LimitDimensionId,
        selected_limit_value: u64,
        actual_value: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_repository_contract;

    #[test]
    fn clock_quantum_uses_the_protocol_observation_count_and_is_positive() {
        assert_eq!(CLOCK_QUANTUM_OBSERVATION_COUNT, 100_000);
        assert!(observe_clock_quantum_ns().expect("clock quantum") > 0);
    }

    #[test]
    fn timed_boundary_returns_only_after_out_of_band_exact_verification() {
        let trusted = load_repository_contract().expect("frozen contract");
        for graph_profile in GraphProfileId::ALL {
            let sample =
                measure_identity_stage_once(&trusted, graph_profile, 1).expect("timing sample");
            assert!(sample.wall_time_ns > 0);
            assert_eq!(sample.stage_summary.graph_profile, graph_profile);
            assert_eq!(sample.stage_summary.n, 1);
            assert_eq!(sample.stage_summary.counts.semantic_output_record, 32);
        }
    }

    #[test]
    fn evidence_instance_identity_must_be_explicit_and_nonempty() {
        let trusted = load_repository_contract().expect("frozen contract");
        assert!(matches!(
            IdentityCompilerInstance::from_trusted_contract_with_id(&trusted, String::new()),
            Err(TimingError::EmptyCompilerInstanceId)
        ));

        let instance = IdentityCompilerInstance::from_trusted_contract_with_id(
            &trusted,
            "pilot/compiler-instance-0".to_owned(),
        )
        .expect("identified instance");
        assert_eq!(
            instance.compiler_instance_id(),
            Some("pilot/compiler-instance-0")
        );
    }

    #[test]
    fn controlled_allocation_at_bound_succeeds_and_plus_one_fails_before_reserve() {
        let trusted = load_repository_contract().expect("frozen contract");
        let mut baseline =
            IdentityCompilerInstance::<true>::from_trusted_contract_with_id_and_allocation_ceiling(
                &trusted,
                "allocation-baseline".to_owned(),
                u64::MAX,
            )
            .expect("baseline instance");
        baseline
            .measure(GraphProfileId::WideStar, 1)
            .expect("baseline measurement");
        let exact_peak = baseline.peak_live_requested_bytes();
        assert!(exact_peak > 1);

        let mut at_bound =
            IdentityCompilerInstance::<true>::from_trusted_contract_with_id_and_allocation_ceiling(
                &trusted,
                "allocation-at-bound".to_owned(),
                exact_peak,
            )
            .expect("at-bound instance");
        at_bound
            .measure(GraphProfileId::WideStar, 1)
            .expect("exact peak must remain allowed");
        assert_eq!(at_bound.peak_live_requested_bytes(), exact_peak);

        let mut plus_one =
            IdentityCompilerInstance::<true>::from_trusted_contract_with_id_and_allocation_ceiling(
                &trusted,
                "allocation-plus-one".to_owned(),
                exact_peak - 1,
            )
            .expect("plus-one instance");
        assert!(matches!(
            plus_one.measure(GraphProfileId::WideStar, 1),
            Err(TimingError::StageGeneration(
                crate::StageGenerationError::ControlledAllocationHardCeiling {
                    hard_ceiling_bytes,
                    ..
                }
            )) if hard_ceiling_bytes == exact_peak - 1
        ));
        let first_failure = plus_one.allocation_snapshot();
        assert_eq!(first_failure.live_requested_bytes, 0);
        assert!(first_failure.freed_bytes > 0);

        assert!(matches!(
            plus_one.measure(GraphProfileId::WideStar, 1),
            Err(TimingError::StageGeneration(
                crate::StageGenerationError::ControlledAllocationHardCeiling {
                    hard_ceiling_bytes,
                    ..
                }
            )) if hard_ceiling_bytes == exact_peak - 1
        ));
        let repeated_failure = plus_one.allocation_snapshot();
        assert_eq!(repeated_failure.live_requested_bytes, 0);
        assert!(repeated_failure.freed_bytes > first_failure.freed_bytes);
    }

    #[test]
    fn every_identity_profile_reuses_only_empty_stage_capacities() {
        let trusted = load_repository_contract().expect("frozen contract");
        for graph_profile in GraphProfileId::ALL {
            for n in [1, 2] {
                let mut instance =
                    IdentityCompilerInstance::from_trusted_contract(&trusted).expect("instance");
                assert_eq!(
                    instance.retained_capacity_bytes().expect("empty capacity"),
                    StageRetainedCapacityBytes::default()
                );

                let sequence = instance
                    .run_stable_capacity_sequence(graph_profile, n)
                    .expect("stable-capacity sequence");
                let retained = sequence.retained_capacity_bytes;
                assert!(retained.source_input > 0);
                assert!(retained.typed_ast > 0);
                assert!(retained.hir > 0);
                assert!(retained.mir > 0);
                assert!(retained.canonical_lir > 0);
                assert_eq!(retained.diagnostics, 0);
                assert!(retained.scratch > 0);
                assert!(retained.output_construction > 0);
                assert_eq!(
                    retained.total,
                    retained.source_input
                        + retained.typed_ast
                        + retained.hir
                        + retained.mir
                        + retained.canonical_lir
                        + retained.diagnostics
                        + retained.scratch
                        + retained.output_construction
                );
                assert_eq!(
                    sequence.stable_capacity_reuse.len(),
                    STABLE_CAPACITY_SAMPLE_COUNT as usize
                );
                assert!(
                    sequence
                        .stable_capacity_reuse
                        .iter()
                        .all(|sample| sample.stage_summary == sequence.cold_instance.stage_summary)
                );
                assert!(matches!(
                    instance.run_stable_capacity_sequence(graph_profile, n),
                    Err(TimingError::CompilerInstanceAlreadyUsed)
                ));
            }
        }
    }

    #[test]
    fn template_timing_enforces_actual_at_bound_plus_one_and_recovers() {
        let trusted = load_repository_contract().expect("frozen contract");
        for workload_id in [
            ScalableWorkloadId::Corridor,
            ScalableWorkloadId::JunctionGrid,
        ] {
            let mut probe = ScalableTimingCompilerInstance::from_trusted_contract_with_id(
                &trusted,
                format!("{}/actual-peak-probe", workload_id.as_str()),
                workload_id,
            )
            .expect("probe instance");
            let sample = probe
                .measure(GraphProfileId::WideStar, 1)
                .expect("actual peak probe");
            let exact_peak = sample.guard_peak_live_requested_bytes;
            assert!(exact_peak > 1);

            let mut at_bound =
                ScalableTimingCompilerInstance::from_trusted_contract_with_id_and_allocation_ceiling(
                    &trusted,
                    format!("{}/at-bound", workload_id.as_str()),
                    workload_id,
                    exact_peak,
                )
                .expect("at-bound instance");
            let at_bound_sample = at_bound
                .measure(GraphProfileId::WideStar, 1)
                .expect("exact observed peak must remain allowed");
            assert_eq!(at_bound_sample.guard_peak_live_requested_bytes, exact_peak);

            let mut plus_one =
                ScalableTimingCompilerInstance::from_trusted_contract_with_id_and_allocation_ceiling(
                    &trusted,
                    format!("{}/plus-one", workload_id.as_str()),
                    workload_id,
                    exact_peak - 1,
                )
                .expect("plus-one instance");
            for _ in 0..2 {
                assert!(matches!(
                    plus_one.measure(GraphProfileId::WideStar, 1),
                    Err(TimingError::StageGeneration(
                        crate::StageGenerationError::ControlledAllocationHardCeiling {
                            hard_ceiling_bytes,
                            live_requested_bytes,
                            requested_bytes,
                            ..
                        }
                    )) if hard_ceiling_bytes == exact_peak - 1
                        && live_requested_bytes
                            .checked_add(requested_bytes)
                            .is_none_or(|total| total > hard_ceiling_bytes)
                ));
            }
        }
    }

    #[test]
    fn every_scalable_workload_recovers_after_a_larger_scale_hits_the_allocation_ceiling() {
        let trusted = load_repository_contract().expect("frozen contract");
        for workload_id in ScalableWorkloadId::ALL {
            let mut probe = ScalableAttributionCompilerInstance::from_trusted_contract_with_id(
                &trusted,
                format!("{}/recovery-peak-probe", workload_id.as_str()),
                workload_id,
            )
            .expect("probe instance");
            let exact_n1_peak = probe
                .measure(GraphProfileId::WideStar, 1)
                .expect("N=1 peak probe")
                .guard_peak_live_requested_bytes;

            let mut instance =
                ScalableAttributionCompilerInstance::from_trusted_contract_with_id_and_allocation_ceiling(
                    &trusted,
                    format!("{}/recovery-instance", workload_id.as_str()),
                    workload_id,
                    exact_n1_peak,
                )
                .expect("bounded recovery instance");
            let before_failure = instance
                .measure(GraphProfileId::WideStar, 1)
                .expect("legal baseline measurement");
            assert!(matches!(
                instance.measure(GraphProfileId::WideStar, 2),
                Err(TimingError::StageGeneration(
                    crate::StageGenerationError::ControlledAllocationHardCeiling { .. }
                ))
            ));
            let after_failure = instance
                .measure(GraphProfileId::WideStar, 1)
                .expect("legal measurement after rejected larger scale");
            assert_eq!(
                after_failure.semantic_digest_sha256,
                before_failure.semantic_digest_sha256
            );
        }
    }
}
