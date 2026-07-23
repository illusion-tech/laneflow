//! v0.8 replay contract 使用的 SplitMix64。

const INCREMENT: u64 = 0x9E37_79B9_7F4A_7C15;
const MULTIPLIER_1: u64 = 0xBF58_476D_1CE4_E5B9;
const MULTIPLIER_2: u64 = 0x94D0_49BB_1331_11EB;

/// v0.8 signalized-corridor replay contract 使用的显式 PRNG state。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// 直接以 caller seed 初始化；零 seed 合法。
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// 返回当前内部 state，供 replay/验证记录使用。
    pub const fn state(self) -> u64 {
        self.state
    }

    /// 生成下一个冻结的 SplitMix64 值。
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(INCREMENT);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(MULTIPLIER_1);
        value = (value ^ (value >> 27)).wrapping_mul(MULTIPLIER_2);
        value ^ (value >> 31)
    }

    /// 使用 rejection sampling 返回 `0..bound` 的无偏值。
    ///
    /// # Panics
    ///
    /// `bound == 0` 表示调用方违反已规范化 catalog invariant。
    pub fn uniform(&mut self, bound: u64) -> u64 {
        assert!(bound > 0, "uniform bound must be positive");
        let threshold = bound.wrapping_neg() % bound;
        loop {
            let draw = self.next_u64();
            if draw >= threshold {
                return draw % bound;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SplitMix64;

    #[test]
    fn seed_zero_golden_sequence_is_stable() {
        let mut rng = SplitMix64::new(0);
        assert_eq!(
            [
                rng.next_u64(),
                rng.next_u64(),
                rng.next_u64(),
                rng.next_u64(),
            ],
            [
                0xE220_A839_7B1D_CDAF,
                0x6E78_9E6A_A1B9_65F4,
                0x06C4_5D18_8009_454F,
                0xF88B_B8A8_724C_81EC,
            ]
        );
    }

    #[test]
    fn bounded_sequences_are_stable() {
        let mut two = SplitMix64::new(7);
        let mut three = SplitMix64::new(7);
        let mut five = SplitMix64::new(7);
        assert_eq!(
            (0..8).map(|_| two.uniform(2)).collect::<Vec<_>>(),
            [1, 0, 0, 1, 0, 1, 0, 0]
        );
        assert_eq!(
            (0..8).map(|_| three.uniform(3)).collect::<Vec<_>>(),
            [0, 0, 0, 0, 1, 0, 1, 0]
        );
        assert_eq!(
            (0..8).map(|_| five.uniform(5)).collect::<Vec<_>>(),
            [2, 4, 1, 3, 4, 0, 3, 2]
        );
    }
}
