//! 将来源站位扣除走廊起点后，按共享站位格量化一次。

use laneflow_static_contract::StationSpan;

/// 对两个有序非负来源站位的差、或一个有符号实体偏移取最近偶数毫米。
/// 差值拆成和与舍入余项，FMA 比较真实乘积和相邻半格，无需拆分浮点位。
pub(super) fn rounded_difference_mm(end: f64, start: f64) -> Option<i64> {
    let delta = end - start;
    let scaled = delta * 1000.0;
    if !scaled.is_finite() || scaled.abs() > StationSpan::MAX_DISTANCE_MM as f64 + 1.0 {
        return None;
    }
    // |end| ≥ |start|；FastTwoSum 确保 delta + tail 与真实的差相同。
    let tail = (end - delta) - start;
    let half = scaled.floor() + 0.5;
    let from_half = tail.mul_add(1000.0, delta.mul_add(1000.0, -half));
    let lower = half.floor() as i64;
    Some(if from_half < 0.0 {
        lower
    } else if from_half > 0.0 {
        lower + 1
    } else {
        lower + (lower & 1)
    })
}

/// 规范站位以毫米计。超出查询算术可表示的局部范围时拒绝编制。
pub(super) fn local_millimetres(origin: f64, station: f64) -> Option<u64> {
    if !origin.is_finite() || origin < 0.0 || station < origin {
        return None;
    }
    let mm = u64::try_from(rounded_difference_mm(station, origin)?).ok()?;
    (mm <= StationSpan::MAX_DISTANCE_MM).then_some(mm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_millimetres_rounds_after_subtraction() {
        assert_eq!(local_millimetres(0.0, 0.0625), Some(62));
        assert_eq!(local_millimetres(5_000_000.0, 5_000_000.187_5), Some(188));
        // binary64 的 0.0005 略高于半格，直接乘 1000 却得到恰好 0.5。
        assert_eq!(local_millimetres(0.0, 0.0005), Some(1));
        // 非零走廊起点使真实差略低于 187.5 毫米，binary64 的差会舍入到中点。
        assert_eq!(local_millimetres(f64::EPSILON / 16.0, 0.1875), Some(187));
        assert_eq!(local_millimetres(0.0, 0.06249), Some(62));
        assert_eq!(local_millimetres(0.0, 0.06251), Some(63));
        assert_eq!(local_millimetres(2.0, 1.0), None);
        assert_eq!(local_millimetres(0.0, f64::MAX), None);
    }
}
