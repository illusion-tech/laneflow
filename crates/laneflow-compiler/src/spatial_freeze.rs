//! 共同 Spatial HIR 使用的规范点表冻结原语。
//!
//! Synthetic 显式 frame 几何与 RoadEditingSource 编译点表都必须经过这里。该模块只
//! 负责点/segment 数值冻结和最终方向谓词；frame 解析、路径推导和诊断排序仍由 HIR
//! 拥有，避免来源编码渗入共同几何内核。

use laneflow_static_contract::{
    CANONICAL_POINT_COMPONENT_MAX_METERS, CANONICAL_POINT_COMPONENT_MIN_METERS,
    SPATIAL_CORE_LENGTH_QUANTIZATION_ALLOWANCE_METERS, SPATIAL_LENGTH_ABS_TOLERANCE_METERS,
    SPATIAL_LENGTH_REL_TOLERANCE, SPATIAL_MIN_PROJECTED_UP_LENGTH,
    SPATIAL_MIN_SEGMENT_LENGTH_METERS,
};

use crate::declaration::CanonicalPoint3F32Input;
use crate::hir::{HirCanonicalPoint3F32, HirSpatialSegment};
use crate::{GeometryDirectionProfile, SpatialAxis, SpatialGeometryViolation};

pub(crate) struct FrozenSpatialPolyline {
    pub(crate) point_start: usize,
    pub(crate) point_count: usize,
    pub(crate) segment_start: usize,
    pub(crate) segment_count: usize,
    pub(crate) arc_length_meters: f32,
}

pub(crate) struct FrozenCanonicalPolyline {
    pub(crate) point_start: usize,
    pub(crate) point_count: usize,
    pub(crate) arc_length_meters: f32,
}

#[derive(Clone, Copy)]
pub(crate) struct SpatialDirectionCheck {
    pub(crate) accepted: bool,
    pub(crate) dot_bits: u64,
    pub(crate) lhs_bits: u64,
    pub(crate) rhs_bits: u64,
}

/// 把已经由官方前端构造的规范点列冻结为 HIR 点/segment 表。
///
/// 失败时恢复两个输出向量的进入前长度，因此调用方可以继续收集规范诊断而不会留下
/// 部分几何。所有算术顺序与原 Spatial HIR 实现保持一致。
pub(crate) fn freeze_spatial_polyline(
    input: &[CanonicalPoint3F32Input],
    expected_length_meters: f64,
    points: &mut Vec<HirCanonicalPoint3F32>,
    segments: &mut Vec<HirSpatialSegment>,
) -> Result<FrozenSpatialPolyline, SpatialGeometryViolation> {
    let point_start = points.len();
    let segment_start = segments.len();
    let result = freeze_polyline(input, expected_length_meters, points, Some(&mut *segments)).map(
        |frozen| FrozenSpatialPolyline {
            point_start: frozen.point_start,
            point_count: frozen.point_count,
            segment_start,
            segment_count: segments.len().saturating_sub(segment_start),
            arc_length_meters: frozen.arc_length_meters,
        },
    );
    if result.is_err() {
        points.truncate(point_start);
        segments.truncate(segment_start);
    }
    result
}

/// 冻结不可遍历几何的规范点并校验声明长度，不派生 Spatial segment 或交通长度。
pub(crate) fn freeze_canonical_polyline(
    input: &[CanonicalPoint3F32Input],
    expected_length_meters: f64,
    points: &mut Vec<HirCanonicalPoint3F32>,
) -> Result<FrozenCanonicalPolyline, SpatialGeometryViolation> {
    let point_start = points.len();
    let result = freeze_polyline(input, expected_length_meters, points, None);
    if result.is_err() {
        points.truncate(point_start);
    }
    result
}

