//! 在实际规范弦上证明来源站位、规范进度与原线累计行程的连续误差。
mod bezier;

use super::NativeChordBounds;
use super::compile_geometry::GeometryCompilationError;
use super::cut_stations::local_millimetres;
use super::geometry::{
    ApproximationPoint, CurveSegment, NumericFreezeError, NumericFreezeKind, SegmentEvaluator,
};
use super::native_curve::{Scalar, exact_point, norm};
use crate::declaration::{
    AuthoringWidthProfile, EdgeLength, StationBandProof, StationChordProof, StationEdgeProof,
};
use laneflow_static_contract::StationSpan;
use std::sync::Arc;
const UNPROVEN: NumericFreezeKind = NumericFreezeKind::StationMappingNotProven;
const MM_PER_M: f64 = 1000.0;
const MAX_PREFIX_ERROR_METERS: f64 = 0.0005;
type Result<T> = std::result::Result<T, NumericFreezeError>;

pub(super) struct Chord {
    stations: [Scalar; 2],
    station_length: Scalar,
    travel: Scalar,
    prefix_speed: Scalar,
    duration: Scalar,
    station_rate: Scalar,
}

impl Chord {
    // 先按本弦占边长的比例分配累计行程预算。可确认的长弦亏损需要重新采样；
    // 端点量化已经主导的区间留给实际 f32 累计表的最终证明。
    pub(super) fn fits_chord(&self, edge_length: f64, points: [ApproximationPoint; 2]) -> bool {
        let s = Scalar::exact;
        let points = points.map(|p| [p.x, p.y, p.z]);
        let [a, b] = points;
        let delta = std::array::from_fn(|i| s(f64::from(b[i])) - s(f64::from(a[i])));
        let quantization = points
            .into_iter()
            .flatten()
            .map(|x| f64::from(x.abs().next_up() - x.abs()))
            .sum::<f64>();
        let share = (s(0.00045) * self.station_length / s(edge_length)).lo;
        prefix_interpolation_error(self.duration, self.prefix_speed) <= MAX_PREFIX_ERROR_METERS
            && self.travel.lo - norm(delta).hi <= share + quantization
    }
}

fn prefix_interpolation_error(duration: Scalar, speed: Scalar) -> f64 {
    (duration * (Scalar::exact(speed.hi) - Scalar::exact(speed.lo)) * Scalar::exact(0.25)).hi
}
fn interval(lo: f64, hi: f64) -> Scalar {
    Scalar {
        value: lo + (hi - lo) * 0.5,
        lo,
        hi,
    }
}

