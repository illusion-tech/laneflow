//! RoadEditingSource authoring curve 到共同规范几何的有界两遍编译。

use crate::declaration::{
    AuthoringCurveProgramDeclaration, AuthoringCurveSegmentDeclaration,
    AuthoringCurveSegmentGeometry, AuthoringLaneDirection, AuthoringPoint3F64, AuthoringStationEnd,
    AuthoringWidthProfile, CanonicalPoint3F32Input, CompiledFacilityBandGeometry,
    CompiledGeometrySourceRange, CompiledLaneEdgeGeometry, EdgeLength, FacilityBandDeclaration,
    LaneEdgeGeometryAuthority, OwnedCorridorElementReference, RoadAlignmentDeclaration,
    RoadSectionDeclaration, TypedAstDeclaration, TypedAstEntityAddress,
};
use crate::{GeometryAccuracyProfile, GeometryDirectionProfile};

use super::geometry::{
    ApproximationInterval, ApproximationPoint, ApproximationPointSink, ApproximationVertex,
    CurveSegment, NumericFreezeError, OffsetInterval, Point3, SegmentEvaluator, StationInterval,
    approximate_interval, canonical_point_distance, direction_accepts, full_angle_cosine_squared,
    numeric_stack_scratch_bytes, point_distance, quantize_point, validate_canonical_polyline,
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
    pub(super) source_ranges: Box<[CompiledGeometrySourceRange]>,
}

pub(super) struct CompiledAlignmentReference {
    pub(super) station_rows: Box<[ReferenceStationRow]>,
    pub(super) horizontal_regularity_visits: Box<[u32]>,
}

#[derive(Debug, Eq, PartialEq)]
struct AlignmentReferenceSizing {
    station_rows: usize,
    horizontal_regularity_visits: Box<[u32]>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct MemberOffsetEndpoints {
    pub(super) start_meters: f64,
    pub(super) end_meters: f64,
}

fn canonicalize_zero(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
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
    let mut left_start = canonicalize_zero(0.5 * reference.start_width_meters);
    let mut left_end = canonicalize_zero(0.5 * reference.end_width_meters);
    if !left_start.is_finite() || !left_end.is_finite() {
        return Err(NumericFreezeError::NonFinite);
    }
    for ordinal in (0..reference_ordinal).rev() {
        let width = width_profiles[ordinal];
        let target_start = canonicalize_zero(left_start + 0.5 * width.start_width_meters);
        let target_end = canonicalize_zero(left_end + 0.5 * width.end_width_meters);
        if !target_start.is_finite() || !target_end.is_finite() {
            return Err(NumericFreezeError::NonFinite);
        }
        offsets[ordinal] = MemberOffsetEndpoints {
            start_meters: target_start,
            end_meters: target_end,
        };
        left_start = canonicalize_zero(left_start + width.start_width_meters);
        left_end = canonicalize_zero(left_end + width.end_width_meters);
        if !left_start.is_finite() || !left_end.is_finite() {
            return Err(NumericFreezeError::NonFinite);
        }
    }

    let mut right_start = canonicalize_zero(-(0.5 * reference.start_width_meters));
    let mut right_end = canonicalize_zero(-(0.5 * reference.end_width_meters));
    if !right_start.is_finite() || !right_end.is_finite() {
        return Err(NumericFreezeError::NonFinite);
    }
    for ordinal in (reference_ordinal + 1)..width_profiles.len() {
        let width = width_profiles[ordinal];
        let target_start = canonicalize_zero(right_start - 0.5 * width.start_width_meters);
        let target_end = canonicalize_zero(right_end - 0.5 * width.end_width_meters);
        if !target_start.is_finite() || !target_end.is_finite() {
            return Err(NumericFreezeError::NonFinite);
        }
        offsets[ordinal] = MemberOffsetEndpoints {
            start_meters: target_start,
            end_meters: target_end,
        };
        right_start = canonicalize_zero(right_start - width.start_width_meters);
        right_end = canonicalize_zero(right_end - width.end_width_meters);
        if !right_start.is_finite() || !right_end.is_finite() {
            return Err(NumericFreezeError::NonFinite);
        }
    }
    Ok(offsets.into_boxed_slice())
}

struct CompiledAlignmentEntry<'a> {
    declaration: &'a RoadAlignmentDeclaration,
    reference: CompiledAlignmentReference,
}

enum CorridorMemberTarget<'a> {
    LaneEdge {
        target: &'a TypedAstEntityAddress,
        direction: AuthoringLaneDirection,
    },
    FacilityBand {
        target: &'a TypedAstEntityAddress,
    },
}

struct CorridorMember<'a> {
    width_profile: AuthoringWidthProfile,
    target: CorridorMemberTarget<'a>,
}