fn freeze_polyline(
    input: &[CanonicalPoint3F32Input],
    expected_length_meters: f64,
    points: &mut Vec<HirCanonicalPoint3F32>,
    mut segments: Option<&mut Vec<HirSpatialSegment>>,
) -> Result<FrozenCanonicalPolyline, SpatialGeometryViolation> {
    if input.len() < 2 {
        return Err(SpatialGeometryViolation::InsufficientPoints {
            minimum: 2,
            actual: u32::try_from(input.len()).unwrap_or(u32::MAX),
        });
    }
    for (point_index, point) in input.iter().enumerate() {
        for (axis, value) in [
            (SpatialAxis::X, point.x),
            (SpatialAxis::Y, point.y),
            (SpatialAxis::Z, point.z),
        ] {
            let point_index = u32::try_from(point_index).unwrap_or(u32::MAX);
            if !value.is_finite() {
                return Err(SpatialGeometryViolation::NonFiniteCoordinate {
                    point_index,
                    axis,
                    value_bits: value.to_bits(),
                });
            }
            if !(CANONICAL_POINT_COMPONENT_MIN_METERS..=CANONICAL_POINT_COMPONENT_MAX_METERS)
                .contains(&value)
            {
                return Err(SpatialGeometryViolation::CoordinateOutOfRange {
                    point_index,
                    axis,
                    value_bits: value.to_bits(),
                    minimum_bits: CANONICAL_POINT_COMPONENT_MIN_METERS.to_bits(),
                    maximum_bits: CANONICAL_POINT_COMPONENT_MAX_METERS.to_bits(),
                });
            }
        }
    }

    let point_start = points.len();
    points.extend(input.iter().map(|point| HirCanonicalPoint3F32 {
        x: point.x,
        y: point.y,
        z: point.z,
    }));
    let mut cumulative = 0.0_f32;
    for (segment_index, pair) in input.windows(2).enumerate() {
        let delta = [
            canonicalize_spatial_zero(pair[1].x - pair[0].x),
            canonicalize_spatial_zero(pair[1].y - pair[0].y),
            canonicalize_spatial_zero(pair[1].z - pair[0].z),
        ];
        let length = delta[0].hypot(delta[1]).hypot(delta[2]);
        if length <= SPATIAL_MIN_SEGMENT_LENGTH_METERS {
            return Err(SpatialGeometryViolation::DegenerateSegment {
                segment_index: u32::try_from(segment_index).unwrap_or(u32::MAX),
                length_bits: length.to_bits(),
                minimum_bits: SPATIAL_MIN_SEGMENT_LENGTH_METERS.to_bits(),
            });
        }
        let next_cumulative = cumulative + length;
        if !next_cumulative.is_finite() || next_cumulative <= cumulative {
            return Err(SpatialGeometryViolation::ArcLengthAccumulationFailed {
                segment_index: u32::try_from(segment_index).unwrap_or(u32::MAX),
                accumulated_bits: cumulative.to_bits(),
                segment_length_bits: length.to_bits(),
            });
        }
        if let Some(segments) = segments.as_deref_mut() {
            let tangent = normalize_spatial_vector(delta);
            let projected_up = tangent[0].hypot(tangent[2]);
            if projected_up < SPATIAL_MIN_PROJECTED_UP_LENGTH {
                return Err(SpatialGeometryViolation::DegenerateProjectedUp {
                    segment_index: u32::try_from(segment_index).unwrap_or(u32::MAX),
                    projected_up_bits: projected_up.to_bits(),
                    minimum_bits: SPATIAL_MIN_PROJECTED_UP_LENGTH.to_bits(),
                });
            }
            let left = normalize_spatial_vector([tangent[2], 0.0, -tangent[0]]);
            let raw_up = [
                tangent[1] * left[2],
                tangent[2] * left[0] - tangent[0] * left[2],
                -tangent[1] * left[0],
            ];
            let up = normalize_spatial_vector(raw_up);
            segments.push(HirSpatialSegment {
                length_meters: length,
                cumulative_end_meters: next_cumulative,
                tangent,
                up,
            });
        }
        cumulative = next_cumulative;
    }

    let tolerance = SPATIAL_LENGTH_ABS_TOLERANCE_METERS
        .max(SPATIAL_LENGTH_REL_TOLERANCE * expected_length_meters.max(f64::from(cumulative)))
        + SPATIAL_CORE_LENGTH_QUANTIZATION_ALLOWANCE_METERS;
    if (expected_length_meters - f64::from(cumulative)).abs() > tolerance {
        return Err(SpatialGeometryViolation::LengthMismatch {
            expected_length_bits: expected_length_meters.to_bits(),
            geometry_length_bits: cumulative.to_bits(),
            tolerance_bits: tolerance.to_bits(),
        });
    }

    Ok(FrozenCanonicalPolyline {
        point_start,
        point_count: points.len().saturating_sub(point_start),
        arc_length_meters: cumulative,
    })
}