pub(super) fn chord(
    evaluator: SegmentEvaluator<'_>,
    parameters: [f64; 2],
    native: Option<NativeChordBounds>,
    edge_length: f64,
) -> Result<Chord> {
    let s = Scalar::exact;
    let SegmentEvaluator::Offset {
        segment,
        station,
        offset,
    } = evaluator
    else {
        return Err(UNPROVEN.into());
    };
    let dt = s(parameters[1]) - s(parameters[0]);
    let ds = s(station.cumulative_end_meters) - s(station.cumulative_start_meters);
    let row_dt = s(station.parameter_end) - s(station.parameter_start);
    let station_rate = match segment {
        CurveSegment::Native(c) => c.law.road_length(),
        _ => ds / row_dt,
    };
    if station_rate.lo <= 0.0 {
        return Err(UNPROVEN.into());
    }
    let at = |t: f64| {
        s(station.cumulative_start_meters).widen(station.station_error_meters)
            + station_rate * (s(t) - s(station.parameter_start))
    };
    let stations = parameters.map(at);
    let (speed, travel) = match (segment, native) {
        (CurveSegment::Native(c), Some(b)) => {
            let speed = interval(b.speed[0], b.speed[1]) * c.law.length;
            let travel = if (speed.hi - speed.lo) * dt.hi > 1e-7 {
                // 将单调站位区间划小，包围每一小段上的速度积分。常曲率等偏移直接用原弦证据。
                let law = c.law;
                let mut sum = s(0.0);
                let mut left = parameters[0];
                for j in 1..=128 {
                    let right = if j == 128 {
                        parameters[1]
                    } else {
                        parameters[0] + (parameters[1] - parameters[0]) * (f64::from(j) / 128.0)
                    };
                    let (a, dp) = super::native_bounds::offset_at(c, Some((station, offset)), left);
                    let (z, _) = super::native_bounds::offset_at(c, Some((station, offset)), right);
                    let width = interval(a.lo.min(z.lo), a.hi.max(z.hi));
                    let (ka, kb) = (law.curvature(left), law.curvature(right));
                    let curvature = interval(ka.lo.min(kb.lo), ka.hi.max(kb.hi));
                    sum = sum
                        + (s(right) - s(left))
                            * law.length
                            * norm([s(1.0) - width * curvature, law.grade, dp]);
                    left = right;
                }
                let endpoint_error = (s(station.station_error_meters)
                    + s(offset.end_station_error_meters)
                    + (s(station.cumulative_end_meters) + law.road_length())
                        * s(64.0 * f64::EPSILON)
                    + s(64.0 * f64::from_bits(1)))
                    / (s(1.0) + law.grade.square()).sqrt();
                let refined = sum.widen((s(2.0) * endpoint_error * s(b.speed[1])).hi);
                interval(
                    refined.lo.max(b.travel_distance[0]),
                    refined.hi.min(b.travel_distance[1]),
                )
            } else {
                interval(b.travel_distance[0], b.travel_distance[1])
            };
            (speed, travel)
        }
        (CurveSegment::Line { start, end }, _) => {
            let a = exact_point(start);
            let b = exact_point(end);
            let delta = std::array::from_fn(|i| b[i] - a[i]);
            let offset_rate = (s(offset.offset_end_meters) - s(offset.offset_start_meters))
                * station_rate
                / (s(offset.station_end_meters).widen(offset.end_station_error_meters)
                    - s(offset.station_start_meters));
            let speed = (norm(delta).square() + offset_rate.square()).sqrt();
            (speed, speed * dt)
        }
        (
            CurveSegment::CubicBezier {
                start,
                control_1,
                control_2,
                end,
            },
            _,
        ) => {
            let curve = bezier::OffsetSpeed::new(
                [start, control_1, control_2, end],
                station,
                offset,
                station_rate,
            );
            // 整边留出 0.05 毫米给积分区间宽度，与弦亏损和弦内前缀的预算合计 1 毫米。
            let share = (s(0.00005) * station_rate * dt / s(edge_length)).lo;
            curve.measure(parameters, share)?
        }
        _ => return Err(UNPROVEN.into()),
    };
    if !speed.lo.is_finite() || speed.lo <= 0.0 || !speed.hi.is_finite() {
        return Err(UNPROVEN.into());
    }
    Ok(Chord {
        stations,
        station_length: station_rate * dt,
        travel,
        prefix_speed: speed,
        duration: dt,
        station_rate,
    })
}

// 毫米格在超出 binary64 精确整数范围时也取得向外包围。
fn meters(n: u64) -> Scalar {
    let f = n as f64;
    let v = if n <= 1 << 53 {
        Scalar::exact(f)
    } else {
        interval(f.next_down(), f.next_up())
    };
    v / Scalar::exact(MM_PER_M)
}
fn upper(m: f64) -> Result<u32> {
    let mm = (Scalar::exact(m) * Scalar::exact(MM_PER_M)).hi.ceil();
    if !(0.0..=f64::from(u32::MAX)).contains(&mm) {
        return Err(UNPROVEN.into());
    }
    Ok(mm as u32)
}
fn measured_node(origin: f64, position: Scalar, forced: Option<f64>) -> Result<(u64, f64)> {
    let n = local_millimetres(origin, forced.unwrap_or(position.value)).ok_or(UNPROVEN)?;
    let error_m = (position - Scalar::exact(origin) - meters(n)).abs_max();
    if !error_m.is_finite() || error_m > 0.001 {
        return Err(UNPROVEN.into());
    }
    Ok((n, error_m))
}

pub(super) fn node(origin: f64, position: Scalar, forced: Option<f64>) -> Result<(u64, u32)> {
    let (n, error_m) = measured_node(origin, position, forced)?;
    Ok((n, upper(error_m)?))
}

