//! 同一候选的三维后轴行程与有向站位互查。
use super::*;
use crate::station::vehicle::TRACK_WORK_LIMIT;
use crate::station::vehicle::pose::{RearAxle, Vector};

/// 指定候选起点之后的走廊站位或累计后轴行程，单位毫米。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandoffTravelTarget {
    /// 从该修订的走廊局部原点量取的后轴站位。
    RearStationMm(u64),
    /// 从候选起点沿后轴三维轨迹量取的行程。
    DistanceMm(u64),
}

impl HandoffTravelTarget {
    /// 输入毫米是否落在几何查询的数值域内。走廊或轨迹的范围由绑定后校验。
    #[must_use]
    pub fn is_representable(self) -> bool {
        let (Self::RearStationMm(mm) | Self::DistanceMm(mm)) = self;
        mm_to_nm(mm).is_ok()
    }
}

/// 后轴在走廊上的连续站位界，按数值递增。可直接用于本拍的车体与扫过查询。
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct HandoffStationRange(pub(super) [u64; 2]);

impl core::fmt::Debug for HandoffStationRange {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HandoffStationRange")
            .field("stations_mm", &self.millimetres())
            .finish()
    }
}

impl HandoffStationRange {
    /// 以两个整数毫米声明闭区间；倒序或超出求证域时返回空。
    #[must_use]
    pub fn from_millimetres([lo, hi]: [u64; 2]) -> Option<Self> {
        if lo > hi {
            return None;
        }
        Some(Self([
            lo.checked_mul(NM_PER_MM)?,
            hi.checked_mul(NM_PER_MM)?,
        ]))
    }

    /// 向外取整，返回覆盖所有可能后轴位置的毫米闭区间。
    #[must_use]
    pub fn millimetres(self) -> [u64; 2] {
        enclosing_mm(self.0)
    }

    /// 包含两个站位界的最小连续区间；两端保留原求证精度。
    #[must_use]
    pub fn hull(self, other: Self) -> Self {
        Self([self.0[0].min(other.0[0]), self.0[1].max(other.0[1])])
    }
}

/// 真实三维行程的连续求证区间。
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct HandoffDistanceBounds([u64; 2]);

impl core::fmt::Debug for HandoffDistanceBounds {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HandoffDistanceBounds")
            .field("distance_mm", &self.millimetres())
            .finish()
    }
}

impl HandoffDistanceBounds {
    /// 向外取整得到的毫米闭区间。
    #[must_use]
    pub fn millimetres(self) -> [u64; 2] {
        enclosing_mm(self.0)
    }

    /// 两端所在的毫米格。每格包含自身起点，直到下一格前。
    #[must_use]
    pub fn millimetre_cells(self) -> [u64; 2] {
        self.0.map(|x| x / NM_PER_MM)
    }

    /// 积分位置已达到行程上界时给出余程的下、上界。每端为整毫米和千分之一毫米余数。
    #[must_use]
    pub fn remaining_after_motion(self, whole_mm: u64, carry: u16) -> Option<[(u64, u16); 2]> {
        let total = motion_distance(whole_mm, carry)?;
        let [lo, hi] = self.0;
        let rest = [total.checked_sub(hi)? / 1_000, (total - lo).div_ceil(1_000)];
        Some(rest.map(|um| (um / 1_000, (um % 1_000) as u16)))
    }
}

fn motion_distance(whole_mm: u64, carry: u16) -> Option<u64> {
    if carry >= 1_000 {
        return None;
    }
    whole_mm
        .checked_mul(NM_PER_MM)?
        .checked_add(u64::from(carry) * 1_000)
}

/// 同一候选的后轴站位与三维行程界。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HandoffTravelBounds {
    /// 后轴绝对站位。
    pub rear: HandoffStationRange,
    /// 沿行驶方向量取的累计三维行程。
    pub distance: HandoffDistanceBounds,
}

/// 一次累计行程或反向查询及其实际逻辑工作数。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HandoffTravelReport {
    /// 成功时输入量保持精确，连续区间达到请求的容差。显示为毫米闭区间时，两端向外取整。
    pub bounds: Result<HandoffTravelBounds, Failure>,
    /// 包含构型绑定、分段、姿态、投影、积分和反查的实际工作数。
    pub work: u32,
}

/// 原候选多个有向参数端点的累计后轴三维行程。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HandoffTravelMarksReport {
    /// 与请求端点同序的累计行程界。只有全段达到精度时才返回。
    pub distances: Result<Vec<HandoffDistanceBounds>, Failure>,
    /// 全部标记共用的绑定、积分及精度检查工作数。
    pub work: u32,
}

