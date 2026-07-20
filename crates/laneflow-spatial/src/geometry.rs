//! 量化后折线、长度绑定与确定性采样。

use laneflow_core::{EdgeHandle, EdgeLength, EdgeProgress};

use crate::{CanonicalPoint3F32, CanonicalUnitVector3F32, CanonicalVector3F32, SpatialError};

/// 中心线线段允许的最小长度，单位为米；有效线段必须严格大于该值。
pub const SPATIAL_MIN_SEGMENT_LENGTH_METERS: f32 = 0.1;

/// Core 长度与几何弧长绑定的绝对容差下限，单位为米。
pub const SPATIAL_LENGTH_ABS_TOLERANCE_METERS: f64 = 0.01;

/// Core 长度与几何弧长绑定的相对容差系数。
pub const SPATIAL_LENGTH_REL_TOLERANCE: f64 = 1.0e-6;

/// current-f64 Core edge length 的量化余量，单位为米。
pub const SPATIAL_CORE_LENGTH_QUANTIZATION_ALLOWANCE_METERS: f64 = 0.0;

/// 已连接 edge 端点允许的最大距离，单位为米。
pub const SPATIAL_JOIN_POSITION_TOLERANCE_METERS: f32 = 0.005;

/// canonical `+Y` 投影长度允许的最小值；等于该值时有效。
///
/// 该值是 `sin(0.5°)` 的 `f32` 冻结值。
pub const SPATIAL_MIN_PROJECTED_UP_LENGTH: f32 = 0.008_726_535;

/// 一条 Core edge 及其借用的量化后 canonical 中心线输入。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpatialEdgeInput<'a> {
    edge: EdgeHandle,
    points: &'a [CanonicalPoint3F32],
}

impl<'a> SpatialEdgeInput<'a> {
    /// 创建不复制点数据的 edge 绑定输入。
    pub const fn new(edge: EdgeHandle, points: &'a [CanonicalPoint3F32]) -> Self {
        Self { edge, points }
    }

    /// 返回 Core edge handle。
    pub const fn edge(self) -> EdgeHandle {
        self.edge
    }

    /// 返回量化后的 canonical 中心线点。
    pub const fn points(self) -> &'a [CanonicalPoint3F32] {
        self.points
    }
}

/// canonical frame 中的采样位姿。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalPoseF32 {
    position: CanonicalPoint3F32,
    tangent: CanonicalUnitVector3F32,
    up: CanonicalUnitVector3F32,
}

impl CanonicalPoseF32 {
    /// 返回采样位置，单位为米。
    pub const fn position(self) -> CanonicalPoint3F32 {
        self.position
    }

    /// 返回沿中心线行驶方向的单位切向量。
    pub const fn tangent(self) -> CanonicalUnitVector3F32 {
        self.tangent
    }

    /// 返回 canonical `+Y` 在切向量正交平面上的单位投影。
    pub const fn up(self) -> CanonicalUnitVector3F32 {
        self.up
    }
}

