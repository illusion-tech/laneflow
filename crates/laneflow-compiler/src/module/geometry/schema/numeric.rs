//! Geometry v1 reference curve 的确定性 `f64` numeric freeze。

use std::collections::BTreeMap;

use super::road::{ParsedCurve, ParsedCurveSegment, ParsedVec3, RawNumber};
use super::{ByteSpan, ParsedGeometryDocument};
use crate::module::geometry::json::StageScratchMeter;
use crate::module::geometry::{GeometryAccuracyProfile, GeometryDirectionProfile};
use laneflow_static_contract::{
    CANONICAL_POINT_COMPONENT_MAX_METERS, CANONICAL_POINT_COMPONENT_MIN_METERS,
    SPATIAL_MIN_PROJECTED_UP_LENGTH, SPATIAL_MIN_SEGMENT_LENGTH_METERS,
};

const MAX_SUBDIVISION_DEPTH: u8 = 20;

/// 两个显式细分栈的栈帧：当前候选控制点、参数区间与深度。
type SubdivisionFrame = ([Point3; 4], f64, f64, u8);

/// 契约 §7.1：按输入规模的候选容量先以 checked u64 计算（计数或乘法溢出饱和到
/// `u64::MAX`，必然超过任何实际上限），再交给共享账簿在增长前比较上限。
fn scaled_bytes(count: usize, element_bytes: u64) -> u64 {
    u64::try_from(count)
        .unwrap_or(u64::MAX)
        .saturating_mul(element_bytes)
}

