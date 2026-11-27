//! 换道预约的服务制动证明（`lane-change-minimal-contract.md` §5.2）。

/// 反应窗口为两个世界固定步长。
pub(crate) const TRANSITION_REACTION_TICKS: u64 = 2;

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
        .checked_mul(TRANSITION_REACTION_TICKS)?;
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

/// P4 进入裁决（#1199 P4 接线；合同 §5.2）。`MotionReach` 仅为本拍位移
/// 上界——当拍判定不可达只可跳过当拍、存续期逐拍重证（`ReachabilityDeferred`
/// 不得卸除存续期义务）；任一潜在进入者不可安全让行 → 申请 no-grant。
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TransitionEntryVerdict {
    /// 本拍位移上界内不可达（MotionReach）：跳过当拍，存续期重证。
    ReachabilityDeferred,
    /// 潜在进入者存在不可安全让行（具名四项判据失败）→ no-grant。
    NoGrantUnyieldable,
    /// 可达且全部潜在进入者可证让行。
    Enter,
}

/// P4 潜在进入者证明（#1199 P4 接线；§5.2 具名判据 + MotionReach 跳拍重证）。
///
/// `potential_enterers` =（距被预约区间毫米、当前速度毫米/秒）实际车体记录；
/// 判据为具名四项停止距离（`service_stop_distance_um`，A4 冻结），不引用
/// 笼统 profile 制动能力（§7-R2 失败面）。申请人本拍可达性 = 距进入点 ≤
/// 本拍位移上界（速度 × 步长），不可达即 `ReachabilityDeferred`（跳拍重证）。
#[cfg(test)]
pub(crate) fn evaluate_transition_entry(
    applicant_distance_to_entry_mm: u64,
    applicant_speed_mm_s: u64,
    potential_enterers: &[(u32, u32)],
    delta_ms: u64,
    service_decel_m_s2: f32,
    min_gap_mm: u32,
) -> TransitionEntryVerdict {
    let reach_mm = u128::from(applicant_speed_mm_s) * u128::from(delta_ms) / 1_000;
    if u128::from(applicant_distance_to_entry_mm) > reach_mm {
        return TransitionEntryVerdict::ReachabilityDeferred;
    }
    for &(distance_mm, speed_mm_s) in potential_enterers {
        if service_stop_distance_um(speed_mm_s, delta_ms, service_decel_m_s2, min_gap_mm)
            .is_none_or(|stop_um| stop_um > u64::from(distance_mm) * 1_000)
        {
            return TransitionEntryVerdict::NoGrantUnyieldable;
        }
    }
    TransitionEntryVerdict::Enter
}

#[cfg(test)]
mod tests {
    use super::{
        TRANSITION_REACTION_TICKS, TransitionEntryVerdict, evaluate_transition_entry,
        service_stop_distance_um, stop_from_cursor_mm,
    };

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

    /// 钉版参数（sample v7 `safety_proof`/`profiles`，参数自 v3 起未变）：固定步长 100 ms、
    /// 服务减速度 4500 mm/s²、最小间隙 2500 mm、r = 2 拍。
    const DELTA_MS: u64 = 100;
    const SERVICE_DECEL_M_S2: f32 = 4.5;
    const MIN_GAP_MM: u32 = 2_500;

    #[test]
    fn r2_named_stop_distance_matches_pinned_critical_tick() {
        // §7-R2 关键拍钉版：d_stop = 13.9×0.2 + 13.9²/(2×4.5) + 2.5 ≈ 26.75 m。
        assert_eq!(TRANSITION_REACTION_TICKS, 2);
        let stop =
            service_stop_distance_um(13_900, DELTA_MS, SERVICE_DECEL_M_S2, MIN_GAP_MM).unwrap();
        assert_eq!(stop, 26_747_778);
        // s4 tick 0：后车距被预约区间 8 m < 26.75 m，不可安全让行。
        assert!(8_000_000 < stop);
    }

    #[test]
    fn r1_named_stop_distance_matches_pinned_competitors() {
        // §7-R1 钉版：d_stop = 10×0.2 + 10²/(2×4.5) + 2.5 ≈ 15.61 m。
        let stop =
            service_stop_distance_um(10_000, DELTA_MS, SERVICE_DECEL_M_S2, MIN_GAP_MM).unwrap();
        assert_eq!(stop, 15_611_112);
        // s6：直行车距被预约区间 8 m < 15.61 m，不可安全让行（换道 no-grant 前提）。
        assert!(8_000_000 < stop);
        // s12 前提（yieldability）：19 m / 27 m 均 ≥ 15.61 m，双方均可安全让行。
        assert!(stop <= 19_000_000);
        assert!(stop <= 27_000_000);
    }

    #[test]
    fn p4_s4_critical_tick_no_grants_on_unyieldable_follower() {
        // §7-R2 关键拍钉版（s4 tick 0）：换道车 B0_up@7000（边长 8000，距
        // 进入点 1000 mm）本拍可达（上界 1000 mm）；目标后车距被预约区间
        // 8 m、13.9 m/s 不可安全让行 → no-grant。
        assert_eq!(
            evaluate_transition_entry(
                1_000,
                10_000,
                &[(8_000, 13_900)],
                DELTA_MS,
                SERVICE_DECEL_M_S2,
                MIN_GAP_MM,
            ),
            TransitionEntryVerdict::NoGrantUnyieldable
        );
    }

    #[test]
    fn p4_s12_reachability_defers_and_proofs_revalidate_per_tick() {
        // s12 tick 0 钉版：距 T01 入口 19 m/27 m，本拍位移上界 1.39 m
        // （13.9 m/s × 0.1 s 上界口径）→ 双方均不可能本拍进入，跳拍重证；
        // 前提层（双方均可安全让行）下证明均通过 → 到达拍才进入（层②）。
        for distance in [19_000_u64, 27_000] {
            assert_eq!(
                evaluate_transition_entry(
                    distance,
                    10_000,
                    &[(27_000, 10_000), (19_000, 10_000)],
                    DELTA_MS,
                    SERVICE_DECEL_M_S2,
                    MIN_GAP_MM,
                ),
                TransitionEntryVerdict::ReachabilityDeferred
            );
        }
        // 到达拍（进入点 ≤ 上界）前提成立时 Enter（两层断言的层②铺垫）。
        assert_eq!(
            evaluate_transition_entry(
                1_000,
                10_000,
                &[(27_000, 10_000), (19_000, 10_000)],
                DELTA_MS,
                SERVICE_DECEL_M_S2,
                MIN_GAP_MM,
            ),
            TransitionEntryVerdict::Enter
        );
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