fn nm(value: I) -> Result<[u64; 2], Failure> {
    let scaled = value * I::point(1e9);
    if !scaled.finite() || scaled.hi >= u64::MAX as f64 || scaled.hi < 0. {
        return Err(Failure::TravelResolution);
    }
    Ok([scaled.lo.max(0.).floor() as u64, scaled.hi.ceil() as u64])
}

fn rear(c: &Candidate<'_, '_>, at: u64, work: &mut Work) -> Result<Vector, Failure> {
    let Piece::Motion(m) = c.trace.piece(&c.surface, [at; 2], work)? else {
        return Err(Failure::Projection);
    };
    let (heading, _) = m.heading()?;
    Ok(RearAxle::enclose(&c.surface, m.p, heading, c.query.axles, work)?.position)
}

struct Travel {
    distance: I,
    // 可能包含目标的各分段按行驶方向访问，在证明到达之前保留全部可能解。
    offsets: Option<[u64; 2]>,
    target: Option<I>,
    refine_from_end: bool,
    tolerance_nm: u64,
    allowed_spread: I,
}

enum Segment {
    Visited { reached: bool },
    Join(u64),
}

impl Travel {
    fn new(length: u64, tolerance_nm: u64, target: Option<I>) -> Self {
        Self {
            distance: I::ZERO,
            offsets: None,
            target,
            refine_from_end: false,
            tolerance_nm,
            // 按参数宽度分摊整条查询的误差，给加和、向外取整和逆查留出余量。
            allowed_spread: I::meters(tolerance_nm) / I::meters(length) * I::point(0.75),
        }
    }

    fn distance_nm(&self) -> Result<[u64; 2], Failure> {
        let distance = nm(self.distance)?;
        if distance[1] - distance[0] > self.tolerance_nm {
            return Err(Failure::TravelResolution);
        }
        Ok(distance)
    }

    /// 设参数跨度为 h、后轴端点差为 d。Cauchy–Schwarz 给出
    /// L² ≤ |d|² + h² Σ Var(vᵢ)，每个导数分量的方差至多为其值域宽度平方的 1/4。
    /// 与弦长下界及直接速度积分相交，避免把弦长当成弧长。
    fn segment(
        &mut self,
        span: [u64; 2],
        from: Vector,
        to: Vector,
        velocity: Vector,
        speed: I,
    ) -> Result<bool, Failure> {
        let h = I::meters(span[1] - span[0]);
        let mut chord2 = I::ZERO;
        let mut variance = I::ZERO;
        for i in 0..3 {
            let v = velocity[i];
            let delta = (to[i] - from[i]).intersect(h * v).ok_or(Failure::Pose)?;
            chord2 = chord2 + delta.square();
            variance = variance + (I::point(v.hi) - I::point(v.lo)).square();
        }
        let distance = I {
            lo: chord2.sqrt().lo,
            hi: (chord2 + h.square() * variance * I::point(0.25)).sqrt().hi,
        }
        .intersect(h * speed)
        .ok_or(Failure::Pose)?;
        if !distance.finite() || speed.lo <= 0. {
            return Err(Failure::Pose);
        }
        let end = self.distance + distance;
        let mut offsets = self.offsets;
        if let Some(target) = self.target
            && target.hi >= self.distance.lo
            && target.lo <= end.hi
        {
            // 正速度使段内根从首端、末端分别满足同一个积分速度界。
            // 目标可能跨越分割点时，末端反推还能收紧即将保留到下一段的下界。
            let mut local = (target - self.distance) / speed;
            if self.refine_from_end {
                local = local
                    .intersect(h - (end - target) / speed)
                    .ok_or(Failure::Projection)?;
            }
            let local = local
                .intersect(I::ZERO.hull(h))
                .ok_or(Failure::Projection)?;
            let [l, r] = nm(local)?.map(|x| span[0] + x.min(span[1] - span[0]));
            let range = [offsets.map_or(l, |prev| prev[0].min(l)), r];
            if range[1] - range[0] > self.tolerance_nm {
                return Err(Failure::TravelResolution);
            }
            offsets = Some(range);
            if end.lo >= target.hi {
                self.offsets = offsets;
                return Ok(true);
            }
        }
        if (I::point(end.hi) - I::point(end.lo)).hi > (I::meters(span[1]) * self.allowed_spread).lo
        {
            return Err(Failure::TravelResolution);
        }
        self.offsets = offsets;
        self.distance = end;
        Ok(false)
    }

