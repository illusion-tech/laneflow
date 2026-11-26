//! 在一个已绑定的路面窗口内，竖直投影世界水平坐标。逐行检查所有可能的根。
use super::interval::Interval;
use super::{Failure, Work};
use crate::RoadSurfaceSpan;

type Curve = [[Interval; 6]; 3];
type Vector = [Interval; 3];
pub(super) type Horizontal = [Interval; 2];

pub(super) fn dot(a: Horizontal, b: Horizontal) -> Interval {
    a[0] * b[0] + a[1] * b[1]
}
pub(super) fn shifted(p: Horizontal, direction: Horizontal, amount: Interval) -> Horizontal {
    std::array::from_fn(|i| p[i] + direction[i] * amount)
}

pub(super) struct Patch<'a> {
    row: &'a RoadSurfaceSpan,
    length: Interval,
    polynomial: Curve,
}
impl<'a> Patch<'a> {
    pub fn new(row: &'a RoadSurfaceSpan) -> Self {
        let length = Interval::meters(row.stations[1].0 - row.stations[0].0);
        let [start, end] = row
            .jets
            .map(|jet| jet.map(|v| Interval::point(f64::from(v))));
        let n = Interval::point;
        // 先减两端位置，使较大的世界坐标不进入三至五阶系数。
        let polynomial = std::array::from_fn(|i| {
            let a = start[i];
            let b = start[i + 3] * length;
            let c = start[i + 6] * length.square() * n(0.5);
            let delta = end[i] - a - b - c;
            let slope = end[i + 3] * length - b - n(2.) * c;
            let bend = end[i + 6] * length.square() - n(2.) * c;
            [
                a,
                b,
                c,
                n(10.) * delta - n(4.) * slope + n(0.5) * bend,
                -n(15.) * delta + n(7.) * slope - bend,
                n(6.) * delta - n(3.) * slope + n(0.5) * bend,
            ]
        });
        Self {
            row,
            length,
            polynomial,
        }
    }

    pub fn derivative(&self, t: Interval, order: usize) -> Vector {
        let factors = match order {
            0 => [1., 1., 1., 1., 1., 1.],
            1 => [0., 1., 2., 3., 4., 5.],
            2 => [0., 0., 2., 6., 12., 20.],
            3 => [0., 0., 0., 6., 24., 60.],
            4 => [0., 0., 0., 0., 24., 120.],
            _ => unreachable!("LFCA certifies through the fourth derivative"),
        };
        std::array::from_fn(|axis| {
            let mut value = Interval::ZERO;
            for i in (order..=5).rev() {
                value = value * t + Interval::point(factors[i]) * self.polynomial[axis][i];
            }
            for _ in 0..order {
                value = value / self.length;
            }
            value.with_error(f64::from(self.row.derivative_errors[order]))
        })
    }

    fn offset(&self, t: Interval, p: Horizontal) -> (Vector, Vector, Horizontal) {
        let center = self.derivative(t, 0);
        let v = self.derivative(t, 1);
        let delta = [p[0] - center[0], p[1] - center[2]];
        (center, v, delta)
    }

    fn residual(&self, t: Interval, p: Horizontal) -> Interval {
        let (_, v, delta) = self.offset(t, p);
        dot(delta, [v[0], v[2]])
    }

    fn jacobian(&self, t: Interval, p: Horizontal) -> Interval {
        let (_, v, delta) = self.offset(t, p);
        let a = self.derivative(t, 2);
        v[0].square() + v[2].square() - dot(delta, [a[0], a[2]])
    }

    pub fn parameter(&self, station_nm: u64) -> Interval {
        let [(a, _), (b, _)] = self.row.stations;
        if station_nm == a {
            return Interval::ZERO;
        }
        if station_nm == b {
            return Interval::ONE;
        }
        Interval::meters(station_nm - a) / self.length
    }

