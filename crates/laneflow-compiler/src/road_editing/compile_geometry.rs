//! RoadEditingSource authoring curve 到共同规范几何的有界两遍编译。

use crate::declaration::{
    AuthoringCurveProgramDeclaration, AuthoringCurveSegmentGeometry, AuthoringPoint3F64,
    CanonicalPoint3F32Input, EdgeLength,
};
use crate::{GeometryAccuracyProfile, GeometryDirectionProfile};

use super::geometry::{
    ApproximationInterval, ApproximationPointSink, ApproximationVertex, CurveSegment,
    NumericFreezeError, Point3, SegmentEvaluator, approximate_interval, quantize_point,
    validate_canonical_polyline,
};

pub(super) struct CompiledCurve {
    pub(super) length: EdgeLength,
    pub(super) points: Box<[CanonicalPoint3F32Input]>,
}

pub(super) fn compile_explicit_curve(
    program: &AuthoringCurveProgramDeclaration,
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
    remaining_point_limit: u64,
) -> Result<CompiledCurve, NumericFreezeError> {
    let mut counter = CountingSink {
        count: 0,
        limit: remaining_point_limit,
    };
    walk_reference_program(program, accuracy, direction, &mut counter)?;
    let capacity =
        usize::try_from(counter.count).map_err(|_| NumericFreezeError::GeometryPointLimit)?;
    let mut collector = ExactPointSink {
        points: Vec::with_capacity(capacity),
        expected: capacity,
    };
    walk_reference_program(program, accuracy, direction, &mut collector)?;
    debug_assert_eq!(collector.points.len(), collector.expected);
    let length = validate_canonical_polyline(&collector.points, direction)?;
    let length =
        EdgeLength::try_new(length).map_err(|_| NumericFreezeError::DegenerateCanonicalSegment)?;
    Ok(CompiledCurve {
        length,
        points: collector.points.into_boxed_slice(),
    })
}

fn walk_reference_program(
    program: &AuthoringCurveProgramDeclaration,
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
    sink: &mut impl ApproximationPointSink,
) -> Result<(), NumericFreezeError> {
    let mut start = point3(program.start)?;
    let mut welded_start = None;
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
        approximate_interval(
            SegmentEvaluator::Reference(segment),
            ApproximationInterval {
                parameter_start: 0.0,
                parameter_end: 1.0,
                welded_start,
                emit_start: segment_index == 0,
            },
            accuracy,
            direction,
            sink,
        )?;
        welded_start = Some(quantize_point(end)?);
        start = end;
    }
    Ok(())
}

fn point3(value: AuthoringPoint3F64) -> Result<Point3, NumericFreezeError> {
    Point3::try_new(value.x, value.y, value.z)
}

struct CountingSink {
    count: u64,
    limit: u64,
}

impl ApproximationPointSink for CountingSink {
    fn push(&mut self, _vertex: ApproximationVertex) -> Result<(), NumericFreezeError> {
        if self.count == self.limit {
            return Err(NumericFreezeError::GeometryPointLimit);
        }
        self.count += 1;
        Ok(())
    }
}

struct ExactPointSink {
    points: Vec<CanonicalPoint3F32Input>,
    expected: usize,
}

impl ApproximationPointSink for ExactPointSink {
    fn push(&mut self, vertex: ApproximationVertex) -> Result<(), NumericFreezeError> {
        debug_assert!(self.points.len() < self.expected);
        self.points.push(vertex.point);
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
    fn adjacent_segments_reuse_the_retained_canonical_endpoint() {
        let program = AuthoringCurveProgramDeclaration {
            start: point(0.0, 0.0),
            start_span: span(1),
            segments: vec![
                AuthoringCurveSegmentDeclaration {
                    geometry: AuthoringCurveSegmentGeometry::Line {
                        end: point(3.0, 0.0),
                    },
                    span: span(2),
                },
                AuthoringCurveSegmentDeclaration {
                    geometry: AuthoringCurveSegmentGeometry::Line {
                        end: point(6.0, 0.0),
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
            3,
        )
        .expect("adjacent segments share one retained endpoint");

        assert_eq!(compiled.points.len(), 3);
        assert_eq!(compiled.points[0].x, 0.0);
        assert_eq!(compiled.points[1].x, 3.0);
        assert_eq!(compiled.points[2].x, 6.0);
        assert_eq!(compiled.length.value(), 6.0);
    }
}
