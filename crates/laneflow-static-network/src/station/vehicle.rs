//! 同一修订、同层路面的车体参考姿态和连续轮迹查询。
use laneflow_static_contract::{
    AuthoringLaneOrdinal, CanonicalFrameOrdinal, NetworkRevisionId, RoadCorridorOrdinal,
};

use crate::SharedNetworkRevision;
use footprint::Coverage;
use interval::Interval;
pub use path::{CorridorTrace, RearAxleTrace};
use path::{Motion, Path, Piece};
use pose::Pose;
pub use steering::{SteeringConstraint, SteeringLimits};
use surface::{Projection, Surface};

mod footprint;
mod handoff;
mod interval;
mod path;
mod pose;
mod steering;
mod surface;
mod sweep;
pub use handoff::{
    HandoffBinding, HandoffBodyReport, HandoffBodySide, HandoffFootprint, HandoffFootprintReport,
    HandoffLaneSide, HandoffOutcome, HandoffQuery, HandoffReport, HandoffSideFootprint,
    HandoffSourceEvidence, HandoffSourceModule, HandoffTravelBounds, HandoffTravelReport,
    HandoffTravelTarget, HandoffWindow,
};
pub use sweep::{
    BodyEnvelope, BoundaryHit, Clearance, GeometryReport, GeometryWorkLimits, SweepOutcome,
    SweepReport,
};

/// 一条候选的姿态、轮迹与转向共享的逻辑工作上限。
pub const TRACK_WORK_LIMIT: u32 = 4_096;
/// 整车扫过和规则查询每条候选的逻辑工作上限。
pub const SWEEP_WORK_LIMIT: u32 = 16_384;

/// 两轴车型在道路参考姿态中的尺寸。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxleDimensions {
    /// 前后两轴中点的三维距离，单位米。
    pub wheelbase_m: f64,
    /// 半轮距，单位米。
    pub half_track_m: f64,
    /// 两轴中点相对路面采样平均值的竖直高度，单位米。
    pub reference_height_m: f64,
}

/// 在世界水平面内直行的后轴中点。可传入零长度以检查一个姿态。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RearAxleLine {
    /// 起点的规范 x、z 坐标，单位米。
    pub start_xz_m: [f64; 2],
    /// 沿行驶方向的非零水平向量，由查询归一化。
    pub heading_xz: [f64; 2],
    /// 后轴沿该水平直线行进的长度，单位纳米。
    pub length_nm: u64,
}

/// 单条候选的绑定、车型和路幅输入。
#[derive(Clone, Copy, Debug)]
pub struct TrackQuery<'a> {
    /// 所有序号和世界位置所属的修订。
    pub revision: NetworkRevisionId,
    /// 车身所在路层的走廊。
    pub corridor: RoadCorridorOrdinal,
    /// 候选的规范坐标框架。
    pub frame: CanonicalFrameOrdinal,
    /// 允许求高和投影的走廊局部站位窗口，递增，单位纳米。
    pub station_window_nm: [u64; 2],
    /// 调用方根据车型和通行权选定的车道，按走廊结构从左到右排列。
    pub traversable_lanes: &'a [AuthoringLaneOrdinal],
    /// 刚性两轴车型。
    pub axles: AxleDimensions,
    /// 后轴水平轨迹。
    pub trace: RearAxleTrace,
}

/// 覆盖整个所查行程的车体参考姿态。每个 x、y、z 分量保存闭区间。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PoseBounds {
    /// 后轴参考点，单位米。
    pub rear_m: [[f64; 2]; 3],
    /// 从后轴指向前轴的三维单位向量。
    pub forward: [[f64; 2]; 3],
    /// 车体左向单位向量。
    pub left: [[f64; 2]; 3],
    /// 车体上向单位向量。
    pub up: [[f64; 2]; 3],
}
impl PoseBounds {
    fn hull(self, other: Self) -> Self {
        let join = |a: [[f64; 2]; 3], b: [[f64; 2]; 3]| {
            std::array::from_fn(|i| [a[i][0].min(b[i][0]), a[i][1].max(b[i][1])])
        };
        Self {
            rear_m: join(self.rear_m, other.rear_m),
            forward: join(self.forward, other.forward),
            left: join(self.left, other.left),
            up: join(self.up, other.up),
        }
    }
}

/// 首个有定位证据的车轮，同站位按前左、前右、后左、后右检查。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Wheel {
    /// 前左。
    FrontLeft,
    /// 前右。
    FrontRight,
    /// 后左。
    RearLeft,
    /// 后右。
    RearRight,
}
const WHEELS: [Wheel; 4] = [
    Wheel::FrontLeft,
    Wheel::FrontRight,
    Wheel::RearLeft,
    Wheel::RearRight,
];

