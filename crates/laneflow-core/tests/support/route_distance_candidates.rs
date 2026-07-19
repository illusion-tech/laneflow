//! #127 路线派生距离的研究专用候选。

use std::mem::size_of;

const MIN_EDGE_LENGTH_METERS: f64 = 1.0;
const MAX_EDGE_LENGTH_METERS: f64 = 10_000.0;
const F64_SEGMENT_OCCURRENCES: usize = 3_518_437_208;
const CHUNKS_PER_SEGMENT: usize = 586_406_201;
pub const CHUNK_OCCURRENCES: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BoundedDistance {
    Finite(f64),
    BeyondFinite,
}

impl BoundedDistance {
    fn add(self, value: f64) -> Self {
        match self {
            Self::Finite(current) if value <= f64::MAX - current => Self::Finite(current + value),
            Self::Finite(_) | Self::BeyondFinite => Self::BeyondFinite,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum QueryResult {
    Passed,
    BeyondHorizon,
    Within(f64),
}

#[derive(Clone, Copy, Debug, Default)]
struct WidePrefix {
    high: f64,
    low: f64,
}

impl WidePrefix {
    fn add(self, value: f64) -> Self {
        let (high, roundoff) = two_sum(self.high, value);
        let (high, low) = two_sum(high, self.low + roundoff);
        debug_assert!(high.is_finite());
        Self { high, low }
    }

    fn distance_from(self, earlier: Self) -> f64 {
        let (high, roundoff) = two_sum(self.high, -earlier.high);
        high + (self.low - earlier.low + roundoff)
    }
}

fn two_sum(left: f64, right: f64) -> (f64, f64) {
    let sum = left + right;
    let right_virtual = sum - left;
    let roundoff = (left - (sum - right_virtual)) + (right - right_virtual);
    (sum, roundoff)
}

#[derive(Clone, Copy, Debug)]
struct Segment {
    prefix_before: WidePrefix,
    total: f64,
}

#[derive(Clone, Debug)]
pub struct F64PrefixIndex {
    offsets: Vec<f64>,
    segments: Vec<Segment>,
    distance_to_end: Vec<BoundedDistance>,
    segment_occurrences: usize,
}

impl F64PrefixIndex {
    pub fn build(edge_lengths: &[f32]) -> Self {
        Self::build_with_segment_occurrences(edge_lengths, F64_SEGMENT_OCCURRENCES)
    }

    pub fn build_with_segment_occurrences(
        edge_lengths: &[f32],
        segment_occurrences: usize,
    ) -> Self {
        assert!(segment_occurrences > 0);
        let mut offsets = Vec::with_capacity(edge_lengths.len());
        let mut segments = Vec::new();
        let mut segment_total = 0.0;
        let mut prefix_before = WidePrefix::default();
        for (occurrence, edge_length) in edge_lengths.iter().copied().enumerate() {
            let edge_length = f64::from(edge_length);
            assert!(
                edge_length.is_finite()
                    && edge_length > MIN_EDGE_LENGTH_METERS
                    && edge_length <= MAX_EDGE_LENGTH_METERS
            );
            if occurrence > 0 && occurrence.is_multiple_of(segment_occurrences) {
                segments.push(Segment {
                    prefix_before,
                    total: segment_total,
                });
                prefix_before = prefix_before.add(segment_total);
                segment_total = 0.0;
            }
            offsets.push(segment_total);
            segment_total += edge_length;
            debug_assert!(segment_total.is_finite());
        }
        if !edge_lengths.is_empty() {
            segments.push(Segment {
                prefix_before,
                total: segment_total,
            });
        }
        Self {
            offsets,
            segments,
            distance_to_end: suffix_distances(edge_lengths),
            segment_occurrences,
        }
    }

    pub fn distance_to_end_within(
        &self,
        from_occurrence: usize,
        from_progress: f64,
        horizon: f64,
    ) -> QueryResult {
        distance_to_end_within(
            &self.distance_to_end,
            from_occurrence,
            from_progress,
            horizon,
        )
    }

    pub fn distance_within(
        &self,
        from_occurrence: usize,
        from_progress: f64,
        target_occurrence: usize,
        target_progress: f64,
        horizon: f64,
    ) -> QueryResult {
        if is_passed(
            from_occurrence,
            from_progress,
            target_occurrence,
            target_progress,
        ) {
            return QueryResult::Passed;
        }
        let Some(&from_offset) = self.offsets.get(from_occurrence) else {
            return QueryResult::Passed;
        };
        let Some(&target_offset) = self.offsets.get(target_occurrence) else {
            return QueryResult::Passed;
        };
        distance_between_coordinates(
            &self.segments,
            from_occurrence / self.segment_occurrences,
            from_offset,
            from_progress,
            target_occurrence / self.segment_occurrences,
            target_offset,
            target_progress,
            horizon,
        )
    }

    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            + self.offsets.capacity() * size_of::<f64>()
            + self.segments.capacity() * size_of::<Segment>()
            + self.distance_to_end.capacity() * size_of::<BoundedDistance>()
    }
}

#[derive(Clone, Copy, Debug)]
struct Chunk {
    segment_offset: f64,
    total: f64,
}

#[derive(Clone, Debug)]
pub struct ChunkedLocalF32Index {
    local_offsets: Vec<f32>,
    chunks: Vec<Chunk>,
    segments: Vec<Segment>,
    distance_to_end: Vec<BoundedDistance>,
    chunks_per_segment: usize,
}

impl ChunkedLocalF32Index {
    pub fn build(edge_lengths: &[f32]) -> Self {
        Self::build_with_segment_chunks(edge_lengths, CHUNKS_PER_SEGMENT)
    }

    pub fn build_with_segment_chunks(edge_lengths: &[f32], chunks_per_segment: usize) -> Self {
        assert!(chunks_per_segment > 0);
        let mut local_offsets = Vec::with_capacity(edge_lengths.len());
        let mut chunks = Vec::with_capacity(edge_lengths.len().div_ceil(CHUNK_OCCURRENCES));
        let mut segments = Vec::new();
        let mut segment_total = 0.0;
        let mut prefix_before = WidePrefix::default();
        for (chunk_index, chunk_lengths) in edge_lengths.chunks(CHUNK_OCCURRENCES).enumerate() {
            if chunk_index > 0 && chunk_index.is_multiple_of(chunks_per_segment) {
                segments.push(Segment {
                    prefix_before,
                    total: segment_total,
                });
                prefix_before = prefix_before.add(segment_total);
                segment_total = 0.0;
            }
            let chunk_total = chunk_lengths.iter().copied().map(f64::from).sum::<f64>();
            chunks.push(Chunk {
                segment_offset: segment_total,
                total: chunk_total,
            });
            let mut local_total = 0.0_f64;
            for edge_length in chunk_lengths.iter().copied() {
                let edge_length = f64::from(edge_length);
                assert!(
                    edge_length.is_finite()
                        && edge_length > MIN_EDGE_LENGTH_METERS
                        && edge_length <= MAX_EDGE_LENGTH_METERS
                );
                local_offsets.push(local_total as f32);
                local_total += edge_length;
            }
            segment_total += chunk_total;
            debug_assert!(segment_total.is_finite());
        }
        if !edge_lengths.is_empty() {
            segments.push(Segment {
                prefix_before,
                total: segment_total,
            });
        }
        Self {
            local_offsets,
            chunks,
            segments,
            distance_to_end: suffix_distances(edge_lengths),
            chunks_per_segment,
        }
    }

    pub fn distance_to_end_within(
        &self,
        from_occurrence: usize,
        from_progress: f64,
        horizon: f64,
    ) -> QueryResult {
        distance_to_end_within(
            &self.distance_to_end,
            from_occurrence,
            from_progress,
            horizon,
        )
    }

    pub fn distance_within(
        &self,
        from_occurrence: usize,
        from_progress: f64,
        target_occurrence: usize,
        target_progress: f64,
        horizon: f64,
    ) -> QueryResult {
        if is_passed(
            from_occurrence,
            from_progress,
            target_occurrence,
            target_progress,
        ) {
            return QueryResult::Passed;
        }
        let Some(&from_local) = self.local_offsets.get(from_occurrence) else {
            return QueryResult::Passed;
        };
        let Some(&target_local) = self.local_offsets.get(target_occurrence) else {
            return QueryResult::Passed;
        };
        let from_chunk_index = from_occurrence / CHUNK_OCCURRENCES;
        let target_chunk_index = target_occurrence / CHUNK_OCCURRENCES;
        let Some(from_chunk) = self.chunks.get(from_chunk_index) else {
            return QueryResult::Passed;
        };
        let Some(target_chunk) = self.chunks.get(target_chunk_index) else {
            return QueryResult::Passed;
        };
        distance_between_coordinates(
            &self.segments,
            from_chunk_index / self.chunks_per_segment,
            from_chunk.segment_offset + f64::from(from_local),
            from_progress,
            target_chunk_index / self.chunks_per_segment,
            target_chunk.segment_offset + f64::from(target_local),
            target_progress,
            horizon,
        )
    }

    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            + self.local_offsets.capacity() * size_of::<f32>()
            + self.chunks.capacity() * size_of::<Chunk>()
            + self.segments.capacity() * size_of::<Segment>()
            + self.distance_to_end.capacity() * size_of::<BoundedDistance>()
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    pub fn total_chunk_length(&self) -> f64 {
        self.chunks.iter().map(|chunk| chunk.total).sum()
    }

    pub fn max_local_offset_error(&self, edge_lengths: &[f32]) -> f64 {
        let mut maximum = 0.0_f64;
        for (chunk_index, chunk) in edge_lengths.chunks(CHUNK_OCCURRENCES).enumerate() {
            let mut exact = 0.0;
            for (local_index, edge_length) in chunk.iter().copied().enumerate() {
                let occurrence = chunk_index * CHUNK_OCCURRENCES + local_index;
                maximum = maximum.max((f64::from(self.local_offsets[occurrence]) - exact).abs());
                exact += f64::from(edge_length);
            }
        }
        maximum
    }
}

fn suffix_distances(edge_lengths: &[f32]) -> Vec<BoundedDistance> {
    let mut distances = vec![BoundedDistance::Finite(0.0); edge_lengths.len()];
    let mut suffix = BoundedDistance::Finite(0.0);
    for (index, edge_length) in edge_lengths.iter().copied().enumerate().rev() {
        suffix = suffix.add(f64::from(edge_length));
        distances[index] = suffix;
    }
    distances
}

fn distance_to_end_within(
    distances: &[BoundedDistance],
    from_occurrence: usize,
    from_progress: f64,
    horizon: f64,
) -> QueryResult {
    let Some(distance) = distances.get(from_occurrence).copied() else {
        return QueryResult::Passed;
    };
    match distance {
        BoundedDistance::BeyondFinite => QueryResult::BeyondHorizon,
        BoundedDistance::Finite(distance) => classify(distance - from_progress, horizon),
    }
}

#[allow(clippy::too_many_arguments)]
fn distance_between_coordinates(
    segments: &[Segment],
    from_segment: usize,
    from_offset: f64,
    from_progress: f64,
    target_segment: usize,
    target_offset: f64,
    target_progress: f64,
    horizon: f64,
) -> QueryResult {
    if from_segment == target_segment {
        return classify(
            (target_offset + target_progress) - (from_offset + from_progress),
            horizon,
        );
    }
    let mut distance = segments[from_segment].total - (from_offset + from_progress);
    if distance > horizon {
        return QueryResult::BeyondHorizon;
    }
    if target_segment > from_segment + 1 {
        let first_middle = from_segment + 1;
        let middle = segments[target_segment]
            .prefix_before
            .distance_from(segments[first_middle].prefix_before);
        if middle > horizon - distance {
            return QueryResult::BeyondHorizon;
        }
        distance += middle;
    }
    let target_distance = target_offset + target_progress;
    if target_distance > horizon - distance {
        return QueryResult::BeyondHorizon;
    }
    classify(distance + target_distance, horizon)
}

fn is_passed(
    from_occurrence: usize,
    from_progress: f64,
    target_occurrence: usize,
    target_progress: f64,
) -> bool {
    target_occurrence < from_occurrence
        || (target_occurrence == from_occurrence && target_progress < from_progress)
}

fn classify(distance: f64, horizon: f64) -> QueryResult {
    if distance <= horizon {
        QueryResult::Within(distance.max(0.0))
    } else {
        QueryResult::BeyondHorizon
    }
}

pub fn repeated_route_indices(
    route_count: usize,
    occurrences_per_route: usize,
    edge_length: f32,
) -> (Vec<F64PrefixIndex>, Vec<ChunkedLocalF32Index>) {
    let lengths = vec![edge_length; occurrences_per_route];
    let f64_indices = (0..route_count)
        .map(|_| F64PrefixIndex::build(&lengths))
        .collect();
    let chunked_indices = (0..route_count)
        .map(|_| ChunkedLocalF32Index::build(&lengths))
        .collect();
    (f64_indices, chunked_indices)
}

#[allow(clippy::ptr_arg, reason = "研究账本需要读取外层 Vec 的实际容量")]
pub fn collection_retained_bytes<T>(indices: &Vec<T>, nested: impl Fn(&T) -> usize) -> usize {
    indices.capacity() * size_of::<T>() + indices.iter().map(nested).sum::<usize>()
        - indices.len() * size_of::<T>()
}

#[cfg(test)]
mod tests {
    #[allow(
        unused_imports,
        reason = "Criterion 以 cfg(test) 编译共享模块但不收集测试"
    )]
    use super::*;

    #[test]
    fn fixed_segment_cap_keeps_local_f64_ulp_below_one_centimeter() {
        let limit = 2.0_f64.powi(45);
        assert!(F64_SEGMENT_OCCURRENCES as f64 * MAX_EDGE_LENGTH_METERS <= limit);
        assert!((F64_SEGMENT_OCCURRENCES + 1) as f64 * MAX_EDGE_LENGTH_METERS > limit);
        assert!(
            CHUNKS_PER_SEGMENT as f64 * CHUNK_OCCURRENCES as f64 * MAX_EDGE_LENGTH_METERS <= limit
        );
        assert!(
            (CHUNKS_PER_SEGMENT + 1) as f64 * CHUNK_OCCURRENCES as f64 * MAX_EDGE_LENGTH_METERS
                > limit
        );
        assert_eq!(limit.next_up() - limit, 0.007_812_5);
    }

    #[test]
    fn wide_prefix_recovers_small_distance_after_huge_prefix() {
        let mut earlier = WidePrefix::default();
        for _ in 0..10 {
            earlier = earlier.add(1.0e22);
        }
        let expected = 12_345.678_901_234;
        let later = earlier.add(expected);
        assert!((later.distance_from(earlier) - expected).abs() <= 1.0e-9);
    }
}