enum ResolvedCorridorElement<'a> {
    RoadSection(&'a RoadSectionDeclaration),
    FacilityBand(&'a FacilityBandDeclaration),
}

struct GeometryDeclarationIndex<'a> {
    sections: Box<[&'a RoadSectionDeclaration]>,
    facility_bands: Box<[&'a FacilityBandDeclaration]>,
}

struct GeometryScratchBudget {
    limit: u64,
    live: u64,
    peak: u64,
}

impl GeometryScratchBudget {
    fn new(limit: u64) -> Self {
        Self {
            limit,
            live: 0,
            peak: 0,
        }
    }

    fn reserve<T>(&mut self, count: usize) -> Result<(), GeometryCompilationError> {
        let bytes = capacity_bytes::<T>(count).ok_or(GeometryCompilationError::ScratchLimit {
            limit: self.limit,
            observed: u64::MAX,
        })?;
        self.reserve_bytes(bytes)
    }

    fn reserve_bytes(&mut self, bytes: u64) -> Result<(), GeometryCompilationError> {
        let observed = self.live.saturating_add(bytes);
        if observed > self.limit {
            return Err(GeometryCompilationError::ScratchLimit {
                limit: self.limit,
                observed,
            });
        }
        self.live = observed;
        self.peak = self.peak.max(self.live);
        Ok(())
    }

    fn release<T>(&mut self, count: usize) {
        let bytes = capacity_bytes::<T>(count).expect("previously reserved scratch capacity");
        self.release_bytes(bytes);
    }

    fn release_bytes(&mut self, bytes: u64) {
        self.live = self
            .live
            .checked_sub(bytes)
            .expect("scratch release matches a live reservation");
    }

    fn remaining_capacity<T>(&self) -> u64 {
        let size = u64::try_from(core::mem::size_of::<T>()).unwrap_or(u64::MAX);
        debug_assert_ne!(size, 0, "geometry scratch never reserves zero-sized types");
        self.limit.saturating_sub(self.live) / size
    }
}

fn capacity_bytes<T>(count: usize) -> Option<u64> {
    u64::try_from(core::mem::size_of::<T>())
        .ok()
        .and_then(|size| {
            u64::try_from(count)
                .ok()
                .and_then(|count| size.checked_mul(count))
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct GeometryCompilationUsage {
    pub(super) output_point_count: u64,
    pub(super) peak_scratch_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GeometryCompilationError {
    Numeric(NumericFreezeError),
    ScratchLimit { limit: u64, observed: u64 },
}

impl From<NumericFreezeError> for GeometryCompilationError {
    fn from(value: NumericFreezeError) -> Self {
        Self::Numeric(value)
    }
}

impl<'a> GeometryDeclarationIndex<'a> {
    fn new(
        declarations: &'a [TypedAstDeclaration],
        scratch: &mut GeometryScratchBudget,
    ) -> Result<Self, GeometryCompilationError> {
        let section_count = declarations
            .iter()
            .filter(|value| matches!(value, TypedAstDeclaration::RoadSection(_)))
            .count();
        let facility_count = declarations
            .iter()
            .filter(|value| matches!(value, TypedAstDeclaration::FacilityBand(_)))
            .count();
        scratch.reserve::<&RoadSectionDeclaration>(section_count)?;
        scratch.reserve::<&FacilityBandDeclaration>(facility_count)?;
        let mut sections = Vec::with_capacity(section_count);
        let mut facility_bands = Vec::with_capacity(facility_count);
        for declaration in declarations {
            match declaration {
                TypedAstDeclaration::RoadSection(value) => sections.push(value),
                TypedAstDeclaration::FacilityBand(value) => facility_bands.push(value),
                _ => {}
            }
        }
        sections.sort_unstable_by(|left, right| {
            left.header.source_address.cmp(&right.header.source_address)
        });
        facility_bands.sort_unstable_by(|left, right| {
            left.header.source_address.cmp(&right.header.source_address)
        });
        if sections
            .windows(2)
            .any(|pair| pair[0].header.source_address == pair[1].header.source_address)
            || facility_bands
                .windows(2)
                .any(|pair| pair[0].header.source_address == pair[1].header.source_address)
        {
            return Err(NumericFreezeError::GeometryTopologyMismatch.into());
        }
        Ok(Self {
            sections: sections.into_boxed_slice(),
            facility_bands: facility_bands.into_boxed_slice(),
        })
    }

    fn section(&self, address: &TypedAstEntityAddress) -> Option<&'a RoadSectionDeclaration> {
        self.sections
            .binary_search_by(|value| value.header.source_address.cmp(address))
            .ok()
            .map(|index| self.sections[index])
    }

    fn facility_band(
        &self,
        address: &TypedAstEntityAddress,
    ) -> Option<&'a FacilityBandDeclaration> {
        self.facility_bands
            .binary_search_by(|value| value.header.source_address.cmp(address))
            .ok()
            .map(|index| self.facility_bands[index])
    }
}

struct ResolvedCorridorPlan<'a> {
    corridor: &'a crate::declaration::RoadCorridorDeclaration,
    alignment: &'a RoadAlignmentDeclaration,
    elements: Box<[ResolvedCorridorElement<'a>]>,
    member_count: usize,
    reference_ordinal: usize,
}

struct PendingLaneGeometry {
    target: TypedAstEntityAddress,
    value: Option<CompiledLaneEdgeGeometry>,
}

struct PendingFacilityGeometry {
    target: TypedAstEntityAddress,
    value: Option<CompiledFacilityBandGeometry>,
}

#[allow(clippy::too_many_lines)]
#[allow(clippy::too_many_arguments)]
pub(super) fn compile_authoring_geometry(
    authoring_namespace_id: &str,
    alignments: Box<[RoadAlignmentDeclaration]>,
    declarations: &mut [TypedAstDeclaration],
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
    station_row_byte_limit: u64,
    geometry_point_limit: u64,
    geometry_scratch_byte_limit: u64,
) -> Result<GeometryCompilationUsage, GeometryCompilationError> {
    let mut scratch = GeometryScratchBudget::new(geometry_scratch_byte_limit);
    let plans = resolve_corridor_plans(
        authoring_namespace_id,
        &alignments,
        declarations,
        &mut scratch,
    )?;
    validate_station_partitions(&plans)?;
    let mut referenced_alignment_count = 0_usize;
    let mut regularity_visit_count = 0_usize;
    for (index, plan) in plans.iter().enumerate() {
        if index != 0
            && plans[index - 1].alignment.road_alignment_key == plan.alignment.road_alignment_key
        {
            continue;
        }
        referenced_alignment_count = referenced_alignment_count.checked_add(1).ok_or(
            GeometryCompilationError::ScratchLimit {
                limit: geometry_scratch_byte_limit,
                observed: u64::MAX,
            },
        )?;
        regularity_visit_count = regularity_visit_count
            .checked_add(plan.alignment.reference_line.segments.len())
            .ok_or(GeometryCompilationError::ScratchLimit {
                limit: geometry_scratch_byte_limit,
                observed: u64::MAX,
            })?;
    }
    let expected_lane_outputs = declarations
        .iter()
        .filter(|declaration| {
            matches!(
                declaration,
                TypedAstDeclaration::LaneEdge(edge)
                    if matches!(edge.geometry_authority, LaneEdgeGeometryAuthority::Authoring { .. })
            )
        })
        .count();
    let expected_facility_outputs = declarations
        .iter()
        .filter(|declaration| {
            matches!(
                declaration,
                TypedAstDeclaration::FacilityBand(facility)
                    if facility.authoring_width_profile.is_some()
            )
        })
        .count();
    scratch.reserve::<CompiledAlignmentEntry<'_>>(referenced_alignment_count)?;
    scratch.reserve::<u32>(regularity_visit_count)?;
    scratch.reserve::<PendingLaneGeometry>(expected_lane_outputs)?;
    scratch.reserve::<PendingFacilityGeometry>(expected_facility_outputs)?;
    let mut compiled_alignments = Vec::with_capacity(referenced_alignment_count);
    let mut lane_outputs = Vec::with_capacity(expected_lane_outputs);
    let mut facility_outputs = Vec::with_capacity(expected_facility_outputs);
    let mut station_row_count = 0_usize;
    let numeric_scratch_bytes = numeric_stack_scratch_bytes();
    let station_vertex_limit = station_vertex_limit_from_bytes(station_row_byte_limit)?;
    for plan in &plans {
        if compiled_alignments
            .last()
            .is_some_and(|entry: &CompiledAlignmentEntry<'_>| {
                entry.declaration.road_alignment_key == plan.alignment.road_alignment_key
            })
        {
            continue;
        }
        let scratch_vertex_limit = scratch
            .remaining_capacity::<ReferenceStationRow>()
            .saturating_add(1);
        let (transient_vertex_limit, limit_error) = if station_vertex_limit <= scratch_vertex_limit
        {
            (station_vertex_limit, NumericFreezeError::StationRowLimit)
        } else {
            (scratch_vertex_limit, NumericFreezeError::GeometryPointLimit)
        };
        scratch.reserve_bytes(numeric_scratch_bytes)?;
        let sizing_result = measure_alignment_reference(
            &plan.alignment.reference_line,
            transient_vertex_limit,
            limit_error,
        );
        scratch.release_bytes(numeric_scratch_bytes);
        let sizing = match sizing_result {
            Err(NumericFreezeError::GeometryPointLimit) => {
                return Err(GeometryCompilationError::ScratchLimit {
                    limit: geometry_scratch_byte_limit,
                    observed: geometry_scratch_byte_limit.saturating_add(1),
                });
            }
            Err(error) => return Err(error.into()),
            Ok(value) => value,
        };
        scratch.reserve::<ReferenceStationRow>(sizing.station_rows)?;
        station_row_count = station_row_count.checked_add(sizing.station_rows).ok_or(
            GeometryCompilationError::ScratchLimit {
                limit: geometry_scratch_byte_limit,
                observed: u64::MAX,
            },
        )?;
        scratch.reserve_bytes(numeric_scratch_bytes)?;
        let reference_result =
            compile_measured_alignment_reference(&plan.alignment.reference_line, sizing);
        scratch.release_bytes(numeric_scratch_bytes);
        let reference = reference_result?;
        compiled_alignments.push(CompiledAlignmentEntry {
            declaration: plan.alignment,
            reference,
        });
    }

    let mut used_points = 0_u64;

    for declaration in declarations.iter() {
        let TypedAstDeclaration::LaneEdge(edge) = declaration else {
            continue;
        };
        let LaneEdgeGeometryAuthority::Authoring {
            explicit_curve: Some(curve),
        } = &edge.geometry_authority
        else {
            continue;
        };
        scratch.reserve_bytes(numeric_scratch_bytes)?;
        let compiled_result = compile_explicit_curve(
            curve,
            accuracy,
            direction,
            remaining_points(geometry_point_limit, used_points)?,
        );
        scratch.release_bytes(numeric_scratch_bytes);
        let compiled = compiled_result?;
        used_points = charge_points(geometry_point_limit, used_points, compiled.points.len())?;
        lane_outputs.push(PendingLaneGeometry {
            target: edge.header.source_address.clone(),
            value: Some(CompiledLaneEdgeGeometry {
                length: compiled.length,
                canonical_frame: None,
                centerline_points: compiled.points,
                source_ranges: compiled.source_ranges,
            }),
        });
    }

    for plan in &plans {
        let authoring = plan
            .corridor
            .authoring_geometry
            .as_ref()
            .expect("resolved corridor plans retain authoring geometry");
        let alignment = compiled_alignments
            .binary_search_by(|entry| {
                entry
                    .declaration
                    .road_alignment_key
                    .as_bytes()
                    .cmp(plan.alignment.road_alignment_key.as_bytes())
            })
            .ok()
            .map(|index| &compiled_alignments[index])
            .ok_or(NumericFreezeError::GeometryTopologyMismatch)?;
        let station_end = match authoring.end_station {
            AuthoringStationEnd::Finite(value) => value,
            AuthoringStationEnd::AlignmentEnd => {
                alignment
                    .reference
                    .station_rows
                    .last()
                    .ok_or(NumericFreezeError::StationOutOfRange)?
                    .cumulative_end_meters
            }
        };

        scratch.reserve::<CorridorMember<'_>>(plan.member_count)?;
        scratch.reserve::<AuthoringWidthProfile>(plan.member_count)?;
        scratch.reserve::<MemberOffsetEndpoints>(plan.member_count)?;
        let mut members = Vec::with_capacity(plan.member_count);
        expand_corridor_members(&mut members, &plan.elements);
        debug_assert_eq!(members.len(), plan.member_count);
        let mut width_profiles = Vec::with_capacity(plan.member_count);
        width_profiles.extend(members.iter().map(|member| member.width_profile));
        let offsets = derive_member_offset_endpoints(&width_profiles, plan.reference_ordinal)?;

        for (member, offset) in members.iter().zip(offsets.iter()) {
            let lane_direction = match &member.target {
                CorridorMemberTarget::LaneEdge { direction, .. } => *direction,
                CorridorMemberTarget::FacilityBand { .. } => AuthoringLaneDirection::Forward,
            };
            scratch.reserve_bytes(numeric_scratch_bytes)?;
            let compiled_result = compile_offset_curve(
                &alignment.declaration.reference_line,
                &alignment.reference,
                authoring.start_station_meters,
                station_end,
                offset.start_meters,
                offset.end_meters,
                lane_direction,
                accuracy,
                direction,
                remaining_points(geometry_point_limit, used_points)?,
            );
            scratch.release_bytes(numeric_scratch_bytes);
            let compiled = compiled_result?;
            used_points = charge_points(geometry_point_limit, used_points, compiled.points.len())?;
            match &member.target {
                CorridorMemberTarget::LaneEdge { target, .. } => {
                    lane_outputs.push(PendingLaneGeometry {
                        target: (*target).clone(),
                        value: Some(CompiledLaneEdgeGeometry {
                            length: compiled.length,
                            canonical_frame: Some(alignment.declaration.canonical_frame.clone()),
                            centerline_points: compiled.points,
                            source_ranges: compiled.source_ranges,
                        }),
                    });
                }
                CorridorMemberTarget::FacilityBand { target } => {
                    facility_outputs.push(PendingFacilityGeometry {
                        target: (*target).clone(),
                        value: Some(CompiledFacilityBandGeometry {
                            length: compiled.length,
                            canonical_frame: alignment.declaration.canonical_frame.clone(),
                            centerline_points: compiled.points,
                            source_ranges: compiled.source_ranges,
                        }),
                    });
                }
            }
        }
        drop(offsets);
        drop(width_profiles);
        drop(members);
        scratch.release::<MemberOffsetEndpoints>(plan.member_count);
        scratch.release::<AuthoringWidthProfile>(plan.member_count);
        scratch.release::<CorridorMember<'_>>(plan.member_count);
    }

    lane_outputs.sort_unstable_by(|left, right| left.target.cmp(&right.target));
    facility_outputs.sort_unstable_by(|left, right| left.target.cmp(&right.target));
    if lane_outputs
        .windows(2)
        .any(|pair| pair[0].target == pair[1].target)
        || facility_outputs
            .windows(2)
            .any(|pair| pair[0].target == pair[1].target)
    {
        return Err(NumericFreezeError::GeometryTopologyMismatch.into());
    }

    if expected_lane_outputs != lane_outputs.len()
        || expected_facility_outputs != facility_outputs.len()
        || declarations.iter().any(|declaration| match declaration {
            TypedAstDeclaration::LaneEdge(edge)
                if matches!(
                    edge.geometry_authority,
                    LaneEdgeGeometryAuthority::Authoring { .. }
                ) =>
            {
                lane_outputs
                    .binary_search_by(|output| output.target.cmp(&edge.header.source_address))
                    .is_err()
            }
            TypedAstDeclaration::FacilityBand(facility)
                if facility.authoring_width_profile.is_some() =>
            {
                facility_outputs
                    .binary_search_by(|output| output.target.cmp(&facility.header.source_address))
                    .is_err()
            }
            _ => false,
        })
    {
        return Err(NumericFreezeError::GeometryTopologyMismatch.into());
    }

    let plan_count = plans.len();
    let resolved_element_count = plans
        .iter()
        .try_fold(0_usize, |total, plan| {
            total.checked_add(plan.elements.len())
        })
        .ok_or(GeometryCompilationError::ScratchLimit {
            limit: geometry_scratch_byte_limit,
            observed: u64::MAX,
        })?;
    drop(plans);
    scratch.release::<ResolvedCorridorElement<'_>>(resolved_element_count);
    scratch.release::<ResolvedCorridorPlan<'_>>(plan_count);
    drop(compiled_alignments);
    scratch.release::<ReferenceStationRow>(station_row_count);
    scratch.release::<u32>(regularity_visit_count);
    scratch.release::<CompiledAlignmentEntry<'_>>(referenced_alignment_count);

    for declaration in declarations.iter_mut() {
        match declaration {
            TypedAstDeclaration::LaneEdge(edge) => {
                if matches!(
                    edge.geometry_authority,
                    LaneEdgeGeometryAuthority::Authoring { .. }
                ) {
                    let index = lane_outputs
                        .binary_search_by(|output| output.target.cmp(&edge.header.source_address))
                        .expect("geometry targets were prevalidated before atomic replacement");
                    let geometry = lane_outputs[index]
                        .value
                        .take()
                        .expect("each prevalidated geometry target is consumed once");
                    edge.geometry_authority = LaneEdgeGeometryAuthority::Compiled(geometry);
                }
            }
            TypedAstDeclaration::RoadCorridor(corridor) => {
                corridor.authoring_geometry = None;
            }
            TypedAstDeclaration::RoadSection(section) => {
                for lane in &mut section.lanes {
                    lane.authoring_geometry = None;
                }
            }
            TypedAstDeclaration::FacilityBand(facility)
                if facility.authoring_width_profile.is_some() =>
            {
                let index = facility_outputs
                    .binary_search_by(|output| output.target.cmp(&facility.header.source_address))
                    .expect("geometry targets were prevalidated before atomic replacement");
                facility.compiled_geometry = facility_outputs[index].value.take();
                facility.authoring_width_profile = None;
            }
            _ => {}
        }
    }
    debug_assert!(lane_outputs.iter().all(|output| output.value.is_none()));
    debug_assert!(facility_outputs.iter().all(|output| output.value.is_none()));
    drop(lane_outputs);
    drop(facility_outputs);
    scratch.release::<PendingLaneGeometry>(expected_lane_outputs);
    scratch.release::<PendingFacilityGeometry>(expected_facility_outputs);
    debug_assert_eq!(scratch.live, 0);
    Ok(GeometryCompilationUsage {
        output_point_count: used_points,
        peak_scratch_bytes: scratch.peak,
    })
}

fn resolve_corridor_plans<'a>(
    authoring_namespace_id: &str,
    alignments: &'a [RoadAlignmentDeclaration],
    declarations: &'a [TypedAstDeclaration],
    scratch: &mut GeometryScratchBudget,
) -> Result<Vec<ResolvedCorridorPlan<'a>>, GeometryCompilationError> {
    let index = GeometryDeclarationIndex::new(declarations, scratch)?;
    scratch.reserve::<&RoadAlignmentDeclaration>(alignments.len())?;
    let mut alignment_index: Vec<_> = Vec::with_capacity(alignments.len());
    alignment_index.extend(alignments);
    alignment_index.sort_unstable_by(|left, right| {
        left.road_alignment_key
            .as_bytes()
            .cmp(right.road_alignment_key.as_bytes())
    });
    if alignment_index
        .windows(2)
        .any(|pair| pair[0].road_alignment_key == pair[1].road_alignment_key)
    {
        return Err(NumericFreezeError::GeometryTopologyMismatch.into());
    }

    let corridor_count = declarations
        .iter()
        .filter(|value| {
            matches!(
                value,
                TypedAstDeclaration::RoadCorridor(corridor)
                    if corridor.authoring_geometry.is_some()
            )
        })
        .count();
    scratch.reserve::<ResolvedCorridorPlan<'_>>(corridor_count)?;
    let mut plans = Vec::with_capacity(corridor_count);
    for declaration in declarations {
        let TypedAstDeclaration::RoadCorridor(corridor) = declaration else {
            continue;
        };
        let Some(authoring) = &corridor.authoring_geometry else {
            continue;
        };
        let alignment = alignment_index
            .binary_search_by(|value| {
                value
                    .road_alignment_key
                    .as_bytes()
                    .cmp(authoring.road_alignment_key.as_bytes())
            })
            .ok()
            .map(|ordinal| alignment_index[ordinal])
            .ok_or(NumericFreezeError::GeometryTopologyMismatch)?;
        let corridor_key = corridor.header.source_address.local_key().as_ref();
        if corridor.reference_section.module_namespace.as_ref() != authoring_namespace_id
            || authoring.reference_lane.module_namespace.as_ref() != authoring_namespace_id
            || !address_has_owners(&corridor.reference_section.target_address, &[corridor_key])
        {
            return Err(NumericFreezeError::GeometryTopologyMismatch.into());
        }
        let reference_section = index
            .section(&corridor.reference_section.target_address)
            .ok_or(NumericFreezeError::GeometryTopologyMismatch)?;
        let reference_section_key = reference_section.header.source_address.local_key().as_ref();
        if !address_has_owners(
            &authoring.reference_lane.target_address,
            &[corridor_key, reference_section_key],
        ) {
            return Err(NumericFreezeError::GeometryTopologyMismatch.into());
        }

        scratch.reserve::<ResolvedCorridorElement<'_>>(corridor.elements.len())?;
        let mut elements = Vec::with_capacity(corridor.elements.len());
        let mut member_count = 0_usize;
        let mut reference_ordinal = None;
        let mut saw_reference_section = false;
        for element in &corridor.elements {
            match element {
                OwnedCorridorElementReference::RoadSection(reference) => {
                    if reference.module_namespace.as_ref() != authoring_namespace_id
                        || !address_has_owners(&reference.target_address, &[corridor_key])
                    {
                        return Err(NumericFreezeError::GeometryTopologyMismatch.into());
                    }
                    let section = index
                        .section(&reference.target_address)
                        .ok_or(NumericFreezeError::GeometryTopologyMismatch)?;
                    let section_key = section.header.source_address.local_key().as_ref();
                    if !address_has_owners(&section.header.source_address, &[corridor_key]) {
                        return Err(NumericFreezeError::GeometryTopologyMismatch.into());
                    }
                    let is_reference_section =
                        section.header.source_address == reference_section.header.source_address;
                    if is_reference_section && saw_reference_section {
                        return Err(NumericFreezeError::GeometryTopologyMismatch.into());
                    }
                    let section_member_start = member_count;
                    for (lane_ordinal, lane) in section.lanes.iter().enumerate() {
                        if !address_has_owners(
                            &lane.header.source_address,
                            &[corridor_key, section_key],
                        ) {
                            return Err(NumericFreezeError::GeometryTopologyMismatch.into());
                        }
                        let geometry = lane
                            .authoring_geometry
                            .as_ref()
                            .ok_or(NumericFreezeError::GeometryTopologyMismatch)?;
                        let [edge] = lane.edge_chain.as_ref() else {
                            return Err(NumericFreezeError::GeometryTopologyMismatch.into());
                        };
                        if edge.module_namespace.as_ref() != authoring_namespace_id {
                            return Err(NumericFreezeError::GeometryTopologyMismatch.into());
                        }
                        if is_reference_section
                            && lane.header.source_address == authoring.reference_lane.target_address
                        {
                            if geometry.direction != AuthoringLaneDirection::Forward
                                || reference_ordinal.is_some()
                            {
                                return Err(NumericFreezeError::GeometryTopologyMismatch.into());
                            }
                            reference_ordinal = section_member_start.checked_add(lane_ordinal);
                            if reference_ordinal.is_none() {
                                return Err(GeometryCompilationError::ScratchLimit {
                                    limit: scratch.limit,
                                    observed: u64::MAX,
                                });
                            }
                        }
                    }
                    member_count = member_count.checked_add(section.lanes.len()).ok_or(
                        GeometryCompilationError::ScratchLimit {
                            limit: scratch.limit,
                            observed: u64::MAX,
                        },
                    )?;
                    if is_reference_section {
                        saw_reference_section = true;
                    }
                    elements.push(ResolvedCorridorElement::RoadSection(section));
                }
                OwnedCorridorElementReference::FacilityBand(reference) => {
                    if reference.module_namespace.as_ref() != authoring_namespace_id
                        || !address_has_owners(&reference.target_address, &[corridor_key])
                    {
                        return Err(NumericFreezeError::GeometryTopologyMismatch.into());
                    }
                    let facility = index
                        .facility_band(&reference.target_address)
                        .ok_or(NumericFreezeError::GeometryTopologyMismatch)?;
                    if !address_has_owners(&facility.header.source_address, &[corridor_key])
                        || facility.authoring_width_profile.is_none()
                    {
                        return Err(NumericFreezeError::GeometryTopologyMismatch.into());
                    }
                    member_count = member_count.checked_add(1).ok_or(
                        GeometryCompilationError::ScratchLimit {
                            limit: scratch.limit,
                            observed: u64::MAX,
                        },
                    )?;
                    elements.push(ResolvedCorridorElement::FacilityBand(facility));
                }
            }
        }
        if !saw_reference_section || reference_ordinal.is_none() {
            return Err(NumericFreezeError::GeometryTopologyMismatch.into());
        }
        plans.push(ResolvedCorridorPlan {
            corridor,
            alignment,
            elements: elements.into_boxed_slice(),
            member_count,
            reference_ordinal: reference_ordinal.expect("reference lane was validated"),
        });
    }
    plans.sort_unstable_by(|left, right| {
        left.alignment
            .road_alignment_key
            .as_bytes()
            .cmp(right.alignment.road_alignment_key.as_bytes())
            .then_with(|| {
                left.corridor
                    .authoring_geometry
                    .as_ref()
                    .expect("resolved plan")
                    .start_station_meters
                    .total_cmp(
                        &right
                            .corridor
                            .authoring_geometry
                            .as_ref()
                            .expect("resolved plan")
                            .start_station_meters,
                    )
            })
            .then_with(|| {
                left.corridor
                    .header
                    .source_address
                    .cmp(&right.corridor.header.source_address)
            })
    });
    let section_count = index.sections.len();
    let facility_count = index.facility_bands.len();
    drop(index);
    scratch.release::<&RoadSectionDeclaration>(section_count);
    scratch.release::<&FacilityBandDeclaration>(facility_count);
    drop(alignment_index);
    scratch.release::<&RoadAlignmentDeclaration>(alignments.len());
    Ok(plans)
}

fn validate_station_partitions(
    plans: &[ResolvedCorridorPlan<'_>],
) -> Result<(), NumericFreezeError> {
    let mut group_start = 0;
    while group_start < plans.len() {
        let alignment_key = &plans[group_start].alignment.road_alignment_key;
        let mut group_end = group_start + 1;
        while group_end < plans.len()
            && plans[group_end].alignment.road_alignment_key == *alignment_key
        {
            group_end += 1;
        }
        let group = &plans[group_start..group_end];
        let first = group[0]
            .corridor
            .authoring_geometry
            .as_ref()
            .expect("resolved plan");
        if first.start_station_meters.to_bits() != 0.0_f64.to_bits() {
            return Err(NumericFreezeError::GeometryTopologyMismatch);
        }
        for (index, plan) in group.iter().enumerate() {
            let authoring = plan
                .corridor
                .authoring_geometry
                .as_ref()
                .expect("resolved plan");
            let is_last = index + 1 == group.len();
            match authoring.end_station {
                AuthoringStationEnd::Finite(end) if !is_last => {
                    let next = group[index + 1]
                        .corridor
                        .authoring_geometry
                        .as_ref()
                        .expect("resolved plan");
                    if end.to_bits() != next.start_station_meters.to_bits() {
                        return Err(NumericFreezeError::GeometryTopologyMismatch);
                    }
                }
                AuthoringStationEnd::AlignmentEnd if is_last => {}
                _ => return Err(NumericFreezeError::GeometryTopologyMismatch),
            }
        }
        group_start = group_end;
    }
    Ok(())
}

fn address_has_owners(address: &TypedAstEntityAddress, expected: &[&str]) -> bool {
    address.owner_local_keys().len() == expected.len()
        && address
            .owner_local_keys()
            .iter()
            .zip(expected)
            .all(|(actual, expected)| actual.as_ref() == *expected)
}

fn expand_corridor_members<'a>(
    members: &mut Vec<CorridorMember<'a>>,
    elements: &'a [ResolvedCorridorElement<'a>],
) {
    for element in elements {
        match element {
            ResolvedCorridorElement::RoadSection(section) => {
                append_resolved_section_members(members, section);
            }
            ResolvedCorridorElement::FacilityBand(facility) => {
                members.push(CorridorMember {
                    width_profile: facility
                        .authoring_width_profile
                        .expect("resolved facility retains its authoring width"),
                    target: CorridorMemberTarget::FacilityBand {
                        target: &facility.header.source_address,
                    },
                });
            }
        }
    }
}

