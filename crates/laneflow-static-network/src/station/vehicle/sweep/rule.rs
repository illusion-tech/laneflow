use super::*;
use crate::station::vehicle::pose::PoseRate;
use crate::station::vehicle::surface::{Projection, dot};

fn anchors(e: &BoundaryExtent) -> [I; 2] {
    std::array::from_fn(|i| {
        I::meters(e.stations_nm[i]).with_error(I::meters(u64::from(e.errors_nm[i])).hi)
    })
}
fn overlaps(s: I, [lo, hi]: [I; 2]) -> bool {
    s.hi >= lo.lo && s.lo < hi.hi
}
fn covers(s: I, [lo, hi]: [I; 2]) -> bool {
    s.lo >= lo.hi && s.hi < hi.lo
}

enum Permission<'a> {
    Allow,
    Deny(Option<&'a str>),
    Unknown,
}
fn permission<'a>(
    s: Seam<'a>,
    station: I,
    direction: u8,
    work: &mut Work,
) -> Result<Permission<'a>, Failure> {
    let mut possible_allow = false;
    let mut possible_deny = false;
    let mut allow = false;
    for e in s.extents {
        work.charge()?;
        if let Some(BoundaryPayload::Rule { directions, effect }) = e.payload
            && directions & direction != 0
        {
            let a = anchors(e);
            if overlaps(station, a) {
                if effect == 2 {
                    if covers(station, a) {
                        return Ok(Permission::Deny(Some(e.local_key())));
                    }
                    possible_deny = true;
                } else {
                    possible_allow = true;
                    allow |= covers(station, a);
                }
            }
        }
    }
    if !possible_allow {
        return Ok(Permission::Deny(None));
    }
    if possible_deny {
        return Ok(Permission::Unknown);
    }
    if allow {
        return Ok(Permission::Allow);
    }
    // 同名录中的规则按局部键排列。每轮向右扩张已经证明连续的允许并集；
    // 两行共用同一锚点时，其半开区间在那个截面交接，即使站位有误差也没有空隙。
    let mut reach = station.lo;
    let mut end_anchor = None;
    loop {
        let previous = end_anchor;
        for e in s.extents {
            work.charge()?;
            if let Some(BoundaryPayload::Rule {
                directions,
                effect: 1,
            }) = e.payload
                && directions & direction != 0
            {
                let [lo, hi] = anchors(e);
                if (lo.hi <= reach || end_anchor == Some(e.stations_nm[0])) && hi.lo > reach {
                    reach = hi.lo;
                    end_anchor = Some(e.stations_nm[1]);
                    if reach > station.hi {
                        return Ok(Permission::Allow);
                    }
                }
            }
        }
        if end_anchor == previous {
            return Ok(Permission::Unknown);
        }
    }
}

// 车身点到标线的有符号横向偏移 F(P) = d(P) - d缝(s(P)) 的世界水平梯度。
fn gradient(p: Projection, slope: I) -> Horizontal {
    std::array::from_fn(|i| p.normal[i] - slope * p.station_gradient[i])
}

// 从后轴到任一真实触点积分 ∇F。触点的三个车体坐标 x 满足
// 0 ∈ F(后轴)+Σkᵢxᵢ，其中 kᵢ 包住整条积分线上的 ∇F·车体轴ᵢ。
// 逐个解出可收紧的坐标，各做一次区间交；保留下来的范围包含全部可能触点。
fn touch_coordinates(
    offset: I,
    coefficients: Vector,
    mut coords: Vector,
    work: &mut Work,
) -> Result<Option<Vector>, Failure> {
    let signed = (0..3).fold(offset, |sum, i| sum + coefficients[i] * coords[i]);
    if signed.lo > 0. || signed.hi < 0. {
        return Ok(None);
    }
    for (i, coefficient) in coefficients.into_iter().enumerate() {
        if coefficient.lo <= 0. && coefficient.hi >= 0. {
            continue;
        }
        work.charge()?;
        let residual = (0..3)
            .filter(|&j| j != i)
            .fold(offset, |sum, j| sum + coefficients[j] * coords[j]);
        let Some(bound) = coords[i].intersect(-residual / coefficient) else {
            return Ok(None);
        };
        coords[i] = bound;
    }
    Ok(Some(coords))
}

fn directions(rate: &PoseRate, grad: Horizontal, coords: Vector) -> u8 {
    let mut signed = dot(grad, horizontal(rate.rear));
    for (coordinate, derivative) in coords.into_iter().zip([rate.forward, rate.left, rate.up]) {
        signed = signed + coordinate * dot(grad, horizontal(derivative));
    }
    if signed.hi < 0. {
        1
    } else if signed.lo > 0. {
        2
    } else {
        3
    }
}

fn allowed<'a>(
    s: Seam<'a>,
    station: I,
    mask: u8,
    work: &mut Work,
) -> Result<Permission<'a>, Failure> {
    let mut any_allow = false;
    let mut any_deny = false;
    let mut key = None;
    for bit in [1, 2] {
        if mask & bit != 0 {
            match permission(s, station, bit, work)? {
                Permission::Unknown => return Ok(Permission::Unknown),
                Permission::Allow => any_allow = true,
                Permission::Deny(k) => {
                    any_deny = true;
                    key = key.or(k);
                }
            }
        }
    }
    Ok(match (any_allow, any_deny) {
        (true, false) => Permission::Allow,
        (false, true) => Permission::Deny(key),
        _ => Permission::Unknown,
    })
}

