//! 已绑定出现对的几何部分：沿候选车体的前保险杠中点证明唯一横向交接。
use super::interval::Interval as I;
use super::path::{Motion, Piece};
use super::surface::dot;
use super::*;
use crate::{BoundaryOwner, CorridorElement, LaneFootprint, StationAddress, StationQuery};
use laneflow_static_contract::LaneEdgeOrdinal;

/// 两个相邻路线出现所声明的交接边界和闭窗口。边的顺序为源、目标。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HandoffWindow {
    /// 当前出现、下一出现实际使用的边。
    pub edges: [LaneEdgeOrdinal; 2],
    /// 结构边界所属走廊或区段。
    pub owner: BoundaryOwner,
    /// 在所有者的有序成员表中位于边界左侧的成员下标。
    pub left_index: u32,
    /// 允许前保险杠交接的走廊站位闭区间，单位纳米。
    pub stations_nm: [u64; 2],
}

/// 在两条已发布车道的中心之间生成五次横移。运行时负责车型与交通通行权的绑定。
#[derive(Clone, Copy, Debug)]
pub struct HandoffQuery {
    /// 边、边界和坐标框架所属的修订。
    pub revision: NetworkRevisionId,
    /// 所属路层的走廊。
    pub corridor: RoadCorridorOrdinal,
    /// 该修订发布的坐标框架。
    pub frame: CanonicalFrameOrdinal,
    /// 允许求高与车身投影的走廊站位闭区间，单位纳米。
    pub station_window_nm: [u64; 2],
    /// 两轴车型。
    pub axles: AxleDimensions,
    /// 后轴起、止站位，单位纳米，顺序为行驶方向。
    pub rear_stations_nm: [u64; 2],
}

/// 一次候选的静态几何交接证据。两侧交通预约由运行时另行裁决。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandoffOutcome {
    /// 全段几何可行，唯一过缝截面完整落入允许窗口及指定的两条边。
    /// 此证据只定位一次横向穿越；沿轨迹的三维行程、前杠纵向推进与各拍预约须分别求解。
    Located {
        /// 前保险杠中点过缝时的走廊站位闭区间，单位纳米。
        stations_nm: [u64; 2],
        /// 从起点沿行驶方向量取的后轴站位差闭区间，单位纳米。
        trace_range_nm: [u64; 2],
        /// 源、目标边上对应的保守局部进度范围。
        addresses: [StationAddress; 2],
    },
    /// 已证明唯一交接截面与允许窗口分离。
    OutsideWindow {
        /// 前保险杠过缝的走廊站位闭区间，单位纳米。
        stations_nm: [u64; 2],
    },
    /// 完整轮迹、转向、静态跨越规则或净空未能全部通过。具体结果见几何报告。
    GeometryRejected,
    /// 绑定、唯一过缝、窗口包含或两条边的唯一归属未能证明。
    Unproven {
        /// 受影响的候选参数区间，单位纳米。
        range_nm: [u64; 2],
        /// 无法完成查询的原因。
        reason: Failure,
    },
}

/// 完整几何检查与前保险杠交接共享同一条候选及其工作预算。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HandoffReport<'a> {
    /// 成功绑定路幅和有向边界后得到的完整几何报告。
    pub geometry: Option<GeometryReport<'a>>,
    /// 该候选的交接结果。
    pub outcome: HandoffOutcome,
    /// 绑定、过缝证明及两侧投影所用的附加工作，计入整条候选的扫过预算。
    pub work: u32,
}

struct Crossing {
    line: sweep::BoundaryLine,
    // 使源侧残差为负，目标侧为正。
    sign: I,
}

fn lane_pair(
    network: &SharedNetworkRevision,
    corridor: RoadCorridorOrdinal,
    window: HandoffWindow,
    work: &mut Work,
) -> Result<[AuthoringLaneOrdinal; 2], Failure> {
    let r = network.traffic().relations();
    let i = window.left_index as usize;
    let j = i.checked_add(1).ok_or(Failure::Binding)?;
    work.charge()?;
    match window.owner {
        BoundaryOwner::Section(s) => {
            if r.section_corridor(s) != Some(corridor) {
                return Err(Failure::Binding);
            }
            work.charge()?;
            let lanes = r.section_lanes(s).ok_or(Failure::Binding)?;
            Ok([
                *lanes.get(i).ok_or(Failure::Binding)?,
                *lanes.get(j).ok_or(Failure::Binding)?,
            ])
        }
        BoundaryOwner::Corridor(c) if c == corridor => {
            let elems = r.corridor_elements(c).ok_or(Failure::Binding)?;
            let (
                Some(CorridorElement::RoadSection(left)),
                Some(CorridorElement::RoadSection(right)),
            ) = (elems.get(i), elems.get(j))
            else {
                return Err(Failure::Binding);
            };
            work.charge()?;
            let a = r.section_lanes(*left).and_then(|v| v.last());
            work.charge()?;
            let b = r.section_lanes(*right).and_then(|v| v.first());
            Ok([*a.ok_or(Failure::Binding)?, *b.ok_or(Failure::Binding)?])
        }
        _ => Err(Failure::Binding),
    }
}

