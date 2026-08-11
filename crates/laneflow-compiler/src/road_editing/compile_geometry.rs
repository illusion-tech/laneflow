//! RoadEditingSource authoring curve 到共同规范几何的有界两遍编译。

use crate::declaration::{
    AuthoringCurveProgramDeclaration, AuthoringCurveSegmentDeclaration,
    AuthoringCurveSegmentGeometry, AuthoringLaneDirection, AuthoringPoint3F64, AuthoringStationEnd,
    AuthoringWidthProfile, CanonicalPoint3F32Input, CompiledFacilityBandGeometry,
    CompiledLaneEdgeGeometry, EdgeLength, FacilityBandDeclaration, LaneEdgeGeometryAuthority,
    OwnedCorridorElementReference, RoadAlignmentDeclaration, RoadSectionDeclaration,
    TypedAstDeclaration, TypedAstEntityAddress,
};
use crate::{GeometryAccuracyProfile, GeometryDirectionProfile};

use super::geometry::{
    ApproximationInterval, ApproximationPoint, ApproximationPointSink, ApproximationVertex,
    CurveSegment, NumericFreezeError, OffsetInterval, Point3, SegmentEvaluator, StationInterval,
    approximate_interval, canonical_point_distance, numeric_stack_scratch_bytes, point_distance,
    quantize_point, validate_canonical_polyline,
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

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ReferenceStationPosition {
    pub(super) row_index: u32,
    pub(super) segment_ordinal: u32,
    pub(super) parameter: f64,
}

#[cfg(test)]
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

#[derive(Debug, Eq, PartialEq)]
struct AlignmentReferenceSizing {
    station_rows: usize,
    horizontal_regularity_visits: Box<[u32]>,
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
    source_address: &'a TypedAstEntityAddress,
    width_profile: AuthoringWidthProfile,
    target: CorridorMemberTarget<'a>,
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
pub(super) fn compile_authoring_geometry(
    authoring_namespace_id: &str,
    alignments: Box<[RoadAlignmentDeclaration]>,
    declarations: &mut [TypedAstDeclaration],
    accuracy: GeometryAccuracyProfile,
    direction: GeometryDirectionProfile,
    geometry_point_limit: u64,
    scratch_limit: u64,
) -> Result<GeometryCompilationUsage, GeometryCompilationError> {
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

    // alignment station rows and visit caches stay live together until every corridor has
    // consumed them. Freeze their complete requested-capacity peak before allocating the first
    // input-dependent box; the per-corridor member arrays are checked against the same base below.
    let regularity_visit_count = alignments.iter().fold(0_usize, |total, alignment| {
        total.saturating_add(alignment.reference_line.segments.len())
    });
    let fixed_scratch_bytes = numeric_stack_scratch_bytes()
        .saturating_add(capacity_bytes::<AlignmentReferenceSizing>(alignments.len()))
        .saturating_add(capacity_bytes::<CompiledAlignmentEntry<'_>>(
            alignments.len(),
        ))
        .saturating_add(capacity_bytes::<u32>(regularity_visit_count))
        .saturating_add(capacity_bytes::<PendingLaneGeometry>(expected_lane_outputs))
        .saturating_add(capacity_bytes::<PendingFacilityGeometry>(
            expected_facility_outputs,
        ));
    check_scratch_limit(scratch_limit, fixed_scratch_bytes)?;
    let station_row_budget = scratch_limit.saturating_sub(fixed_scratch_bytes)
        / u64::try_from(core::mem::size_of::<ReferenceStationRow>()).unwrap_or(u64::MAX);
    let mut remaining_station_rows = station_row_budget;
    let mut alignment_sizings = Vec::with_capacity(alignments.len());
    for alignment in &alignments {
        let transient_vertex_limit = remaining_station_rows.saturating_add(1);
        let sizing =
            match measure_alignment_reference(&alignment.reference_line, transient_vertex_limit) {
                Ok(value) => value,
                Err(NumericFreezeError::GeometryPointLimit) => {
                    return Err(GeometryCompilationError::ScratchLimit {
                        limit: scratch_limit,
                        observed: scratch_limit.saturating_add(1),
                    });
                }
                Err(error) => return Err(error.into()),
            };
        remaining_station_rows = remaining_station_rows
            .saturating_sub(u64::try_from(sizing.station_rows).unwrap_or(u64::MAX));
        alignment_sizings.push(sizing);
    }
    let station_row_count = station_row_budget.saturating_sub(remaining_station_rows);
    let retained_alignment_scratch_bytes = numeric_stack_scratch_bytes()
        .saturating_add(capacity_bytes::<CompiledAlignmentEntry<'_>>(
            alignments.len(),
        ))
        .saturating_add(capacity_bytes::<ReferenceStationRow>(
            usize::try_from(station_row_count).unwrap_or(usize::MAX),
        ))
        .saturating_add(capacity_bytes::<u32>(regularity_visit_count))
        .saturating_add(capacity_bytes::<PendingLaneGeometry>(expected_lane_outputs))
        .saturating_add(capacity_bytes::<PendingFacilityGeometry>(
            expected_facility_outputs,
        ));
    let mut peak_scratch_bytes =
        fixed_scratch_bytes.saturating_add(capacity_bytes::<ReferenceStationRow>(
            usize::try_from(station_row_count).unwrap_or(usize::MAX),
        ));
    check_scratch_limit(scratch_limit, peak_scratch_bytes)?;

    let mut compiled_alignments = Vec::with_capacity(alignments.len());
    for (alignment, sizing) in alignments.iter().zip(alignment_sizings) {
        compiled_alignments.push(CompiledAlignmentEntry {
            declaration: alignment,
            reference: compile_measured_alignment_reference(&alignment.reference_line, sizing)?,
        });
    }

    let mut used_points = 0_u64;
    let mut lane_outputs = Vec::with_capacity(expected_lane_outputs);
    let mut facility_outputs = Vec::with_capacity(expected_facility_outputs);

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
        let compiled = compile_explicit_curve(
            curve,
            accuracy,
            direction,
            remaining_points(geometry_point_limit, used_points)?,
        )?;
        used_points = charge_points(geometry_point_limit, used_points, compiled.points.len())?;
        lane_outputs.push(PendingLaneGeometry {
            target: edge.header.source_address.clone(),
            value: Some(CompiledLaneEdgeGeometry {
                length: compiled.length,
                canonical_frame: None,
                centerline_points: compiled.points,
            }),
        });
    }

    for declaration in declarations.iter() {
        let TypedAstDeclaration::RoadCorridor(corridor) = declaration else {
            continue;
        };
        let Some(authoring) = &corridor.authoring_geometry else {
            continue;
        };
        let alignment = compiled_alignments
            .iter()
            .find(|entry| {
                entry.declaration.road_alignment_key.as_ref()
                    == authoring.road_alignment_key.as_ref()
            })
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

        let expected_member_count =
            corridor_member_count(corridor, declarations, authoring_namespace_id)?;
        // `members`, width profiles and derived offsets coexist only for this corridor. Their
        // canonical member count is known before allocation, so the peak remains exact and local.
        let corridor_scratch_bytes = retained_alignment_scratch_bytes
            .saturating_add(capacity_bytes::<CorridorMember<'_>>(expected_member_count))
            .saturating_add(capacity_bytes::<AuthoringWidthProfile>(
                expected_member_count,
            ))
            .saturating_add(capacity_bytes::<MemberOffsetEndpoints>(
                expected_member_count,
            ));
        check_scratch_limit(scratch_limit, corridor_scratch_bytes)?;
        peak_scratch_bytes = peak_scratch_bytes.max(corridor_scratch_bytes);

        let mut members = Vec::with_capacity(expected_member_count);
        for element in &corridor.elements {
            match element {
                OwnedCorridorElementReference::RoadSection(reference) => {
                    if reference.module_namespace.as_ref() != authoring_namespace_id {
                        return Err(NumericFreezeError::GeometryTopologyMismatch.into());
                    }
                    let section = find_road_section(declarations, &reference.target_address)
                        .ok_or(NumericFreezeError::GeometryTopologyMismatch)?;
                    append_section_members(&mut members, section, authoring_namespace_id)?;
                }
                OwnedCorridorElementReference::FacilityBand(reference) => {
                    if reference.module_namespace.as_ref() != authoring_namespace_id {
                        return Err(NumericFreezeError::GeometryTopologyMismatch.into());
                    }
                    let facility = find_facility_band(declarations, &reference.target_address)
                        .ok_or(NumericFreezeError::GeometryTopologyMismatch)?;
                    let width_profile = facility
                        .authoring_width_profile
                        .ok_or(NumericFreezeError::GeometryTopologyMismatch)?;
                    members.push(CorridorMember {
                        source_address: &facility.header.source_address,
                        width_profile,
                        target: CorridorMemberTarget::FacilityBand {
                            target: &facility.header.source_address,
                        },
                    });
                }
            }
        }
        debug_assert_eq!(members.len(), expected_member_count);
        let reference_ordinal = members
            .iter()
            .position(|member| {
                member.source_address == &authoring.reference_lane.target_address
                    && authoring.reference_lane.module_namespace.as_ref() == authoring_namespace_id
            })
            .ok_or(NumericFreezeError::GeometryTopologyMismatch)?;
        let mut width_profiles = Vec::with_capacity(expected_member_count);
        width_profiles.extend(members.iter().map(|member| member.width_profile));
        let offsets = derive_member_offset_endpoints(&width_profiles, reference_ordinal)?;
        drop(width_profiles);

        for (member, offset) in members.into_iter().zip(offsets.iter()) {
            let lane_direction = match member.target {
                CorridorMemberTarget::LaneEdge { direction, .. } => direction,
                CorridorMemberTarget::FacilityBand { .. } => AuthoringLaneDirection::Forward,
            };
            let compiled = compile_offset_curve(
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
            )?;
            used_points = charge_points(geometry_point_limit, used_points, compiled.points.len())?;
            match member.target {
                CorridorMemberTarget::LaneEdge { target, .. } => {
                    lane_outputs.push(PendingLaneGeometry {
                        target: target.clone(),
                        value: Some(CompiledLaneEdgeGeometry {
                            length: compiled.length,
                            canonical_frame: Some(alignment.declaration.canonical_frame.clone()),
                            centerline_points: compiled.points,
                        }),
                    });
                }
                CorridorMemberTarget::FacilityBand { target } => {
                    facility_outputs.push(PendingFacilityGeometry {
                        target: target.clone(),
                        value: Some(CompiledFacilityBandGeometry {
                            length: compiled.length,
                            canonical_frame: alignment.declaration.canonical_frame.clone(),
                            centerline_points: compiled.points,
                        }),
                    });
                }
            }
        }
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
    Ok(GeometryCompilationUsage {
        output_point_count: used_points,
        peak_scratch_bytes,
    })
}

fn corridor_member_count(
    corridor: &crate::declaration::RoadCorridorDeclaration,
    declarations: &[TypedAstDeclaration],
    authoring_namespace_id: &str,
) -> Result<usize, NumericFreezeError> {
    let mut count = 0_usize;
    for element in &corridor.elements {
        match element {
            OwnedCorridorElementReference::RoadSection(reference) => {
                if reference.module_namespace.as_ref() != authoring_namespace_id {
                    return Err(NumericFreezeError::GeometryTopologyMismatch);
                }
                let section = find_road_section(declarations, &reference.target_address)
                    .ok_or(NumericFreezeError::GeometryTopologyMismatch)?;
                count = count.saturating_add(section.lanes.len());
            }
            OwnedCorridorElementReference::FacilityBand(reference) => {
                if reference.module_namespace.as_ref() != authoring_namespace_id
                    || find_facility_band(declarations, &reference.target_address).is_none()
                {
                    return Err(NumericFreezeError::GeometryTopologyMismatch);
                }
                count = count.saturating_add(1);
            }
        }
    }
    Ok(count)
}

fn capacity_bytes<T>(capacity: usize) -> u64 {
    u64::try_from(core::mem::size_of::<T>())
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(capacity).unwrap_or(u64::MAX))
}