    fn visit(
        &mut self,
        c: &Candidate<'_, '_>,
        span: [u64; 2],
        endpoints: [Vector; 2],
        depth: u8,
        work: &mut Work,
    ) -> Result<bool, Failure> {
        work.charge()?;
        let result = c
            .trace
            .piece(&c.surface, span, work)
            .and_then(|piece| match piece {
                Piece::Join {
                    at, smooth: true, ..
                } => Ok(Segment::Join(at)),
                Piece::Join { .. } => Err(Failure::SteeringSeam),
                Piece::Motion(m) => {
                    let (heading, d_heading) = m.heading()?;
                    let rear = RearAxle::enclose(&c.surface, m.p, heading, c.query.axles, work)?;
                    let (v, speed) = rear.rate(c.query.axles, m.v, d_heading);
                    self.segment(span, endpoints[0], endpoints[1], v, speed)
                        .map(|reached| Segment::Visited { reached })
                }
            });
        let (split, depth) = match result {
            Ok(Segment::Visited { reached }) => return Ok(reached),
            Ok(Segment::Join(at)) => (at, depth),
            Err(Failure::TravelResolution | Failure::Pose | Failure::Projection)
                if depth < 20 && span[1] - span[0] > 1 =>
            {
                (span[0] + (span[1] - span[0]) / 2, depth + 1)
            }
            Err(failure) => return Err(failure),
        };
        let middle = rear(c, split, work)?;
        if self.visit(c, [span[0], split], [endpoints[0], middle], depth, work)? {
            return Ok(true);
        }
        self.visit(c, [split, span[1]], [middle, endpoints[1]], depth, work)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Station(u64),
    Distance(u64),
}

impl SharedNetworkRevision {
    /// 在同一条五次横移上按行驶方向一次积分，取得一组原参数端点的累计三维行程。
    /// `offsets_mm` 为相对起点的后轴站位差，严格递增，末项须为候选参数总长，
    /// 可含起点零。反向行驶同样使用递增站位差。先按每个标记一项核验输入，
    /// 再绑定候选并积分；所有端点共用一次工作预算，
    /// 连续求界容差为正整数毫米，公开的整毫米范围再向外取整。几何准入由 `vehicle_handoff` 求证。
    #[must_use]
    pub fn handoff_travel_marks(
        &self,
        q: &HandoffQuery,
        window: HandoffWindow,
        offsets_mm: &[u64],
        tolerance_mm: u64,
        work_limit: u32,
    ) -> HandoffTravelMarksReport {
        let mut work = Work::new(work_limit.min(TRACK_WORK_LIMIT));
        let mut run = || {
            let length = q.rear_stations_mm[0].abs_diff(q.rear_stations_mm[1]);
            if length == 0 || offsets_mm.last() != Some(&length) || tolerance_mm == 0 {
                return Err(Failure::InvalidInput);
            }
            for (i, &end) in offsets_mm.iter().enumerate() {
                work.charge()?;
                if end > length || (i > 0 && end <= offsets_mm[i - 1]) {
                    return Err(Failure::InvalidInput);
                }
            }
            let bound = bind(self, q, window, &mut work)?;
            let query = bound.query(q);
            let c = Candidate::bind(self, &query, &mut work)?;
            let mut previous = 0;
            let mut from = rear(&c, 0, &mut work)?;
            let length = mm_to_nm(length)?;
            let mut walk = Travel::new(length, mm_to_nm(tolerance_mm)?, None);
            let mut marks = Vec::new();
            for &end_mm in offsets_mm {
                let end = mm_to_nm(end_mm)?;
                let distance = if end == 0 {
                    [0; 2]
                } else {
                    let to = rear(&c, end, &mut work)?;
                    walk.visit(&c, [previous, end], [from, to], 0, &mut work)?;
                    from = to;
                    walk.distance_nm()?
                };
                marks.try_reserve(1).map_err(|_| Failure::Unavailable)?;
                marks.push(HandoffDistanceBounds(distance));
                previous = end;
            }
            Ok(marks)
        };
        let distances = run();
        HandoffTravelMarksReport {
            distances,
            work: work.used,
        }
    }

    /// 在指定站位求累计三维行程，或由真实行程反求站位。连续求界容差为正整数毫米；
    /// 输出为毫米读数时下界向下、上界向上取整。超过预算或无法达到容差时返回失败。
    #[must_use]
    pub fn handoff_travel(
        &self,
        q: &HandoffQuery,
        window: HandoffWindow,
        target: HandoffTravelTarget,
        tolerance_mm: u64,
        work_limit: u32,
    ) -> HandoffTravelReport {
        let target = match target {
            HandoffTravelTarget::RearStationMm(s) => mm_to_nm(s).map(Target::Station),
            HandoffTravelTarget::DistanceMm(d) => mm_to_nm(d).map(Target::Distance),
        };
        self.travel(q, window, target, tolerance_mm, work_limit)
    }

