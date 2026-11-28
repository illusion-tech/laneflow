//! 将已证明的规范弦按所属走廊合并锚点，保持各车道独立的切边与行驶顺序。
use super::*;
use crate::ledger::PassBudget;
use crate::lir::LirUnit;
use std::cmp::Reverse;
type Result<T> = std::result::Result<T, PortableEmissionError>;
const INVALID: PortableEmissionError = PortableEmissionError::InternalBindingMismatch;

type Anchor = (u32, u64, Reverse<u32>);
type Corridor = (u32, u64, u64, u32, u32, u32);
type Lane = (u32, u8, u32, u32);
type Edge = (u32, u32, u32, u32);
type Span = (u32, u64, u32, u32, u32);
type Band = (u32, u32, u32, [i32; 4], u32);

use crate::road_editing::boundary::{NamedExtent, SurfaceRow};
use crate::road_editing::boundary_payload::QuantizedBoundaryPayload as Payload;

type Extent<'a> = (u16, u32, &'a NamedExtent, u32, u32);
type Footprint = (u32, [i32; 4], u32, bool);
type Surface<'a> = (u32, u32, u32, u32, &'a SurfaceRow);

pub(super) struct Inputs<'a> {
    extents: BoundedVec<Extent<'a>>,
    footprints: BoundedVec<Footprint>,
    surface: BoundedVec<Surface<'a>>,
    corridors: BoundedVec<Corridor>,
    anchors: BoundedVec<Anchor>,
    lanes: BoundedVec<Lane>,
    edges: BoundedVec<Edge>,
    spans: BoundedVec<Span>,
    bands: BoundedVec<Band>,
}
#[derive(Default)]
pub(super) struct Sizes {
    edges: usize,
    spans: usize,
    bands: usize,
    extents: usize,
    footprints: usize,
    surface: usize,
}

pub(super) fn sizes(lir: &LirUnit) -> Result<(Sizes, Plan)> {
    let mut n = Sizes::default();
    for g in &lir.lane_edge_geometries {
        if let Some(e) = &g.station {
            n.edges += 1;
            n.spans = n
                .spans
                .checked_add(e.spans.len())
                .ok_or(PortableEmissionError::ArithmeticOverflow)?
        }
    }
    n.bands = lir
        .facility_band_geometries
        .iter()
        .filter(|g| g.station.is_some())
        .count();
    for c in &lir.road_corridors {
        if let Some(p) = &c.cross_section {
            n.extents += p.extents.len();
            n.footprints += p.widths.iter().filter(|w| w.lane.is_some()).count();
            n.surface += p.surface.len();
        }
    }
    let count = n
        .edges
        .checked_add(n.spans)
        .ok_or(PortableEmissionError::ArithmeticOverflow)?;
    let plan = Plan::ZERO
        .array::<Anchor>(count + 2 * (n.extents + n.surface))
        .array::<Extent>(n.extents)
        .array::<Footprint>(n.footprints)
        .array::<Surface>(n.surface)
        .array::<Corridor>(n.edges.min(lir.road_corridors.len()))
        .array::<Lane>(n.edges.min(lir.authoring_lanes.len()))
        .array::<Edge>(n.edges)
        .array::<Span>(n.spans)
        .array::<Band>(n.bands);
    Ok((n, plan))
}

