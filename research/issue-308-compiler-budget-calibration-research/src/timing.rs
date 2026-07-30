//! 正式外层计时区的最小测量原语。
//!
//! 本模块建立 #308 协议要求的 prepare -> timed execute -> finalize/verify 边界，并由
//! 同一个编译器实例只保留已清空的阶段容器容量；计时区内的具名阶段容量请求同时受
//! 实例硬上限约束。正式轮次和证据写出仍由后续 runner 切片负责。

use crate::pipeline::{
    IdentityStageBufferPool, execute_identity_stage_case_with_buffers,
    finalize_identity_stage_case, prepare_identity_stage_case, recycle_identity_stage_case,
};
use crate::stage::{IdentityStageSummary, StageRetainedCapacityBytes};
use crate::stage_oracle::verify_identity_stage_exact;
use crate::{GeneratorContract, GraphProfileId, IdentityContract, StageContract, TrustedContract};
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

#[derive(Debug)]
pub struct IdentityCompilerInstance {
    compiler_instance_id: Option<String>,
    workload_manifest: serde_json::Value,
    generator: GeneratorContract,
    identity: IdentityContract,
    stage: StageContract,
    buffers: IdentityStageBufferPool,
    completed_compilations: u32,
}

impl IdentityCompilerInstance {
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

    fn from_trusted_contract_with_optional_id_and_allocation_ceiling(
        trusted: &TrustedContract,
        compiler_instance_id: Option<String>,
        controlled_allocation_hard_ceiling_bytes: u64,
    ) -> Result<Self, TimingError> {
        Ok(Self {
            compiler_instance_id,
            workload_manifest: trusted.workload_manifest.clone(),
            generator: trusted.generator_contract()?,
            identity: trusted.identity_contract()?,
            stage: trusted.stage_contract()?,
            buffers: IdentityStageBufferPool::with_controlled_allocation_hard_ceiling(
                controlled_allocation_hard_ceiling_bytes,
            ),
            completed_compilations: 0,
        })
    }

    pub fn compiler_instance_id(&self) -> Option<&str> {
        self.compiler_instance_id.as_deref()
    }

    pub fn controlled_allocation_hard_ceiling_bytes(&self) -> u64 {
        self.buffers.controlled_allocation_hard_ceiling_bytes()
    }

    pub fn peak_live_requested_bytes(&self) -> u64 {
        self.buffers.peak_live_requested_bytes()
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
        let materialized = execute_identity_stage_case_with_buffers(
            &self.generator,
            &self.identity,
            &self.stage,
            &plan,
            &mut self.buffers,
        )?;
        black_box(materialized.output_construction.as_slice());
        let elapsed = started.elapsed();

        let stage_summary =
            self.finalize_verify_and_recycle(graph_profile, n, &plan, materialized)?;
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
        let materialized = execute_identity_stage_case_with_buffers(
            &self.generator,
            &self.identity,
            &self.stage,
            &plan,
            &mut self.buffers,
        )?;
        self.finalize_verify_and_recycle(graph_profile, n, &plan, materialized)
    }

    pub fn retained_capacity_bytes(&self) -> Result<StageRetainedCapacityBytes, TimingError> {
        Ok(self.buffers.retained_capacity_bytes()?)
    }

    fn finalize_verify_and_recycle(
        &mut self,
        graph_profile: GraphProfileId,
        n: u32,
        plan: &crate::stage::IdentityStagePlan,
        materialized: crate::pipeline::IdentityStageMaterialization,
    ) -> Result<IdentityStageSummary, TimingError> {
        // 摘要、完整形状检查、独立精确预言机和容量回收均在停表后执行。
        let produced = finalize_identity_stage_case(plan, materialized)?;
        verify_identity_stage_exact(&self.workload_manifest, graph_profile, n, &produced)?;
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
    instance.measure(graph_profile, n)
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
        let mut baseline = IdentityCompilerInstance::from_trusted_contract_with_id(
            &trusted,
            "allocation-baseline".to_owned(),
        )
        .expect("baseline instance");
        baseline
            .measure(GraphProfileId::WideStar, 1)
            .expect("baseline measurement");
        let exact_peak = baseline.peak_live_requested_bytes();
        assert!(exact_peak > 1);

        let mut at_bound =
            IdentityCompilerInstance::from_trusted_contract_with_id_and_allocation_ceiling(
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
            IdentityCompilerInstance::from_trusted_contract_with_id_and_allocation_ceiling(
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
}
