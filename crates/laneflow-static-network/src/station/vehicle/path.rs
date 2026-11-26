use super::interval::Interval as I;
use super::surface::{Horizontal, Patch, Surface, dot, shifted};
use super::{Failure, RearAxleLine, Work, partition};
use crate::RoadSurfaceSpan;

/// 给定两条随站位线性变化的左偏移，在两者间作五次横移。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CorridorTrace {
    /// 后轴起止站位，单位纳米。递减表示反向行驶。
    pub stations_nm: [u64; 2],
    /// 始端车道中心在候选起、止截面的左偏移（米）。
    pub from_m: [f64; 2],
    /// 目标车道中心在同一对截面的左偏移（米）。两组相同时沿该偏移行驶。
    pub to_m: [f64; 2],
}

/// 同一候选中的车轴、转向和车身共用的水平轨迹。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RearAxleTrace {
    /// 世界坐标中的直线，允许只查一个位置。
    Line(RearAxleLine),
    /// 根据该修订发布的路面多项式重建。
    Corridor(CorridorTrace),
}
impl RearAxleTrace {
    pub(super) fn length(self) -> u64 {
        match self {
            Self::Line(t) => t.length_nm,
            Self::Corridor(t) => t.stations_nm[0].abs_diff(t.stations_nm[1]),
        }
    }
}

pub(super) struct Motion {
    pub p: Horizontal,
    pub v: Horizontal,
    pub a: Horizontal,
    pub j: Horizontal,
}
impl Motion {
    pub fn heading(&self) -> Result<(Horizontal, Horizontal), Failure> {
        let speed = (self.v[0].square() + self.v[1].square()).sqrt();
        if speed.lo <= 0. || !speed.finite() {
            return Err(Failure::Pose);
        }
        let h = self.v.map(|v| v / speed);
        let yaw = (self.v[1] * self.a[0] - self.v[0] * self.a[1]) / speed.square();
        Ok((h, [h[1] * yaw, -h[0] * yaw]))
    }
}

// 按普通导数（不除阶乘）计算乘积和水平单位法向。由 H·H=1 与 v=|v|H 逐阶求导。
fn product(a: [I; 4], b: [I; 4]) -> [I; 4] {
    let n = I::point;
    [
        a[0] * b[0],
        a[1] * b[0] + a[0] * b[1],
        a[2] * b[0] + n(2.) * a[1] * b[1] + a[0] * b[2],
        a[3] * b[0] + n(3.) * (a[2] * b[1] + a[1] * b[2]) + a[0] * b[3],
    ]
}

fn motion(
    row: &RoadSurfaceSpan,
    bounds: [u64; 2],
    trace: CorridorTrace,
) -> Result<Motion, Failure> {
    let patch = Patch::new(row);
    let t = patch.parameter(bounds[0]).hull(patch.parameter(bounds[1]));
    let c: [Horizontal; 5] = std::array::from_fn(|k| {
        let d = patch.derivative(t, k);
        [d[0], d[2]]
    });
    let n = I::point;
    let signed = n(if trace.stations_nm[0] < trace.stations_nm[1] {
        1.
    } else {
        -1.
    });
    let duration = I::meters(trace.stations_nm[0].abs_diff(trace.stations_nm[1]));
    let u = I::meters(trace.stations_nm[0].abs_diff(bounds[0]))
        .hull(I::meters(trace.stations_nm[0].abs_diff(bounds[1])))
        / duration;
    let u = u.intersect(I::UNIT).ok_or(Failure::Binding)?;
    let v = I::ONE - u;
    let rate = signed / duration;
    let mix = |u: I| u * u * u * (n(10.) + u * (n(-15.) + n(6.) * u));
    // 横移权重单调，端值直接给出零阶的紧界。导数用因式分解抑制端点相消。
    let weight = [
        mix(n(u.lo))
            .hull(mix(n(u.hi)))
            .intersect(I::UNIT)
            .ok_or(Failure::Pose)?,
        n(30.) * u.square() * v.square() * rate,
        n(60.) * u * v * (I::ONE - n(2.) * u) * rate.square(),
        n(60.) * (I::ONE - n(6.) * u * v) * rate.square() * rate,
    ];
    let linear = |[a, b]: [f64; 2]| {
        let delta = n(b) - n(a);
        [n(a) + delta * u, delta * rate, I::ZERO, I::ZERO]
    };
    let from = linear(trace.from_m);
    let to = linear(trace.to_m);
    let offset = product(weight, std::array::from_fn(|i| to[i] - from[i]));
    let offset: [I; 4] = std::array::from_fn(|i| offset[i] + from[i]);
    let mut q = [c[0], c[1], c[2], c[3]];
    if offset.iter().any(|v| *v != I::ZERO) {
        let speed = (c[1][0].square() + c[1][1].square()).sqrt();
        if speed.lo <= 0. {
            return Err(Failure::Pose);
        }
        let h = c[1].map(|v| v / speed);
        let s1 = dot(h, c[2]);
        let h1 = shifted(c[2], h, -s1).map(|v| v / speed);
        let s2 = dot(h1, c[2]) + dot(h, c[3]);
        let h2 = shifted(shifted(c[3], h, -s2), h1, -n(2.) * s1).map(|v| v / speed);
        let s3 = dot(h2, c[2]) + n(2.) * dot(h1, c[3]) + dot(h, c[4]);
        let h3 = shifted(
            shifted(shifted(c[4], h, -s3), h1, -n(3.) * s2),
            h2,
            -n(3.) * s1,
        )
        .map(|v| v / speed);
        for (axis, sign) in [(0, I::ONE), (1, -I::ONE)] {
            let component = [h, h1, h2, h3].map(|v| v[1 - axis] * sign);
            for (d, extra) in q.iter_mut().zip(product(offset, component)) {
                d[axis] = d[axis] + extra;
            }
        }
    }
    Ok(Motion {
        p: q[0],
        v: q[1].map(|v| v * signed),
        a: q[2],
        j: q[3].map(|v| v * signed),
    })
}

