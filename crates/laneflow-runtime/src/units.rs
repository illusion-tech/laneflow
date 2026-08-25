//! 已提交毫米与 IIDM 瞬时 SI 之间的换算。米制只作瞬时，不得回写。

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
