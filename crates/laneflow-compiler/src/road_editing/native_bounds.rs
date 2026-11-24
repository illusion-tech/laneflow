//! 原生来源曲线到实际规范弦的连续误差界（ADR 0022 §6.1）。
use super::geometry::{
    NumericFreezeError, NumericFreezeKind, OffsetInterval, Point3, StationInterval,
};
use super::native_curve::{NativeCurve, Scalar, Vector, exact_point, norm, unit};

/// 编译候选中一条已通过误差门槛的规范弦。回调以来源参数递增的次序执行。
#[derive(Clone, Copy, Debug)]
pub struct NativeChord<'a> {
    /// 本次模块内接收该弦的车道边或设施带。
    pub target_kind: laneflow_static_contract::EntityKind,
    /// 目标声明的局部键。
    pub target_key: &'a str,
    /// 目标声明的有序所属键。
    pub target_owners: &'a [std::sync::Arc<str>],
    /// 走行方向是否与来源参数相反。
    pub backward: bool,
    /// 来源段的 owner-local 位置。
    pub source: &'a crate::SourceLocation,
    /// 来源程序中的原段序号。
    pub segment_ordinal: u32,
    /// 实际保留的 f32 弦端点，按来源方向排列。
    pub endpoints: [[f32; 3]; 2],
    /// 来源段的归一化参数。
    pub parameters: [f64; 2],
    /// 对此弦的连续误差证据。
    pub bounds: NativeChordBounds,
}
pub(super) type Observer<'a> = dyn FnMut(NativeChord<'_>) + 'a;

/// 同一来源段内、同一横断面间原线相对实际规范弦的证明。里程按弦所在车道计算。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeChordBounds {
    /// 当前来源段缓存的转角积分子区间数。
    pub integration_blocks: usize,
    /// 从段首量起的水平弧长，两个端点向外舍入（米）。
    pub horizontal_interval: [f64; 2],
    /// 原车道在该区间内的三维行程下、上界（米）。
    pub travel_distance: [f64; 2],
    /// 原线与规范弦在相同参数处的位置差上界（米）。
    pub position_error: f64,
    /// 三维单位切向差的范数上界。
    pub direction_error: f64,
    /// 水平单位切向相对实际弦水平方向的范数差上界。
    pub heading_error: f64,
    /// 原线在参数中点的位置（米）。
    pub midpoint: [f64; 3],
    /// 原线中点相对报告中心值的三维位置误差上界（米）。
    pub midpoint_error: f64,
    /// 以水平米为参数的车道速度下、上界。
    pub speed: [f64; 2],
    /// 每水平米的三维单位切向变化率上界。
    pub direction_rate: f64,
}
/// 当前证据未能给出净空或接触结论的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeUnprovenReason {
    /// 传播全部误差后，间隔区间跨过零。
    ThresholdOverlap,
    /// 调用方给出的余量、刚性偏移半径或额外误差无有效界。
    InvalidMargin,
}

/// 对给定区间最小有符号间隔的证明状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeClearance {
    /// 原线整段有正净空。
    Clear,
    /// 原线上存在接触或侵入。
    Contact,
    /// 误差区间跨判定阈值，或消费方提供的余量无有效界。
    Unproven(NativeUnprovenReason),
}
impl NativeChordBounds {
    /// 将累计里程的不确定量传回水平参数、位置及三维单位切向。
    #[must_use]
    pub fn inverse_travel_error(&self, meters: f64) -> Option<[f64; 3]> {
        if !meters.is_finite() || meters < 0.0 || self.speed[0] <= 0.0 {
            return None;
        }
        let du = Scalar::exact(meters) / Scalar::exact(self.speed[0]);
        let errors = [
            du,
            du * Scalar::exact(self.speed[1]),
            du * Scalar::exact(self.direction_rate),
        ];
        errors
            .iter()
            .all(|e| e.hi.is_finite())
            .then(|| errors.map(|e| e.hi))
    }
    /// 用规范弦最小有符号间隔的上下界比较净空。姿态基与障碍误差由调用方计入额外米数。
    /// 若耗尽外层细分预算，调用方直接保留返回的未证结果。
    #[must_use]
    pub fn clearance(
        &self,
        chord_minimum: [f64; 2],
        radius: f64,
        extra_meters: f64,
    ) -> NativeClearance {
        if !chord_minimum.iter().all(|x| x.is_finite())
            || chord_minimum[0] > chord_minimum[1]
            || !radius.is_finite()
            || radius < 0.0
            || !extra_meters.is_finite()
            || extra_meters < 0.0
        {
            return NativeClearance::Unproven(NativeUnprovenReason::InvalidMargin);
        }
        let error = Scalar::exact(self.position_error)
            + Scalar::exact(radius) * Scalar::exact(self.heading_error)
            + Scalar::exact(extra_meters);
        if (Scalar::exact(chord_minimum[0]) - error).lo > 0.0 {
            NativeClearance::Clear
        } else if (Scalar::exact(chord_minimum[1]) + error).hi <= 0.0 {
            NativeClearance::Contact
        } else {
            NativeClearance::Unproven(NativeUnprovenReason::ThresholdOverlap)
        }
    }
}

