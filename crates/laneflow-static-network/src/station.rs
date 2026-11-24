//! 同一修订内，走廊截面与有向边局部进度之间的有界查询。
use laneflow_static_contract::{
    AuthoringLaneOrdinal, LaneEdgeOrdinal, RoadCorridorOrdinal, StationSpan,
};

use crate::RangeU32;

mod build;
pub(crate) use build::build;

/// 走廊相对于道路参考线的起点和可查询长度。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StationDomain {
    /// 在道路参考线上的起点，单位纳米。
    pub origin_nm: u64,
    /// 走廊局部域为零至该长度的闭区间。
    pub length_nm: u64,
    /// 来源起点相对于规范起点的误差上界。
    pub origin_error_nm: u32,
}

/// 一次边进度反查的结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StationObservation {
    /// 源边没有站位映射。
    Unavailable,
    /// 输入进度越出源边。
    Outside,
    /// 查询预算内无法证明覆盖整个观测。
    Unproven {
        /// 包含未检查站位的走廊局部区间。
        unchecked_nm: [u64; 2],
    },
    /// 该观测的所有可能站位均在给出的闭区间内。
    Complete {
        /// 站位所属走廊。
        corridor: RoadCorridorOrdinal,
        /// 所属走廊的局部站位闭区间，单位纳米。
        stations_nm: [u64; 2],
    },
}

/// 一条候选边及保守的局部毫米进度闭区间。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StationAddress {
    /// 沿目标车道行驶方向的规范边。
    pub edge: LaneEdgeOrdinal,
    /// 已裁到边长的进度闭区间。
    pub progress_mm: [u32; 2],
}

/// 一次跨车道查询的完整结果；工作量与返回的候选数均有固定上界。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StationQuery {
    /// 输入边或车道没有映射，或它们属于不同走廊。
    Unavailable,
    /// 输入进度或站位越出所选边或走廊。
    Outside,
    /// 还有站位未检查，无法确认全部候选。
    Unproven {
        /// 所属走廊的局部站位闭区间，以纳米计。
        unchecked_nm: [u64; 2],
    },
    /// 已覆盖整个站位区间。
    Complete(StationCandidates),
}

/// 有界工作区按目标车道行驶方向保存候选边。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StationCandidates {
    stations: [u64; 2],
    addresses: [Option<StationAddress>; 8],
    len: usize,
}

impl StationCandidates {
    /// 源观测对应的可能站位闭区间。
    #[must_use]
    pub const fn stations_nm(&self) -> [u64; 2] {
        self.stations
    }

    /// 枚举所有候选及其各自的进度范围。
    pub fn addresses(&self) -> impl Iterator<Item = StationAddress> + '_ {
        self.addresses[..self.len].iter().flatten().copied()
    }
}

#[derive(Clone, Copy)]
struct Lane {
    corridor: RoadCorridorOrdinal,
    increasing: bool,
    spans: RangeU32,
}

#[derive(Clone, Copy)]
struct Edge {
    lane: AuthoringLaneOrdinal,
    length_mm: u32,
    spans: RangeU32,
}

#[derive(Clone, Copy)]
struct Segment {
    edge: LaneEdgeOrdinal,
    map: StationSpan,
}

/// 从受检 LFCA 构建，只保存交通裁决所需的站位列。
#[derive(Default)]
pub struct SharedStationMapping {
    domains: Box<[Option<StationDomain>]>,
    lanes: Box<[Option<Lane>]>,
    edges: Box<[Option<Edge>]>,
    segments: Box<[Segment]>,
}

// 每次比较后直接排除一个中点。4096 段至多比较 13 次，1600 万段至多 24 次。
fn first_matching_span(spans: &[Segment], before: impl Fn(&Segment) -> bool) -> usize {
    let (mut lo, mut hi) = (0, spans.len());
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if before(&spans[mid]) {
            lo = mid + 1
        } else {
            hi = mid
        }
    }
    lo
}