/// 使用 ADR 0022 的固定逐运算图检查两条最终 `f32` 弦的方向跳变。
pub(crate) fn check_spatial_direction(
    left: [f64; 3],
    right: [f64; 3],
    profile: GeometryDirectionProfile,
) -> SpatialDirectionCheck {
    let x = left[0] * right[0];
    let y = left[1] * right[1];
    let xy = x + y;
    let z = left[2] * right[2];
    let dot = xy + z;

    let left_x = left[0] * left[0];
    let left_y = left[1] * left[1];
    let left_xy = left_x + left_y;
    let left_z = left[2] * left[2];
    let left_norm = left_xy + left_z;

    let right_x = right[0] * right[0];
    let right_y = right[1] * right[1];
    let right_xy = right_x + right_y;
    let right_z = right[2] * right[2];
    let right_norm = right_xy + right_z;

    let lhs = dot * dot;
    let weighted_left_norm = profile.full_angle_cosine_squared() * left_norm;
    let rhs = weighted_left_norm * right_norm;
    SpatialDirectionCheck {
        accepted: dot > 0.0 && lhs >= rhs,
        dot_bits: dot.to_bits(),
        lhs_bits: lhs.to_bits(),
        rhs_bits: rhs.to_bits(),
    }
}

fn normalize_spatial_vector(vector: [f32; 3]) -> [f32; 3] {
    let scale = vector[0].abs().max(vector[1].abs()).max(vector[2].abs());
    debug_assert!(scale > 0.0, "validated spatial direction must be non-zero");
    let scaled = [vector[0] / scale, vector[1] / scale, vector[2] / scale];
    let scaled_length = scaled[0].hypot(scaled[1]).hypot(scaled[2]);
    [
        canonicalize_spatial_zero(scaled[0] / scaled_length),
        canonicalize_spatial_zero(scaled[1] / scaled_length),
        canonicalize_spatial_zero(scaled[2] / scaled_length),
    ]
}

const fn canonicalize_spatial_zero(value: f32) -> f32 {
    if value == 0.0 { 0.0 } else { value }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_polyline_failure_restores_the_existing_point_table() {
        let sentinel = HirCanonicalPoint3F32 {
            x: 7.0,
            y: 8.0,
            z: 9.0,
        };
        let mut points = vec![sentinel];
        let input = [
            CanonicalPoint3F32Input {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            CanonicalPoint3F32Input {
                x: 10.0,
                y: 0.0,
                z: 0.0,
            },
        ];

        assert!(matches!(
            freeze_canonical_polyline(&input, 11.0, &mut points),
            Err(SpatialGeometryViolation::LengthMismatch { .. })
        ));
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].x.to_bits(), sentinel.x.to_bits());
        assert_eq!(points[0].y.to_bits(), sentinel.y.to_bits());
        assert_eq!(points[0].z.to_bits(), sentinel.z.to_bits());
    }

    #[test]
    fn direction_check_uses_the_selected_full_angle_profile() {
        let one_degree = [0.999_847_7_f32, 0.0, 0.017_452_406_f32];
        assert!(
            check_spatial_direction(
                [1.0, 0.0, 0.0],
                one_degree.map(f64::from),
                GeometryDirectionProfile::Smooth1Deg,
            )
            .accepted
        );
        assert!(
            !check_spatial_direction(
                [1.0, 0.0, 0.0],
                [0.999_390_84, 0.0, 0.034_899_496],
                GeometryDirectionProfile::Smooth1Deg,
            )
            .accepted
        );
        assert!(
            check_spatial_direction(
                [1.0, 0.0, 0.0],
                [0.999_390_84, 0.0, 0.034_899_496],
                GeometryDirectionProfile::Balanced2Deg,
            )
            .accepted
        );
    }
}
