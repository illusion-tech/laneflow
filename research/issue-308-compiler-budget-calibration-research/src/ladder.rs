//! 正式规模阶梯的精确汇总、拐点判定与规模选择。
//!
//! 本模块只处理已经由 runner 验证为有效的正式子进程报告。所有中位数、MAD 和相邻
//! 级别比值都使用整数或约分后的正整数比值；不使用浮点数，也不负责 Evidence v1
//! 封套或制品认证。

use crate::{
    ScalableLadderBinaryMode, ScalableLadderChildReport, ScalableLadderOutcome, ScalableWorkloadId,
};
use serde::Serialize;
use std::cmp::Ordering;

pub const FORMAL_LADDER_BATCH_COUNT: u32 = 2;
pub const FORMAL_LADDER_ROUND_COUNT: u32 = 5;
pub const FORMAL_LADDER_MINIMUM_LEVEL_COUNT: usize = 5;
pub const FORMAL_LADDER_AGGREGATION_METHOD: &str = "median-and-mad-of-exact-integers-v1";
pub const FORMAL_LADDER_SCALE_SELECTION_RULE: &str = "first-confirmed-knee-or-max-complete-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FormalLadderMetric {
    WallTimeNs,
    PeakLiveRequestedBytes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FormalLadderSampleKind {
    ColdInstance,
    StableCapacityReuse,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormalLadderRoundRun {
    pub batch: u32,
    pub round: u32,
    pub binary_mode: ScalableLadderBinaryMode,
    pub report: ScalableLadderChildReport,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormalLadderCompletedLevel {
    pub workload_id: ScalableWorkloadId,
    pub graph_profile: String,
    pub n: u32,
    pub primary_record_count: u64,
    pub canonical_lir_record_count: u64,
    pub runs: Vec<FormalLadderRoundRun>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormalLadderRoundMetricSummary {
    pub summary_id: String,
    pub workload_id: ScalableWorkloadId,
    pub graph_profile: String,
    pub n: u32,
    pub batch: u32,
    pub round: u32,
    pub metric: FormalLadderMetric,
    pub sample_kind: FormalLadderSampleKind,
    pub binary_mode: ScalableLadderBinaryMode,
    pub values: Vec<u64>,
    pub median: u64,
    pub median_absolute_deviation: u64,
    pub normalizer: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormalLadderBatchSummary {
    pub summary_id: String,
    pub workload_id: ScalableWorkloadId,
    pub graph_profile: String,
    pub n: u32,
    pub batch: u32,
    pub metric: FormalLadderMetric,
    pub sample_kind: FormalLadderSampleKind,
    pub binary_mode: ScalableLadderBinaryMode,
    pub contributing_round_summary_ids: Vec<String>,
    pub round_medians: Vec<u64>,
    pub median: u64,
    pub median_absolute_deviation: u64,
    pub normalizer: u64,
    pub aggregation_method: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactPositiveRatio {
    pub numerator: String,
    pub denominator: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormalAdjacentLevelRatio {
    pub workload_id: ScalableWorkloadId,
    pub graph_profile: String,
    pub lower_n: u32,
    pub upper_n: u32,
    pub batch: u32,
    pub metric: FormalLadderMetric,
    pub sample_kind: FormalLadderSampleKind,
    pub lower_batch_summary_id: String,
    pub upper_batch_summary_id: String,
    pub round_ratios: Vec<ExactPositiveRatio>,
    pub median_ratio: ExactPositiveRatio,
    pub candidate_knee: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormalKneeAssessment {
    pub workload_id: ScalableWorkloadId,
    pub graph_profile: String,
    pub lower_n: u32,
    pub upper_n: u32,
    pub metric: FormalLadderMetric,
    pub sample_kind: FormalLadderSampleKind,
    pub batch_zero_candidate: bool,
    pub batch_one_confirmation: bool,
    pub confirmed_knee: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FormalScaleSelectionDisposition {
    ConfirmedKnee,
    NoObservedKnee,
    InsufficientCompleteLevels,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormalScaleSelection {
    pub selection_rule: String,
    pub disposition: FormalScaleSelectionDisposition,
    pub calibration_n: Option<u32>,
    pub stress_n: Option<u32>,
    pub first_confirmed_knee_n: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormalLadderAnalysis {
    pub round_summaries: Vec<FormalLadderRoundMetricSummary>,
    pub batch_summaries: Vec<FormalLadderBatchSummary>,
    pub adjacent_level_ratios: Vec<FormalAdjacentLevelRatio>,
    pub knees: Vec<FormalKneeAssessment>,
    pub scale_selection: FormalScaleSelection,
}

pub fn analyze_formal_ladder(
    levels: &[FormalLadderCompletedLevel],
) -> Result<FormalLadderAnalysis, FormalLadderError> {
    validate_level_sequence(levels)?;
    let mut round_summaries = Vec::new();
    for level in levels {
        round_summaries.extend(summarize_level_rounds(level)?);
    }
    let batch_summaries = summarize_batches(&round_summaries)?;
    let adjacent_level_ratios = build_adjacent_ratios(levels, &round_summaries, &batch_summaries)?;
    let knees = build_knee_assessments(&adjacent_level_ratios)?;
    let scale_selection = select_scales(levels, &knees);
    Ok(FormalLadderAnalysis {
        round_summaries,
        batch_summaries,
        adjacent_level_ratios,
        knees,
        scale_selection,
    })
}

fn validate_level_sequence(levels: &[FormalLadderCompletedLevel]) -> Result<(), FormalLadderError> {
    for (index, level) in levels.iter().enumerate() {
        if level.n == 0 || level.primary_record_count == 0 || level.canonical_lir_record_count == 0
        {
            return Err(FormalLadderError::InvalidLevel { n: level.n });
        }
        if let Some(previous) = index.checked_sub(1).and_then(|i| levels.get(i))
            && (previous.workload_id != level.workload_id
                || previous.graph_profile != level.graph_profile
                || previous.n.checked_mul(2) != Some(level.n))
        {
            return Err(FormalLadderError::InvalidLevelSequence);
        }
    }
    Ok(())
}

fn summarize_level_rounds(
    level: &FormalLadderCompletedLevel,
) -> Result<Vec<FormalLadderRoundMetricSummary>, FormalLadderError> {
    let mut summaries =
        Vec::with_capacity((FORMAL_LADDER_BATCH_COUNT * FORMAL_LADDER_ROUND_COUNT * 4) as usize);
    for batch in 0..FORMAL_LADDER_BATCH_COUNT {
        for round in 0..FORMAL_LADDER_ROUND_COUNT {
            let timing = unique_run(level, batch, round, ScalableLadderBinaryMode::Timing)?;
            let attribution =
                unique_run(level, batch, round, ScalableLadderBinaryMode::Attribution)?;
            validate_report(level, timing)?;
            validate_report(level, attribution)?;
            let timing_cold = timing
                .report
                .cold_instance
                .as_ref()
                .ok_or(FormalLadderError::IncompleteChildReport)?;
            let attribution_cold = attribution
                .report
                .cold_instance
                .as_ref()
                .ok_or(FormalLadderError::IncompleteChildReport)?;
            summaries.push(round_summary(
                level,
                batch,
                round,
                FormalLadderMetric::WallTimeNs,
                FormalLadderSampleKind::ColdInstance,
                ScalableLadderBinaryMode::Timing,
                vec![
                    timing_cold
                        .wall_time_ns
                        .ok_or(FormalLadderError::MetricModeMismatch)?,
                ],
            )?);
            summaries.push(round_summary(
                level,
                batch,
                round,
                FormalLadderMetric::WallTimeNs,
                FormalLadderSampleKind::StableCapacityReuse,
                ScalableLadderBinaryMode::Timing,
                timing
                    .report
                    .stable_capacity_reuse
                    .iter()
                    .map(|sample| {
                        sample
                            .wall_time_ns
                            .ok_or(FormalLadderError::MetricModeMismatch)
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            )?);
            summaries.push(round_summary(
                level,
                batch,
                round,
                FormalLadderMetric::PeakLiveRequestedBytes,
                FormalLadderSampleKind::ColdInstance,
                ScalableLadderBinaryMode::Attribution,
                vec![attribution_cold.guard_peak_live_requested_bytes],
            )?);
            summaries.push(round_summary(
                level,
                batch,
                round,
                FormalLadderMetric::PeakLiveRequestedBytes,
                FormalLadderSampleKind::StableCapacityReuse,
                ScalableLadderBinaryMode::Attribution,
                attribution
                    .report
                    .stable_capacity_reuse
                    .iter()
                    .map(|sample| sample.guard_peak_live_requested_bytes)
                    .collect(),
            )?);
        }
    }
    Ok(summaries)
}

fn unique_run(
    level: &FormalLadderCompletedLevel,
    batch: u32,
    round: u32,
    mode: ScalableLadderBinaryMode,
) -> Result<&FormalLadderRoundRun, FormalLadderError> {
    let mut matching = level
        .runs
        .iter()
        .filter(|run| run.batch == batch && run.round == round && run.binary_mode == mode);
    let run = matching.next().ok_or(FormalLadderError::MissingFormalRun {
        n: level.n,
        batch,
        round,
        mode,
    })?;
    if matching.next().is_some() {
        return Err(FormalLadderError::DuplicateFormalRun {
            n: level.n,
            batch,
            round,
            mode,
        });
    }
    Ok(run)
}

fn validate_report(
    level: &FormalLadderCompletedLevel,
    run: &FormalLadderRoundRun,
) -> Result<(), FormalLadderError> {
    let report = &run.report;
    if report.outcome != ScalableLadderOutcome::Success
        || report.binary_mode != run.binary_mode
        || report.workload_id != level.workload_id
        || report.graph_profile != level.graph_profile
        || report.n != level.n
        || report.cold_instance.is_none()
        || report.stable_capacity_reuse.len() != 7
    {
        return Err(FormalLadderError::IncompleteChildReport);
    }
    let expected_digest = &report
        .cold_instance
        .as_ref()
        .ok_or(FormalLadderError::IncompleteChildReport)?
        .semantic_digest_sha256;
    if report
        .stable_capacity_reuse
        .iter()
        .any(|sample| sample.semantic_digest_sha256 != *expected_digest)
    {
        return Err(FormalLadderError::SemanticDigestMismatch);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn round_summary(
    level: &FormalLadderCompletedLevel,
    batch: u32,
    round: u32,
    metric: FormalLadderMetric,
    sample_kind: FormalLadderSampleKind,
    binary_mode: ScalableLadderBinaryMode,
    values: Vec<u64>,
) -> Result<FormalLadderRoundMetricSummary, FormalLadderError> {
    let expected_count = match sample_kind {
        FormalLadderSampleKind::ColdInstance => 1,
        FormalLadderSampleKind::StableCapacityReuse => 7,
    };
    if values.len() != expected_count || values.contains(&0) {
        return Err(FormalLadderError::InvalidMetricValues);
    }
    let (median, median_absolute_deviation) = median_and_mad(&values)?;
    let normalizer = match metric {
        FormalLadderMetric::WallTimeNs => level.primary_record_count,
        FormalLadderMetric::PeakLiveRequestedBytes => level.canonical_lir_record_count,
    };
    Ok(FormalLadderRoundMetricSummary {
        summary_id: format!(
            "round/{}/{}/n-{}/batch-{batch}/round-{round}/{}/{}/{}",
            level.workload_id.as_str(),
            level.graph_profile,
            level.n,
            metric_token(metric),
            sample_kind_token(sample_kind),
            mode_token(binary_mode)
        ),
        workload_id: level.workload_id,
        graph_profile: level.graph_profile.clone(),
        n: level.n,
        batch,
        round,
        metric,
        sample_kind,
        binary_mode,
        values,
        median,
        median_absolute_deviation,
        normalizer,
    })
}

fn summarize_batches(
    round_summaries: &[FormalLadderRoundMetricSummary],
) -> Result<Vec<FormalLadderBatchSummary>, FormalLadderError> {
    let mut output = Vec::new();
    if round_summaries.is_empty() {
        return Ok(output);
    }
    let level_keys = round_summaries
        .iter()
        .map(|summary| {
            (
                summary.workload_id,
                summary.graph_profile.clone(),
                summary.n,
            )
        })
        .collect::<std::collections::BTreeSet<_>>();
    for (workload_id, graph_profile, n) in level_keys {
        for batch in 0..FORMAL_LADDER_BATCH_COUNT {
            for (metric, sample_kind, mode) in required_strata() {
                let mut matching = round_summaries
                    .iter()
                    .filter(|summary| {
                        summary.workload_id == workload_id
                            && summary.graph_profile == graph_profile
                            && summary.n == n
                            && summary.batch == batch
                            && summary.metric == metric
                            && summary.sample_kind == sample_kind
                            && summary.binary_mode == mode
                    })
                    .collect::<Vec<_>>();
                matching.sort_by_key(|summary| summary.round);
                if matching.len() != FORMAL_LADDER_ROUND_COUNT as usize
                    || matching
                        .iter()
                        .enumerate()
                        .any(|(round, summary)| summary.round != round as u32)
                {
                    return Err(FormalLadderError::IncompleteBatch { n, batch });
                }
                let round_medians = matching
                    .iter()
                    .map(|summary| summary.median)
                    .collect::<Vec<_>>();
                let (median, median_absolute_deviation) = median_and_mad(&round_medians)?;
                output.push(FormalLadderBatchSummary {
                    summary_id: format!(
                        "batch/{}/{}/n-{n}/batch-{batch}/{}/{}/{}",
                        workload_id.as_str(),
                        graph_profile,
                        metric_token(metric),
                        sample_kind_token(sample_kind),
                        mode_token(mode)
                    ),
                    workload_id,
                    graph_profile: graph_profile.clone(),
                    n,
                    batch,
                    metric,
                    sample_kind,
                    binary_mode: mode,
                    contributing_round_summary_ids: matching
                        .iter()
                        .map(|summary| summary.summary_id.clone())
                        .collect(),
                    round_medians,
                    median,
                    median_absolute_deviation,
                    normalizer: matching[0].normalizer,
                    aggregation_method: FORMAL_LADDER_AGGREGATION_METHOD.to_owned(),
                });
            }
        }
    }
    Ok(output)
}

fn build_adjacent_ratios(
    levels: &[FormalLadderCompletedLevel],
    round_summaries: &[FormalLadderRoundMetricSummary],
    batch_summaries: &[FormalLadderBatchSummary],
) -> Result<Vec<FormalAdjacentLevelRatio>, FormalLadderError> {
    let mut output = Vec::new();
    for pair in levels.windows(2) {
        let lower = &pair[0];
        let upper = &pair[1];
        for batch in 0..FORMAL_LADDER_BATCH_COUNT {
            for (metric, sample_kind, mode) in required_strata() {
                let lower_batch =
                    unique_batch_summary(batch_summaries, lower.n, batch, metric, sample_kind)?;
                let upper_batch =
                    unique_batch_summary(batch_summaries, upper.n, batch, metric, sample_kind)?;
                let mut round_ratios = Vec::with_capacity(FORMAL_LADDER_ROUND_COUNT as usize);
                for round in 0..FORMAL_LADDER_ROUND_COUNT {
                    let lower_round = unique_round_summary(
                        round_summaries,
                        lower.n,
                        batch,
                        round,
                        metric,
                        sample_kind,
                    )?;
                    let upper_round = unique_round_summary(
                        round_summaries,
                        upper.n,
                        batch,
                        round,
                        metric,
                        sample_kind,
                    )?;
                    round_ratios.push(normalized_ratio(upper_round, lower_round)?);
                }
                let median_ratio = median_ratio(&round_ratios)?;
                let candidate_knee = candidate_knee(metric, &round_ratios, &median_ratio)?;
                output.push(FormalAdjacentLevelRatio {
                    workload_id: lower.workload_id,
                    graph_profile: lower.graph_profile.clone(),
                    lower_n: lower.n,
                    upper_n: upper.n,
                    batch,
                    metric,
                    sample_kind,
                    lower_batch_summary_id: lower_batch.summary_id.clone(),
                    upper_batch_summary_id: upper_batch.summary_id.clone(),
                    round_ratios,
                    median_ratio,
                    candidate_knee,
                });
                debug_assert_eq!(mode, lower_batch.binary_mode);
            }
        }
    }
    Ok(output)
}

fn build_knee_assessments(
    ratios: &[FormalAdjacentLevelRatio],
) -> Result<Vec<FormalKneeAssessment>, FormalLadderError> {
    let mut output = Vec::new();
    for candidate in ratios.iter().filter(|ratio| ratio.batch == 0) {
        let confirmation = ratios
            .iter()
            .find(|ratio| {
                ratio.batch == 1
                    && ratio.workload_id == candidate.workload_id
                    && ratio.graph_profile == candidate.graph_profile
                    && ratio.lower_n == candidate.lower_n
                    && ratio.upper_n == candidate.upper_n
                    && ratio.metric == candidate.metric
                    && ratio.sample_kind == candidate.sample_kind
            })
            .ok_or(FormalLadderError::MissingConfirmationRatio)?;
        output.push(FormalKneeAssessment {
            workload_id: candidate.workload_id,
            graph_profile: candidate.graph_profile.clone(),
            lower_n: candidate.lower_n,
            upper_n: candidate.upper_n,
            metric: candidate.metric,
            sample_kind: candidate.sample_kind,
            batch_zero_candidate: candidate.candidate_knee,
            batch_one_confirmation: confirmation.candidate_knee,
            confirmed_knee: candidate.candidate_knee && confirmation.candidate_knee,
        });
    }
    Ok(output)
}

fn select_scales(
    levels: &[FormalLadderCompletedLevel],
    knees: &[FormalKneeAssessment],
) -> FormalScaleSelection {
    if levels.len() < FORMAL_LADDER_MINIMUM_LEVEL_COUNT {
        return FormalScaleSelection {
            selection_rule: FORMAL_LADDER_SCALE_SELECTION_RULE.to_owned(),
            disposition: FormalScaleSelectionDisposition::InsufficientCompleteLevels,
            calibration_n: None,
            stress_n: None,
            first_confirmed_knee_n: None,
        };
    }
    let first_confirmed_knee_n = knees
        .iter()
        .filter(|knee| knee.confirmed_knee)
        .map(|knee| knee.upper_n)
        .min();
    if let Some(stress_n) = first_confirmed_knee_n {
        let calibration_n = levels
            .iter()
            .rev()
            .find(|level| level.n < stress_n)
            .map(|level| level.n);
        FormalScaleSelection {
            selection_rule: FORMAL_LADDER_SCALE_SELECTION_RULE.to_owned(),
            disposition: FormalScaleSelectionDisposition::ConfirmedKnee,
            calibration_n,
            stress_n: Some(stress_n),
            first_confirmed_knee_n: Some(stress_n),
        }
    } else {
        let stress_n = levels.last().map(|level| level.n);
        let calibration_n = levels
            .get(levels.len().saturating_sub(2))
            .map(|level| level.n);
        FormalScaleSelection {
            selection_rule: FORMAL_LADDER_SCALE_SELECTION_RULE.to_owned(),
            disposition: FormalScaleSelectionDisposition::NoObservedKnee,
            calibration_n,
            stress_n,
            first_confirmed_knee_n: None,
        }
    }
}

fn unique_batch_summary(
    summaries: &[FormalLadderBatchSummary],
    n: u32,
    batch: u32,
    metric: FormalLadderMetric,
    sample_kind: FormalLadderSampleKind,
) -> Result<&FormalLadderBatchSummary, FormalLadderError> {
    unique_matching(
        summaries
            .iter()
            .filter(|summary| {
                summary.n == n
                    && summary.batch == batch
                    && summary.metric == metric
                    && summary.sample_kind == sample_kind
            })
            .collect(),
    )
}

fn unique_round_summary(
    summaries: &[FormalLadderRoundMetricSummary],
    n: u32,
    batch: u32,
    round: u32,
    metric: FormalLadderMetric,
    sample_kind: FormalLadderSampleKind,
) -> Result<&FormalLadderRoundMetricSummary, FormalLadderError> {
    unique_matching(
        summaries
            .iter()
            .filter(|summary| {
                summary.n == n
                    && summary.batch == batch
                    && summary.round == round
                    && summary.metric == metric
                    && summary.sample_kind == sample_kind
            })
            .collect(),
    )
}

fn unique_matching<T>(matching: Vec<&T>) -> Result<&T, FormalLadderError> {
    match matching.as_slice() {
        [only] => Ok(*only),
        [] => Err(FormalLadderError::MissingSummary),
        _ => Err(FormalLadderError::DuplicateSummary),
    }
}

fn normalized_ratio(
    upper: &FormalLadderRoundMetricSummary,
    lower: &FormalLadderRoundMetricSummary,
) -> Result<ExactPositiveRatio, FormalLadderError> {
    let numerator = u128::from(upper.median)
        .checked_mul(u128::from(lower.normalizer))
        .ok_or(FormalLadderError::ArithmeticOverflow)?;
    let denominator = u128::from(lower.median)
        .checked_mul(u128::from(upper.normalizer))
        .ok_or(FormalLadderError::ArithmeticOverflow)?;
    exact_ratio(numerator, denominator)
}

fn exact_ratio(
    numerator: u128,
    denominator: u128,
) -> Result<ExactPositiveRatio, FormalLadderError> {
    if numerator == 0 || denominator == 0 {
        return Err(FormalLadderError::InvalidRatio);
    }
    let divisor = gcd(numerator, denominator);
    Ok(ExactPositiveRatio {
        numerator: (numerator / divisor).to_string(),
        denominator: (denominator / divisor).to_string(),
    })
}

fn median_ratio(ratios: &[ExactPositiveRatio]) -> Result<ExactPositiveRatio, FormalLadderError> {
    if ratios.len() != FORMAL_LADDER_ROUND_COUNT as usize {
        return Err(FormalLadderError::InvalidRatioSet);
    }
    let mut sorted = ratios.to_vec();
    sorted.sort_by(compare_ratios);
    Ok(sorted[sorted.len() / 2].clone())
}

fn compare_ratios(left: &ExactPositiveRatio, right: &ExactPositiveRatio) -> Ordering {
    compare_positive_fractions(
        parse_ratio_component(&left.numerator),
        parse_ratio_component(&left.denominator),
        parse_ratio_component(&right.numerator),
        parse_ratio_component(&right.denominator),
    )
}

fn compare_positive_fractions(
    mut left_numerator: u128,
    mut left_denominator: u128,
    mut right_numerator: u128,
    mut right_denominator: u128,
) -> Ordering {
    let mut reversed = false;
    loop {
        let left_quotient = left_numerator / left_denominator;
        let right_quotient = right_numerator / right_denominator;
        if left_quotient != right_quotient {
            let order = left_quotient.cmp(&right_quotient);
            return if reversed { order.reverse() } else { order };
        }
        let left_remainder = left_numerator % left_denominator;
        let right_remainder = right_numerator % right_denominator;
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
        left_numerator = left_denominator;
        left_denominator = left_remainder;
        right_numerator = right_denominator;
        right_denominator = right_remainder;
        reversed = !reversed;
    }
}

fn candidate_knee(
    metric: FormalLadderMetric,
    ratios: &[ExactPositiveRatio],
    median: &ExactPositiveRatio,
) -> Result<bool, FormalLadderError> {
    match metric {
        FormalLadderMetric::WallTimeNs => Ok(ratios
            .iter()
            .filter(|ratio| ratio_at_least(ratio, 11, 10))
            .count()
            >= 4
            && ratio_at_least(median, 6, 5)),
        FormalLadderMetric::PeakLiveRequestedBytes => {
            Ok(ratios.iter().all(|ratio| ratio_at_least(ratio, 21, 20))
                && ratio_at_least(median, 11, 10))
        }
    }
}

fn ratio_at_least(ratio: &ExactPositiveRatio, numerator: u128, denominator: u128) -> bool {
    compare_positive_fractions(
        parse_ratio_component(&ratio.numerator),
        parse_ratio_component(&ratio.denominator),
        numerator,
        denominator,
    ) != Ordering::Less
}

fn parse_ratio_component(value: &str) -> u128 {
    value
        .parse()
        .expect("ratio components are produced from u128 values")
}

fn median_and_mad(values: &[u64]) -> Result<(u64, u64), FormalLadderError> {
    if values.is_empty() || values.len().is_multiple_of(2) {
        return Err(FormalLadderError::InvalidMetricValues);
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

fn required_strata() -> [(
    FormalLadderMetric,
    FormalLadderSampleKind,
    ScalableLadderBinaryMode,
); 4] {
    [
        (
            FormalLadderMetric::WallTimeNs,
            FormalLadderSampleKind::ColdInstance,
            ScalableLadderBinaryMode::Timing,
        ),
        (
            FormalLadderMetric::WallTimeNs,
            FormalLadderSampleKind::StableCapacityReuse,
            ScalableLadderBinaryMode::Timing,
        ),
        (
            FormalLadderMetric::PeakLiveRequestedBytes,
            FormalLadderSampleKind::ColdInstance,
            ScalableLadderBinaryMode::Attribution,
        ),
        (
            FormalLadderMetric::PeakLiveRequestedBytes,
            FormalLadderSampleKind::StableCapacityReuse,
            ScalableLadderBinaryMode::Attribution,
        ),
    ]
}

fn metric_token(metric: FormalLadderMetric) -> &'static str {
    match metric {
        FormalLadderMetric::WallTimeNs => "wall-time-ns",
        FormalLadderMetric::PeakLiveRequestedBytes => "peak-live-requested-bytes",
    }
}

fn sample_kind_token(sample_kind: FormalLadderSampleKind) -> &'static str {
    match sample_kind {
        FormalLadderSampleKind::ColdInstance => "cold-instance",
        FormalLadderSampleKind::StableCapacityReuse => "stable-capacity-reuse",
    }
}

fn mode_token(mode: ScalableLadderBinaryMode) -> &'static str {
    match mode {
        ScalableLadderBinaryMode::Timing => "timing",
        ScalableLadderBinaryMode::Attribution => "attribution",
    }
}

const fn gcd(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

#[derive(Debug, thiserror::Error)]
pub enum FormalLadderError {
    #[error("正式阶梯级别 {n} 的规模或归一化计数无效")]
    InvalidLevel { n: u32 },
    #[error("正式阶梯级别必须属于同一自然身份并按严格二倍递增")]
    InvalidLevelSequence,
    #[error("正式阶梯缺少 N={n}、batch={batch}、round={round}、mode={mode:?} 的运行")]
    MissingFormalRun {
        n: u32,
        batch: u32,
        round: u32,
        mode: ScalableLadderBinaryMode,
    },
    #[error("正式阶梯重复登记 N={n}、batch={batch}、round={round}、mode={mode:?} 的运行")]
    DuplicateFormalRun {
        n: u32,
        batch: u32,
        round: u32,
        mode: ScalableLadderBinaryMode,
    },
    #[error("正式阶梯子进程报告不完整或与运行分层不一致")]
    IncompleteChildReport,
    #[error("正式阶梯同一子进程内的语义摘要不一致")]
    SemanticDigestMismatch,
    #[error("正式阶梯指标样本数量或数值无效")]
    InvalidMetricValues,
    #[error("正式阶梯 N={n}、batch={batch} 未形成完整五轮")]
    IncompleteBatch { n: u32, batch: u32 },
    #[error("正式阶梯指标与二进制模式不一致")]
    MetricModeMismatch,
    #[error("正式阶梯缺少派生汇总")]
    MissingSummary,
    #[error("正式阶梯派生汇总自然身份重复")]
    DuplicateSummary,
    #[error("正式阶梯缺少 batch 1 的同分层确认比值")]
    MissingConfirmationRatio,
    #[error("正式阶梯比值必须由正整数形成")]
    InvalidRatio,
    #[error("正式阶梯相邻级别必须形成五个同轮比值")]
    InvalidRatioSet,
    #[error("正式阶梯精确整数运算溢出")]
    ArithmeticOverflow,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ATTRIBUTION_BINARY_ID, BASE_SCALE_STRING_PROFILE, GENERATOR_VERSION_V1,
        SCALABLE_LADDER_CHILD_SCHEMA, SCALABLE_LADDER_CHILD_SCHEMA_VERSION,
        STABLE_CAPACITY_WARMUP_COUNT, ScalableLadderSample, StageRetainedCapacityBytes,
        TIMING_BINARY_ID, WORKLOAD_REVISION_V1,
    };

    #[test]
    fn exact_medians_knees_and_first_confirmed_scale_are_recomputed_from_raw_reports() {
        let levels = [
            synthetic_level(1, 100, 100, 100, 100),
            synthetic_level(2, 200, 200, 200, 200),
            synthetic_level(4, 400, 400, 400, 400),
            synthetic_level(8, 800, 800, 1_000, 800),
            synthetic_level(16, 1_600, 1_600, 2_000, 1_600),
        ];
        let analysis = analyze_formal_ladder(&levels).expect("complete ladder");
        assert_eq!(analysis.round_summaries.len(), 5 * 2 * 5 * 4);
        assert_eq!(analysis.batch_summaries.len(), 5 * 2 * 4);
        assert!(analysis.knees.iter().any(|knee| {
            knee.upper_n == 8
                && knee.metric == FormalLadderMetric::WallTimeNs
                && knee.sample_kind == FormalLadderSampleKind::ColdInstance
                && knee.confirmed_knee
        }));
        assert_eq!(
            analysis.scale_selection.disposition,
            FormalScaleSelectionDisposition::ConfirmedKnee
        );
        assert_eq!(analysis.scale_selection.calibration_n, Some(4));
        assert_eq!(analysis.scale_selection.stress_n, Some(8));
    }

    #[test]
    fn five_complete_linear_levels_select_the_last_two_without_fabricating_a_knee() {
        let levels = [
            synthetic_level(1, 100, 100, 100, 100),
            synthetic_level(2, 200, 200, 200, 200),
            synthetic_level(4, 400, 400, 400, 400),
            synthetic_level(8, 800, 800, 800, 800),
            synthetic_level(16, 1_600, 1_600, 1_600, 1_600),
        ];
        let analysis = analyze_formal_ladder(&levels).expect("linear ladder");
        assert!(analysis.knees.iter().all(|knee| !knee.confirmed_knee));
        assert_eq!(
            analysis.scale_selection.disposition,
            FormalScaleSelectionDisposition::NoObservedKnee
        );
        assert_eq!(analysis.scale_selection.calibration_n, Some(8));
        assert_eq!(analysis.scale_selection.stress_n, Some(16));
    }

    #[test]
    fn fewer_than_five_levels_never_select_scales() {
        let levels = [
            synthetic_level(1, 100, 100, 100, 100),
            synthetic_level(2, 200, 200, 200, 200),
        ];
        let analysis = analyze_formal_ladder(&levels).expect("short ladder");
        assert_eq!(
            analysis.scale_selection.disposition,
            FormalScaleSelectionDisposition::InsufficientCompleteLevels
        );
        assert_eq!(analysis.scale_selection.calibration_n, None);
        assert_eq!(analysis.scale_selection.stress_n, None);
    }

    #[test]
    fn knee_thresholds_are_inclusive_and_exact() {
        let wall = [
            ratio(11, 10),
            ratio(6, 5),
            ratio(6, 5),
            ratio(6, 5),
            ratio(1, 1),
        ];
        assert!(
            candidate_knee(
                FormalLadderMetric::WallTimeNs,
                &wall,
                &median_ratio(&wall).unwrap()
            )
            .unwrap()
        );

        let peak = [
            ratio(21, 20),
            ratio(21, 20),
            ratio(11, 10),
            ratio(11, 10),
            ratio(11, 10),
        ];
        assert!(
            candidate_knee(
                FormalLadderMetric::PeakLiveRequestedBytes,
                &peak,
                &median_ratio(&peak).unwrap()
            )
            .unwrap()
        );
    }

    #[test]
    fn ratio_ordering_remains_exact_when_cross_products_would_overflow() {
        assert_eq!(
            compare_positive_fractions(u128::MAX, u128::MAX - 1, u128::MAX - 1, u128::MAX),
            Ordering::Greater
        );
        assert!(!ratio_at_least(&ratio(u128::MAX / 2, u128::MAX), 11, 10));
    }

    fn ratio(numerator: u128, denominator: u128) -> ExactPositiveRatio {
        exact_ratio(numerator, denominator).unwrap()
    }

    fn synthetic_level(
        n: u32,
        primary_record_count: u64,
        canonical_lir_record_count: u64,
        cold_wall_time: u64,
        peak_live_requested_bytes: u64,
    ) -> FormalLadderCompletedLevel {
        let mut runs = Vec::new();
        for batch in 0..FORMAL_LADDER_BATCH_COUNT {
            for round in 0..FORMAL_LADDER_ROUND_COUNT {
                runs.push(FormalLadderRoundRun {
                    batch,
                    round,
                    binary_mode: ScalableLadderBinaryMode::Timing,
                    report: synthetic_report(
                        n,
                        ScalableLadderBinaryMode::Timing,
                        cold_wall_time,
                        peak_live_requested_bytes,
                    ),
                });
                runs.push(FormalLadderRoundRun {
                    batch,
                    round,
                    binary_mode: ScalableLadderBinaryMode::Attribution,
                    report: synthetic_report(
                        n,
                        ScalableLadderBinaryMode::Attribution,
                        cold_wall_time,
                        peak_live_requested_bytes,
                    ),
                });
            }
        }
        FormalLadderCompletedLevel {
            workload_id: ScalableWorkloadId::Identity,
            graph_profile: "wide-star-v1".to_owned(),
            n,
            primary_record_count,
            canonical_lir_record_count,
            runs,
        }
    }

    fn synthetic_report(
        n: u32,
        mode: ScalableLadderBinaryMode,
        wall_time_ns: u64,
        peak_live_requested_bytes: u64,
    ) -> ScalableLadderChildReport {
        let sample = |sample_ordinal| ScalableLadderSample {
            sample_ordinal,
            wall_time_ns: (mode == ScalableLadderBinaryMode::Timing).then_some(wall_time_ns),
            attribution_wall_time_ns_diagnostic: (mode == ScalableLadderBinaryMode::Attribution)
                .then_some(wall_time_ns),
            semantic_digest_sha256: "00".repeat(32),
            guard_peak_live_requested_bytes: peak_live_requested_bytes,
            allocation: None,
        };
        ScalableLadderChildReport {
            schema: SCALABLE_LADDER_CHILD_SCHEMA.to_owned(),
            schema_version: SCALABLE_LADDER_CHILD_SCHEMA_VERSION,
            binary_id: match mode {
                ScalableLadderBinaryMode::Timing => TIMING_BINARY_ID,
                ScalableLadderBinaryMode::Attribution => ATTRIBUTION_BINARY_ID,
            }
            .to_owned(),
            binary_mode: mode,
            allocation_instrumentation_enabled: mode == ScalableLadderBinaryMode::Attribution,
            compiler_instance_id: format!("synthetic/{n}/{mode:?}"),
            child_pid: 1,
            workload_id: ScalableWorkloadId::Identity,
            workload_revision: WORKLOAD_REVISION_V1,
            graph_profile: "wide-star-v1".to_owned(),
            string_profile: BASE_SCALE_STRING_PROFILE.to_owned(),
            generator_version: GENERATOR_VERSION_V1,
            n,
            outcome: ScalableLadderOutcome::Success,
            controlled_allocation_hard_ceiling_bytes: u64::MAX,
            warmup_count: STABLE_CAPACITY_WARMUP_COUNT,
            cold_instance: Some(sample(0)),
            stable_capacity_reuse: (0..7).map(sample).collect(),
            retained_capacity_bytes: Some(StageRetainedCapacityBytes {
                total: peak_live_requested_bytes,
                ..StageRetainedCapacityBytes::default()
            }),
            controlled_allocation_guard: None,
        }
    }
}