impl SharedStationMapping {
    pub(crate) fn retained_logical_bytes(&self) -> u64 {
        (size_of_val(&*self.domains)
            + size_of_val(&*self.lanes)
            + size_of_val(&*self.edges)
            + size_of_val(&*self.segments)) as u64
    }

    /// 返回已开放走廊的参考线起点与局部长度。
    #[must_use]
    pub fn corridor_domain(&self, corridor: RoadCorridorOrdinal) -> Option<StationDomain> {
        self.domains.get(corridor.index()).copied().flatten()
    }

    /// 仅反查普通整数毫米读数；结果以源边所属走廊的局部站位表示。
    #[must_use]
    pub fn stations_for_progress(
        &self,
        edge: LaneEdgeOrdinal,
        progress_mm: u32,
    ) -> StationObservation {
        match self.edges.get(edge.index()).copied().flatten() {
            Some(source) => self.observe(source, progress_mm, &mut 64),
            None => StationObservation::Unavailable,
        }
    }

    /// 从同一走廊内的规范站位投影到目标车道。内切点归沿行驶方向的下游边。
    #[must_use]
    pub fn at_station(
        &self,
        corridor: RoadCorridorOrdinal,
        station_nm: u64,
        target: AuthoringLaneOrdinal,
    ) -> StationQuery {
        let Some(lane) = self.lanes.get(target.index()).copied().flatten() else {
            return StationQuery::Unavailable;
        };
        if lane.corridor != corridor {
            return StationQuery::Unavailable;
        }
        self.project(lane, [station_nm; 2], 64)
    }

    /// 从源边的普通整数毫米观测反查走廊站位，再投影到目标车道。
    #[must_use]
    pub fn from_progress(
        &self,
        edge: LaneEdgeOrdinal,
        progress_mm: u32,
        target: AuthoringLaneOrdinal,
    ) -> StationQuery {
        let Some(lane) = self.lanes.get(target.index()).copied().flatten() else {
            return StationQuery::Unavailable;
        };
        let Some(source) = self.edges.get(edge.index()).copied().flatten() else {
            return StationQuery::Unavailable;
        };
        if self.lanes[source.lane.index()].is_none_or(|l| l.corridor != lane.corridor) {
            return StationQuery::Unavailable;
        }
        let mut budget = 64;
        match self.observe(source, progress_mm, &mut budget) {
            StationObservation::Complete { stations_nm, .. } => {
                self.project(lane, stations_nm, budget)
            }
            StationObservation::Unavailable => StationQuery::Unavailable,
            StationObservation::Outside => StationQuery::Outside,
            StationObservation::Unproven { unchecked_nm } => {
                StationQuery::Unproven { unchecked_nm }
            }
        }
    }

    fn observe(&self, source: Edge, progress_mm: u32, budget: &mut u8) -> StationObservation {
        if progress_mm > source.length_mm {
            return StationObservation::Outside;
        }
        let center = u64::from(progress_mm) * 1_000_000;
        let radius = 500_000 + u64::from(StationSpan::MAX_CANONICAL_ERROR_NM);
        let spans = source.spans.slice(&self.segments);
        let start = first_matching_span(spans, |s| {
            s.map.progress_nm()[1] < center.saturating_sub(radius)
        });
        let mut range = [u64::MAX, 0];
        for segment in &spans[start..] {
            if segment.map.progress_nm()[0] > center + radius {
                break;
            }
            if *budget == 0 {
                let a = segment.map.stations_nm()[0];
                let b = spans[spans.len() - 1].map.stations_nm()[1];
                return StationObservation::Unproven {
                    unchecked_nm: [a.min(b), a.max(b)],
                };
            }
            *budget -= 1;
            if let Some([a, b]) = segment.map.stations_for_progress_mm(progress_mm) {
                range = [range[0].min(a), range[1].max(b)];
            }
        }
        if range[0] > range[1] {
            return StationObservation::Outside;
        }
        StationObservation::Complete {
            corridor: self.lanes[source.lane.index()]
                .expect("validated mapping owner")
                .corridor,
            stations_nm: range,
        }
    }

