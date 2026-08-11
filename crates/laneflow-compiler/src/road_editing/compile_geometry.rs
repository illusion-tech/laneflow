//! RoadEditingSource authoring curve 到共同规范几何的有界两遍编译。

use crate::declaration::{
    AuthoringCurveProgramDeclaration, AuthoringCurveSegmentGeometry, AuthoringPoint3F64,
    CanonicalPoint3F32Input, EdgeLength,
};
use crate::{GeometryAccuracyProfile, GeometryDirectionProfile};

use super::geometry::{
    ApproximationInterval, ApproximationPointSink, ApproximationVertex, CurveSegment,
    NumericFreezeError, Point3, SegmentEvaluator, approximate_interval, point_distance,
    validate_canonical_polyline,
};

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
    let station_delta = station_meters - row.cumulative_start_meters;
    let row_length = row.cumulative_end_meters - row.cumulative_start_meters;
    let station_fraction = station_delta / row_length;
    let parameter_delta = row.parameter_end - row.parameter_start;
    let parameter_scaled = parameter_delta * station_fraction;
    let parameter = row.parameter_start + parameter_scaled;
    if !parameter.is_finite() || parameter < row.parameter_start || parameter > row.parameter_end {
        return Err(NumericFreezeError::StationOutOfRange);
    }
    Ok(ReferenceStationPosition {
        row_index: u32::try_from(row_index).map_err(|_| NumericFreezeError::GeometryPointLimit)?,
        segment_ordinal: row.segment_ordinal,
        parameter,
    })
}

pub(super) struct CompiledCurve {
    pub(super) length: EdgeLength,
    pub(super) points: Box<[CanonicalPoint3F32Input]>,
    pub(super) reference_station_rows: Box<[ReferenceStationRow]>,
}

pub(super) fn compile_explicit_curve(
    program: &AuthoringCurveProgramDeclaration,
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
    station_row_byte_limit: u64,
    remaining_point_limit: u64,
) -> Result<CompiledCurve, NumericFreezeError> {
    let station_row_size = u64::try_from(core::mem::size_of::<ReferenceStationRow>())
        .map_err(|_| NumericFreezeError::StationRowLimit)?;
    let station_vertex_limit = station_row_byte_limit
        .checked_div(station_row_size)
        .and_then(|row_limit| row_limit.checked_add(1))
        .ok_or(NumericFreezeError::StationRowLimit)?;
    let mut station_counter = CountingSink {
        count: 0,
        limit: station_vertex_limit,
        limit_error: NumericFreezeError::StationRowLimit,
    };
    walk_reference_program(
        program,
        GeometryAccuracyProfile::Fine2Cm,
        GeometryDirectionProfile::Smooth1Deg,
        &mut station_counter,
    )?;
    let expected_station_rows = usize::try_from(station_counter.count.saturating_sub(1))
        .map_err(|_| NumericFreezeError::StationRowLimit)?;
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

    let mut output_counter = CountingSink {
        count: 0,
        limit: remaining_point_limit,
        limit_error: NumericFreezeError::GeometryPointLimit,
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
        reference_station_rows: station_collector.rows.into_boxed_slice(),
    })
}

