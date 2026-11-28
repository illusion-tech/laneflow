use super::*;
use crate::station::vehicle::surface::{Patch, dot, shifted};

fn millimetres(value: f64, error_mm: u32) -> I {
    (I::point(value) / I::point(1_000.)).with_error(I::from_millimetres(error_mm).hi)
}

// 从后轴投影起算，分别加入三个车体轴在待测水平轴上的投影。
fn body_span(body: &Body<'_>, axis: Horizontal, rear: I) -> I {
    let mut span = rear;
    for (extent, direction) in
        body.shape
            .ranges()
            .into_iter()
            .zip([body.pose.forward, body.pose.left, body.pose.up])
    {
        span = span + extent * dot(axis, horizontal(direction));
    }
    span
}
fn body_projection(body: &Body<'_>, origin: Horizontal, axis: Horizontal) -> I {
    let rear = horizontal(body.pose.rear);
    body_span(
        body,
        axis,
        dot(axis, std::array::from_fn(|i| rear[i] - origin[i])),
    )
}
struct Rail {
    origin: Horizontal,
    heading: Horizontal,
    length: I,
    radius: I,
    heights: [I; 2],
}
impl Rail {
    fn cap(&self) -> Horizontal {
        shifted(self.origin, self.heading, self.length)
    }

    // 对车身和整条有限轴线求有向投影。候选分离轴可以由近似值选取，
    // 所有用于判定的量（含轴长）重新用闭区间计算。
    fn gap(&self, body: &Body<'_>, axis: [f64; 2]) -> f64 {
        let a = axis.map(I::point);
        let norm = (a[0].square() + a[1].square()).sqrt();
        if norm.lo <= 0. || !norm.finite() {
            return f64::NEG_INFINITY;
        }
        let projected = body_projection(body, self.origin, a);
        let end = self.length * dot(a, self.heading);
        let gap = projected - end.hull(I::ZERO);
        (gap.abs() / norm - self.radius).lo
    }
    fn clearance(&self, body: &Body<'_>, metric: ClearanceMetric) -> Option<f64> {
        let y = body.world()[1];
        let vertical = (self.heights[0] - y).lo.max((y - self.heights[1]).lo);
        let mut best = match metric {
            ClearanceMetric::Spatial => vertical,
            ClearanceMetric::Side => {
                if vertical > 0. {
                    return None;
                }
                let f = horizontal(body.pose.forward);
                let ends = I::ZERO.hull(self.length * dot(f, self.heading));
                // 前向轴随整段姿态变化。端帽在该未归一化水平轴上的半径为 r·|f|。
                let norm = (f[0].square() + f[1].square()).sqrt();
                let separation = (body_projection(body, self.origin, f) - ends).abs();
                if (separation - self.radius * norm).lo > 0. {
                    return None;
                }
                0.
            }
        };
        let h = self.heading.map(mid);
        for a in [
            [1., 0.],
            [0., 1.],
            h,
            [h[1], -h[0]],
            horizontal(body.pose.forward).map(mid),
            horizontal(body.pose.left).map(mid),
        ] {
            best = best.max(self.gap(body, a));
        }
        let [aa, bb, zz] = body.shape.extents();
        for a in aa {
            for b in bb {
                for z in zz {
                    let corner = horizontal(body.vertex([a, b, z])).map(mid);
                    for cap in [self.origin, self.cap()] {
                        best = best.max(
                            self.gap(body, [corner[0] - mid(cap[0]), corner[1] - mid(cap[1])]),
                        );
                    }
                }
            }
        }
        Some(best)
    }
    fn contains(&self, p: Vector) -> bool {
        if p[1].lo < self.heights[0].hi || p[1].hi > self.heights[1].lo || self.radius.lo < 0. {
            return false;
        }
        let d = std::array::from_fn(|i| horizontal(p)[i] - self.origin[i]);
        // 任一固定轴线比例均给出距离上界，包括两个闭端面。
        let h = self.heading.map(mid);
        let f = ((mid(d[0]) * h[0] + mid(d[1]) * h[1]) / mid(self.length)).clamp(0., 1.);
        let offset = shifted(d, self.heading, -(self.length * I::point(f)));
        (offset[0] == I::ZERO && offset[1].abs().hi <= self.radius.lo)
            || (offset[1] == I::ZERO && offset[0].abs().hi <= self.radius.lo)
            || (offset[0].square() + offset[1].square()).hi <= self.radius.square().lo
    }
    fn witness(&self, body: &Body<'_>) -> Option<[f64; 3]> {
        let [aa, bb, zz] = body.shape.extents();
        for a in aa {
            for b in bb {
                for z in zz {
                    let q = [a, b, z];
                    if self.contains(body.vertex(q)) {
                        return Some(q);
                    }
                }
            }
        }
        // 端帽可碰到车身面的内部。用其中心和三个高度挑选车体内点，再核对同一点。
        for cap in [
            self.origin,
            self.cap(),
            shifted(self.origin, self.heading, self.length * I::point(0.5)),
        ] {
            for height in [
                mid(self.heights[0]),
                (mid(self.heights[0]) + mid(self.heights[1])) * 0.5,
                mid(self.heights[1]),
            ] {
                let target = [mid(cap[0]), height, mid(cap[1])];
                let q = body.nearest_local(target);
                if self.contains(body.vertex(q)) {
                    return Some(q);
                }
            }
        }
        None
    }
}

