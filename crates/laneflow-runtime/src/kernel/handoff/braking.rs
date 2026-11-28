//! 动态横移保护的服务制动、两拍反应和最小间距。

/// 反应窗口为两个世界固定步长。
const REACTION_TICKS: u64 = 2;

/// 按进入者服务减速度的精确 binary32 值计算所需最小空间（微米）。
/// 两拍反应行程与最小间距均为整数微米，只有制动项需要向上取整。
/// 静止时只需最小间距。步长为零、缺少有效制动档位或距离溢出时无法证明。
pub(crate) fn service_stop_distance_um(
    speed_mm_s: u32,
    delta_ms: u64,
    service_decel_m_s2: f32,
    min_gap_mm: u32,
) -> Option<u64> {
    if delta_ms == 0 || !(service_decel_m_s2.is_finite() && service_decel_m_s2 > 0.0) {
        return None;
    }
    let reaction_um = u64::from(speed_mm_s)
        .checked_mul(delta_ms)?
        .checked_mul(REACTION_TICKS)?;
    let braking_um = laneflow_motion_kernel::stop_distance_um(speed_mm_s, service_decel_m_s2);
    reaction_um
        .checked_add(braking_um)?
        .checked_add(u64::from(min_gap_mm) * 1_000)
}

/// 从拍初整数毫米车头到预计停点的距离上界，已经计入本车跨拍行程余数。
pub(crate) fn stop_from_cursor_mm(stop_um: Option<u64>, carry_um: u16) -> Option<u64> {
    let travel = stop_um?;
    let fraction = travel % 1_000 + u64::from(carry_um);
    Some(travel / 1_000 + fraction.div_ceil(1_000))
}

#[cfg(test)]
mod tests {
    use super::{service_stop_distance_um, stop_from_cursor_mm};

    #[test]
    fn stop_from_cursor_rounds_after_adding_the_motion_remainder() {
        assert_eq!(stop_from_cursor_mm(Some(15_611_112), 888), Some(15_612));
        assert_eq!(stop_from_cursor_mm(Some(15_611_112), 889), Some(15_613));
        assert_eq!(stop_from_cursor_mm(Some(0), 0), Some(0));
        assert_eq!(stop_from_cursor_mm(None, 999), None);
        let max = (u128::from(u64::MAX) + 999).div_ceil(1_000);
        assert_eq!(
            stop_from_cursor_mm(Some(u64::MAX), 999).map(u128::from),
            Some(max)
        );
    }

    #[test]
    fn two_reaction_ticks_and_service_braking_leave_the_minimum_gap() {
        for (speed, distance) in [(10_000, 15_611_112), (13_900, 26_747_778)] {
            assert_eq!(
                service_stop_distance_um(speed, 100, 4.5, 2_500),
                Some(distance)
            );
        }
    }

    #[test]
    fn service_stop_preserves_binary32_profile_precision_and_fails_closed_on_overflow() {
        // 4.501_f32 略小于 4.501。若先用 f32 乘 1,000，会把制动能力放大到
        // 4,501 mm/s²，错误地少算 1 微米。
        assert_eq!(
            service_stop_distance_um(4_501, 100, 4.501, 2_500),
            Some(5_650_701)
        );
        assert_eq!(service_stop_distance_um(0, 100, 4.5, 17), Some(17_000));
        for invalid in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(service_stop_distance_um(0, 100, invalid, 17), None);
            assert_eq!(service_stop_distance_um(1, 100, invalid, 17), None);
        }
        assert_eq!(service_stop_distance_um(1, 0, 1.0, 0), None);
        assert_eq!(service_stop_distance_um(1, 100, f32::from_bits(1), 0), None);
        let delta = u64::MAX / 2;
        assert_eq!(service_stop_distance_um(1, delta, 0.5, 0), Some(u64::MAX));
        assert_eq!(service_stop_distance_um(1, delta, 0.5, 1), None);
        assert_eq!(service_stop_distance_um(1, delta + 1, 0.5, 0), None);
        assert_eq!(service_stop_distance_um(u32::MAX, u64::MAX, 0.5, 0), None);
    }
}
