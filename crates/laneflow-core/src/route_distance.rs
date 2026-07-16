//! Overflow-safe route occurrence distance index。

const ROUTE_DISTANCE_SEGMENT_LIMIT: f64 = f64::MAX / 16.0;

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
    pub(crate) fn build(edge_lengths: &[f64]) -> Self {
        let mut occurrences = Vec::with_capacity(edge_lengths.len());
        let mut segment_totals = Vec::new();
        let mut current_total = 0.0;
        let mut current_has_occurrence = false;

        for edge_length in edge_lengths.iter().copied() {
            debug_assert!(edge_length.is_finite() && edge_length > 0.0);
            let must_start_segment = current_has_occurrence
                && (edge_length > ROUTE_DISTANCE_SEGMENT_LIMIT
                    || current_total > ROUTE_DISTANCE_SEGMENT_LIMIT - edge_length
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

            if edge_length > ROUTE_DISTANCE_SEGMENT_LIMIT {
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
        for (index, edge_length) in edge_lengths.iter().copied().enumerate().rev() {
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

    #[test]
    fn total_overflow_is_beyond_any_finite_horizon() {
        let index = RouteDistanceIndex::build(&[f64::MAX, f64::MAX]);

        assert_eq!(
            index.distance_to_end_within(0, 0.0, f64::MAX),
            RouteDistanceQuery::BeyondHorizon
        );
        assert_eq!(
            index.distance_to_end_within(1, 0.0, f64::MAX),
            RouteDistanceQuery::Within(f64::MAX)
        );
    }

    #[test]
    fn segmented_query_preserves_huge_tiny_boundaries() {
        let index = RouteDistanceIndex::build(&[f64::MAX, 1.0, 2.0, f64::MAX]);

        assert_eq!(
            index.distance_within(1, 0.0, 2, 2.0, 3.0),
            RouteDistanceQuery::Within(3.0)
        );
        assert_eq!(
            index.distance_within(1, 0.0, 3, f64::MAX, f64::MAX),
            RouteDistanceQuery::BeyondHorizon
        );
        assert_eq!(
            index.distance_within(2, 1.0, 1, 0.0, f64::MAX),
            RouteDistanceQuery::Passed
        );
    }
}