fn append_resolved_section_members<'a>(
    members: &mut Vec<CorridorMember<'a>>,
    section: &'a RoadSectionDeclaration,
) {
    for lane in &section.lanes {
        let geometry = lane
            .authoring_geometry
            .as_ref()
            .expect("resolved lane retains its authoring geometry");
        let [edge] = lane.edge_chain.as_ref() else {
            unreachable!("resolved lane has exactly one edge target");
        };
        members.push(CorridorMember {
            width_profile: geometry.width_profile,
            target: CorridorMemberTarget::LaneEdge {
                target: &edge.target_address,
                direction: geometry.direction,
            },
        });
    }
}

fn remaining_points(limit: u64, used: u64) -> Result<u64, NumericFreezeError> {
    limit
        .checked_sub(used)
        .ok_or(NumericFreezeError::GeometryPointLimit)
}

fn charge_points(limit: u64, used: u64, additional: usize) -> Result<u64, NumericFreezeError> {
    let additional =
        u64::try_from(additional).map_err(|_| NumericFreezeError::GeometryPointLimit)?;
    let total = used
        .checked_add(additional)
        .ok_or(NumericFreezeError::GeometryPointLimit)?;
    if total > limit {
        return Err(NumericFreezeError::GeometryPointLimit);
    }
    Ok(total)
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
        limit_error: NumericFreezeError::GeometryPointLimit,
        last_point: None,
    };
    walk_reference_program(program, accuracy, direction, &mut output_counter)?;
    let expected_points = usize::try_from(output_counter.count)
        .map_err(|_| NumericFreezeError::GeometryPointLimit)?;
    let mut point_collector = ExactPointSink {
        points: Vec::with_capacity(expected_points),
        expected_points,
        active_source: None,
        source_ranges: Vec::with_capacity(program.segments.len()),
    };
    walk_reference_program(program, accuracy, direction, &mut point_collector)?;
    if point_collector.points.len() != point_collector.expected_points {
        return Err(NumericFreezeError::ApproximationNotConverged);
    }
    let length = validate_canonical_polyline(&point_collector.points, direction)?;
    let length =
        EdgeLength::try_new(length).map_err(|_| NumericFreezeError::DegenerateCanonicalSegment)?;
    let source_ranges = point_collector.finish_source_ranges()?;
    Ok(CompiledCurve {
        length,
        points: point_collector.points.into_boxed_slice(),
        source_ranges,
    })
}

