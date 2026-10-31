//! 最小换道过渡执行判据（#1199 S4；合同 `lane-change-minimal-contract.md`
//! §5.2，A4 具名四项冻结；§7 R1/R2 钉版）。
//!
//! 本模块先落**具名参数判据**（R2 失败面明令：不得笼统引用 profile 制动
//! 能力），P3/P4 预约接线（staged→committed 复用，G2 裁定①）在其上集成。
#![cfg_attr(not(test), allow(dead_code))]

/// 反应窗口拍数 `r`（安全证明冻结：r = 2 拍；`fixed_delta_time_ms = 100`）。
pub(crate) const TRANSITION_REACTION_TICKS: u64 = 2;

/// 具名四项停止距离（毫米，向下取整）：
/// `d_stop = v·(r·Δt) + v²/(2·a_service) + min_gap`。
///
/// 参数具名（A4 冻结，§7-R2 失败面「笼统引用 profile 制动能力」即违例）：
/// `speed_mm_s` 当前速度、`delta_ms` 固定步长、`service_decel_mm_s2` 服务
/// 减速度、`min_gap_mm` 最小间隙。紧急减速度不入判据（仅作「不得迫使
/// 紧急制动」的外侧界断言）。
///
/// # Panics
///
/// 除零保护：`delta_ms` 与 `service_decel_mm_s2` 为 0 时按 0 距离项处理，
/// 不 panic（判据失败关闭方向由调用方的比较方向保证）。
pub(crate) fn stop_distance_millimetres(
    speed_mm_s: u64,
    delta_ms: u64,
    service_decel_mm_s2: u64,
    min_gap_mm: u64,
) -> u64 {
    let speed = u128::from(speed_mm_s);
    let reaction_mm = speed * u128::from(TRANSITION_REACTION_TICKS) * u128::from(delta_ms) / 1_000;
    let braking_mm = if service_decel_mm_s2 == 0 {
        u128::from(u64::MAX)
    } else {
        speed * speed / (2 * u128::from(service_decel_mm_s2))
    };
    let total = reaction_mm + braking_mm + u128::from(min_gap_mm);
    u64::try_from(total).unwrap_or(u64::MAX)
}

/// 让行可证性（§5.2 具名判据）：车辆能在被预约区间前安全停住 =
/// 距区间距离 ≥ `d_stop`。任一潜在进入者返回 `false` → 申请 no-grant。
pub(crate) fn yieldable_by_stopping(
    distance_to_interval_mm: u64,
    speed_mm_s: u64,
    delta_ms: u64,
    service_decel_mm_s2: u64,
    min_gap_mm: u64,
) -> bool {
    distance_to_interval_mm
        >= stop_distance_millimetres(speed_mm_s, delta_ms, service_decel_mm_s2, min_gap_mm)
}

#[cfg(test)]
mod tests {
    use super::{TRANSITION_REACTION_TICKS, stop_distance_millimetres, yieldable_by_stopping};

    /// 钉版参数（sample v5 `safety_proof`/`profiles`）：固定步长 100 ms、
    /// 服务减速度 4500 mm/s²、最小间隙 2500 mm、r = 2 拍。
    const DELTA_MS: u64 = 100;
    const SERVICE_DECEL_MM_S2: u64 = 4_500;
    const MIN_GAP_MM: u64 = 2_500;

    #[test]
    fn r2_named_stop_distance_matches_pinned_critical_tick() {
        // §7-R2 关键拍钉版：d_stop = 13.9×0.2 + 13.9²/(2×4.5) + 2.5 ≈ 26.75 m。
        assert_eq!(TRANSITION_REACTION_TICKS, 2);
        assert_eq!(
            stop_distance_millimetres(13_900, DELTA_MS, SERVICE_DECEL_MM_S2, MIN_GAP_MM),
            26_747
        );
        // s4 tick 0：后车距被预约区间 8 m < 26.75 m，不可安全让行。
        assert!(!yieldable_by_stopping(
            8_000,
            13_900,
            DELTA_MS,
            SERVICE_DECEL_MM_S2,
            MIN_GAP_MM
        ));
    }

    #[test]
    fn r1_named_stop_distance_matches_pinned_competitors() {
        // §7-R1 钉版：d_stop = 10×0.2 + 10²/(2×4.5) + 2.5 ≈ 15.61 m。
        assert_eq!(
            stop_distance_millimetres(10_000, DELTA_MS, SERVICE_DECEL_MM_S2, MIN_GAP_MM),
            15_611
        );
        // s6：直行车距被预约区间 8 m < 15.61 m，不可安全让行（换道 no-grant 前提）。
        assert!(!yieldable_by_stopping(
            8_000,
            10_000,
            DELTA_MS,
            SERVICE_DECEL_MM_S2,
            MIN_GAP_MM
        ));
        // s12 前提（yieldability）：19 m / 27 m 均 ≥ 15.61 m，双方均可安全让行。
        assert!(yieldable_by_stopping(
            19_000,
            10_000,
            DELTA_MS,
            SERVICE_DECEL_MM_S2,
            MIN_GAP_MM
        ));
        assert!(yieldable_by_stopping(
            27_000,
            10_000,
            DELTA_MS,
            SERVICE_DECEL_MM_S2,
            MIN_GAP_MM
        ));
    }

    #[test]
    fn control_branch_is_yieldable_beyond_stop_distance() {
        // §7-R2 对照分支（防证明过保守恒拒绝）：处于 d_stop + min_gap 之外
        // 的后车必须可让行。
        let beyond = stop_distance_millimetres(13_900, DELTA_MS, SERVICE_DECEL_MM_S2, MIN_GAP_MM)
            + MIN_GAP_MM;
        assert!(yieldable_by_stopping(
            beyond,
            13_900,
            DELTA_MS,
            SERVICE_DECEL_MM_S2,
            MIN_GAP_MM
        ));
    }
}
