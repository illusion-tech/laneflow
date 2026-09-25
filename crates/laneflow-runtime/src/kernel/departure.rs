//! 显式出发状态的速度上界。恢复和修订切换不调用。

use laneflow_static_contract::LaneEdgeOrdinal;
use laneflow_static_network::BoundedDistance;

use super::input::VehicleDepartureState;
use super::tables::distance_to_occurrence_progress;
use super::units::{round_mm, round_um};
use crate::DepartureStateError;

/// 来源检查结果。数值无法证明时不拒绝。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DepartureBound {
    /// 没有声明，或初速不高于出发速度，或现有距离不短于保守下界。
    Satisfied,
    /// 声明自身不合法。
    Invalid(DepartureStateError),
    /// 已证明初速超过这份声明的速度上界。
    Exceeds,
}

/// 沿候选路线比较声明起点到当前车头的距离，和达到初速至少需要的距离。
#[allow(clippy::too_many_arguments)]
pub(crate) fn departure_bound(
    edges: &[LaneEdgeOrdinal],
    lengths_mm: &[u32],
    segments: &[u32],
    offsets: &[u32],
    totals: &[u32],
    current_index: u32,
    current_progress_mm: u32,
    initial_speed_mm_s: u32,
    departure: VehicleDepartureState,
    max_accel_m_s2: f32,
    delta_s: f32,
) -> DepartureBound {
    let Some(departure_index) = usize::try_from(departure.route_edge_index()).ok() else {
        return DepartureBound::Invalid(DepartureStateError::RouteIndexOutOfRange);
    };
    if departure_index >= edges.len() {
        return DepartureBound::Invalid(DepartureStateError::RouteIndexOutOfRange);
    }
    let edge = edges[departure_index];
    let Some(edge_length) = lengths_mm.get(edge.index()).copied() else {
        return DepartureBound::Invalid(DepartureStateError::RouteIndexOutOfRange);
    };
    if departure.progress_mm() > edge_length {
        return DepartureBound::Invalid(DepartureStateError::InvalidProgress);
    }
    if departure.speed_mm_s() > 100_000 {
        return DepartureBound::Invalid(DepartureStateError::SpeedOutOfRange);
    }
    let Some(current) = usize::try_from(current_index).ok() else {
        return DepartureBound::Invalid(DepartureStateError::AfterPlacement);
    };
    if current < departure_index
        || (current == departure_index && current_progress_mm < departure.progress_mm())
    {
        return DepartureBound::Invalid(DepartureStateError::AfterPlacement);
    }
    let Some(available) = distance_to_occurrence_progress(
        segments,
        offsets,
        totals,
        departure_index,
        departure.progress_mm(),
        current,
        current_progress_mm,
    ) else {
        return DepartureBound::Invalid(DepartureStateError::AfterPlacement);
    };
    if initial_speed_mm_s <= departure.speed_mm_s() {
        return DepartureBound::Satisfied;
    }
    if !max_accel_m_s2.is_finite() || !delta_s.is_finite() || max_accel_m_s2 < 0.0 || delta_s <= 0.0
    {
        return DepartureBound::Satisfied;
    }
    if matches!(available, BoundedDistance::BeyondFinite) {
        return DepartureBound::Satisfied;
    }
    match minimum_accel_distance(
        departure.speed_mm_s(),
        initial_speed_mm_s,
        max_accel_m_s2,
        delta_s,
        available,
    ) {
        DistanceNeed::Unsupported => DepartureBound::Satisfied,
        DistanceNeed::Unreachable | DistanceNeed::ExceedsAvailable => DepartureBound::Exceeds,
        DistanceNeed::WithinAvailable(required_um) => {
            let _ = required_um;
            DepartureBound::Satisfied
        }
    }
}

#[derive(Debug)]
enum DistanceNeed {
    WithinAvailable(u128),
    ExceedsAvailable,
    Unreachable,
    Unsupported,
}

impl crate::kernel::state::WorldState {
    /// 没有出发声明时直接通过。声明不合法或超过速度上界时失败，不提交车辆。
    pub(crate) fn admit_declared_departure(
        &self,
        input: crate::VehicleSpawnInput,
    ) -> Result<(), crate::SpawnError> {
        let Some(departure) = input.departure() else {
            return Ok(());
        };
        let Some(compiled) = self.compiled_route(input.route()) else {
            return Err(crate::SpawnError::UnknownRoute);
        };
        let profile = self
            .binding
            .revision
            .traffic()
            .relations()
            .vehicle_profile(input.profile())
            .expect("未停车校验已经解析过车辆 profile");
        let delta_s = self.binding.config.fixed_delta_time_ms() as f32 / 1_000.0;
        match departure_bound(
            compiled.edges.as_slice(),
            self.binding.revision.traffic().lane_lengths_millimetres(),
            &compiled.occurrence_segments,
            &compiled.occurrence_offsets,
            &compiled.segment_totals,
            input.route_edge_index(),
            input.progress_mm(),
            input.initial_speed_mm_s(),
            departure,
            profile.max_accel(),
            delta_s,
        ) {
            DepartureBound::Satisfied => Ok(()),
            DepartureBound::Invalid(reason) => {
                Err(crate::SpawnError::InvalidDepartureState(reason))
            }
            DepartureBound::Exceeds => Err(crate::SpawnError::InitialSpeedExceedsDepartureBound),
        }
    }
}