pub(super) fn frame(
    c: &Candidate<'_, '_>,
    nm: u64,
    err: u64,
    work: &mut Work,
) -> Result<(Vector, Horizontal), Failure> {
    let rows = c.all_surface;
    let lo = nm.saturating_sub(err);
    let hi = nm.saturating_add(err);
    let first = partition(rows, |r| r.stations_nm()[1].0 < lo, work)?;
    let mut frame: Option<(Vector, Horizontal)> = None;
    for r in &rows[first..] {
        work.charge()?;
        if r.stations_nm()[0].0 > hi {
            break;
        }
        let patch = Patch::new(r);
        let t = patch
            .parameter(lo.max(r.stations_nm()[0].0))
            .hull(patch.parameter(hi.min(r.stations_nm()[1].0)));
        let p = patch.derivative(t, 0);
        let v = horizontal(patch.derivative(t, 1));
        let norm = (v[0].square() + v[1].square()).sqrt();
        if norm.lo <= 0. {
            return Err(Failure::Projection);
        }
        let h = v.map(|x| x / norm);
        frame = Some(frame.map_or((p, h), |(old, oh)| {
            (
                std::array::from_fn(|i| old[i].hull(p[i])),
                std::array::from_fn(|i| oh[i].hull(h[i])),
            )
        }));
    }
    frame.ok_or(Failure::Unavailable)
}

