use core::cmp::Ordering;

const EXACT_BASE_EXPONENT: i32 = -300;
const EXACT_LIMB_COUNT: usize = 6;
const MAX_SUPPORTED_OPERAND: f32 = 65_536.0;
const MAX_SEARCH_RESULT: f32 = 131_072.0;
const MAX_SEARCH_RESULT_BITS: u32 = MAX_SEARCH_RESULT.to_bits();

#[derive(Clone, Copy)]
struct ExactPositive([u64; EXACT_LIMB_COUNT]);

impl ExactPositive {
    const ZERO: Self = Self([0; EXACT_LIMB_COUNT]);

    fn hypot_square(left: f32, right: f32) -> Self {
        let mut result = Self::ZERO;
        result.add_f32_square(left);
        result.add_f32_square(right);
        result
    }

    fn f32_square(value: f32) -> Self {
        let mut result = Self::ZERO;
        result.add_f32_square(value);
        result
    }

    fn midpoint_square(lower: f32, upper: f32) -> Self {
        let (lower_significand, lower_exponent) = decompose_f32(lower);
        let (upper_significand, upper_exponent) = decompose_f32(upper);
        let common_exponent = if lower_significand == 0 {
            upper_exponent
        } else {
            lower_exponent.min(upper_exponent)
        };
        let lower_shift =
            u32::try_from(lower_exponent - common_exponent).expect("adjacent f32 exponent order");
        let upper_shift =
            u32::try_from(upper_exponent - common_exponent).expect("adjacent f32 exponent order");
        let midpoint_significand = (u64::from(lower_significand) << lower_shift)
            + (u64::from(upper_significand) << upper_shift);
        let midpoint_exponent = common_exponent - 1;

        let mut result = Self::ZERO;
        result.add_shifted(
            midpoint_significand * midpoint_significand,
            midpoint_exponent * 2,
        );
        result
    }

    fn add_f32_square(&mut self, value: f32) {
        let (significand, exponent) = decompose_f32(value.abs());
        self.add_shifted(
            u64::from(significand) * u64::from(significand),
            exponent * 2,
        );
    }

    fn add_shifted(&mut self, significand: u64, exponent: i32) {
        if significand == 0 {
            return;
        }
        let shift =
            usize::try_from(exponent - EXACT_BASE_EXPONENT).expect("supported f32 square exponent");
        let word = shift / 64;
        let bit = shift % 64;
        let shifted = u128::from(significand) << bit;
        self.add_limb(word, shifted as u64);
        self.add_limb(word + 1, (shifted >> 64) as u64);
    }

    fn add_limb(&mut self, mut index: usize, mut value: u64) {
        while value != 0 {
            let limb = self
                .0
                .get_mut(index)
                .expect("supported f32 square fits fixed exact accumulator");
            let (sum, carry) = limb.overflowing_add(value);
            *limb = sum;
            value = u64::from(carry);
            index += 1;
        }
    }

    fn compare(self, other: Self) -> Ordering {
        for index in (0..EXACT_LIMB_COUNT).rev() {
            match self.0[index].cmp(&other.0[index]) {
                Ordering::Equal => {}
                ordering => return ordering,
            }
        }
        Ordering::Equal
    }
}

/// 返回 `RN32(sqrt(Exact(left)^2 + Exact(right)^2))`。
///
/// 平台 `f64::sqrt` 只提供通常命中正确相邻值的初始猜测；最终选择由固定宽度整数对
/// 精确平方与两个舍入中点的比较决定。异常初始猜测会退回精确二分，因此不影响结果。
pub(crate) fn hypot_rte_f32(left: f32, right: f32) -> f32 {
    debug_assert!(left.is_finite() && right.is_finite());
    debug_assert!(left.abs() <= MAX_SUPPORTED_OPERAND);
    debug_assert!(right.abs() <= MAX_SUPPORTED_OPERAND);

    let exact = ExactPositive::hypot_square(left, right);
    if exact.compare(ExactPositive::ZERO) == Ordering::Equal {
        return 0.0;
    }

    let left_f64 = f64::from(left);
    let right_f64 = f64::from(right);
    let approximate = (left_f64 * left_f64 + right_f64 * right_f64).sqrt() as f32;
    if approximate.is_finite() && approximate <= MAX_SEARCH_RESULT {
        let mut candidate_bits = approximate.to_bits();
        for _ in 0..4 {
            match candidate_direction(exact, candidate_bits) {
                Ordering::Equal => return f32::from_bits(candidate_bits),
                Ordering::Less => candidate_bits -= 1,
                Ordering::Greater => candidate_bits += 1,
            }
        }
    }

    binary_search_rounded(exact)
}