struct Bound {
    lanes: [AuthoringLaneOrdinal; 2],
    source_target: [AuthoringLaneOrdinal; 2],
    crossing: Crossing,
    trace: CorridorTrace,
}

fn bind(
    network: &SharedNetworkRevision,
    q: &HandoffQuery,
    w: HandoffWindow,
    work: &mut Work,
) -> Result<Bound, Failure> {
    if network.network_revision() != q.revision {
        return Err(Failure::Binding);
    }
    let m = network.station_mapping();
    work.charge()?;
    let length = m
        .corridor_domain(q.corridor)
        .ok_or(Failure::Unavailable)?
        .length_nm;
    if length == 0
        || w.stations_nm[0] > w.stations_nm[1]
        || w.stations_nm[1] > length
        || q.rear_stations_nm.iter().any(|s| *s > length)
        || q.rear_stations_nm[0] == q.rear_stations_nm[1]
    {
        return Err(Failure::InvalidInput);
    }
    let lanes = lane_pair(network, q.corridor, w, work)?;
    let mut source_target = lanes;
    let increasing = q.rear_stations_nm[0] < q.rear_stations_nm[1];
    for (lane, edge) in source_target.iter_mut().zip(w.edges) {
        work.charge()?;
        *lane = m
            .edges
            .get(edge.index())
            .copied()
            .flatten()
            .ok_or(Failure::Unavailable)?
            .lane;
        work.charge()?;
        let row = m.lanes[lane.index()].ok_or(Failure::Unavailable)?;
        if row.corridor != q.corridor || row.increasing != increasing {
            return Err(Failure::Binding);
        }
    }
    let to_right = source_target == lanes;
    if !to_right && source_target != [lanes[1], lanes[0]] {
        return Err(Failure::Binding);
    }
    let lookup = |lane, work: &mut Work| {
        let i = partition(&m.footprints, |r| r.lane < lane, work)?;
        work.charge()?;
        m.footprints
            .get(i)
            .filter(|r| r.lane == lane && r.supports_wheel)
            .ok_or(Failure::Unavailable)
    };
    let left = lookup(lanes[0], work)?;
    let right = lookup(lanes[1], work)?;
    let centers = |f: &LaneFootprint| {
        let [low, high] = f.ends_um.map(|[l, r]| 0.5 * (f64::from(l) + f64::from(r)));
        q.rear_stations_nm
            .map(|s| (low + (high - low) * (s as f64 / length as f64)) / 1e6)
    };
    let ordered = if to_right {
        [left, right]
    } else {
        [right, left]
    };
    let [from_m, to_m] = ordered.map(centers);
    // 五次横移在两端的一、二阶权重导数均为零，端点处沿原车道的偏移线接续。
    // 恰在发布行端点时仍须比较行的另一侧；候选内部的转向检查不包含它。
    let begin = partition(&m.surface, |r| r.corridor < q.corridor, work)?;
    let count = partition(&m.surface[begin..], |r| r.corridor == q.corridor, work)?;
    let rows = &m.surface[begin..begin + count];
    for (station, lane) in q.rear_stations_nm.into_iter().zip(ordered) {
        if station == 0 || station == length {
            return Err(Failure::Unavailable);
        }
        let index = partition(rows, |r| r.stations[1].0 < station, work)?;
        work.charge()?;
        let row = rows.get(index).ok_or(Failure::Unavailable)?;
        if row.stations[1].0 == station {
            work.charge()?;
            let next = rows.get(index + 1).ok_or(Failure::Unavailable)?;
            let [a, b] = lane.ends_um.map(|[l, r]| i64::from(l) + i64::from(r));
            if !path::smooth_join(row, next, a == b) {
                return Err(Failure::SteeringSeam);
            }
        }
    }
    Ok(Bound {
        lanes,
        source_target,
        crossing: Crossing {
            line: sweep::BoundaryLine {
                offsets_um: left.ends_um.map(|ends| ends[1]),
                error_nm: left.error_nm,
            },
            sign: I::point(if to_right { -1. } else { 1. }),
        },
        trace: CorridorTrace {
            stations_nm: q.rear_stations_nm,
            from_m,
            to_m,
        },
    })
}