/// 轮迹检查没有取得连续覆盖证据的原因。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Failure {
    /// 非有限坐标、零朝向、非法车型尺寸或纵向窗口。
    InvalidInput,
    /// 修订、走廊、坐标框架或车道序列错配。
    Binding,
    /// 走廊没有发布查询所需的完整路面或路幅。
    Unavailable,
    /// 给定窗口不足以证明同层截面的投影唯一且位于域内。
    Projection,
    /// 轴距前向根或车体方向缺少有界证据。
    Pose,
    /// 指定车轮的误差范围跨越可用路幅的外缘。
    Footprint(Wheel),
    /// 当前位置的曲率或转角速率包络跨越车型限值。
    Steering,
    /// 接缝两侧的规范轨迹没有共同转向的证明。
    SteeringSeam,
    /// 车身对实体或跨越规则的误差包络仍跨越判定边界。
    Boundary,
    /// 完成下一项检查会超过整条候选的逻辑预算。
    WorkLimit,
    /// 发布的误差或细分分辨率不足以在请求精度内确定行程或站位。
    TravelResolution,
}

/// 路幅检查结果。禁跨规则和实体净空另行裁决。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrackOutcome {
    /// 四条轮迹在整个行程内得到覆盖，并给出共同姿态包络。
    Covered(PoseBounds),
    /// 车型无法按给定车速连续通过。
    Infeasible {
        /// 前轮。
        wheel: Wheel,
        /// 违反的车型条件。
        constraint: SteeringConstraint,
        /// 沿候选参数自起点量起的见证位置，单位纳米。
        at_nm: u64,
    },
    /// 指定部位在给定后轴水平行程处确定越出可用路幅。
    Outside {
        /// 与可用路幅并集的横向距离下界（米）。该并集为空时返回空。
        overrun_lower_m: Option<f64>,
        /// 车轮。
        wheel: Wheel,
        /// 从候选起点量起的见证行程，单位纳米。
        at_nm: u64,
    },
    /// 缺少有效绑定、输入或足以判定的证据。
    Unproven {
        /// 首个未证后轴水平行程区间，单位纳米。
        range_nm: [u64; 2],
        /// 停止原因。
        reason: Failure,
    },
}

/// 一次完整调用的连续证明及资源用量。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackReport {
    /// 所查候选的裁决。
    pub outcome: TrackOutcome,
    /// 从起点开始已经连续证明覆盖的水平行程，单位纳米。
    pub covered_prefix_nm: u64,
    /// 从索引入口开始的逻辑工作数。
    pub work: u32,
    /// 因预算耗尽而未能完成检查的行程后缀。
    pub unchecked_nm: Option<[u64; 2]>,
}

