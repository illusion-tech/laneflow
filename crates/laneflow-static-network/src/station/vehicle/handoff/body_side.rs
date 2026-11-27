//! 同一候选的三维车体在结构共边哪一侧。
use super::*;

/// 对查询的每一个后轴位置都成立的车身与共边关系。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandoffBodySide {
    /// 完整车身严格位于源侧。
    SourceOnly,
    /// 同一姿态的实体车身在两侧分别有严格位于该侧的点。
    Straddling,
    /// 完整车身严格位于目标侧。
    TargetOnly,
}

/// 车身与共边的横向关系及本次查询已用的逻辑工作量。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HandoffBodyReport {
    /// 单侧须证明整车分离；相切、区间内关系改变或证据不足时返回错误。
    pub side: Result<HandoffBodySide, Failure>,
    /// 包括绑定、姿态、车身投影和分段在内的实际工作数。
    pub work: u32,
}

fn strict_side(signed: I) -> Option<HandoffBodySide> {
    if signed.hi < 0. {
        Some(HandoffBodySide::SourceOnly)
    } else if signed.lo > 0. {
        Some(HandoffBodySide::TargetOnly)
    } else {
        None
    }
}

fn classify(
    c: &Candidate<'_, '_>,
    seam: &Crossing,
    shape: BodyEnvelope,
    range: [u64; 2],
    work: &mut Work,
) -> Result<HandoffBodySide, Failure> {
    work.charge()?;
    let motion = match c.trace.piece(&c.surface, range, work)? {
        Piece::Join {
            at, smooth: true, ..
        } => {
            let before = classify(c, seam, shape, [range[0], at], work)?;
            let after = classify(c, seam, shape, [at, range[1]], work)?;
            return if before == after {
                Ok(before)
            } else {
                Err(Failure::Boundary)
            };
        }
        Piece::Join { .. } => return Err(Failure::SteeringSeam),
        Piece::Motion(motion) => motion,
    };
    let pose = c.pose(&motion, work)?;
    let [long, lat, height] = shape.ranges();
    let signed = |p: Projection| seam.sign * (p.lateral - seam.line.lateral(c, p.station));
    let project =
        |point: pose::Vector, work: &mut Work| c.surface.project([point[0], point[2]], work);
    if let sweep::BodyBoundary::Separated(offset) =
        seam.line
            .enclose_body(c, &pose, [long, lat, height], work)?
        && let Some(side) = strict_side(seam.sign * offset)
    {
        return Ok(side);
    }
    // 整车界含零只表示可能相交。两个严格异号的实体点，分别在每个可能的
    // 同时刻姿态中都落在相应一侧，才能证明整个站位区间均有双侧占用。
    let mut on_source = false;
    let mut on_target = false;
    for a in [long.lo, long.hi, long.lo.midpoint(long.hi)] {
        for b in [lat.lo, lat.hi] {
            for z in [height.lo, height.hi] {
                let point = pose.point(I::point(a), I::point(b), I::point(z));
                let value = signed(project(point, work)?);
                on_source |= value.hi < 0.;
                on_target |= value.lo > 0.;
                if on_source && on_target {
                    return Ok(HandoffBodySide::Straddling);
                }
            }
        }
    }
    Err(Failure::Boundary)
}

pub(super) fn body_offsets(
    q: &HandoffQuery,
    body: BodyEnvelope,
    stations: [u64; 2],
) -> Result<[u64; 2], Failure> {
    let [start, end] = q.rear_stations_nm;
    let [low, high] = stations;
    if low > high
        || low < start.min(end)
        || high > start.max(end)
        || !body.valid(q.axles.wheelbase_m)
        || body.half_width_m < q.axles.half_track_m
    {
        return Err(Failure::InvalidInput);
    }
    Ok(if start < end {
        [low - start, high - start]
    } else {
        [start - high, start - low]
    })
}

impl SharedNetworkRevision {
    /// 在原横移候选上检验实体车身相对绑定共边的横向位置。后轴绝对站位闭区间须
    /// 按数值递增；行驶方向来自候选起止站位。工作上限为 [`SWEEP_WORK_LIMIT`]。
    /// 结果只覆盖这条结构共边。预约释放还须检查相应物理区间和下游清空义务。
    #[must_use]
    pub fn handoff_body_side(
        &self,
        q: &HandoffQuery,
        window: HandoffWindow,
        body: BodyEnvelope,
        rear_stations_nm: [u64; 2],
        work_limit: u32,
    ) -> HandoffBodyReport {
        let mut work = Work::new(work_limit.min(SWEEP_WORK_LIMIT));
        let mut run = || {
            let offsets = body_offsets(q, body, rear_stations_nm)?;
            let bound = bind(self, q, window, &mut work)?;
            let query = bound.query(q);
            let c = Candidate::bind(self, &query, &mut work)?;
            classify(&c, &bound.crossing, body, offsets, &mut work)
        };
        let side = run();
        HandoffBodyReport {
            side,
            work: work.used,
        }
    }
}
