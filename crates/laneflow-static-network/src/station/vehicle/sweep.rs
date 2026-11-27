use super::interval::Interval as I;
use super::path::{Motion, Piece};
use super::pose::{Pose, Vector};
use super::surface::Horizontal;
use super::{
    Candidate, Failure, SWEEP_WORK_LIMIT, SteeringLimits, TrackQuery, TrackReport, Work, partition,
};
use crate::{
    BoundaryExtent, BoundaryOwner, BoundaryPayload, CorridorElement, SharedNetworkRevision,
};

mod obstacle;
mod rule;
mod side_clearance;
pub(super) use rule::{BodyBoundary, window_permission};
pub(super) use side_clearance::side_clearance_segments;
pub use side_clearance::{SideClearanceReport, SideClearanceSegment};

/// 后轴坐标系中沿前、左、上三个方向的实体包络，单位米。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyEnvelope {
    /// 车尾至后轴、后轴至车头，均为非负距离。
    pub rear_overhang_m: f64,
    /// 后轴至车头的距离，至少包含轴距。
    pub front_from_rear_m: f64,
    /// 车身半宽。
    pub half_width_m: f64,
    /// 相对后轴参考点的底高、顶高。
    pub heights_m: [f64; 2],
}
impl BodyEnvelope {
    fn extents(self) -> [[f64; 2]; 3] {
        [
            [-self.rear_overhang_m, self.front_from_rear_m],
            [-self.half_width_m, self.half_width_m],
            self.heights_m,
        ]
    }
    pub(super) fn ranges(self) -> Vector {
        self.extents().map(|[lo, hi]| I { lo, hi })
    }
    pub(super) fn valid(self, axle: f64) -> bool {
        self.extents().into_iter().flatten().all(f64::is_finite)
            && self.rear_overhang_m >= 0.
            && self.front_from_rear_m >= axle
            && self.half_width_m > 0.
            && self.heights_m[0] < self.heights_m[1]
    }
}

/// 本修订中一条有向结构边界，及其可选的命名规则或实体。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct BoundaryHit<'a> {
    /// 边界所属的走廊或区段。
    pub owner: BoundaryOwner,
    /// 所属有序成员表中的左成员。
    pub left_index: u32,
    /// 已发布的局部名。使用边界默认禁止时为空。
    pub key: Option<&'a str>,
}
/// 实体分离的保守净空，附产生最小下界的候选参数区间。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clearance<'a> {
    /// 实体距离的正下界（米）。整车准入查询使用三维距离，侧向查询使用水平距离。
    pub lower_m: f64,
    /// 对应实体的发布身份。
    pub boundary: BoundaryHit<'a>,
    /// 在此候选参数区间上得到该下界，单位纳米。
    pub range_nm: [u64; 2],
}
/// 实体范围闭合，跨越许可沿走廊站位半开。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SweepOutcome<'a> {
    /// 整条候选有跨越许可，实体净空为正。有实体时给出保守下界（米）和限制它的实体。
    Clear(Option<Clearance<'a>>),
    /// 同一候选位置上的车身点落入实体的水平及竖直内界。
    Contact {
        /// 发生接触的实体。
        boundary: BoundaryHit<'a>,
        /// 候选自起点量起的位置。
        at_nm: u64,
        /// 该点在车体前、左、上轴上的坐标，单位米。
        body_point_m: [f64; 3],
    },
    /// 整车与禁跨边界相交。
    Denied {
        /// 生效的规则或默认禁止的边界。
        boundary: BoundaryHit<'a>,
        /// 候选自起点量起的位置。
        at_nm: u64,
        /// 车身实际触线的走廊站位包络，单位米。
        touch_station_m: [f64; 2],
    },
    /// 仍可能影响判定的第一个区间。
    Unproven {
        /// 候选参数，单位纳米。
        range_nm: [u64; 2],
        /// 已定位的边界。
        boundary: Option<BoundaryHit<'a>>,
        /// 未证原因。
        reason: Failure,
    },
}
/// 整车实体与跨越判定、连续已证前缀及独立工作计数。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SweepReport<'a> {
    /// 有序扫过结果。
    pub outcome: SweepOutcome<'a>,
    /// 连续完成的安全前缀。
    pub covered_prefix_nm: u64,
    /// 已扣的工作项。
    pub work: u32,
    /// 预算耗尽时仍待检查的后缀。
    pub unchecked_nm: Option<[u64; 2]>,
}
/// 两项均通过时该车型的候选才满足静态几何准入。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeometryReport<'a> {
    /// 姿态、路幅和转向。
    pub tracks: TrackReport,
    /// 实体净空和静态跨越规则。
    pub sweep: SweepReport<'a>,
}
impl GeometryReport<'_> {
    /// 路幅、转向、跨越许可及净空均取得连续证明。
    #[must_use]
    pub fn feasible(&self) -> bool {
        matches!(self.tracks.outcome, super::TrackOutcome::Covered(_))
            && matches!(self.sweep.outcome, SweepOutcome::Clear(_))
    }
}
/// 一次候选查询的两项独立逻辑预算。较小的调用预算可用于逐级准入。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GeometryWorkLimits {
    /// 姿态、路幅与转向。
    pub tracks: u32,
    /// 规则与实体。
    pub sweep: u32,
}
impl GeometryWorkLimits {
    /// 生产查询的两项硬上限。
    pub const STANDARD: Self = Self {
        tracks: super::TRACK_WORK_LIMIT,
        sweep: SWEEP_WORK_LIMIT,
    };
}