    fn project(
        &self,
        mut t: Interval,
        p: Horizontal,
        work: &mut Work,
    ) -> Result<Option<Projection>, Failure> {
        let residual = self.residual(t, p);
        if residual.lo > 0. || residual.hi < 0. {
            return Ok(None);
        }
        // F=(P-C)·C' 在每个可能的根处向下穿零。行内 -F' 须严格为正；
        // 共端的位置和水平法向已由共享构建核对，因此跨行的零点也不能反向穿越。
        let mut jacobian = self.jacobian(t, p);
        if jacobian.lo <= 0. || !jacobian.finite() {
            return Err(Failure::Projection);
        }
        // 区间 Newton 排除不含根的部分。即使数值包络无法继续缩小，当前区间仍含全部根。
        for _ in 0..4 {
            work.charge()?;
            let center = Interval::point(t.lo + 0.5 * (t.hi - t.lo));
            let correction = self.residual(center, p) / (self.length * jacobian);
            let Some(next) = t.intersect(center + correction) else {
                return Ok(None);
            };
            let old_width = t.hi - t.lo;
            t = next;
            jacobian = jacobian
                .intersect(self.jacobian(t, p))
                .ok_or(Failure::Projection)?;
            if t.hi - t.lo >= old_width * 0.75 {
                break;
            }
        }
        let (center, v, delta) = self.offset(t, p);
        let speed = (v[0].square() + v[2].square()).sqrt();
        if speed.lo <= 0. {
            return Err(Failure::Projection);
        }
        let stations = self.row.stations;
        let station_error = Interval::meters(u64::from(stations[0].1.max(stations[1].1)));
        let result = Projection {
            station: (Interval::meters(stations[0].0) + self.length * t)
                .with_error(station_error.hi),
            lateral: dot(delta, [v[2], -v[0]]) / speed,
            height: center[1],
            // 隐函数 F(s,P)=0 给出 ds/dP=C'_xz/(-F_s)。辅助点可在路幅以外。
            gradient: [v[0], v[2]].map(|h| v[1] * h / jacobian),
            normal: [v[2] / speed, -v[0] / speed],
            station_gradient: [v[0] / jacobian, v[2] / jacobian],
        };
        if !result.station.finite()
            || !result.lateral.finite()
            || !result.height.finite()
            || !result.gradient.into_iter().all(Interval::finite)
        {
            return Err(Failure::Projection);
        }
        Ok(Some(result))
    }
}

#[derive(Clone, Copy)]
pub(super) struct Projection {
    pub station: Interval,
    pub lateral: Interval,
    pub height: Interval,
    pub gradient: Horizontal,
    pub normal: Horizontal,
    pub station_gradient: Horizontal,
}
impl Projection {
    fn hull(self, other: Self) -> Self {
        Self {
            station: self.station.hull(other.station),
            lateral: self.lateral.hull(other.lateral),
            height: self.height.hull(other.height),
            gradient: std::array::from_fn(|i| self.gradient[i].hull(other.gradient[i])),
            normal: std::array::from_fn(|i| self.normal[i].hull(other.normal[i])),
            station_gradient: std::array::from_fn(|i| {
                self.station_gradient[i].hull(other.station_gradient[i])
            }),
        }
    }
}

pub(super) struct Surface<'a> {
    pub rows: &'a [RoadSurfaceSpan],
    pub window: [u64; 2],
}
impl Surface<'_> {
    pub fn project(&self, p: Horizontal, work: &mut Work) -> Result<Projection, Failure> {
        work.charge()?; // 本次同层求高或投影。
        let mut total: Option<Projection> = None;
        for (i, row) in self.rows.iter().enumerate() {
            work.charge()?; // 每个访问的原语，包括被排除的行。
            let patch = Patch::new(row);
            let lo = patch.parameter(self.window[0].max(row.stations[0].0));
            let hi = patch.parameter(self.window[1].min(row.stations[1].0));
            if (i == 0 && patch.residual(lo, p).lo < 0.)
                || (i + 1 == self.rows.len() && patch.residual(hi, p).hi > 0.)
            {
                return Err(Failure::Projection);
            }
            let domain = lo
                .hull(hi)
                .intersect(Interval::UNIT)
                .ok_or(Failure::Projection)?;
            if let Some(hit) = patch.project(domain, p, work)? {
                total = Some(total.map_or(hit, |old| old.hull(hit)));
            }
        }
        let total = total.ok_or(Failure::Projection)?;
        // 在真实站位误差内仍必须位于指定的纵向窗口。
        if total.station.lo < Interval::meters(self.window[0]).hi
            || total.station.hi > Interval::meters(self.window[1]).lo
        {
            return Err(Failure::Projection);
        }
        Ok(total)
    }
}
