use super::*;
use crate::road_editing::{
    boundary::{MemberEntry, Owner, extent_field, field, left_index},
    model::{BoundaryExtentKind, BoundaryPayloadInput, BoundaryStation, local_key_valid},
};
use crate::{
    RoadEditingPropertyStep as Step, RoadEditingStructKind as Struct,
    RoadEditingTableKind as Table, RoadEditingUnionKind as Union,
};

fn payload_location<T>(variant: u8, name: &str, locate: impl FnOnce(&[Step]) -> T) -> T {
    let (table, names): (_, &[&str]) = match variant {
        1 => (
            Table::CrossingRule,
            &["crossingRule.directions", "crossingRule.effect"],
        ),
        2 => (
            Table::Guardrail,
            &[
                "guardrail.anchorRole",
                "guardrail.lateralOffsetMeters",
                "guardrail.horizontalLengthMeters",
                "guardrail.radiusMeters",
                "guardrail.bottomHeightMeters",
                "guardrail.topHeightMeters",
                "guardrail.parameterErrorMeters",
            ],
        ),
        3 => (
            Table::Curb,
            &[
                "curb.low.leftOffset",
                "curb.low.rightOffset",
                "curb.low.bottomHeight",
                "curb.low.topHeight",
                "curb.high.leftOffset",
                "curb.high.rightOffset",
                "curb.high.bottomHeight",
                "curb.high.topHeight",
                "curb.parameterErrorMeters",
            ],
        ),
        _ => return locate(&[extent_field(6)]),
    };
    let Some(index) = names.iter().position(|&n| n == name) else {
        return locate(&[extent_field(7)]);
    };
    let union = Step::UnionVariant {
        union: Union::BoundaryPayload,
        discriminant: variant,
    };
    if variant == 3 && index < 8 {
        return locate(&[
            extent_field(7),
            union,
            field(table, (index / 4) as u16),
            Step::StructMember {
                structure: Struct::CurbEnd,
                member_id: (index % 4) as u8,
            },
        ]);
    }
    locate(&[
        extent_field(7),
        union,
        field(table, if variant == 3 { 2 } else { index as u16 }),
    ])
}

type ExtentKey<'a> = (u32, u8, &'a str, usize);

pub(super) fn count(root: wire::RoadEditingSource<'_>) -> usize {
    Owner::all(root).fold(0usize, |n, o| n.saturating_add(o.extents().len()))
}

pub(super) fn plan(root: wire::RoadEditingSource<'_>) -> Plan {
    let mut peak = Plan::ZERO;
    for o in Owner::all(root).filter(|o| !o.extents().is_empty()) {
        peak = peak.max(
            Plan::ZERO
                .array::<MemberEntry>(o.members().len())
                .array::<ExtentKey>(o.extents().len()),
        );
    }
    let needs_coverage = root
        .road_corridors()
        .iter()
        .any(|c| c.cross_section_coverage() == wire::CrossSectionCoverage::Complete)
        || root
            .authoring_lanes()
            .iter()
            .any(|lane| lane.supports_wheel().is_some())
        || root
            .road_sections()
            .iter()
            .any(|s| !s.boundary_extents().is_empty());
    if needs_coverage {
        peak.plus(Plan::ZERO.array::<(&str, u8)>(root.road_corridors().len()))
    } else {
        peak
    }
}

