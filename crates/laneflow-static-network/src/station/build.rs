use super::*;
use crate::builder::{
    allocate_vec, checked_field, checked_i32, checked_u8, checked_u32, poll_cancelled,
};
use crate::{
    BuildError, BuildStructure, CorridorElement, SharedNetworkBuildOptions, SharedTrafficNetwork,
};
use laneflow_format::{RegistryCheckedFieldValue, RegistryCheckedRowView, ValueCheckedObjectView};
use laneflow_static_contract::{EntityKind, RoadSectionOrdinal};

mod cross_section;
const STRUCTURE: BuildStructure = BuildStructure::StationMapping;
const INVALID: BuildError = BuildError::InputInvariant {
    structure: STRUCTURE,
};

fn number(row: RegistryCheckedRowView<'_>, tag: u16) -> Result<u64, BuildError> {
    match checked_field(row, tag, STRUCTURE)? {
        RegistryCheckedFieldValue::U64(n) => Ok(n),
        _ => Err(INVALID),
    }
}
fn index(row: RegistryCheckedRowView<'_>, tag: u16) -> Result<u32, BuildError> {
    checked_u32(row, tag, STRUCTURE)
}
fn check(ok: bool) -> Result<(), BuildError> {
    if ok { Ok(()) } else { Err(INVALID) }
}
fn mapping_row(table_kind: u16, row: u32) -> BuildError {
    BuildError::StationMappingRow { table_kind, row }
}
fn validate(at: BuildError, ok: bool) -> Result<(), BuildError> {
    if ok { Ok(()) } else { Err(at) }
}
fn slots<T: Copy>(n: u32, value: T) -> Result<Vec<T>, BuildError> {
    let mut v = allocate_vec(n, STRUCTURE)?;
    v.resize(n as usize, value);
    Ok(v)
}

