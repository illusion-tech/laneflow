use super::*;

/// 一个原候选参数区间内，车体对可能在侧方的边界实体的连续水平分离证据。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SideClearanceSegment<'a> {
    /// 自候选起点量取的参数闭区间，单位毫米。走廊候选按后轴站位差递增。
    pub range_mm: [u64; 2],
    /// 有关实体的水平间隔正下界和限制它的子区间。无有关实体时为 `None`。
    pub clearance: Option<Clearance<'a>>,
}

/// 全段侧向净空证据。此查询不发放轮迹、转向或跨越许可。
#[derive(Clone, Debug, PartialEq)]
pub struct SideClearanceReport<'a> {
    /// 按行驶方向连续覆盖整条轨迹的区间。任一处未证即返回错误。
    pub segments: Result<Vec<SideClearanceSegment<'a>>, Failure>,
    /// 绑定、分段、姿态、边界与实体查询的实际工作量。
    pub work: u32,
    /// 失败所在的候选参数闭区间，单位毫米。
    pub unproven_range_mm: Option<[u64; 2]>,
}

impl Clearance<'_> {
    /// 产生此下界的候选行程闭区间，向外取整到毫米。
    #[must_use]
    pub fn range_mm(&self) -> [u64; 2] {
        enclosing_mm(self.range_nm)
    }

    /// 已证净空的整数毫米下界。小于一毫米的正间隔取零，超过最大控制点的距离饱和。
    #[must_use]
    pub fn lower_millimetres(&self) -> u32 {
        super::super::lower_mm(self.lower_m)
    }
}

type Gap = ([u64; 2], Failure);

fn interval_clearance<'a>(
    c: &Candidate<'a, '_>,
    shape: BodyEnvelope,
    range: [u64; 2],
    depth: u8,
    work: &mut Work,
) -> Result<Option<Clearance<'a>>, Gap> {
    work.charge().map_err(|e| (range, e))?;
    let piece = c.trace.piece(&c.surface, range, work);
    let check = match piece {
        Ok(Piece::Join {
            at, smooth: true, ..
        }) => Ok(at),
        Ok(Piece::Join { .. }) => Err(Failure::SteeringSeam),
        Ok(Piece::Motion(m)) => {
            match check_motion(c, shape, &m, range, ClearanceMetric::Side, work) {
                Ok(Verdict::Clear(value)) => return Ok(value),
                Ok(Verdict::Unproven { reason, .. }) | Err(reason) => Err(reason),
                Ok(Verdict::Contact { .. } | Verdict::Denied { .. }) => {
                    return Err((range, Failure::Boundary));
                }
            }
        }
        Err(e) => Err(e),
    };
    let (mid, depth) = match check {
        Ok(at) => (at, depth),
        Err(Failure::Boundary | Failure::Pose | Failure::Projection)
            if depth < 20 && range[1] - range[0] > 1 =>
        {
            (range[0] + (range[1] - range[0]) / 2, depth + 1)
        }
        Err(e) => return Err((range, e)),
    };
    let a = interval_clearance(c, shape, [range[0], mid], depth, work)?;
    let b = interval_clearance(c, shape, [mid, range[1]], depth, work)?;
    Ok(nearer(a, b))
}

pub(in crate::station::vehicle) fn side_clearance_segments<'a>(
    network: &'a SharedNetworkRevision,
    query: &TrackQuery<'_>,
    body: BodyEnvelope,
    max_span_mm: u64,
    work: &mut Work,
) -> Result<Vec<SideClearanceSegment<'a>>, Gap> {
    let length =
        super::super::mm_to_nm(query.trace.length_mm()).map_err(|reason| ([0, 0], reason))?;
    let max_span_nm = max_span_mm.saturating_mul(1_000_000);
    let full = [0, length];
    if max_span_mm == 0 || !body.valid(query.axles.wheelbase_m) {
        return Err((full, Failure::InvalidInput));
    }
    let c = Candidate::bind(network, query, work).map_err(|e| (full, e))?;
    let mut segments = Vec::new();
    let mut start: u64 = 0;
    loop {
        let end = start.saturating_add(max_span_nm).min(length);
        let range = [start, end];
        let clearance = interval_clearance(&c, body, range, 0, work)?;
        segments
            .try_reserve(1)
            .map_err(|_| (range, Failure::Unavailable))?;
        segments.push(SideClearanceSegment {
            range_mm: enclosing_mm(range),
            clearance,
        });
        if end == length {
            return Ok(segments);
        }
        start = end;
    }
}

impl<'a> SideClearanceReport<'a> {
    pub(in crate::station::vehicle) fn finish(
        result: Result<Vec<SideClearanceSegment<'a>>, Gap>,
        work: Work,
    ) -> Self {
        let unproven_range_mm = result.as_ref().err().map(|(range, _)| enclosing_mm(*range));
        Self {
            segments: result.map_err(|(_, e)| e),
            work: work.used,
            unproven_range_mm,
        }
    }
}

impl SharedNetworkRevision {
    /// 从轨迹起点按 `max_span_mm` 等长分段，末段可较短；每段覆盖完整车身。
    /// 实体在竖直方向或车身水平前向轴上已证全段分离时，免去它的侧向限制。
    /// 其余实体须有正的水平分离下界。曲线接缝和局部未证区间在共用预算内细分。
    /// 使用结果前须另经同一候选、姿态和车型的 `vehicle_geometry` 完整准入。
    #[must_use]
    pub fn vehicle_side_clearances(
        &self,
        query: &TrackQuery<'_>,
        body: BodyEnvelope,
        max_span_mm: u64,
        work_limit: u32,
    ) -> SideClearanceReport<'_> {
        let mut work = Work::new(work_limit.min(SWEEP_WORK_LIMIT));
        if super::super::mm_to_nm(query.trace.length_mm()).is_err() {
            return SideClearanceReport {
                segments: Err(Failure::InvalidInput),
                work: 0,
                unproven_range_mm: Some([0, query.trace.length_mm()]),
            };
        }
        let result = side_clearance_segments(self, query, body, max_span_mm, &mut work);
        SideClearanceReport::finish(result, work)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use laneflow_static_contract::RoadCorridorOrdinal;

    #[test]
    fn millimetres_round_the_binary_bound_down_before_saturation() {
        for (lower_m, expected) in [
            (f64::from_bits(1), 0),
            (0.001_f64.next_down(), 0),
            (0.001, 1),
            // 0.009 米的 binary64 值小于 9 毫米，乘 1000 后浮点数会舍入到 9。
            (0.009, 8),
            (0.009_f64.next_up(), 9),
            (1_f64.next_down(), 999),
            (1., 1_000),
            (2_f64.powi(52), u32::MAX),
            (f64::MAX, u32::MAX),
        ] {
            let evidence = Clearance {
                lower_m,
                range_nm: [0, 1],
                boundary: BoundaryHit {
                    owner: BoundaryOwner::Corridor(RoadCorridorOrdinal::from_raw(0)),
                    left_index: 0,
                    key: Some("post"),
                },
            };
            assert_eq!(evidence.lower_millimetres(), expected);
        }
    }
}