#[derive(Clone, Copy)]
struct Seam<'a> {
    id: BoundaryHit<'a>,
    line: BoundaryLine,
    extents: &'a [BoundaryExtent],
}
#[derive(Clone, Copy)]
pub(super) struct BoundaryLine {
    pub offsets_um: [i32; 2],
    pub error_nm: u32,
}
impl BoundaryLine {
    pub(super) fn lateral(&self, c: &Candidate<'_, '_>, s: I) -> I {
        let [a, b] = self.offsets_um.map(f64::from);
        ((I::point(a) + I::point(b - a) * (s - c.origin) / c.length) / I::point(1e6))
            .with_error(I::meters(u64::from(self.error_nm)).hi)
    }
    pub(super) fn slope(&self, c: &Candidate<'_, '_>) -> I {
        let [a, b] = self.offsets_um.map(f64::from);
        let err = I::point(2.) * I::meters(u64::from(self.error_nm));
        ((I::point(b - a) / I::point(1e6)).with_error(err.hi)) / c.length
    }
}

fn boundaries<'a>(
    c: &Candidate<'a, '_>,
    work: &mut Work,
    mut visit: impl FnMut(Seam<'a>, &mut Work) -> Result<(), Failure>,
) -> Result<(), Failure> {
    let r = c.network.traffic().relations();
    let m = c.network.station_mapping();
    let mut seam = |owner, left_index, offsets_um, error_nm, work: &mut Work| {
        let key = (owner, left_index);
        let k = |r: &BoundaryExtent| (r.owner, r.left_index);
        let lo = partition(&m.extents, |r| k(r) < key, work)?;
        let count = partition(&m.extents[lo..], |r| k(r) == key, work)?;
        visit(
            Seam {
                id: BoundaryHit {
                    owner,
                    left_index,
                    key: None,
                },
                line: BoundaryLine {
                    offsets_um,
                    error_nm,
                },
                extents: &m.extents[lo..lo + count],
            },
            work,
        )
    };
    let lane_right = |lane, work: &mut Work| {
        let i = partition(&m.footprints, |r| r.lane < lane, work)?;
        work.charge()?;
        let f = m
            .footprints
            .get(i)
            .filter(|r| r.lane == lane)
            .ok_or(Failure::Unavailable)?;
        Ok((f.ends_um.map(|e| e[1]), f.error_nm))
    };
    let corridor = c.query.corridor;
    work.charge()?;
    let elements = r.corridor_elements(corridor).ok_or(Failure::Binding)?;
    for (i, &element) in elements
        .iter()
        .take(elements.len().saturating_sub(1))
        .enumerate()
    {
        work.charge()?;
        let (offset, error) = match element {
            CorridorElement::RoadSection(s) => {
                work.charge()?;
                let lane = *r
                    .section_lanes(s)
                    .and_then(|l| l.last())
                    .ok_or(Failure::Binding)?;
                lane_right(lane, work)?
            }
            CorridorElement::FacilityBand(b) => {
                let i = partition(&m.bands, |r| r.ordinal < b, work)?;
                work.charge()?;
                let band = m
                    .bands
                    .get(i)
                    .filter(|r| r.ordinal == b)
                    .ok_or(Failure::Unavailable)?;
                (band.right_um, band.error_nm)
            }
        };
        seam(
            BoundaryOwner::Corridor(corridor),
            i as u32,
            offset,
            error,
            work,
        )?;
    }
    for element in elements {
        work.charge()?;
        if let CorridorElement::RoadSection(s) = *element {
            work.charge()?;
            let lanes = r.section_lanes(s).ok_or(Failure::Binding)?;
            for (j, &lane) in lanes.iter().take(lanes.len().saturating_sub(1)).enumerate() {
                let (offset, error) = lane_right(lane, work)?;
                seam(BoundaryOwner::Section(s), j as u32, offset, error, work)?;
            }
        }
    }
    Ok(())
}