struct Curb {
    ends: [[i32; 4]; 2],
    error: u32,
    anchors: [I; 2],
    station_bounds_nm: [u64; 2],
}
impl Curb {
    fn profile(&self, s: I) -> Option<[I; 4]> {
        let t = ((s - self.anchors[0]) / (self.anchors[1] - self.anchors[0])).intersect(I::UNIT)?;
        Some(std::array::from_fn(|i| {
            let a = millimetres(f64::from(self.ends[0][i]), 0);
            let b = millimetres(f64::from(self.ends[1][i]), 0);
            (a + (b - a) * t).with_error(I::from_millimetres(self.error).hi)
        }))
    }
    fn check<'a>(
        &self,
        c: &Candidate<'a, '_>,
        body: &Body<'_>,
        s: Seam<'a>,
        range: [u64; 2],
        metric: ClearanceMetric,
        work: &mut Work,
    ) -> Result<Verdict<'a>, Failure> {
        let id = s.id;
        let p = body.world();
        let world = c.surface.project(horizontal(p), work)?;
        // 车身和路缘都在世界坐标中求距离下界。路面投影用于后续接触见证。
        let [low, high] = self.station_bounds_nm;
        let first = partition(c.all_surface, |r| r.stations_nm()[1].0 < low, work)?;
        let mut lower = f64::INFINITY;
        let mut inspected = false;
        let mut relevant = false;
        for r in &c.all_surface[first..] {
            work.charge()?;
            if r.stations_nm()[0].0 > high {
                break;
            }
            let patch = Patch::new(r);
            let a = low.max(r.stations_nm()[0].0);
            let b = high.min(r.stations_nm()[1].0);
            let station = I::meters(a).hull(I::meters(b));
            let Some([l, rr, bottom, top]) = self.profile(station) else {
                continue;
            };
            let t = patch.parameter(a).hull(patch.parameter(b));
            let pos = patch.derivative(t, 0);
            let v = horizontal(patch.derivative(t, 1));
            let speed = (v[0].square() + v[1].square()).sqrt();
            if speed.lo <= 0. {
                return Err(Failure::Projection);
            }
            let d = s.line.lateral(c, station) + l.hull(rr);
            let normal = [v[1] / speed, -v[0] / speed];
            let bounds = [
                pos[0] + d * normal[0],
                pos[1] + bottom.hull(top),
                pos[2] + d * normal[1],
            ];
            inspected = true;
            let distance = std::array::from_fn::<_, 3, _>(|i| (bounds[i] - p[i]).square());
            let mut gap = match metric {
                ClearanceMetric::Spatial => (distance[0] + distance[1] + distance[2]).sqrt().lo,
                ClearanceMetric::Side => {
                    let f = horizontal(body.pose.forward);
                    let origin = horizontal(body.pose.rear);
                    let curb = dot(f, [bounds[0] - origin[0], bounds[2] - origin[1]]);
                    if (bounds[1] - p[1]).abs().lo > 0.
                        || (body_span(body, f, I::ZERO) - curb).abs().lo > 0.
                    {
                        continue;
                    }
                    (distance[0] + distance[2]).sqrt().lo
                }
            };
            relevant = true;
            // 轴对齐包围盒只要分离即可证明不接触。按净空选速还需比较斜向分离轴。
            if gap == 0. || metric == ClearanceMetric::Side {
                let middle = a + (b - a) / 2;
                let center = horizontal(patch.derivative(patch.parameter(middle), 0));
                let ds = (-I::meters(middle - a)).hull(I::meters(b - middle));
                // 从中点积分参考线沿固定轴的导数，包住整个路缘区间。
                // 对直路沿法向投影时，纵向长度不计入横向宽度。
                for direction in [
                    normal,
                    horizontal(body.pose.left),
                    horizontal(body.pose.forward),
                ] {
                    let axis = direction.map(|component| I::point(mid(component)));
                    let norm = (axis[0].square() + axis[1].square()).sqrt();
                    if norm.lo > 0. && norm.finite() {
                        let curb = ds * dot(axis, v) + d * dot(axis, normal);
                        let separation = body_projection(body, center, axis) - curb;
                        gap = gap.max((separation.abs() / norm).lo);
                    }
                }
            }
            lower = lower.min(gap);
        }
        if inspected && !relevant {
            return Ok(Verdict::Clear(None));
        }
        if lower > 0. && lower.is_finite() {
            return Ok(Verdict::Clear(Some(Clearance {
                lower_m: lower,
                boundary: id,
                range_nm: range,
            })));
        }
        if let Some(overlap) = world.station.intersect(I {
            lo: self.anchors[0].lo,
            hi: self.anchors[1].hi,
        }) {
            let check_point = |q, work: &mut Work| -> Result<Option<Verdict<'a>>, Failure> {
                work.charge()?;
                let p = body.vertex(q);
                let hit = c.surface.project(horizontal(p), work)?;
                if hit.station.lo >= self.anchors[0].hi
                    && hit.station.hi <= self.anchors[1].lo
                    && let Some([l, r, bottom, top]) = self.profile(hit.station)
                {
                    let d = hit.lateral - s.line.lateral(c, hit.station);
                    let h = p[1] - hit.height;
                    if d.lo >= r.hi && d.hi <= l.lo && h.lo >= bottom.hi && h.hi <= top.lo {
                        return Ok(Some(Verdict::Contact {
                            boundary: id,
                            at_nm: range[0],
                            body_point_m: q,
                        }));
                    }
                }
                Ok(None)
            };
            let nm = (mid(overlap) * 1e9).round() as u64;
            let st = I::meters(nm);
            if let Some([l, r, bottom, top]) = self.profile(st) {
                let (center, h) = frame(c, nm, 0, work)?;
                let point = shifted(
                    horizontal(center),
                    [h[1], -h[0]],
                    s.line.lateral(c, st) + (l + r) * I::point(0.5),
                );
                let q = body.nearest_local([
                    mid(point[0]),
                    mid(center[1] + (bottom + top) * I::point(0.5)),
                    mid(point[1]),
                ]);
                if let Some(hit) = check_point(q, work)? {
                    return Ok(hit);
                }
            }
            let [aa, bb, zz] = body.shape.extents();
            for a in aa {
                for b in bb {
                    for z in zz {
                        if let Some(hit) = check_point([a, b, z], work)? {
                            return Ok(hit);
                        }
                    }
                }
            }
        }
        Ok(Verdict::Unproven {
            range_nm: range,
            boundary: Some(id),
            reason: Failure::Boundary,
        })
    }
}

