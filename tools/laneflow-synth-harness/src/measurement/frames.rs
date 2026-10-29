use laneflow_synth_generator::Case;
use serde::Serialize;

use crate::Plan;

#[derive(Clone, Copy, Debug, Serialize)]
pub(super) struct FrameInput {
    pub index: u64,
    pub delta_ns: u64,
    pub boundary: bool,
    pub observation: bool,
    pub burst: Option<u8>,
}

pub(super) fn inputs(plan: &Plan) -> Vec<FrameInput> {
    let quantum = u64::from(plan.identity.fixed_delta_ms) * 1_000_000;
    let mut targets = vec![plan.warm_up_ticks];
    if plan.identity.case != Case::W4 {
        targets.extend([1_024, 2_048, 4_096].map(|n| plan.warm_up_ticks + n));
        targets.push(plan.warm_up_ticks + plan.observation_ticks);
    }
    targets.sort_unstable();
    targets.dedup();
    let mut result = Vec::new();
    let mut elapsed = 0;
    let mut index = 0;
    for target in targets {
        let target_ns = target * quantum;
        while elapsed < target_ns {
            let nominal = (index + 1) * 1_000_000_000 / 60 - index * 1_000_000_000 / 60;
            let delta_ns = nominal.min(target_ns - elapsed);
            result.push(FrameInput {
                index,
                delta_ns,
                // 抵达检查点的完整帧也单列，避免审计工作与普通帧身份混淆。
                boundary: elapsed + delta_ns == target_ns,
                observation: elapsed >= plan.warm_up_ticks * quantum,
                burst: None,
            });
            elapsed += delta_ns;
            index += 1;
        }
    }
    if plan.identity.case == Case::W4 {
        for burst in 0..2 {
            result.push(FrameInput {
                index,
                delta_ns: quantum,
                boundary: false,
                observation: true,
                burst: Some(burst),
            });
            index += 1;
        }
    }
    result
}

pub(super) fn selection(scale: u32, percent: u8) -> Vec<usize> {
    let mut ranked: Vec<_> = (0..scale)
        .map(|rank| {
            let mut key = u64::from(rank).wrapping_add(0x9e37_79b9_7f4a_7c15);
            key = (key ^ (key >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            key = (key ^ (key >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            (key ^ (key >> 31), rank)
        })
        .collect();
    ranked.sort_unstable();
    ranked
        .into_iter()
        .take(scale as usize * usize::from(percent) / 100)
        .map(|(_, rank)| rank as usize)
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct Distribution {
    pub samples: usize,
    pub p50_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub max_ns: u64,
}

pub(super) fn distribution(values: impl Iterator<Item = u64>) -> Option<Distribution> {
    let mut values: Vec<_> = values.collect();
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let at = |percent: usize| values[(values.len() * percent).div_ceil(100) - 1];
    Some(Distribution {
        samples: values.len(),
        p50_ns: at(50),
        p95_ns: at(95),
        p99_ns: at(99),
        max_ns: *values.last().unwrap(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_is_exact_nested_and_not_an_id_prefix() {
        let full = selection(10_000, 100);
        let mut sorted = full.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..10_000).collect::<Vec<_>>());
        for percent in [1, 10, 50, 100] {
            let selected = selection(10_000, percent);
            assert_eq!(selected, full[..100 * usize::from(percent)]);
            assert!(selected.iter().any(|&rank| rank >= 9_000));
            assert!(selected.iter().any(|&rank| rank % 100 >= 75));
        }
        assert_eq!(selection(10_000, 1).len(), 100);
        assert_eq!(
            &full[..12],
            &[
                7_326, 8_715, 5_644, 9_342, 558, 4_440, 4_522, 1_048, 6_426, 4_403, 3_235, 1_486
            ]
        );
    }

    #[test]
    fn nearest_rank_uses_the_full_tail_and_keeps_empty_distinct() {
        assert!(distribution(std::iter::empty()).is_none());
        let d = distribution((1..=100).rev()).unwrap();
        assert_eq!(
            (d.samples, d.p50_ns, d.p95_ns, d.p99_ns, d.max_ns),
            (100, 50, 95, 99, 100)
        );
    }

    #[test]
    fn frozen_inputs_cover_exact_boundaries_without_zero_or_lost_time() {
        for dt in [16, 33] {
            for case in [Case::W1, Case::W3, Case::W4] {
                let warm = 4 * crate::cycle_ms(dt) / u64::from(dt);
                let observation = if case == Case::W4 { 2 } else { 2 * warm };
                let plan = Plan {
                    format: String::new(),
                    identity: laneflow_synth_generator::Identity::new(10_000, case, dt).unwrap(),
                    warm_up_ticks: warm,
                    observation_ticks: observation,
                    boundary_order: Vec::new(),
                    scheduled: Vec::new(),
                    rules: Vec::new(),
                };
                let frames = inputs(&plan);
                let quantum = u64::from(dt) * 1_000_000;
                assert_eq!(
                    frames.iter().map(|f| f.delta_ns).sum::<u64>(),
                    (warm + observation) * quantum
                );
                let mut elapsed = 0;
                let mut boundaries = Vec::new();
                for (index, frame) in frames.iter().enumerate() {
                    assert_eq!(frame.index, index as u64);
                    assert!(frame.delta_ns > 0);
                    assert_eq!(frame.observation, elapsed >= warm * quantum);
                    elapsed += frame.delta_ns;
                    if frame.boundary {
                        assert!(elapsed.is_multiple_of(quantum));
                        boundaries.push(elapsed / quantum);
                    }
                }
                if case == Case::W4 {
                    assert_eq!(boundaries, [warm]);
                    let observed: Vec<_> = frames.iter().filter(|f| f.observation).collect();
                    assert_eq!(observed.len(), 2);
                    assert_eq!(
                        observed
                            .iter()
                            .map(|f| (f.delta_ns, f.burst))
                            .collect::<Vec<_>>(),
                        [(quantum, Some(0)), (quantum, Some(1))]
                    );
                } else {
                    assert_eq!(
                        boundaries,
                        [
                            warm,
                            warm + 1_024,
                            warm + 2_048,
                            warm + 4_096,
                            warm + observation
                        ]
                    );
                }
            }
        }
    }
}