fn walk_reference_program(
    program: &AuthoringCurveProgramDeclaration,
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
    sink: &mut impl ReferenceProgramSink,
) -> Result<(), NumericFreezeError> {
    let mut start = point3(program.start)?;
    for (segment_index, source) in program.segments.iter().enumerate() {
        let (segment, end) = match source.geometry {
            AuthoringCurveSegmentGeometry::Line { end } => {
                let end = point3(end)?;
                (CurveSegment::Line { start, end }, end)
            }
            AuthoringCurveSegmentGeometry::CubicBezier {
                control_1,
                control_2,
                end,
            } => {
                let control_1 = point3(control_1)?;
                let control_2 = point3(control_2)?;
                let end = point3(end)?;
                (
                    CurveSegment::CubicBezier {
                        start,
                        control_1,
                        control_2,
                        end,
                    },
                    end,
                )
            }
        };
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
    limit_error: NumericFreezeError,
}

impl ApproximationPointSink for CountingSink {
    fn push(&mut self, _vertex: ApproximationVertex) -> Result<(), NumericFreezeError> {
        if self.count == self.limit {
            return Err(self.limit_error);
        }
        self.count += 1;
        Ok(())
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

    fn station_row_bytes(rows: u64) -> u64 {
        rows * u64::try_from(core::mem::size_of::<ReferenceStationRow>()).unwrap()
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
            station_row_bytes(1),
            2,
        )
        .unwrap();
        assert_eq!(compiled.points.len(), 2);
        assert_eq!(compiled.reference_station_rows.len(), 1);
        assert_eq!(compiled.length.value(), 5.0);
        assert_eq!(compiled.points[1].x, 3.0);
        assert_eq!(compiled.points[1].z, 4.0);
        assert_eq!(
            compiled.reference_station_rows[0],
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
            station_row_bytes(153),
            154,
        )
        .unwrap();
        assert_eq!(compiled.points.len(), 154);
        assert_eq!(compiled.reference_station_rows.len(), 153);
        assert!(compiled.length.value() > 189.5);
        assert_eq!(
            compiled
                .reference_station_rows
                .last()
                .unwrap()
                .parameter_end,
            1.0
        );
        let compact = compile_explicit_curve(
            &program,
            GeometryAccuracyProfile::Compact10Cm,
            GeometryDirectionProfile::Compact5Deg,
            station_row_bytes(153),
            153,
        )
        .unwrap();
        assert!(compact.points.len() < compiled.points.len());
        assert_eq!(compact.reference_station_rows.len(), 153);
        assert_eq!(
            compile_explicit_curve(
                &program,
                GeometryAccuracyProfile::Fine2Cm,
                GeometryDirectionProfile::Smooth1Deg,
                station_row_bytes(153),
                153,
            )
            .err(),
            Some(NumericFreezeError::GeometryPointLimit)
        );
        assert_eq!(
            compile_explicit_curve(
                &program,
                GeometryAccuracyProfile::Compact10Cm,
                GeometryDirectionProfile::Compact5Deg,
                station_row_bytes(152),
                153,
            )
            .err(),
            Some(NumericFreezeError::StationRowLimit)
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
        let compiled = compile_explicit_curve(
            &program,
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
            station_row_bytes(2),
            3,
        )
        .unwrap();
        assert_eq!(compiled.points.len(), 3);
        assert_eq!(compiled.reference_station_rows.len(), 2);
        assert_eq!(compiled.reference_station_rows[0].segment_ordinal, 0);
        assert_eq!(compiled.reference_station_rows[0].parameter_end, 1.0);
        assert_eq!(
            compiled.reference_station_rows[0].cumulative_end_meters,
            1.0
        );
        assert_eq!(compiled.reference_station_rows[1].segment_ordinal, 1);
        assert_eq!(compiled.reference_station_rows[1].parameter_start, 0.0);
        assert_eq!(
            compiled.reference_station_rows[1].cumulative_start_meters,
            1.0
        );
        assert_eq!(
            locate_reference_station(&compiled.reference_station_rows, 1.0).unwrap(),
            ReferenceStationPosition {
                row_index: 0,
                segment_ordinal: 0,
                parameter: 1.0,
            }
        );
        assert_eq!(
            locate_reference_station(&compiled.reference_station_rows, 1.5).unwrap(),
            ReferenceStationPosition {
                row_index: 1,
                segment_ordinal: 1,
                parameter: 0.5,
            }
        );
        assert_eq!(
            locate_reference_station(&compiled.reference_station_rows, 2.1),
            Err(NumericFreezeError::StationOutOfRange)
        );
    }
}