pub(super) fn compile_alignment_reference(
    program: &AuthoringCurveProgramDeclaration,
    station_row_byte_limit: u64,
) -> Result<CompiledAlignmentReference, NumericFreezeError> {
    let station_vertex_limit = station_vertex_limit_from_bytes(station_row_byte_limit)?;
    let sizing = measure_alignment_reference(
        program,
        station_vertex_limit,
        NumericFreezeError::StationRowLimit,
    )?;
    compile_measured_alignment_reference(program, sizing)
}

fn station_vertex_limit_from_bytes(station_row_byte_limit: u64) -> Result<u64, NumericFreezeError> {
    let station_row_size = u64::try_from(core::mem::size_of::<ReferenceStationRow>())
        .map_err(|_| NumericFreezeError::StationRowLimit)?;
    station_row_byte_limit
        .checked_div(station_row_size)
        .and_then(|row_limit| row_limit.checked_add(1))
        .ok_or(NumericFreezeError::StationRowLimit)
}

fn measure_alignment_reference(
    program: &AuthoringCurveProgramDeclaration,
    transient_vertex_limit: u64,
    limit_error: NumericFreezeError,
) -> Result<AlignmentReferenceSizing, NumericFreezeError> {
    let mut visits = Vec::with_capacity(program.segments.len());
    let mut start = point3(program.start)?;
    for source in &program.segments {
        let (segment, end) = source_segment(start, source)?;
        visits.push(segment.prove_horizontal_regularity()?);
        start = end;
    }
    let mut station_counter = CountingSink {
        count: 0,
        limit: transient_vertex_limit,
        limit_error,
        last_point: None,
    };
    walk_reference_program(
        program,
        GeometryAccuracyProfile::Fine2Cm,
        GeometryDirectionProfile::Smooth1Deg,
        &mut station_counter,
    )?;
    let expected_station_rows =
        usize::try_from(station_counter.count.saturating_sub(1)).map_err(|_| limit_error)?;
    Ok(AlignmentReferenceSizing {
        station_rows: expected_station_rows,
        horizontal_regularity_visits: visits.into_boxed_slice(),
    })
}

