//! RoadEditingSource authoring curve 到共同规范几何的有界两遍编译。

use crate::declaration::{
    AuthoringCurveProgramDeclaration, AuthoringCurveSegmentDeclaration,
    AuthoringCurveSegmentGeometry, AuthoringLaneDirection, AuthoringPoint3F64,
    AuthoringWidthProfile, CanonicalPoint3F32Input, EdgeLength,
};
use crate::{GeometryAccuracyProfile, GeometryDirectionProfile};

use super::geometry::{
    ApproximationInterval, ApproximationPoint, ApproximationPointSink, ApproximationVertex,
    CurveSegment, NumericFreezeError, OffsetInterval, Point3, SegmentEvaluator, StationInterval,
    approximate_interval, canonical_point_distance, point_distance, quantize_point,
    validate_canonical_polyline,
};

const MAX_SOURCE_JOIN_GAP_METERS: f64 = 0.005;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ReferenceStationRow {
    pub(super) segment_ordinal: u32,
    pub(super) parameter_start: f64,
    pub(super) parameter_end: f64,
    pub(super) cumulative_start_meters: f64,
    pub(super) cumulative_end_meters: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ReferenceStationPosition {
    pub(super) row_index: u32,
    pub(super) segment_ordinal: u32,
    pub(super) parameter: f64,
}

pub(super) fn locate_reference_station(
    rows: &[ReferenceStationRow],
    station_meters: f64,
) -> Result<ReferenceStationPosition, NumericFreezeError> {
    if !station_meters.is_finite() || station_meters < 0.0 || rows.is_empty() {
        return Err(NumericFreezeError::StationOutOfRange);
    }
    let row_index = rows.partition_point(|row| row.cumulative_end_meters < station_meters);
    let Some(row) = rows.get(row_index) else {
        return Err(NumericFreezeError::StationOutOfRange);
    };
    let parameter = parameter_in_station_row(row, station_meters)?;
    Ok(ReferenceStationPosition {
        row_index: u32::try_from(row_index).map_err(|_| NumericFreezeError::GeometryPointLimit)?,
        segment_ordinal: row.segment_ordinal,
        parameter,
    })
}

pub(super) struct CompiledCurve {
    pub(super) length: EdgeLength,
    pub(super) points: Box<[CanonicalPoint3F32Input]>,
}

pub(super) struct CompiledAlignmentReference {
    pub(super) station_rows: Box<[ReferenceStationRow]>,
    pub(super) horizontal_regularity_visits: Box<[u32]>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct MemberOffsetEndpoints {
    pub(super) start_meters: f64,
    pub(super) end_meters: f64,
}

pub(super) fn derive_member_offset_endpoints(
    width_profiles: &[AuthoringWidthProfile],
    reference_ordinal: usize,
) -> Result<Box<[MemberOffsetEndpoints]>, NumericFreezeError> {
    if width_profiles.is_empty() || reference_ordinal >= width_profiles.len() {
        return Err(NumericFreezeError::StationOutOfRange);
    }
    for profile in width_profiles {
        if !profile.start_width_meters.is_finite()
            || !profile.end_width_meters.is_finite()
            || profile.start_width_meters < 0.0
            || profile.end_width_meters < 0.0
        {
            return Err(NumericFreezeError::NonFinite);
        }
    }
    let mut offsets = vec![
        MemberOffsetEndpoints {
            start_meters: 0.0,
            end_meters: 0.0,
        };
        width_profiles.len()
    ];
    let reference = width_profiles[reference_ordinal];
    let mut left_start = 0.5 * reference.start_width_meters;
    let mut left_end = 0.5 * reference.end_width_meters;
    if !left_start.is_finite() || !left_end.is_finite() {
        return Err(NumericFreezeError::NonFinite);
    }
    for ordinal in (0..reference_ordinal).rev() {
        let width = width_profiles[ordinal];
        left_start += 0.5 * width.start_width_meters;
        left_end += 0.5 * width.end_width_meters;
        if !left_start.is_finite() || !left_end.is_finite() {
            return Err(NumericFreezeError::NonFinite);
        }
        offsets[ordinal] = MemberOffsetEndpoints {
            start_meters: left_start,
            end_meters: left_end,
        };
        left_start += 0.5 * width.start_width_meters;
        left_end += 0.5 * width.end_width_meters;
        if !left_start.is_finite() || !left_end.is_finite() {
            return Err(NumericFreezeError::NonFinite);
        }
    }

    let mut right_start = -(0.5 * reference.start_width_meters);
    let mut right_end = -(0.5 * reference.end_width_meters);
    if !right_start.is_finite() || !right_end.is_finite() {
        return Err(NumericFreezeError::NonFinite);
    }
    for ordinal in (reference_ordinal + 1)..width_profiles.len() {
        let width = width_profiles[ordinal];
        right_start -= 0.5 * width.start_width_meters;
        right_end -= 0.5 * width.end_width_meters;
        if !right_start.is_finite() || !right_end.is_finite() {
            return Err(NumericFreezeError::NonFinite);
        }
        offsets[ordinal] = MemberOffsetEndpoints {
            start_meters: right_start,
            end_meters: right_end,
        };
        right_start -= 0.5 * width.start_width_meters;
        right_end -= 0.5 * width.end_width_meters;
        if !right_start.is_finite() || !right_end.is_finite() {
            return Err(NumericFreezeError::NonFinite);
        }
    }
    Ok(offsets.into_boxed_slice())
}

pub(super) fn compile_explicit_curve(
    program: &AuthoringCurveProgramDeclaration,
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
    remaining_point_limit: u64,
) -> Result<CompiledCurve, NumericFreezeError> {
    let mut output_counter = CountingSink {
        count: 0,
        limit: remaining_point_limit,
        last_point: None,
    };
    walk_reference_program(program, accuracy, direction, &mut output_counter)?;
    let expected_points = usize::try_from(output_counter.count)
        .map_err(|_| NumericFreezeError::GeometryPointLimit)?;
    let mut point_collector = ExactPointSink {
        points: Vec::with_capacity(expected_points),
        expected_points,
    };
    walk_reference_program(program, accuracy, direction, &mut point_collector)?;
    if point_collector.points.len() != point_collector.expected_points {
        return Err(NumericFreezeError::ApproximationNotConverged);
    }
    let length = validate_canonical_polyline(&point_collector.points, direction)?;
    let length =
        EdgeLength::try_new(length).map_err(|_| NumericFreezeError::DegenerateCanonicalSegment)?;
    Ok(CompiledCurve {
        length,
        points: point_collector.points.into_boxed_slice(),
    })
}

pub(super) fn compile_alignment_reference(
    program: &AuthoringCurveProgramDeclaration,
    transient_vertex_limit: u64,
) -> Result<CompiledAlignmentReference, NumericFreezeError> {
    let mut station_counter = CountingSink {
        count: 0,
        limit: transient_vertex_limit,
        last_point: None,
    };
    walk_reference_program(
        program,
        GeometryAccuracyProfile::Fine2Cm,
        GeometryDirectionProfile::Smooth1Deg,
        &mut station_counter,
    )?;
    let expected_station_rows = usize::try_from(station_counter.count.saturating_sub(1))
        .map_err(|_| NumericFreezeError::GeometryPointLimit)?;
    let mut station_collector = StationRowSink {
        rows: Vec::with_capacity(expected_station_rows),
        active_segment: None,
        cumulative_meters: 0.0,
        seen_first_point: false,
        expected_rows: expected_station_rows,
    };
    walk_reference_program(
        program,
        GeometryAccuracyProfile::Fine2Cm,
        GeometryDirectionProfile::Smooth1Deg,
        &mut station_collector,
    )?;
    if station_collector.rows.len() != station_collector.expected_rows {
        return Err(NumericFreezeError::ApproximationNotConverged);
    }

    let mut visits = Vec::with_capacity(program.segments.len());
    let mut start = point3(program.start)?;
    for source in &program.segments {
        let (segment, end) = source_segment(start, source)?;
        visits.push(segment.prove_horizontal_regularity()?);
        start = end;
    }
    Ok(CompiledAlignmentReference {
        station_rows: station_collector.rows.into_boxed_slice(),
        horizontal_regularity_visits: visits.into_boxed_slice(),
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn compile_offset_curve(
    program: &AuthoringCurveProgramDeclaration,
    reference: &CompiledAlignmentReference,
    corridor_start_meters: f64,
    corridor_end_meters: f64,
    offset_start_meters: f64,
    offset_end_meters: f64,
    lane_direction: AuthoringLaneDirection,
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
    remaining_point_limit: u64,
) -> Result<CompiledCurve, NumericFreezeError> {
    validate_offset_domain(
        program,
        reference,
        corridor_start_meters,
        corridor_end_meters,
    )?;
    let mut counter = CountingSink {
        count: 0,
        limit: remaining_point_limit,
        last_point: None,
    };
    walk_offset_program(
        program,
        &reference.station_rows,
        corridor_start_meters,
        corridor_end_meters,
        offset_start_meters,
        offset_end_meters,
        accuracy,
        direction,
        &mut counter,
    )?;
    let expected_points =
        usize::try_from(counter.count).map_err(|_| NumericFreezeError::GeometryPointLimit)?;
    let mut collector = ExactPointSink {
        points: Vec::with_capacity(expected_points),
        expected_points,
    };
    walk_offset_program(
        program,
        &reference.station_rows,
        corridor_start_meters,
        corridor_end_meters,
        offset_start_meters,
        offset_end_meters,
        accuracy,
        direction,
        &mut collector,
    )?;
    if collector.points.len() != collector.expected_points {
        return Err(NumericFreezeError::ApproximationNotConverged);
    }
    if lane_direction == AuthoringLaneDirection::Backward {
        collector.points.reverse();
    }
    let length = validate_canonical_polyline(&collector.points, direction)?;
    let length =
        EdgeLength::try_new(length).map_err(|_| NumericFreezeError::DegenerateCanonicalSegment)?;
    Ok(CompiledCurve {
        length,
        points: collector.points.into_boxed_slice(),
    })
}

fn validate_offset_domain(
    program: &AuthoringCurveProgramDeclaration,
    reference: &CompiledAlignmentReference,
    corridor_start_meters: f64,
    corridor_end_meters: f64,
) -> Result<(), NumericFreezeError> {
    if reference.horizontal_regularity_visits.len() != program.segments.len() {
        return Err(NumericFreezeError::StationOutOfRange);
    }
    let station_rows = &reference.station_rows;
    let Some(last_row) = station_rows.last() else {
        return Err(NumericFreezeError::StationOutOfRange);
    };
    if !corridor_start_meters.is_finite()
        || !corridor_end_meters.is_finite()
        || corridor_start_meters < 0.0
        || corridor_start_meters >= corridor_end_meters
        || corridor_end_meters > last_row.cumulative_end_meters
    {
        return Err(NumericFreezeError::StationOutOfRange);
    }
    let mut row_index = 0_usize;
    let mut start = point3(program.start)?;
    for (segment_index, source) in program.segments.iter().enumerate() {
        let (_segment, end) = source_segment(start, source)?;
        let segment_ordinal =
            u32::try_from(segment_index).map_err(|_| NumericFreezeError::GeometryPointLimit)?;
        while let Some(row) = station_rows.get(row_index) {
            if row.segment_ordinal < segment_ordinal {
                return Err(NumericFreezeError::StationOutOfRange);
            }
            if row.segment_ordinal != segment_ordinal {
                break;
            }
            row_index += 1;
        }
        start = end;
    }
    if row_index != station_rows.len() {
        return Err(NumericFreezeError::StationOutOfRange);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn walk_offset_program(
    program: &AuthoringCurveProgramDeclaration,
    station_rows: &[ReferenceStationRow],
    corridor_start_meters: f64,
    corridor_end_meters: f64,
    offset_start_meters: f64,
    offset_end_meters: f64,
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
    sink: &mut impl CanonicalPointSink,
) -> Result<(), NumericFreezeError> {
    let mut row_index = 0_usize;
    let mut start = point3(program.start)?;
    let mut emitted_segment = None;
    for (segment_index, source) in program.segments.iter().enumerate() {
        let (segment, end) = source_segment(start, source)?;
        let segment_ordinal =
            u32::try_from(segment_index).map_err(|_| NumericFreezeError::GeometryPointLimit)?;
        while let Some(row) = station_rows.get(row_index) {
            if row.segment_ordinal != segment_ordinal {
                break;
            }
            let station_start = row.cumulative_start_meters.max(corridor_start_meters);
            let station_end = row.cumulative_end_meters.min(corridor_end_meters);
            if station_start < station_end {
                let parameter_start = parameter_in_station_row(row, station_start)?;
                let parameter_end = parameter_in_station_row(row, station_end)?;
                let evaluator = SegmentEvaluator::Offset {
                    segment,
                    station: StationInterval {
                        parameter_start: row.parameter_start,
                        parameter_end: row.parameter_end,
                        cumulative_start_meters: row.cumulative_start_meters,
                        cumulative_end_meters: row.cumulative_end_meters,
                    },
                    offset: OffsetInterval {
                        station_start_meters: corridor_start_meters,
                        station_end_meters: corridor_end_meters,
                        offset_start_meters,
                        offset_end_meters,
                    },
                };
                let source_boundary = emitted_segment.is_some_and(|value| value != segment_ordinal);
                let welded_start = if source_boundary {
                    let previous = sink
                        .last_point()
                        .ok_or(NumericFreezeError::DegenerateCanonicalSegment)?;
                    let actual = quantize_point(evaluator.evaluate(parameter_start)?.point)?;
                    if canonical_point_distance(previous, actual)? > MAX_SOURCE_JOIN_GAP_METERS {
                        return Err(NumericFreezeError::SourceJoinGapExceeded);
                    }
                    Some(previous)
                } else {
                    None
                };
                approximate_interval(
                    evaluator,
                    ApproximationInterval {
                        parameter_start,
                        parameter_end,
                        welded_start,
                        emit_start: sink.last_point().is_none(),
                    },
                    accuracy,
                    direction,
                    sink,
                )?;
                emitted_segment = Some(segment_ordinal);
            }
            row_index += 1;
        }
        start = end;
    }
    if sink.last_point().is_none() {
        return Err(NumericFreezeError::StationOutOfRange);
    }
    Ok(())
}

fn walk_reference_program(
    program: &AuthoringCurveProgramDeclaration,
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
    sink: &mut impl ReferenceProgramSink,
) -> Result<(), NumericFreezeError> {
    let mut start = point3(program.start)?;
    for (segment_index, source) in program.segments.iter().enumerate() {
        let (segment, end) = source_segment(start, source)?;
        let segment_ordinal =
            u32::try_from(segment_index).map_err(|_| NumericFreezeError::GeometryPointLimit)?;
        sink.begin_segment(segment_ordinal, segment)?;
        approximate_interval(
            SegmentEvaluator::Reference(segment),
            ApproximationInterval {
                parameter_start: 0.0,
                parameter_end: 1.0,
                welded_start: None,
                emit_start: segment_index == 0,
            },
            accuracy,
            direction,
            sink,
        )?;
        start = end;
    }
    Ok(())
}

fn parameter_in_station_row(
    row: &ReferenceStationRow,
    station_meters: f64,
) -> Result<f64, NumericFreezeError> {
    if !station_meters.is_finite()
        || station_meters < row.cumulative_start_meters
        || station_meters > row.cumulative_end_meters
    {
        return Err(NumericFreezeError::StationOutOfRange);
    }
    let station_delta = station_meters - row.cumulative_start_meters;
    let row_length = row.cumulative_end_meters - row.cumulative_start_meters;
    let station_fraction = station_delta / row_length;
    let parameter_delta = row.parameter_end - row.parameter_start;
    let parameter_scaled = parameter_delta * station_fraction;
    let parameter = row.parameter_start + parameter_scaled;
    if !parameter.is_finite() || parameter < row.parameter_start || parameter > row.parameter_end {
        return Err(NumericFreezeError::StationOutOfRange);
    }
    Ok(parameter)
}

fn source_segment(
    start: Point3,
    source: &AuthoringCurveSegmentDeclaration,
) -> Result<(CurveSegment, Point3), NumericFreezeError> {
    match source.geometry {
        AuthoringCurveSegmentGeometry::Line { end } => {
            let end = point3(end)?;
            Ok((CurveSegment::Line { start, end }, end))
        }
        AuthoringCurveSegmentGeometry::CubicBezier {
            control_1,
            control_2,
            end,
        } => {
            let control_1 = point3(control_1)?;
            let control_2 = point3(control_2)?;
            let end = point3(end)?;
            Ok((
                CurveSegment::CubicBezier {
                    start,
                    control_1,
                    control_2,
                    end,
                },
                end,
            ))
        }
    }
}

trait ReferenceProgramSink: ApproximationPointSink {
    fn begin_segment(
        &mut self,
        segment_ordinal: u32,
        segment: CurveSegment,
    ) -> Result<(), NumericFreezeError>;
}

fn point3(value: AuthoringPoint3F64) -> Result<Point3, NumericFreezeError> {
    Point3::try_new(value.x, value.y, value.z)
}

struct CountingSink {
    count: u64,
    limit: u64,
    last_point: Option<ApproximationPoint>,
}

impl ApproximationPointSink for CountingSink {
    fn push(&mut self, vertex: ApproximationVertex) -> Result<(), NumericFreezeError> {
        if self.count == self.limit {
            return Err(NumericFreezeError::GeometryPointLimit);
        }
        self.count += 1;
        self.last_point = Some(vertex.point);
        Ok(())
    }
}

trait CanonicalPointSink: ApproximationPointSink {
    fn last_point(&self) -> Option<ApproximationPoint>;
}

impl CanonicalPointSink for CountingSink {
    fn last_point(&self) -> Option<ApproximationPoint> {
        self.last_point
    }
}

impl ReferenceProgramSink for CountingSink {
    fn begin_segment(
        &mut self,
        _segment_ordinal: u32,
        _segment: CurveSegment,
    ) -> Result<(), NumericFreezeError> {
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct ActiveStationSegment {
    ordinal: u32,
    evaluator: CurveSegment,
    previous_parameter: f64,
    previous_point: Point3,
}

struct StationRowSink {
    rows: Vec<ReferenceStationRow>,
    active_segment: Option<ActiveStationSegment>,
    cumulative_meters: f64,
    seen_first_point: bool,
    expected_rows: usize,
}

impl ApproximationPointSink for StationRowSink {
    fn push(&mut self, vertex: ApproximationVertex) -> Result<(), NumericFreezeError> {
        let active = self
            .active_segment
            .as_mut()
            .expect("reference program begins a segment before emitting its points");
        if vertex.parameter != active.previous_parameter {
            let current_point = active.evaluator.evaluate(vertex.parameter)?.point;
            let chord_length = point_distance(active.previous_point, current_point)?;
            if chord_length == 0.0 {
                return Err(NumericFreezeError::DegenerateCanonicalSegment);
            }
            let cumulative_end = self.cumulative_meters + chord_length;
            if !cumulative_end.is_finite() {
                return Err(NumericFreezeError::NonFinite);
            }
            self.rows.push(ReferenceStationRow {
                segment_ordinal: active.ordinal,
                parameter_start: active.previous_parameter,
                parameter_end: vertex.parameter,
                cumulative_start_meters: self.cumulative_meters,
                cumulative_end_meters: cumulative_end,
            });
            self.cumulative_meters = cumulative_end;
            active.previous_parameter = vertex.parameter;
            active.previous_point = current_point;
        } else if self.seen_first_point {
            return Err(NumericFreezeError::DegenerateCanonicalSegment);
        }
        self.seen_first_point = true;
        Ok(())
    }
}

impl ReferenceProgramSink for StationRowSink {
    fn begin_segment(
        &mut self,
        segment_ordinal: u32,
        segment: CurveSegment,
    ) -> Result<(), NumericFreezeError> {
        self.active_segment = Some(ActiveStationSegment {
            ordinal: segment_ordinal,
            evaluator: segment,
            previous_parameter: 0.0,
            previous_point: segment.evaluate(0.0)?.point,
        });
        Ok(())
    }
}

struct ExactPointSink {
    points: Vec<CanonicalPoint3F32Input>,
    expected_points: usize,
}

impl ApproximationPointSink for ExactPointSink {
    fn push(&mut self, vertex: ApproximationVertex) -> Result<(), NumericFreezeError> {
        debug_assert!(self.points.len() < self.expected_points);
        self.points.push(vertex.point);
        Ok(())
    }
}

impl CanonicalPointSink for ExactPointSink {
    fn last_point(&self) -> Option<ApproximationPoint> {
        self.points.last().copied()
    }
}

impl ReferenceProgramSink for ExactPointSink {
    fn begin_segment(
        &mut self,
        _segment_ordinal: u32,
        _segment: CurveSegment,
    ) -> Result<(), NumericFreezeError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::SourceSpan;
    use crate::declaration::{
        AuthoringCurveSegmentDeclaration, AuthoringCurveSegmentGeometry, AuthoringPoint3F64,
    };

    fn point(x: f64, z: f64) -> AuthoringPoint3F64 {
        AuthoringPoint3F64 { x, y: 0.0, z }
    }

    fn span(column: u32) -> crate::SourceLocation {
        SourceSpan::point(Arc::from("roads/main"), 1, column).into()
    }

    #[test]
    fn explicit_line_uses_two_pass_exact_allocation_and_frozen_length() {
        let program = AuthoringCurveProgramDeclaration {
            start: point(0.0, 0.0),
            start_span: span(1),
            segments: vec![AuthoringCurveSegmentDeclaration {
                geometry: AuthoringCurveSegmentGeometry::Line {
                    end: point(3.0, 4.0),
                },
                span: span(2),
            }]
            .into_boxed_slice(),
        };
        let compiled = compile_explicit_curve(
            &program,
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
            2,
        )
        .unwrap();
        assert_eq!(compiled.points.len(), 2);
        assert_eq!(compiled.length.value(), 5.0);
        assert_eq!(compiled.points[1].x, 3.0);
        assert_eq!(compiled.points[1].z, 4.0);
        let reference = compile_alignment_reference(&program, 2).unwrap();
        assert_eq!(reference.station_rows.len(), 1);
        assert_eq!(
            reference.station_rows[0],
            ReferenceStationRow {
                segment_ordinal: 0,
                parameter_start: 0.0,
                parameter_end: 1.0,
                cumulative_start_meters: 0.0,
                cumulative_end_meters: 5.0,
            }
        );
    }

    #[test]
    fn companion_cubic_freezes_exact_point_count_before_allocation() {
        let program = AuthoringCurveProgramDeclaration {
            start: point(0.0, 0.0),
            start_span: span(1),
            segments: vec![AuthoringCurveSegmentDeclaration {
                geometry: AuthoringCurveSegmentGeometry::CubicBezier {
                    control_1: point(20.0, 20.0),
                    control_2: point(20.0, 0.0),
                    end: point(189.5, 0.0),
                },
                span: span(2),
            }]
            .into_boxed_slice(),
        };
        let compiled = compile_explicit_curve(
            &program,
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
            154,
        )
        .unwrap();
        assert_eq!(compiled.points.len(), 154);
        assert!(compiled.length.value() > 189.5);
        let reference = compile_alignment_reference(&program, 154).unwrap();
        assert_eq!(reference.station_rows.len(), 153);
        assert_eq!(reference.horizontal_regularity_visits.as_ref(), [3]);
        assert_eq!(reference.station_rows.last().unwrap().parameter_end, 1.0);
        let compact = compile_explicit_curve(
            &program,
            GeometryAccuracyProfile::Compact10Cm,
            GeometryDirectionProfile::Compact5Deg,
            154,
        )
        .unwrap();
        assert!(compact.points.len() < compiled.points.len());
        assert_eq!(
            compile_explicit_curve(
                &program,
                GeometryAccuracyProfile::Fine2Cm,
                GeometryDirectionProfile::Smooth1Deg,
                153,
            )
            .err(),
            Some(NumericFreezeError::GeometryPointLimit)
        );
    }

    #[test]
    fn source_segment_boundary_belongs_to_the_preceding_station_interval() {
        let program = AuthoringCurveProgramDeclaration {
            start: point(0.0, 0.0),
            start_span: span(1),
            segments: vec![
                AuthoringCurveSegmentDeclaration {
                    geometry: AuthoringCurveSegmentGeometry::Line {
                        end: point(1.0, 0.0),
                    },
                    span: span(2),
                },
                AuthoringCurveSegmentDeclaration {
                    geometry: AuthoringCurveSegmentGeometry::Line {
                        end: point(2.0, 0.0),
                    },
                    span: span(3),
                },
            ]
            .into_boxed_slice(),
        };
        let reference = compile_alignment_reference(&program, 3).unwrap();
        assert_eq!(reference.station_rows.len(), 2);
        assert_eq!(reference.station_rows[0].segment_ordinal, 0);
        assert_eq!(reference.station_rows[0].parameter_end, 1.0);
        assert_eq!(reference.station_rows[0].cumulative_end_meters, 1.0);
        assert_eq!(reference.station_rows[1].segment_ordinal, 1);
        assert_eq!(reference.station_rows[1].parameter_start, 0.0);
        assert_eq!(reference.station_rows[1].cumulative_start_meters, 1.0);
        assert_eq!(
            locate_reference_station(&reference.station_rows, 1.0).unwrap(),
            ReferenceStationPosition {
                row_index: 0,
                segment_ordinal: 0,
                parameter: 1.0,
            }
        );
        assert_eq!(
            locate_reference_station(&reference.station_rows, 1.5).unwrap(),
            ReferenceStationPosition {
                row_index: 1,
                segment_ordinal: 1,
                parameter: 0.5,
            }
        );
        assert_eq!(
            locate_reference_station(&reference.station_rows, 2.1),
            Err(NumericFreezeError::StationOutOfRange)
        );
    }

    #[test]
    fn offset_curve_clips_station_interval_and_reverses_backward_lane() {
        let program = AuthoringCurveProgramDeclaration {
            start: point(0.0, 0.0),
            start_span: span(1),
            segments: vec![AuthoringCurveSegmentDeclaration {
                geometry: AuthoringCurveSegmentGeometry::Line {
                    end: point(10.0, 0.0),
                },
                span: span(2),
            }]
            .into_boxed_slice(),
        };
        let reference = compile_alignment_reference(&program, 2).unwrap();
        let forward = compile_offset_curve(
            &program,
            &reference,
            2.0,
            8.0,
            2.0,
            4.0,
            AuthoringLaneDirection::Forward,
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
            2,
        )
        .unwrap();
        assert_eq!(forward.points.len(), 2);
        assert_eq!(forward.points[0].x, 2.0);
        assert_eq!(forward.points[0].z, -2.0);
        assert_eq!(forward.points[1].x, 8.0);
        assert_eq!(forward.points[1].z, -4.0);
        assert_eq!(forward.length.value(), 40.0_f64.sqrt());

        let backward = compile_offset_curve(
            &program,
            &reference,
            2.0,
            8.0,
            2.0,
            4.0,
            AuthoringLaneDirection::Backward,
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
            2,
        )
        .unwrap();
        assert_eq!(
            backward.points,
            forward.points.iter().rev().copied().collect()
        );
        assert_eq!(backward.length.value(), forward.length.value());
    }

    #[test]
    fn offset_source_join_welds_only_within_the_five_millimeter_gate() {
        fn joined_program(second_end_z: f64) -> AuthoringCurveProgramDeclaration {
            AuthoringCurveProgramDeclaration {
                start: point(0.0, 0.0),
                start_span: span(1),
                segments: vec![
                    AuthoringCurveSegmentDeclaration {
                        geometry: AuthoringCurveSegmentGeometry::Line {
                            end: point(10.0, 0.0),
                        },
                        span: span(2),
                    },
                    AuthoringCurveSegmentDeclaration {
                        geometry: AuthoringCurveSegmentGeometry::Line {
                            end: point(20.0, second_end_z),
                        },
                        span: span(3),
                    },
                ]
                .into_boxed_slice(),
            }
        }

        let accepted = joined_program(0.001);
        let reference = compile_alignment_reference(&accepted, 3).unwrap();
        let station_end = reference.station_rows.last().unwrap().cumulative_end_meters;
        let offset = compile_offset_curve(
            &accepted,
            &reference,
            0.0,
            station_end,
            1.0,
            1.0,
            AuthoringLaneDirection::Forward,
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
            3,
        )
        .unwrap();
        assert_eq!(offset.points.len(), 3);
        assert_eq!(offset.points[1].x, 10.0);
        assert_eq!(offset.points[1].z, -1.0);

        let rejected = joined_program(0.1);
        let reference = compile_alignment_reference(&rejected, 3).unwrap();
        let station_end = reference.station_rows.last().unwrap().cumulative_end_meters;
        assert_eq!(
            compile_offset_curve(
                &rejected,
                &reference,
                0.0,
                station_end,
                1.0,
                1.0,
                AuthoringLaneDirection::Forward,
                GeometryAccuracyProfile::Fine2Cm,
                GeometryDirectionProfile::Smooth1Deg,
                3,
            )
            .err(),
            Some(NumericFreezeError::SourceJoinGapExceeded)
        );
    }

    #[test]
    fn member_offsets_sum_widths_from_the_reference_outward() {
        let profiles = [
            AuthoringWidthProfile {
                start_width_meters: 2.0,
                end_width_meters: 4.0,
            },
            AuthoringWidthProfile {
                start_width_meters: 4.0,
                end_width_meters: 6.0,
            },
            AuthoringWidthProfile {
                start_width_meters: 6.0,
                end_width_meters: 8.0,
            },
            AuthoringWidthProfile {
                start_width_meters: 2.0,
                end_width_meters: 2.0,
            },
        ];
        assert_eq!(
            derive_member_offset_endpoints(&profiles, 1)
                .unwrap()
                .as_ref(),
            [
                MemberOffsetEndpoints {
                    start_meters: 3.0,
                    end_meters: 5.0,
                },
                MemberOffsetEndpoints {
                    start_meters: 0.0,
                    end_meters: 0.0,
                },
                MemberOffsetEndpoints {
                    start_meters: -5.0,
                    end_meters: -7.0,
                },
                MemberOffsetEndpoints {
                    start_meters: -9.0,
                    end_meters: -12.0,
                },
            ]
        );
        assert_eq!(
            derive_member_offset_endpoints(&profiles, profiles.len()),
            Err(NumericFreezeError::StationOutOfRange)
        );
    }
}
