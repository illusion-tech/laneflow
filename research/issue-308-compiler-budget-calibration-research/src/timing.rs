//! 正式外层计时区的最小测量原语。
//!
//! 本模块只建立 #308 协议要求的 prepare -> timed execute -> finalize/verify 边界；
//! 新进程编排、稳定容量复用、停止护栏和证据写出仍由后续 runner 切片负责。

use crate::pipeline::{
    execute_identity_stage_case, finalize_identity_stage_case, prepare_identity_stage_case,
};
use crate::stage::IdentityStageSummary;
use crate::stage_oracle::verify_identity_stage_exact;
use crate::{GraphProfileId, TrustedContract};
use serde::Serialize;
use std::hint::black_box;
use std::time::Instant;

pub const CLOCK_QUANTUM_OBSERVATION_COUNT: u32 = 100_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityTimingSample {
    pub wall_time_ns: u64,
    pub stage_summary: IdentityStageSummary,
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
    // 受信任契约解析与固定大小规模计划必须位于计时区外。
    let generator = trusted.generator_contract()?;
    let identity = trusted.identity_contract()?;
    let stage = trusted.stage_contract()?;
    let plan = prepare_identity_stage_case(&identity, &stage, graph_profile, n)?;

    // 正式墙钟样本只允许这一对外层时钟读取。
    let started = Instant::now();
    let materialized = execute_identity_stage_case(&generator, &identity, &stage, &plan)?;
    black_box(materialized.output_construction.as_slice());
    let elapsed = started.elapsed();

    // 摘要、完整形状检查和独立精确预言机均在停表后执行。
    let produced = finalize_identity_stage_case(&plan, materialized)?;
    verify_identity_stage_exact(&trusted.workload_manifest, graph_profile, n, &produced)?;

    Ok(IdentityTimingSample {
        wall_time_ns: u64::try_from(elapsed.as_nanos())
            .map_err(|_| TimingError::ClockDurationOverflow)?,
        stage_summary: produced.summary,
    })
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
}