pub(super) fn prepare<'a>(lir: &'a LirUnit, n: Sizes, pass: &PassBudget<'_>) -> Result<Inputs<'a>> {
    let mut m = Inputs {
        anchors: pass.vec(n.edges + n.spans + 2 * (n.extents + n.surface))?,
        extents: pass.vec(n.extents)?,
        footprints: pass.vec(n.footprints)?,
        surface: pass.vec(n.surface)?,
        corridors: pass.vec(n.edges.min(lir.road_corridors.len()))?,
        lanes: pass.vec(n.edges.min(lir.authoring_lanes.len()))?,
        edges: pass.vec(n.edges)?,
        spans: pass.vec(n.spans)?,
        bands: pass.vec(n.bands)?,
    };
    if n.edges == 0 && n.bands == 0 {
        return Ok(m);
    }
    let corridor = |l: &crate::lir::LirAuthoringLane| {
        lir.road_sections[l.road_section.index()]
            .road_corridor
            .raw()
    };
    let chain =
        |l: &crate::lir::LirAuthoringLane| &lir.authoring_lane_edges[l.edge_chain.as_usize_range()];
    for lane in &lir.authoring_lanes {
        let c = corridor(lane);
        for edge in chain(lane) {
            if let Some(p) = &lir
                .lane_edge_geometries
                .get(edge.index())
                .ok_or(INVALID)?
                .station
            {
                m.anchors.push((c, p.start, Reverse(p.start_error)))?;
                m.anchors.try_extend(
                    p.spans
                        .iter()
                        .map(|s| (c, s.station, Reverse(s.station_error))),
                )?;
            }
        }
    }
    for c in &lir.road_corridors {
        if let Some(p) = &c.cross_section {
            for stations in p
                .extents
                .iter()
                .map(|e| e.stations)
                .chain(p.surface.iter().map(|s| s.stations))
            {
                m.anchors
                    .try_extend(stations.map(|(s, e)| (c.ordinal.raw(), s, Reverse(e))))?;
            }
        }
    }
    m.anchors.sort_unstable();
    m.anchors.dedup_by_key(|a| (a.0, a.1));
    let anchor = |c, s| {
        m.anchors
            .binary_search_by_key(&(c, s), |a| (a.0, a.1))
            .map(|i| i as u32)
            .map_err(|_| INVALID)
    };
    for lane in &lir.authoring_lanes {
        let c = corridor(lane);
        let edges = chain(lane);
        let Some(first) = edges.first() else { continue };
        let Some(head) = &lir.lane_edge_geometries[first.index()].station else {
            continue;
        };
        let tail = head.spans.last().ok_or(INVALID)?.station;
        let forward = tail > head.start;
        let mut start = if forward { 0 } else { head.corridor_length };
        let first_edge = m.edges.len() as u32;
        for &e in edges {
            let g = &lir.lane_edge_geometries[e.index()];
            let p = g.station.as_ref().ok_or(INVALID)?;
            if p.origin != head.origin
                || p.origin_error_mm != head.origin_error_mm
                || p.corridor_length != head.corridor_length
                || p.start != start
            {
                return Err(INVALID);
            }
            let actual = &lir.spatial_segments[g.segments.as_usize_range()];
            let terminal = actual.last().ok_or(INVALID)?.cumulative_end_meters;
            if actual.len() != p.spans.len()
                || laneflow_static_contract::millimetres_from_si(f64::from(terminal))
                    != Some(lir.lane_edges[e.index()].length_mm)
            {
                return Err(INVALID);
            }
            m.edges.push((
                e.raw(),
                anchor(c, start)?,
                m.spans.len() as u32,
                actual.len() as u32,
            ))?;
            for (s, segment) in p.spans.iter().zip(actual) {
                if s.cumulative.to_bits() != segment.cumulative_end_meters.to_bits()
                    || (s.station > start) != forward
                {
                    return Err(INVALID);
                }
                m.spans.push((
                    anchor(c, s.station)?,
                    s.progress,
                    s.canonical_error,
                    s.travel_error,
                    s.rate,
                ))?;
                start = s.station;
            }
        }
        if start != if forward { head.corridor_length } else { 0 } {
            return Err(INVALID);
        }
        m.lanes.push((
            lane.ordinal.raw(),
            if forward { 1 } else { 2 },
            first_edge,
            edges.len() as u32,
        ))?;
    }
    for chunk in m.anchors.chunk_by(|a, b| a.0 == b.0) {
        let (c, _, _) = chunk[0];
        let r = &lir.road_corridors[c as usize];
        let section = &lir.road_sections[r.reference_section.index()];
        let lane = lir.road_section_lanes[section.lanes.as_usize_range()]
            .first()
            .ok_or(INVALID)?;
        let e = chain(&lir.authoring_lanes[lane.index()])
            .first()
            .ok_or(INVALID)?;
        let head = lir.lane_edge_geometries[e.index()]
            .station
            .as_ref()
            .ok_or(INVALID)?;
        if chunk[0].1 != 0 || chunk.last().ok_or(INVALID)?.1 != head.corridor_length {
            return Err(INVALID);
        }
        m.corridors.push((
            c,
            head.origin,
            head.corridor_length,
            head.origin_error_mm,
            anchor(c, 0)?,
            chunk.len() as u32,
        ))?;
    }
    for g in &lir.facility_band_geometries {
        if let Some(b) = g.station {
            let c = lir.facility_bands[g.facility_band.index()]
                .road_corridor
                .raw();
            let i = m
                .corridors
                .binary_search_by_key(&c, |r| r.0)
                .map_err(|_| INVALID)?;
            let (_, _, _, _, start, n) = m.corridors[i];
            m.bands.push((
                g.facility_band.raw(),
                start,
                start + n - 1,
                b.offsets_mm,
                b.lateral_error,
            ))?;
        }
    }
    for c in &lir.road_corridors {
        let Some(p) = &c.cross_section else { continue };
        let corridor = c.ordinal.raw();
        for w in &p.widths {
            if let Some((proof, supports)) = w.lane {
                let (_, l) = c.width_owner(lir, w.member).ok_or(INVALID)?;
                m.footprints
                    .push((l, proof.offsets_mm, proof.lateral_error, supports))?;
            }
        }
        for e in &p.extents {
            let (kind, ord) = c.extent_owner(lir, e.section_element).ok_or(INVALID)?;
            m.extents.push((
                kind.code(),
                ord,
                &e.declaration,
                anchor(corridor, e.stations[0].0)?,
                anchor(corridor, e.stations[1].0)?,
            ))?;
        }
        if !p.surface.is_empty() {
            let s = &lir.road_sections[c.reference_section.index()];
            let l = lir.road_section_lanes[s.lanes.as_usize_range()]
                .first()
                .ok_or(INVALID)?;
            let e = chain(&lir.authoring_lanes[l.index()])
                .first()
                .ok_or(INVALID)?;
            let frame = lir.lane_edge_geometries[e.index()].canonical_frame.raw();
            for row in &p.surface {
                m.surface.push((
                    corridor,
                    frame,
                    anchor(corridor, row.stations[0].0)?,
                    anchor(corridor, row.stations[1].0)?,
                    row,
                ))?;
            }
        }
    }
    m.footprints.sort_unstable_by_key(|r| r.0);
    m.extents.sort_unstable_by(|a, b| {
        (a.0, a.1, a.2.left, a.2.kind, &a.2.key).cmp(&(b.0, b.1, b.2.left, b.2.kind, &b.2.key))
    });
    if m.spans.len() != n.spans || m.edges.len() != n.edges {
        return Err(INVALID);
    }
    Ok(m)
}
impl Inputs<'_> {
    pub(super) const fn bytes(&self) -> u64 {
        self.anchors.granted_bytes()
            + self.corridors.granted_bytes()
            + self.lanes.granted_bytes()
            + self.edges.granted_bytes()
            + self.spans.granted_bytes()
            + self.bands.granted_bytes()
            + self.extents.granted_bytes()
            + self.footprints.granted_bytes()
            + self.surface.granted_bytes()
    }
    pub(super) fn section(&self, t: &TreeBuilder) -> SectionRef {
        use Value::{I32, U8, U32, U64};
        t.section(
            9,
            [
                t.table(
                    1,
                    self.corridors.iter().map(|&(c, o, n, error, f, len)| {
                        t.row([
                            field(1, U32(c)),
                            field(2, U64(o)),
                            field(3, U64(n)),
                            field(4, U32(error)),
                            field(5, U32(f)),
                            field(6, U32(len)),
                        ])
                    }),
                ),
                t.table(
                    2,
                    self.anchors
                        .iter()
                        .map(|&(_, s, Reverse(e))| t.row([field(1, U64(s)), field(2, U32(e))])),
                ),
                t.table(
                    3,
                    self.lanes.iter().map(|&(l, d, f, n)| {
                        t.row([
                            field(1, U32(l)),
                            field(2, U8(d)),
                            field(3, U32(f)),
                            field(4, U32(n)),
                        ])
                    }),
                ),
                t.table(
                    4,
                    self.edges.iter().map(|&(e, a, f, n)| {
                        t.row([
                            field(1, U32(e)),
                            field(2, U32(a)),
                            field(3, U32(f)),
                            field(4, U32(n)),
                        ])
                    }),
                ),
                t.table(
                    5,
                    self.spans.iter().map(|&(a, p, e, q, r)| {
                        t.row([
                            field(1, U32(a)),
                            field(2, U64(p)),
                            field(3, U32(e)),
                            field(4, U32(q)),
                            field(5, U32(r)),
                        ])
                    }),
                ),
                t.table(
                    6,
                    self.bands.iter().map(|&(b, l, h, d, e)| {
                        t.row([
                            field(1, U32(b)),
                            field(2, U32(l)),
                            field(3, U32(h)),
                            field(4, I32(d[0])),
                            field(5, I32(d[1])),
                            field(6, I32(d[2])),
                            field(7, I32(d[3])),
                            field(8, U32(e)),
                        ])
                    }),
                ),
                t.table(
                    7,
                    self.extents.iter().map(|&(kind, owner, e, low, high)| {
                        t.row([
                            field(1, Value::U16(kind)),
                            field(2, U32(owner)),
                            field(3, U32(e.left)),
                            field(4, U8(e.kind)),
                            field(5, t.utf8(&e.key)),
                            field(6, U32(low)),
                            field(7, U32(high)),
                        ])
                    }),
                ),
                t.table(
                    8,
                    self.footprints.iter().map(|&(lane, d, error, supports)| {
                        t.row([
                            field(1, U32(lane)),
                            field(2, I32(d[0])),
                            field(3, I32(d[1])),
                            field(4, I32(d[2])),
                            field(5, I32(d[3])),
                            field(6, U32(error)),
                            field(7, U8(u8::from(supports))),
                        ])
                    }),
                ),
                t.table(
                    9,
                    self.surface.iter().map(|&(corridor, frame, low, high, s)| {
                        t.row([
                            field(1, U32(corridor)),
                            field(2, U32(frame)),
                            field(3, U32(low)),
                            field(4, U32(high)),
                            field(5, t.bytes(s.jets[0].map(f32::to_le_bytes).as_flattened())),
                            field(6, t.bytes(s.jets[1].map(f32::to_le_bytes).as_flattened())),
                            field(7, t.bytes(s.errors.map(f32::to_le_bytes).as_flattened())),
                        ])
                    }),
                ),
                t.table(
                    10,
                    self.extents
                        .iter()
                        .enumerate()
                        .filter_map(|(i, e)| match e.2.payload? {
                            Payload::Rule { directions, effect } => Some(t.row([
                                field(1, U32(i as u32)),
                                field(2, U8(directions)),
                                field(3, U8(effect)),
                            ])),
                            _ => None,
                        }),
                ),
                t.table(
                    11,
                    self.extents.iter().enumerate().filter_map(|(i, e)| {
                        let mut bytes = [0; 36];
                        let (shape, len) = e.2.payload?.obstacle_bytes(&mut bytes)?;
                        Some(t.row([
                            field(1, U32(i as u32)),
                            field(2, U8(shape)),
                            field(3, t.bytes(&bytes[..len])),
                        ]))
                    }),
                ),
            ],
        )
    }
}