pub(super) fn finish(
    mut chords: Vec<Chord>,
    points: &[ApproximationPoint],
    domain: [f64; 2],
    edge: [f64; 2],
    backward: bool,
) -> Result<Arc<StationEdgeProof>> {
    let s = Scalar::exact;
    let count = chords.len();
    if count + 1 != points.len() || !(1..=4096).contains(&count) {
        return Err(UNPROVEN.into());
    }
    if backward {
        chords.reverse()
    }
    let i = usize::from(backward);
    let (start, start_error_m) = measured_node(domain[0], chords[0].stations[i], Some(edge[i]))?;
    let mut spans = Vec::with_capacity(count);
    let mut cumulative = 0.0_f32;
    let mut drift = s(0.0);
    let mut max_slope = 0.0_f64;
    let mut max_node_error_m = start_error_m;
    let (mut station, mut progress) = (start, 0_u64);
    for (k, (chord, pair)) in chords.iter().zip(points.windows(2)).enumerate() {
        let (end, station_error_m) = measured_node(
            domain[0],
            chord.stations[1 - i],
            (k + 1 == count).then_some(edge[1 - i]),
        )?;
        let (begin, left_error_m) =
            measured_node(domain[0], chord.stations[i], (k == 0).then_some(edge[i]))?;
        if begin != station || begin == end || (end < begin) != backward {
            return Err(UNPROVEN.into());
        }
        if let Some(previous) = spans.last_mut() {
            let previous: &mut StationChordProof = previous;
            previous.station_error = previous.station_error.max(upper(left_error_m)?)
        }
        max_node_error_m = max_node_error_m.max(station_error_m).max(left_error_m);
        let (_, chord_length) = crate::spatial_freeze::canonical_spatial_chord(pair[0], pair[1]);
        let next = cumulative + chord_length;
        if !next.is_finite() || next <= cumulative {
            return Err(UNPROVEN.into());
        }
        let dp = s(f64::from(next)) - s(f64::from(cumulative));
        let q = if k + 1 == count {
            let length =
                EdgeLength::try_from_authoring_metres(f64::from(next)).map_err(|_| UNPROVEN)?;
            u64::from(length.millimetres())
        } else {
            local_millimetres(0.0, f64::from(next)).ok_or(UNPROVEN)?
        };
        if q <= progress {
            return Err(UNPROVEN.into());
        }
        let canonical_rate = dp / chord.station_length;
        max_slope = max_slope.max(canonical_rate.hi);
        let interpolated = (meters(q) - meters(progress)) / meters(station.abs_diff(end));
        let rate = (s(canonical_rate
            .lo
            .min((chord.prefix_speed / chord.station_rate).lo)
            .min(interpolated.lo))
            * s(1_000_000.0))
        .lo
        .floor();
        if rate < 1.0 || !rate.is_finite() {
            return Err(UNPROVEN.into());
        }
        let prior = drift;
        drift = drift + chord.travel - dp;
        // 与连接两端的行程插值比较，前缀最大偏差为 D × (vmax−vmin) × f(1−f)。
        let bow = s(prefix_interpolation_error(
            chord.duration,
            chord.prefix_speed,
        ));
        let travel_error = upper((s(prior.abs_max().max(drift.abs_max())) + bow).hi)?;
        let encode = upper(
            (meters(q) - s(f64::from(next)))
                .abs_max()
                .max((meters(progress) - s(f64::from(cumulative))).abs_max()),
        )?;
        let rate = rate.min(f64::from(u32::MAX)) as u32;
        StationSpan::try_from_millimetres(
            [station, end],
            [progress, q],
            encode,
            travel_error,
            rate,
        )
        .map_err(|_| UNPROVEN)?;
        spans.push(StationChordProof {
            station: end,
            station_error: upper(station_error_m)?,
            progress: q,
            cumulative: next,
            canonical_error: encode,
            travel_error,
            rate,
        });
        station = end;
        progress = q;
        cumulative = next;
    }
    // 所有弦共用的 Lipschitz 上界也覆盖量化节点落到相邻弦一侧的情形。
    let station_shift_m = s(max_slope) * s(max_node_error_m);
    let mut previous_error_m = 0_f64;
    for span in &mut spans {
        let endpoint_error_m = (meters(span.progress) - s(f64::from(span.cumulative))).abs_max();
        span.canonical_error =
            upper((s(endpoint_error_m.max(previous_error_m)) + station_shift_m).hi)?;
        previous_error_m = endpoint_error_m;
        if span.canonical_error > StationSpan::MAX_CANONICAL_ERROR_MM {
            return Err(UNPROVEN.into());
        }
    }
    let origin = local_millimetres(0.0, domain[0]).ok_or(UNPROVEN)?;
    let corridor_length = local_millimetres(domain[0], domain[1]).ok_or(UNPROVEN)?;
    if origin
        .checked_add(corridor_length)
        .is_none_or(|end| end > StationSpan::MAX_DISTANCE_MM)
    {
        return Err(UNPROVEN.into());
    }
    let origin_error = domain[0].mul_add(MM_PER_M, -(origin as f64)).abs();
    if origin_error > 1.0 {
        return Err(UNPROVEN.into());
    }
    Ok(Arc::new(StationEdgeProof {
        origin,
        origin_error_mm: u32::from(origin_error > 0.0),
        corridor_length,
        start,
        start_error: upper(start_error_m)?,
        spans: spans.into_boxed_slice(),
    }))
}

