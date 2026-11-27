//! 从同一车体姿态定位前杠和车身在两侧物理边上的范围。
use super::*;
use crate::StationCandidates;
use crate::station::vehicle::pose::Vector;

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
    /// 在整个查询域内前保险杠中点所在的一侧。
    pub front_side: HandoffLaneSide,
    /// 前保险杠中点在该侧独立边链上的可能位置。
    pub front: StationCandidates,
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
    front_side: HandoffLaneSide,
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
fn station_enclosure(
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

fn stations(
    c: &Candidate<'_, '_>,
    bound: &Bound,
    shape: BodyEnvelope,
    range: [u64; 2],
    work: &mut Work,
) -> Result<Stations, Failure> {
    work.charge()?;
    let motion = match c.trace.piece(&c.surface, range, work)? {
        Piece::Join {
            at, smooth: true, ..
        } => {
            let mut a = stations(c, bound, shape, [range[0], at], work)?;
            let b = stations(c, bound, shape, [at, range[1]], work)?;
            if a.front_side != b.front_side {
                return Err(Failure::Boundary);
            }
            a.front = a.front.hull(b.front);
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
    // 只在整车与前杠参考点均留在这对车道内时返回映射。
    for (outer, sign) in bound.outer.into_iter().zip([-1., 1.]) {
        let sign = I::point(sign);
        let sweep::BodyBoundary::Separated(d) = outer.enclose_body(c, &pose, coords, work)? else {
            return Err(Failure::Boundary);
        };
        if (sign * d).lo <= 0.
            || (sign * (front.lateral - outer.lateral(c, front.station))).lo <= 0.
        {
            return Err(Failure::Boundary);
        }
    }
    let seam = &bound.crossing;
    let signed = |p: Projection| seam.sign * (p.lateral - seam.line.lateral(c, p.station));
    let front_side = HandoffLaneSide::of(signed(front)).ok_or(Failure::Boundary)?;
    let mut regions = [None; 2];
    match seam.line.enclose_body(c, &pose, coords, work)? {
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
        let mut work = Work::new(work_limit.min(SWEEP_WORK_LIMIT));
        let mut run = || {
            let offsets = body_side::body_offsets(q, body, rear_stations_nm)?;
            let bound = bind(self, q, window, &mut work)?;
            let query = bound.query(q);
            let c = Candidate::bind(self, &query, &mut work)?;
            let positions = stations(&c, &bound, body, offsets, &mut work)?;
            let mut address = |i: usize, s: I| {
                work.charge()?;
                match self.station_mapping().in_station_range(
                    q.corridor,
                    nanometres(s)?,
                    bound.source_target[i],
                ) {
                    StationQuery::Complete(hits) => Ok(hits),
                    _ => Err(Failure::Projection),
                }
            };
            let front = address(positions.front_side as usize, positions.front)?;
            let mut sides = [None, None];
            for (i, side) in positions.sides.into_iter().enumerate() {
                if let Some(side) = side {
                    sides[i] = Some(HandoffSideFootprint {
                        enclosure: address(i, side.enclosure)?,
                        witness: address(i, side.witness)?,
                    });
                }
            }
            Ok(HandoffFootprint {
                front_side: positions.front_side,
                front,
                sides,
            })
        };
        let footprint = run();
        HandoffFootprintReport {
            footprint,
            work: work.used,
        }
    }
}