pub(crate) fn departure_replace_error(error: crate::SpawnError) -> crate::ReplaceError {
    match error {
        crate::SpawnError::InvalidDepartureState(reason) => {
            crate::ReplaceError::InvalidDepartureState(reason)
        }
        crate::SpawnError::InitialSpeedExceedsDepartureBound => {
            crate::ReplaceError::InitialSpeedExceedsDepartureBound
        }
        other => panic!("departure admission returned {other:?}"),
    }
}

fn minimum_accel_distance(
    departure_speed: u32,
    initial_speed: u32,
    accel: f32,
    dt: f32,
    available: BoundedDistance,
) -> DistanceNeed {
    let BoundedDistance::Finite(available_mm) = available else {
        return DistanceNeed::WithinAvailable(0);
    };
    let available_um = u128::from(available_mm).saturating_mul(1_000);
    let mut end = initial_speed;
    let mut required_um = 0u128;
    while end > departure_speed {
        let Some(start) = minimum_start_speed(end, accel, dt) else {
            return DistanceNeed::Unsupported;
        };
        if start >= end {
            return DistanceNeed::Unreachable;
        }
        let step_start = start.max(departure_speed);
        let Some(step_um) = step_distance_lower_um(step_start, end, dt) else {
            return DistanceNeed::Unsupported;
        };
        required_um = required_um.saturating_add(u128::from(step_um));
        if required_um > available_um {
            return DistanceNeed::ExceedsAvailable;
        }
        if start <= departure_speed {
            break;
        }
        end = start;
    }
    DistanceNeed::WithinAvailable(required_um)
}

fn minimum_start_speed(target: u32, accel: f32, dt: f32) -> Option<u32> {
    if quantized_speed_upper(target, accel, dt)? < target {
        return Some(target);
    }
    let rewind_mm = f64::from(accel) * f64::from(dt) * 1_000.0;
    let guess = f64::from(target) - rewind_mm;
    let mut cursor = if guess <= 0.0 {
        0
    } else {
        u32::try_from(guess.floor() as u64).unwrap_or(target)
    };
    if cursor > target {
        cursor = target;
    }
    let mut steps = 0u32;
    while quantized_speed_upper(cursor, accel, dt)? < target {
        if cursor >= target {
            return Some(target);
        }
        cursor = cursor.saturating_add(1);
        steps = steps.saturating_add(1);
        if steps > 64 {
            return binary_start_speed(cursor, target, target, accel, dt);
        }
    }
    while cursor > 0 && quantized_speed_upper(cursor - 1, accel, dt)? >= target {
        cursor -= 1;
        steps = steps.saturating_add(1);
        if steps > 64 {
            break;
        }
    }
    Some(cursor)
}

fn binary_start_speed(
    mut low: u32,
    mut high: u32,
    target: u32,
    accel: f32,
    dt: f32,
) -> Option<u32> {
    while low < high {
        let mid = low + (high - low) / 2;
        if quantized_speed_upper(mid, accel, dt)? >= target {
            high = mid;
        } else {
            low = mid.saturating_add(1);
        }
    }
    Some(low)
}

/// 与运行时同一拍速度积分：`f32` 先相加，再按 ties-to-even 量化到毫米/秒。
fn quantized_speed_upper(start_mm_s: u32, accel: f32, dt: f32) -> Option<u32> {
    let speed = start_mm_s as f32 / 1_000.0;
    let next = speed + accel * dt;
    if !next.is_finite() {
        return None;
    }
    match round_mm(f64::from(next)) {
        Some(speed) => Some(speed),
        None if next > 0.0 => Some(u32::MAX),
        None => None,
    }
}

fn step_distance_lower_um(start_mm_s: u32, end_mm_s: u32, dt: f32) -> Option<u64> {
    let start = start_mm_s as f32 / 1_000.0;
    let end = minimum_unrounded_speed_mps(end_mm_s) as f32;
    let travel = (start + end) * 0.5 * dt;
    if !travel.is_finite() {
        return None;
    }
    if travel <= 0.0 {
        return Some(0);
    }
    round_um(f64::from(travel))
}