/// 前杠在有向边界上的残差、导数及截面。从同一参考姿态取点及速度。
struct Sample {
    value: I,
    rate: I,
    station: I,
}
struct RootSearch<'a, 'q> {
    candidate: Candidate<'a, 'q>,
    boundary: Crossing,
    front: I,
    roots: Option<([u64; 2], I)>,
}
type Gap = ([u64; 2], Failure);

// 对二进制浮点米数先向外乘十亿，再向外取整。几何和 Newton 结果均用现有域裁剪。
fn nanometres(v: I) -> Result<[u64; 2], Failure> {
    let n = v * I::point(1e9);
    if !n.finite() || n.hi < 0. || n.lo >= u64::MAX as f64 {
        return Err(Failure::Projection);
    }
    Ok([n.lo.floor() as u64, n.hi.ceil() as u64])
}
impl RootSearch<'_, '_> {
    fn sample(&self, m: &Motion, work: &mut Work) -> Result<Sample, Failure> {
        let c = &self.candidate;
        let (h, dh) = m.heading()?;
        let pose = Pose::enclose(&c.surface, m.p, h, c.query.axles, work)?;
        let projected = c.surface.project(pose.wheel(self.front.lo, 0.), work)?;
        let station = projected.station;
        let seam = self.boundary.line.lateral(c, station);
        let slope = self.boundary.line.slope(c);
        let value = self.boundary.sign * (projected.lateral - seam);
        let mut rate = I::ZERO;
        if value.lo <= 0. && value.hi >= 0. {
            let r = pose.rates(c.query.axles, m.v, h, dh)?;
            let velocity = [0, 2].map(|i| r.rear[i] + self.front * r.forward[i]);
            let gradient = projected.boundary_gradient(slope);
            rate = self.boundary.sign * dot(velocity, gradient);
        }
        Ok(Sample {
            value,
            rate,
            station,
        })
    }
    fn at(&self, range: [u64; 2], work: &mut Work) -> Result<Sample, Failure> {
        work.charge()?;
        let Piece::Motion(m) = self
            .candidate
            .trace
            .piece(&self.candidate.surface, range, work)?
        else {
            return Err(Failure::Projection);
        };
        self.sample(&m, work)
    }
    fn collect(&mut self, range: [u64; 2], depth: u8, work: &mut Work) -> Result<(), Gap> {
        work.charge().map_err(|e| (range, e))?;
        let piece = self
            .candidate
            .trace
            .piece(&self.candidate.surface, range, work)
            .map_err(|e| (range, e))?;
        let sample = match piece {
            Piece::Join { at, .. } => {
                self.collect([range[0], at], depth, work)?;
                return self.collect([at, range[1]], depth, work);
            }
            Piece::Motion(m) => self.sample(&m, work),
        };
        match sample {
            Ok(s) if s.value.lo > 0. || s.value.hi < 0. => return Ok(()),
            Ok(s) if s.rate.lo > 0. => {
                return self.contract(range, s, work).map_err(|e| (range, e));
            }
            Err(e) if !matches!(e, Failure::Pose | Failure::Projection) => return Err((range, e)),
            _ => {}
        }
        if depth == 20 || range[1] - range[0] <= 1 {
            return Err((range, Failure::Boundary));
        }
        let mid = range[0] + (range[1] - range[0]) / 2;
        self.collect([range[0], mid], depth + 1, work)?;
        self.collect([mid, range[1]], depth + 1, work)
    }
    fn contract(
        &mut self,
        mut range: [u64; 2],
        mut s: Sample,
        work: &mut Work,
    ) -> Result<(), Failure> {
        // 已证明本区间内所有可能的根均沿源→目标穿越。Newton 只作外包收缩，
        // 对不含根的区间允许返回空；存在性由整条候选首尾的严格异号证明。
        let mut rate = s.rate;
        loop {
            work.charge()?;
            let mid = range[0] + (range[1] - range[0]) / 2;
            let point = self.at([mid, mid], work)?;
            let estimate = I::meters(mid) - point.value / rate;
            let domain = I::meters(range[0]).hull(I::meters(range[1]));
            let Some(root) = estimate.intersect(domain) else {
                return Ok(());
            };
            let bounds = nanometres(root)?;
            let next = [range[0].max(bounds[0]), range[1].min(bounds[1])];
            if next[0] > next[1] {
                return Ok(());
            }
            if next == range {
                break;
            }
            range = next;
            s = self.at(range, work)?;
            if s.value.lo > 0. || s.value.hi < 0. {
                return Ok(());
            }
            rate = rate.intersect(s.rate).ok_or(Failure::Boundary)?;
        }
        self.roots = Some(self.roots.map_or((range, s.station), |(old, station)| {
            (
                [old[0].min(range[0]), old[1].max(range[1])],
                station.hull(s.station),
            )
        }));
        Ok(())
    }
    fn locate(mut self, length: u64, work: &mut Work) -> Result<([u64; 2], [u64; 2]), Gap> {
        let full = [0, length];
        let start = self.at([0, 0], work).map_err(|e| ([0, 0], e))?;
        let end = self
            .at([length, length], work)
            .map_err(|e| ([length, length], e))?;
        if start.value.hi >= 0. || end.value.lo <= 0. {
            return Err((full, Failure::Boundary));
        }
        self.collect(full, 0, work)?;
        let (range, station) = self.roots.ok_or((full, Failure::Boundary))?;
        Ok((range, nanometres(station).map_err(|e| (range, e))?))
    }
}