type Offset = Option<(StationInterval, OffsetInterval)>;
fn hull(a: Scalar, b: Scalar) -> Scalar {
    Scalar {
        value: a.value,
        lo: a.lo.min(b.lo),
        hi: a.hi.max(b.hi),
    }
}
fn difference(a: Vector, b: Vector) -> Vector {
    std::array::from_fn(|i| a[i] - b[i])
}
fn offset_at(curve: &NativeCurve, offset: Offset, t: f64) -> (Scalar, Scalar) {
    let Some((row, width)) = offset else {
        return (Scalar::exact(0.0), Scalar::exact(0.0));
    };
    let s = Scalar::exact;
    let full_length = curve.law.road_length();
    let slope = (s(width.offset_end_meters) - s(width.offset_start_meters))
        / (s(width.station_end_meters).widen(width.end_station_error_meters)
            - s(width.station_start_meters));
    let station =
        s(row.cumulative_start_meters).widen(row.station_error_meters) + full_length * s(t);
    (
        s(width.offset_start_meters) + slope * (station - s(width.station_start_meters)),
        slope * (s(1.0) + curve.law.grade.square()).sqrt(),
    )
}
fn frame(
    curve: &NativeCurve,
    offset: Offset,
    t: f64,
) -> Result<(Vector, Vector), NumericFreezeError> {
    let (p, tangent) = curve.vectors(t)?;
    let (d, dp) = offset_at(curve, offset, t);
    let forward = Scalar::exact(1.0) - d * curve.law.curvature(t);
    let [x, g, z] = tangent;
    Ok((
        [p[0] + d * z, p[1], p[2] - d * x],
        [forward * x + dp * z, g, forward * z - dp * x],
    ))
}
pub(super) fn measure(
    curve: &NativeCurve,
    offset: Offset,
    a: f64,
    b: f64,
    chord: [Point3; 2],
) -> Result<Option<NativeChordBounds>, NumericFreezeError> {
    let s = Scalar::exact;
    // Native station rows span the entire segment. Bound the inverse of both forced cuts
    // and the rounded midpoints along any of the at most 20 dyadic subdivisions.
    let du = offset
        .map_or(s(0.0), |(row, width)| {
            let scale = s(row.cumulative_end_meters) + curve.law.road_length();
            let rounding = scale * s(64.0 * f64::EPSILON) + s(64.0 * f64::from_bits(1));
            (s(row.station_error_meters) + s(width.end_station_error_meters) + rounding)
                / (s(1.0) + curve.law.grade.square()).sqrt()
        })
        .finite()?;
    let dt = du / curve.law.length;
    let first = (s(a) - dt).lo.max(0.0);
    let last = (s(b) + dt).hi.min(1.0);
    let (da, dp) = offset_at(curve, offset, first);
    let (db, _) = offset_at(curve, offset, last);
    let d = hull(da, db);
    let k = hull(curve.law.curvature(first), curve.law.curvature(last));
    let forward = s(1.0) - d * k;
    // A small subinterval may prove positive even when an overly wide product cannot.
    if forward.hi <= 0.0 {
        return Err(NumericFreezeKind::HorizontalDerivativeNotProvenNonZero.into());
    }
    if forward.lo <= 0.0 {
        return Ok(None);
    }
    let kp = if curve.law.k0 == curve.law.k1 {
        s(0.0)
    } else {
        (curve.law.k1 - curve.law.k0) / curve.law.length
    };
    let acceleration = norm([-s(2.0) * dp * k - d * kp, s(0.0), forward * k]).finite()?;
    let speed = norm([forward, curve.law.grade, dp]).finite()?;
    if speed.lo <= 0.0 {
        return Ok(None);
    }
    let h = curve.law.length * (s(b) - s(a));
    let (p0, _) = frame(curve, offset, a)?;
    let (p1, _) = frame(curve, offset, b)?;
    let [c0, c1] = chord.map(exact_point);
    let e0 = norm(difference(p0, c0)).finite()?;
    let e1 = norm(difference(p1, c1)).finite()?;
    let position = h.square() * acceleration / s(8.0) + s(e0.hi.max(e1.hi)) + du * speed;
    let mid = a + (b - a) * 0.5;
    let radius = curve.law.length * s((s(mid) - s(a)).hi.max((s(b) - s(mid)).hi)) + du;
    let (midpoint, derivative) = frame(curve, offset, mid)?;
    let Ok(chord_unit) = unit(difference(c1, c0)) else {
        return Ok(None);
    };
    let rate = s(acceleration.hi) / s(speed.lo);
    let direction = norm(difference(unit(derivative)?, chord_unit)) + radius * rate;
    let horizontal = |v: Vector| [v[0], s(0.0), v[2]];
    let Ok(yaw_chord) = unit(horizontal(difference(c1, c0))) else {
        return Ok(None);
    };
    let yaw_rate = s(acceleration.hi) / norm([forward, dp, s(0.0)]);
    let heading = norm(difference(unit(horizontal(derivative))?, yaw_chord)) + radius * yaw_rate;
    let mid_error = norm(difference(midpoint, midpoint.map(|c| s(c.value)))).finite()?;
    let travel = (h * speed).widen((s(2.0) * du * speed).hi);
    Ok(Some(NativeChordBounds {
        integration_blocks: curve.block_count(),
        horizontal_interval: [
            (s(first) * curve.law.length).lo.max(0.0),
            (s(last) * curve.law.length).hi,
        ],
        travel_distance: [travel.finite()?.lo, travel.hi],
        position_error: position.finite()?.hi,
        midpoint: midpoint.map(|c| c.value),
        midpoint_error: (mid_error + du * speed).hi,
        heading_error: heading.finite()?.hi,
        direction_error: direction.finite()?.hi,
        speed: [speed.lo, speed.hi],
        direction_rate: rate.finite()?.hi,
    }))
}
