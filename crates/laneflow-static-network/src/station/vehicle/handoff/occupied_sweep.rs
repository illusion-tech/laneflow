//! 一段横移内车体扫过的两侧物理边范围。
use super::*;
use crate::StationCandidates;

/// 从源侧到目标侧的整段扫过外包和实际工作量。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HandoffSweepReport {
    /// `None` 表示整段已证该侧分离。非空范围可用于阻挡，并不证明其内任何
    /// 特定位置有车。整段或任一侧无法完整投影时返回错误。
    pub sides: Result<[Option<StationCandidates>; 2], Failure>,
    /// 包括绑定、分段、姿态、车身投影和边链映射。
    pub work: u32,
}

fn motion_regions(
    c: &Candidate<'_, '_>,
    bound: &Bound,
    shape: BodyEnvelope,
    motion: &Motion,
    work: &mut Work,
) -> Result<[Option<I>; 2], Failure> {
    let pose = c.pose(motion, work)?;
    let coords = shape.ranges();
    footprint::within_outer(c, bound, &pose, coords, None, work)?;
    let regions = footprint::body_regions(c, bound, &pose, coords, work)?;
    let rear = c.surface.project([pose.rear[0], pose.rear[2]], work)?;
    let mut sides = [None; 2];
    for (side, coords) in sides.iter_mut().zip(regions) {
        if let Some(coords) = coords {
            *side = Some(footprint::station_enclosure(c, &pose, coords, rear, work)?);
        }
    }
    Ok(sides)
}

fn visit(
    c: &Candidate<'_, '_>,
    bound: &Bound,
    shape: BodyEnvelope,
    range: [u64; 2],
    depth: u8,
    work: &mut Work,
) -> Result<[Option<I>; 2], Failure> {
    enum Span {
        Enclosed([Option<I>; 2]),
        Join(u64),
    }
    work.charge()?;
    let result = c
        .trace
        .piece(&c.surface, range, work)
        .and_then(|p| match p {
            Piece::Motion(motion) => {
                motion_regions(c, bound, shape, &motion, work).map(Span::Enclosed)
            }
            Piece::Join {
                at, smooth: true, ..
            } => Ok(Span::Join(at)),
            Piece::Join { .. } => Err(Failure::SteeringSeam),
        });
    let (mid, depth) = match result {
        Ok(Span::Enclosed(sides)) => return Ok(sides),
        Ok(Span::Join(mid)) => (mid, depth),
        Err(Failure::Boundary | Failure::Pose | Failure::Projection)
            if depth < 20 && range[1] - range[0] > 1 =>
        {
            (range[0] + (range[1] - range[0]) / 2, depth + 1)
        }
        Err(failure) => return Err(failure),
    };
    let left = visit(c, bound, shape, [range[0], mid], depth, work)?;
    let right = visit(c, bound, shape, [mid, range[1]], depth, work)?;
    Ok(std::array::from_fn(|i| match (left[i], right[i]) {
        (Some(a), Some(b)) => Some(a.hull(b)),
        (a, b) => a.or(b),
    }))
}

impl SharedNetworkRevision {
    /// 在数值递增的后轴站位闭区间上连续包住整车。车头交接、车尾清空及与共边
    /// 的可能相切均计入扫过。外包可以跨越源侧和目标侧各自的切点，每侧按行驶
    /// 方向最多返回八条边。定位拍末车身及严格侧别使用 [`Self::handoff_footprint`]。
    #[must_use]
    pub fn handoff_swept_occupancy(
        &self,
        q: &HandoffQuery,
        window: HandoffWindow,
        body: BodyEnvelope,
        rear: HandoffStationRange,
        work_limit: u32,
    ) -> HandoffSweepReport {
        let mut work = Work::new(work_limit.min(SWEEP_WORK_LIMIT));
        let mut run = || {
            let offsets = body_side::body_offsets(q, body, rear.0)?;
            let bound = bind(self, q, window, &mut work)?;
            let query = bound.query(q);
            let c = Candidate::bind(self, &query, &mut work)?;
            let stations = visit(&c, &bound, body, offsets, 0, &mut work)?;
            let mut sides = [None, None];
            for (i, station) in stations.into_iter().enumerate() {
                if let Some(s) = station {
                    sides[i] = Some(footprint::map_side(self, q, &bound, i, s, &mut work)?);
                }
            }
            Ok(sides)
        };
        let sides = run();
        HandoffSweepReport {
            sides,
            work: work.used,
        }
    }
}