struct Work {
    used: u32,
    limit: u32,
    exhausted: bool,
}
impl Work {
    fn new(limit: u32) -> Self {
        Self {
            used: 0,
            limit,
            exhausted: false,
        }
    }
    fn charge(&mut self) -> Result<(), Failure> {
        if self.used == self.limit {
            self.exhausted = true;
            return Err(Failure::WorkLimit);
        }
        self.used += 1;
        Ok(())
    }
}
fn partition<T>(
    rows: &[T],
    before: impl Fn(&T) -> bool,
    work: &mut Work,
) -> Result<usize, Failure> {
    let (mut lo, mut hi) = (0, rows.len());
    while lo < hi {
        work.charge()?;
        let mid = lo + (hi - lo) / 2;
        if before(&rows[mid]) {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    Ok(lo)
}

struct Candidate<'a, 'q> {
    network: &'a SharedNetworkRevision,
    query: &'q TrackQuery<'q>,
    surface: Surface<'a>,
    all_surface: &'a [crate::RoadSurfaceSpan],
    trace: Path,
    origin: Interval,
    length: Interval,
    covered: Option<PoseBounds>,
    prefix: u64,
    first_gap: Option<([u64; 2], Failure)>,
    unchecked: Option<[u64; 2]>,
}
impl<'a, 'q> Candidate<'a, 'q> {
    fn bind(
        network: &'a SharedNetworkRevision,
        query: &'q TrackQuery<'q>,
        work: &mut Work,
    ) -> Result<Self, Failure> {
        if network.network_revision() != query.revision {
            return Err(Failure::Binding);
        }
        let axle = query.axles;
        let window = query.station_window_nm;
        if !axle.wheelbase_m.is_finite()
            || axle.wheelbase_m <= 0.
            || !axle.half_track_m.is_finite()
            || axle.half_track_m <= 0.
            || !axle.reference_height_m.is_finite()
            || window[0] >= window[1]
        {
            return Err(Failure::InvalidInput);
        }
        let trace = Path::bind(query.trace, window)?;
        let mapping = network.station_mapping();
        work.charge()?;
        let domain = mapping
            .corridor_domain(query.corridor)
            .ok_or(Failure::Unavailable)?;
        if window[1] > domain.length_nm {
            return Err(Failure::InvalidInput);
        }
        let first = partition(&mapping.surface, |r| r.corridor < query.corridor, work)?;
        let end = partition(
            &mapping.surface[first..],
            |r| r.corridor == query.corridor,
            work,
        )?;
        let rows = &mapping.surface[first..first + end];
        work.charge()?;
        let low = rows.first().ok_or(Failure::Unavailable)?;
        if low.frame != query.frame {
            return Err(Failure::Binding);
        }
        work.charge()?;
        let high = rows.last().ok_or(Failure::Unavailable)?;
        let origin = Interval::ZERO.with_error(Interval::meters(u64::from(low.stations[0].1)).hi);
        let end = Interval::meters(domain.length_nm)
            .with_error(Interval::meters(u64::from(high.stations[1].1)).hi);
        let length = end - origin;
        if length.lo <= 0. {
            return Err(Failure::Projection);
        }
        let first = partition(rows, |r| r.stations[1].0 <= window[0], work)?;
        let end = partition(rows, |r| r.stations[0].0 < window[1], work)?;
        Ok(Self {
            network,
            query,
            trace,
            origin,
            length,
            surface: Surface {
                rows: &rows[first..end],
                window,
            },
            all_surface: rows,
            covered: None,
            prefix: 0,
            first_gap: None,
            unchecked: None,
        })
    }

    fn pose(&self, m: &Motion, work: &mut Work) -> Result<Pose, Failure> {
        Pose::enclose(&self.surface, m.p, m.heading()?.0, self.query.axles, work)
    }

    fn wheels(&self, pose: &Pose, work: &mut Work) -> Result<Coverage, Failure> {
        let axle = self.query.axles;
        let mut wheel = |a, b| -> Result<Projection, Failure> {
            work.charge()?;
            self.surface.project(pose.wheel(a, b), work)
        };
        let wheels = [
            wheel(axle.wheelbase_m, axle.half_track_m)?,
            wheel(axle.wheelbase_m, -axle.half_track_m)?,
            wheel(0., axle.half_track_m)?,
            wheel(0., -axle.half_track_m)?,
        ];
        let fractions = wheels.map(|w| (w.station - self.origin) / self.length);
        footprint::classify(self.network, self.query, fractions, wheels, work)
    }

    fn check_steering(
        &self,
        m: &Motion,
        pose: &Pose,
        range: [u64; 2],
        depth: u8,
        steering: &steering::Steering,
        work: &mut Work,
    ) -> Result<Option<TrackOutcome>, Failure> {
        match steering.check(m, pose, self.query.axles) {
            Ok(found) => Ok(found.map(|(wheel, constraint)| TrackOutcome::Infeasible {
                wheel,
                constraint,
                at_nm: range[0],
            })),
            Err(Failure::Steering) if depth > 0 && range[1] - range[0] > 1 => {
                let mid = range[0] + (range[1] - range[0]) / 2;
                for piece in [[range[0], mid], [mid, range[1]]] {
                    work.charge()?;
                    let Piece::Motion(child) = self.trace.piece(&self.surface, piece, work)? else {
                        return Err(Failure::SteeringSeam);
                    };
                    if let Some(found) =
                        self.check_steering(&child, pose, piece, depth - 1, steering, work)?
                    {
                        return Ok(Some(found));
                    }
                }
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }

    fn check(
        &self,
        m: &Motion,
        range: [u64; 2],
        depth: u8,
        steering: &steering::Steering,
        work: &mut Work,
    ) -> Result<TrackOutcome, Failure> {
        let pose = self.pose(m, work)?;
        let covered = self.wheels(&pose, work)?;
        if let Coverage::Outside(i, overrun_lower_m) = covered {
            return Ok(TrackOutcome::Outside {
                overrun_lower_m,
                wheel: WHEELS[i],
                at_nm: range[0],
            });
        }
        if let Some(failed) =
            self.check_steering(m, &pose, range, (20 - depth).min(4), steering, work)?
        {
            return Ok(failed);
        }
        match covered {
            Coverage::Inside => Ok(TrackOutcome::Covered(pose.bounds())),
            Coverage::Uncertain(i) => Err(Failure::Footprint(WHEELS[i])),
            Coverage::Outside(..) => unreachable!(),
        }
    }

    fn seam(&self, index: usize, work: &mut Work) -> Result<bool, Failure> {
        let Path::Corridor(t) = self.trace else {
            return Err(Failure::Binding);
        };
        let a = &self.surface.rows[index];
        let b = &self.surface.rows[index + 1];
        let at = a.stations[1].0;
        let mut k = |row| {
            work.charge()?;
            let m = path::sided(row, at, t)?;
            let pose = self.pose(&m, work)?;
            Ok(steering::curvature(&m, &pose, self.query.axles)?.0)
        };
        let left: Interval = k(a)?;
        let right = k(b)?;
        if left.intersect(right).is_none() {
            Ok(false)
        } else {
            Err(Failure::SteeringSeam)
        }
    }

    fn visit(
        &mut self,
        range: [u64; 2],
        depth: u8,
        steering: &steering::Steering,
        work: &mut Work,
    ) -> Option<TrackOutcome> {
        let piece = work
            .charge()
            .and_then(|()| self.trace.piece(&self.surface, range, work));
        let result = match piece {
            Ok(Piece::Join { at, smooth, rows }) => {
                if let Some(found) = self.visit([range[0], at], depth, steering, work) {
                    return Some(found);
                }
                if steering.moving() && !smooth {
                    match self.seam(rows, work) {
                        Ok(false) => {
                            return Some(TrackOutcome::Infeasible {
                                wheel: Wheel::FrontLeft,
                                constraint: SteeringConstraint::Jump,
                                at_nm: at,
                            });
                        }
                        Err(reason) => {
                            self.first_gap.get_or_insert(([at, at], reason));
                        }
                        Ok(true) => {}
                    }
                }
                return self.visit([at, range[1]], depth, steering, work);
            }
            Ok(Piece::Motion(m)) => self.check(&m, range, depth, steering, work),
            Err(reason) => Err(reason),
        };
        let reason = match result {
            Ok(TrackOutcome::Covered(pose)) => {
                if self.first_gap.is_none() {
                    self.prefix = range[1];
                }
                self.covered = Some(self.covered.map_or(pose, |old| old.hull(pose)));
                return None;
            }
            Ok(other) => return Some(other),
            Err(reason) => reason,
        };
        let width = range[1] - range[0];
        let can_split = matches!(
            reason,
            Failure::Footprint(_) | Failure::Projection | Failure::Pose | Failure::Steering
        );
        if can_split && depth < 20 && width > 1 {
            let mid = range[0] + width / 2;
            return self
                .visit([range[0], mid], depth + 1, steering, work)
                .or_else(|| self.visit([mid, range[1]], depth + 1, steering, work));
        }
        if reason == Failure::WorkLimit {
            self.unchecked = Some([range[0], self.query.trace.length()]);
        }
        let (range_nm, first) = *self.first_gap.get_or_insert((range, reason));
        if !can_split {
            return Some(TrackOutcome::Unproven {
                range_nm,
                reason: first,
            });
        }
        None
    }
}

impl SharedNetworkRevision {
    /// 在给定车速下连续检查四轮路幅和转向。车速按三维后轴实际行程计。
    /// 返回位置沿输入参数递增；反向候选仍从实际行驶起点量起。
    #[must_use]
    pub fn wheel_tracks(
        &self,
        query: &TrackQuery<'_>,
        limits: SteeringLimits,
        budget: u32,
    ) -> TrackReport {
        let range = [0, query.trace.length()];
        let mut work = Work::new(budget.min(TRACK_WORK_LIMIT));
        let bound = steering::Steering::new(limits).and_then(|s| {
            if range[1] > 0 && !s.moving() {
                return Err(Failure::InvalidInput);
            }
            Ok((Candidate::bind(self, query, &mut work)?, s))
        });
        let (outcome, prefix, unchecked_nm) = match bound {
            Err(reason) => (
                TrackOutcome::Unproven {
                    range_nm: range,
                    reason,
                },
                0,
                work.exhausted.then_some(range),
            ),
            Ok((mut candidate, steering)) => {
                let result = candidate.visit(range, 0, &steering, &mut work).unwrap_or({
                    if let Some((range_nm, reason)) = candidate.first_gap {
                        TrackOutcome::Unproven { range_nm, reason }
                    } else if let Some(pose) = candidate.covered {
                        TrackOutcome::Covered(pose)
                    } else {
                        TrackOutcome::Unproven {
                            range_nm: range,
                            reason: Failure::Pose,
                        }
                    }
                });
                (result, candidate.prefix, candidate.unchecked)
            }
        };
        TrackReport {
            outcome,
            covered_prefix_nm: prefix,
            work: work.used,
            unchecked_nm,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn both_hard_caps_admit_the_last_logical_visit_and_refuse_the_next() {
        for limit in [TRACK_WORK_LIMIT, SWEEP_WORK_LIMIT] {
            let mut w = Work::new(limit);
            for _ in 0..limit - 1 {
                w.charge().unwrap();
            }
            assert_eq!(w.used, limit - 1);
            w.charge().unwrap();
            assert_eq!(w.used, limit);
            assert!(!w.exhausted);
            assert_eq!(w.charge(), Err(Failure::WorkLimit));
            assert!(w.exhausted);
            assert_eq!(w.used, limit);
        }
    }
}