/// `StageScratchBytes` 超限的统一 freeze 错误；字节细节由 builder 映射为资源上限诊断。
const fn scratch_exceeded(span: ByteSpan) -> NumericFreezeError {
    NumericFreezeError {
        violation: NumericFreezeViolation::StageScratchExceeded,
        field: "stageScratchBytes",
        span,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Point3 {
    x: f64,
    y: f64,
    z: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::module::geometry) struct FrozenReferenceSample {
    pub(in crate::module::geometry) segment_index: u32,
    pub(in crate::module::geometry) parameter: f64,
    pub(in crate::module::geometry) point: [f64; 3],
}

#[derive(Debug)]
pub(in crate::module::geometry) struct FrozenRoadReference {
    pub(in crate::module::geometry) road_key: Box<str>,
    pub(in crate::module::geometry) samples: Box<[FrozenReferenceSample]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::module::geometry) struct StationInterval {
    pub(in crate::module::geometry) segment_index: u32,
    pub(in crate::module::geometry) t0_bits: u64,
    pub(in crate::module::geometry) t1_bits: u64,
    pub(in crate::module::geometry) cumulative_start_length_bits: u64,
    pub(in crate::module::geometry) cumulative_end_length_bits: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::module::geometry) struct FrozenCurveParameter {
    pub(in crate::module::geometry) segment_index: u32,
    pub(in crate::module::geometry) parameter: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::module::geometry) struct FrozenSpanStations {
    pub(in crate::module::geometry) start: FrozenCurveParameter,
    pub(in crate::module::geometry) end: FrozenCurveParameter,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct FrozenRoadStationing {
    pub(in crate::module::geometry) road_key: Box<str>,
    pub(in crate::module::geometry) intervals: Box<[StationInterval]>,
    pub(in crate::module::geometry) spans: Box<[FrozenSpanStations]>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::module::geometry) struct FrozenOffsetSample {
    pub(in crate::module::geometry) segment_index: u32,
    pub(in crate::module::geometry) parameter: f64,
    pub(in crate::module::geometry) point: [f64; 3],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LateralIntentKind {
    ForwardLane,
    BackwardLane,
    FacilityBand,
}

#[derive(Debug, PartialEq)]
pub(in crate::module::geometry) struct FrozenLateralIntent {
    pub(in crate::module::geometry) key: Box<str>,
    pub(in crate::module::geometry) kind: LateralIntentKind,
    pub(in crate::module::geometry) center_offset_meters: f64,
    pub(in crate::module::geometry) left_boundary_offset_meters: f64,
    pub(in crate::module::geometry) right_boundary_offset_meters: f64,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct FrozenCrossSectionLayout {
    pub(in crate::module::geometry) span_key: Box<str>,
    pub(in crate::module::geometry) items: Box<[FrozenLateralIntent]>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FrozenCanonicalPoint {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) z: f32,
}

#[derive(Debug, PartialEq)]
pub(crate) struct FrozenLateralCurve {
    pub(crate) span_key: Box<str>,
    pub(crate) key: Box<str>,
    pub(crate) kind: LateralIntentKind,
    pub(crate) points: Box<[FrozenCanonicalPoint]>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct FrozenInternalEdgeCurve {
    pub(crate) junction_key: Box<str>,
    pub(crate) lane_edge_key: Box<str>,
    pub(crate) points: Box<[FrozenCanonicalPoint]>,
}

#[derive(Debug)]
pub(crate) struct FrozenGeometryPayload {
    pub(crate) lateral_curves: Box<[FrozenLateralCurve]>,
    pub(crate) internal_edge_curves: Box<[FrozenInternalEdgeCurve]>,
    pub(crate) geometry_point_count: u64,
    pub(crate) offset_curve_distribution: Box<[FrozenOffsetCurveBucket]>,
}

impl FrozenGeometryPayload {
    /// payload 自有 heap allocation 的确定性逻辑字节数。除曲线/点数组外，显式
    /// 计入每个 `Box<str>` 键和 offset 分布桶，避免字符串副本从受控账本中消失。
    pub(crate) fn controlled_live_bytes(&self) -> u64 {
        let mut total = scaled_bytes(
            self.lateral_curves.len(),
            size_of::<FrozenLateralCurve>() as u64,
        )
        .saturating_add(scaled_bytes(
            self.internal_edge_curves.len(),
            size_of::<FrozenInternalEdgeCurve>() as u64,
        ))
        .saturating_add(scaled_bytes(
            self.offset_curve_distribution.len(),
            size_of::<FrozenOffsetCurveBucket>() as u64,
        ));
        for curve in &self.lateral_curves {
            total = total
                .saturating_add(u64::try_from(curve.span_key.len()).unwrap_or(u64::MAX))
                .saturating_add(u64::try_from(curve.key.len()).unwrap_or(u64::MAX))
                .saturating_add(scaled_bytes(
                    curve.points.len(),
                    size_of::<FrozenCanonicalPoint>() as u64,
                ));
        }
        for curve in &self.internal_edge_curves {
            total = total
                .saturating_add(u64::try_from(curve.junction_key.len()).unwrap_or(u64::MAX))
                .saturating_add(u64::try_from(curve.lane_edge_key.len()).unwrap_or(u64::MAX))
                .saturating_add(scaled_bytes(
                    curve.points.len(),
                    size_of::<FrozenCanonicalPoint>() as u64,
                ));
        }
        total
    }
}

/// 横向 offset 曲线按 |中心偏移| f64 位模式分组的冻结分布桶（§9.2 前端计数）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FrozenOffsetCurveBucket {
    pub(crate) absolute_offset_meters_bits: u64,
    pub(crate) curve_count: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::module::geometry) enum NumericFreezeViolation {
    InvalidNumber,
    NonFiniteNumber,
    DegenerateTangent,
    DiscontinuousSourceJoin,
    SubdivisionDepthExceeded,
    ArithmeticOverflow,
    InvalidStation,
    IncompleteStationCoverage,
    InvalidCrossSectionReference,
    InvalidWidth,
    MixedLaneDirection,
    CoordinateOutOfRange,
    QuantizedSegmentTooShort,
    DegenerateProjectedUp,
    FinalDirectionExceeded,
    TotalPositionErrorExceeded,
    GeometryPointLimitExceeded,
    StageScratchExceeded,
}

#[derive(Debug)]
pub(in crate::module::geometry) struct NumericFreezeError {
    pub(in crate::module::geometry) violation: NumericFreezeViolation,
    pub(in crate::module::geometry) field: &'static str,
    pub(in crate::module::geometry) span: ByteSpan,
}

pub(super) fn freeze_reference_lines(
    document: &ParsedGeometryDocument,
    accuracy_profile: GeometryAccuracyProfile,
    direction_profile: GeometryDirectionProfile,
    meter: &mut StageScratchMeter,
) -> Result<Box<[FrozenRoadReference]>, NumericFreezeError> {
    document
        .roads
        .iter()
        .map(|road| {
            Ok(FrozenRoadReference {
                road_key: road.road_key.value.clone(),
                samples: freeze_curve(
                    &road.reference_line,
                    accuracy_profile,
                    direction_profile,
                    direction_profile,
                    meter,
                )?,
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Vec::into_boxed_slice)
}

/// Station 表按契约 §6.1 固定 `Fine2Cm`/`Smooth1Deg` 细分，与调用方配置档无关；
/// 来源 join 的方向门禁仍按 §6.1 使用调用方所选方向档。
pub(super) fn freeze_stationing(
    document: &ParsedGeometryDocument,
    direction_profile: GeometryDirectionProfile,
    meter: &mut StageScratchMeter,
) -> Result<Box<[FrozenRoadStationing]>, NumericFreezeError> {
    document
        .roads
        .iter()
        .map(|road| {
            let samples = freeze_curve(
                &road.reference_line,
                GeometryAccuracyProfile::Fine2Cm,
                GeometryDirectionProfile::Smooth1Deg,
                direction_profile,
                meter,
            )?;
            let intervals = build_station_intervals(&samples, road.reference_line.span, meter)?;
            let spans = freeze_span_stations(&road.cross_section_spans, &intervals, meter)?;
            // samples 已被 intervals/spans 消费完，归还全部暂存字节。
            meter.shrink(scaled_bytes(
                samples.len(),
                size_of::<FrozenReferenceSample>() as u64,
            ));
            Ok(FrozenRoadStationing {
                road_key: road.road_key.value.clone(),
                intervals,
                spans,
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Vec::into_boxed_slice)
}

pub(super) fn freeze_cross_section_layouts(
    document: &ParsedGeometryDocument,
    meter: &mut StageScratchMeter,
) -> Result<Box<[FrozenCrossSectionLayout]>, NumericFreezeError> {
    let mut output = Vec::new();
    for road in &document.roads {
        for span in &road.cross_section_spans {
            let reference_section_key = span.reference_section_key.value.as_ref();
            let reference_lane_key = span.reference_lane_key.value.as_ref();
            let mut pending = Vec::new();
            let mut pending_scratch_bytes = 0_u64;
            let mut reference_index = None;
            let mut reference_section_was_element = false;
            for element in &span.elements {
                match element {
                    super::road::ParsedCorridorElement::RoadSection {
                        section_key,
                        span: element_span,
                    } => {
                        let section_key = section_key.value.as_ref();
                        let Some(section) = span
                            .road_sections
                            .iter()
                            .find(|section| section.section_key.value.as_ref() == section_key)
                        else {
                            return Err(NumericFreezeError {
                                violation: NumericFreezeViolation::InvalidCrossSectionReference,
                                field: "elements.sectionKey",
                                span: *element_span,
                            });
                        };
                        if section.section_key.value.as_ref() == reference_section_key {
                            reference_section_was_element = true;
                        }
                        let expected_direction = section.lanes[0].direction;
                        for lane in &section.lanes {
                            if lane.direction != expected_direction {
                                return Err(NumericFreezeError {
                                    violation: NumericFreezeViolation::MixedLaneDirection,
                                    field: "lanes.direction",
                                    span: lane.span,
                                });
                            }
                            let width = parse_positive(&lane.width_meters, "widthMeters")?;
                            if section.section_key.value.as_ref() == reference_section_key
                                && lane.lane_key.value.as_ref() == reference_lane_key
                            {
                                if lane.direction != super::road::ParsedLaneDirection::Forward {
                                    return Err(NumericFreezeError {
                                        violation:
                                            NumericFreezeViolation::InvalidCrossSectionReference,
                                        field: "referenceLaneKey",
                                        span: span.reference_lane_key.span,
                                    });
                                }
                                reference_index = Some(pending.len());
                            }
                            meter
                                .grow(size_of::<PendingLateralIntent>() as u64)
                                .map_err(|_| scratch_exceeded(span.span))?;
                            pending_scratch_bytes += size_of::<PendingLateralIntent>() as u64;
                            pending.push(PendingLateralIntent {
                                key: lane.lane_key.value.clone(),
                                kind: match lane.direction {
                                    super::road::ParsedLaneDirection::Forward => {
                                        LateralIntentKind::ForwardLane
                                    }
                                    super::road::ParsedLaneDirection::Backward => {
                                        LateralIntentKind::BackwardLane
                                    }
                                },
                                width,
                            });
                        }
                    }
                    super::road::ParsedCorridorElement::FacilityBand {
                        facility_band_key,
                        span: element_span,
                    } => {
                        let facility_band_key = facility_band_key.value.as_ref();
                        let Some(facility) = span.facility_bands.iter().find(|facility| {
                            facility.facility_band_key.value.as_ref() == facility_band_key
                        }) else {
                            return Err(NumericFreezeError {
                                violation: NumericFreezeViolation::InvalidCrossSectionReference,
                                field: "elements.facilityBandKey",
                                span: *element_span,
                            });
                        };
                        meter
                            .grow(size_of::<PendingLateralIntent>() as u64)
                            .map_err(|_| scratch_exceeded(span.span))?;
                        pending_scratch_bytes += size_of::<PendingLateralIntent>() as u64;
                        pending.push(PendingLateralIntent {
                            key: facility.facility_band_key.value.clone(),
                            kind: LateralIntentKind::FacilityBand,
                            width: parse_positive(&facility.width_meters, "widthMeters")?,
                        });
                    }
                }
            }
            let Some(reference_index) = reference_index else {
                return Err(NumericFreezeError {
                    violation: NumericFreezeViolation::InvalidCrossSectionReference,
                    field: if reference_section_was_element {
                        "referenceLaneKey"
                    } else {
                        "referenceSectionKey"
                    },
                    span: if reference_section_was_element {
                        span.reference_lane_key.span
                    } else {
                        span.reference_section_key.span
                    },
                });
            };
            let items = compute_lateral_offsets(pending, reference_index, span.span, meter)?;
            // pending 已交给 compute_lateral_offsets 消费，归还整份暂存字节。
            meter.shrink(pending_scratch_bytes);
            meter
                .grow(size_of::<FrozenCrossSectionLayout>() as u64)
                .map_err(|_| scratch_exceeded(span.span))?;
            output.push(FrozenCrossSectionLayout {
                span_key: span.span_key.value.clone(),
                items,
            });
        }
    }
    Ok(output.into_boxed_slice())
}

fn normalize_owner_local_reference(
    value: &mut super::SpannedString,
    namespace: &str,
    field: &'static str,
    meter: &mut StageScratchMeter,
) -> Result<(), NumericFreezeError> {
    let (explicit_namespace, key) = super::split_reference_spelling(&value.value);
    match explicit_namespace {
        Some(explicit_namespace) if explicit_namespace != namespace => {
            return Err(NumericFreezeError {
                violation: NumericFreezeViolation::InvalidCrossSectionReference,
                field,
                span: value.span,
            });
        }
        None => return Ok(()),
        Some(_) => {}
    }

    // 新 key 分配与旧 self-qualified token 在替换瞬间同时存活；把这段短暂重叠
    // 计入 StageScratchBytes。替换后新 key 已属于解析树，不继续占用阶段暂存。
    let normalized_bytes = u64::try_from(key.len()).unwrap_or(u64::MAX);
    meter
        .grow(normalized_bytes)
        .map_err(|_| scratch_exceeded(value.span))?;
    let normalized: Box<str> = key.into();
    value.value = normalized;
    meter.shrink(normalized_bytes);
    Ok(())
}

/// 把只允许指向 owning module 的 cross-section 引用恰好归一化一次。
/// numeric freeze 与后续 lowering 随后消费同一份 owner-local key，不再各自解释拼写。
fn normalize_owner_local_cross_section_references(
    document: &mut ParsedGeometryDocument,
    meter: &mut StageScratchMeter,
) -> Result<(), NumericFreezeError> {
    let namespace = document.module.namespace.value.as_ref();
    for road in &mut document.roads {
        for span in &mut road.cross_section_spans {
            normalize_owner_local_reference(
                &mut span.reference_section_key,
                namespace,
                "referenceSectionKey",
                meter,
            )?;
            normalize_owner_local_reference(
                &mut span.reference_lane_key,
                namespace,
                "referenceLaneKey",
                meter,
            )?;
            for element in &mut span.elements {
                match element {
                    super::road::ParsedCorridorElement::RoadSection { section_key, .. } => {
                        normalize_owner_local_reference(
                            section_key,
                            namespace,
                            "elements.sectionKey",
                            meter,
                        )?;
                    }
                    super::road::ParsedCorridorElement::FacilityBand {
                        facility_band_key, ..
                    } => {
                        normalize_owner_local_reference(
                            facility_band_key,
                            namespace,
                            "elements.facilityBandKey",
                            meter,
                        )?;
                    }
                }
            }
        }
    }
    Ok(())
}

pub(super) fn freeze_lateral_curves(
    document: &ParsedGeometryDocument,
    stationing: &[FrozenRoadStationing],
    layouts: &[FrozenCrossSectionLayout],
    accuracy_profile: GeometryAccuracyProfile,
    direction_profile: GeometryDirectionProfile,
    meter: &mut StageScratchMeter,
) -> Result<Box<[FrozenLateralCurve]>, NumericFreezeError> {
    freeze_lateral_curves_with_limit(
        document,
        stationing,
        layouts,
        accuracy_profile,
        direction_profile,
        u64::MAX,
        meter,
    )
}

pub(super) fn freeze_geometry_payload(
    document: &mut ParsedGeometryDocument,
    accuracy_profile: GeometryAccuracyProfile,
    direction_profile: GeometryDirectionProfile,
    geometry_point_limit: u64,
    meter: &mut StageScratchMeter,
) -> Result<FrozenGeometryPayload, NumericFreezeError> {
    normalize_owner_local_cross_section_references(document, meter)?;
    let stationing = freeze_stationing(document, direction_profile, meter)?;
    let layouts = freeze_cross_section_layouts(document, meter)?;
    let lateral_curves = freeze_lateral_curves_with_limit(
        document,
        &stationing,
        &layouts,
        accuracy_profile,
        direction_profile,
        geometry_point_limit,
        meter,
    )?;
    let geometry_point_count = lateral_curves.iter().try_fold(0_u64, |total, curve| {
        total.checked_add(u64::try_from(curve.points.len()).ok()?)
    });
    let Some(mut geometry_point_count) = geometry_point_count else {
        return Err(arithmetic_error());
    };
    // internal edge 紧随 lane/facility 曲线消费同一 GeometryPointCount 预算。
    let internal_edge_curves = freeze_internal_edge_curves(
        document,
        accuracy_profile,
        direction_profile,
        geometry_point_limit
            .checked_sub(geometry_point_count)
            .ok_or_else(arithmetic_error)?,
        meter,
    )?;
    geometry_point_count = internal_edge_curves
        .iter()
        .try_fold(geometry_point_count, |total, curve| {
            total.checked_add(u64::try_from(curve.points.len()).ok()?)
        })
        .ok_or_else(arithmetic_error)?;
    debug_assert!(geometry_point_count <= geometry_point_limit);
    Ok(FrozenGeometryPayload {
        lateral_curves,
        internal_edge_curves,
        geometry_point_count,
        offset_curve_distribution: offset_curve_distribution(&layouts),
    })
}

/// §9.2 前端计数消费的横向 offset 曲线分布：按 |中心偏移| 的 f64 位模式精确分组，
/// 桶序即位模式升序，与曲线声明顺序无关。
fn offset_curve_distribution(
    layouts: &[FrozenCrossSectionLayout],
) -> Box<[FrozenOffsetCurveBucket]> {
    let mut buckets = BTreeMap::<u64, u64>::new();
    for layout in layouts {
        for intent in &layout.items {
            *buckets
                .entry(intent.center_offset_meters.abs().to_bits())
                .or_insert(0) += 1;
        }
    }
    buckets
        .into_iter()
        .map(
            |(absolute_offset_meters_bits, curve_count)| FrozenOffsetCurveBucket {
                absolute_offset_meters_bits,
                curve_count,
            },
        )
        .collect()
}

/// §4.4 的 internal edge 显式 geometry freeze：复用 reference 细分机器与规范量化，
/// 冻结点计入同一 GeometryPointCount 预算，按 `(junction, laneEdgeKey)` 可枚举。
fn freeze_internal_edge_curves(
    document: &ParsedGeometryDocument,
    accuracy_profile: GeometryAccuracyProfile,
    direction_profile: GeometryDirectionProfile,
    geometry_point_limit: u64,
    meter: &mut StageScratchMeter,
) -> Result<Box<[FrozenInternalEdgeCurve]>, NumericFreezeError> {
    let mut output = Vec::new();
    let mut geometry_point_count = 0_u64;
    for junction in &document.junctions {
        for edge in &junction.internal_edges {
            let samples = freeze_curve(
                &edge.geometry,
                accuracy_profile,
                direction_profile,
                direction_profile,
                meter,
            )?;
            let points = quantize_and_validate_explicit_curve(
                &edge.geometry,
                &samples,
                accuracy_profile,
                direction_profile,
                edge.geometry.span,
                meter,
            )?;
            // 每条 edge 的 samples 在 quantize 完后即消费完，归还其暂存字节。
            meter.shrink(scaled_bytes(
                samples.len(),
                size_of::<FrozenReferenceSample>() as u64,
            ));
            geometry_point_count = geometry_point_count
                .checked_add(u64::try_from(points.len()).map_err(|_| arithmetic_error())?)
                .ok_or_else(arithmetic_error)?;
            if geometry_point_count > geometry_point_limit {
                return Err(point_limit_error(edge.geometry.span));
            }
            output.push(FrozenInternalEdgeCurve {
                junction_key: junction.junction_key.value.clone(),
                lane_edge_key: edge.lane_edge_key.value.clone(),
                points: points.into_boxed_slice(),
            });
        }
    }
    Ok(output.into_boxed_slice())
}

fn freeze_lateral_curves_with_limit(
    document: &ParsedGeometryDocument,
    stationing: &[FrozenRoadStationing],
    layouts: &[FrozenCrossSectionLayout],
    accuracy_profile: GeometryAccuracyProfile,
    direction_profile: GeometryDirectionProfile,
    geometry_point_limit: u64,
    meter: &mut StageScratchMeter,
) -> Result<Box<[FrozenLateralCurve]>, NumericFreezeError> {
    if stationing.len() != document.roads.len() {
        return Err(arithmetic_error());
    }
    let expected_layout_count = document
        .roads
        .iter()
        .map(|road| road.cross_section_spans.len())
        .sum::<usize>();
    if layouts.len() != expected_layout_count {
        return Err(arithmetic_error());
    }

    let mut output = Vec::new();
    let mut geometry_point_count = 0_u64;
    let mut layout_index = 0_usize;
    for (road, road_stationing) in document.roads.iter().zip(stationing) {
        if road_stationing.road_key.as_ref() != road.road_key.value.as_ref()
            || road_stationing.spans.len() != road.cross_section_spans.len()
        {
            return Err(arithmetic_error());
        }
        for ((span, stations), layout) in road
            .cross_section_spans
            .iter()
            .zip(&road_stationing.spans)
            .zip(&layouts[layout_index..layout_index + road.cross_section_spans.len()])
        {
            if layout.span_key.as_ref() != span.span_key.value.as_ref() {
                return Err(arithmetic_error());
            }
            for intent in &layout.items {
                let remaining_point_count = geometry_point_limit
                    .checked_sub(geometry_point_count)
                    .ok_or_else(|| point_limit_error(span.span))?;
                let samples = freeze_offset_curve_range(
                    &road.reference_line,
                    intent.center_offset_meters,
                    *stations,
                    accuracy_profile,
                    direction_profile,
                    remaining_point_count,
                    meter,
                )?;
                let mut points = quantize_and_validate_curve(
                    &road.reference_line,
                    intent.center_offset_meters,
                    &samples,
                    accuracy_profile,
                    direction_profile,
                    span.span,
                    meter,
                )?;
                // samples 在 quantize 完后即消费完，归还其暂存字节。
                meter.shrink(scaled_bytes(
                    samples.len(),
                    size_of::<FrozenOffsetSample>() as u64,
                ));
                if intent.kind == LateralIntentKind::BackwardLane {
                    points.reverse();
                }
                geometry_point_count = geometry_point_count
                    .checked_add(u64::try_from(points.len()).map_err(|_| arithmetic_error())?)
                    .ok_or_else(arithmetic_error)?;
                output.push(FrozenLateralCurve {
                    span_key: layout.span_key.clone(),
                    key: intent.key.clone(),
                    kind: intent.kind,
                    points: points.into_boxed_slice(),
                });
            }
            layout_index += 1;
        }
    }
    Ok(output.into_boxed_slice())
}

struct PendingLateralIntent {
    key: Box<str>,
    kind: LateralIntentKind,
    width: f64,
}

fn compute_lateral_offsets(
    pending: Vec<PendingLateralIntent>,
    reference_index: usize,
    span: ByteSpan,
    meter: &mut StageScratchMeter,
) -> Result<Box<[FrozenLateralIntent]>, NumericFreezeError> {
    if pending.is_empty() || reference_index >= pending.len() {
        return Err(NumericFreezeError {
            violation: NumericFreezeViolation::InvalidCrossSectionReference,
            field: "referenceLaneKey",
            span,
        });
    }
    // boundaries 与 pending 同亡：分配前入账，成功返回前归还；错误路径不归还。
    let boundaries_bytes = scaled_bytes(pending.len(), size_of::<(f64, f64)>() as u64);
    meter
        .grow(boundaries_bytes)
        .map_err(|_| scratch_exceeded(span))?;
    let mut boundaries = vec![(0.0_f64, 0.0_f64); pending.len()];
    let reference_half_width = pending[reference_index].width * 0.5;
    boundaries[reference_index] = (reference_half_width, -reference_half_width);

    let mut adjacent_left_boundary = reference_half_width;
    for index in (0..reference_index).rev() {
        let right = adjacent_left_boundary;
        let left = right + pending[index].width;
        if !left.is_finite() || left <= right {
            return Err(lateral_overflow(span));
        }
        boundaries[index] = (left, right);
        adjacent_left_boundary = left;
    }

    let mut adjacent_right_boundary = -reference_half_width;
    for index in (reference_index + 1)..pending.len() {
        let left = adjacent_right_boundary;
        let right = left - pending[index].width;
        if !right.is_finite() || left <= right {
            return Err(lateral_overflow(span));
        }
        boundaries[index] = (left, right);
        adjacent_right_boundary = right;
    }

    let items = pending
        .into_iter()
        .zip(boundaries)
        .map(|(pending, (left, right))| {
            let center = (left + right) * 0.5;
            if !center.is_finite() {
                return Err(lateral_overflow(span));
            }
            Ok(FrozenLateralIntent {
                key: pending.key,
                kind: pending.kind,
                center_offset_meters: canonical_zero(center),
                left_boundary_offset_meters: canonical_zero(left),
                right_boundary_offset_meters: canonical_zero(right),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    meter.shrink(boundaries_bytes);
    Ok(items.into_boxed_slice())
}

fn parse_positive(value: &RawNumber, field: &'static str) -> Result<f64, NumericFreezeError> {
    let parsed = parse_finite(value, field)?;
    if parsed <= 0.0 {
        return Err(NumericFreezeError {
            violation: NumericFreezeViolation::InvalidWidth,
            field,
            span: value.span,
        });
    }
    Ok(parsed)
}

const fn lateral_overflow(span: ByteSpan) -> NumericFreezeError {
    NumericFreezeError {
        violation: NumericFreezeViolation::ArithmeticOverflow,
        field: "crossSectionSpans.elements",
        span,
    }
}

fn build_station_intervals(
    samples: &[FrozenReferenceSample],
    span: ByteSpan,
    meter: &mut StageScratchMeter,
) -> Result<Box<[StationInterval]>, NumericFreezeError> {
    let Some(first) = samples.first() else {
        return Err(NumericFreezeError {
            violation: NumericFreezeViolation::ArithmeticOverflow,
            field: "referenceLine.segments",
            span,
        });
    };
    // intervals 存续到 station 表冻结完成（payload 结束），只入账不归还。
    meter
        .grow(scaled_bytes(
            samples.len().saturating_sub(1),
            size_of::<StationInterval>() as u64,
        ))
        .map_err(|_| scratch_exceeded(span))?;
    let mut intervals = Vec::with_capacity(samples.len().saturating_sub(1));
    let mut previous_segment = first.segment_index;
    let mut previous_parameter = first.parameter;
    let mut previous_point = Point3::from_array(first.point);
    let mut cumulative_length = 0.0_f64;
    for sample in &samples[1..] {
        let t0 = if sample.segment_index == previous_segment {
            previous_parameter
        } else {
            0.0
        };
        let point = Point3::from_array(sample.point);
        let chord = point.subtract(previous_point);
        let chord_length = chord.length_squared().sqrt();
        let cumulative_end = cumulative_length + chord_length;
        if !chord_length.is_finite()
            || chord_length <= 0.0
            || !cumulative_end.is_finite()
            || cumulative_end <= cumulative_length
        {
            return Err(NumericFreezeError {
                violation: NumericFreezeViolation::ArithmeticOverflow,
                field: "referenceLine.stationIntervals",
                span,
            });
        }
        intervals.push(StationInterval {
            segment_index: sample.segment_index,
            t0_bits: canonical_zero(t0).to_bits(),
            t1_bits: canonical_zero(sample.parameter).to_bits(),
            cumulative_start_length_bits: canonical_zero(cumulative_length).to_bits(),
            cumulative_end_length_bits: cumulative_end.to_bits(),
        });
        previous_segment = sample.segment_index;
        previous_parameter = sample.parameter;
        previous_point = point;
        cumulative_length = cumulative_end;
    }
    Ok(intervals.into_boxed_slice())
}

fn freeze_span_stations(
    spans: &[super::road::ParsedCrossSectionSpan],
    intervals: &[StationInterval],
    meter: &mut StageScratchMeter,
) -> Result<Box<[FrozenSpanStations]>, NumericFreezeError> {
    let Some(last_interval) = intervals.last() else {
        return Err(NumericFreezeError {
            violation: NumericFreezeViolation::ArithmeticOverflow,
            field: "referenceLine.stationIntervals",
            span: ByteSpan { start: 0, end: 0 },
        });
    };
    let final_station = f64::from_bits(last_interval.cumulative_end_length_bits);
    // frozen 存续到 station 表冻结完成，只入账不归还。
    meter
        .grow(scaled_bytes(
            spans.len(),
            size_of::<FrozenSpanStations>() as u64,
        ))
        .map_err(|_| {
            scratch_exceeded(
                spans
                    .first()
                    .map_or(ByteSpan { start: 0, end: 0 }, |span| span.span),
            )
        })?;
    let mut frozen = Vec::with_capacity(spans.len());
    let mut expected_start_bits = 0.0_f64.to_bits();
    for (index, span) in spans.iter().enumerate() {
        let start = parse_finite(&span.start_station_meters, "startStationMeters")?;
        if start.to_bits() != expected_start_bits || start < 0.0 || start >= final_station {
            return Err(NumericFreezeError {
                violation: NumericFreezeViolation::IncompleteStationCoverage,
                field: "startStationMeters",
                span: span.start_station_meters.span,
            });
        }
        let start_parameter = locate_station(intervals, start, span.start_station_meters.span)?;
        let is_last = index + 1 == spans.len();
        let (end_parameter, next_start_bits) = match &span.end_station_meters {
            super::road::ParsedEndStation::Number(value) => {
                if is_last {
                    return Err(NumericFreezeError {
                        violation: NumericFreezeViolation::IncompleteStationCoverage,
                        field: "endStationMeters",
                        span: value.span,
                    });
                }
                let end = parse_finite(value, "endStationMeters")?;
                if end <= start || end >= final_station {
                    return Err(NumericFreezeError {
                        violation: NumericFreezeViolation::InvalidStation,
                        field: "endStationMeters",
                        span: value.span,
                    });
                }
                (locate_station(intervals, end, value.span)?, end.to_bits())
            }
            super::road::ParsedEndStation::End(end_span) => {
                if !is_last {
                    return Err(NumericFreezeError {
                        violation: NumericFreezeViolation::IncompleteStationCoverage,
                        field: "endStationMeters",
                        span: *end_span,
                    });
                }
                (
                    FrozenCurveParameter {
                        segment_index: last_interval.segment_index,
                        parameter: f64::from_bits(last_interval.t1_bits),
                    },
                    final_station.to_bits(),
                )
            }
        };
        frozen.push(FrozenSpanStations {
            start: start_parameter,
            end: end_parameter,
        });
        expected_start_bits = next_start_bits;
    }
    Ok(frozen.into_boxed_slice())
}

fn locate_station(
    intervals: &[StationInterval],
    station: f64,
    span: ByteSpan,
) -> Result<FrozenCurveParameter, NumericFreezeError> {
    let Some(last) = intervals.last() else {
        return Err(NumericFreezeError {
            violation: NumericFreezeViolation::ArithmeticOverflow,
            field: "referenceLine.stationIntervals",
            span,
        });
    };
    let final_station = f64::from_bits(last.cumulative_end_length_bits);
    if !station.is_finite() || station < 0.0 || station >= final_station {
        return Err(NumericFreezeError {
            violation: NumericFreezeViolation::InvalidStation,
            field: "stationMeters",
            span,
        });
    }
    let index = intervals
        .partition_point(|interval| f64::from_bits(interval.cumulative_end_length_bits) < station);
    let interval = &intervals[index];
    let t0 = f64::from_bits(interval.t0_bits);
    let t1 = f64::from_bits(interval.t1_bits);
    let start_length = f64::from_bits(interval.cumulative_start_length_bits);
    let end_length = f64::from_bits(interval.cumulative_end_length_bits);
    let alpha = (station - start_length) / (end_length - start_length);
    let parameter = t0 + (alpha * (t1 - t0));
    if !parameter.is_finite() || parameter < t0 || parameter > t1 {
        return Err(NumericFreezeError {
            violation: NumericFreezeViolation::ArithmeticOverflow,
            field: "stationMeters",
            span,
        });
    }
    Ok(FrozenCurveParameter {
        segment_index: interval.segment_index,
        parameter: canonical_zero(parameter),
    })
}

fn freeze_curve(
    curve: &ParsedCurve,
    accuracy_profile: GeometryAccuracyProfile,
    direction_profile: GeometryDirectionProfile,
    join_profile: GeometryDirectionProfile,
    meter: &mut StageScratchMeter,
) -> Result<Box<[FrozenReferenceSample]>, NumericFreezeError> {
    let start = parse_vec3(&curve.start, "referenceLine.start")?;
    meter
        .grow(size_of::<FrozenReferenceSample>() as u64)
        .map_err(|_| scratch_exceeded(curve.span))?;
    let mut samples = vec![FrozenReferenceSample {
        segment_index: 0,
        parameter: 0.0,
        point: start.into_array(),
    }];
    let mut segment_start = start;
    let mut previous_end_tangent = None;

    for (segment_index, segment) in curve.segments.iter().enumerate() {
        let segment_index = u32::try_from(segment_index).map_err(|_| NumericFreezeError {
            violation: NumericFreezeViolation::ArithmeticOverflow,
            field: "referenceLine.segments",
            span: curve.span,
        })?;
        let (end, start_tangent, end_tangent, span) = match segment {
            ParsedCurveSegment::Line { end, span } => {
                let end = parse_vec3(end, "referenceLine.segments.end")?;
                let tangent = end.subtract(segment_start);
                (end, tangent, tangent, *span)
            }
            ParsedCurveSegment::CubicBezier {
                controls,
                end,
                span,
            } => {
                let first = parse_vec3(&controls[0], "referenceLine.segments.controls")?;
                let second = parse_vec3(&controls[1], "referenceLine.segments.controls")?;
                let end = parse_vec3(end, "referenceLine.segments.end")?;
                (
                    end,
                    first.subtract(segment_start),
                    end.subtract(second),
                    *span,
                )
            }
        };
        validate_tangent(start_tangent, span)?;
        validate_tangent(end_tangent, span)?;
        if let Some(previous) = previous_end_tangent {
            validate_source_join(previous, start_tangent, join_profile, span)?;
        }

        match segment {
            ParsedCurveSegment::Line { .. } => {
                meter
                    .grow(size_of::<FrozenReferenceSample>() as u64)
                    .map_err(|_| scratch_exceeded(span))?;
                samples.push(FrozenReferenceSample {
                    segment_index,
                    parameter: 1.0,
                    point: end.into_array(),
                });
            }
            ParsedCurveSegment::CubicBezier { controls, .. } => {
                let first = parse_vec3(&controls[0], "referenceLine.segments.controls")?;
                let second = parse_vec3(&controls[1], "referenceLine.segments.controls")?;
                subdivide_cubic(
                    [segment_start, first, second, end],
                    segment_index,
                    accuracy_profile,
                    direction_profile,
                    span,
                    &mut samples,
                    meter,
                )?;
            }
        }
        segment_start = end;
        previous_end_tangent = Some(end_tangent);
    }
    Ok(samples.into_boxed_slice())
}

#[allow(
    dead_code,
    reason = "called after cross-section prefix offsets are frozen in the following slice"
)]
fn freeze_offset_curve(
    curve: &ParsedCurve,
    offset_meters: f64,
    accuracy_profile: GeometryAccuracyProfile,
    direction_profile: GeometryDirectionProfile,
    meter: &mut StageScratchMeter,
) -> Result<Box<[FrozenOffsetSample]>, NumericFreezeError> {
    freeze_offset_curve_with_forced(
        curve,
        offset_meters,
        accuracy_profile,
        direction_profile,
        &[],
        meter,
    )
}

fn freeze_offset_curve_with_forced(
    curve: &ParsedCurve,
    offset_meters: f64,
    accuracy_profile: GeometryAccuracyProfile,
    direction_profile: GeometryDirectionProfile,
    forced_parameters: &[FrozenCurveParameter],
    meter: &mut StageScratchMeter,
) -> Result<Box<[FrozenOffsetSample]>, NumericFreezeError> {
    if !offset_meters.is_finite() {
        return Err(NumericFreezeError {
            violation: NumericFreezeViolation::NonFiniteNumber,
            field: "offsetMeters",
            span: curve.span,
        });
    }
    let mut segment_start = parse_vec3(&curve.start, "referenceLine.start")?;
    let first = evaluate_segment_offset(segment_start, &curve.segments[0], 0.0, offset_meters)?;
    meter
        .grow(size_of::<FrozenOffsetSample>() as u64)
        .map_err(|_| scratch_exceeded(curve.span))?;
    let mut output = vec![FrozenOffsetSample {
        segment_index: 0,
        parameter: 0.0,
        point: first.position.into_array(),
    }];

    for (index, segment) in curve.segments.iter().enumerate() {
        let segment_index = u32::try_from(index).map_err(|_| arithmetic_error())?;
        let parameters = segment_parameters(segment_index, forced_parameters, curve.span)?;
        match segment {
            ParsedCurveSegment::Line { end, span } => {
                let end = parse_vec3(end, "referenceLine.segments.end")?;
                for parameter in parameters.iter().copied().skip(1) {
                    let frozen =
                        evaluate_line_offset(segment_start, end, parameter, offset_meters, *span)?;
                    push_offset_sample(
                        &mut output,
                        FrozenOffsetSample {
                            segment_index,
                            parameter,
                            point: frozen.position.into_array(),
                        },
                        u64::MAX,
                        *span,
                        meter,
                    )?;
                }
                segment_start = end;
            }
            ParsedCurveSegment::CubicBezier { end, span, .. } => {
                let controls = cubic_controls(segment_start, segment)?;
                for pair in parameters.windows(2) {
                    let cropped = crop_cubic(controls, pair[0], pair[1], *span)?;
                    subdivide_offset_cubic(
                        cropped,
                        OffsetSubdivisionSpec {
                            segment_index,
                            parameter_start: pair[0],
                            parameter_end: pair[1],
                            offset_meters,
                            accuracy_profile,
                            direction_profile,
                            span: *span,
                            max_point_count: u64::MAX,
                        },
                        &mut output,
                        meter,
                    )?;
                }
                segment_start = parse_vec3(end, "referenceLine.segments.end")?;
            }
        }
    }
    reevaluate_offset_samples(curve, offset_meters, &mut output, meter)?;
    Ok(output.into_boxed_slice())
}

fn freeze_offset_curve_range(
    curve: &ParsedCurve,
    offset_meters: f64,
    range: FrozenSpanStations,
    accuracy_profile: GeometryAccuracyProfile,
    direction_profile: GeometryDirectionProfile,
    point_limit: u64,
    meter: &mut StageScratchMeter,
) -> Result<Box<[FrozenOffsetSample]>, NumericFreezeError> {
    if !offset_meters.is_finite()
        || range.start.segment_index > range.end.segment_index
        || (range.start.segment_index == range.end.segment_index
            && range.start.parameter >= range.end.parameter)
    {
        return Err(NumericFreezeError {
            violation: NumericFreezeViolation::InvalidStation,
            field: "crossSectionSpans",
            span: curve.span,
        });
    }

    let start_index = usize::try_from(range.start.segment_index).map_err(|_| arithmetic_error())?;
    let start_segment = curve
        .segments
        .get(start_index)
        .ok_or_else(arithmetic_error)?;
    // segment_starts 随本函数结束而归还（见循环后的 shrink）。
    let segment_starts_bytes = scaled_bytes(curve.segments.len(), size_of::<Point3>() as u64);
    meter
        .grow(segment_starts_bytes)
        .map_err(|_| scratch_exceeded(curve.span))?;
    let segment_starts = parsed_segment_starts(curve)?;
    let first = evaluate_segment_offset(
        segment_starts[start_index],
        start_segment,
        range.start.parameter,
        offset_meters,
    )?;
    if point_limit == 0 {
        return Err(point_limit_error(curve.span));
    }
    meter
        .grow(size_of::<FrozenOffsetSample>() as u64)
        .map_err(|_| scratch_exceeded(curve.span))?;
    let mut output = vec![FrozenOffsetSample {
        segment_index: range.start.segment_index,
        parameter: range.start.parameter,
        point: first.position.into_array(),
    }];

    for segment_index in range.start.segment_index..=range.end.segment_index {
        let index = usize::try_from(segment_index).map_err(|_| arithmetic_error())?;
        let segment = curve.segments.get(index).ok_or_else(arithmetic_error)?;
        let parameter_start = if segment_index == range.start.segment_index {
            range.start.parameter
        } else {
            0.0
        };
        let parameter_end = if segment_index == range.end.segment_index {
            range.end.parameter
        } else {
            1.0
        };
        if parameter_start == parameter_end {
            continue;
        }
        if !parameter_start.is_finite()
            || !parameter_end.is_finite()
            || parameter_start < 0.0
            || parameter_start >= parameter_end
            || parameter_end > 1.0
        {
            return Err(NumericFreezeError {
                violation: NumericFreezeViolation::InvalidStation,
                field: "stationMeters",
                span: curve.span,
            });
        }
        match segment {
            ParsedCurveSegment::Line { end, span } => {
                let frozen = evaluate_line_offset(
                    segment_starts[index],
                    parse_vec3(end, "referenceLine.segments.end")?,
                    parameter_end,
                    offset_meters,
                    *span,
                )?;
                push_offset_sample(
                    &mut output,
                    FrozenOffsetSample {
                        segment_index,
                        parameter: parameter_end,
                        point: frozen.position.into_array(),
                    },
                    point_limit,
                    *span,
                    meter,
                )?;
            }
            ParsedCurveSegment::CubicBezier { span, .. } => {
                let controls = cubic_controls(segment_starts[index], segment)?;
                let cropped = crop_cubic(controls, parameter_start, parameter_end, *span)?;
                subdivide_offset_cubic(
                    cropped,
                    OffsetSubdivisionSpec {
                        segment_index,
                        parameter_start,
                        parameter_end,
                        offset_meters,
                        accuracy_profile,
                        direction_profile,
                        span: *span,
                        max_point_count: point_limit,
                    },
                    &mut output,
                    meter,
                )?;
            }
        }
    }
    // segment_starts 此后不再使用；samples 输出由调用方在 quantize 完后归还。
    meter.shrink(segment_starts_bytes);
    drop(segment_starts);
    reevaluate_offset_samples(curve, offset_meters, &mut output, meter)?;
    Ok(output.into_boxed_slice())
}

fn push_offset_sample(
    output: &mut Vec<FrozenOffsetSample>,
    sample: FrozenOffsetSample,
    point_limit: u64,
    span: ByteSpan,
    meter: &mut StageScratchMeter,
) -> Result<(), NumericFreezeError> {
    if u64::try_from(output.len()).unwrap_or(u64::MAX) >= point_limit {
        return Err(point_limit_error(span));
    }
    meter
        .grow(size_of::<FrozenOffsetSample>() as u64)
        .map_err(|_| scratch_exceeded(span))?;
    output.push(sample);
    Ok(())
}

const fn point_limit_error(span: ByteSpan) -> NumericFreezeError {
    NumericFreezeError {
        violation: NumericFreezeViolation::GeometryPointLimitExceeded,
        field: "canonicalGeometryPoints",
        span,
    }
}

fn reevaluate_offset_samples(
    curve: &ParsedCurve,
    offset_meters: f64,
    samples: &mut [FrozenOffsetSample],
    meter: &mut StageScratchMeter,
) -> Result<(), NumericFreezeError> {
    let segment_starts_bytes = scaled_bytes(curve.segments.len(), size_of::<Point3>() as u64);
    meter
        .grow(segment_starts_bytes)
        .map_err(|_| scratch_exceeded(curve.span))?;
    let segment_starts = parsed_segment_starts(curve)?;
    for sample in samples {
        let index = usize::try_from(sample.segment_index).map_err(|_| arithmetic_error())?;
        let segment = curve.segments.get(index).ok_or_else(arithmetic_error)?;
        sample.point = evaluate_segment_offset(
            segment_starts[index],
            segment,
            sample.parameter,
            offset_meters,
        )?
        .position
        .into_array();
    }
    meter.shrink(segment_starts_bytes);
    Ok(())
}

fn parsed_segment_starts(curve: &ParsedCurve) -> Result<Vec<Point3>, NumericFreezeError> {
    let mut starts = Vec::with_capacity(curve.segments.len());
    let mut start = parse_vec3(&curve.start, "referenceLine.start")?;
    for segment in &curve.segments {
        starts.push(start);
        start = match segment {
            ParsedCurveSegment::Line { end, .. } | ParsedCurveSegment::CubicBezier { end, .. } => {
                parse_vec3(end, "referenceLine.segments.end")?
            }
        };
    }
    Ok(starts)
}

fn quantize_and_validate_curve(
    curve: &ParsedCurve,
    offset_meters: f64,
    samples: &[FrozenOffsetSample],
    accuracy_profile: GeometryAccuracyProfile,
    direction_profile: GeometryDirectionProfile,
    span: ByteSpan,
    meter: &mut StageScratchMeter,
) -> Result<Vec<FrozenCanonicalPoint>, NumericFreezeError> {
    if samples.len() < 2 {
        return Err(NumericFreezeError {
            violation: NumericFreezeViolation::QuantizedSegmentTooShort,
            field: "centerline",
            span,
        });
    }
    // 输出 points 是 GeometryPointCount 维度管控的载荷，不计入暂存账簿。
    let points = samples
        .iter()
        .map(|sample| quantize_point(Point3::from_array(sample.point), span))
        .collect::<Result<Vec<_>, _>>()?;

    validate_quantized_segments(&points, direction_profile, span)?;
    // segment_starts 是按输入规模的中间 Vec：分配前入账，用完归还。
    let segment_starts_bytes = scaled_bytes(curve.segments.len(), size_of::<Point3>() as u64);
    meter
        .grow(segment_starts_bytes)
        .map_err(|_| scratch_exceeded(curve.span))?;
    let segment_starts = parsed_segment_starts(curve)?;
    for (index, pair) in samples.windows(2).enumerate() {
        let segment_index = pair[1].segment_index;
        let segment_usize = usize::try_from(segment_index).map_err(|_| arithmetic_error())?;
        let segment = curve
            .segments
            .get(segment_usize)
            .ok_or_else(arithmetic_error)?;
        let t0 = if pair[0].segment_index == segment_index {
            pair[0].parameter
        } else {
            0.0
        };
        let t1 = pair[1].parameter;
        let analytic_bound = match segment {
            ParsedCurveSegment::Line { .. } => 0.0,
            ParsedCurveSegment::CubicBezier {
                span: segment_span, ..
            } => {
                let controls = cubic_controls(segment_starts[segment_usize], segment)?;
                let cropped = crop_cubic(controls, t0, t1, *segment_span)?;
                let Some(bound) = offset_curvature_bound(cropped, offset_meters)? else {
                    return Err(NumericFreezeError {
                        violation: NumericFreezeViolation::TotalPositionErrorExceeded,
                        field: "centerline",
                        span: *segment_span,
                    });
                };
                bound / 8.0
            }
        };
        let endpoint_error =
            point_distance(Point3::from_array(pair[0].point), points[index].as_f64())?.max(
                point_distance(
                    Point3::from_array(pair[1].point),
                    points[index + 1].as_f64(),
                )?,
            );
        let total_bound = analytic_bound + endpoint_error;
        if !total_bound.is_finite() || total_bound > accuracy_profile.max_position_error_meters() {
            return Err(NumericFreezeError {
                violation: NumericFreezeViolation::TotalPositionErrorExceeded,
                field: "centerline",
                span: segment_span(segment),
            });
        }
    }
    meter.shrink(segment_starts_bytes);
    Ok(points)
}

/// 显式 authoring 曲线的量化与生产 oracle：cubic 解析界取 de Casteljau 子曲线
/// 内控制点到有限端点弦的最大距离（§6.1 的 reference 证明），不复用 offset `K/8` 界。
fn quantize_and_validate_explicit_curve(
    curve: &ParsedCurve,
    samples: &[FrozenReferenceSample],
    accuracy_profile: GeometryAccuracyProfile,
    direction_profile: GeometryDirectionProfile,
    span: ByteSpan,
    meter: &mut StageScratchMeter,
) -> Result<Vec<FrozenCanonicalPoint>, NumericFreezeError> {
    if samples.len() < 2 {
        return Err(NumericFreezeError {
            violation: NumericFreezeViolation::QuantizedSegmentTooShort,
            field: "centerline",
            span,
        });
    }
    // 输出 points 是 GeometryPointCount 维度管控的载荷，不计入暂存账簿。
    let points = samples
        .iter()
        .map(|sample| quantize_point(Point3::from_array(sample.point), span))
        .collect::<Result<Vec<_>, _>>()?;

    validate_quantized_segments(&points, direction_profile, span)?;
    // segment_starts 是按输入规模的中间 Vec：分配前入账，用完归还。
    let segment_starts_bytes = scaled_bytes(curve.segments.len(), size_of::<Point3>() as u64);
    meter
        .grow(segment_starts_bytes)
        .map_err(|_| scratch_exceeded(curve.span))?;
    let segment_starts = parsed_segment_starts(curve)?;
    for (index, pair) in samples.windows(2).enumerate() {
        let segment_index = pair[1].segment_index;
        let segment_usize = usize::try_from(segment_index).map_err(|_| arithmetic_error())?;
        let segment = curve
            .segments
            .get(segment_usize)
            .ok_or_else(arithmetic_error)?;
        let t0 = if pair[0].segment_index == segment_index {
            pair[0].parameter
        } else {
            0.0
        };
        let t1 = pair[1].parameter;
        let analytic_bound = match segment {
            ParsedCurveSegment::Line { .. } => 0.0,
            ParsedCurveSegment::CubicBezier {
                span: segment_span, ..
            } => {
                let controls = cubic_controls(segment_starts[segment_usize], segment)?;
                let cropped = crop_cubic(controls, t0, t1, *segment_span)?;
                reference_hull_bound(cropped)?
            }
        };
        let endpoint_error =
            point_distance(Point3::from_array(pair[0].point), points[index].as_f64())?.max(
                point_distance(
                    Point3::from_array(pair[1].point),
                    points[index + 1].as_f64(),
                )?,
            );
        let total_bound = analytic_bound + endpoint_error;
        if !total_bound.is_finite() || total_bound > accuracy_profile.max_position_error_meters() {
            return Err(NumericFreezeError {
                violation: NumericFreezeViolation::TotalPositionErrorExceeded,
                field: "centerline",
                span: segment_span(segment),
            });
        }
    }
    meter.shrink(segment_starts_bytes);
    Ok(points)
}

/// reference cubic 的解析弦距界：两个内控制点到端点有限线段的最大三维距离。
fn reference_hull_bound(controls: [Point3; 4]) -> Result<f64, NumericFreezeError> {
    let first = point_segment_distance_squared(controls[1], controls[0], controls[3])?;
    let second = point_segment_distance_squared(controls[2], controls[0], controls[3])?;
    let bound = first.max(second).sqrt();
    if !bound.is_finite() {
        return Err(arithmetic_error());
    }
    Ok(bound)
}

fn quantize_point(
    point: Point3,
    span: ByteSpan,
) -> Result<FrozenCanonicalPoint, NumericFreezeError> {
    if [point.x, point.y, point.z].into_iter().any(|component| {
        !component.is_finite()
            || !(f64::from(CANONICAL_POINT_COMPONENT_MIN_METERS)
                ..=f64::from(CANONICAL_POINT_COMPONENT_MAX_METERS))
                .contains(&component)
    }) {
        return Err(NumericFreezeError {
            violation: NumericFreezeViolation::CoordinateOutOfRange,
            field: "centerline.points",
            span,
        });
    }
    let canonical = FrozenCanonicalPoint {
        x: canonical_f32(point.x as f32),
        y: canonical_f32(point.y as f32),
        z: canonical_f32(point.z as f32),
    };
    if [canonical.x, canonical.y, canonical.z]
        .into_iter()
        .any(|component| {
            !component.is_finite()
                || !(CANONICAL_POINT_COMPONENT_MIN_METERS..=CANONICAL_POINT_COMPONENT_MAX_METERS)
                    .contains(&component)
        })
    {
        return Err(NumericFreezeError {
            violation: NumericFreezeViolation::CoordinateOutOfRange,
            field: "centerline.points",
            span,
        });
    }
    Ok(canonical)
}

fn validate_quantized_segments(
    points: &[FrozenCanonicalPoint],
    direction_profile: GeometryDirectionProfile,
    span: ByteSpan,
) -> Result<(), NumericFreezeError> {
    let mut previous_chord = None;
    for pair in points.windows(2) {
        let delta_f32 = [
            canonical_f32(pair[1].x - pair[0].x),
            canonical_f32(pair[1].y - pair[0].y),
            canonical_f32(pair[1].z - pair[0].z),
        ];
        let length = delta_f32[0].hypot(delta_f32[1]).hypot(delta_f32[2]);
        if !length.is_finite() || length <= SPATIAL_MIN_SEGMENT_LENGTH_METERS {
            return Err(NumericFreezeError {
                violation: NumericFreezeViolation::QuantizedSegmentTooShort,
                field: "centerline.points",
                span,
            });
        }
        let tangent = normalize_f32(delta_f32);
        let projected_up = tangent[0].hypot(tangent[2]);
        if !projected_up.is_finite() || projected_up < SPATIAL_MIN_PROJECTED_UP_LENGTH {
            return Err(NumericFreezeError {
                violation: NumericFreezeViolation::DegenerateProjectedUp,
                field: "centerline.points",
                span,
            });
        }
        let chord = pair[1]
            .as_f64()
            .subtract(pair[0].as_f64())
            .canonicalized_zero();
        if let Some(previous) = previous_chord
            && !direction_ok(previous, chord, direction_profile.runtime_cos_squared()).map_err(
                |mut error| {
                    error.span = span;
                    error
                },
            )?
        {
            return Err(NumericFreezeError {
                violation: NumericFreezeViolation::FinalDirectionExceeded,
                field: "centerline.points",
                span,
            });
        }
        previous_chord = Some(chord);
    }
    Ok(())
}

fn normalize_f32(value: [f32; 3]) -> [f32; 3] {
    let scale = value[0].abs().max(value[1].abs()).max(value[2].abs());
    let scaled = [value[0] / scale, value[1] / scale, value[2] / scale];
    let length = scaled[0].hypot(scaled[1]).hypot(scaled[2]);
    [scaled[0] / length, scaled[1] / length, scaled[2] / length]
}

fn point_distance(left: Point3, right: Point3) -> Result<f64, NumericFreezeError> {
    let distance = left.subtract(right).length_squared().sqrt();
    if !distance.is_finite() {
        return Err(arithmetic_error());
    }
    Ok(distance)
}

const fn canonical_f32(value: f32) -> f32 {
    if value == 0.0 { 0.0 } else { value }
}

const fn segment_span(segment: &ParsedCurveSegment) -> ByteSpan {
    match segment {
        ParsedCurveSegment::Line { span, .. } | ParsedCurveSegment::CubicBezier { span, .. } => {
            *span
        }
    }
}

impl FrozenCanonicalPoint {
    fn as_f64(self) -> Point3 {
        Point3 {
            x: f64::from(self.x),
            y: f64::from(self.y),
            z: f64::from(self.z),
        }
    }
}

fn segment_parameters(
    segment_index: u32,
    forced_parameters: &[FrozenCurveParameter],
    span: ByteSpan,
) -> Result<Vec<f64>, NumericFreezeError> {
    let mut parameters = vec![0.0, 1.0];
    for forced in forced_parameters {
        if forced.segment_index != segment_index {
            continue;
        }
        if !forced.parameter.is_finite() || forced.parameter < 0.0 || forced.parameter > 1.0 {
            return Err(NumericFreezeError {
                violation: NumericFreezeViolation::InvalidStation,
                field: "stationMeters",
                span,
            });
        }
        parameters.push(canonical_zero(forced.parameter));
    }
    parameters.sort_unstable_by(f64::total_cmp);
    parameters.dedup_by(|left, right| left.to_bits() == right.to_bits());
    Ok(parameters)
}

fn crop_cubic(
    controls: [Point3; 4],
    t0: f64,
    t1: f64,
    span: ByteSpan,
) -> Result<[Point3; 4], NumericFreezeError> {
    if !t0.is_finite() || !t1.is_finite() || t0 < 0.0 || t0 >= t1 || t1 > 1.0 {
        return Err(NumericFreezeError {
            violation: NumericFreezeViolation::InvalidStation,
            field: "stationMeters",
            span,
        });
    }
    let retained_left = if t1 < 1.0 {
        split_cubic(controls, t1)?.0
    } else {
        controls
    };
    if t0 == 0.0 {
        return Ok(retained_left);
    }
    let local_t0 = if t1 < 1.0 { t0 / t1 } else { t0 };
    Ok(split_cubic(retained_left, local_t0)?.1)
}

fn split_cubic(
    points: [Point3; 4],
    parameter: f64,
) -> Result<([Point3; 4], [Point3; 4]), NumericFreezeError> {
    let p01 = points[0].lerp(points[1], parameter);
    let p12 = points[1].lerp(points[2], parameter);
    let p23 = points[2].lerp(points[3], parameter);
    let p012 = p01.lerp(p12, parameter);
    let p123 = p12.lerp(p23, parameter);
    let middle = p012.lerp(p123, parameter);
    if [p01, p12, p23, p012, p123, middle]
        .iter()
        .any(|point| !point.is_finite())
    {
        return Err(arithmetic_error());
    }
    Ok((
        [points[0], p01, p012, middle],
        [middle, p123, p23, points[3]],
    ))
}

fn evaluate_segment_offset(
    start: Point3,
    segment: &ParsedCurveSegment,
    parameter: f64,
    offset_meters: f64,
) -> Result<OffsetEvaluation, NumericFreezeError> {
    match segment {
        ParsedCurveSegment::Line { end, span } => evaluate_line_offset(
            start,
            parse_vec3(end, "referenceLine.segments.end")?,
            parameter,
            offset_meters,
            *span,
        ),
        ParsedCurveSegment::CubicBezier { span, .. } => evaluate_offset(
            cubic_controls(start, segment)?,
            parameter,
            offset_meters,
            *span,
        ),
    }
}

fn evaluate_line_offset(
    start: Point3,
    end: Point3,
    parameter: f64,
    offset_meters: f64,
    span: ByteSpan,
) -> Result<OffsetEvaluation, NumericFreezeError> {
    let tangent = end.subtract(start);
    validate_tangent(tangent, span)?;
    let left_bits = horizontal_left(tangent).map_err(|mut error| {
        error.span = span;
        error
    })?;
    let left = Point3 {
        x: f64::from_bits(left_bits[0]),
        y: f64::from_bits(left_bits[1]),
        z: f64::from_bits(left_bits[2]),
    };
    let position = start
        .lerp(end, parameter)
        .add(left.scale(offset_meters))
        .canonicalized_zero();
    if !position.is_finite() {
        return Err(NumericFreezeError {
            violation: NumericFreezeViolation::ArithmeticOverflow,
            field: "offsetCurve",
            span,
        });
    }
    Ok(OffsetEvaluation { position, tangent })
}

#[derive(Clone, Copy)]
struct OffsetSubdivisionSpec {
    segment_index: u32,
    parameter_start: f64,
    parameter_end: f64,
    offset_meters: f64,
    accuracy_profile: GeometryAccuracyProfile,
    direction_profile: GeometryDirectionProfile,
    span: ByteSpan,
    max_point_count: u64,
}

fn subdivide_offset_cubic(
    controls: [Point3; 4],
    spec: OffsetSubdivisionSpec,
    output: &mut Vec<FrozenOffsetSample>,
    meter: &mut StageScratchMeter,
) -> Result<(), NumericFreezeError> {
    meter
        .grow(size_of::<SubdivisionFrame>() as u64)
        .map_err(|_| scratch_exceeded(spec.span))?;
    let mut stack: Vec<SubdivisionFrame> =
        vec![(controls, spec.parameter_start, spec.parameter_end, 0_u8)];
    while let Some((candidate, t0, t1, depth)) = stack.pop() {
        meter.shrink(size_of::<SubdivisionFrame>() as u64);
        if offset_cubic_is_acceptable(
            candidate,
            spec.offset_meters,
            spec.accuracy_profile,
            spec.direction_profile,
            spec.span,
        )? {
            let end = evaluate_offset(candidate, 1.0, spec.offset_meters, spec.span)?;
            push_offset_sample(
                output,
                FrozenOffsetSample {
                    segment_index: spec.segment_index,
                    parameter: t1,
                    point: end.position.into_array(),
                },
                spec.max_point_count,
                spec.span,
                meter,
            )?;
            continue;
        }
        if depth == MAX_SUBDIVISION_DEPTH {
            return Err(NumericFreezeError {
                violation: NumericFreezeViolation::SubdivisionDepthExceeded,
                field: "offsetCurve",
                span: spec.span,
            });
        }
        let (left, right) = split_half(candidate).map_err(|mut error| {
            error.span = spec.span;
            error
        })?;
        let midpoint = t0 + ((t1 - t0) * 0.5);
        let next_depth = depth + 1;
        meter
            .grow(size_of::<SubdivisionFrame>() as u64)
            .map_err(|_| scratch_exceeded(spec.span))?;
        stack.push((right, midpoint, t1, next_depth));
        meter
            .grow(size_of::<SubdivisionFrame>() as u64)
            .map_err(|_| scratch_exceeded(spec.span))?;
        stack.push((left, t0, midpoint, next_depth));
    }
    Ok(())
}

fn offset_cubic_is_acceptable(
    controls: [Point3; 4],
    offset_meters: f64,
    accuracy_profile: GeometryAccuracyProfile,
    direction_profile: GeometryDirectionProfile,
    span: ByteSpan,
) -> Result<bool, NumericFreezeError> {
    let Some(bound) = offset_curvature_bound(controls, offset_meters)? else {
        return Ok(false);
    };
    if bound > 8.0 * accuracy_profile.subdivision_budget_meters() {
        return Ok(false);
    }
    let start = evaluate_offset(controls, 0.0, offset_meters, span)?;
    let end = evaluate_offset(controls, 1.0, offset_meters, span)?;
    let chord = end.position.subtract(start.position);
    let threshold = direction_profile.candidate_cos_squared();
    Ok(
        direction_ok(start.tangent, chord, threshold).map_err(|mut error| {
            error.span = span;
            error
        })? && direction_ok(end.tangent, chord, threshold).map_err(|mut error| {
            error.span = span;
            error
        })?,
    )
}

#[derive(Clone, Copy)]
struct OffsetEvaluation {
    position: Point3,
    tangent: Point3,
}

fn evaluate_offset(
    controls: [Point3; 4],
    parameter: f64,
    offset_meters: f64,
    span: ByteSpan,
) -> Result<OffsetEvaluation, NumericFreezeError> {
    let position = evaluate_cubic(controls, parameter);
    let (first, second) = cubic_derivatives(controls, parameter);
    let q_squared = (first.x * first.x) + (first.z * first.z);
    let q_length = q_squared.sqrt();
    if q_squared == 0.0 || !q_squared.is_finite() || !q_length.is_finite() {
        return Err(NumericFreezeError {
            violation: NumericFreezeViolation::DegenerateTangent,
            field: "offsetCurve",
            span,
        });
    }
    let radial_derivative = ((first.x * second.x) + (first.z * second.z)) / q_length;
    let left = Point3 {
        x: first.z / q_length,
        y: 0.0,
        z: -first.x / q_length,
    };
    let left_derivative = Point3 {
        x: ((second.z * q_length) - (first.z * radial_derivative)) / q_squared,
        y: 0.0,
        z: ((-second.x * q_length) + (first.x * radial_derivative)) / q_squared,
    };
    let offset_position = position.add(left.scale(offset_meters));
    let offset_tangent = first.add(left_derivative.scale(offset_meters));
    if !offset_position.is_finite() || !offset_tangent.is_finite() {
        return Err(NumericFreezeError {
            violation: NumericFreezeViolation::ArithmeticOverflow,
            field: "offsetCurve",
            span,
        });
    }
    validate_tangent(offset_tangent, span)?;
    Ok(OffsetEvaluation {
        position: offset_position.canonicalized_zero(),
        tangent: offset_tangent.canonicalized_zero(),
    })
}

fn offset_curvature_bound(
    controls: [Point3; 4],
    offset_meters: f64,
) -> Result<Option<f64>, NumericFreezeError> {
    let v0 = controls[1].subtract(controls[0]).scale(3.0);
    let v1 = controls[2].subtract(controls[1]).scale(3.0);
    let v2 = controls[3].subtract(controls[2]).scale(3.0);
    let rx = distance_zero_to_range(v0.x.min(v1.x).min(v2.x), v0.x.max(v1.x).max(v2.x));
    let rz = distance_zero_to_range(v0.z.min(v1.z).min(v2.z), v0.z.max(v1.z).max(v2.z));
    let r_min = ((rx * rx) + (rz * rz)).sqrt();
    if !r_min.is_finite() {
        return Err(arithmetic_error());
    }
    if r_min == 0.0 {
        return Ok(None);
    }

    let a0 = controls[2]
        .subtract(controls[1].scale(2.0))
        .add(controls[0])
        .scale(6.0);
    let a1 = controls[3]
        .subtract(controls[2].scale(2.0))
        .add(controls[1])
        .scale(6.0);
    let jerk = controls[3]
        .subtract(controls[2].scale(3.0))
        .add(controls[1].scale(3.0))
        .subtract(controls[0])
        .scale(6.0);
    let acceleration_bound = a0.norm3().max(a1.norm3());
    let horizontal_acceleration_bound = a0.norm_xz().max(a1.norm_xz());
    let horizontal_jerk_bound = jerk.norm_xz();
    let inverse_square = r_min * r_min;
    let left_second_bound = ((2.0 * horizontal_jerk_bound) / r_min)
        + ((6.0 * horizontal_acceleration_bound * horizontal_acceleration_bound) / inverse_square);
    let bound = acceleration_bound + (offset_meters.abs() * left_second_bound);
    if !bound.is_finite() {
        return Err(arithmetic_error());
    }
    Ok(Some(bound))
}

fn cubic_controls(
    start: Point3,
    segment: &ParsedCurveSegment,
) -> Result<[Point3; 4], NumericFreezeError> {
    match segment {
        ParsedCurveSegment::Line { .. } => unreachable!("caller only requests cubic controls"),
        ParsedCurveSegment::CubicBezier { controls, end, .. } => Ok([
            start,
            parse_vec3(&controls[0], "referenceLine.segments.controls")?,
            parse_vec3(&controls[1], "referenceLine.segments.controls")?,
            parse_vec3(end, "referenceLine.segments.end")?,
        ]),
    }
}

fn evaluate_cubic(controls: [Point3; 4], parameter: f64) -> Point3 {
    let p01 = controls[0].lerp(controls[1], parameter);
    let p12 = controls[1].lerp(controls[2], parameter);
    let p23 = controls[2].lerp(controls[3], parameter);
    let p012 = p01.lerp(p12, parameter);
    let p123 = p12.lerp(p23, parameter);
    p012.lerp(p123, parameter)
}

fn cubic_derivatives(controls: [Point3; 4], parameter: f64) -> (Point3, Point3) {
    let u = 1.0 - parameter;
    let d10 = controls[1].subtract(controls[0]);
    let d21 = controls[2].subtract(controls[1]);
    let d32 = controls[3].subtract(controls[2]);
    let first = d10
        .scale(u * u)
        .add(d21.scale((2.0 * u) * parameter))
        .add(d32.scale(parameter * parameter))
        .scale(3.0);
    let a0 = controls[2]
        .subtract(controls[1].scale(2.0))
        .add(controls[0]);
    let a1 = controls[3]
        .subtract(controls[2].scale(2.0))
        .add(controls[1]);
    let second = a0.scale(u).add(a1.scale(parameter)).scale(6.0);
    (first, second)
}

fn distance_zero_to_range(minimum: f64, maximum: f64) -> f64 {
    if minimum <= 0.0 && maximum >= 0.0 {
        0.0
    } else {
        minimum.abs().min(maximum.abs())
    }
}

fn subdivide_cubic(
    controls: [Point3; 4],
    segment_index: u32,
    accuracy_profile: GeometryAccuracyProfile,
    direction_profile: GeometryDirectionProfile,
    span: ByteSpan,
    output: &mut Vec<FrozenReferenceSample>,
    meter: &mut StageScratchMeter,
) -> Result<(), NumericFreezeError> {
    meter
        .grow(size_of::<SubdivisionFrame>() as u64)
        .map_err(|_| scratch_exceeded(span))?;
    let mut stack: Vec<SubdivisionFrame> = vec![(controls, 0.0_f64, 1.0_f64, 0_u8)];
    while let Some((candidate, t0, t1, depth)) = stack.pop() {
        meter.shrink(size_of::<SubdivisionFrame>() as u64);
        if cubic_is_acceptable(candidate, accuracy_profile, direction_profile).map_err(
            |mut error| {
                error.span = span;
                error
            },
        )? {
            meter
                .grow(size_of::<FrozenReferenceSample>() as u64)
                .map_err(|_| scratch_exceeded(span))?;
            output.push(FrozenReferenceSample {
                segment_index,
                parameter: t1,
                point: candidate[3].into_array(),
            });
            continue;
        }
        if depth == MAX_SUBDIVISION_DEPTH {
            return Err(NumericFreezeError {
                violation: NumericFreezeViolation::SubdivisionDepthExceeded,
                field: "referenceLine.segments",
                span,
            });
        }
        let (left, right) = split_half(candidate)?;
        let midpoint = t0 + ((t1 - t0) * 0.5);
        let next_depth = depth + 1;
        meter
            .grow(size_of::<SubdivisionFrame>() as u64)
            .map_err(|_| scratch_exceeded(span))?;
        stack.push((right, midpoint, t1, next_depth));
        meter
            .grow(size_of::<SubdivisionFrame>() as u64)
            .map_err(|_| scratch_exceeded(span))?;
        stack.push((left, t0, midpoint, next_depth));
    }
    Ok(())
}

fn cubic_is_acceptable(
    controls: [Point3; 4],
    accuracy_profile: GeometryAccuracyProfile,
    direction_profile: GeometryDirectionProfile,
) -> Result<bool, NumericFreezeError> {
    let chord = controls[3].subtract(controls[0]);
    validate_tangent(chord, ByteSpan { start: 0, end: 0 })?;
    let budget = accuracy_profile.subdivision_budget_meters();
    let budget_squared = budget * budget;
    let position_ok = point_segment_distance_squared(controls[1], controls[0], controls[3])?
        <= budget_squared
        && point_segment_distance_squared(controls[2], controls[0], controls[3])? <= budget_squared;
    if !position_ok {
        return Ok(false);
    }
    let start_tangent = controls[1].subtract(controls[0]);
    let end_tangent = controls[3].subtract(controls[2]);
    Ok(direction_ok(
        start_tangent,
        chord,
        direction_profile.candidate_cos_squared(),
    )? && direction_ok(
        end_tangent,
        chord,
        direction_profile.candidate_cos_squared(),
    )?)
}

fn validate_source_join(
    left: Point3,
    right: Point3,
    profile: GeometryDirectionProfile,
    span: ByteSpan,
) -> Result<(), NumericFreezeError> {
    let direction_matches =
        direction_ok(left, right, profile.runtime_cos_squared()).map_err(|mut error| {
            error.span = span;
            error
        })?;
    let left_direction = horizontal_left(left).map_err(|mut error| {
        error.span = span;
        error
    })?;
    let right_direction = horizontal_left(right).map_err(|mut error| {
        error.span = span;
        error
    })?;
    if !direction_matches || left_direction != right_direction {
        return Err(NumericFreezeError {
            violation: NumericFreezeViolation::DiscontinuousSourceJoin,
            field: "referenceLine.segments",
            span,
        });
    }
    Ok(())
}

fn direction_ok(left: Point3, right: Point3, threshold: f64) -> Result<bool, NumericFreezeError> {
    validate_tangent(left, ByteSpan { start: 0, end: 0 })?;
    validate_tangent(right, ByteSpan { start: 0, end: 0 })?;
    let dot = left.dot(right);
    let left_length_squared = left.length_squared();
    let right_length_squared = right.length_squared();
    let bound = (left_length_squared * right_length_squared) * threshold;
    if !dot.is_finite() || !bound.is_finite() {
        return Err(arithmetic_error());
    }
    Ok(dot > 0.0 && (dot * dot) >= bound)
}

fn horizontal_left(tangent: Point3) -> Result<[u64; 3], NumericFreezeError> {
    let length_squared = (tangent.x * tangent.x) + (tangent.z * tangent.z);
    let length = length_squared.sqrt();
    if !length.is_finite() || length == 0.0 {
        return Err(NumericFreezeError {
            violation: NumericFreezeViolation::DegenerateTangent,
            field: "referenceLine.segments",
            span: ByteSpan { start: 0, end: 0 },
        });
    }
    let left = Point3 {
        x: canonical_zero(tangent.z / length),
        y: 0.0,
        z: canonical_zero(-tangent.x / length),
    };
    Ok([left.x.to_bits(), left.y.to_bits(), left.z.to_bits()])
}

fn validate_tangent(tangent: Point3, span: ByteSpan) -> Result<(), NumericFreezeError> {
    let length_squared = tangent.length_squared();
    if !length_squared.is_finite() {
        return Err(arithmetic_error());
    }
    if length_squared == 0.0 || ((tangent.x * tangent.x) + (tangent.z * tangent.z)) == 0.0 {
        return Err(NumericFreezeError {
            violation: NumericFreezeViolation::DegenerateTangent,
            field: "referenceLine.segments",
            span,
        });
    }
    Ok(())
}

fn point_segment_distance_squared(
    point: Point3,
    start: Point3,
    end: Point3,
) -> Result<f64, NumericFreezeError> {
    let chord = end.subtract(start);
    let chord_squared = chord.length_squared();
    if !chord_squared.is_finite() || chord_squared == 0.0 {
        return Err(arithmetic_error());
    }
    let projection = point.subtract(start).dot(chord) / chord_squared;
    let clamped = projection.clamp(0.0, 1.0);
    let nearest = start.add(chord.scale(clamped));
    let distance_squared = point.subtract(nearest).length_squared();
    if !distance_squared.is_finite() {
        return Err(arithmetic_error());
    }
    Ok(distance_squared)
}

fn split_half(points: [Point3; 4]) -> Result<([Point3; 4], [Point3; 4]), NumericFreezeError> {
    let p01 = points[0].lerp_half(points[1]);
    let p12 = points[1].lerp_half(points[2]);
    let p23 = points[2].lerp_half(points[3]);
    let p012 = p01.lerp_half(p12);
    let p123 = p12.lerp_half(p23);
    let middle = p012.lerp_half(p123);
    if [p01, p12, p23, p012, p123, middle]
        .iter()
        .any(|point| !point.is_finite())
    {
        return Err(arithmetic_error());
    }
    Ok((
        [points[0], p01, p012, middle],
        [middle, p123, p23, points[3]],
    ))
}

fn parse_vec3(value: &ParsedVec3, field: &'static str) -> Result<Point3, NumericFreezeError> {
    Ok(Point3 {
        x: parse_finite(&value.components[0], field)?,
        y: parse_finite(&value.components[1], field)?,
        z: parse_finite(&value.components[2], field)?,
    })
}

pub(in crate::module::geometry) fn parse_finite(
    value: &RawNumber,
    field: &'static str,
) -> Result<f64, NumericFreezeError> {
    let parsed = value.token.parse::<f64>().map_err(|_| NumericFreezeError {
        violation: NumericFreezeViolation::InvalidNumber,
        field,
        span: value.span,
    })?;
    if !parsed.is_finite() {
        return Err(NumericFreezeError {
            violation: NumericFreezeViolation::NonFiniteNumber,
            field,
            span: value.span,
        });
    }
    Ok(canonical_zero(parsed))
}

const fn canonical_zero(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

/// 按 §6.1 的固定顺序从冻结 `f32` 折线派生弧长：逐对精确提升为 `f64`，
/// 计算欧氏弦长并从左到右累加；所有降阶长度必须复用本约定。
pub(in crate::module::geometry) fn frozen_polyline_length_meters(
    points: &[FrozenCanonicalPoint],
) -> f64 {
    points.windows(2).fold(0.0_f64, |total, pair| {
        let dx = f64::from(pair[1].x) - f64::from(pair[0].x);
        let dy = f64::from(pair[1].y) - f64::from(pair[0].y);
        let dz = f64::from(pair[1].z) - f64::from(pair[0].z);
        total + ((dx * dx + dy * dy) + dz * dz).sqrt()
    })
}

const fn arithmetic_error() -> NumericFreezeError {
    NumericFreezeError {
        violation: NumericFreezeViolation::ArithmeticOverflow,
        field: "referenceLine.segments",
        span: ByteSpan { start: 0, end: 0 },
    }
}

impl Point3 {
    const fn from_array(value: [f64; 3]) -> Self {
        Self {
            x: value[0],
            y: value[1],
            z: value[2],
        }
    }

    const fn into_array(self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }

    fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    fn subtract(self, right: Self) -> Self {
        Self {
            x: self.x - right.x,
            y: self.y - right.y,
            z: self.z - right.z,
        }
    }

    fn add(self, right: Self) -> Self {
        Self {
            x: self.x + right.x,
            y: self.y + right.y,
            z: self.z + right.z,
        }
    }

    fn scale(self, scalar: f64) -> Self {
        Self {
            x: scalar * self.x,
            y: scalar * self.y,
            z: scalar * self.z,
        }
    }

    fn dot(self, right: Self) -> f64 {
        ((self.x * right.x) + (self.y * right.y)) + (self.z * right.z)
    }

    fn length_squared(self) -> f64 {
        self.dot(self)
    }

    fn lerp_half(self, right: Self) -> Self {
        self.lerp(right, 0.5)
    }

    fn lerp(self, right: Self, parameter: f64) -> Self {
        Self {
            x: self.x + (parameter * (right.x - self.x)),
            y: self.y + (parameter * (right.y - self.y)),
            z: self.z + (parameter * (right.z - self.z)),
        }
    }

    fn norm3(self) -> f64 {
        self.length_squared().sqrt()
    }

    fn norm_xz(self) -> f64 {
        ((self.x * self.x) + (self.z * self.z)).sqrt()
    }

    fn canonicalized_zero(self) -> Self {
        Self {
            x: canonical_zero(self.x),
            y: canonical_zero(self.y),
            z: canonical_zero(self.z),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::geometry::schema::parse_geometry_document;
    use crate::module::geometry::schema::road::ParsedEndStation;

    #[test]
    fn line_reference_freezes_exact_endpoints_and_negative_zero() {
        let document = parse_geometry_document(super::super::MINIMAL_DOCUMENT).unwrap();
        let roads = freeze_reference_lines(
            &document,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            &mut unlimited_meter(),
        )
        .unwrap();

        assert_eq!(roads.len(), 1);
        assert_eq!(roads[0].road_key.as_ref(), "road.main");
        assert_eq!(roads[0].samples.len(), 2);
        assert_eq!(roads[0].samples[0].point, [0.0, 0.0, 0.0]);
        assert_eq!(roads[0].samples[1].point, [10.0, 0.0, 0.0]);
    }

    #[test]
    fn cubic_subdivision_is_left_first_and_profile_monotone() {
        let curve = ParsedCurve {
            start: vec3(["0", "0", "0"]),
            segments: vec![ParsedCurveSegment::CubicBezier {
                controls: Box::new([vec3(["3", "0", "0"]), vec3(["7", "0", "3"])]),
                end: vec3(["10", "0", "3"]),
                span: test_span(),
            }]
            .into_boxed_slice(),
            span: test_span(),
        };
        let fine = freeze_curve(
            &curve,
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
            GeometryDirectionProfile::Smooth1Deg,
            &mut unlimited_meter(),
        )
        .unwrap();
        let compact = freeze_curve(
            &curve,
            GeometryAccuracyProfile::Compact10Cm,
            GeometryDirectionProfile::Compact5Deg,
            GeometryDirectionProfile::Compact5Deg,
            &mut unlimited_meter(),
        )
        .unwrap();

        assert!(fine.len() >= compact.len());
        assert!(
            fine.windows(2)
                .all(|pair| pair[0].parameter < pair[1].parameter)
        );
        assert_eq!(fine.first().unwrap().parameter, 0.0);
        assert_eq!(fine.last().unwrap().parameter, 1.0);
    }

    #[test]
    fn source_join_rejects_different_horizontal_left_bits() {
        let curve = ParsedCurve {
            start: vec3(["0", "0", "0"]),
            segments: vec![
                ParsedCurveSegment::Line {
                    end: vec3(["1", "0", "0"]),
                    span: test_span(),
                },
                ParsedCurveSegment::Line {
                    end: vec3(["2", "0", "0.01"]),
                    span: test_span(),
                },
            ]
            .into_boxed_slice(),
            span: test_span(),
        };
        let error = freeze_curve(
            &curve,
            GeometryAccuracyProfile::Compact10Cm,
            GeometryDirectionProfile::Compact5Deg,
            GeometryDirectionProfile::Compact5Deg,
            &mut unlimited_meter(),
        )
        .unwrap_err();

        assert_eq!(
            error.violation,
            NumericFreezeViolation::DiscontinuousSourceJoin
        );
    }

    /// §6.1：来源 join 的三维夹角门禁使用调用方所选方向档，而非细分档；同方位
    /// 纯俯仰变化（两侧规范化 `left` bits 相同）在更宽档下必须被接受。
    #[test]
    fn source_join_pitch_gate_uses_join_profile() {
        // 切向 (10,0,0) → (8,8·tan(1.5°),0)：三维夹角 1.5°，XZ 方位一致，
        // 两侧规范化 `left` 均为 (0,0,-1) 且 bits 相同。
        let curve = ParsedCurve {
            start: vec3(["0", "0", "0"]),
            segments: vec![
                ParsedCurveSegment::Line {
                    end: vec3(["10", "0", "0"]),
                    span: test_span(),
                },
                ParsedCurveSegment::Line {
                    end: vec3(["18", "0.20948737255349544", "0"]),
                    span: test_span(),
                },
            ]
            .into_boxed_slice(),
            span: test_span(),
        };
        // 细分档固定 Smooth1Deg 时，join 档 Balanced2Deg 必须接受 1.5° 俯仰 join。
        freeze_curve(
            &curve,
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
            GeometryDirectionProfile::Balanced2Deg,
            &mut unlimited_meter(),
        )
        .unwrap();
        // 同一曲线 join 档 Smooth1Deg 必须失败关闭。
        let error = freeze_curve(
            &curve,
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
            GeometryDirectionProfile::Smooth1Deg,
            &mut unlimited_meter(),
        )
        .expect_err("join 档 Smooth1Deg 必须拒绝 1.5° 俯仰 join");
        assert_eq!(
            error.violation,
            NumericFreezeViolation::DiscontinuousSourceJoin
        );
    }

    #[test]
    fn source_join_pitch_gate_compact_accepts_wider_angle() {
        // 3.5° 俯仰 join：join 档 Balanced2Deg 拒绝、Compact5Deg 接受。
        let curve = ParsedCurve {
            start: vec3(["0", "0", "0"]),
            segments: vec![
                ParsedCurveSegment::Line {
                    end: vec3(["10", "0", "0"]),
                    span: test_span(),
                },
                ParsedCurveSegment::Line {
                    end: vec3(["18", "0.4893009602177391", "0"]),
                    span: test_span(),
                },
            ]
            .into_boxed_slice(),
            span: test_span(),
        };
        let error = freeze_curve(
            &curve,
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
            GeometryDirectionProfile::Balanced2Deg,
            &mut unlimited_meter(),
        )
        .expect_err("join 档 Balanced2Deg 必须拒绝 3.5° 俯仰 join");
        assert_eq!(
            error.violation,
            NumericFreezeViolation::DiscontinuousSourceJoin
        );
        freeze_curve(
            &curve,
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
            GeometryDirectionProfile::Compact5Deg,
            &mut unlimited_meter(),
        )
        .unwrap();
    }

    /// §6.1：station 表细分保持固定档（每 line segment 一个区间），但 join
    /// 门禁按所选方向档裁决；payload 路径把所选方向档传入 station freeze。
    #[test]
    fn station_and_payload_join_gate_use_selected_direction_profile() {
        let minimal = std::str::from_utf8(super::super::MINIMAL_DOCUMENT).unwrap();
        let source = minimal.replace(
            r#""segments":[{"kind":"line","end":[10,0,0]}]"#,
            r#""segments":[{"kind":"line","end":[10,0,0]},{"kind":"line","end":[18,0.20948737255349544,0]}]"#,
        );
        assert_ne!(source, minimal);
        let mut document = parse_geometry_document(source.as_bytes()).unwrap();
        // 细分仍按固定档：两条 line segment → 每 segment 恰好一个 station 区间。
        let roads = freeze_stationing(
            &document,
            GeometryDirectionProfile::Balanced2Deg,
            &mut unlimited_meter(),
        )
        .unwrap();
        assert_eq!(roads[0].intervals.len(), 2);
        let error = freeze_stationing(
            &document,
            GeometryDirectionProfile::Smooth1Deg,
            &mut unlimited_meter(),
        )
        .expect_err("station join 档 Smooth1Deg 必须拒绝 1.5° 俯仰 join");
        assert_eq!(
            error.violation,
            NumericFreezeViolation::DiscontinuousSourceJoin
        );
        // payload 路径以所选方向档裁决同一 join。
        let error = freeze_geometry_payload(
            &mut document,
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
            u64::MAX,
            &mut unlimited_meter(),
        )
        .expect_err("payload 路径必须把所选方向档传入 station join 门禁");
        assert_eq!(
            error.violation,
            NumericFreezeViolation::DiscontinuousSourceJoin
        );
        freeze_geometry_payload(
            &mut document,
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Balanced2Deg,
            u64::MAX,
            &mut unlimited_meter(),
        )
        .unwrap();
    }

    #[test]
    fn station_lower_bound_assigns_exact_boundary_to_preceding_segment() {
        let intervals = [
            StationInterval {
                segment_index: 0,
                t0_bits: 0.0_f64.to_bits(),
                t1_bits: 1.0_f64.to_bits(),
                cumulative_start_length_bits: 0.0_f64.to_bits(),
                cumulative_end_length_bits: 10.0_f64.to_bits(),
            },
            StationInterval {
                segment_index: 1,
                t0_bits: 0.0_f64.to_bits(),
                t1_bits: 1.0_f64.to_bits(),
                cumulative_start_length_bits: 10.0_f64.to_bits(),
                cumulative_end_length_bits: 20.0_f64.to_bits(),
            },
        ];

        assert_eq!(
            locate_station(&intervals, 0.0, test_span()).unwrap(),
            FrozenCurveParameter {
                segment_index: 0,
                parameter: 0.0,
            }
        );
        assert_eq!(
            locate_station(&intervals, 10.0, test_span()).unwrap(),
            FrozenCurveParameter {
                segment_index: 0,
                parameter: 1.0,
            }
        );
        assert_eq!(
            locate_station(&intervals, 15.0, test_span()).unwrap(),
            FrozenCurveParameter {
                segment_index: 1,
                parameter: 0.5,
            }
        );
    }

    #[test]
    fn station_table_is_fixed_fine_smooth_and_bit_contiguous() {
        let document = parse_geometry_document(super::super::MINIMAL_DOCUMENT).unwrap();
        let roads = freeze_stationing(
            &document,
            GeometryDirectionProfile::Smooth1Deg,
            &mut unlimited_meter(),
        )
        .unwrap();
        let road = &roads[0];

        assert_eq!(road.road_key.as_ref(), "road.main");
        assert_eq!(road.intervals.len(), 1);
        assert_eq!(road.intervals[0].segment_index, 0);
        assert_eq!(road.intervals[0].t0_bits, 0.0_f64.to_bits());
        assert_eq!(road.intervals[0].t1_bits, 1.0_f64.to_bits());
        assert_eq!(
            road.intervals[0].cumulative_start_length_bits,
            0.0_f64.to_bits()
        );
        assert_eq!(
            road.intervals[0].cumulative_end_length_bits,
            10.0_f64.to_bits()
        );
    }

    #[test]
    fn span_station_coverage_accepts_exact_chain_and_rejects_gap() {
        let intervals = [StationInterval {
            segment_index: 0,
            t0_bits: 0.0_f64.to_bits(),
            t1_bits: 1.0_f64.to_bits(),
            cumulative_start_length_bits: 0.0_f64.to_bits(),
            cumulative_end_length_bits: 10.0_f64.to_bits(),
        }];
        let exact = [
            cross_section_span("0", ParsedEndStation::Number(raw("4"))),
            cross_section_span("4", ParsedEndStation::End(test_span())),
        ];
        let frozen = freeze_span_stations(&exact, &intervals, &mut unlimited_meter()).unwrap();
        assert_eq!(frozen[0].start.parameter, 0.0);
        assert_eq!(frozen[0].end.parameter, 0.4);
        assert_eq!(frozen[1].start.parameter, 0.4);
        assert_eq!(frozen[1].end.parameter, 1.0);

        let gap = [
            cross_section_span("0", ParsedEndStation::Number(raw("4"))),
            cross_section_span("5", ParsedEndStation::End(test_span())),
        ];
        assert_eq!(
            freeze_span_stations(&gap, &intervals, &mut unlimited_meter())
                .unwrap_err()
                .violation,
            NumericFreezeViolation::IncompleteStationCoverage
        );
    }

    #[test]
    fn straight_offset_uses_exact_horizontal_left_and_no_subdivision() {
        let curve = ParsedCurve {
            start: vec3(["0", "0", "0"]),
            segments: vec![ParsedCurveSegment::Line {
                end: vec3(["10", "0", "0"]),
                span: test_span(),
            }]
            .into_boxed_slice(),
            span: test_span(),
        };
        let samples = freeze_offset_curve(
            &curve,
            2.0,
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
            &mut unlimited_meter(),
        )
        .unwrap();

        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].point, [0.0, 0.0, -2.0]);
        assert_eq!(samples[1].point, [10.0, 0.0, -2.0]);
    }

    #[test]
    fn offset_k_bound_is_zero_for_collinear_cubic() {
        let controls = [
            Point3::from_array([0.0, 0.0, 0.0]),
            Point3::from_array([1.0, 0.0, 0.0]),
            Point3::from_array([2.0, 0.0, 0.0]),
            Point3::from_array([3.0, 0.0, 0.0]),
        ];

        assert_eq!(offset_curvature_bound(controls, 100.0).unwrap(), Some(0.0));
        let start = evaluate_offset(controls, 0.0, 2.0, test_span()).unwrap();
        let end = evaluate_offset(controls, 1.0, 2.0, test_span()).unwrap();
        assert_eq!(start.position.into_array(), [0.0, 0.0, -2.0]);
        assert_eq!(end.position.into_array(), [3.0, 0.0, -2.0]);
    }

    #[test]
    fn curved_offset_subdivision_is_left_first_and_profile_monotone() {
        let curve = ParsedCurve {
            start: vec3(["0", "0", "0"]),
            segments: vec![ParsedCurveSegment::CubicBezier {
                controls: Box::new([vec3(["3", "0", "0"]), vec3(["7", "0", "3"])]),
                end: vec3(["10", "0", "3"]),
                span: test_span(),
            }]
            .into_boxed_slice(),
            span: test_span(),
        };
        let fine = freeze_offset_curve(
            &curve,
            2.0,
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
            &mut unlimited_meter(),
        )
        .unwrap();
        let compact = freeze_offset_curve(
            &curve,
            2.0,
            GeometryAccuracyProfile::Compact10Cm,
            GeometryDirectionProfile::Compact5Deg,
            &mut unlimited_meter(),
        )
        .unwrap();

        assert!(fine.len() >= compact.len());
        assert!(
            fine.windows(2)
                .all(|pair| pair[0].parameter < pair[1].parameter)
        );
        assert_eq!(fine.first().unwrap().parameter, 0.0);
        assert_eq!(fine.last().unwrap().parameter, 1.0);
    }

    #[test]
    fn station_forced_parameters_are_preserved_exactly_after_crop() {
        let curve = ParsedCurve {
            start: vec3(["0", "0", "0"]),
            segments: vec![ParsedCurveSegment::CubicBezier {
                controls: Box::new([vec3(["3", "0", "0"]), vec3(["7", "0", "3"])]),
                end: vec3(["10", "0", "3"]),
                span: test_span(),
            }]
            .into_boxed_slice(),
            span: test_span(),
        };
        let forced = FrozenCurveParameter {
            segment_index: 0,
            parameter: 0.3,
        };
        let samples = freeze_offset_curve_with_forced(
            &curve,
            1.0,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            &[forced, forced],
            &mut unlimited_meter(),
        )
        .unwrap();

        assert!(samples.iter().any(|sample| {
            sample.segment_index == 0 && sample.parameter.to_bits() == forced.parameter.to_bits()
        }));
        assert_eq!(
            samples
                .iter()
                .filter(|sample| sample.parameter.to_bits() == forced.parameter.to_bits())
                .count(),
            1
        );
        let forced_sample = samples
            .iter()
            .find(|sample| sample.parameter.to_bits() == forced.parameter.to_bits())
            .unwrap();
        assert_eq!(
            forced_sample.point,
            evaluate_segment_offset(
                Point3::from_array([0.0, 0.0, 0.0]),
                &curve.segments[0],
                forced.parameter,
                1.0,
            )
            .unwrap()
            .position
            .into_array()
        );
        assert!(
            samples
                .windows(2)
                .all(|pair| pair[0].parameter < pair[1].parameter)
        );
    }

    #[test]
    fn lateral_prefix_sum_anchors_reference_lane_and_preserves_left_to_right_order() {
        let pending = vec![
            pending("left", 2.0),
            pending("reference", 4.0),
            pending("right", 6.0),
        ];
        let frozen =
            compute_lateral_offsets(pending, 1, test_span(), &mut unlimited_meter()).unwrap();

        assert_eq!(frozen[0].center_offset_meters, 3.0);
        assert_eq!(frozen[0].left_boundary_offset_meters, 4.0);
        assert_eq!(frozen[0].right_boundary_offset_meters, 2.0);
        assert_eq!(frozen[1].center_offset_meters, 0.0);
        assert_eq!(frozen[1].left_boundary_offset_meters, 2.0);
        assert_eq!(frozen[1].right_boundary_offset_meters, -2.0);
        assert_eq!(frozen[2].center_offset_meters, -5.0);
        assert_eq!(frozen[2].left_boundary_offset_meters, -2.0);
        assert_eq!(frozen[2].right_boundary_offset_meters, -8.0);
        assert!(frozen.windows(2).all(|pair| {
            pair[0].right_boundary_offset_meters == pair[1].left_boundary_offset_meters
        }));
    }

    #[test]
    fn minimal_document_freezes_reference_lane_at_zero_offset() {
        let document = parse_geometry_document(super::super::MINIMAL_DOCUMENT).unwrap();
        let layouts = freeze_cross_section_layouts(&document, &mut unlimited_meter()).unwrap();

        assert_eq!(layouts.len(), 1);
        assert_eq!(layouts[0].span_key.as_ref(), "span.main");
        assert_eq!(layouts[0].items.len(), 1);
        assert_eq!(layouts[0].items[0].key.as_ref(), "lane.main");
        assert_eq!(layouts[0].items[0].kind, LateralIntentKind::ForwardLane);
        assert_eq!(layouts[0].items[0].center_offset_meters, 0.0);
    }

    #[test]
    fn cross_section_layout_normalizes_self_qualified_owner_local_references() {
        let minimal = std::str::from_utf8(super::super::MINIMAL_DOCUMENT).unwrap();
        let source = minimal
            .replace(
                r#""referenceSectionKey":"section.main","referenceLaneKey":"lane.main""#,
                r#""referenceSectionKey":"city/main::section.main","referenceLaneKey":"city/main::lane.main""#,
            )
            .replace(
                r#""elements":[{"kind":"roadSection","sectionKey":"section.main"}]"#,
                r#""elements":[{"kind":"roadSection","sectionKey":"city/main::section.main"},{"kind":"facilityBand","facilityBandKey":"city/main::facility.main"}]"#,
            )
            .replace(
                r#""facilityBands":[]"#,
                r#""facilityBands":[{"facilityBandKey":"facility.main","kindId":"sidewalk","widthMeters":2}]"#,
            );
        assert_ne!(source, minimal);
        let mut document = parse_geometry_document(source.as_bytes()).unwrap();
        normalize_owner_local_cross_section_references(&mut document, &mut unlimited_meter())
            .unwrap();
        let layouts = freeze_cross_section_layouts(&document, &mut unlimited_meter()).unwrap();

        assert_eq!(layouts[0].items.len(), 2);
        assert_eq!(layouts[0].items[0].key.as_ref(), "lane.main");
        assert_eq!(layouts[0].items[1].key.as_ref(), "facility.main");
    }

    #[test]
    fn cross_section_layout_rejects_foreign_qualified_owner_local_reference() {
        let minimal = std::str::from_utf8(super::super::MINIMAL_DOCUMENT).unwrap();
        let source = minimal.replace(
            r#""referenceSectionKey":"section.main""#,
            r#""referenceSectionKey":"city/other::section.main""#,
        );
        let mut document = parse_geometry_document(source.as_bytes()).unwrap();
        let error =
            normalize_owner_local_cross_section_references(&mut document, &mut unlimited_meter())
                .unwrap_err();

        assert_eq!(error.field, "referenceSectionKey");
        assert_eq!(
            error.violation,
            NumericFreezeViolation::InvalidCrossSectionReference
        );
    }

    #[test]
    fn lateral_curve_range_only_emits_owned_station_span() {
        let curve = ParsedCurve {
            start: vec3(["0", "0", "0"]),
            segments: vec![ParsedCurveSegment::Line {
                end: vec3(["10", "0", "0"]),
                span: test_span(),
            }]
            .into_boxed_slice(),
            span: test_span(),
        };
        let range = FrozenSpanStations {
            start: FrozenCurveParameter {
                segment_index: 0,
                parameter: 0.2,
            },
            end: FrozenCurveParameter {
                segment_index: 0,
                parameter: 0.6,
            },
        };
        let samples = freeze_offset_curve_range(
            &curve,
            1.0,
            range,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            u64::MAX,
            &mut unlimited_meter(),
        )
        .unwrap();
        let points = quantize_and_validate_curve(
            &curve,
            1.0,
            &samples,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            test_span(),
            &mut unlimited_meter(),
        )
        .unwrap();

        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].point, [2.0, 0.0, -1.0]);
        assert_eq!(samples[1].point, [6.0, 0.0, -1.0]);
        assert_eq!(points[0].x, 2.0);
        assert_eq!(points[1].x, 6.0);
    }

    #[test]
    fn lateral_curve_freeze_reverses_only_backward_lane_output() {
        let minimal = std::str::from_utf8(super::super::MINIMAL_DOCUMENT).unwrap();
        let source = minimal
            .replace(
                r#""elements":[{"kind":"roadSection","sectionKey":"section.main"}]"#,
                r#""elements":[{"kind":"roadSection","sectionKey":"section.main"},{"kind":"roadSection","sectionKey":"section.back"}]"#,
            )
            .replace(
                r#""roadSections":[{"sectionKey":"section.main","kindId":"road.vehicle","lanes":[{"laneKey":"lane.main","laneEdgeKey":"edge.main","direction":"forward","widthMeters":3.5,"speedLimitMetersPerSecond":10,"successors":[]}],"laneGroups":[]}]"#,
                r#""roadSections":[{"sectionKey":"section.main","kindId":"road.vehicle","lanes":[{"laneKey":"lane.main","laneEdgeKey":"edge.main","direction":"forward","widthMeters":3.5,"speedLimitMetersPerSecond":10,"successors":[]}],"laneGroups":[]},{"sectionKey":"section.back","kindId":"road.vehicle","lanes":[{"laneKey":"lane.back","laneEdgeKey":"edge.back","direction":"backward","widthMeters":3.5,"speedLimitMetersPerSecond":10,"successors":[]}],"laneGroups":[]}]"#,
            );
        assert_ne!(source, minimal);
        let document = parse_geometry_document(source.as_bytes()).unwrap();
        let stationing = freeze_stationing(
            &document,
            GeometryDirectionProfile::Balanced2Deg,
            &mut unlimited_meter(),
        )
        .unwrap();
        let layouts = freeze_cross_section_layouts(&document, &mut unlimited_meter()).unwrap();
        let curves = freeze_lateral_curves(
            &document,
            &stationing,
            &layouts,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            &mut unlimited_meter(),
        )
        .unwrap();

        assert_eq!(curves.len(), 2);
        assert_eq!(curves[0].kind, LateralIntentKind::ForwardLane);
        assert_eq!(curves[0].points[0].x, 0.0);
        assert_eq!(curves[0].points[1].x, 10.0);
        assert_eq!(curves[1].kind, LateralIntentKind::BackwardLane);
        assert_eq!(curves[1].points[0].x, 10.0);
        assert_eq!(curves[1].points[1].x, 0.0);
    }

    #[test]
    fn quantized_curve_rejects_existing_minimum_segment_boundary() {
        let points = [
            FrozenCanonicalPoint {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            FrozenCanonicalPoint {
                x: SPATIAL_MIN_SEGMENT_LENGTH_METERS,
                y: 0.0,
                z: 0.0,
            },
        ];
        assert_eq!(
            validate_quantized_segments(
                &points,
                GeometryDirectionProfile::Balanced2Deg,
                test_span(),
            )
            .unwrap_err()
            .violation,
            NumericFreezeViolation::QuantizedSegmentTooShort
        );
    }

    #[test]
    fn scratch_meter_fails_closed_on_small_limit_and_passes_when_unlimited() {
        // 单条 line segment：首元素 + line push 各记入 size_of::<FrozenReferenceSample>()
        // （40B）；限 64 时第二次 push 越限失败关闭，unlimited 账簿照常通过。
        let curve = ParsedCurve {
            start: vec3(["0", "0", "0"]),
            segments: vec![ParsedCurveSegment::Line {
                end: vec3(["10", "0", "0"]),
                span: test_span(),
            }]
            .into_boxed_slice(),
            span: test_span(),
        };
        let error = freeze_curve(
            &curve,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            GeometryDirectionProfile::Balanced2Deg,
            &mut StageScratchMeter::new(64),
        )
        .unwrap_err();
        assert_eq!(
            error.violation,
            NumericFreezeViolation::StageScratchExceeded
        );
        assert_eq!(error.field, "stageScratchBytes");

        let samples = freeze_curve(
            &curve,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            GeometryDirectionProfile::Balanced2Deg,
            &mut unlimited_meter(),
        )
        .unwrap();
        assert_eq!(samples.len(), 2);
    }

    #[test]
    fn cubic_subdivision_accounts_stack_frames_in_meter_peak() {
        // cubic 细分的显式栈帧计入账簿：无上限时峰值至少覆盖一个栈帧。
        let curve = ParsedCurve {
            start: vec3(["0", "0", "0"]),
            segments: vec![ParsedCurveSegment::CubicBezier {
                controls: Box::new([vec3(["3", "0", "0"]), vec3(["7", "0", "3"])]),
                end: vec3(["10", "0", "3"]),
                span: test_span(),
            }]
            .into_boxed_slice(),
            span: test_span(),
        };
        let mut meter = unlimited_meter();
        freeze_curve(
            &curve,
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
            GeometryDirectionProfile::Smooth1Deg,
            &mut meter,
        )
        .unwrap();
        assert!(meter.peak() >= size_of::<SubdivisionFrame>() as u64);
    }

    fn unlimited_meter() -> StageScratchMeter {
        StageScratchMeter::unlimited()
    }

    fn vec3(tokens: [&str; 3]) -> ParsedVec3 {
        ParsedVec3 {
            components: tokens.map(|token| RawNumber {
                token: token.into(),
                span: test_span(),
            }),
            span: test_span(),
        }
    }

    fn raw(token: &str) -> RawNumber {
        RawNumber {
            token: token.into(),
            span: test_span(),
        }
    }

    fn pending(key: &str, width: f64) -> PendingLateralIntent {
        PendingLateralIntent {
            key: key.into(),
            kind: LateralIntentKind::ForwardLane,
            width,
        }
    }

    fn cross_section_span(
        start: &str,
        end: super::super::road::ParsedEndStation,
    ) -> super::super::road::ParsedCrossSectionSpan {
        use super::super::{SpannedString, road::ParsedCrossSectionSpan};

        let token = || SpannedString {
            value: "test".into(),
            span: test_span(),
        };
        ParsedCrossSectionSpan {
            span_key: token(),
            corridor_key: token(),
            start_station_meters: raw(start),
            end_station_meters: end,
            reference_section_key: token(),
            reference_lane_key: token(),
            elements: Box::default(),
            road_sections: Box::default(),
            facility_bands: Box::default(),
            span: test_span(),
        }
    }

    const fn test_span() -> ByteSpan {
        ByteSpan { start: 0, end: 1 }
    }
}