    fn project(&self, lane: Lane, stations: [u64; 2], mut budget: u8) -> StationQuery {
        let spans = lane.spans.slice(&self.segments);
        let last = spans.len() - 1;
        let [lo, hi] = stations;
        let (entry, exit) = if lane.increasing { (lo, hi) } else { (hi, lo) };
        let lane_start = spans[0].map.stations_nm()[0];
        let lane_end = spans[last].map.stations_nm()[1];
        if lo < lane_start.min(lane_end) || hi > lane_start.max(lane_end) {
            return StationQuery::Outside;
        }
        // 同一边相邻分段在共同节点可登记不同误差，因此普通节点需覆盖两侧。
        let first = first_matching_span(spans, |s| {
            let end = s.map.stations_nm()[1];
            if lane.increasing {
                end < entry
            } else {
                end > entry
            }
        });
        let mut found = StationCandidates {
            stations,
            addresses: [None; 8],
            len: 0,
        };
        for (i, s) in spans.iter().enumerate().skip(first) {
            let [a, b] = s.map.stations_nm();
            if (lane.increasing && a > exit) || (!lane.increasing && a < exit) {
                break;
            }
            let x = lo.max(a.min(b));
            let y = hi.min(a.max(b));
            let end_edge = i == last || spans[i + 1].edge != s.edge;
            if budget == 0 {
                return StationQuery::Unproven {
                    unchecked_nm: if lane.increasing { [x, hi] } else { [lo, y] },
                };
            }
            budget -= 1;
            if x == y && x == b && end_edge && i != last {
                continue;
            }
            let edge = self.edges[s.edge.index()].expect("validated mapping owner");
            let length = u64::from(edge.length_mm);
            let bounds = |v| {
                if v == a && s.map.progress_nm()[0] == 0 {
                    return [0, 0];
                }
                if v == b && end_edge && i == last {
                    return [length, length];
                }
                s.map
                    .progress_mm_bounds_at(v)
                    .expect("intersection is in segment")
                    .map(|p| p.min(length))
            };
            let (begin, end) = if lane.increasing { (x, y) } else { (y, x) };
            let mut low = bounds(begin)[0];
            let mut high = bounds(end)[1];
            // 在精确拓扑端点，地址固定。含邻近内部站位时，保留单侧极限的误差包围。
            if begin != end {
                low = low.min(s.map.progress_mm_bounds_at(begin).unwrap()[0].min(length));
                high = high.max(s.map.progress_mm_bounds_at(end).unwrap()[1].min(length));
            }
            let progress = [low as u32, high as u32];
            if let Some(Some(prev)) = found.len.checked_sub(1).map(|i| &mut found.addresses[i])
                && prev.edge == s.edge
            {
                prev.progress_mm = [
                    prev.progress_mm[0].min(progress[0]),
                    prev.progress_mm[1].max(progress[1]),
                ];
                continue;
            }
            if found.len == found.addresses.len() {
                return StationQuery::Unproven {
                    unchecked_nm: if lane.increasing { [x, hi] } else { [lo, y] },
                };
            }
            found.addresses[found.len] = Some(StationAddress {
                edge: s.edge,
                progress_mm: progress,
            });
            found.len += 1;
        }
        StationQuery::Complete(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn mapping(edges: usize, each: usize, reverse: bool) -> (SharedStationMapping, Lane) {
        let corridor = RoadCorridorOrdinal::try_from_usize(0).unwrap();
        let lane = AuthoringLaneOrdinal::try_from_usize(0).unwrap();
        let length = (edges * each) as u64 * 1_000_000;
        let mut segments = Vec::new();
        let mut owners = Vec::new();
        for edge in 0..edges {
            let id = LaneEdgeOrdinal::try_from_usize(edge).unwrap();
            let first = segments.len();
            for k in 0..each {
                let station = (edge * each + k) as u64 * 1_000_000;
                let s =
                    [station, station + 1_000_000].map(|v| if reverse { length - v } else { v });
                let p = k as u64 * 1_000_000;
                segments.push(Segment {
                    edge: id,
                    map: StationSpan::try_new(s, [p, p + 1_000_000], 0, 0, 1_000_000).unwrap(),
                });
            }
            owners.push(Some(Edge {
                lane,
                length_mm: each as u32,
                spans: RangeU32::new(first as u32, each as u32),
            }));
        }
        let row = Lane {
            corridor,
            increasing: !reverse,
            spans: RangeU32::new(0, segments.len() as u32),
        };
        (
            SharedStationMapping {
                domains: vec![Some(StationDomain {
                    origin_nm: 0,
                    length_nm: length,
                    origin_error_nm: 0,
                })]
                .into(),
                lanes: vec![Some(row)].into(),
                edges: owners.into(),
                segments: segments.into(),
            },
            row,
        )
    }

    #[test]
    fn exact_query_limits_include_last_allowed_span_and_candidate() {
        for reverse in [false, true] {
            for (edges, each, complete) in [(8, 8, true), (1, 65, false), (9, 1, false)] {
                let (map, lane) = mapping(edges, each, reverse);
                let end = (edges * each) as u64 * 1_000_000;
                let result = map.project(lane, [0, end], 64);
                match result {
                    StationQuery::Complete(c) if complete => {
                        assert_eq!(c.addresses().count(), edges)
                    }
                    StationQuery::Unproven { unchecked_nm } if !complete => {
                        assert!(unchecked_nm[0] < unchecked_nm[1] && unchecked_nm[1] <= end)
                    }
                    other => panic!("unexpected {other:?}"),
                }
            }
        }
    }

    #[test]
    fn inverse_and_projection_spend_one_shared_budget() {
        let target = AuthoringLaneOrdinal::try_from_usize(0).unwrap();
        let source = LaneEdgeOrdinal::try_from_usize(0).unwrap();
        for reverse in [false, true] {
            for count in [32, 33, 64, 65] {
                let (mut map, _) = mapping(1, count, reverse);
                let end = count as u64 * 100_000;
                for (i, seg) in map.segments.iter_mut().enumerate() {
                    let p = [i as u64 * 100_000, (i as u64 + 1) * 100_000];
                    let s = p.map(|v| if reverse { end - v } else { v });
                    seg.map = StationSpan::try_new(
                        s,
                        p,
                        StationSpan::MAX_CANONICAL_ERROR_NM,
                        0,
                        1_000_000,
                    )
                    .unwrap();
                }
                map.edges[0].as_mut().unwrap().length_mm =
                    (end as f64 / 1e6).round_ties_even() as u32;
                let observed = map.stations_for_progress(source, 0);
                match observed {
                    StationObservation::Complete { stations_nm, .. } if count <= 64 => {
                        assert_eq!(stations_nm, [0, end])
                    }
                    StationObservation::Unproven { unchecked_nm } if count == 65 => assert_eq!(
                        unchecked_nm,
                        if reverse {
                            [0, 100_000]
                        } else {
                            [end - 100_000, end]
                        }
                    ),
                    other => panic!("unexpected source result {other:?}"),
                }
                let result = map.from_progress(source, 0, target);
                if count == 32 {
                    assert!(matches!(result, StationQuery::Complete(_)));
                } else {
                    assert!(matches!(result, StationQuery::Unproven { .. }));
                }
            }
        }
    }

    #[test]
    fn terminal_anchor_and_millimetre_observation_have_different_domains() {
        let (map, lane) = mapping(2, 128, false);
        let l = AuthoringLaneOrdinal::try_from_usize(0).unwrap();
        let e = LaneEdgeOrdinal::try_from_usize(0).unwrap();
        let StationQuery::Complete(at) = map.at_station(lane.corridor, 128_000_000, l) else {
            panic!()
        };
        assert_eq!(
            at.addresses().collect::<Vec<_>>(),
            [StationAddress {
                edge: LaneEdgeOrdinal::try_from_usize(1).unwrap(),
                progress_mm: [0, 0]
            }]
        );
        let StationQuery::Complete(reading) = map.from_progress(e, 128, l) else {
            panic!()
        };
        assert_eq!(reading.addresses().count(), 2);
        assert_eq!(map.from_progress(e, 129, l), StationQuery::Outside);
    }
}
