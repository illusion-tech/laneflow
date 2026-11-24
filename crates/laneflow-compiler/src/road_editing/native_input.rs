use laneflow_road_editing_wire::generated::lane_flow::road_editing::v1 as wire;

use super::model::RoadEditingCurveSegmentGeometry as Geometry;
use super::rules::{finite_violation, positive_violation};
use crate::RoadEditingInputViolation as Violation;

pub(super) type FieldError = (&'static str, Violation);

impl Geometry {
    pub(super) const fn is_native(self) -> bool {
        matches!(self, Self::CircularArc { .. } | Self::Clothoid { .. })
    }
}

pub(super) fn validate_entry(
    native: bool,
    smooth: bool,
    tangent: Option<[f64; 2]>,
) -> Result<(), FieldError> {
    if smooth {
        return Err(("curveSegment.smooth_entry", Violation::InvalidCombination));
    }
    if native != tangent.is_some()
        || tangent.is_some_and(|[x, z]| !x.is_finite() || !z.is_finite() || x == 0.0 && z == 0.0)
    {
        return Err((
            "curveProgram.initial_tangent_xz",
            Violation::InvalidCombination,
        ));
    }
    Ok(())
}

const LENGTH: &str = "curveSegment.geometry.length_meters";
const RADIUS: &str = "curveSegment.geometry.signed_radius_meters";
const START: &str = "curveSegment.geometry.start_curvature_per_meter";
const END: &str = "curveSegment.geometry.end_curvature_per_meter";
const GRADE: &str = "curveSegment.geometry.grade";

pub(super) fn validate(geometry: Geometry) -> Result<(), FieldError> {
    let (length, grade) = match geometry {
        Geometry::CircularArc {
            length_meters,
            signed_radius_meters,
            grade,
        } => {
            if let Some(cause) = finite_violation(signed_radius_meters) {
                return Err((RADIUS, cause));
            }
            let curvature = 1.0 / signed_radius_meters;
            if !curvature.is_finite() || curvature == 0.0 {
                return Err((RADIUS, Violation::InvalidCombination));
            }
            (length_meters, grade)
        }
        Geometry::Clothoid {
            length_meters,
            start_curvature_per_meter,
            end_curvature_per_meter,
            grade,
        } => {
            for (field, value) in [
                (START, start_curvature_per_meter),
                (END, end_curvature_per_meter),
            ] {
                if let Some(cause) = finite_violation(value) {
                    return Err((field, cause));
                }
            }
            if start_curvature_per_meter == end_curvature_per_meter {
                return Err((END, Violation::InvalidCombination));
            }
            (length_meters, grade)
        }
        _ => return Ok(()),
    };
    if let Some(cause) = positive_violation(length) {
        return Err((LENGTH, cause));
    }
    if let Some(cause) = finite_violation(grade) {
        return Err((GRADE, cause));
    }
    Ok(())
}

fn required(value: Option<f64>, field: &'static str) -> Result<f64, FieldError> {
    value.ok_or((field, Violation::InvalidCombination))
}

pub(super) fn decode(segment: wire::CurveSegment<'_>) -> Result<Geometry, FieldError> {
    let geometry = if let Some(arc) = segment.geometry_as_circular_arc_segment() {
        Geometry::CircularArc {
            length_meters: required(arc.length_meters(), LENGTH)?,
            signed_radius_meters: required(arc.signed_radius_meters(), RADIUS)?,
            grade: required(arc.grade(), GRADE)?,
        }
    } else if let Some(spiral) = segment.geometry_as_clothoid_segment() {
        Geometry::Clothoid {
            length_meters: required(spiral.length_meters(), LENGTH)?,
            start_curvature_per_meter: required(spiral.start_curvature_per_meter(), START)?,
            end_curvature_per_meter: required(spiral.end_curvature_per_meter(), END)?,
            grade: required(spiral.grade(), GRADE)?,
        }
    } else {
        return Err(("curveSegment.geometry", Violation::InvalidCombination));
    };
    validate(geometry)?;
    Ok(geometry)
}