pub(super) fn check<'a>(
    c: &Candidate<'a, '_>,
    body: &Body<'_>,
    s: Seam<'a>,
    e: &'a BoundaryExtent,
    range: [u64; 2],
    metric: ClearanceMetric,
    work: &mut Work,
) -> Result<Verdict<'a>, Failure> {
    work.charge()?;
    let id = BoundaryHit {
        key: Some(e.local_key()),
        ..s.id
    };
    let anchors = std::array::from_fn(|i| {
        I::meters(e.stations_nm()[i]).with_error(I::meters(e.errors_nm()[i]).hi)
    });
    match e.payload.ok_or(Failure::Unavailable)? {
        BoundaryPayload::Guardrail {
            anchor_role,
            lateral_mm,
            length_mm,
            radius_mm,
            heights_mm,
            error_mm,
        } => {
            let side = usize::from(anchor_role == 2);
            let (center, h) = frame(c, e.stations_nm()[side], e.errors_nm()[side], work)?;
            let d = s.line.lateral(c, anchors[side]) + millimetres(f64::from(lateral_mm), error_mm);
            let rail = Rail {
                origin: shifted(horizontal(center), [h[1], -h[0]], d),
                heading: if side == 1 { h.map(|v| -v) } else { h },
                length: millimetres(f64::from(length_mm), error_mm),
                radius: millimetres(f64::from(radius_mm), error_mm),
                heights: heights_mm.map(|v| center[1] + millimetres(f64::from(v), error_mm)),
            };
            let Some(lower) = rail.clearance(body, metric) else {
                return Ok(Verdict::Clear(None));
            };
            if lower > 0. && lower.is_finite() {
                Ok(Verdict::Clear(Some(Clearance {
                    lower_m: lower,
                    boundary: id,
                    range_nm: range,
                })))
            } else if let Some(q) = rail.witness(body) {
                Ok(Verdict::Contact {
                    boundary: id,
                    at_nm: range[0],
                    body_point_m: q,
                })
            } else {
                Ok(Verdict::Unproven {
                    range_nm: range,
                    boundary: Some(id),
                    reason: Failure::Boundary,
                })
            }
        }
        BoundaryPayload::Curb { ends_mm, error_mm } => Curb {
            ends: ends_mm,
            error: error_mm,
            anchors,
            station_bounds_nm: [
                e.stations_nm()[0].saturating_sub(e.errors_nm()[0]),
                e.stations_nm()[1].saturating_add(e.errors_nm()[1]),
            ],
        }
        .check(c, body, Seam { id, ..s }, range, metric, work),
        BoundaryPayload::Rule { .. } => Err(Failure::Binding),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_cap_and_height_faces_are_closed() {
        let rail = Rail {
            origin: [I::ZERO; 2],
            heading: [I::ZERO, I::ONE],
            length: I::point(2.),
            radius: I::point(0.25),
            heights: [I::ZERO, I::ONE],
        };
        for q in [[0.25, 1., 0.], [0., 0., -0.25]] {
            assert!(rail.contains(q.map(I::point)));
        }
        for q in [[0.25_f64.next_up(), 0.5, 0.], [0., 1_f64.next_up(), 0.]] {
            assert!(!rail.contains(q.map(I::point)));
        }
    }
}
