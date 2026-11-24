//! 在实际规范弦上证明来源站位、规范进度与原线累计行程的连续误差。
mod bezier;

use super::NativeChordBounds;
use super::compile_geometry::MemberOffsetEndpoints;
use super::cut_stations::local_nanometres;
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
const BILLION: f64 = 1_000_000_000.0;
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

// 纳米格在超出 binary64 精确整数范围时也取得向外包围。
fn meters(n: u64) -> Scalar {
    let f = n as f64;
    let v = if n <= 1 << 53 {
        Scalar::exact(f)
    } else {
        interval(f.next_down(), f.next_up())
    };
    v / Scalar::exact(BILLION)
}
fn upper(m: f64) -> Result<u32> {
    let nm = (Scalar::exact(m) * Scalar::exact(BILLION)).hi.ceil();
    if !(0.0..=f64::from(u32::MAX)).contains(&nm) {
        return Err(UNPROVEN.into());
    }
    Ok(nm as u32)
}
fn node(origin: f64, position: Scalar, forced: Option<f64>) -> Result<(u64, u32)> {
    let n = local_nanometres(origin, forced.unwrap_or(position.value)).ok_or(UNPROVEN)?;
    let err = upper((position - Scalar::exact(origin) - meters(n)).abs_max())?;
    if err > 1000 {
        return Err(UNPROVEN.into());
    }
    Ok((n, err))
}

pub(super) fn finish(
    mut chords: Vec<Chord>,
    points: &[ApproximationPoint],
    domain: [f64; 2],
    edge: [f64; 2],
    backward: bool,
    length: EdgeLength,
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
    let (start, start_error) = node(domain[0], chords[0].stations[i], Some(edge[i]))?;
    let mut spans = Vec::with_capacity(count);
    let mut cumulative = 0.0_f32;
    let mut drift = s(0.0);
    let mut max_slope = 0.0_f64;
    let mut max_node_error = start_error;
    let (mut station, mut progress) = (start, 0_u64);
    for (k, (chord, pair)) in chords.iter().zip(points.windows(2)).enumerate() {
        let (end, station_error) = node(
            domain[0],
            chord.stations[1 - i],
            (k + 1 == count).then_some(edge[1 - i]),
        )?;
        let (begin, left_error) = node(domain[0], chord.stations[i], (k == 0).then_some(edge[i]))?;
        if begin != station || begin == end || (end < begin) != backward {
            return Err(UNPROVEN.into());
        }
        if let Some(previous) = spans.last_mut() {
            let previous: &mut StationChordProof = previous;
            previous.station_error = previous.station_error.max(left_error)
        }
        max_node_error = max_node_error.max(station_error).max(left_error);
        let (_, chord_length) = crate::spatial_freeze::canonical_spatial_chord(pair[0], pair[1]);
        let next = cumulative + chord_length;
        if !next.is_finite() || next <= cumulative {
            return Err(UNPROVEN.into());
        }
        let dp = s(f64::from(next)) - s(f64::from(cumulative));
        let mut q = local_nanometres(0.0, f64::from(next)).ok_or(UNPROVEN)?;
        if k + 1 == count {
            if EdgeLength::try_from_authoring_metres(f64::from(next))
                .ok()
                .map(EdgeLength::millimetres)
                != Some(length.millimetres())
            {
                return Err(UNPROVEN.into());
            }
            let mm = u64::from(length.millimetres());
            q = q.clamp(
                mm * 1_000_000 - 500_000 + mm % 2,
                mm * 1_000_000 + 500_000 - mm % 2,
            );
        }
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
        StationSpan::try_new([station, end], [progress, q], encode, travel_error, rate)
            .map_err(|_| UNPROVEN)?;
        spans.push(StationChordProof {
            station: end,
            station_error,
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
    let station_shift = upper((s(max_slope) * meters(u64::from(max_node_error))).hi)?;
    for span in &mut spans {
        span.canonical_error = span
            .canonical_error
            .checked_add(station_shift)
            .ok_or(UNPROVEN)?;
        if span.canonical_error > StationSpan::MAX_CANONICAL_ERROR_NM {
            return Err(UNPROVEN.into());
        }
    }
    Ok(Arc::new(StationEdgeProof {
        origin: local_nanometres(0.0, domain[0]).ok_or(UNPROVEN)?,
        corridor_length: local_nanometres(domain[0], domain[1]).ok_or(UNPROVEN)?,
        start,
        start_error,
        spans: spans.into_boxed_slice(),
    }))
}

pub(super) fn width_rounding_error(profiles: &[AuthoringWidthProfile]) -> f64 {
    let s = Scalar::exact;
    // 求中心偏移时，各侧分别按同一符号累加宽度。以全部宽度和运算次数包住逐次舍入。
    let total = profiles.iter().fold(s(0.0), |v, p| {
        v + s(p.start_width_meters.max(p.end_width_meters))
    });
    (total * s((profiles.len() + 3) as f64) * s(4.0 * f64::EPSILON) + s(f64::MIN_POSITIVE)).hi
}

pub(super) fn band(
    offset: MemberOffsetEndpoints,
    width: AuthoringWidthProfile,
    domain: [f64; 2],
    round: f64,
    station_error: f64,
) -> Result<StationBandProof> {
    let s = Scalar::exact;
    let values = [
        s(offset.start_meters).widen(round) + s(width.start_width_meters) * s(0.5),
        s(offset.start_meters).widen(round) - s(width.start_width_meters) * s(0.5),
        s(offset.end_meters).widen(round) + s(width.end_width_meters) * s(0.5),
        s(offset.end_meters).widen(round) - s(width.end_width_meters) * s(0.5),
    ];
    let mut offsets_um = [0; 4];
    let mut err = 0.0_f64;
    for (j, v) in values.iter().enumerate() {
        let um = (v.value * 1_000_000.0).round_ties_even();
        if !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&um) {
            return Err(UNPROVEN.into());
        }
        offsets_um[j] = um as i32;
        err = err.max((*v - s(um) / s(1_000_000.0)).abs_max());
    }
    let length = s(domain[1]) - s(domain[0]);
    let slope = ((values[0] - values[2]) / length)
        .abs_max()
        .max(((values[1] - values[3]) / length).abs_max());
    let lateral_error = upper((s(err) + s(slope) * (s(station_error) + s(1e-6))).hi)?;
    if lateral_error > 1000
        || offsets_um[0] < offsets_um[1]
        || offsets_um[2] < offsets_um[3]
        || (offsets_um[0] == offsets_um[1] && offsets_um[2] == offsets_um[3])
    {
        return Err(UNPROVEN.into());
    }
    Ok(StationBandProof {
        offsets_um,
        lateral_error,
    })
}
