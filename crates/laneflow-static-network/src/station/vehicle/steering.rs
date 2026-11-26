use super::interval::Interval as I;
use super::path::Motion;
use super::pose::Pose;
use super::surface::dot;
use super::{AxleDimensions, Failure, Wheel};

/// 车型前轮转向限值及这次连续前进使用的实际后轴车速。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SteeringLimits {
    /// 两侧前轮的最大绝对转角，单位弧度，范围 [0, π/2)。
    pub max_angle_rad: f64,
    /// 最大绝对转向角速率，单位弧度每秒。
    pub max_rate_rad_s: f64,
    /// 实际后轴三维行程每秒推进的米数。
    pub speed_m_s: f64,
}

/// 已定位的前轮转向约束。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SteeringConstraint {
    /// 内轮不再连续向前滚动。
    Forward,
    /// 前轮转角超过车型限值。
    Angle,
    /// 当前车速要求的转向角速率过大。
    Rate,
    /// 两侧已发布轨迹要求在接缝瞬时改变转角。
    Jump,
}

pub(super) struct Steering {
    limits: SteeringLimits,
    sine: I,
    cosine: I,
}
impl Steering {
    pub fn new(limits: SteeringLimits) -> Result<Self, Failure> {
        if ![
            limits.max_angle_rad,
            limits.max_rate_rad_s,
            limits.speed_m_s,
        ]
        .into_iter()
        .all(|v| v.is_finite() && v >= 0.)
            || limits.max_angle_rad >= std::f64::consts::FRAC_PI_2
        {
            return Err(Failure::InvalidInput);
        }
        // 交错幂级数相邻部分和夹住真值。浮点平台的 libm 无须参与限值裁决。
        let x = I::point(limits.max_angle_rad);
        let step = -x.square();
        let trig = |mut term: I, odd: u32| {
            let mut sum = term;
            for k in 1..=10 {
                let i = 2 * k + odd;
                term = term * step / I::point(f64::from(i * (i - 1)));
                let next = sum + term;
                if k == 10 {
                    return sum.hull(next);
                }
                sum = next;
            }
            unreachable!()
        };
        Ok(Self {
            limits,
            sine: trig(x, 1),
            cosine: trig(I::ONE, 0),
        })
    }
    pub fn moving(&self) -> bool {
        self.limits.speed_m_s > 0.
    }

    pub fn check(
        &self,
        motion: &Motion,
        pose: &Pose,
        axle: AxleDimensions,
    ) -> Result<Option<(Wheel, SteeringConstraint)>, Failure> {
        let (k, dk) = curvature(motion, pose, axle)?;
        let ell = I::point(axle.wheelbase_m);
        let mut uncertain = false;
        for (wheel, b) in [
            (Wheel::FrontLeft, axle.half_track_m),
            (Wheel::FrontRight, -axle.half_track_m),
        ] {
            let forward = I::ONE - I::point(b) * k;
            if forward.hi <= 0. {
                return Ok(Some((wheel, SteeringConstraint::Forward)));
            }
            uncertain |= forward.lo <= 0.;
            let tangent = ell * k;
            let angle = tangent.abs() * self.cosine - forward * self.sine;
            if angle.lo > 0. {
                return Ok(Some((wheel, SteeringConstraint::Angle)));
            }
            uncertain |= angle.hi > 0.;
            let rate = (ell * dk * I::point(self.limits.speed_m_s)).abs();
            let available =
                I::point(self.limits.max_rate_rad_s) * (forward.square() + tangent.square());
            if rate.lo > available.hi {
                return Ok(Some((wheel, SteeringConstraint::Rate)));
            }
            uncertain |= rate.hi > available.lo;
        }
        if uncertain {
            Err(Failure::Steering)
        } else {
            Ok(None)
        }
    }
}

pub(super) fn curvature(m: &Motion, pose: &Pose, axle: AxleDimensions) -> Result<(I, I), Failure> {
    if m.a == [I::ZERO; 2] && m.j == [I::ZERO; 2] {
        return Ok((I::ZERO, I::ZERO));
    }
    let (h, dh) = m.heading()?;
    let rates = pose.rates(axle, m.v, h, dh)?;
    let dn = rates.up;
    let rear_speed = rates.rear_speed;
    let normal = pose.up;
    let nh = [normal[0], normal[2]];
    let ny = normal[1];
    let vy = -dot(nh, m.v) / ny;
    let speed2 = m.v[0].square() + m.v[1].square() + vy.square();
    let denom = ny * speed2 * speed2.sqrt();
    if denom.lo <= 0. || rear_speed.lo <= 0. {
        return Err(Failure::Steering);
    }
    let det = |v: [I; 2], a: [I; 2]| v[1] * a[0] - v[0] * a[1];
    let k = det(m.v, m.a) / denom;
    let dvy = -(dot([dn[0], dn[2]], m.v) + dot(nh, m.a) + dn[1] * vy) / ny;
    let dk = (det(m.v, m.j) / denom
        - k * (dn[1] / ny + I::point(3.) * (dot(m.v, m.a) + vy * dvy) / speed2))
        / rear_speed;
    if !k.finite() || !dk.finite() {
        return Err(Failure::Steering);
    }
    Ok((k, dk))
}