pub(super) fn check<'a>(
    c: &Candidate<'a, '_>,
    body: &Body<'_>,
    s: Seam<'a>,
    range: [u64; 2],
    work: &mut Work,
) -> Result<Verdict<'a>, Failure> {
    work.charge()?;
    let box_xz = horizontal(body.world());
    let origin = horizontal(body.pose.rear);
    let p = c
        .surface
        .project(std::array::from_fn(|i| box_xz[i].hull(origin[i])), work)?;
    let signed = p.lateral - s.lateral(c, p.station);
    if signed.lo > 0. || signed.hi < 0. {
        return Ok(Verdict::Clear(None));
    }
    // 世界轴向包围盒的角可能根本不在车身内。在整盒上求横向坐标梯度，
    // 把车体三个方向分别投到该梯度，再由后轴点积分到每个真实车身点。
    let rear = c.surface.project(horizontal(body.pose.rear), work)?;
    let slope = s.slope(c);
    let grad = gradient(p, slope);
    let coefficients =
        [body.pose.forward, body.pose.left, body.pose.up].map(|axis| dot(grad, horizontal(axis)));
    let ranges = body.shape.ranges();
    let Some(touch) = touch_coordinates(
        rear.lateral - s.lateral(c, rear.station),
        coefficients,
        ranges,
        work,
    )?
    else {
        return Ok(Verdict::Clear(None));
    };
    let [a, b, z] = touch;
    // 用同一组可能触点同时求站位和运动方向，再查该站位的单向许可。
    let region = if touch == ranges {
        p
    } else {
        c.surface
            .project(horizontal(body.pose.point(a, b, z)), work)?
    };
    let grad = gradient(region, slope);
    let (h, dh) = body.motion.heading()?;
    let rate = body.pose.rates(c.query.axles, body.motion.v, h, dh)?;
    let mask = directions(&rate, grad, touch);
    if matches!(allowed(s, region.station, mask, work)?, Permission::Allow) {
        return Ok(Verdict::Clear(None));
    }
    // 命名禁止区间的中点若确定在车身内，可直接在该触点核对禁止方向。
    let [[rear, front], [right, left], [bottom, top]] = body.shape.extents();
    for e in s.extents {
        work.charge()?;
        if let Some(BoundaryPayload::Rule {
            directions: forbidden,
            effect: 2,
        }) = e.payload
            && forbidden & mask != 0
            && overlaps(region.station, anchors(e))
        {
            let nm = e.stations_nm[0] + (e.stations_nm[1] - e.stations_nm[0]) / 2;
            let station = I::meters(nm);
            if !covers(station, anchors(e)) {
                continue;
            }
            let (center, h) = obstacle::frame(c, nm, 0, work)?;
            let point = crate::station::vehicle::surface::shifted(
                horizontal(center),
                [h[1], -h[0]],
                s.lateral(c, station),
            );
            let t = horizontal(body.pose.forward);
            let b = horizontal(body.pose.left);
            let det = |v: Horizontal, w: Horizontal| v[1] * w[0] - v[0] * w[1];
            for z in [bottom, top] {
                let origin = horizontal(body.vertex([0., 0., z]));
                let delta = std::array::from_fn(|i| point[i] - origin[i]);
                let a = det(delta, b) / body.pose.up[1];
                let y = det(t, delta) / body.pose.up[1];
                if a.lo >= rear && a.hi <= front && y.lo >= right && y.hi <= left {
                    let local = mask & directions(&rate, grad, [a, y, I::point(z)]);
                    if local == 0 || forbidden & local != local {
                        continue;
                    }
                    return Ok(Verdict::Denied {
                        boundary: BoundaryHit {
                            key: Some(e.local_key()),
                            ..s.id
                        },
                        at_nm: range[0],
                        touch_station_m: station.bounds(),
                    });
                }
            }
        }
    }
    // 同一车身横线的两端分处标线两侧时，至少有一个真实触点。
    // 用该横线仍可能触线的部分，核对站位与方向。
    for a in [front, rear, (front + rear) * 0.5] {
        for z in [bottom, top] {
            work.charge()?;
            let endpoint = |b| horizontal(body.vertex([a, b, z]));
            let l = endpoint(left);
            let r = endpoint(right);
            let pl = c.surface.project(l, work)?;
            let pr = c.surface.project(r, work)?;
            let dl = pl.lateral - s.lateral(c, pl.station);
            let dr = pr.lateral - s.lateral(c, pr.station);
            if (dl.lo >= 0. && dr.hi <= 0.) || (dr.lo >= 0. && dl.hi <= 0.) {
                let coords = [I::point(a), touch[1], I::point(z)];
                let span = c.surface.project(
                    horizontal(body.pose.point(coords[0], coords[1], coords[2])),
                    work,
                )?;
                let local = mask & directions(&rate, gradient(span, slope), coords);
                if local != 0
                    && let Permission::Deny(key) = allowed(s, span.station, local, work)?
                {
                    return Ok(Verdict::Denied {
                        boundary: BoundaryHit { key, ..s.id },
                        at_nm: range[0],
                        touch_station_m: span.station.bounds(),
                    });
                }
            }
        }
    }
    Ok(Verdict::Unproven {
        range_nm: range,
        boundary: Some(s.id),
        reason: Failure::Boundary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn low_endpoint_is_included_and_high_endpoint_is_excluded() {
        let a = [I::point(12.), I::point(18.)];
        assert!(covers(a[0], a));
        assert!(!overlaps(a[1], a));
        let observation = I::point(18.).with_error(0.01);
        assert!(overlaps(observation, a) && !covers(observation, a));
    }
}
