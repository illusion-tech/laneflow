//! 将编制站位的 binary64 精确差舍入到最近偶数纳米。

use std::cmp::Ordering;

/// 确定切点归属时，先从绝对站位扣除走廊起点，再舍入一次。
/// 若先用浮点数相减或先取整两个绝对坐标，半纳米附近可能落在另一格。
pub(super) fn local_nanometres(origin: f64, station: f64) -> Option<u64> {
    if station < origin {
        return None;
    }
    let (start, start_fraction) = half_nanometres(origin)?;
    let (end, end_fraction) = half_nanometres(station)?;
    let half_units = end.checked_sub(start)?;
    let whole = half_units / 2;
    let up = half_units % 2 == 1
        && match end_fraction.cmp(&start_fraction) {
            Ordering::Less => false,
            Ordering::Equal => whole % 2 == 1,
            Ordering::Greater => true,
        };
    u64::try_from(whole + u128::from(up)).ok()
}

// 以半纳米为单位精确拆为整数和不足一单位的余数。余数用非零首位的
// 二进制指数和左对齐的有效位排序；零的排序码小于所有非零余数。
fn half_nanometres(meters: f64) -> Option<(u128, (i32, u128))> {
    if !meters.is_finite() || meters < 0.0 {
        return None;
    }
    let bits = meters.to_bits();
    let exponent = i32::try_from((bits >> 52) & 0x7ff).ok()?;
    let mantissa = (bits & ((1_u64 << 52) - 1)) | (u64::from(exponent != 0) << 52);
    let scaled = u128::from(mantissa) * 2_000_000_000;
    let power = exponent.max(1) - 1023 - 52;
    let zero = (i32::MIN, 0);
    if power >= 0 {
        let shift = u32::try_from(power).ok()?;
        if shift > scaled.leading_zeros() {
            return None;
        }
        return Some((scaled.checked_shl(shift)?, zero));
    }
    let shift = power.unsigned_abs();
    let integer = scaled.checked_shr(shift).unwrap_or(0);
    let remainder = scaled - integer.checked_shl(shift).unwrap_or(0);
    let fraction = if remainder == 0 {
        zero
    } else {
        let leading = remainder.leading_zeros();
        (power - i32::try_from(leading).ok()?, remainder << leading)
    };
    Some((integer, fraction))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subtracts_before_rounding_even_at_half_nanometres() {
        // 2^-10 米是精确的 976562.5 纳米。两侧的 binary64 紧邻值必须可区分。
        let half: f64 = 1.0 / 1024.0;
        assert_eq!(local_nanometres(0.0, half), Some(976_562));
        assert_eq!(local_nanometres(0.0, half.next_up()), Some(976_563));
        assert_eq!(local_nanometres(0.0, 3.0 * half), Some(2_929_688));
        assert_eq!(
            local_nanometres(f64::from_bits(1), 3.0 * half),
            Some(2_929_687)
        );
        assert_eq!(local_nanometres(half, 2.0 * half), Some(976_562));
        assert_eq!(
            local_nanometres(5_000_000.0, 5_000_000.0 + half),
            Some(976_562)
        );
        assert_eq!(local_nanometres(0.0, f64::MAX), None);
    }
}