/// 仍会量化到 `target_mm_s` 的最小未舍入速度（米/秒）。偶数临界点含在内。
fn minimum_unrounded_speed_mps(target_mm_s: u32) -> f64 {
    if target_mm_s == 0 {
        return 0.0;
    }
    let half = f64::from(target_mm_s) - 0.5;
    let millimetres = if target_mm_s.is_multiple_of(2) {
        half
    } else {
        half.next_up()
    };
    millimetres.max(0.0) / 1_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forward_max_accel(start: u32, accel: f32, dt: f32) -> Option<(u32, u64)> {
        let next = start as f32 / 1_000.0 + accel * dt;
        let end = round_mm(f64::from(next))?;
        let travel = (start as f32 / 1_000.0 + next) * 0.5 * dt;
        let um = round_um(f64::from(travel))?;
        Some((end, um))
    }

    #[test]
    fn speed_upper_matches_the_runtime_f32_step() {
        let accel = 0.5_f32;
        let dt = 0.005_f32;
        assert_eq!(quantized_speed_upper(1, accel, dt), Some(4));
    }

    #[test]
    fn terminal_step_uses_the_declared_speed() {
        let need = minimum_accel_distance(100, 180, 1.8, 0.1, BoundedDistance::Finite(1_000));
        let DistanceNeed::WithinAvailable(um) = need else {
            panic!("expected a finite bound, got {need:?}");
        };
        assert!(
            um >= 13_000,
            "declared 100 mm/s start needs at least 13 mm, got {um} um"
        );
    }

    #[test]
    fn micrometre_residuals_survive_until_the_final_comparison() {
        let need = minimum_accel_distance(0, 1_000, 0.5, 0.004, BoundedDistance::Finite(2_000));
        let DistanceNeed::WithinAvailable(um) = need else {
            panic!("expected a finite bound, got {need:?}");
        };
        assert!(
            um > 750_000,
            "dropping each tick's remainder collapses the bound to 750 mm, got {um} um"
        );
    }

    #[test]
    fn speed_upper_is_monotone_and_matches_forward_quantization() {
        let accel = 1.8_f32;
        let dt = 0.1_f32;
        let mut previous = 0u32;
        for speed in [0_u32, 1, 2, 50, 100, 180, 181, 999, 1_000, 10_000] {
            let upper = quantized_speed_upper(speed, accel, dt).expect("finite");
            assert!(
                upper >= previous,
                "{speed} -> {upper} dropped below {previous}"
            );
            let (forward, _) = forward_max_accel(speed, accel, dt).expect("forward");
            assert!(upper >= forward, "upper {upper} underestimated {forward}");
            previous = upper;
        }
    }

    #[test]
    fn ties_to_even_keeps_the_even_half_and_excludes_the_odd_half() {
        assert_eq!(round_mm(1.5 / 1_000.0), Some(2));
        assert_eq!(round_mm(0.5 / 1_000.0), Some(0));
        assert_eq!(round_mm(2.5 / 1_000.0), Some(2));
        let even = minimum_unrounded_speed_mps(2);
        assert_eq!(round_mm(even), Some(2));
        let odd = minimum_unrounded_speed_mps(1);
        assert_eq!(round_mm(odd), Some(1));
        assert!(odd > 0.5 / 1_000.0);
    }

    #[test]
    fn step_size_changes_the_distance_lower_bound() {
        let fine = minimum_accel_distance(0, 4_000, 1.8, 0.1, BoundedDistance::Finite(u32::MAX));
        let coarse = minimum_accel_distance(0, 4_000, 1.8, 1.0, BoundedDistance::Finite(u32::MAX));
        assert_ne!(
            bound_um(&fine),
            bound_um(&coarse),
            "100 ms and 1 s must not share one distance lower bound"
        );
    }

    fn bound_um(need: &DistanceNeed) -> u128 {
        match need {
            DistanceNeed::WithinAvailable(um) => *um,
            other => panic!("expected a completed bound, got {other:?}"),
        }
    }

    #[test]
    fn distance_lower_bound_does_not_exceed_a_forward_max_accel_run() {
        let cases = [
            (0_u32, 1_000_u32, 1.8_f32, 0.1_f32),
            (0, 180, 1.8, 0.1),
            (500, 2_000, 1.25, 0.05),
            (100, 180, 1.8, 0.1),
            (0, 1_000, 0.5, 0.004),
        ];
        for (start, target, accel, dt) in cases {
            if target <= start {
                continue;
            }
            let need =
                minimum_accel_distance(start, target, accel, dt, BoundedDistance::Finite(u32::MAX));
            let DistanceNeed::WithinAvailable(lower_um) = need else {
                panic!("expected a finite lower bound for {start}->{target}, got {need:?}");
            };
            let mut speed = start;
            let mut travelled_um = 0u128;
            for _ in 0..20_000 {
                if speed >= target {
                    break;
                }
                let (next, step_um) = forward_max_accel(speed, accel, dt).expect("step");
                travelled_um = travelled_um.saturating_add(u128::from(step_um));
                if next <= speed {
                    break;
                }
                speed = next;
            }
            assert!(
                speed >= target,
                "reference did not reach {target} from {start}, stopped at {speed}"
            );
            assert!(
                lower_um <= travelled_um,
                "lower bound {lower_um} exceeded forward distance {travelled_um} for {start}->{target}"
            );
        }
    }
}
