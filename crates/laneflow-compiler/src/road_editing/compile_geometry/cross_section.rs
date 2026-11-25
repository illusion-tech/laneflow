//! 横断面行在来源仍可定位时形成，稳定实体的序号由发射遍解析。
use super::*;
use crate::RoadEditingTableKind as Table;
use crate::road_editing::{
    BoundaryStation,
    boundary::{
        ExtentRow, NamedExtent, PublishedCrossSection, RowSource, WidthMember, WidthRow, field,
    },
};
use std::sync::Arc;
mod surface;

pub(super) fn needed(plan: &ResolvedCorridorPlan<'_>) -> bool {
    let Some(g) = &plan.corridor.authoring_geometry else {
        return false;
    };
    g.station_mapping_request.is_some()
        && (g.complete
            || !g.extents.is_empty()
            || plan.elements.iter().any(|e| match e {
                ResolvedCorridorElement::FacilityBand(_) => true,
                ResolvedCorridorElement::RoadSection(s) => !s.extents.is_empty(),
            }))
}

fn retain<T>(
    scratch: &mut GeometryScratchBudget,
    n: usize,
) -> Result<(), GeometryCompilationError> {
    let bytes = capacity_bytes::<T>(n).ok_or(GeometryCompilationError::LiveLimit {
        limit: scratch.live_headroom,
        observed: u64::MAX,
    })?;
    scratch.station_bytes = scratch.station_bytes.saturating_add(bytes);
    check_live_limit(scratch.live_headroom, scratch.dynamic_live(), 0, 0)?;
    Ok(())
}

pub(super) fn not_proven(source: &SourceLocation) -> GeometryCompilationError {
    GeometryCompilationError::Constraint(Box::new((
        crate::RoadEditingNumericViolation::StationMappingNotProven,
        source.clone(),
    )))
}