struct Body<'a> {
    pose: &'a Pose,
    motion: &'a Motion,
    shape: BodyEnvelope,
}
impl Body<'_> {
    fn world(&self) -> Vector {
        let [a, b, c] = self.shape.ranges();
        self.pose.point(a, b, c)
    }
    fn vertex(&self, q: [f64; 3]) -> Vector {
        let [a, b, c] = q.map(I::point);
        self.pose.point(a, b, c)
    }
    fn nearest_local(&self, target: [f64; 3]) -> [f64; 3] {
        let axes = [self.pose.forward, self.pose.left, self.pose.up].map(|v| v.map(mid));
        let limits = self.shape.extents();
        std::array::from_fn(|i| {
            (0..3)
                .map(|j| (target[j] - mid(self.pose.rear[j])) * axes[i][j])
                .sum::<f64>()
                .clamp(limits[i][0], limits[i][1])
        })
    }
}
fn horizontal(p: Vector) -> Horizontal {
    [p[0], p[2]]
}
fn mid(x: I) -> f64 {
    x.lo + (x.hi - x.lo) * 0.5
}
fn nearer<'a>(a: Option<Clearance<'a>>, b: Option<Clearance<'a>>) -> Option<Clearance<'a>> {
    match (a, b) {
        (Some(a), Some(b)) => Some(
            if a.lower_m < b.lower_m
                || (a.lower_m == b.lower_m && (a.range_nm, a.boundary) <= (b.range_nm, b.boundary))
            {
                a
            } else {
                b
            },
        ),
        _ => a.or(b),
    }
}
type Verdict<'a> = SweepOutcome<'a>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClearanceMetric {
    Spatial,
    Side,
}
fn witness_key(v: Verdict<'_>) -> Option<(u64, BoundaryHit<'_>)> {
    let (at, b) = match v {
        Verdict::Contact {
            at_nm, boundary, ..
        }
        | Verdict::Denied {
            at_nm, boundary, ..
        } => (at_nm, boundary),
        _ => return None,
    };
    Some((at, b))
}
fn combine<'a>(a: Verdict<'a>, b: Verdict<'a>) -> Verdict<'a> {
    use SweepOutcome::*;
    if let (Some(x), Some(y)) = (witness_key(a), witness_key(b)) {
        return if x <= y { a } else { b };
    }
    match (a, b) {
        (Clear(a), Clear(b)) => Clear(nearer(a, b)),
        (Contact { .. } | Denied { .. }, _) => a,
        (_, Contact { .. } | Denied { .. }) => b,
        (Unproven { .. }, _) => a,
        _ => b,
    }
}
struct Sweep<'a, 'q> {
    candidate: Candidate<'a, 'q>,
    shape: BodyEnvelope,
    result: Verdict<'a>,
    prefix: u64,
    unchecked: Option<[u64; 2]>,
    witness: Option<(u64, Verdict<'a>)>,
}
fn check_motion<'a>(
    c: &Candidate<'a, '_>,
    shape: BodyEnvelope,
    m: &Motion,
    range: [u64; 2],
    metric: ClearanceMetric,
    work: &mut Work,
) -> Result<Verdict<'a>, Failure> {
    let pose = c.pose(m, work)?;
    let body = Body {
        pose: &pose,
        motion: m,
        shape,
    };
    let mut result = Verdict::Clear(None);
    let complete = boundaries(c, work, |s, work| {
        let mut observe = |v: Result<Verdict<'a>, Failure>, boundary| -> Result<(), Failure> {
            let v = match v {
                Err(Failure::WorkLimit) => return Err(Failure::WorkLimit),
                Err(reason) => Verdict::Unproven {
                    range_nm: range,
                    boundary: Some(boundary),
                    reason,
                },
                Ok(v) => v,
            };
            result = combine(result, v);
            Ok(())
        };
        if metric == ClearanceMetric::Spatial {
            observe(rule::check(c, &body, s, range, work), s.id)?;
        }
        for e in s.extents {
            work.charge()?;
            if let Some(payload) = e.payload
                && !matches!(payload, BoundaryPayload::Rule { .. })
            {
                observe(
                    obstacle::check(c, &body, s, e, range, metric, work),
                    BoundaryHit {
                        key: Some(e.local_key()),
                        ..s.id
                    },
                )?;
            }
        }
        Ok(())
    });
    if witness_key(result).is_none() {
        complete?;
    }
    Ok(result)
}
impl<'a> Sweep<'a, '_> {
    fn check(&self, m: &Motion, range: [u64; 2], work: &mut Work) -> Result<Verdict<'a>, Failure> {
        check_motion(
            &self.candidate,
            self.shape,
            m,
            range,
            ClearanceMetric::Spatial,
            work,
        )
    }
    fn visit(&mut self, range: [u64; 2], depth: u8, work: &mut Work) {
        if self.witness.is_some_and(|(at, _)| range[0] >= at) {
            return;
        }
        let result = work.charge().and_then(|()| {
            self.candidate
                .trace
                .piece(&self.candidate.surface, range, work)
        });
        let mut split = None;
        let mut result = match result {
            Ok(Piece::Join { at, .. }) => {
                split = Some((at, depth));
                Ok(Verdict::Clear(None))
            }
            Ok(Piece::Motion(m)) => self.check(&m, range, work),
            Err(e) => Err(e),
        }
        .unwrap_or_else(|reason| Verdict::Unproven {
            range_nm: range,
            boundary: None,
            reason,
        });
        if work.exhausted {
            self.unchecked = Some([range[0], self.candidate.query.trace.length()]);
        }
        if let Verdict::Unproven { reason, .. } = result {
            if reason == Failure::WorkLimit {
                self.unchecked = Some([range[0], self.candidate.query.trace.length()]);
            }
            if matches!(
                reason,
                Failure::Boundary | Failure::Pose | Failure::Projection
            ) && depth < 20
                && range[1] - range[0] > 1
            {
                let mid = range[0] + (range[1] - range[0]) / 2;
                split = Some((mid, depth + 1));
                if reason == Failure::Boundary {
                    // 保留右侧已经证实的具体见证，即使左侧触碰起始位置落在输入误差带内。
                    // 单点也无法消除的实体/规则不确定性，不会因继续二分该点而消失。
                    for at in [range[0], mid, range[1]] {
                        if self.witness.is_some_and(|(old, _)| at >= old) {
                            continue;
                        }
                        let probe = work
                            .charge()
                            .and_then(|()| {
                                self.candidate
                                    .trace
                                    .piece(&self.candidate.surface, [at, at], work)
                            })
                            .and_then(|piece| match piece {
                                Piece::Motion(m) => self.check(&m, [at, at], work),
                                Piece::Join { .. } => Err(Failure::Binding),
                            });
                        match probe {
                            Ok(found @ (Verdict::Contact { .. } | Verdict::Denied { .. })) => {
                                self.witness = Some((at, found));
                                if at == range[0] {
                                    result = found;
                                    split = None;
                                    break;
                                }
                            }
                            Ok(Verdict::Unproven {
                                boundary: Some(_),
                                reason: Failure::Boundary,
                                ..
                            }) if at == mid => {
                                split = None;
                            }
                            Err(Failure::WorkLimit) => {
                                self.unchecked =
                                    Some([range[0], self.candidate.query.trace.length()]);
                                split = None;
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        if let Some((mid, depth)) = split {
            self.visit([range[0], mid], depth, work);
            if !work.exhausted
                && !matches!(
                    self.result,
                    Verdict::Contact { .. } | Verdict::Denied { .. }
                )
            {
                self.visit([mid, range[1]], depth, work);
            }
        } else {
            self.result = combine(self.result, result);
            if matches!(self.result, Verdict::Clear(_)) {
                self.prefix = range[1];
            }
        }
    }
}
impl SharedNetworkRevision {
    /// 从同一修订、车型和候选计算四轮、转向、跨越规则和整车实体净空。
    /// 两项工作各自计数，超出标准上限的请求取标准上限。
    #[must_use]
    pub fn vehicle_geometry(
        &self,
        query: &TrackQuery<'_>,
        steering: SteeringLimits,
        body: BodyEnvelope,
        budgets: GeometryWorkLimits,
    ) -> GeometryReport<'_> {
        let tracks = self.wheel_tracks(query, steering, budgets.tracks);
        let range = [0, query.trace.length()];
        let mut work = Work::new(budgets.sweep.min(SWEEP_WORK_LIMIT));
        let bound = if body.valid(query.axles.wheelbase_m) {
            Candidate::bind(self, query, &mut work)
        } else {
            Err(Failure::InvalidInput)
        };
        let (outcome, prefix, unchecked) = match bound {
            Ok(candidate) => {
                let mut sweep = Sweep {
                    candidate,
                    shape: body,
                    result: Verdict::Clear(None),
                    prefix: 0,
                    unchecked: None,
                    witness: None,
                };
                sweep.visit(range, 0, &mut work);
                (
                    sweep
                        .witness
                        .map_or(sweep.result, |(_, w)| combine(sweep.result, w)),
                    sweep.prefix,
                    sweep.unchecked,
                )
            }
            Err(reason) => (
                Verdict::Unproven {
                    range_nm: range,
                    boundary: None,
                    reason,
                },
                0,
                work.exhausted.then_some(range),
            ),
        };
        GeometryReport {
            tracks,
            sweep: SweepReport {
                outcome,
                covered_prefix_nm: prefix,
                work: work.used,
                unchecked_nm: unchecked,
            },
        }
    }
}
