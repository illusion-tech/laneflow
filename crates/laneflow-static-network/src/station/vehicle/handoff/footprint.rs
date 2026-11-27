//! 从同一车体姿态定位前杠和车身在两侧物理边上的范围。
use super::*;
use crate::station::vehicle::pose::Vector;
use crate::{StationAddress, StationCandidates};

/// 相对于有向交接的车道。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandoffLaneSide {
    /// 换出车道。
    Source,
    /// 换入车道。
    Target,
}

impl HandoffLaneSide {
    fn of(offset: I) -> Option<Self> {
        if offset.hi < 0. {
            Some(Self::Source)
        } else if offset.lo > 0. {
            Some(Self::Target)
        } else {
            None
        }
    }
}

/// 一侧已有实体车身的证据。在每个可能姿态中该侧均有车体点。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HandoffSideFootprint {
    /// 该侧全部实体点的保守站位与边内范围。外包中的每个位置不一定都被占用。
    pub enclosure: StationCandidates,
    /// 至少一个严格位于此侧的实体点所在的可能位置。跨切点时各候选边是备选。
    pub witness: StationCandidates,
}

/// 给定后轴位置域内的前杠中点与两侧车身。此结果不发放或释放交通预约。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HandoffFootprint {
    /// 前杠的已证侧别。已预约查询若保留源侧游标，同时返回目标侧可能位置。
    pub front_side: HandoffLaneSide,
    /// 前保险杠中点在该侧独立边链上的可能位置。
    pub front: StationCandidates,
    /// 已预约且尚未证实前杠离开源侧时，目标侧同一前杠的可能位置。
    pub possible_target_front: Option<StationAddress>,
    /// 顺序为源、目标。`None` 表示已证明该侧没有车身。
    pub sides: [Option<HandoffSideFootprint>; 2],
}

/// 一次完整定位的结果和实际逻辑工作量。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HandoffFootprintReport {
    /// 缺少完整证据时不返回部分覆盖。
    pub footprint: Result<HandoffFootprint, Failure>,
    /// 包括绑定、连续求界、实体点投影和边链映射。
    pub work: u32,
}

#[derive(Clone, Copy)]
struct Side {
    enclosure: I,
    witness: I,
}

struct Stations {
    pose: PoseBounds,
    front_side: Option<HandoffLaneSide>,
    front: I,
    sides: [Option<Side>; 2],
}

// 对该侧的真实点，至少有 F(后轴)+Σkᵢxᵢ ≥ 0；kᵢ 包住从后轴到点的梯度积分。
// 只丢弃由这个必要条件排除的局部坐标。留下的外包独立于实体点见证。
fn side_coordinates(
    offset: I,
    coefficients: Vector,
    mut coords: Vector,
    work: &mut Work,
) -> Result<Option<Vector>, Failure> {
    for (i, k) in coefficients.into_iter().enumerate() {
        if k.lo <= 0. && k.hi >= 0. {
            continue;
        }
        work.charge()?;
        let residual = (0..3)
            .filter(|&j| j != i)
            .fold(offset, |sum, j| sum + coefficients[j] * coords[j]);
        let root = -residual / k;
        if k.lo > 0. {
            coords[i].lo = coords[i].lo.max(root.lo);
        } else {
            coords[i].hi = coords[i].hi.min(root.hi);
        }
        if coords[i].hi < coords[i].lo {
            return Ok(None);
        }
    }
    Ok(Some(coords))
}

fn horizontal(p: Vector) -> [I; 2] {
    [p[0], p[2]]
}

// 站位梯度沿后轴到实体点的整条线段求界，使车体长度与宽度在旋转后仍各沿自身的轴。
pub(super) fn station_enclosure(
    c: &Candidate<'_, '_>,
    pose: &Pose,
    coords: Vector,
    rear: Projection,
    work: &mut Work,
) -> Result<I, Failure> {
    let [a, b, z] = coords;
    let world = horizontal(pose.point(a, b, z));
    let region = c.surface.project(world, work)?;
    let origin = horizontal(pose.rear);
    let domain = std::array::from_fn(|i| world[i].hull(origin[i]));
    let gradient = if domain == world {
        region.station_gradient
    } else {
        c.surface.project(domain, work)?.station_gradient
    };
    let mut s = rear.station;
    for (vector, coefficient) in [pose.forward, pose.left, pose.up].into_iter().zip(coords) {
        s = s + dot(gradient, horizontal(vector)) * coefficient;
    }
    // 梯度约束的是连续曲线参数；目标端还保留其各发布行声明的站位误差。
    s.with_error(region.station_error_m)
        .intersect(region.station)
        .ok_or(Failure::Projection)
}