pub(super) fn compile(
    plan: &ResolvedCorridorPlan<'_>,
    alignment: &CompiledAlignmentEntry<'_>,
    offsets: &[MemberOffsetEndpoints],
    domain: [f64; 2],
    width_error: f64,
    scratch: &mut GeometryScratchBudget,
) -> Result<PublishedCrossSection, GeometryCompilationError> {
    let g = plan
        .corridor
        .authoring_geometry
        .as_ref()
        .expect("resolved corridor");
    let coverage = g
        .station_mapping_request
        .as_ref()
        .expect("mapping required");
    let last_error = alignment
        .reference
        .station_rows
        .last()
        .ok_or_else(|| not_proven(coverage))?
        .station_error_meters;
    let n_widths = if g.complete {
        plan.member_count
    } else {
        plan.elements
            .iter()
            .filter(|e| matches!(e, ResolvedCorridorElement::FacilityBand(_)))
            .count()
    };
    let n_extents = plan.elements.iter().fold(g.extents.len(), |n, e| {
        n.saturating_add(match e {
            ResolvedCorridorElement::RoadSection(s) => s.extents.len(),
            _ => 0,
        })
    });
    retain::<PublishedCrossSection>(scratch, 1)?;
    retain::<usize>(scratch, 2)?;
    retain::<WidthRow>(scratch, n_widths)?;
    retain::<ExtentRow>(scratch, n_extents)?;
    let mut extents = Vec::with_capacity(n_extents);
    let mut widths = Vec::with_capacity(n_widths);
    let mut add_extent =
        |decl: &Arc<NamedExtent>, section_element| -> Result<(), GeometryCompilationError> {
            let mut stations = [(0, 0); 2];
            for (i, end) in decl.ends.iter().enumerate() {
                let value = match end {
                    BoundaryStation::Fixed(s) => *s,
                    BoundaryStation::CorridorEndpoint => domain[i],
                };
                let source = decl.station_source(i);
                if !(domain[0]..=domain[1]).contains(&value) {
                    return Err(not_proven(&source));
                }
                stations[i] = crate::road_editing::station_proof::node(
                    domain[0],
                    crate::road_editing::native_curve::Scalar::exact(value).widen(last_error),
                    Some(value),
                )
                .map_err(|_| not_proven(&source))?;
            }
            if stations[0].0 >= stations[1].0 {
                return Err(not_proven(&decl.name_source.primary));
            }
            extents.push(ExtentRow {
                section_element,
                declaration: decl.clone(),
                stations,
            });
            Ok(())
        };
    for e in &g.extents {
        add_extent(e, None)?;
    }
    for (i, element) in plan.elements.iter().enumerate() {
        if let ResolvedCorridorElement::RoadSection(s) = element {
            for e in &s.extents {
                add_extent(e, Some(i as u32))?;
            }
        }
    }

    // 参考车道中心和目标两侧之间的宽度全都参与横向界。只保留该范围实际用到的区段顺序。
    type WidthInput<'a> = (
        WidthMember,
        SourceLocation,
        Option<&'a SourceLocation>,
        AuthoringWidthProfile,
        Option<bool>,
    );
    scratch.reserve::<WidthInput>(plan.member_count)?;
    let mut inputs = Vec::with_capacity(plan.member_count);
    for (element_index, element) in plan.elements.iter().enumerate() {
        let i = element_index as u32;
        match element {
            ResolvedCorridorElement::FacilityBand(b) => inputs.push((
                WidthMember::Band(i),
                b.header.span.with_property([field(Table::FacilityBand, 2)]),
                None,
                b.authoring_width_profile.expect("resolved band"),
                None,
            )),
            ResolvedCorridorElement::RoadSection(s) => {
                for (j, l) in s.lanes.iter().enumerate() {
                    let lane = l.authoring_geometry.as_ref().expect("resolved lane");
                    inputs.push((
                        WidthMember::Lane {
                            element: i,
                            lane: j as u32,
                        },
                        l.header
                            .span
                            .with_property([field(Table::AuthoringLane, 3)]),
                        Some(&s.header.span),
                        lane.width_profile,
                        lane.supports_wheel,
                    ));
                }
            }
        }
    }
    let order = plan
        .corridor
        .header
        .span
        .with_property([field(Table::RoadCorridor, 7)]);
    let mut width_abs = 0.0_f64;
    for (i, (member, loc, _, width, supports)) in inputs.iter().enumerate() {
        for (center, width) in [
            (offsets[i].start_meters, width.start_width_meters),
            (offsets[i].end_meters, width.end_width_meters),
        ] {
            width_abs = width_abs.max((center.abs() + 0.5 * width).next_up());
        }
        if !g.complete && matches!(member, WidthMember::Lane { .. }) {
            continue;
        }
        let range = i.min(plan.reference_ordinal)..=i.max(plan.reference_ordinal);
        let capacity = 3 + 2 * range.clone().count();
        retain::<SourceLocation>(scratch, capacity)?;
        let mut sources = Vec::with_capacity(capacity);
        sources.extend([
            coverage.clone(),
            order.clone(),
            g.reference_lane.span.clone(),
        ]);
        for j in range {
            sources.push(inputs[j].1.clone());
            if let Some(s) = inputs[j].2 {
                sources.push(s.with_property([field(Table::RoadSection, 2)]));
            }
        }
        sources.sort_unstable();
        sources.dedup();
        let lane = if let Some(supports) = supports {
            Some((
                crate::road_editing::station_proof::band(
                    Table::AuthoringLane,
                    offsets[i],
                    *width,
                    domain,
                    width_error,
                    last_error,
                    loc,
                )?,
                *supports,
            ))
        } else {
            None
        };
        let primary = if lane.is_some() {
            loc.with_property([field(Table::AuthoringLane, 8)])
        } else {
            loc.clone()
        };
        sources.retain(|s| s != &primary);
        widths.push(WidthRow {
            member: *member,
            lane,
            source: RowSource {
                primary,
                contributing: sources.into_boxed_slice(),
            },
        });
    }
    drop(inputs);
    scratch.release::<WidthInput>(plan.member_count);
    let surface = if g.complete {
        surface::build(
            plan,
            alignment,
            domain,
            &extents,
            (width_abs + width_error).next_up(),
            scratch,
        )?
    } else {
        Box::new([])
    };
    Ok(PublishedCrossSection {
        widths: widths.into_boxed_slice(),
        extents: extents.into_boxed_slice(),
        surface,
    })
}
