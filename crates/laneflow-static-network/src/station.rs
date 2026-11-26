//! 同一修订内，走廊截面与有向边局部进度之间的有界查询。
use laneflow_static_contract::{
    AuthoringLaneOrdinal, FacilityBandOrdinal, LaneEdgeOrdinal, RoadCorridorOrdinal, StationSpan,
};

use crate::RangeU32;

mod build;
mod cross_section;
pub mod vehicle;
pub(crate) use build::build;
pub use cross_section::{
    BoundaryExtent, BoundaryOwner, BoundaryPayload, LaneFootprint, RoadSurfaceSpan,
};

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

/// 所查站位范围内，设施带两条边界各自的横向偏移闭区间，沿参考方向左正。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FacilityBandBounds {
    /// 左边界偏移的下、上界，单位纳米，已计入横向误差。
    pub left_nm: [i64; 2],
    /// 右边界偏移的下、上界，单位纳米，已计入横向误差。
    pub right_nm: [i64; 2],
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
    /// 输入进度或站位越出所选映射的覆盖域，或站位区间倒序。
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

struct Band {
    ordinal: FacilityBandOrdinal,
    corridor: RoadCorridorOrdinal,
    left_um: [i32; 2],
    right_um: [i32; 2],
    error_nm: u32,
}

