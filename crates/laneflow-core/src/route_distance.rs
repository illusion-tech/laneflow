//! Overflow-safe route occurrence distance index。

/// 将 route 前缀限制在局部 segment 内，避免长 route 查询用两个过大的全局前缀相减。
const ROUTE_DISTANCE_SEGMENT_LIMIT_METERS: f64 = 1_000_000_000.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum BoundedDistance {
    Finite(f64),
    BeyondFinite,
}

impl BoundedDistance {
    pub(crate) fn add(self, value: f64) -> Self {
        match self {
            Self::Finite(current) if value <= f64::MAX - current => Self::Finite(current + value),
            Self::Finite(_) | Self::BeyondFinite => Self::BeyondFinite,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RouteDistanceQuery {
    Passed,
    BeyondHorizon,
    Within(f64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct OccurrenceCoordinate {
    segment: usize,
    offset: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RouteDistanceIndex {
    occurrences: Vec<OccurrenceCoordinate>,
    segment_totals: Vec<f64>,
    distance_to_end: Vec<BoundedDistance>,
}

impl RouteDistanceIndex {
    pub(crate) fn build(edge_lengths: &[f32]) -> Self {
        Self::build_with_segment_limit(edge_lengths, ROUTE_DISTANCE_SEGMENT_LIMIT_METERS)
    }

    fn build_with_segment_limit(edge_lengths: &[f32], segment_limit: f64) -> Self {
        let mut occurrences = Vec::with_capacity(edge_lengths.len());
        let mut segment_totals = Vec::new();
        let mut current_total = 0.0;
        let mut current_has_occurrence = false;

        for edge_length in edge_lengths.iter().copied().map(f64::from) {
            debug_assert!(edge_length.is_finite() && edge_length > 0.0);
            let must_start_segment = current_has_occurrence
                && (edge_length > segment_limit
                    || current_total > segment_limit - edge_length
                    || current_total + edge_length == current_total);
            if must_start_segment {
                segment_totals.push(current_total);
                current_total = 0.0;
            }

            let segment = segment_totals.len();
            occurrences.push(OccurrenceCoordinate {
                segment,
                offset: current_total,
            });
            current_total += edge_length;
            current_has_occurrence = true;

            if edge_length > segment_limit {
                segment_totals.push(current_total);
                current_total = 0.0;
                current_has_occurrence = false;
            }
        }
        if current_has_occurrence {
            segment_totals.push(current_total);
        }

        let mut distance_to_end = vec![BoundedDistance::Finite(0.0); edge_lengths.len()];
        let mut suffix = BoundedDistance::Finite(0.0);
        for (index, edge_length) in edge_lengths
            .iter()
            .copied()
            .map(f64::from)
            .enumerate()
            .rev()
        {
            suffix = suffix.add(edge_length);
            distance_to_end[index] = suffix;
        }

        Self {
            occurrences,
            segment_totals,
            distance_to_end,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.occurrences.clear();
        self.segment_totals.clear();
        self.distance_to_end.clear();
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        self.occurrences.capacity() * std::mem::size_of::<OccurrenceCoordinate>()
            + self.segment_totals.capacity() * std::mem::size_of::<f64>()
            + self.distance_to_end.capacity() * std::mem::size_of::<BoundedDistance>()
    }

    pub(crate) fn distance_to_end_within(
        &self,
        from_occurrence: usize,
        from_progress: f64,
        horizon: f64,
    ) -> RouteDistanceQuery {
        let Some(distance) = self.distance_to_end.get(from_occurrence).copied() else {
            return RouteDistanceQuery::Passed;
        };
        match distance {
            BoundedDistance::BeyondFinite => RouteDistanceQuery::BeyondHorizon,
            BoundedDistance::Finite(distance) => {
                let remaining = distance - from_progress;
                if remaining <= horizon {
                    RouteDistanceQuery::Within(remaining.max(0.0))
                } else {
                    RouteDistanceQuery::BeyondHorizon
                }
            }
        }
    }

    pub(crate) fn distance_within(
        &self,
        from_occurrence: usize,
        from_progress: f64,
        target_occurrence: usize,
        target_progress: f64,
        horizon: f64,
    ) -> RouteDistanceQuery {
        if target_occurrence < from_occurrence
            || (target_occurrence == from_occurrence && target_progress < from_progress)
        {
            return RouteDistanceQuery::Passed;
        }
        let Some(from) = self.occurrences.get(from_occurrence).copied() else {
            return RouteDistanceQuery::Passed;
        };
        let Some(target) = self.occurrences.get(target_occurrence).copied() else {
            return RouteDistanceQuery::Passed;
        };

        if from.segment == target.segment {
            let distance = (target.offset + target_progress) - (from.offset + from_progress);
            return if distance <= horizon {
                RouteDistanceQuery::Within(distance.max(0.0))
            } else {
                RouteDistanceQuery::BeyondHorizon
            };
        }

        let from_segment_total = self.segment_totals[from.segment];
        let mut distance = from_segment_total - (from.offset + from_progress);
        if distance > horizon {
            return RouteDistanceQuery::BeyondHorizon;
        }
        for segment in (from.segment + 1)..target.segment {
            let segment_total = self.segment_totals[segment];
            if segment_total > horizon - distance || (distance > 0.0 && segment_total >= horizon) {
                return RouteDistanceQuery::BeyondHorizon;
            }
            distance += segment_total;
        }
        let target_distance = target.offset + target_progress;
        if target_distance > horizon - distance || (distance > 0.0 && target_distance >= horizon) {
            return RouteDistanceQuery::BeyondHorizon;
        }
        RouteDistanceQuery::Within(distance + target_distance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn bounded_index_matches_independent_integer_oracle(
            raw_lengths in prop::collection::vec(1_u16..=1_000, 1..=64),
            raw_from in any::<usize>(),
            raw_target in any::<usize>(),
            raw_from_progress in any::<u16>(),
            raw_target_progress in any::<u16>(),
            horizon in 0_u32..=100_000,
        ) {
            let lengths = raw_lengths
                .iter()
                .copied()
                .map(f32::from)
                .collect::<Vec<_>>();
            let from = raw_from % lengths.len();
            let target = raw_target % lengths.len();
            let from_progress = f64::from(raw_from_progress % (raw_lengths[from] + 1));
            let target_progress = f64::from(raw_target_progress % (raw_lengths[target] + 1));
            let horizon = f64::from(horizon);
            let expected = if target < from
                || (target == from && target_progress < from_progress)
            {
                RouteDistanceQuery::Passed
            } else {
                let distance = if target == from {
                    target_progress - from_progress
                } else {
                    f64::from(lengths[from]) - from_progress
                        + lengths[(from + 1)..target]
                            .iter()
                            .copied()
                            .map(f64::from)
                            .sum::<f64>()
                        + target_progress
                };
                if distance <= horizon {
                    RouteDistanceQuery::Within(distance)
                } else {
                    RouteDistanceQuery::BeyondHorizon
                }
            };

            let actual = RouteDistanceIndex::build(&lengths).distance_within(
                from,
                from_progress,
                target,
                target_progress,
                horizon,
            );
            prop_assert_eq!(actual, expected);
        }
    }

    #[test]
    fn segmented_query_preserves_local_boundaries() {
        let index =
            RouteDistanceIndex::build_with_segment_limit(&[10_000.0, 1.0, 2.0, 10_000.0], 10_000.0);

        assert_eq!(
            index.distance_within(1, 0.0, 2, 2.0, 3.0),
            RouteDistanceQuery::Within(3.0)
        );
        assert_eq!(
            index.distance_within(1, 0.0, 3, 10_000.0, 10_003.0),
            RouteDistanceQuery::Within(10_003.0)
        );
        assert_eq!(
            index.distance_within(2, 1.0, 1, 0.0, 20_000.0),
            RouteDistanceQuery::Passed
        );
    }
}