fn check_scratch_limit(limit: u64, observed: u64) -> Result<(), GeometryCompilationError> {
    if observed > limit {
        return Err(GeometryCompilationError::ScratchLimit { limit, observed });
    }
    Ok(())
}

fn append_section_members<'a>(
    members: &mut Vec<CorridorMember<'a>>,
    section: &'a RoadSectionDeclaration,
    authoring_namespace_id: &str,
) -> Result<(), NumericFreezeError> {
    for lane in &section.lanes {
        let geometry = lane
            .authoring_geometry
            .as_ref()
            .ok_or(NumericFreezeError::GeometryTopologyMismatch)?;
        let [edge] = lane.edge_chain.as_ref() else {
            return Err(NumericFreezeError::GeometryTopologyMismatch);
        };
        if edge.module_namespace.as_ref() != authoring_namespace_id {
            return Err(NumericFreezeError::GeometryTopologyMismatch);
        }
        members.push(CorridorMember {
            source_address: &lane.header.source_address,
            width_profile: geometry.width_profile,
            target: CorridorMemberTarget::LaneEdge {
                target: &edge.target_address,
                direction: geometry.direction,
            },
        });
    }
    Ok(())
}

fn find_road_section<'a>(
    declarations: &'a [TypedAstDeclaration],
    address: &TypedAstEntityAddress,
) -> Option<&'a RoadSectionDeclaration> {
    declarations
        .iter()
        .find_map(|declaration| match declaration {
            TypedAstDeclaration::RoadSection(section)
                if &section.header.source_address == address =>
            {
                Some(section)
            }
            _ => None,
        })
}