pub(super) enum Path {
    Line {
        origin: Horizontal,
        heading: Horizontal,
    },
    Corridor(CorridorTrace),
}
fn constant_normal(row: &RoadSurfaceSpan) -> bool {
    let [start, end] = row.jets.map(|j| j.map(f64::from));
    let [x, z] = [start[3], start[5]];
    if [start, end]
        .into_iter()
        .any(|j| x * j[5] != z * j[3] || x * j[8] != z * j[6])
    {
        return false;
    }
    // 两个 f32 的积在 f64 中精确。TwoSum 保留两项之和的舍入尾数，
    // 以此核对 (end-start) 也平行于共同切向。
    let sum = |a: f64, b: f64| {
        let s = a + b;
        let v = s - a;
        (s, (a - (s - v)) + (b - v))
    };
    sum(x * end[2], z * start[0]) == sum(x * start[2], z * end[0])
}
impl Path {
    pub fn bind(trace: RearAxleTrace, window: [u64; 2]) -> Result<Self, Failure> {
        match trace {
            RearAxleTrace::Line(t) => {
                let scale = t.heading_xz[0].abs().max(t.heading_xz[1].abs());
                if scale == 0.
                    || !t
                        .start_xz_m
                        .into_iter()
                        .chain(t.heading_xz)
                        .all(f64::is_finite)
                {
                    return Err(Failure::InvalidInput);
                }
                let h = t.heading_xz.map(|v| I::point(v) / I::point(scale));
                let norm = (h[0].square() + h[1].square()).sqrt();
                Ok(Self::Line {
                    origin: t.start_xz_m.map(I::point),
                    heading: h.map(|v| v / norm),
                })
            }
            RearAxleTrace::Corridor(t) => {
                if t.stations_nm[0] == t.stations_nm[1]
                    || t.stations_nm
                        .iter()
                        .any(|s| !(window[0]..=window[1]).contains(s))
                    || !t.from_m.into_iter().chain(t.to_m).all(f64::is_finite)
                {
                    return Err(Failure::InvalidInput);
                }
                Ok(Self::Corridor(t))
            }
        }
    }

    pub fn piece(
        &self,
        surface: &Surface<'_>,
        range: [u64; 2],
        work: &mut Work,
    ) -> Result<Piece, Failure> {
        let Self::Corridor(t) = self else {
            let Self::Line { origin, heading } = self else {
                unreachable!()
            };
            return Ok(Piece::Motion(Motion {
                p: shifted(
                    *origin,
                    *heading,
                    I::meters(range[0]).hull(I::meters(range[1])),
                ),
                v: *heading,
                a: [I::ZERO; 2],
                j: [I::ZERO; 2],
            }));
        };
        let start = t.stations_nm[0];
        let increasing = start < t.stations_nm[1];
        let stations = range.map(|s| if increasing { start + s } else { start - s });
        let low = stations[0].min(stations[1]);
        let high = stations[0].max(stations[1]);
        let index = partition(surface.rows, |r| r.stations[1].0 <= low, work)?
            .min(surface.rows.len().saturating_sub(1));
        work.charge()?;
        let row = surface.rows.get(index).ok_or(Failure::Unavailable)?;
        if high > row.stations[1].0 {
            let last = partition(&surface.rows[index + 1..], |r| r.stations[1].0 < high, work)?
                + index
                + 1;
            let index = index + (last - index - 1) / 2;
            work.charge()?;
            let row = &surface.rows[index];
            let next = &surface.rows[index + 1];
            work.charge()?;
            let seam = row.stations[1].0;
            let offset_constant = t.from_m == t.to_m && t.from_m[0] == t.from_m[1];
            // 相同的水平一、二阶端导数给出共同的后轴位置、朝向和曲率。
            // 恒定偏移的曲率只依赖原线曲率和偏移（在 1-dκ>0 时）。
            // 横移中若偏移不为零且仍在改变，横移曲率还依赖法向的二阶导数。
            let smooth = smooth_join(row, next, offset_constant);
            return Ok(Piece::Join {
                at: start.abs_diff(seam),
                smooth,
                rows: index,
            });
        }
        Ok(Piece::Motion(motion(row, [low, high], *t)?))
    }
}
pub(super) fn smooth_join(
    row: &RoadSurfaceSpan,
    next: &RoadSurfaceSpan,
    offset_constant: bool,
) -> bool {
    let equal = [3, 5, 6, 8]
        .into_iter()
        .all(|k| row.jets[1][k] == next.jets[0][k]);
    let straight = constant_normal(row) && constant_normal(next);
    equal && (offset_constant || straight) || straight && offset_constant
}
pub(super) enum Piece {
    Motion(Motion),
    Join { at: u64, smooth: bool, rows: usize },
}

pub(super) fn sided(row: &RoadSurfaceSpan, at: u64, t: CorridorTrace) -> Result<Motion, Failure> {
    motion(row, [at, at], t)
}
