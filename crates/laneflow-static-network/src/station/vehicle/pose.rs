use super::interval::Interval;
use super::surface::{Horizontal, Projection, Surface, dot, shifted};
use super::{AxleDimensions, Failure, PoseBounds, Work};

pub(super) type Vector = [Interval; 3];
pub(super) struct PoseRate {
    pub rear: Vector,
    pub forward: Vector,
    pub left: Vector,
    pub up: Vector,
    pub rear_speed: Interval,
}

pub(super) struct Pose {
    pub rear: Vector,
    pub forward: Vector,
    pub left: Vector,
    pub up: Vector,
    horizontal: Interval,
    rise: Interval,
    across: Interval,
    raw_length: Interval,
    samples: [[Projection; 2]; 2],
}

fn cross(a: Vector, b: Vector) -> Vector {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

struct AxleHeights {
    mean: Interval,
    difference: Interval,
    samples: [Projection; 2],
}

fn wheel_samples(
    surface: &Surface<'_>,
    center: Horizontal,
    left: Horizontal,
    half_track: Interval,
    work: &mut Work,
) -> Result<[Projection; 2], Failure> {
    Ok([
        surface.project(shifted(center, left, half_track), work)?,
        surface.project(shifted(center, left, -half_track), work)?,
    ])
}

fn mean([a, b]: [Interval; 2]) -> Interval {
    (a + b) * Interval::point(0.5)
}

fn rear_position(q: Horizontal, height: Interval, axle: AxleDimensions) -> Vector {
    [
        q[0],
        height + Interval::point(axle.reference_height_m),
        q[1],
    ]
}

fn wheel_velocities(velocity: Horizontal, dl: Horizontal, half: Interval) -> [Horizontal; 2] {
    [half, -half].map(|b| shifted(velocity, dl, b))
}

fn height_rates(samples: [Projection; 2], v: [Horizontal; 2]) -> [Interval; 2] {
    std::array::from_fn(|i| dot(samples[i].gradient, v[i]))
}

fn rear_velocity(horizontal: Horizontal, vertical: Interval) -> (Vector, Interval) {
    let v = [horizontal[0], vertical, horizontal[1]];
    (v, (v[0].square() + v[2].square() + v[1].square()).sqrt())
}

/// 行程只依赖后轴的两轮取样，共用完整姿态的后轴高度和导数构造。
pub(super) struct RearAxle {
    pub position: Vector,
    samples: [Projection; 2],
}

impl RearAxle {
    pub fn enclose(
        surface: &Surface<'_>,
        q: Horizontal,
        heading: Horizontal,
        axle: AxleDimensions,
        work: &mut Work,
    ) -> Result<Self, Failure> {
        let left = [heading[1], -heading[0]];
        let samples = wheel_samples(surface, q, left, Interval::point(axle.half_track_m), work)?;
        let position = rear_position(q, mean(samples.map(|s| s.height)), axle);
        Ok(Self { position, samples })
    }

    pub fn rate(&self, axle: AxleDimensions, v: Horizontal, dh: Horizontal) -> (Vector, Interval) {
        let vr = wheel_velocities(v, [dh[1], -dh[0]], Interval::point(axle.half_track_m));
        rear_velocity(v, mean(height_rates(self.samples, vr)))
    }
}

fn heights(
    surface: &Surface<'_>,
    center: Horizontal,
    left: Horizontal,
    half_track: Interval,
    lateral_rise: Interval,
    work: &mut Work,
) -> Result<AxleHeights, Failure> {
    let [a, b] = wheel_samples(surface, center, left, half_track, work)?;
    Ok(AxleHeights {
        mean: mean([a.height, b.height]),
        difference: (a.height - b.height)
            .intersect(lateral_rise)
            .ok_or(Failure::Pose)?,
        samples: [a, b],
    })
}

impl Pose {
    pub fn enclose(
        surface: &Surface<'_>,
        q: Horizontal,
        heading: Horizontal,
        axle: AxleDimensions,
        work: &mut Work,
    ) -> Result<Self, Failure> {
        let wheelbase = Interval::point(axle.wheelbase_m);
        let half_track = Interval::point(axle.half_track_m);
        let track = Interval::point(2.) * half_track;
        let left = [heading[1], -heading[0]];
        // 在两轴之间的整个辅助取样区求坡度，由此同时包住每条左右、前后取样线。
        let strip = shifted(
            shifted(q, heading, Interval::ZERO.hull(wheelbase)),
            left,
            (-half_track).hull(half_track),
        );
        let gradient = surface.project(strip, work)?.gradient;
        let grade = dot(gradient, heading);
        // |F-R|² 对 d 的导数为 2d(1+平均纵坡×前轴处纵坡)。两者同属 grade；
        // d=0 时为零，d=轴距时至少为轴距平方，严格递增保证唯一、连续的前向根。
        if (Interval::ONE + grade * grade).lo <= 0. {
            return Err(Failure::Pose);
        }
        work.charge()?;
        let horizontal = (wheelbase / (Interval::ONE + grade.square()).sqrt())
            .intersect(Interval::ZERO.hull(wheelbase))
            .ok_or(Failure::Pose)?;
        if horizontal.lo <= 0. {
            return Err(Failure::Pose);
        }
        let lateral_rise = track * dot(gradient, left);
        let rear = heights(surface, q, left, half_track, lateral_rise, work)?;
        let front = heights(
            surface,
            shifted(q, heading, horizontal),
            left,
            half_track,
            lateral_rise,
            work,
        )?;
        let rise = (front.mean - rear.mean)
            .intersect(horizontal * grade)
            .ok_or(Failure::Pose)?;
        // 再以实测高差收紧同一个根，所有方向和部位均复用这份轴距证据。
        work.charge()?;
        let horizontal = horizontal
            .intersect((wheelbase.square() - rise.square()).sqrt())
            .ok_or(Failure::Pose)?;
        let forward =
            [heading[0] * horizontal, rise, heading[1] * horizontal].map(|x| x / wheelbase);
        let across = (front.difference + rear.difference) * Interval::point(0.5);
        let raw = [track * left[0], across, track * left[1]];
        // 水平左右向与车头水平分量正交，因而 D·T 仅有竖直项。
        let projection = across * forward[1];
        let raw = std::array::from_fn::<_, 3, _>(|i| raw[i] - forward[i] * projection);
        let mut length = (raw[0].square() + raw[1].square() + raw[2].square()).sqrt();
        // 正交化保留水平左向上的整段轮距。
        length.lo = length.lo.max(track.lo);
        if !length.finite() || length.lo <= 0. {
            return Err(Failure::Pose);
        }
        let left = raw.map(|v| v / length);
        let mut up = cross(forward, left);
        if up[1].lo > 0. {
            up[1] = up[1]
                .intersect((Interval::ONE - up[0].square() - up[2].square()).sqrt())
                .ok_or(Failure::Pose)?;
        }
        let samples = [rear.samples, front.samples];
        let rear = rear_position(q, rear.mean, axle);
        if up[1].lo <= 0.
            || ![rear, forward, left, up]
                .into_iter()
                .flatten()
                .all(Interval::finite)
        {
            return Err(Failure::Pose);
        }
        Ok(Self {
            rear,
            forward,
            left,
            up,
            horizontal,
            rise,
            across,
            raw_length: length,
            samples,
        })
    }

    /// 对轴距方程和正交化后的三轴求导。投影梯度已包括接缝两侧和来源误差。
    pub fn rates(
        &self,
        axle: AxleDimensions,
        velocity: Horizontal,
        h: Horizontal,
        dh: Horizontal,
    ) -> Result<PoseRate, Failure> {
        let n = Interval::point;
        let ell = n(axle.wheelbase_m);
        let half = n(axle.half_track_m);
        let dl = [dh[1], -dh[0]];
        let [rear, front] = self.samples;
        let vr = wheel_velocities(velocity, dl, half);
        let vf = vr.map(|v| shifted(v, dh, self.horizontal));
        let rear_rate = height_rates(rear, vr);
        let front_rate = height_rates(front, vf);
        let grade = front.map(|s| dot(s.gradient, h));
        let dy = mean(front_rate) - mean(rear_rate);
        let denom = self.horizontal + self.rise * mean(grade);
        if denom.lo <= 0. {
            return Err(Failure::Pose);
        }
        let dd = -self.rise * dy / denom;
        let drise = dy + mean(grade) * dd;
        let df = shifted(dh.map(|v| v * self.horizontal), h, dd);
        let dt = [df[0], drise, df[1]].map(|v| v / ell);
        let dacross = ((front_rate[0] - front_rate[1])
            + (grade[0] - grade[1]) * dd
            + (rear_rate[0] - rear_rate[1]))
            * n(0.5);
        let mut raw = [n(2.) * half * dl[0], dacross, n(2.) * half * dl[1]];
        let proj = self.across * self.forward[1];
        let dproj = dacross * self.forward[1] + self.across * dt[1];
        for (i, v) in raw.iter_mut().enumerate() {
            *v = *v - dt[i] * proj - self.forward[i] * dproj;
        }
        let along = self.left[0] * raw[0] + self.left[1] * raw[1] + self.left[2] * raw[2];
        let db = std::array::from_fn(|i| (raw[i] - self.left[i] * along) / self.raw_length);
        let a = cross(dt, self.left);
        let b = cross(self.forward, db);
        let (rear, speed) = rear_velocity(velocity, mean(rear_rate));
        let mut dn = std::array::from_fn(|i| a[i] + b[i]);
        dn[1] = dn[1]
            .intersect(-(self.up[0] * dn[0] + self.up[2] * dn[2]) / self.up[1])
            .ok_or(Failure::Pose)?;
        Ok(PoseRate {
            rear,
            forward: dt,
            left: db,
            up: dn,
            rear_speed: speed,
        })
    }

    pub fn point(&self, a: Interval, b: Interval, c: Interval) -> Vector {
        std::array::from_fn(|i| {
            self.rear[i] + a * self.forward[i] + b * self.left[i] + c * self.up[i]
        })
    }

    pub fn wheel(&self, a: f64, b: f64) -> Horizontal {
        [0, 2].map(|i| {
            self.rear[i] + Interval::point(a) * self.forward[i] + Interval::point(b) * self.left[i]
        })
    }

    pub fn bounds(&self) -> PoseBounds {
        PoseBounds {
            rear_m: self.rear.map(Interval::bounds),
            forward: self.forward.map(Interval::bounds),
            left: self.left.map(Interval::bounds),
            up: self.up.map(Interval::bounds),
        }
    }
}
