//! 已提交毫米与 IIDM 瞬时 SI 之间的换算。米制只作瞬时，不得回写。
//! 跟车行走窗用 `ceil_mm`：溢出饱和到 `u32::MAX`，禁止包成更短视距。

pub(crate) fn round_mm(meters: f64) -> Option<u32> {
    if !meters.is_finite() {
        return None;
    }
    let mm = (meters.max(0.0) * 1_000.0).round_ties_even();
    if mm > f64::from(u32::MAX) {
        return None;
    }
    Some(mm as u32)
}

/// 跟车前视：SI 有限且非负后向上取整到毫米。溢出饱和到 `u32::MAX`，禁止缩短视距。
pub(crate) fn ceil_mm(meters: f64) -> Option<u32> {
    if !meters.is_finite() || meters < 0.0 {
        return None;
    }
    let mm = (meters * 1_000.0).ceil();
    if mm > f64::from(u32::MAX) {
        return Some(u32::MAX);
    }
    Some(mm as u32)
}

pub(crate) fn round_um(meters: f64) -> Option<u64> {
    if !meters.is_finite() {
        return None;
    }
    let um = (meters.max(0.0) * 1_000_000.0).round_ties_even();
    if um < 0.0 || um > u64::MAX as f64 {
        return None;
    }
    Some(um as u64)
}