pub(super) fn width_rounding_error(profiles: &[AuthoringWidthProfile]) -> f64 {
    let s = Scalar::exact;
    // 从参考车道向每一侧按同一符号累加宽度。以全部宽度和运算次数包住边界与中心的逐次舍入。
    let total = profiles.iter().fold(s(0.0), |v, p| {
        v + s(p.start_width_meters.max(p.end_width_meters))
    });
    // binary64 精确的整毫米数以米表示时分母至多为 8。半宽和累加边界因而
    // 都在 1/16 米格上；所有中间整数未超出有效位时，这些加法和对半均精确。
    if total.hi * 16.0 < (1_u64 << 53) as f64
        && profiles.iter().all(|p| {
            [p.start_width_meters, p.end_width_meters]
                .into_iter()
                .all(|m| {
                    let mm = (m * MM_PER_M).round_ties_even();
                    m.mul_add(MM_PER_M, -mm) == 0.0
                })
        })
    {
        return 0.0;
    }
    (total * s((profiles.len() + 3) as f64) * s(4.0 * f64::EPSILON) + s(f64::MIN_POSITIVE)).hi
}

#[derive(Clone, Copy, Default)]
pub(super) struct WidthBoundary {
    pub(super) meters: [f64; 2],
    // 只在发布引用本边界的成员时报告越界，诊断可以指向实际的宽度端点。
    millimeters: [Option<i32>; 2],
}

impl WidthBoundary {
    pub(super) fn new(meters: [f64; 2]) -> Self {
        let millimeters =
            meters.map(|m| i32::try_from(super::cut_stations::rounded_difference_mm(m, 0.0)?).ok());
        Self {
            meters,
            millimeters,
        }
    }
}

pub(super) fn band(
    width_table: crate::RoadEditingTableKind,
    [left, right]: [WidthBoundary; 2],
    width: AuthoringWidthProfile,
    domain: [f64; 2],
    round: f64,
    station_error: f64,
    source: &crate::SourceLocation,
) -> std::result::Result<StationBandProof, GeometryCompilationError> {
    let s = Scalar::exact;
    let unproven = |location| {
        GeometryCompilationError::Constraint(Box::new((
            crate::RoadEditingNumericViolation::StationMappingNotProven,
            location,
        )))
    };
    let values = [
        left.meters[0],
        right.meters[0],
        left.meters[1],
        right.meters[1],
    ]
    .map(|meters| s(meters).widen(round));
    let quantized = [
        left.millimeters[0],
        right.millimeters[0],
        left.millimeters[1],
        right.millimeters[1],
    ];
    let mut offsets_mm = [0; 4];
    let mut err = 0.0_f64;
    for (j, v) in values.iter().enumerate() {
        let mm = quantized[j]
            .ok_or_else(|| unproven(source.width_endpoint(width_table, (j / 2) as u8)))?;
        offsets_mm[j] = mm;
        err = err.max((*v - s(f64::from(mm)) / s(MM_PER_M)).abs_max());
    }
    for (end, source_width) in [width.start_width_meters, width.end_width_meters]
        .into_iter()
        .enumerate()
    {
        let (left, right) = (offsets_mm[2 * end], offsets_mm[2 * end + 1]);
        if left < right || (left == right) != (source_width == 0.0) {
            return Err(unproven(source.width_endpoint(width_table, end as u8)));
        }
    }
    let length = s(domain[1]) - s(domain[0]);
    let slope = ((values[0] - values[2]) / length)
        .abs_max()
        .max(((values[1] - values[3]) / length).abs_max());
    // 恒定的精确边界没有站位误差传播。同一来源、同一边界的 FMA 残差为零
    // 才能发布零误差，避免将区间库为通用运算保留的末位包围进位到整毫米。
    let exact_constant = round == 0.0
        && left.meters[0] == left.meters[1]
        && right.meters[0] == right.meters[1]
        && [left.meters[0], right.meters[0]]
            .into_iter()
            .zip(offsets_mm)
            .all(|(m, mm)| m.mul_add(MM_PER_M, -f64::from(mm)) == 0.0);
    let lateral_error = if exact_constant {
        0
    } else {
        upper((s(err) + s(slope) * (s(station_error) + s(0.001))).hi)
            .map_err(|_| unproven(source.clone()))?
    };
    if lateral_error > StationSpan::MAX_SOURCE_ERROR_MM {
        return Err(unproven(source.clone()));
    }
    Ok(StationBandProof {
        offsets_mm,
        lateral_error,
    })
}