/// 从受检 LFCA 构建，只保存交通裁决所需的站位列。
#[derive(Default)]
pub struct SharedStationMapping {
    domains: Box<[Option<StationDomain>]>,
    lanes: Box<[Option<Lane>]>,
    edges: Box<[Option<Edge>]>,
    segments: Box<[Segment]>,
    bands: Box<[Band]>,
    extents: Box<[BoundaryExtent]>,
    footprints: Box<[LaneFootprint]>,
    surface: Box<[RoadSurfaceSpan]>,
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
            + size_of_val(&*self.segments)
            + size_of_val(&*self.bands)
            + size_of_val(&*self.extents)
            + size_of_val(&*self.footprints)
            + size_of_val(&*self.surface)) as u64
    }

    /// 完整开放走廊的路面区间按站位排列。空切片表示没有完整的横断面声明。
    #[must_use]
    pub fn surface_spans(&self, corridor: RoadCorridorOrdinal) -> &[RoadSurfaceSpan] {
        let first = self.surface.partition_point(|s| s.corridor < corridor);
        let end = self.surface.partition_point(|s| s.corridor <= corridor);
        &self.surface[first..end]
    }
    /// 返回该车道声明的路幅与支承位。
    #[must_use]
    pub fn lane_footprint(&self, lane: AuthoringLaneOrdinal) -> Option<&LaneFootprint> {
        self.footprints
            .binary_search_by_key(&lane, |f| f.lane)
            .ok()
            .map(|i| &self.footprints[i])
    }
    /// 在本修订内按所有者、左成员、种类及局部名排列的命名区间。
    #[must_use]
    pub fn boundary_extents(&self) -> &[BoundaryExtent] {
        &self.extents
    }

    /// 返回已开放走廊的参考线起点与局部长度。
    #[must_use]
    pub fn corridor_domain(&self, corridor: RoadCorridorOrdinal) -> Option<StationDomain> {
        self.domains.get(corridor.index()).copied().flatten()
    }

    /// 查询走廊局部站位闭区间内的设施带边界。单截面以两个相同的纳米站位表示。
    /// 对已发布设施带按序号二分定位，沿走廊递增方向插值，返回向外取整的误差包络。
    /// 走廊或设施带未开放映射、归属不符、站位倒序或越界时返回空。
    #[must_use]
    pub fn facility_band_bounds(
        &self,
        corridor: RoadCorridorOrdinal,
        band: FacilityBandOrdinal,
        stations_nm: [u64; 2],
    ) -> Option<FacilityBandBounds> {
        let length = self.corridor_domain(corridor)?.length_nm;
        if stations_nm[0] > stations_nm[1] || stations_nm[1] > length {
            return None;
        }
        let index = self.bands.binary_search_by_key(&band, |b| b.ordinal).ok()?;
        let band = &self.bands[index];
        if band.corridor != corridor {
            return None;
        }
        let bounds = |[start, end]: [i32; 2]| {
            // 长度为 u64、横向端点为 i32 微米，分子的带符号纳米积可由 i128 完整容纳。
            let values = stations_nm.map(|s| {
                (i128::from(start) * i128::from(length - s) + i128::from(end) * i128::from(s))
                    * 1_000
            });
            let denominator = i128::from(length);
            let low = values[0].min(values[1]).div_euclid(denominator);
            let upper = values[0].max(values[1]);
            let high =
                upper.div_euclid(denominator) + i128::from(upper.rem_euclid(denominator) != 0);
            let error = i64::from(band.error_nm);
            [low as i64 - error, high as i64 + error]
        };
        Some(FacilityBandBounds {
            left_nm: bounds(band.left_um),
            right_nm: bounds(band.right_um),
        })
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
        self.in_station_range(corridor, [station_nm; 2], target)
    }

    /// 把走廊局部站位闭区间投影到一条车道，按行驶顺序返回各边及其毫米进度范围。
    /// 正反向车道均按站位从小到大传入两端。内切点归下游边，全链终点归末边。
    /// 区间倒序或部分越界返回 `Outside`，缺少同走廊映射返回 `Unavailable`。
    /// 超过 64 个分段或八条候选边时返回 `Unproven` 和未检查的站位区间。
    #[must_use]
    pub fn in_station_range(
        &self,
        corridor: RoadCorridorOrdinal,
        stations_nm: [u64; 2],
        target: AuthoringLaneOrdinal,
    ) -> StationQuery {
        let Some(lane) = self.lanes.get(target.index()).copied().flatten() else {
            return StationQuery::Unavailable;
        };
        if lane.corridor != corridor {
            return StationQuery::Unavailable;
        }
        self.project(lane, stations_nm, 64)
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
        if lo > hi || lo < lane_start.min(lane_end) || hi > lane_start.max(lane_end) {
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
                ..SharedStationMapping::default()
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
                let result = map.in_station_range(
                    lane.corridor,
                    [0, end],
                    AuthoringLaneOrdinal::from_raw(0),
                );
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
    fn closed_station_range_observes_directed_edge_ownership_at_both_ends() {
        let target = AuthoringLaneOrdinal::from_raw(0);
        for reverse in [false, true] {
            let (map, lane) = mapping(3, 3, reverse);
            // 沿行驶方向的 2 至 6 毫米含两个切点，终点刚进入第三条边。
            let stations = if reverse {
                [3_000_000, 7_000_000]
            } else {
                [2_000_000, 6_000_000]
            };
            let StationQuery::Complete(candidates) =
                map.in_station_range(lane.corridor, stations, target)
            else {
                panic!("expected complete coverage");
            };
            assert_eq!(candidates.stations_nm(), stations);
            let addresses: Vec<_> = candidates
                .addresses()
                .map(|a| (a.edge.raw(), a.progress_mm))
                .collect();
            assert_eq!(addresses, [(0, [2, 3]), (1, [0, 3]), (2, [0, 0])]);

            for (distance_mm, expected_edge, progress_mm) in [(0, 0, 0), (3, 1, 0), (9, 2, 3)] {
                let station_nm = if reverse {
                    9 - distance_mm
                } else {
                    distance_mm
                } * 1_000_000;
                let result = map.in_station_range(lane.corridor, [station_nm; 2], target);
                assert_eq!(map.at_station(lane.corridor, station_nm, target), result);
                let StationQuery::Complete(at) = result else {
                    panic!("expected exact point");
                };
                assert_eq!(
                    at.addresses().collect::<Vec<_>>(),
                    [StationAddress {
                        edge: LaneEdgeOrdinal::from_raw(expected_edge),
                        progress_mm: [progress_mm; 2],
                    }]
                );
            }
        }
    }

    #[test]
    fn station_range_rejects_reversed_or_outside_intervals_and_unmapped_targets() {
        let target = AuthoringLaneOrdinal::from_raw(0);
        let (map, lane) = mapping(2, 1, false);
        for stations in [[1_000_000, 0], [0, 2_000_001], [u64::MAX; 2]] {
            assert_eq!(
                map.in_station_range(lane.corridor, stations, target),
                StationQuery::Outside
            );
        }
        for (corridor, target) in [
            (lane.corridor, AuthoringLaneOrdinal::from_raw(1)),
            (RoadCorridorOrdinal::from_raw(1), target),
        ] {
            assert_eq!(
                map.in_station_range(corridor, [0, 1], target),
                StationQuery::Unavailable
            );
        }
    }

    #[test]
    fn facility_band_query_encloses_both_slopes_and_rejects_wrong_domains() {
        let (mut map, lane) = mapping(1, 1, false);
        let corridor = lane.corridor;
        let other = RoadCorridorOrdinal::from_raw(1);
        let band = FacilityBandOrdinal::from_raw(8);
        let retained = map.retained_logical_bytes();
        map.domains = vec![
            Some(StationDomain {
                origin_nm: 50,
                length_nm: 3,
                origin_error_nm: 1,
            });
            2
        ]
        .into();
        map.bands = vec![Band {
            ordinal: band,
            corridor,
            left_um: [2, 0],
            right_um: [-2, 0],
            error_nm: 7,
        }]
        .into();
        assert_eq!(
            map.retained_logical_bytes() - retained,
            (size_of::<Option<StationDomain>>() + size_of::<Band>()) as u64
        );
        assert_eq!(
            map.facility_band_bounds(corridor, band, [1, 1]),
            Some(FacilityBandBounds {
                left_nm: [1_326, 1_341],
                right_nm: [-1_341, -1_326],
            })
        );
        assert_eq!(
            map.facility_band_bounds(corridor, band, [1, 2]),
            Some(FacilityBandBounds {
                left_nm: [659, 1_341],
                right_nm: [-1_341, -659],
            })
        );
        assert_eq!(
            map.facility_band_bounds(corridor, band, [3, 3]),
            Some(FacilityBandBounds {
                left_nm: [-7, 7],
                right_nm: [-7, 7],
            })
        );
        for (owner, id, stations) in [
            (other, band, [0, 0]),
            (corridor, FacilityBandOrdinal::from_raw(9), [0, 0]),
            (corridor, band, [2, 1]),
            (corridor, band, [3, 4]),
        ] {
            assert_eq!(map.facility_band_bounds(owner, id, stations), None);
        }
    }

    #[test]
    fn facility_band_interpolation_preserves_sub_nanometre_extrema_on_the_full_integer_domain() {
        let (mut map, lane) = mapping(1, 1, false);
        let corridor = lane.corridor;
        let band = FacilityBandOrdinal::from_raw(0);
        map.domains[0].as_mut().unwrap().length_nm = u64::MAX;
        map.bands = vec![Band {
            ordinal: band,
            corridor,
            left_um: [i32::MAX, i32::MIN],
            right_um: [i32::MIN; 2],
            error_nm: 0,
        }]
        .into();
        let mid = u64::MAX / 2;
        for (s, expected) in [(mid, [-500, -499]), (mid + 1, [-501, -500])] {
            let bounds = map.facility_band_bounds(corridor, band, [s, s]).unwrap();
            assert_eq!(bounds.left_nm, expected);
            assert_eq!(bounds.right_nm, [i64::from(i32::MIN) * 1_000; 2]);
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