#[derive(Clone, Debug, PartialEq)]
struct PolylineSegment {
    length: f32,
    cumulative_end: f32,
    tangent: CanonicalUnitVector3F32,
    up: CanonicalUnitVector3F32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BoundPolyline {
    points: Vec<CanonicalPoint3F32>,
    segments: Vec<PolylineSegment>,
    core_length: EdgeLength,
    arc_length: f32,
}

impl BoundPolyline {
    pub(crate) fn try_new(
        edge: EdgeHandle,
        core_length: EdgeLength,
        points: &[CanonicalPoint3F32],
    ) -> Result<Self, SpatialError> {
        if points.len() < 2 {
            return Err(SpatialError::InsufficientPolylinePoints {
                edge,
                actual: points.len(),
                min: 2,
            });
        }

        let mut segments = Vec::with_capacity(points.len() - 1);
        let mut cumulative = 0.0_f32;
        for (segment_index, pair) in points.windows(2).enumerate() {
            let start = pair[0];
            let end = pair[1];
            let delta = start
                .checked_vector_to(end)
                .expect("two valid canonical points always have a finite difference");
            let length = vector_length(delta);
            if length <= SPATIAL_MIN_SEGMENT_LENGTH_METERS {
                return Err(SpatialError::DegenerateSegment {
                    edge,
                    segment_index,
                    length_meters: length,
                    min_exclusive_meters: SPATIAL_MIN_SEGMENT_LENGTH_METERS,
                });
            }

            let tangent = delta
                .try_normalize()
                .expect("a segment longer than the minimum has a non-zero direction");
            let projected_up_length = tangent.x().hypot(tangent.z());
            validate_projected_up_length(edge, segment_index, projected_up_length)?;

            // `left = Y × tangent`; `up = tangent × left`. This is algebraically
            // equivalent to projecting canonical +Y into the tangent's normal plane.
            let left = CanonicalVector3F32::try_new(tangent.z(), 0.0, -tangent.x())
                .expect("finite tangent produces a finite left vector")
                .try_normalize()
                .expect("projected-up threshold excludes a zero left vector");
            let up = CanonicalVector3F32::try_new(
                tangent.y() * left.z(),
                tangent.z() * left.x() - tangent.x() * left.z(),
                -tangent.y() * left.x(),
            )
            .expect("unit basis products remain finite")
            .try_normalize()
            .expect("orthogonal unit vectors produce a non-zero up vector");

            let next_cumulative =
                checked_accumulate_arc_length(edge, segment_index, cumulative, length)?;
            segments.push(PolylineSegment {
                length,
                cumulative_end: next_cumulative,
                tangent,
                up,
            });
            cumulative = next_cumulative;
        }

        validate_length_binding(edge, core_length, cumulative)?;

        Ok(Self {
            points: points.to_vec(),
            segments,
            core_length,
            arc_length: cumulative,
        })
    }

    pub(crate) fn first_point(&self) -> CanonicalPoint3F32 {
        self.points[0]
    }

    pub(crate) fn last_point(&self) -> CanonicalPoint3F32 {
        *self
            .points
            .last()
            .expect("a bound polyline always has at least two points")
    }

    pub(crate) fn sample(
        &self,
        edge: EdgeHandle,
        progress: EdgeProgress,
    ) -> Result<CanonicalPoseF32, SpatialError> {
        let progress_meters = progress.value();
        let core_length_meters = self.core_length.value();
        if progress_meters > core_length_meters {
            return Err(SpatialError::ProgressOutOfRange {
                edge,
                progress_meters,
                max_meters: core_length_meters,
            });
        }

        if progress_meters == 0.0 {
            return Ok(self.pose_at_point(0, 0));
        }
        if progress_meters == core_length_meters {
            let last_segment = self.segments.len() - 1;
            return Ok(self.pose_at_point(self.points.len() - 1, last_segment));
        }

        let geometry_s =
            ((progress_meters / core_length_meters) * f64::from(self.arc_length)) as f32;
        if geometry_s >= self.arc_length {
            let last_segment = self.segments.len() - 1;
            return Ok(self.pose_at_point(self.points.len() - 1, last_segment));
        }

        let segment_index = self
            .segments
            .partition_point(|segment| segment.cumulative_end <= geometry_s);
        let segment = &self.segments[segment_index];
        let cumulative_start = if segment_index == 0 {
            0.0
        } else {
            self.segments[segment_index - 1].cumulative_end
        };
        let segment_ratio = (geometry_s - cumulative_start) / segment.length;
        let start = self.points[segment_index];
        let end = self.points[segment_index + 1];
        let position = CanonicalPoint3F32::try_new(
            start.x() + (end.x() - start.x()) * segment_ratio,
            start.y() + (end.y() - start.y()) * segment_ratio,
            start.z() + (end.z() - start.z()) * segment_ratio,
        )
        .map_err(|source| SpatialError::SamplePositionComputation {
            edge,
            segment_index,
            source: Box::new(source),
        })?;

        Ok(CanonicalPoseF32 {
            position,
            tangent: segment.tangent,
            up: segment.up,
        })
    }

