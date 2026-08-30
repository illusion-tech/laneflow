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

use core::cmp::Ordering;

use crate::declaration::{AuthoringPoint2F64, CanonicalPoint3F32Input};
use crate::hir::{HirCanonicalPoint2F32, HirCanonicalPoint3F32, HirSpatialSegment};
use crate::{
    ConflictZoneRegionViolation, GeometryDirectionProfile, SpatialAxis, SpatialGeometryViolation,
};

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

pub(crate) struct FrozenConflictZoneRegion {
    pub(crate) point_start: usize,
    pub(crate) point_count: usize,
    pub(crate) min_y: f32,
    pub(crate) max_y: f32,
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

/// 把 LFRE binary64 编制区域冻结为唯一规范 binary32 ring。
///
/// 量化后拒绝重复点、零面积与自交；有效 ring 统一为从 `+Y` 观察逆时针，并旋转到
/// 唯一词典序最小点开头。全部拓扑谓词只对 binary32 exact value 做扩展求和，不依赖
/// 平台 epsilon 或几何库。
pub(crate) fn freeze_conflict_zone_region(
    min_y: f64,
    max_y: f64,
    input: &[AuthoringPoint2F64],
    output: &mut Vec<HirCanonicalPoint2F32>,
) -> Result<FrozenConflictZoneRegion, SpatialGeometryViolation> {
    if input.len() < 3 {
        return Err(SpatialGeometryViolation::InsufficientPoints {
            minimum: 3,
            actual: u32::try_from(input.len()).unwrap_or(u32::MAX),
        });
    }
    let min_y = quantize_region_component(min_y, u32::MAX, SpatialAxis::Y)?;
    let max_y = quantize_region_component(max_y, u32::MAX, SpatialAxis::Y)?;
    if min_y >= max_y {
        return Err(SpatialGeometryViolation::InvalidConflictZoneRegion(
            ConflictZoneRegionViolation::QuantizedHeightOrder {
                min_y_bits: min_y.to_bits(),
                max_y_bits: max_y.to_bits(),
            },
        ));
    }

    let mut points = Vec::with_capacity(input.len());
    for (index, point) in input.iter().enumerate() {
        let point_index = u32::try_from(index).unwrap_or(u32::MAX);
        points.push(HirCanonicalPoint2F32 {
            x: quantize_region_component(point.x, point_index, SpatialAxis::X)?,
            z: quantize_region_component(point.z, point_index, SpatialAxis::Z)?,
        });
    }

    let mut ordered = points.iter().copied().enumerate().collect::<Vec<_>>();
    ordered.sort_unstable_by(|left, right| lexicographic_point_order(left.1, right.1));
    for pair in ordered.windows(2) {
        if same_point(pair[0].1, pair[1].1) {
            return Err(SpatialGeometryViolation::InvalidConflictZoneRegion(
                ConflictZoneRegionViolation::DuplicateQuantizedPoint {
                    first_index: u32::try_from(pair[0].0).unwrap_or(u32::MAX),
                    duplicate_index: u32::try_from(pair[1].0).unwrap_or(u32::MAX),
                },
            ));
        }
    }

    match polygon_area_sign(&points) {
        Ordering::Equal => {
            return Err(SpatialGeometryViolation::InvalidConflictZoneRegion(
                ConflictZoneRegionViolation::NonPositiveArea,
            ));
        }
        Ordering::Less => points.reverse(),
        Ordering::Greater => {}
    }
    let first = points
        .iter()
        .enumerate()
        .min_by(|left, right| lexicographic_point_order(*left.1, *right.1))
        .map(|(index, _)| index)
        .expect("three-point region has a first point");
    points.rotate_left(first);
    validate_simple_ring(&points)?;

    let point_start = output.len();
    output.extend_from_slice(&points);
    Ok(FrozenConflictZoneRegion {
        point_start,
        point_count: points.len(),
        min_y,
        max_y,
    })
}

fn quantize_region_component(
    value: f64,
    point_index: u32,
    axis: SpatialAxis,
) -> Result<f32, SpatialGeometryViolation> {
    if !value.is_finite() {
        return Err(SpatialGeometryViolation::InvalidConflictZoneRegion(
            ConflictZoneRegionViolation::NonFiniteAuthoringCoordinate {
                point_index,
                axis,
                value_bits: value.to_bits(),
            },
        ));
    }
    let minimum = f64::from(CANONICAL_POINT_COMPONENT_MIN_METERS);
    let maximum = f64::from(CANONICAL_POINT_COMPONENT_MAX_METERS);
    if !(minimum..=maximum).contains(&value) {
        return Err(SpatialGeometryViolation::InvalidConflictZoneRegion(
            ConflictZoneRegionViolation::AuthoringCoordinateOutOfRange {
                point_index,
                axis,
                value_bits: value.to_bits(),
            },
        ));
    }
    Ok(canonicalize_spatial_zero(value as f32))
}

fn lexicographic_point_order(
    left: HirCanonicalPoint2F32,
    right: HirCanonicalPoint2F32,
) -> Ordering {
    left.x
        .total_cmp(&right.x)
        .then_with(|| left.z.total_cmp(&right.z))
}

fn same_point(left: HirCanonicalPoint2F32, right: HirCanonicalPoint2F32) -> bool {
    left.x.to_bits() == right.x.to_bits() && left.z.to_bits() == right.z.to_bits()
}

fn polygon_area_sign(points: &[HirCanonicalPoint2F32]) -> Ordering {
    let mut expansion = Vec::new();
    let mut scratch = Vec::new();
    for index in 0..points.len() {
        let left = points[index];
        let right = points[(index + 1) % points.len()];
        add_expansion_term(
            &mut expansion,
            &mut scratch,
            f64::from(left.x) * f64::from(right.z),
        );
        add_expansion_term(
            &mut expansion,
            &mut scratch,
            -(f64::from(left.z) * f64::from(right.x)),
        );
    }
    expansion_sign(&expansion)
}

fn orientation(
    first: HirCanonicalPoint2F32,
    second: HirCanonicalPoint2F32,
    third: HirCanonicalPoint2F32,
) -> Ordering {
    let mut expansion = Vec::new();
    let mut scratch = Vec::new();
    for term in [
        f64::from(first.x) * f64::from(second.z),
        f64::from(second.x) * f64::from(third.z),
        f64::from(third.x) * f64::from(first.z),
        -(f64::from(first.z) * f64::from(second.x)),
        -(f64::from(second.z) * f64::from(third.x)),
        -(f64::from(third.z) * f64::from(first.x)),
    ] {
        add_expansion_term(&mut expansion, &mut scratch, term);
    }
    expansion_sign(&expansion)
}

fn add_expansion_term(expansion: &mut Vec<f64>, scratch: &mut Vec<f64>, term: f64) {
    if term == 0.0 {
        return;
    }
    scratch.clear();
    let mut accumulator = term;
    for &component in expansion.iter() {
        let (sum, error) = two_sum(accumulator, component);
        if error != 0.0 {
            scratch.push(error);
        }
        accumulator = sum;
    }
    if accumulator != 0.0 || scratch.is_empty() {
        scratch.push(accumulator);
    }
    core::mem::swap(expansion, scratch);
}

fn two_sum(left: f64, right: f64) -> (f64, f64) {
    let sum = left + right;
    let right_virtual = sum - left;
    let left_virtual = sum - right_virtual;
    let right_roundoff = right - right_virtual;
    let left_roundoff = left - left_virtual;
    (sum, left_roundoff + right_roundoff)
}

fn expansion_sign(expansion: &[f64]) -> Ordering {
    expansion
        .iter()
        .rev()
        .copied()
        .find(|value| *value != 0.0)
        .map_or(Ordering::Equal, |value| {
            if value.is_sign_negative() {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        })
}

fn validate_simple_ring(points: &[HirCanonicalPoint2F32]) -> Result<(), SpatialGeometryViolation> {
    let count = points.len();
    for vertex in 0..count {
        let previous = points[(vertex + count - 1) % count];
        let current = points[vertex];
        let next = points[(vertex + 1) % count];
        if orientation(previous, current, next) == Ordering::Equal
            && !strictly_between(current, previous, next)
        {
            return Err(SpatialGeometryViolation::InvalidConflictZoneRegion(
                ConflictZoneRegionViolation::SelfIntersection {
                    first_edge: u32::try_from((vertex + count - 1) % count).unwrap_or(u32::MAX),
                    second_edge: u32::try_from(vertex).unwrap_or(u32::MAX),
                },
            ));
        }
    }
    for first_edge in 0..count {
        for second_edge in (first_edge + 1)..count {
            if second_edge == first_edge + 1 || (first_edge == 0 && second_edge == count - 1) {
                continue;
            }
            if segments_intersect(
                points[first_edge],
                points[(first_edge + 1) % count],
                points[second_edge],
                points[(second_edge + 1) % count],
            ) {
                return Err(SpatialGeometryViolation::InvalidConflictZoneRegion(
                    ConflictZoneRegionViolation::SelfIntersection {
                        first_edge: u32::try_from(first_edge).unwrap_or(u32::MAX),
                        second_edge: u32::try_from(second_edge).unwrap_or(u32::MAX),
                    },
                ));
            }
        }
    }
    Ok(())
}

fn strictly_between(
    point: HirCanonicalPoint2F32,
    first: HirCanonicalPoint2F32,
    second: HirCanonicalPoint2F32,
) -> bool {
    if first.x != second.x {
        first.x.min(second.x) < point.x && point.x < first.x.max(second.x)
    } else {
        first.z.min(second.z) < point.z && point.z < first.z.max(second.z)
    }
}

fn segments_intersect(
    first_start: HirCanonicalPoint2F32,
    first_end: HirCanonicalPoint2F32,
    second_start: HirCanonicalPoint2F32,
    second_end: HirCanonicalPoint2F32,
) -> bool {
    let first_second_start = orientation(first_start, first_end, second_start);
    let first_second_end = orientation(first_start, first_end, second_end);
    let second_first_start = orientation(second_start, second_end, first_start);
    let second_first_end = orientation(second_start, second_end, first_end);
    if first_second_start == Ordering::Equal
        && point_on_segment(second_start, first_start, first_end)
        || first_second_end == Ordering::Equal
            && point_on_segment(second_end, first_start, first_end)
        || second_first_start == Ordering::Equal
            && point_on_segment(first_start, second_start, second_end)
        || second_first_end == Ordering::Equal
            && point_on_segment(first_end, second_start, second_end)
    {
        return true;
    }
    first_second_start != first_second_end && second_first_start != second_first_end
}

fn point_on_segment(
    point: HirCanonicalPoint2F32,
    start: HirCanonicalPoint2F32,
    end: HirCanonicalPoint2F32,
) -> bool {
    start.x.min(end.x) <= point.x
        && point.x <= start.x.max(end.x)
        && start.z.min(end.z) <= point.z
        && point.z <= start.z.max(end.z)
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
    let left = scale_spatial_direction(left);
    let right = scale_spatial_direction(right);
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

fn scale_spatial_direction(vector: [f64; 3]) -> [f64; 3] {
    let scale = vector[0].abs().max(vector[1].abs()).max(vector[2].abs());
    debug_assert!(
        scale.is_finite() && scale > 0.0,
        "validated spatial chord must have a finite non-zero scale"
    );
    [vector[0] / scale, vector[1] / scale, vector[2] / scale]
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

    #[test]
    fn direction_check_scales_each_chord_before_the_frozen_operation_graph() {
        let check = check_spatial_direction(
            [2.0, 1.0, 0.0],
            [4.0, 1.0, 0.0],
            GeometryDirectionProfile::Compact5Deg,
        );

        assert_eq!(f64::from_bits(check.dot_bits), 1.125);
        assert_eq!(f64::from_bits(check.lhs_bits), 1.265_625);
        assert_ne!(f64::from_bits(check.dot_bits), 9.0);
    }
}