fn find_facility_band<'a>(
    declarations: &'a [TypedAstDeclaration],
    address: &TypedAstEntityAddress,
) -> Option<&'a FacilityBandDeclaration> {
    declarations
        .iter()
        .find_map(|declaration| match declaration {
            TypedAstDeclaration::FacilityBand(facility)
                if &facility.header.source_address == address =>
            {
                Some(facility)
            }
            _ => None,
        })
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

#[cfg(test)]
pub(super) fn compile_alignment_reference(
    program: &AuthoringCurveProgramDeclaration,
    transient_vertex_limit: u64,
) -> Result<CompiledAlignmentReference, NumericFreezeError> {
    let sizing = measure_alignment_reference(program, transient_vertex_limit)?;
    compile_measured_alignment_reference(program, sizing)
}

fn measure_alignment_reference(
    program: &AuthoringCurveProgramDeclaration,
    transient_vertex_limit: u64,
) -> Result<AlignmentReferenceSizing, NumericFreezeError> {
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

    let mut visits = Vec::with_capacity(program.segments.len());
    let mut start = point3(program.start)?;
    for source in &program.segments {
        let (segment, end) = source_segment(start, source)?;
        visits.push(segment.prove_horizontal_regularity()?);
        start = end;
    }
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
    if station_collector.rows.len() != station_collector.expected_rows {
        return Err(NumericFreezeError::ApproximationNotConverged);
    }

    if sizing.horizontal_regularity_visits.len() != program.segments.len() {
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
        let alignments = super::super::lowering::lower_road_alignments(verified.root(), &locations);
        let mut declarations = super::super::lowering::lower_topology_authoring_declarations(
            verified.root(),
            &locations,
        )
        .unwrap();
        declarations.extend(super::super::lowering::lower_owner_scoped_declarations(
            verified.root(),
            &locations,
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

    #[test]
    fn module_geometry_compilation_replaces_authoring_authorities_atomically() {
        let (alignments, mut declarations) = lowered_geometry_fixture();
        let used_points = compile_authoring_geometry(
            "city",
            alignments,
            &mut declarations,
            GeometryAccuracyProfile::Balanced5Cm,
            GeometryDirectionProfile::Balanced2Deg,
            6,
            u64::MAX,
        )
        .unwrap();
        assert_eq!(used_points.output_point_count, 6);
        assert!(used_points.peak_scratch_bytes > 0);

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

        let (alignments, mut rejected) = lowered_geometry_fixture();
        assert_eq!(
            compile_authoring_geometry(
                "city",
                alignments,
                &mut rejected,
                GeometryAccuracyProfile::Balanced5Cm,
                GeometryDirectionProfile::Balanced2Deg,
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
}
