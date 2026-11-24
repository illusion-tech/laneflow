//! 将已证明的规范弦按所属走廊合并锚点，保持各车道独立的切边与行驶顺序。
use super::*;
use crate::ledger::PassBudget;
use crate::lir::LirUnit;
use std::cmp::Reverse;
type Result<T> = std::result::Result<T, PortableEmissionError>;
const INVALID: PortableEmissionError = PortableEmissionError::InternalBindingMismatch;

type Anchor = (u32, u64, Reverse<u32>);
type Corridor = (u32, u64, u64, u32, u32);
type Lane = (u32, u8, u32, u32);
type Edge = (u32, u32, u32, u32);
type Span = (u32, u64, u32, u32, u32);
type Band = (u32, u32, u32, [i32; 4], u32);

pub(super) struct Inputs {
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
    let count = n
        .edges
        .checked_add(n.spans)
        .ok_or(PortableEmissionError::ArithmeticOverflow)?;
    let plan = Plan::ZERO
        .array::<Anchor>(count)
        .array::<Corridor>(n.edges.min(lir.road_corridors.len()))
        .array::<Lane>(n.edges.min(lir.authoring_lanes.len()))
        .array::<Edge>(n.edges)
        .array::<Span>(n.spans)
        .array::<Band>(n.bands);
    Ok((n, plan))
}

pub(super) fn prepare(lir: &LirUnit, n: Sizes, pass: &PassBudget<'_>) -> Result<Inputs> {
    let mut m = Inputs {
        anchors: pass.vec(n.edges + n.spans)?,
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
                || p.corridor_length != head.corridor_length
                || p.start != start
            {
                return Err(INVALID);
            }
            let actual = &lir.spatial_segments[g.segments.as_usize_range()];
            if actual.len() != p.spans.len() {
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
            let (_, _, _, start, n) = m.corridors[i];
            m.bands.push((
                g.facility_band.raw(),
                start,
                start + n - 1,
                b.offsets_um,
                b.lateral_error,
            ))?;
        }
    }
    if m.spans.len() != n.spans || m.edges.len() != n.edges {
        return Err(INVALID);
    }
    Ok(m)
}
impl Inputs {
    pub(super) const fn bytes(&self) -> u64 {
        self.anchors.granted_bytes()
            + self.corridors.granted_bytes()
            + self.lanes.granted_bytes()
            + self.edges.granted_bytes()
            + self.spans.granted_bytes()
            + self.bands.granted_bytes()
    }
    pub(super) fn section(&self, t: &TreeBuilder) -> SectionRef {
        use Value::{I32, U8, U32, U64};
        t.section(
            9,
            [
                t.table(
                    1,
                    self.corridors.iter().map(|&(c, o, n, f, len)| {
                        t.row([
                            field(1, U32(c)),
                            field(2, U64(o)),
                            field(3, U64(n)),
                            field(4, U32(1)),
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
                t.table(7, []),
            ],
        )
    }
}