pub(super) fn within_outer(
    c: &Candidate<'_, '_>,
    bound: &Bound,
    pose: &Pose,
    coords: Vector,
    front: Option<Projection>,
    work: &mut Work,
) -> Result<(), Failure> {
    for (outer, sign) in bound.outer.into_iter().zip([-1., 1.]) {
        let sign = I::point(sign);
        let sweep::BodyBoundary::Separated(d) = outer.enclose_body(c, pose, coords, work)? else {
            return Err(Failure::Boundary);
        };
        // 车型的实体车底可能高于前杠参考点，查询前杠时另核其外缘。
        if (sign * d).lo <= 0.
            || front.is_some_and(|p| (sign * (p.lateral - outer.lateral(c, p.station))).lo <= 0.)
        {
            return Err(Failure::Boundary);
        }
    }
    Ok(())
}

// 两侧均使用含等号的必要条件，相切可进入扫过外包。此处不判定实际占用。
pub(super) fn body_regions(
    c: &Candidate<'_, '_>,
    bound: &Bound,
    pose: &Pose,
    coords: Vector,
    work: &mut Work,
) -> Result<[Option<Vector>; 2], Failure> {
    let seam = &bound.crossing;
    let mut regions = [None; 2];
    match seam.line.enclose_body(c, pose, coords, work)? {
        sweep::BodyBoundary::Separated(offset) => {
            let side = HandoffLaneSide::of(seam.sign * offset).ok_or(Failure::Boundary)?;
            regions[side as usize] = Some(coords);
        }
        sweep::BodyBoundary::MayTouch {
            rear_offset,
            coefficients,
            ..
        } => {
            for (region, sign) in regions.iter_mut().zip([-seam.sign, seam.sign]) {
                *region = side_coordinates(
                    sign * rear_offset,
                    coefficients.map(|k| sign * k),
                    coords,
                    work,
                )?;
            }
        }
    }
    Ok(regions)
}

pub(super) fn map_side(
    revision: &SharedNetworkRevision,
    q: &HandoffQuery,
    bound: &Bound,
    i: usize,
    s: I,
    work: &mut Work,
) -> Result<StationCandidates, Failure> {
    work.charge()?;
    match revision.station_mapping().in_station_range(
        q.corridor,
        nanometres(s)?,
        bound.source_target[i],
    ) {
        StationQuery::Complete(hits) => Ok(hits),
        _ => Err(Failure::Projection),
    }
}

fn stations(
    c: &Candidate<'_, '_>,
    bound: &Bound,
    shape: BodyEnvelope,
    range: [u64; 2],
    reserved: bool,
    work: &mut Work,
) -> Result<Stations, Failure> {
    work.charge()?;
    let motion = match c.trace.piece(&c.surface, range, work)? {
        Piece::Join {
            at, smooth: true, ..
        } => {
            let mut a = stations(c, bound, shape, [range[0], at], reserved, work)?;
            let b = stations(c, bound, shape, [at, range[1]], reserved, work)?;
            if a.front_side != b.front_side {
                if !reserved {
                    return Err(Failure::Boundary);
                }
                a.front_side = None;
            }
            a.front = a.front.hull(b.front);
            a.pose = a.pose.hull(b.pose);
            for (first, second) in a.sides.iter_mut().zip(b.sides) {
                match (first, second) {
                    (Some(x), Some(y)) => {
                        x.enclosure = x.enclosure.hull(y.enclosure);
                        x.witness = x.witness.hull(y.witness);
                    }
                    (None, None) => {}
                    _ => return Err(Failure::Boundary),
                }
            }
            return Ok(a);
        }
        Piece::Join { .. } => return Err(Failure::SteeringSeam),
        Piece::Motion(m) => m,
    };
    let pose = c.pose(&motion, work)?;
    let coords = shape.ranges();
    let front = c
        .surface
        .project(pose.wheel(shape.front_from_rear_m, 0.), work)?;
    within_outer(c, bound, &pose, coords, Some(front), work)?;
    let seam = &bound.crossing;
    let signed = |p: Projection| seam.sign * (p.lateral - seam.line.lateral(c, p.station));
    let front_side = HandoffLaneSide::of(signed(front));
    if front_side.is_none() && !reserved {
        return Err(Failure::Boundary);
    }
    let regions = body_regions(c, bound, &pose, coords, work)?;
    let mut witnesses = [None; 2];
    let [long, lat, height] = coords;
    for a in [long.lo, long.hi, long.lo.midpoint(long.hi)] {
        for b in [lat.lo, lat.hi] {
            for z in [height.lo, height.hi] {
                let p = pose.point(I::point(a), I::point(b), I::point(z));
                let projected = c.surface.project(horizontal(p), work)?;
                if let Some(side) = HandoffLaneSide::of(signed(projected)) {
                    witnesses[side as usize].get_or_insert(projected.station);
                }
            }
        }
        if (0..2).all(|i| regions[i].is_some() == witnesses[i].is_some()) {
            break;
        }
    }
    let mut sides = [None; 2];
    let rear = c.surface.project(horizontal(pose.rear), work)?;
    for i in 0..2 {
        match (regions[i], witnesses[i]) {
            (Some(coords), Some(witness)) => {
                let enclosure = station_enclosure(c, &pose, coords, rear, work)?;
                sides[i] = Some(Side { enclosure, witness });
            }
            (None, None) => {}
            _ => return Err(Failure::Boundary),
        }
    }
    Ok(Stations {
        pose: pose.bounds(),
        front_side,
        front: front.station,
        sides,
    })
}