pub(super) fn validate(
    usage: &mut RoadEditingPreflightCounts,
    root: wire::RoadEditingSource<'_>,
    ns: &str,
    imports: StringVector<'_>,
    limits: &PreflightScratch<'_>,
    doc: &str,
) -> Result<(), DiagnosticBundle> {
    use wire::CrossSectionCoverage as Coverage;
    let needs_index = root
        .road_corridors()
        .iter()
        .any(|c| c.cross_section_coverage() == Coverage::Complete)
        || root
            .authoring_lanes()
            .iter()
            .any(|lane| lane.supports_wheel().is_some())
        || root
            .road_sections()
            .iter()
            .any(|s| !s.boundary_extents().is_empty());
    let mut coverage = if needs_index {
        Some(
            limits.collect(
                root.road_corridors()
                    .iter()
                    .map(|c| (c.road_corridor_key(), c.cross_section_coverage().0)),
            )?,
        )
    } else {
        None
    };
    if let Some(v) = coverage.as_mut() {
        v.sort_unstable();
    }
    let by_key = |key: &str| -> Result<Coverage, DiagnosticBundle> {
        let Some(c) = &coverage else {
            return Ok(Coverage::None);
        };
        let i = c
            .binary_search_by_key(&key, |v| v.0)
            .map_err(|_| invalid_combination("boundaryExtent.owner", doc))?;
        Ok(Coverage(c[i].1))
    };
    for owner in Owner::all(root) {
        let extents = owner.extents();
        let scope = match owner {
            Owner::Corridor(c) => c.cross_section_coverage(),
            Owner::Section(s) if !extents.is_empty() => by_key(s.road_corridor())?,
            Owner::Section(_) => continue,
        };
        if !matches!(
            scope,
            Coverage::None | Coverage::StationMapping | Coverage::Complete
        ) {
            return Err(
                invalid_combination("roadCorridor.crossSectionCoverage", doc)
                    .with_fallback_primary_location(
                        super::super::location::RoadEditingLocationFactory::invalid_boundary(
                            ns,
                            doc,
                            owner,
                            None,
                            &[field(Table::RoadCorridor, 9)],
                        ),
                    ),
            );
        }
        if extents.is_empty() {
            continue;
        }
        let at = |e: DiagnosticBundle, index: Option<u32>, path: &[Step]| {
            e.with_fallback_primary_location(
                super::super::location::RoadEditingLocationFactory::invalid_boundary(
                    ns, doc, owner, index, path,
                ),
            )
        };
        if scope == Coverage::None {
            return Err(at(
                invalid_combination("boundaryExtent.coverage", doc),
                None,
                &[owner.field()],
            ));
        }
        usage.require_relation_capacity(extents.len(), limits)?;
        usage.charge_relation(extents.len());
        let mut members = limits.collect(owner.members())?;
        members.sort_unstable();
        let mut keys = limits.collect(extents.iter().enumerate().map(|(i, e)| {
            (
                left_index(&members, e).unwrap_or(u32::MAX),
                e.extent_kind().0,
                e.local_key(),
                i,
            )
        }))?;
        keys.sort_unstable();
        let mut prev = None;
        for (ordinal, &(left, kind, key, i)) in keys.iter().enumerate() {
            if left == u32::MAX || prev == Some((left, kind, key)) {
                let table = extents.get(i)._tab;
                let location =
                    super::super::location::RoadEditingLocationFactory::invalid_boundary(
                        ns,
                        doc,
                        owner,
                        None,
                        &[owner.field()],
                    )
                    .with_wire_table(table.loc(), table.buf().len());
                return Err(invalid_combination("boundaryExtent.members", doc)
                    .with_fallback_primary_location(location));
            }
            prev = Some((left, kind, key));
            let e = extents.get(i);
            let ordinal = Some(ordinal as u32);
            let err = |field, path: &[Step]| at(invalid_combination(field, doc), ordinal, path);
            if !local_key_valid(key) {
                return Err(err("boundaryExtent.localKey", &[extent_field(3)]));
            }
            usage.charge_token(key, "boundaryExtent.localKey", limits, doc)?;
            usage.charge_canvas(e.canvas_selection(), limits, doc)?;
            usage.typed_ast_record_count = usage
                .typed_ast_record_count
                .saturating_add(3 + u64::from(e.payload().is_some()));
            for m in [e.left(), e.right()] {
                usage.charge_reference(
                    m.entity_reference(),
                    if m.kind() == wire::BoundaryMemberKind::AuthoringLane {
                        3
                    } else {
                        2
                    },
                    false,
                    "boundaryMember.entityReference",
                    ns,
                    imports,
                    limits,
                    doc,
                )?;
            }
            let shape = BoundaryExtentKind::from_wire(e.extent_kind())
                .map_err(|f| err(f, &[extent_field(2)]))?;
            let station = |value: &wire::BoundaryStation, id| {
                BoundaryStation::from_wire(value).map_err(|f| {
                    let member_id = u8::from(matches!(
                        value.kind(),
                        wire::BoundaryStationKind::Fixed
                            | wire::BoundaryStationKind::CorridorEndpoint
                    ));
                    err(
                        f,
                        &[
                            extent_field(id),
                            Step::StructMember {
                                structure: Struct::BoundaryStation,
                                member_id,
                            },
                        ],
                    )
                })
            };
            let lo = station(e.low(), 4)?;
            let hi = station(e.high(), 5)?;
            if matches!((lo,hi),(BoundaryStation::Fixed(a),BoundaryStation::Fixed(b)) if a>=b) {
                return Err(err(
                    "boundaryExtent.high",
                    &[
                        extent_field(5),
                        Step::StructMember {
                            structure: Struct::BoundaryStation,
                            member_id: 1,
                        },
                    ],
                ));
            }
            let payload_error =
                |name| payload_location(e.payload_type().0, name, |path| err(name, path));
            let payload = BoundaryPayloadInput::from_wire(e).map_err(payload_error)?;
            if (scope == Coverage::Complete) != payload.is_some()
                || payload.is_some_and(|p| {
                    matches!(p, BoundaryPayloadInput::CrossingRule { .. })
                        != (shape == BoundaryExtentKind::Rule)
                })
            {
                return Err(err("boundaryExtent.payload", &[extent_field(6)]));
            }
            if let Some(p) = payload {
                p.quantize().map_err(payload_error)?;
            }
        }
    }
    if coverage.is_some() {
        for lane in root.authoring_lanes() {
            let (road, _) = lane
                .road_section()
                .split_once('>')
                .ok_or_else(|| invalid_combination("authoringLane.roadSection", doc))?;
            if lane.supports_wheel().is_some() != (by_key(road)? == Coverage::Complete) {
                return Err(invalid_combination("authoringLane.supportsWheel", doc)
                    .with_fallback_primary_location(
                        super::super::location::RoadEditingLocationFactory::invalid_lane_field(
                            ns, doc, lane, 8, None,
                        ),
                    ));
            }
        }
    }
    Ok(())
}