impl SharedNetworkRevision {
    /// 在同一候选上核对几何许可、唯一前杠交接、窗口与两侧边地址。
    /// 轨迹的起止偏移直接来自指定两条车道在发布路幅内的中心。只在所有可能过零
    /// 位置都已证明沿声明方向严格穿越时给出交接。各侧站位映射各受 64 段、8 条边上限。
    #[must_use]
    pub fn vehicle_handoff(
        &self,
        q: &HandoffQuery,
        steering: SteeringLimits,
        body: BodyEnvelope,
        limits: GeometryWorkLimits,
        window: HandoffWindow,
    ) -> HandoffReport<'_> {
        let mut work = Work::new(limits.sweep.min(SWEEP_WORK_LIMIT));
        let mut geometry = None;
        let full = [0, q.rear_stations_nm[0].abs_diff(q.rear_stations_nm[1])];
        let mut run = || -> Result<HandoffOutcome, Gap> {
            if body.half_width_m < q.axles.half_track_m {
                return Err((full, Failure::InvalidInput));
            }
            let bound = bind(self, q, window, &mut work).map_err(|e| (full, e))?;
            let query = TrackQuery {
                revision: q.revision,
                corridor: q.corridor,
                frame: q.frame,
                station_window_nm: q.station_window_nm,
                axles: q.axles,
                traversable_lanes: &bound.lanes,
                trace: RearAxleTrace::Corridor(bound.trace),
            };
            let report = self.vehicle_geometry(
                &query,
                steering,
                body,
                GeometryWorkLimits {
                    tracks: limits.tracks,
                    sweep: work.limit - work.used,
                },
            );
            work.limit -= report.sweep.work;
            geometry = Some(report);
            if !report.feasible() {
                return Ok(HandoffOutcome::GeometryRejected);
            }
            let search = RootSearch {
                candidate: Candidate::bind(self, &query, &mut work).map_err(|e| (full, e))?,
                front: I::point(body.front_from_rear_m),
                boundary: bound.crossing,
                roots: None,
            };
            let (trace_range_nm, stations_nm) = search.locate(full[1], &mut work)?;
            let [lo, hi] = window.stations_nm;
            if stations_nm[0] > hi || stations_nm[1] < lo {
                return Ok(HandoffOutcome::OutsideWindow { stations_nm });
            }
            if stations_nm[0] < lo || stations_nm[1] > hi {
                return Err((trace_range_nm, Failure::Boundary));
            }
            let mut address = |lane, expected| {
                work.charge().map_err(|e| (trace_range_nm, e))?;
                let StationQuery::Complete(hits) =
                    self.station_mapping()
                        .in_station_range(q.corridor, stations_nm, lane)
                else {
                    return Err((trace_range_nm, Failure::Projection));
                };
                let mut all = hits.addresses();
                match (all.next(), all.next()) {
                    (Some(a), None) if a.edge == expected => Ok(a),
                    _ => Err((trace_range_nm, Failure::Binding)),
                }
            };
            let [source, target] = bound.source_target;
            let addresses = [
                address(source, window.edges[0])?,
                address(target, window.edges[1])?,
            ];
            Ok(HandoffOutcome::Located {
                stations_nm,
                trace_range_nm,
                addresses,
            })
        };
        let outcome = run()
            .unwrap_or_else(|(range_nm, reason)| HandoffOutcome::Unproven { range_nm, reason });
        HandoffReport {
            geometry,
            outcome,
            work: work.used,
        }
    }
}
