//! 正式外层计时区的最小测量原语。
//!
//! 本模块建立 #308 协议要求的 prepare -> timed execute -> finalize 边界，并由
//! 同一个编译器实例只保留已清空的阶段容器容量。计时与归因实例通过编译期常量分别
//! 单态化：正式计时路径只执行停止护栏所必需的受控分配预占，不累计分配归因指标；
//! 归因路径额外记录分配、释放与存续峰值。独立正确性验证不进入这两个实例；正式轮次
//! 和证据写出仍由后续 runner 切片负责。

use crate::bounded_template::{
    BoundedTemplateExecution, allocation_observation, execute_bounded_template_stage_case,
    finalize_bounded_template_stage_case,
};
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
    GeneratorContract, GraphProfileId, IdentityContract, ScalableWorkloadId, StageContract,
    TrustedContract,
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
}

#[derive(Debug)]
pub struct ScalableTimingCompilerInstance {
    compiler_instance_id: String,
    controlled_allocation_hard_ceiling_bytes: u64,
    plans: ScalableStagePlanFactory,
    inner: ScalableTimingCompilerInner,
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
enum ScalableTimingCompilerInner {
    Identity(Box<IdentityTimingCompilerInstance>),
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
    allocator: ControlledAllocator,
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
        self.buffers.guard_peak_live_requested_bytes()
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

    pub fn allocation_snapshot(&self) -> IdentityAllocationSnapshot {
        self.buffers.allocation_snapshot()
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

impl ScalableTimingCompilerInstance {
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
        if compiler_instance_id.is_empty() {
            return Err(TimingError::EmptyCompilerInstanceId);
        }
        let (inner, plans) = match workload_id {
            ScalableWorkloadId::Identity => (
                ScalableTimingCompilerInner::Identity(Box::new(
                    IdentityTimingCompilerInstance::from_trusted_contract_with_id_and_allocation_ceiling(
                        trusted,
                        compiler_instance_id.clone(),
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
                    ScalableTimingCompilerInner::Corridor(TemplateTimingCompilerInstance {
                        generator: trusted.generator_contract()?,
                        identity: trusted.identity_contract()?,
                        stage: trusted.stage_contract()?,
                        contract,
                        template,
                        allocator: ControlledAllocator::new(
                            controlled_allocation_hard_ceiling_bytes,
                        ),
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
                    ScalableTimingCompilerInner::JunctionGrid(TemplateTimingCompilerInstance {
                        generator: trusted.generator_contract()?,
                        identity: trusted.identity_contract()?,
                        stage: trusted.stage_contract()?,
                        contract,
                        template,
                        allocator: ControlledAllocator::new(
                            controlled_allocation_hard_ceiling_bytes,
                        ),
                    }),
                    plans,
                )
            }
        };
        Ok(Self {
            compiler_instance_id,
            controlled_allocation_hard_ceiling_bytes,
            plans,
            inner,
        })
    }

    pub fn compiler_instance_id(&self) -> &str {
        &self.compiler_instance_id
    }

    pub fn controlled_allocation_hard_ceiling_bytes(&self) -> u64 {
        self.controlled_allocation_hard_ceiling_bytes
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

    pub fn prepare(
        &self,
        graph_profile: GraphProfileId,
        n: u32,
    ) -> Result<ScalablePreparedMeasurement, TimingError> {
        let (workload_id, inner) = match &self.inner {
            ScalableTimingCompilerInner::Identity(instance) => (
                ScalableWorkloadId::Identity,
                ScalablePreparedMeasurementInner::Identity(Box::new(prepare_identity_stage_case(
                    &instance.identity,
                    &instance.stage,
                    graph_profile,
                    n,
                )?)),
            ),
            ScalableTimingCompilerInner::Corridor(instance) => {
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
            ScalableTimingCompilerInner::JunctionGrid(instance) => {
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
        let ScalablePreparedMeasurement {
            workload_id,
            graph_profile,
            n,
            inner,
        } = prepared;
        match &self.inner {
            ScalableTimingCompilerInner::Identity(_) => {}
            ScalableTimingCompilerInner::Corridor(instance) => {
                instance.allocator.begin_request()?;
            }
            ScalableTimingCompilerInner::JunctionGrid(instance) => {
                instance.allocator.begin_request()?;
            }
        }
        let started = Instant::now();
        let executed_inner = match (&mut self.inner, inner) {
            (
                ScalableTimingCompilerInner::Identity(instance),
                ScalablePreparedMeasurementInner::Identity(plan),
            ) => {
                let materialized = instance.execute_with_failure_recovery(&plan)?;
                black_box(materialized.output_construction.as_slice());
                ScalableExecutedMeasurementInner::Identity { plan, materialized }
            }
            (
                ScalableTimingCompilerInner::Corridor(instance),
                ScalablePreparedMeasurementInner::Corridor(plan),
            ) => {
                let execution = execute_bounded_template_stage_case(
                    &instance.generator,
                    &instance.identity,
                    &instance.stage,
                    ScalableWorkloadId::Corridor,
                    &instance.template,
                    graph_profile,
                    n,
                    &plan,
                    instance.allocator.clone(),
                )?;
                black_box(execution.output_construction());
                ScalableExecutedMeasurementInner::Corridor(execution)
            }
            (
                ScalableTimingCompilerInner::JunctionGrid(instance),
                ScalablePreparedMeasurementInner::JunctionGrid(plan),
            ) => {
                let execution = execute_bounded_template_stage_case(
                    &instance.generator,
                    &instance.identity,
                    &instance.stage,
                    ScalableWorkloadId::JunctionGrid,
                    &instance.template,
                    graph_profile,
                    n,
                    &plan,
                    instance.allocator.clone(),
                )?;
                black_box(execution.output_construction());
                ScalableExecutedMeasurementInner::JunctionGrid(execution)
            }
            _ => return Err(TimingError::ScalableMeasurementInstanceMismatch),
        };
        let wall_time_ns = u64::try_from(started.elapsed().as_nanos())
            .map_err(|_| TimingError::ClockDurationOverflow)?;
        let guard_peak_live_requested_bytes = match &self.inner {
            ScalableTimingCompilerInner::Identity(instance) => {
                instance.guard_peak_live_requested_bytes()
            }
            ScalableTimingCompilerInner::Corridor(instance) => {
                allocation_observation(&instance.allocator).peak_live_requested_bytes
            }
            ScalableTimingCompilerInner::JunctionGrid(instance) => {
                allocation_observation(&instance.allocator).peak_live_requested_bytes
            }
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
        let semantic_digest_sha256 = match (&mut self.inner, inner) {
            (
                ScalableTimingCompilerInner::Identity(instance),
                ScalableExecutedMeasurementInner::Identity { plan, materialized },
            ) => {
                instance
                    .finalize_and_recycle(&plan, materialized)?
                    .semantic_digest_sha256
            }
            (
                ScalableTimingCompilerInner::Corridor(_),
                ScalableExecutedMeasurementInner::Corridor(execution),
            ) => finalize_bounded_template_stage_case(execution)?,
            (
                ScalableTimingCompilerInner::JunctionGrid(_),
                ScalableExecutedMeasurementInner::JunctionGrid(execution),
            ) => finalize_bounded_template_stage_case(execution)?,
            _ => return Err(TimingError::ScalableMeasurementInstanceMismatch),
        };
        Ok(ScalableTimingSample {
            workload_id,
            graph_profile,
            n,
            wall_time_ns,
            semantic_digest_sha256,
            guard_peak_live_requested_bytes,
        })
    }
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
}
