//! 对一行站位基表内的偏移贝塞尔曲线证明速度和行程。积分分段只用于编译期求界。
use super::super::geometry::{OffsetInterval, Point3, StationInterval};
use super::super::native_curve::{Scalar, exact_point, norm};
use super::{MAX_PREFIX_ERROR_METERS, Result, UNPROVEN, interval, prefix_interpolation_error};

// 每根规范弦独立二分至多七层（最后一轮 128 个积分区间），各轮使用常量工作空间。
const MAX_INTEGRATION_LEVEL: u32 = 7;

pub(super) struct OffsetSpeed {
    // 每个坐标的一阶导数为 c₀ + t(c₁ + t c₂)。
    coefficients: [[Scalar; 3]; 3],
    row_start: Scalar,
    initial_offset: Scalar,
    offset_rate: Scalar,
}

impl OffsetSpeed {
    pub(super) fn new(
        controls: [Point3; 4],
        station: StationInterval,
        offset: OffsetInterval,
        station_rate: Scalar,
    ) -> Self {
        let s = Scalar::exact;
        let [a, b, c, d] = controls.map(exact_point);
        let coefficients = std::array::from_fn(|i| {
            let u = (b[i] - a[i]) * s(3.0);
            let v = (c[i] - b[i]) * s(3.0);
            let w = (d[i] - c[i]) * s(3.0);
            [u, s(2.0) * (v - u), (w - v) - (v - u)]
        });
        let dw = (s(offset.offset_end_meters) - s(offset.offset_start_meters))
            / (s(offset.station_end_meters).widen(offset.end_station_error_meters)
                - s(offset.station_start_meters));
        Self {
            coefficients,
            row_start: s(station.parameter_start),
            initial_offset: s(offset.offset_start_meters)
                + dw * (s(station.cumulative_start_meters).widen(station.station_error_meters)
                    - s(offset.station_start_meters)),
            offset_rate: dw * station_rate,
        }
    }

    // 在随水平切向转动的正交系内，偏移线速度分量为 (h − wθ′, y′, w′)。
    // 一行内 w 对参数为仿射函数。对速度模长求导时 w″ 为零。
    fn speed_and_derivative(&self, t: Scalar) -> Result<(Scalar, Scalar)> {
        let s = Scalar::exact;
        let [(x, xx, xxx), (y, yy, _), (z, zz, zzz)] = self
            .coefficients
            .map(|[a, b, c]| (a + t * (b + t * c), b + s(2.0) * t * c, s(2.0) * c));
        let horizontal_squared = x.square() + z.square();
        let horizontal = horizontal_squared.sqrt();
        if horizontal.lo <= 0.0 {
            return Err(UNPROVEN.into());
        }
        let yaw_rate = (z * xx - x * zz) / horizontal_squared;
        let width = self.initial_offset + self.offset_rate * (t - self.row_start);
        let along = horizontal - width * yaw_rate;
        let speed = norm([along, y, self.offset_rate]);
        if speed.lo <= 0.0 || !speed.hi.is_finite() {
            return Err(UNPROVEN.into());
        }
        let horizontal_rate = (x * xx + z * zz) / horizontal;
        let yaw_accel = (z * xxx - x * zzz) / horizontal_squared
            - s(2.0) * yaw_rate * horizontal_rate / horizontal;
        let along_rate = horizontal_rate - self.offset_rate * yaw_rate - width * yaw_accel;
        Ok((speed, (along * along_rate + y * yy) / speed))
    }

    // 中点公式的余项是速度导数在两半区间的带权积分，两侧权重面积各为 dt²/8，
    // 符号相反。因此只需导数的振幅。数学中点及所有算术舍入也包含在外包内。
    fn integral(&self, a: f64, b: f64, derivative: Scalar) -> Result<Scalar> {
        let s = Scalar::exact;
        let dt = s(b) - s(a);
        let mid = (s(a) + s(b)) * s(0.5);
        let (speed, _) = self.speed_and_derivative(mid)?;
        let error = dt.square() * s(0.125) * (s(derivative.hi) - s(derivative.lo));
        Ok((dt * speed).widen(error.hi))
    }

    pub(super) fn measure(&self, [a, b]: [f64; 2], share: f64) -> Result<(Scalar, Scalar)> {
        let s = Scalar::exact;
        let dt = s(b) - s(a);
        let (speed, derivative) = self.speed_and_derivative(interval(a, b))?;
        let mut travel = speed * dt;
        // 前缀本身太宽时由几何层拆弦，跳过即将丢弃的积分工作。
        if prefix_interpolation_error(dt, speed) > MAX_PREFIX_ERROR_METERS {
            return Ok((speed, travel));
        }
        for level in 0..=MAX_INTEGRATION_LEVEL {
            if (s(travel.hi) - s(travel.lo)).hi <= share {
                break;
            }
            let parts = 1 << level;
            let mut left = a;
            let mut sum = s(0.0);
            for i in 1..=parts {
                let right = if i == parts {
                    b
                } else {
                    a + (b - a) * (f64::from(i) / f64::from(parts))
                };
                if right <= left {
                    return Ok((speed, travel));
                }
                let slope = if parts == 1 {
                    derivative
                } else {
                    self.speed_and_derivative(interval(left, right))?.1
                };
                sum = sum + self.integral(left, right, slope)?;
                left = right;
            }
            let lo = travel.lo.max(sum.lo);
            let hi = travel.hi.min(sum.hi);
            if lo > hi {
                return Err(UNPROVEN.into());
            }
            travel = interval(lo, hi);
        }
        // 工作量到顶时保留已有的连续界；最终是否可发布由真实 f32 累计表判定。
        Ok((speed, travel))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quadrature_refines_within_one_chord_of_a_graded_widening_lane() {
        let station = StationInterval {
            parameter_start: 0.0,
            parameter_end: 1.0,
            cumulative_start_meters: 0.0,
            cumulative_end_meters: 60.0,
            station_error_meters: 0.0,
        };
        let offset = OffsetInterval {
            station_start_meters: 0.0,
            station_end_meters: 60.0,
            offset_start_meters: 0.0,
            offset_end_meters: 4.0,
            end_station_error_meters: 0.0,
        };
        let controls = [(0.0, 0.0), (1.0, 10.0), (2.0, 30.0), (3.0, 60.0)]
            .map(|(y, z)| Point3::try_new(0.0, y, z).unwrap());
        let curve = OffsetSpeed::new(controls, station, offset, Scalar::exact(60.0));
        let (a, b) = (0.25, 0.25 + 1.0 / 256.0);
        let budget = 0.00005 * (b - a);
        let (_, slope) = curve.speed_and_derivative(interval(a, b)).unwrap();
        let first = curve.integral(a, b, slope).unwrap();
        assert!(first.hi - first.lo > budget);

        // 三个正交速度分量为 30+60t、3、4。对 sqrt((30+60t)²+25) 直接求原函数。
        let primitive = |t: f64| {
            let v = 30.0 + 60.0 * t;
            (v * v.hypot(5.0) + 25.0 * (v / 5.0).asinh()) / 120.0
        };
        let expected = primitive(b) - primitive(a);
        let (_, refined) = curve.measure([a, b], budget).unwrap();
        assert!((refined.lo..=refined.hi).contains(&expected));
        assert!(refined.hi - refined.lo <= budget);
    }
}