fn candidate_direction(exact: ExactPositive, candidate_bits: u32) -> Ordering {
    if candidate_bits > 0 {
        let lower_boundary = ExactPositive::midpoint_square(
            f32::from_bits(candidate_bits - 1),
            f32::from_bits(candidate_bits),
        );
        let comparison = exact.compare(lower_boundary);
        if comparison == Ordering::Less
            || (comparison == Ordering::Equal && candidate_bits & 1 != 0)
        {
            return Ordering::Less;
        }
    }

    let upper_boundary = ExactPositive::midpoint_square(
        f32::from_bits(candidate_bits),
        f32::from_bits(candidate_bits + 1),
    );
    let comparison = exact.compare(upper_boundary);
    if comparison == Ordering::Greater || (comparison == Ordering::Equal && candidate_bits & 1 != 0)
    {
        return Ordering::Greater;
    }
    Ordering::Equal
}

#[cold]
fn binary_search_rounded(exact: ExactPositive) -> f32 {
    let mut lower_bits = 0_u32;
    let mut upper_bits = MAX_SEARCH_RESULT_BITS;
    while lower_bits < upper_bits {
        let middle = lower_bits + (upper_bits - lower_bits).div_ceil(2);
        if ExactPositive::f32_square(f32::from_bits(middle)).compare(exact) != Ordering::Greater {
            lower_bits = middle;
        } else {
            upper_bits = middle - 1;
        }
    }

    let lower = f32::from_bits(lower_bits);
    if ExactPositive::f32_square(lower).compare(exact) == Ordering::Equal {
        return lower;
    }
    let boundary = ExactPositive::midpoint_square(lower, f32::from_bits(lower_bits + 1));
    match exact.compare(boundary) {
        Ordering::Less => lower,
        Ordering::Greater => f32::from_bits(lower_bits + 1),
        Ordering::Equal if lower_bits & 1 == 0 => lower,
        Ordering::Equal => f32::from_bits(lower_bits + 1),
    }
}

fn decompose_f32(value: f32) -> (u32, i32) {
    let bits = value.to_bits() & 0x7fff_ffff;
    let exponent_bits = (bits >> 23) & 0xff;
    let fraction = bits & 0x007f_ffff;
    debug_assert_ne!(exponent_bits, 0xff);
    if exponent_bits == 0 {
        (fraction, -149)
    } else {
        (
            0x0080_0000 | fraction,
            i32::try_from(exponent_bits).expect("f32 exponent fits i32") - 150,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::hypot_rte_f32;

    #[test]
    fn correctly_rounds_exact_values_subnormals_and_midpoint_ties() {
        assert_eq!(hypot_rte_f32(-3.0, 4.0).to_bits(), 5.0_f32.to_bits());
        assert_eq!(hypot_rte_f32(-0.0, 0.0).to_bits(), 0.0_f32.to_bits());
        assert_eq!(
            hypot_rte_f32(f32::from_bits(1), 0.0).to_bits(),
            f32::from_bits(1).to_bits()
        );

        // (16_777_215, 8_192, 16_777_217) / 1_024 是精确勾股三元组；斜边
        // 正好位于 16_384.0 与其下一 binary32 之间，ties-to-even 选择前者。
        let leg = f32::from_bits(16_384.0_f32.to_bits() - 1);
        assert_eq!(hypot_rte_f32(leg, 8.0).to_bits(), 16_384.0_f32.to_bits());
        assert_eq!(
            hypot_rte_f32(leg, f32::from_bits(8.0_f32.to_bits() + 1)).to_bits(),
            16_384.0_f32.to_bits() + 1
        );
    }
}
