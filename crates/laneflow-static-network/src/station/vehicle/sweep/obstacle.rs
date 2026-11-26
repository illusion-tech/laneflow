use super::*;
use crate::station::vehicle::surface::{Patch, dot, shifted};

fn microns(value: f64, error_nm: u32) -> I {
    (I::point(value) / I::point(1e6)).with_error(I::meters(u64::from(error_nm)).hi)
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
        let d = std::array::from_fn(|i| horizontal(body.pose.rear)[i] - self.origin[i]);
        let mut projected = dot(a, d);
        for (extent, axis) in
            body.shape
                .ranges()
                .into_iter()
                .zip([body.pose.forward, body.pose.left, body.pose.up])
        {
            projected = projected + extent * dot(a, horizontal(axis));
        }
        let end = self.length * dot(a, self.heading);
        let gap = projected - end.hull(I::ZERO);
        (gap.abs() / norm - self.radius).lo
    }
    fn clearance(&self, body: &Body<'_>) -> f64 {
        let y = body.world()[1];
        let mut best = (self.heights[0] - y).lo.max((y - self.heights[1]).lo);
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
        best
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
    let first = partition(rows, |r| r.stations[1].0 < lo, work)?;
    let mut frame: Option<(Vector, Horizontal)> = None;
    for r in &rows[first..] {
        work.charge()?;
        if r.stations[0].0 > hi {
            break;
        }
        let patch = Patch::new(r);
        let t = patch
            .parameter(lo.max(r.stations[0].0))
            .hull(patch.parameter(hi.min(r.stations[1].0)));
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
            let a = microns(f64::from(self.ends[0][i]), 0);
            let b = microns(f64::from(self.ends[1][i]), 0);
            (a + (b - a) * t).with_error(I::meters(u64::from(self.error)).hi)
        }))
    }
    fn check<'a>(
        &self,
        c: &Candidate<'a, '_>,
        body: &Body<'_>,
        s: Seam<'a>,
        id: BoundaryHit<'a>,
        range: [u64; 2],
        work: &mut Work,
    ) -> Result<Verdict<'a>, Failure> {
        let p = body.world();
        let world = c.surface.project(horizontal(p), work)?;
        // 用投影的局部梯度给出世界米制距离下界需要覆盖两者间整条线段。
        // 改用实体在世界坐标中的封闭外包作宽相分离；投影只用于证实车体内点。
        let [low, high] = self.station_bounds_nm;
        let first = partition(c.all_surface, |r| r.stations[1].0 < low, work)?;
        let mut lower = f64::INFINITY;
        for r in &c.all_surface[first..] {
            work.charge()?;
            if r.stations[0].0 > high {
                break;
            }
            let patch = Patch::new(r);
            let a = low.max(r.stations[0].0);
            let b = high.min(r.stations[1].0);
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
            let d = s.lateral(c, station) + l.hull(rr);
            let bounds = [
                pos[0] + d * v[1] / speed,
                pos[1] + bottom.hull(top),
                pos[2] - d * v[0] / speed,
            ];
            let distance = std::array::from_fn::<_, 3, _>(|i| (bounds[i] - p[i]).square());
            lower = lower.min((distance[0] + distance[1] + distance[2]).sqrt().lo);
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
                    let d = hit.lateral - s.lateral(c, hit.station);
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
                    s.lateral(c, st) + (l + r) * I::point(0.5),
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
    work: &mut Work,
) -> Result<Verdict<'a>, Failure> {
    work.charge()?;
    let id = BoundaryHit {
        key: Some(e.local_key()),
        ..s.id
    };
    let anchors = std::array::from_fn(|i| {
        I::meters(e.stations_nm[i]).with_error(I::meters(u64::from(e.errors_nm[i])).hi)
    });
    match e.payload.ok_or(Failure::Unavailable)? {
        BoundaryPayload::Guardrail {
            anchor_role,
            lateral_um,
            length_um,
            radius_um,
            heights_um,
            error_nm,
        } => {
            let side = usize::from(anchor_role == 2);
            let (center, h) = frame(c, e.stations_nm[side], u64::from(e.errors_nm[side]), work)?;
            let d = s.lateral(c, anchors[side]) + microns(f64::from(lateral_um), error_nm);
            let rail = Rail {
                origin: shifted(horizontal(center), [h[1], -h[0]], d),
                heading: if side == 1 { h.map(|v| -v) } else { h },
                length: microns(f64::from(length_um), error_nm),
                radius: microns(f64::from(radius_um), error_nm),
                heights: heights_um.map(|v| center[1] + microns(f64::from(v), error_nm)),
            };
            let lower = rail.clearance(body);
            if lower > 0. {
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
        BoundaryPayload::Curb { ends_um, error_nm } => Curb {
            ends: ends_um,
            error: error_nm,
            anchors,
            station_bounds_nm: [
                e.stations_nm[0].saturating_sub(u64::from(e.errors_nm[0])),
                e.stations_nm[1].saturating_add(u64::from(e.errors_nm[1])),
            ],
        }
        .check(c, body, s, id, range, work),
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
