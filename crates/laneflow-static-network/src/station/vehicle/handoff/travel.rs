//! 同一候选的三维后轴行程与有向站位互查。
use super::*;
use crate::station::vehicle::TRACK_WORK_LIMIT;
use crate::station::vehicle::pose::{RearAxle, Vector};

/// 指定候选起点之后的绝对走廊站位或累计后轴行程。两者均以纳米计。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandoffTravelTarget {
    /// 从该修订的走廊局部原点量取的后轴站位。
    RearStationNm(u64),
    /// 从候选起点沿后轴三维轨迹量取的行程。
    DistanceNm(u64),
}

/// 同一候选位置的闭区间。两端始终按数值递增，包括反向车道。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HandoffTravelBounds {
    /// 后轴的绝对走廊站位，单位纳米。
    pub rear_stations_nm: [u64; 2],
    /// 从候选起点沿行驶方向量取的三维行程，单位纳米。
    pub distance_nm: [u64; 2],
}

/// 一次累计行程或反向查询及其实际逻辑工作数。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HandoffTravelReport {
    /// 成功时输入量保持精确，求得的区间宽度不超过请求精度。
    pub bounds: Result<HandoffTravelBounds, Failure>,
    /// 包含构型绑定、分段、姿态、投影、积分和反查的实际工作数。
    pub work: u32,
}

/// 原候选多个有向参数端点的累计后轴三维行程。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HandoffTravelMarksReport {
    /// 与请求端点同序的累计行程闭区间（纳米）。只有全段达到精度时才返回。
    pub distances_nm: Result<Vec<[u64; 2]>, Failure>,
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
            let local = ((target - self.distance) / speed)
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

impl SharedNetworkRevision {
    /// 在同一条五次横移上按行驶方向一次积分，取得一组原参数端点的累计三维行程。
    /// `offsets_nm` 为相对起点的后轴站位差，严格递增，末项须为候选参数总长，
    /// 可含起点零。反向行驶同样使用递增站位差。先按每个标记一项核验输入，
    /// 再绑定候选并积分；所有端点共用一次工作预算，
    /// 每个累计结果的宽度均不超过正的 `tolerance_nm`。几何准入由 `vehicle_handoff` 求证。
    #[must_use]
    pub fn handoff_travel_marks(
        &self,
        q: &HandoffQuery,
        window: HandoffWindow,
        offsets_nm: &[u64],
        tolerance_nm: u64,
        work_limit: u32,
    ) -> HandoffTravelMarksReport {
        let mut work = Work::new(work_limit.min(TRACK_WORK_LIMIT));
        let mut run = || {
            let length = q.rear_stations_nm[0].abs_diff(q.rear_stations_nm[1]);
            if length == 0 || offsets_nm.last() != Some(&length) || tolerance_nm == 0 {
                return Err(Failure::InvalidInput);
            }
            for (i, &end) in offsets_nm.iter().enumerate() {
                work.charge()?;
                if end > length || (i > 0 && end <= offsets_nm[i - 1]) {
                    return Err(Failure::InvalidInput);
                }
            }
            let bound = bind(self, q, window, &mut work)?;
            let query = bound.query(q);
            let c = Candidate::bind(self, &query, &mut work)?;
            let mut previous = 0;
            let mut from = rear(&c, 0, &mut work)?;
            let mut walk = Travel::new(length, tolerance_nm, None);
            let mut marks = Vec::new();
            for &end in offsets_nm {
                let distance = if end == 0 {
                    [0; 2]
                } else {
                    let to = rear(&c, end, &mut work)?;
                    walk.visit(&c, [previous, end], [from, to], 0, &mut work)?;
                    from = to;
                    walk.distance_nm()?
                };
                marks.try_reserve(1).map_err(|_| Failure::Unavailable)?;
                marks.push(distance);
                previous = end;
            }
            Ok(marks)
        };
        let distances_nm = run();
        HandoffTravelMarksReport {
            distances_nm,
            work: work.used,
        }
    }

    /// 在给定的车型、路层及五次横移上求后轴累计三维行程，或由行程反求站位。
    /// `tolerance_nm` 是输出闭区间的最大宽度，须大于零。输入超出候选或无法
    /// 证明指定精度时返回错误。工作预算钳制到 `TRACK_WORK_LIMIT`，耗尽后不发布近似值。
    /// 此查询量取几何行程，候选通行许可由 `vehicle_handoff` 提供。
    #[must_use]
    pub fn handoff_travel(
        &self,
        q: &HandoffQuery,
        window: HandoffWindow,
        target: HandoffTravelTarget,
        tolerance_nm: u64,
        work_limit: u32,
    ) -> HandoffTravelReport {
        let mut work = Work::new(work_limit.min(TRACK_WORK_LIMIT));
        let mut run = || {
            let [start, end] = q.rear_stations_nm;
            let (length, sought) = match target {
                HandoffTravelTarget::RearStationNm(s)
                    if (start.min(end)..=start.max(end)).contains(&s) =>
                {
                    (start.abs_diff(s), None)
                }
                HandoffTravelTarget::RearStationNm(_) => return Err(Failure::InvalidInput),
                HandoffTravelTarget::DistanceNm(d) => (start.abs_diff(end), Some(I::meters(d))),
            };
            if tolerance_nm == 0 {
                return Err(Failure::InvalidInput);
            }
            let bound = bind(self, q, window, &mut work)?;
            let query = bound.query(q);
            let c = Candidate::bind(self, &query, &mut work)?;
            let first = rear(&c, 0, &mut work)?;
            if length == 0 || target == HandoffTravelTarget::DistanceNm(0) {
                return Ok(HandoffTravelBounds {
                    rear_stations_nm: [start; 2],
                    distance_nm: [0; 2],
                });
            }
            let last = rear(&c, length, &mut work)?;
            let mut walk = Travel::new(length, tolerance_nm, sought);
            let reached = walk.visit(&c, [0, length], [first, last], 0, &mut work)?;
            match target {
                HandoffTravelTarget::RearStationNm(s) => {
                    let distance_nm = walk.distance_nm()?;
                    Ok(HandoffTravelBounds {
                        rear_stations_nm: [s; 2],
                        distance_nm,
                    })
                }
                HandoffTravelTarget::DistanceNm(d) => {
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
                        rear_stations_nm: stations,
                        distance_nm: [d; 2],
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