fn compile_measured_alignment_reference(
    program: &AuthoringCurveProgramDeclaration,
    sizing: AlignmentReferenceSizing,
) -> Result<CompiledAlignmentReference, NumericFreezeError> {
    let mut station_collector = StationRowSink {
        rows: Vec::with_capacity(sizing.station_rows),
        active_segment: None,
        cumulative_meters: 0.0,
        seen_first_point: false,
        expected_rows: sizing.station_rows,
    };
    walk_reference_program(
        program,
        GeometryAccuracyProfile::Fine2Cm,
        GeometryDirectionProfile::Smooth1Deg,
        &mut station_collector,
    )?;
    if station_collector.rows.len() != station_collector.expected_rows
        || sizing.horizontal_regularity_visits.len() != program.segments.len()
    {
        return Err(NumericFreezeError::ApproximationNotConverged);
    }
    Ok(CompiledAlignmentReference {
        station_rows: station_collector.rows.into_boxed_slice(),
        horizontal_regularity_visits: sizing.horizontal_regularity_visits,
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
        limit_error: NumericFreezeError::GeometryPointLimit,
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
        active_source: None,
        source_ranges: Vec::with_capacity(program.segments.len()),
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
        collector.reverse_source_ranges()?;
    }
    let length = validate_canonical_polyline(&collector.points, direction)?;
    let length =
        EdgeLength::try_new(length).map_err(|_| NumericFreezeError::DegenerateCanonicalSegment)?;
    let source_ranges = collector.finish_source_ranges()?;
    Ok(CompiledCurve {
        length,
        points: collector.points.into_boxed_slice(),
        source_ranges,
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
    let mut emitted_endpoint = None;
    for (segment_index, source) in program.segments.iter().enumerate() {
        let (segment, end) = source_segment(start, source)?;
        let segment_ordinal =
            u32::try_from(segment_index).map_err(|_| NumericFreezeError::GeometryPointLimit)?;
        let mut began_source_segment = false;
        while let Some(row) = station_rows.get(row_index) {
            if row.segment_ordinal != segment_ordinal {
                break;
            }
            let station_start = row.cumulative_start_meters.max(corridor_start_meters);
            let station_end = row.cumulative_end_meters.min(corridor_end_meters);
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
            if sink.last_point().is_none()
                && station_start == corridor_start_meters
                && station_start == station_end
            {
                let parameter = parameter_in_station_row(row, corridor_start_meters)?;
                sink.push(ApproximationVertex {
                    parameter,
                    point: quantize_point(evaluator.evaluate(parameter)?.point)?,
                })?;
                emitted_endpoint = Some((segment_ordinal, evaluator, parameter));
            }
            if station_start < station_end {
                if !began_source_segment {
                    sink.begin_source_segment(segment_ordinal, &source.span)?;
                    began_source_segment = true;
                }
                let parameter_start = parameter_in_station_row(row, station_start)?;
                let parameter_end = parameter_in_station_row(row, station_end)?;
                let source_boundary =
                    emitted_endpoint.is_some_and(|(ordinal, _, _)| ordinal != segment_ordinal);
                let welded_start = if let Some((_, previous_evaluator, previous_parameter)) =
                    emitted_endpoint
                {
                    let previous = sink
                        .last_point()
                        .ok_or(NumericFreezeError::DegenerateCanonicalSegment)?;
                    let actual = quantize_point(evaluator.evaluate(parameter_start)?.point)?;
                    if source_boundary {
                        if canonical_point_distance(previous, actual)? > MAX_SOURCE_JOIN_GAP_METERS
                        {
                            return Err(NumericFreezeError::SourceJoinGapExceeded);
                        }
                        let previous_first = previous_evaluator.evaluate(previous_parameter)?.first;
                        let current_first = evaluator.evaluate(parameter_start)?.first;
                        if !direction_accepts(
                            previous_first,
                            current_first,
                            full_angle_cosine_squared(direction),
                        )? {
                            return Err(NumericFreezeError::DirectionDiscontinuity);
                        }
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
                emitted_endpoint = Some((segment_ordinal, evaluator, parameter_end));
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
    let mut welded_start = None;
    for (segment_index, source) in program.segments.iter().enumerate() {
        let (segment, end) = source_segment(start, source)?;
        let segment_ordinal =
            u32::try_from(segment_index).map_err(|_| NumericFreezeError::GeometryPointLimit)?;
        sink.begin_segment(segment_ordinal, segment, &source.span)?;
        welded_start = Some(approximate_interval(
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
        )?);
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
        source: &crate::SourceLocation,
    ) -> Result<(), NumericFreezeError>;
}

fn point3(value: AuthoringPoint3F64) -> Result<Point3, NumericFreezeError> {
    Point3::try_new(value.x, value.y, value.z)
}

struct CountingSink {
    count: u64,
    limit: u64,
    limit_error: NumericFreezeError,
    last_point: Option<ApproximationPoint>,
}

impl ApproximationPointSink for CountingSink {
    fn push(&mut self, vertex: ApproximationVertex) -> Result<(), NumericFreezeError> {
        if self.count == self.limit {
            return Err(self.limit_error);
        }
        self.count += 1;
        self.last_point = Some(vertex.point);
        Ok(())
    }
}

trait CanonicalPointSink: ApproximationPointSink {
    fn last_point(&self) -> Option<ApproximationPoint>;

    fn begin_source_segment(
        &mut self,
        segment_ordinal: u32,
        source: &crate::SourceLocation,
    ) -> Result<(), NumericFreezeError>;
}

impl CanonicalPointSink for CountingSink {
    fn last_point(&self) -> Option<ApproximationPoint> {
        self.last_point
    }

    fn begin_source_segment(
        &mut self,
        _segment_ordinal: u32,
        _source: &crate::SourceLocation,
    ) -> Result<(), NumericFreezeError> {
        Ok(())
    }
}

impl ReferenceProgramSink for CountingSink {
    fn begin_segment(
        &mut self,
        _segment_ordinal: u32,
        _segment: CurveSegment,
        _source: &crate::SourceLocation,
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
            if self.rows.len() == self.expected_rows {
                return Err(NumericFreezeError::ApproximationNotConverged);
            }
            let current_point = active.evaluator.evaluate(vertex.parameter)?.point;
            let chord_length = point_distance(active.previous_point, current_point)?;
            if chord_length == 0.0 {
                return Err(NumericFreezeError::DegenerateCanonicalSegment);
            }
            let cumulative_end = self.cumulative_meters + chord_length;
            if !cumulative_end.is_finite() {
                return Err(NumericFreezeError::NonFinite);
            }
            if cumulative_end <= self.cumulative_meters {
                return Err(NumericFreezeError::DegenerateCanonicalSegment);
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
        _source: &crate::SourceLocation,
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
    active_source: Option<ActivePointSource>,
    source_ranges: Vec<CompiledGeometrySourceRange>,
}

struct ActivePointSource {
    segment_ordinal: u32,
    point_start: usize,
    source: crate::SourceLocation,
}

impl ExactPointSink {
    fn begin_source(
        &mut self,
        segment_ordinal: u32,
        source: &crate::SourceLocation,
    ) -> Result<(), NumericFreezeError> {
        if self
            .active_source
            .as_ref()
            .is_some_and(|active| active.segment_ordinal == segment_ordinal)
        {
            return Ok(());
        }
        let point_start = if self.active_source.is_some() && !self.points.is_empty() {
            self.points.len() - 1
        } else {
            self.points.len()
        };
        self.finish_active_source(point_start)?;
        self.active_source = Some(ActivePointSource {
            segment_ordinal,
            point_start,
            source: source.clone(),
        });
        Ok(())
    }

    fn finish_active_source(
        &mut self,
        point_end_exclusive: usize,
    ) -> Result<(), NumericFreezeError> {
        let Some(active) = self.active_source.take() else {
            return Ok(());
        };
        if active.point_start < point_end_exclusive {
            self.source_ranges.push(CompiledGeometrySourceRange {
                point_start: u32::try_from(active.point_start)
                    .map_err(|_| NumericFreezeError::GeometryPointLimit)?,
                point_end_exclusive: u32::try_from(point_end_exclusive)
                    .map_err(|_| NumericFreezeError::GeometryPointLimit)?,
                source_segment_ordinal: active.segment_ordinal,
                source: active.source,
            });
        }
        Ok(())
    }

    fn finish_source_ranges(
        &mut self,
    ) -> Result<Box<[CompiledGeometrySourceRange]>, NumericFreezeError> {
        self.finish_active_source(self.points.len())?;
        Ok(core::mem::take(&mut self.source_ranges).into_boxed_slice())
    }

    fn reverse_source_ranges(&mut self) -> Result<(), NumericFreezeError> {
        self.finish_active_source(self.points.len())?;
        let point_count =
            u32::try_from(self.points.len()).map_err(|_| NumericFreezeError::GeometryPointLimit)?;
        for range in &mut self.source_ranges {
            let new_start = point_count
                .checked_sub(range.point_end_exclusive)
                .ok_or(NumericFreezeError::GeometryPointLimit)?;
            let new_end_exclusive = point_count
                .checked_sub(range.point_start)
                .ok_or(NumericFreezeError::GeometryPointLimit)?;
            range.point_start = new_start;
            range.point_end_exclusive = new_end_exclusive;
        }
        self.source_ranges.reverse();
        Ok(())
    }
}

impl ApproximationPointSink for ExactPointSink {
    fn push(&mut self, vertex: ApproximationVertex) -> Result<(), NumericFreezeError> {
        if self.points.len() == self.expected_points {
            return Err(NumericFreezeError::ApproximationNotConverged);
        }
        self.points.push(vertex.point);
        Ok(())
    }
}

impl CanonicalPointSink for ExactPointSink {
    fn last_point(&self) -> Option<ApproximationPoint> {
        self.points.last().copied()
    }

    fn begin_source_segment(
        &mut self,
        segment_ordinal: u32,
        source: &crate::SourceLocation,
    ) -> Result<(), NumericFreezeError> {
        self.begin_source(segment_ordinal, source)
    }
}

impl ReferenceProgramSink for ExactPointSink {
    fn begin_segment(
        &mut self,
        segment_ordinal: u32,
        _segment: CurveSegment,
        source: &crate::SourceLocation,
    ) -> Result<(), NumericFreezeError> {
        self.begin_source(segment_ordinal, source)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::declaration::{
        AuthoringCurveSegmentDeclaration, AuthoringCurveSegmentGeometry, AuthoringPoint3F64,
    };
    use crate::{CompileLimits, SourceSpan};

    fn point(x: f64, z: f64) -> AuthoringPoint3F64 {
        AuthoringPoint3F64 { x, y: 0.0, z }
    }

    fn span(column: u32) -> crate::SourceLocation {
        SourceSpan::point(Arc::from("roads/main"), 1, column).into()
    }

    fn station_row_bytes(rows: u64) -> u64 {
        rows * u64::try_from(core::mem::size_of::<ReferenceStationRow>()).unwrap()
    }

    fn lowered_geometry_fixture() -> (Box<[RoadAlignmentDeclaration]>, Vec<TypedAstDeclaration>) {
        let limits = CompileLimits::p100_initial_v2();
        let module = super::super::writer::tests::module_with_every_declaration(&limits);
        let bytes = super::super::writer::RoadEditingSourceWriter::new(&limits)
            .write(module)
            .unwrap();
        let input = super::super::input::RoadEditingModuleInput::try_new(
            "road-editing",
            bytes.as_bytes(),
            None,
        )
        .unwrap();
        let verified = super::super::reader::verify_source(input, &limits, 0, 0).unwrap();
        let locations =
            super::super::location::RoadEditingLocationFactory::from_verified_root(verified.root());
        let shared_namespace = Arc::from(verified.root().module_header().authoring_namespace_id());
        let alignments = super::super::lowering::lower_road_alignments(
            verified.root(),
            &locations,
            &shared_namespace,
        );
        let mut declarations = super::super::lowering::lower_topology_authoring_declarations(
            verified.root(),
            &locations,
            &shared_namespace,
        )
        .unwrap();
        declarations.extend(super::super::lowering::lower_owner_scoped_declarations(
            verified.root(),
            &locations,
            &shared_namespace,
        ));
        declarations.retain(|declaration| {
            !matches!(
                declaration,
                TypedAstDeclaration::LaneEdge(edge)
                    if edge.header.stable_key.as_ref() == "edge-b"
            )
        });
        (alignments.into_boxed_slice(), declarations)
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
        assert_eq!(compiled.source_ranges.len(), 1);
        assert_eq!(compiled.source_ranges[0].point_start, 0);
        assert_eq!(compiled.source_ranges[0].point_end_exclusive, 2);
        assert_eq!(compiled.source_ranges[0].source_segment_ordinal, 0);
        assert_eq!(compiled.source_ranges[0].source, span(2));
        let reference = compile_alignment_reference(&program, station_row_bytes(1)).unwrap();
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
    fn station_rows_require_strict_cumulative_progress() {
        let start = Point3::try_new(0.0, 0.0, 0.0).expect("start");
        let end = Point3::try_new(1.0, 0.0, 0.0).expect("end");
        let segment = CurveSegment::Line { start, end };
        let mut sink = StationRowSink {
            rows: Vec::with_capacity(1),
            active_segment: Some(ActiveStationSegment {
                ordinal: 0,
                evaluator: segment,
                previous_parameter: 0.0,
                previous_point: start,
            }),
            cumulative_meters: 1.0e20,
            seen_first_point: true,
            expected_rows: 1,
        };

        assert_eq!(
            sink.push(ApproximationVertex {
                parameter: 1.0,
                point: CanonicalPoint3F32Input {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            }),
            Err(NumericFreezeError::DegenerateCanonicalSegment)
        );
        assert!(sink.rows.is_empty());
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
        let reference = compile_alignment_reference(&program, station_row_bytes(153)).unwrap();
        assert_eq!(reference.station_rows.len(), 153);
        assert_eq!(reference.horizontal_regularity_visits.as_ref(), [3]);
        assert_eq!(reference.station_rows.last().unwrap().parameter_end, 1.0);
        let compact = compile_explicit_curve(
            &program,
            GeometryAccuracyProfile::Compact10Cm,
            GeometryDirectionProfile::Compact5Deg,
            153,
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
        assert_eq!(
            compile_alignment_reference(&program, station_row_bytes(152)).err(),
            Some(NumericFreezeError::StationRowLimit)
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

    #[test]
    fn adjacent_segments_reuse_the_endpoint_bits_emitted_by_the_evaluator() {
        let source_endpoint = 1.000_000_048_045_901_5e-10;
        let program = AuthoringCurveProgramDeclaration {
            start: point(16_384.0, 0.0),
            start_span: span(1),
            segments: vec![
                AuthoringCurveSegmentDeclaration {
                    geometry: AuthoringCurveSegmentGeometry::Line {
                        end: point(source_endpoint, 0.0),
                    },
                    span: span(2),
                },
                AuthoringCurveSegmentDeclaration {
                    geometry: AuthoringCurveSegmentGeometry::Line {
                        end: point(-1.0, 0.0),
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
        .expect("the second segment must weld to the endpoint actually emitted by the first");

        assert_eq!(compiled.points.len(), 3);
        assert_eq!(compiled.points[1].x.to_bits(), 0x2edc_0000);
        assert_ne!(
            compiled.points[1].x.to_bits(),
            (source_endpoint as f32).to_bits()
        );
    }

    #[test]
    fn horizontal_regularity_fails_before_station_subdivision_limits() {
        let program = AuthoringCurveProgramDeclaration {
            start: point(0.0, 0.0),
            start_span: span(1),
            segments: vec![AuthoringCurveSegmentDeclaration {
                geometry: AuthoringCurveSegmentGeometry::CubicBezier {
                    control_1: point(0.0, 0.0),
                    control_2: point(0.0, 0.0),
                    end: point(0.0, 0.0),
                },
                span: span(2),
            }]
            .into_boxed_slice(),
        };

        assert_eq!(
            compile_alignment_reference(&program, 0).err(),
            Some(NumericFreezeError::HorizontalDerivativeZero)
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
        let reference = compile_alignment_reference(&program, station_row_bytes(2)).unwrap();
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
    fn generated_point_ranges_assign_shared_boundaries_to_the_following_segment() {
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
            3,
        )
        .unwrap();

        assert_eq!(compiled.source_ranges.len(), 2);
        assert_eq!(compiled.source_ranges[0].point_start, 0);
        assert_eq!(compiled.source_ranges[0].point_end_exclusive, 1);
        assert_eq!(compiled.source_ranges[0].source_segment_ordinal, 0);
        assert_eq!(compiled.source_ranges[0].source, span(2));
        assert_eq!(compiled.source_ranges[1].point_start, 1);
        assert_eq!(compiled.source_ranges[1].point_end_exclusive, 3);
        assert_eq!(compiled.source_ranges[1].source_segment_ordinal, 1);
        assert_eq!(compiled.source_ranges[1].source, span(3));
        let mut next_point = 0;
        for range in &compiled.source_ranges {
            assert_eq!(range.point_start, next_point);
            assert!(range.point_start < range.point_end_exclusive);
            next_point = range.point_end_exclusive;
        }
        assert_eq!(usize::try_from(next_point).unwrap(), compiled.points.len());

        let reference = compile_alignment_reference(&program, station_row_bytes(2)).unwrap();
        let backward = compile_offset_curve(
            &program,
            &reference,
            0.0,
            2.0,
            0.0,
            0.0,
            AuthoringLaneDirection::Backward,
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
            3,
        )
        .unwrap();
        assert_eq!(backward.source_ranges.len(), 2);
        assert_eq!(backward.source_ranges[0].point_start, 0);
        assert_eq!(backward.source_ranges[0].point_end_exclusive, 2);
        assert_eq!(backward.source_ranges[0].source_segment_ordinal, 1);
        assert_eq!(backward.source_ranges[0].source, span(3));
        assert_eq!(backward.source_ranges[1].point_start, 2);
        assert_eq!(backward.source_ranges[1].point_end_exclusive, 3);
        assert_eq!(backward.source_ranges[1].source_segment_ordinal, 0);
        assert_eq!(backward.source_ranges[1].source, span(2));
        let mut next_point = 0;
        for range in &backward.source_ranges {
            assert_eq!(range.point_start, next_point);
            assert!(range.point_start < range.point_end_exclusive);
            next_point = range.point_end_exclusive;
        }
        assert_eq!(usize::try_from(next_point).unwrap(), backward.points.len());
    }

    #[test]
    fn clipped_offset_does_not_assign_the_final_point_to_an_unvisited_source_segment() {
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
                AuthoringCurveSegmentDeclaration {
                    geometry: AuthoringCurveSegmentGeometry::Line {
                        end: point(3.0, 0.0),
                    },
                    span: span(4),
                },
            ]
            .into_boxed_slice(),
        };
        let reference = compile_alignment_reference(&program, station_row_bytes(3)).unwrap();
        let compiled = compile_offset_curve(
            &program,
            &reference,
            0.0,
            1.5,
            0.0,
            0.0,
            AuthoringLaneDirection::Forward,
            GeometryAccuracyProfile::Fine2Cm,
            GeometryDirectionProfile::Smooth1Deg,
            3,
        )
        .unwrap();

        assert_eq!(compiled.points.len(), 3);
        assert_eq!(compiled.source_ranges.len(), 2);
        assert_eq!(compiled.source_ranges[0].source_segment_ordinal, 0);
        assert_eq!(compiled.source_ranges[0].source, span(2));
        assert_eq!(compiled.source_ranges[1].point_end_exclusive, 3);
        assert_eq!(compiled.source_ranges[1].source_segment_ordinal, 1);
        assert_eq!(compiled.source_ranges[1].source, span(3));
        assert!(
            compiled
                .source_ranges
                .iter()
                .all(|range| range.source_segment_ordinal != 2 && range.source != span(4))
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
        let reference = compile_alignment_reference(&program, station_row_bytes(1)).unwrap();
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
        let reference = compile_alignment_reference(&accepted, station_row_bytes(2)).unwrap();
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
        let reference = compile_alignment_reference(&rejected, station_row_bytes(2)).unwrap();
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
    fn corridor_start_at_source_boundary_keeps_the_preceding_offset_owner() {
        let program = AuthoringCurveProgramDeclaration {
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
                        end: point(10.0, 10.0),
                    },
                    span: span(3),
                },
            ]
            .into_boxed_slice(),
        };
        let reference = compile_alignment_reference(&program, station_row_bytes(2)).unwrap();

        assert_eq!(
            compile_offset_curve(
                &program,
                &reference,
                10.0,
                20.0,
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
    fn corridor_start_at_source_boundary_checks_the_preceding_tangent() {
        let program = AuthoringCurveProgramDeclaration {
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
                        end: point(10.0, 10.0),
                    },
                    span: span(3),
                },
            ]
            .into_boxed_slice(),
        };
        let reference = compile_alignment_reference(&program, station_row_bytes(2)).unwrap();

        assert_eq!(
            compile_offset_curve(
                &program,
                &reference,
                10.0,
                20.0,
                0.0,
                0.0,
                AuthoringLaneDirection::Forward,
                GeometryAccuracyProfile::Fine2Cm,
                GeometryDirectionProfile::Smooth1Deg,
                3,
            )
            .err(),
            Some(NumericFreezeError::DirectionDiscontinuity)
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

    #[test]
    fn member_offsets_add_each_intermediate_width_once() {
        let reference_half = f64::from_bits(0x3ff6_13fd_14e5_b137);
        let intermediate = f64::from_bits(0x4005_f2f1_b98a_74a2);
        let outer = f64::from_bits(0x4009_0f04_8499_edb0);
        let profiles = |values: [f64; 3]| {
            values.map(|width| AuthoringWidthProfile {
                start_width_meters: width,
                end_width_meters: width,
            })
        };

        let left = derive_member_offset_endpoints(
            &profiles([outer, intermediate, 2.0 * reference_half]),
            2,
        )
        .unwrap();
        let expected_left = (reference_half + intermediate) + 0.5 * outer;
        let split_left = ((reference_half + 0.5 * intermediate) + 0.5 * intermediate) + 0.5 * outer;
        assert_ne!(expected_left.to_bits(), split_left.to_bits());
        assert_eq!(left[0].start_meters.to_bits(), expected_left.to_bits());

        let right = derive_member_offset_endpoints(
            &profiles([2.0 * reference_half, intermediate, outer]),
            0,
        )
        .unwrap();
        let expected_right = (-reference_half - intermediate) - 0.5 * outer;
        let split_right =
            ((-reference_half - 0.5 * intermediate) - 0.5 * intermediate) - 0.5 * outer;
        assert_ne!(expected_right.to_bits(), split_right.to_bits());
        assert_eq!(right[2].start_meters.to_bits(), expected_right.to_bits());
    }

    #[test]
    fn zero_width_right_offset_is_canonical_positive_zero() {
        let profiles = [
            AuthoringWidthProfile {
                start_width_meters: 0.0,
                end_width_meters: 1.0,
            },
            AuthoringWidthProfile {
                start_width_meters: 0.0,
                end_width_meters: 1.0,
            },
        ];

        let offsets = derive_member_offset_endpoints(&profiles, 0).unwrap();
        assert_eq!(offsets[1].start_meters.to_bits(), 0.0_f64.to_bits());
    }

    #[test]
    fn module_geometry_compilation_replaces_authoring_authorities_atomically() {
        let (alignments, mut declarations) = lowered_geometry_fixture();
        let usage = compile_authoring_geometry(
            "city",
            alignments,
            &mut declarations,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            station_row_bytes(1),
            6,
            u64::MAX,
        )
        .unwrap();
        assert_eq!(usage.output_point_count, 6);
        assert!(usage.peak_scratch_bytes > 0);

        let edge_a = declarations
            .iter()
            .find_map(|declaration| match declaration {
                TypedAstDeclaration::LaneEdge(edge)
                    if edge.header.stable_key.as_ref() == "edge-a" =>
                {
                    Some(edge)
                }
                _ => None,
            })
            .unwrap();
        let LaneEdgeGeometryAuthority::Compiled(geometry) = &edge_a.geometry_authority else {
            panic!("section-derived edge must have compiled geometry");
        };
        assert_eq!(geometry.length.value(), 10.0);
        assert_eq!(geometry.centerline_points.len(), 2);
        assert_eq!(
            geometry
                .canonical_frame
                .as_ref()
                .unwrap()
                .declaration_key()
                .as_ref(),
            "frame"
        );

        let internal = declarations
            .iter()
            .find_map(|declaration| match declaration {
                TypedAstDeclaration::LaneEdge(edge)
                    if edge.header.stable_key.as_ref() == "edge-internal" =>
                {
                    Some(edge)
                }
                _ => None,
            })
            .unwrap();
        let LaneEdgeGeometryAuthority::Compiled(geometry) = &internal.geometry_authority else {
            panic!("junction-internal edge must have compiled geometry");
        };
        assert!(geometry.canonical_frame.is_none());

        let facility = declarations
            .iter()
            .find_map(|declaration| match declaration {
                TypedAstDeclaration::FacilityBand(value) => Some(value),
                _ => None,
            })
            .unwrap();
        assert!(facility.authoring_width_profile.is_none());
        assert_eq!(
            facility
                .compiled_geometry
                .as_ref()
                .unwrap()
                .centerline_points
                .len(),
            2
        );
        assert!(declarations.iter().all(|declaration| {
            match declaration {
                TypedAstDeclaration::RoadCorridor(value) => value.authoring_geometry.is_none(),
                TypedAstDeclaration::RoadSection(value) => value
                    .lanes
                    .iter()
                    .all(|lane| lane.authoring_geometry.is_none()),
                _ => true,
            }
        }));
        for declaration in &declarations {
            let mut visited = Vec::new();
            declaration
                .try_visit_source_locations(|source| {
                    visited.push(source.clone());
                    Ok::<_, ()>(())
                })
                .unwrap();
            let source_ranges = match declaration {
                TypedAstDeclaration::LaneEdge(edge) => {
                    let LaneEdgeGeometryAuthority::Compiled(geometry) = &edge.geometry_authority
                    else {
                        continue;
                    };
                    geometry.source_ranges.as_ref()
                }
                TypedAstDeclaration::FacilityBand(facility) => facility
                    .compiled_geometry
                    .as_ref()
                    .map_or(&[][..], |geometry| geometry.source_ranges.as_ref()),
                _ => continue,
            };
            assert!(
                source_ranges
                    .iter()
                    .all(|range| visited.contains(&range.source))
            );
        }

        let (alignments, mut rejected) = lowered_geometry_fixture();
        assert_eq!(
            compile_authoring_geometry(
                "city",
                alignments,
                &mut rejected,
                GeometryAccuracyProfile::Balanced5Cm,
                GeometryDirectionProfile::Balanced2Deg,
                station_row_bytes(1),
                5,
                u64::MAX,
            ),
            Err(GeometryCompilationError::Numeric(
                NumericFreezeError::GeometryPointLimit
            ))
        );
        assert!(rejected.iter().any(|declaration| matches!(
            declaration,
            TypedAstDeclaration::LaneEdge(edge)
                if matches!(edge.geometry_authority, LaneEdgeGeometryAuthority::Authoring { .. })
        )));
    }

    #[test]
    fn corridor_reference_lane_must_belong_to_the_reference_section_and_be_forward() {
        let (alignments, mut declarations) = lowered_geometry_fixture();
        let section = declarations
            .iter_mut()
            .find_map(|declaration| match declaration {
                TypedAstDeclaration::RoadSection(value) => Some(value),
                _ => None,
            })
            .unwrap();
        section.lanes[0]
            .authoring_geometry
            .as_mut()
            .unwrap()
            .direction = AuthoringLaneDirection::Backward;

        assert_eq!(
            compile_authoring_geometry(
                "city",
                alignments,
                &mut declarations,
                GeometryAccuracyProfile::Balanced5Cm,
                GeometryDirectionProfile::Balanced2Deg,
                station_row_bytes(1),
                6,
                u64::MAX,
            ),
            Err(GeometryCompilationError::Numeric(
                NumericFreezeError::GeometryTopologyMismatch
            ))
        );
    }

    #[test]
    fn corridor_elements_must_retain_the_containing_corridor_owner() {
        let (alignments, mut declarations) = lowered_geometry_fixture();
        let cross_owner_address = TypedAstEntityAddress::owner_scoped(
            Arc::<[Arc<str>]>::from([Arc::from("other-corridor")]),
            Arc::from("facility"),
        );
        let corridor = declarations
            .iter_mut()
            .find_map(|declaration| match declaration {
                TypedAstDeclaration::RoadCorridor(value) => Some(value),
                _ => None,
            })
            .unwrap();
        let facility_reference = corridor
            .elements
            .iter_mut()
            .find_map(|element| match element {
                OwnedCorridorElementReference::FacilityBand(reference) => Some(reference),
                OwnedCorridorElementReference::RoadSection(_) => None,
            })
            .unwrap();
        facility_reference.target_address = cross_owner_address.clone();
        let facility = declarations
            .iter_mut()
            .find_map(|declaration| match declaration {
                TypedAstDeclaration::FacilityBand(value) => Some(value),
                _ => None,
            })
            .unwrap();
        facility.header.source_address = cross_owner_address;

        assert_eq!(
            compile_authoring_geometry(
                "city",
                alignments,
                &mut declarations,
                GeometryAccuracyProfile::Balanced5Cm,
                GeometryDirectionProfile::Balanced2Deg,
                station_row_bytes(1),
                6,
                u64::MAX,
            ),
            Err(GeometryCompilationError::Numeric(
                NumericFreezeError::GeometryTopologyMismatch
            ))
        );
    }

    #[test]
    fn typed_geometry_resolution_is_independent_of_declaration_order() {
        let (alignments, mut declarations) = lowered_geometry_fixture();
        declarations.reverse();
        let usage = compile_authoring_geometry(
            "city",
            alignments,
            &mut declarations,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            station_row_bytes(1),
            6,
            u64::MAX,
        )
        .unwrap();
        assert_eq!(usage.output_point_count, 6);
        assert!(declarations.iter().all(|declaration| match declaration {
            TypedAstDeclaration::LaneEdge(edge) => !matches!(
                edge.geometry_authority,
                LaneEdgeGeometryAuthority::Authoring { .. }
            ),
            TypedAstDeclaration::FacilityBand(facility) => {
                facility.authoring_width_profile.is_none()
            }
            _ => true,
        }));
    }

    #[test]
    fn typed_geometry_index_keeps_entity_kind_in_the_lookup_key() {
        let (alignments, mut declarations) = lowered_geometry_fixture();
        let section_address = declarations
            .iter()
            .find_map(|declaration| match declaration {
                TypedAstDeclaration::RoadSection(section) => {
                    Some(section.header.source_address.clone())
                }
                _ => None,
            })
            .unwrap();
        let corridor = declarations
            .iter_mut()
            .find_map(|declaration| match declaration {
                TypedAstDeclaration::RoadCorridor(value) => Some(value),
                _ => None,
            })
            .unwrap();
        let facility_reference = corridor
            .elements
            .iter_mut()
            .find_map(|element| match element {
                OwnedCorridorElementReference::FacilityBand(reference) => Some(reference),
                OwnedCorridorElementReference::RoadSection(_) => None,
            })
            .unwrap();
        facility_reference.target_address = section_address.clone();
        let facility = declarations
            .iter_mut()
            .find_map(|declaration| match declaration {
                TypedAstDeclaration::FacilityBand(value) => Some(value),
                _ => None,
            })
            .unwrap();
        facility.header.source_address = section_address;

        let usage = compile_authoring_geometry(
            "city",
            alignments,
            &mut declarations,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            station_row_bytes(1),
            6,
            u64::MAX,
        )
        .unwrap();
        assert_eq!(usage.output_point_count, 6);
    }

    #[test]
    fn alignment_corridors_must_form_a_complete_station_partition() {
        let (alignments, mut declarations) = lowered_geometry_fixture();
        let corridor = declarations
            .iter_mut()
            .find_map(|declaration| match declaration {
                TypedAstDeclaration::RoadCorridor(value) => Some(value),
                _ => None,
            })
            .unwrap();
        corridor
            .authoring_geometry
            .as_mut()
            .unwrap()
            .start_station_meters = 1.0;
        assert_eq!(
            compile_authoring_geometry(
                "city",
                alignments,
                &mut declarations,
                GeometryAccuracyProfile::Balanced5Cm,
                GeometryDirectionProfile::Balanced2Deg,
                station_row_bytes(1),
                6,
                u64::MAX,
            ),
            Err(GeometryCompilationError::Numeric(
                NumericFreezeError::GeometryTopologyMismatch
            ))
        );

        let (alignments, mut declarations) = lowered_geometry_fixture();
        let corridor = declarations
            .iter_mut()
            .find_map(|declaration| match declaration {
                TypedAstDeclaration::RoadCorridor(value) => Some(value),
                _ => None,
            })
            .unwrap();
        corridor.authoring_geometry.as_mut().unwrap().end_station =
            AuthoringStationEnd::Finite(10.0);
        assert_eq!(
            compile_authoring_geometry(
                "city",
                alignments,
                &mut declarations,
                GeometryAccuracyProfile::Balanced5Cm,
                GeometryDirectionProfile::Balanced2Deg,
                station_row_bytes(1),
                6,
                u64::MAX,
            ),
            Err(GeometryCompilationError::Numeric(
                NumericFreezeError::GeometryTopologyMismatch
            ))
        );
    }

    #[test]
    fn unused_alignments_do_not_consume_station_compilation_resources() {
        let (alignments, mut declarations) = lowered_geometry_fixture();
        let mut alignments = alignments.into_vec();
        alignments.push(RoadAlignmentDeclaration {
            road_alignment_key: Arc::from("unused"),
            canonical_frame: alignments[0].canonical_frame.clone(),
            reference_line: AuthoringCurveProgramDeclaration {
                start: point(0.0, 0.0),
                start_span: span(10),
                segments: vec![AuthoringCurveSegmentDeclaration {
                    geometry: AuthoringCurveSegmentGeometry::CubicBezier {
                        control_1: point(0.0, 0.0),
                        control_2: point(0.0, 0.0),
                        end: point(0.0, 0.0),
                    },
                    span: span(11),
                }]
                .into_boxed_slice(),
            },
            span: span(9),
        });

        assert_eq!(
            compile_authoring_geometry(
                "city",
                alignments.into_boxed_slice(),
                &mut declarations,
                GeometryAccuracyProfile::Balanced5Cm,
                GeometryDirectionProfile::Balanced2Deg,
                station_row_bytes(1),
                6,
                u64::MAX,
            )
            .map(|usage| usage.output_point_count),
            Ok(6)
        );
    }

    #[test]
    fn geometry_scratch_limit_is_checked_at_the_exact_requested_capacity_boundary() {
        let attempt = |scratch_limit| {
            let (alignments, mut declarations) = lowered_geometry_fixture();
            compile_authoring_geometry(
                "city",
                alignments,
                &mut declarations,
                GeometryAccuracyProfile::Balanced5Cm,
                GeometryDirectionProfile::Balanced2Deg,
                station_row_bytes(1),
                6,
                scratch_limit,
            )
        };
        let unlimited = attempt(u64::MAX).unwrap();
        let exact_peak = unlimited.peak_scratch_bytes;
        assert_eq!(attempt(exact_peak), Ok(unlimited));
        assert_eq!(
            attempt(exact_peak - 1),
            Err(GeometryCompilationError::ScratchLimit {
                limit: exact_peak - 1,
                observed: exact_peak,
            })
        );
        let (alignments, mut declarations) = lowered_geometry_fixture();
        assert!(matches!(
            compile_authoring_geometry(
                "city",
                alignments,
                &mut declarations,
                GeometryAccuracyProfile::Balanced5Cm,
                GeometryDirectionProfile::Balanced2Deg,
                station_row_bytes(1),
                6,
                exact_peak - 1,
            ),
            Err(GeometryCompilationError::ScratchLimit { .. })
        ));
        assert!(declarations.iter().any(|declaration| matches!(
            declaration,
            TypedAstDeclaration::LaneEdge(edge)
                if matches!(edge.geometry_authority, LaneEdgeGeometryAuthority::Authoring { .. })
        )));
    }
}