    fn pose_at_point(&self, point_index: usize, segment_index: usize) -> CanonicalPoseF32 {
        let segment = &self.segments[segment_index];
        CanonicalPoseF32 {
            position: self.points[point_index],
            tangent: segment.tangent,
            up: segment.up,
        }
    }
}

pub(crate) fn point_distance(a: CanonicalPoint3F32, b: CanonicalPoint3F32) -> f32 {
    (b.x() - a.x()).hypot(b.y() - a.y()).hypot(b.z() - a.z())
}

fn vector_length(vector: CanonicalVector3F32) -> f32 {
    vector.x().hypot(vector.y()).hypot(vector.z())
}

fn checked_accumulate_arc_length(
    edge: EdgeHandle,
    segment_index: usize,
    accumulated_meters: f32,
    segment_length_meters: f32,
) -> Result<f32, SpatialError> {
    let next = accumulated_meters + segment_length_meters;
    if !next.is_finite() || next <= accumulated_meters {
        return Err(SpatialError::ArcLengthAccumulationFailed {
            edge,
            segment_index,
            accumulated_meters,
            segment_length_meters,
        });
    }
    Ok(next)
}

fn validate_projected_up_length(
    edge: EdgeHandle,
    segment_index: usize,
    projected_up_length: f32,
) -> Result<(), SpatialError> {
    if projected_up_length < SPATIAL_MIN_PROJECTED_UP_LENGTH {
        return Err(SpatialError::DegenerateBasis {
            edge,
            segment_index,
            projected_up_length,
            min_inclusive: SPATIAL_MIN_PROJECTED_UP_LENGTH,
        });
    }
    Ok(())
}

fn validate_length_binding(
    edge: EdgeHandle,
    core_length: EdgeLength,
    geometry_arc_length: f32,
) -> Result<(), SpatialError> {
    let core_length_meters = core_length.value();
    let geometry_arc_length_meters = f64::from(geometry_arc_length);
    let difference_meters = (core_length_meters - geometry_arc_length_meters).abs();
    let tolerance_meters = SPATIAL_LENGTH_ABS_TOLERANCE_METERS
        .max(SPATIAL_LENGTH_REL_TOLERANCE * core_length_meters.max(geometry_arc_length_meters))
        + SPATIAL_CORE_LENGTH_QUANTIZATION_ALLOWANCE_METERS;

    if difference_meters > tolerance_meters {
        return Err(SpatialError::LengthMismatch {
            edge,
            core_length_meters,
            geometry_arc_length_meters: geometry_arc_length,
            difference_meters,
            tolerance_meters,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use laneflow_core::{EdgeLength, LaneEdge, LaneGraph};

    use super::*;

    #[test]
    fn accumulation_rejects_non_finite_and_precision_stall() {
        let graph = LaneGraph::try_new([LaneEdge::new(
            "A",
            EdgeLength::try_new(1.0).expect("valid length"),
            std::iter::empty::<&str>(),
        )])
        .expect("valid graph");
        let edge = graph.edge_handle("A").expect("edge A");
        assert!(matches!(
            checked_accumulate_arc_length(edge, 7, f32::MAX, f32::MAX),
            Err(SpatialError::ArcLengthAccumulationFailed {
                segment_index: 7,
                ..
            })
        ));
        assert!(matches!(
            checked_accumulate_arc_length(edge, 8, 4_194_304.0, 0.100_000_01),
            Err(SpatialError::ArcLengthAccumulationFailed {
                segment_index: 8,
                ..
            })
        ));
    }

    #[test]
    fn projected_up_threshold_is_inclusive() {
        let graph = LaneGraph::try_new([LaneEdge::new(
            "A",
            EdgeLength::try_new(1.0).expect("valid length"),
            std::iter::empty::<&str>(),
        )])
        .expect("valid graph");
        let edge = graph.edge_handle("A").expect("edge A");

        assert_eq!(
            validate_projected_up_length(edge, 0, SPATIAL_MIN_PROJECTED_UP_LENGTH),
            Ok(())
        );
        let below = f32::from_bits(SPATIAL_MIN_PROJECTED_UP_LENGTH.to_bits() - 1);
        assert!(matches!(
            validate_projected_up_length(edge, 1, below),
            Err(SpatialError::DegenerateBasis {
                segment_index: 1,
                projected_up_length,
                ..
            }) if projected_up_length == below
        ));
    }
}