    /// 从固定步进的累计行程反求站位。整数毫米和积分余数共同确定位置，
    /// 余数的单位为千分之一毫米，有效范围为 0..1000。
    #[must_use]
    pub fn handoff_travel_after_motion(
        &self,
        q: &HandoffQuery,
        window: HandoffWindow,
        whole_mm: u64,
        carry: u16,
        tolerance_mm: u64,
        work_limit: u32,
    ) -> HandoffTravelReport {
        let target = motion_distance(whole_mm, carry)
            .ok_or(Failure::InvalidInput)
            .map(Target::Distance);
        self.travel(q, window, target, tolerance_mm, work_limit)
    }

    fn travel(
        &self,
        q: &HandoffQuery,
        window: HandoffWindow,
        target: Result<Target, Failure>,
        tolerance_mm: u64,
        work_limit: u32,
    ) -> HandoffTravelReport {
        let mut work = Work::new(work_limit.min(TRACK_WORK_LIMIT));
        let mut run = || {
            let target = target?;
            let tolerance_nm = mm_to_nm(tolerance_mm)?;
            let [start, end] = q.stations_nm()?;
            let (length, sought) = match target {
                Target::Station(s) if (start.min(end)..=start.max(end)).contains(&s) => {
                    (start.abs_diff(s), None)
                }
                Target::Station(_) => return Err(Failure::InvalidInput),
                Target::Distance(d) => (start.abs_diff(end), Some(I::meters(d))),
            };
            if tolerance_nm == 0 {
                return Err(Failure::InvalidInput);
            }
            let bound = bind(self, q, window, &mut work)?;
            let query = bound.query(q);
            let c = Candidate::bind(self, &query, &mut work)?;
            let first = rear(&c, 0, &mut work)?;
            if length == 0 || target == Target::Distance(0) {
                return Ok(HandoffTravelBounds {
                    rear: HandoffStationRange([start; 2]),
                    distance: HandoffDistanceBounds([0; 2]),
                });
            }
            let last = rear(&c, length, &mut work)?;
            let mut walk = Travel::new(length, tolerance_nm, sought);
            let visit = |walk: &mut Travel, work: &mut Work| {
                walk.visit(&c, [0, length], [first, last], 0, work)
            };
            let reached = match visit(&mut walk, &mut work) {
                Err(Failure::TravelResolution) if sought.is_some() => {
                    // 保持原本已成功的站位界和切点归属。单向逆查失证才重新遍历，
                    // 利用两端行程界夹逼根；两次共用调用方的一份工作额度。
                    walk = Travel {
                        refine_from_end: true,
                        ..Travel::new(length, tolerance_nm, sought)
                    };
                    visit(&mut walk, &mut work)?
                }
                result => result?,
            };
            match target {
                Target::Station(s) => {
                    let distance_nm = walk.distance_nm()?;
                    Ok(HandoffTravelBounds {
                        rear: HandoffStationRange([s; 2]),
                        distance: HandoffDistanceBounds(distance_nm),
                    })
                }
                Target::Distance(d) => {
                    if !reached {
                        return Err(if walk.offsets.is_some() {
                            Failure::TravelResolution
                        } else {
                            Failure::InvalidInput
                        });
                    }
                    let [lo, hi] = walk.offsets.ok_or(Failure::TravelResolution)?;
                    let stations = if start < end {
                        [start + lo, start + hi]
                    } else {
                        [start - hi, start - lo]
                    };
                    Ok(HandoffTravelBounds {
                        rear: HandoffStationRange(stations),
                        distance: HandoffDistanceBounds([d; 2]),
                    })
                }
            }
        };
        let bounds = run();
        HandoffTravelReport {
            bounds,
            work: work.used,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_millimetre_reads_and_continuous_motion_use_their_own_bounds() {
        let distance = HandoffDistanceBounds([4_999_998, 4_999_999]);
        assert_eq!(distance.millimetres(), [4, 5]);
        assert_eq!(distance.millimetre_cells(), [4, 4]);
        assert_eq!(
            distance.remaining_after_motion(5, 927),
            Some([(0, 927), (0, 928)])
        );
        assert_eq!(distance.remaining_after_motion(4, 999), None);
        assert_eq!(distance.remaining_after_motion(5, 1_000), None);
        assert_eq!(distance.remaining_after_motion(u64::MAX, 0), None);
        let station = HandoffStationRange([4_999_998, 4_999_999]);
        let next = HandoffStationRange::from_millimetres([5, 5]).unwrap();
        assert_eq!(station.hull(next).0, [4_999_998, 5_000_000]);
        assert_eq!(station.hull(next).millimetres(), [4, 5]);
        assert_eq!(HandoffStationRange::from_millimetres([u64::MAX; 2]), None);
    }
}