/// 从表 1–5 顺序构建双向索引。表 6、7 也须与同一走廊的真实成员闭合。
pub(crate) fn build(
    view: ValueCheckedObjectView<'_>,
    traffic: &SharedTrafficNetwork,
    prior: u64,
    options: SharedNetworkBuildOptions<'_>,
) -> Result<SharedStationMapping, BuildError> {
    let section = view.registry_view().section(8).ok_or(INVALID)?;
    let table = |i| section.table(i).ok_or(INVALID);
    let corridors = table(0)?;
    let anchors = table(1)?;
    let lanes = table(2)?;
    let edges = table(3)?;
    let spans = table(4)?;
    let bands = table(5)?;
    let extents = table(6)?;
    let footprints = table(7)?;
    let surface = table(8)?;
    let rules = table(9)?;
    let obstacles = table(10)?;
    let limit = u64::from(options.limits().max_station_rows());
    for (t, structure) in [
        (anchors, BuildStructure::StationAnchors),
        (spans, BuildStructure::StationSpans),
        (extents, BuildStructure::BoundaryExtents),
    ] {
        let required = u64::from(t.row_count());
        if required > limit {
            return Err(BuildError::BudgetExceeded {
                structure,
                required,
                limit,
            });
        }
    }
    if corridors.row_count() == 0 {
        check(
            [
                anchors, lanes, edges, spans, bands, extents, footprints, surface, rules, obstacles,
            ]
            .iter()
            .all(|t| t.row_count() == 0),
        )?;
        return Ok(SharedStationMapping::default());
    }
    let counts = traffic.entity_counts();
    let nc = counts.count(EntityKind::RoadCorridor);
    let nl = counts.count(EntityKind::AuthoringLane);
    let ne = counts.count(EntityKind::LaneEdge);
    check(
        corridors.row_count() <= nc
            && lanes.row_count() <= nl
            && edges.row_count() <= ne
            && bands.row_count() <= counts.count(EntityKind::FacilityBand)
            && footprints.row_count() <= lanes.row_count()
            && surface.row_count() <= anchors.row_count()
            && u64::from(rules.row_count()) + u64::from(obstacles.row_count())
                <= u64::from(extents.row_count()),
    )?;
    let bytes = |n: u32, size: usize| u64::from(n) * size as u64;
    let retained = prior
        .checked_add(
            bytes(nc, size_of::<Option<StationDomain>>())
                + bytes(nl, size_of::<Option<Lane>>())
                + bytes(ne, size_of::<Option<Edge>>())
                + bytes(spans.row_count(), size_of::<Segment>())
                + bytes(bands.row_count(), size_of::<Band>())
                + bytes(extents.row_count(), size_of::<BoundaryExtent>())
                + bytes(footprints.row_count(), size_of::<LaneFootprint>())
                + bytes(surface.row_count(), size_of::<RoadSurfaceSpan>()),
        )
        .ok_or(INVALID)?;
    let anchor_errors = if extents.row_count() != 0 || surface.row_count() != 0 {
        anchors.row_count()
    } else {
        0
    };
    let scratch = bytes(nc, size_of::<Option<RangeU32>>())
        + bytes(anchors.row_count(), size_of::<u64>())
        + bytes(anchor_errors, size_of::<u32>())
        + if surface.row_count() > 0 {
            bytes(nc, size_of::<Option<u32>>())
        } else {
            0
        };
    for (required, limit, structure) in [
        (
            retained,
            options.limits().max_retained_bytes(),
            BuildStructure::RetainedOutput,
        ),
        (
            scratch,
            options.limits().max_scratch_bytes(),
            BuildStructure::BuilderScratch,
        ),
    ] {
        if required > limit {
            return Err(BuildError::BudgetExceeded {
                structure,
                required,
                limit,
            });
        }
    }
    let mut domains = slots(nc, None)?;
    let mut ranges = slots(nc, None)?;
    let mut stations = allocate_vec(anchors.row_count(), STRUCTURE)?;
    let mut errors = allocate_vec(anchor_errors, STRUCTURE)?;
    let mut anchor_rows = anchors.rows();
    let mut last = None;
    let mut expected_lanes = 0usize;
    let mut expected_bands = 0usize;
    for (i, row) in corridors.rows().enumerate() {
        let at = mapping_row(1, i as u32);
        let c = index(row, 1)?;
        poll_cancelled(options, c)?;
        validate(at, c < nc && last.is_none_or(|v| v < c))?;
        last = Some(c);
        let first = index(row, 5)?;
        let count = index(row, 6)?;
        let length = number(row, 3)?;
        validate(
            at,
            first as usize == stations.len()
                && count >= 2
                && first
                    .checked_add(count)
                    .is_some_and(|n| n <= anchors.row_count())
                && number(row, 2)?.checked_add(length).is_some(),
        )?;
        for i in 0..count {
            poll_cancelled(options, i)?;
            let anchor_at = mapping_row(2, first + i);
            let anchor = anchor_rows.next().ok_or(anchor_at)?;
            let s = number(anchor, 1)?;
            validate(
                anchor_at,
                if i == 0 {
                    s == 0
                } else {
                    s > *stations.last().ok_or(INVALID)?
                },
            )?;
            stations.push(s);
            if anchor_errors > 0 {
                errors.push(index(anchor, 2)?);
            }
        }
        validate(at, stations.last() == Some(&length))?;
        domains[c as usize] = Some(StationDomain {
            origin_nm: number(row, 2)?,
            length_nm: length,
            origin_error_nm: index(row, 4)?,
        });
        ranges[c as usize] = Some(RangeU32::new(first, count));
        let ord = RoadCorridorOrdinal::try_from_usize(c as usize).map_err(|_| INVALID)?;
        for member in traffic.relations().corridor_elements(ord).ok_or(INVALID)? {
            match member {
                CorridorElement::RoadSection(sec) => {
                    expected_lanes += traffic
                        .relations()
                        .section_lanes(*sec)
                        .ok_or(INVALID)?
                        .len()
                }
                CorridorElement::FacilityBand(_) => expected_bands += 1,
            }
        }
    }
    check(
        stations.len() == anchors.row_count() as usize
            && expected_lanes == lanes.row_count() as usize
            && expected_bands == bands.row_count() as usize,
    )?;
    let mut lane_index = slots(nl, None)?;
    let mut edge_index = slots(ne, None)?;
    let mut segments = allocate_vec(spans.row_count(), STRUCTURE)?;
    let mut edge_rows = edges.rows();
    let mut span_rows = spans.rows();
    let mut next_edge = 0;
    last = None;
    for (i, row) in lanes.rows().enumerate() {
        let lane_at = mapping_row(3, i as u32);
        let l = index(row, 1)?;
        poll_cancelled(options, l)?;
        validate(lane_at, l < nl && last.is_none_or(|v| v < l))?;
        last = Some(l);
        let lane = AuthoringLaneOrdinal::try_from_usize(l as usize).map_err(|_| INVALID)?;
        let sec = traffic.relations().authoring_section(lane).ok_or(lane_at)?;
        let corridor = traffic.relations().section_corridor(sec).ok_or(lane_at)?;
        let range = ranges[corridor.index()].ok_or(lane_at)?;
        let forward = match checked_u8(row, 2, STRUCTURE)? {
            1 => true,
            2 => false,
            _ => return Err(lane_at),
        };
        let chain = traffic
            .relations()
            .authoring_edge_chain(lane)
            .ok_or(lane_at)?;
        let count = index(row, 4)?;
        validate(
            lane_at,
            !chain.is_empty() && chain.len() == count as usize && index(row, 3)? == next_edge,
        )?;
        next_edge = next_edge.checked_add(count).ok_or(lane_at)?;
        let low = range.start();
        let high = low + range.len() - 1;
        let mut anchor = if forward { low } else { high };
        let first = segments.len() as u32;
        for (index_in_chain, &edge) in chain.iter().enumerate() {
            let edge_at = mapping_row(4, next_edge - count + index_in_chain as u32);
            let row = edge_rows.next().ok_or(edge_at)?;
            let start = index(row, 3)?;
            let n = index(row, 4)?;
            validate(
                edge_at,
                edge.raw() == index(row, 1)?
                    && index(row, 2)? == anchor
                    && start as usize == segments.len()
                    && (1..=4096).contains(&n),
            )?;
            let length_mm = traffic.lane_lengths_millimetres()[edge.index()];
            let mut p = 0;
            for i in 0..n {
                poll_cancelled(options, i)?;
                let span_at = mapping_row(5, segments.len() as u32);
                let row = span_rows.next().ok_or(span_at)?;
                let end = index(row, 1)?;
                let q = number(row, 2)?;
                validate(
                    span_at,
                    (low..=high).contains(&end)
                        && if forward { end > anchor } else { end < anchor },
                )?;
                let map = StationSpan::try_new(
                    [stations[anchor as usize], stations[end as usize]],
                    [p, q],
                    index(row, 3)?,
                    index(row, 4)?,
                    index(row, 5)?,
                )
                .map_err(|_| span_at)?;
                if i + 1 == n {
                    validate(
                        span_at,
                        map.progress_mm_at(stations[end as usize]) == Some(u64::from(length_mm)),
                    )?
                }
                segments.push(Segment { edge, map });
                anchor = end;
                p = q;
            }
            validate(edge_at, edge_index[edge.index()].is_none())?;
            edge_index[edge.index()] = Some(Edge {
                lane,
                length_mm,
                spans: RangeU32::new(start, n),
            });
        }
        validate(lane_at, anchor == if forward { high } else { low })?;
        lane_index[l as usize] = Some(Lane {
            corridor,
            increasing: forward,
            spans: RangeU32::new(first, segments.len() as u32 - first),
        });
    }
    validate(mapping_row(4, next_edge), edge_rows.next().is_none())?;
    validate(
        mapping_row(5, segments.len() as u32),
        span_rows.next().is_none(),
    )?;
    // 每个已开放区段的车道都在同一方向，允许对向车道位于另一有向区段。
    for i in 0..counts.count(EntityKind::RoadSection) {
        poll_cancelled(options, i)?;
        let sec = RoadSectionOrdinal::try_from_usize(i as usize).map_err(|_| INVALID)?;
        let mut direction = None;
        for l in traffic.relations().section_lanes(sec).ok_or(INVALID)? {
            if let Some(lane) = lane_index[l.index()] {
                check(direction.is_none_or(|d| d == lane.increasing))?;
                direction = Some(lane.increasing)
            }
        }
    }
    let mut band_index = allocate_vec(bands.row_count(), STRUCTURE)?;
    last = None;
    for (i, row) in bands.rows().enumerate() {
        let at = mapping_row(6, i as u32);
        let b = index(row, 1)?;
        poll_cancelled(options, b)?;
        validate(at, last.is_none_or(|v| v < b))?;
        last = Some(b);
        let ord = FacilityBandOrdinal::try_from_usize(b as usize).map_err(|_| INVALID)?;
        let c = traffic.relations().band_corridor(ord).ok_or(at)?;
        let range = ranges[c.index()].ok_or(at)?;
        validate(
            at,
            index(row, 2)? == range.start() && index(row, 3)? == range.start() + range.len() - 1,
        )?;
        band_index.push(Band {
            ordinal: ord,
            corridor: c,
            left_um: [
                checked_i32(row, 4, STRUCTURE)?,
                checked_i32(row, 6, STRUCTURE)?,
            ],
            right_um: [
                checked_i32(row, 5, STRUCTURE)?,
                checked_i32(row, 7, STRUCTURE)?,
            ],
            error_nm: index(row, 8)?,
        });
    }
    let mut named = allocate_vec(extents.row_count(), STRUCTURE)?;
    let mut prev = None;
    for (i, row) in extents.rows().enumerate() {
        let at = mapping_row(7, i as u32);
        poll_cancelled(options, i as u32)?;
        let owner = index(row, 2)? as usize;
        let (kind, c, size) = match checked_field(row, 1, STRUCTURE)? {
            RegistryCheckedFieldValue::U16(1) => {
                let c = RoadCorridorOrdinal::try_from_usize(owner).map_err(|_| INVALID)?;
                (
                    1,
                    c,
                    traffic.relations().corridor_elements(c).ok_or(at)?.len(),
                )
            }
            RegistryCheckedFieldValue::U16(2) => {
                let s = RoadSectionOrdinal::try_from_usize(owner).map_err(|_| INVALID)?;
                (
                    2,
                    traffic.relations().section_corridor(s).ok_or(at)?,
                    traffic.relations().section_lanes(s).ok_or(at)?.len(),
                )
            }
            _ => return Err(INVALID),
        };
        let left = index(row, 3)?;
        let shape = checked_u8(row, 4, STRUCTURE)?;
        let RegistryCheckedFieldValue::Utf8(name) = checked_field(row, 5, STRUCTURE)? else {
            return Err(INVALID);
        };
        let key = (kind, owner, left, shape, name);
        validate(
            at,
            prev.is_none_or(|k| k < key) && (left as usize) < size.saturating_sub(1),
        )?;
        prev = Some(key);
        let r = ranges[c.index()].ok_or(at)?;
        let lo = index(row, 6)?;
        let hi = index(row, 7)?;
        validate(at, r.start() <= lo && lo < hi && hi < r.start() + r.len())?;
        let mut key_bytes = [0; 53];
        key_bytes
            .get_mut(..name.len())
            .ok_or(at)?
            .copy_from_slice(name.as_bytes());
        let owner = if kind == 1 {
            BoundaryOwner::Corridor(c)
        } else {
            BoundaryOwner::Section(RoadSectionOrdinal::try_from_usize(owner).map_err(|_| at)?)
        };
        named.push(BoundaryExtent {
            owner,
            corridor: c,
            left_index: left,
            kind: shape,
            stations_nm: [stations[lo as usize], stations[hi as usize]],
            errors_nm: [errors[lo as usize], errors[hi as usize]],
            name: key_bytes,
            name_len: name.len() as u8,
            payload: None,
        });
    }
    let (footprints, surface) = cross_section::build(
        view,
        traffic,
        &ranges,
        &stations,
        &errors,
        &lane_index,
        &edge_index,
        &band_index,
        &mut named,
        options,
    )?;
    Ok(SharedStationMapping {
        domains: domains.into_boxed_slice(),
        lanes: lane_index.into_boxed_slice(),
        edges: edge_index.into_boxed_slice(),
        segments: segments.into_boxed_slice(),
        bands: band_index.into_boxed_slice(),
        extents: named.into_boxed_slice(),
        footprints,
        surface,
    })
}