impl SharedNetworkRevision {
    /// 在同一候选的后轴站位闭区间上求前杠位置及两侧车身证据。前杠触及共边、
    /// 侧别变化、映射不完整或超出逻辑预算时不返回完整定位。
    #[must_use]
    pub fn handoff_footprint(
        &self,
        q: &HandoffQuery,
        window: HandoffWindow,
        body: BodyEnvelope,
        rear_stations_nm: [u64; 2],
        work_limit: u32,
    ) -> HandoffFootprintReport {
        let (result, work) =
            self.handoff_footprint_and_pose(q, window, body, rear_stations_nm, work_limit);
        HandoffFootprintReport {
            footprint: result.map(|(footprint, _)| footprint),
            work,
        }
    }

    /// 从同一次车体求值读回前杠、两侧占用和后轴参考点及三轴的分量界。
    /// 返回值的第二项是实际工作数；跨平滑接缝时姿态界同时覆盖两侧。
    pub fn handoff_footprint_and_pose(
        &self,
        q: &HandoffQuery,
        window: HandoffWindow,
        body: BodyEnvelope,
        rear_stations_nm: [u64; 2],
        work_limit: u32,
    ) -> (Result<(HandoffFootprint, PoseBounds), Failure>, u32) {
        self.footprint_and_pose(q, window, body, rear_stations_nm, work_limit, false)
    }

    /// 已持有组合预约的轨迹可在前杠侧别未定时保留源游标。两侧均须有实体见证，
    /// 前杠在各侧须唯一对应已登记边且整个站位外包在允许窗口内；此结果不能授予预约。
    pub fn reserved_handoff_footprint_and_pose(
        &self,
        q: &HandoffQuery,
        window: HandoffWindow,
        body: BodyEnvelope,
        rear_stations_nm: [u64; 2],
        work_limit: u32,
    ) -> (Result<(HandoffFootprint, PoseBounds), Failure>, u32) {
        self.footprint_and_pose(q, window, body, rear_stations_nm, work_limit, true)
    }

    fn footprint_and_pose(
        &self,
        q: &HandoffQuery,
        window: HandoffWindow,
        body: BodyEnvelope,
        rear_stations_nm: [u64; 2],
        work_limit: u32,
        reserved: bool,
    ) -> (Result<(HandoffFootprint, PoseBounds), Failure>, u32) {
        let mut work = Work::new(work_limit.min(SWEEP_WORK_LIMIT));
        let mut run = || {
            let offsets = body_side::body_offsets(q, body, rear_stations_nm)?;
            let bound = bind(self, q, window, &mut work)?;
            let query = bound.query(q);
            let c = Candidate::bind(self, &query, &mut work)?;
            let positions = stations(&c, &bound, body, offsets, reserved, &mut work)?;
            let mut address = |i, s| map_side(self, q, &bound, i, s, &mut work);
            let front_side = positions.front_side.unwrap_or(HandoffLaneSide::Source);
            let front = address(front_side as usize, positions.front)?;
            let possible_target_front = if positions.front_side.is_none() {
                let range = front.stations_nm();
                let [low, high] = window.stations_nm()?;
                let target = address(HandoffLaneSide::Target as usize, positions.front)?;
                let mut target_addresses = target.addresses();
                let (Some(target), None) = (target_addresses.next(), target_addresses.next())
                else {
                    return Err(Failure::Boundary);
                };
                if positions.sides.iter().any(Option::is_none)
                    || range[0] < low
                    || range[1] > high
                    || !front.addresses().map(|a| a.edge).eq([window.edges[0]])
                    || target.edge != window.edges[1]
                {
                    return Err(Failure::Boundary);
                }
                Some(target)
            } else {
                None
            };
            let mut sides = [None, None];
            for (i, side) in positions.sides.into_iter().enumerate() {
                if let Some(side) = side {
                    sides[i] = Some(HandoffSideFootprint {
                        enclosure: address(i, side.enclosure)?,
                        witness: address(i, side.witness)?,
                    });
                }
            }
            Ok((
                HandoffFootprint {
                    front_side,
                    front,
                    possible_target_front,
                    sides,
                },
                positions.pose,
            ))
        };
        let footprint = run();
        (footprint, work.used)
    }
}
