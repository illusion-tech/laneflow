// #808：直接列输入与稀疏通道；未使用通道不调用领域原语或生成失败。
#[derive(Clone, Copy)]
struct IidmBatch {
    speed: [f32; 4],
    desired: [f32; 4],
    gap: [f32; 4],
    min_gap: [f32; 4],
    headway: [f32; 4],
    accel: [f32; 4],
    delta: [f32; 4],
    gap_mask: [bool; 4],
    used: [bool; 4],
    fallback: [Option<IidmInput>; 4],
}

impl IidmBatch {
    const EMPTY: Self = Self {
        speed: [0.0; 4],
        desired: [1.0; 4],
        gap: [1.0; 4],
        min_gap: [0.0; 4],
        headway: [1.0; 4],
        accel: [1.0; 4],
        delta: [0.033; 4],
        gap_mask: [false; 4],
        used: [false; 4],
        fallback: [None; 4],
    };

    fn write(&mut self, lane: usize, input: IidmInput) {
        self.used[lane] = true;
        if !input.vectorizable() {
            self.fallback[lane] = Some(input);
            return;
        }
        self.fallback[lane] = None;
        self.speed[lane] = input.speed;
        self.desired[lane] = input.desired;
        self.gap[lane] = input.gap.unwrap_or(1.0);
        self.min_gap[lane] = input.min_gap;
        self.headway[lane] = input.headway;
        self.accel[lane] = input.accel;
        self.delta[lane] = input.delta;
        self.gap_mask[lane] = input.gap.is_some();
    }

    #[cfg(test)]
    fn gather(inputs: [IidmInput; 4]) -> Self {
        let mut batch = Self::EMPTY;
        for (lane, input) in inputs.into_iter().enumerate() {
            batch.write(lane, input);
        }
        batch
    }

    #[allow(dead_code)]
    #[inline(never)]
    fn scalar(&self) -> [Option<(f32, f32)>; 4] {
        std::array::from_fn(|i| {
            if !self.used[i] {
                return None;
            }
            if let Some(input) = self.fallback[i] {
                return input.scalar();
            }
            let q = (self.speed[i] / self.desired[i]).max(0.0);
            let speed_term = q.powi(4);
            let s_star = self.min_gap[i] + self.speed[i] * self.headway[i];
            let gap_term = if self.gap_mask[i] {
                (s_star / self.gap[i]).max(0.0).powi(2)
            } else {
                0.0
            };
            let accel = self.accel[i] * (1.0 - speed_term - gap_term);
            let next = (self.speed[i] + accel * self.delta[i])
                .max(0.0)
                .min(self.desired[i].max(0.0));
            let travel = ((self.speed[i] + next) * 0.5 * self.delta[i]).max(0.0);
            (travel.is_finite() && next.is_finite()).then_some((travel, next))
        })
    }

    #[allow(dead_code)]
    #[inline(never)]
    fn simd(&self) -> [Option<(f32, f32)>; 4] {
        let vector_lanes = (0..4)
            .filter(|&i| self.used[i] && self.fallback[i].is_none())
            .count();
        if vector_lanes < 2 {
            return self.scalar();
        }
        use wide::f32x4;
        let v = f32x4::from(self.speed);
        let d = f32x4::from(self.desired);
        let dt = f32x4::from(self.delta);
        let zero = f32x4::ZERO;
        let q = (v / d).max(zero);
        let q2 = q * q;
        let speed_term = q2 * q2;
        let s_star = f32x4::from(self.min_gap) + v * f32x4::from(self.headway);
        let gap_q = (s_star / f32x4::from(self.gap)).max(zero);
        let gap_term = gap_q * gap_q;
        let mask = f32x4::from(
            self.gap_mask
                .map(|yes| f32::from_bits(if yes { u32::MAX } else { 0 })),
        );
        let gap_term = mask.blend(gap_term, zero);
        let accel = f32x4::from(self.accel) * (f32x4::ONE - speed_term - gap_term);
        let next = (v + accel * dt).max(zero).min(d.max(zero));
        let travel = ((v + next) * f32x4::HALF * dt).max(zero);
        let next = next.to_array();
        let travel = travel.to_array();
        std::array::from_fn(|i| {
            if !self.used[i] {
                return None;
            }
            if let Some(input) = self.fallback[i] {
                return input.scalar();
            }
            (travel[i].is_finite() && next[i].is_finite()).then_some((travel[i], next[i]))
        })
    }
}

#[cfg(test)]
mod hot_iidm_tests {
    use super::*;
    fn bits(result: Option<(f32, f32)>) -> Option<(u32, u32)> {
        result.map(|(travel, speed)| (travel.to_bits(), speed.to_bits()))
    }

    #[test]
    fn sparse_masks_and_tail_lanes_preserve_scalar_contract() {
        for mask in 0..16 {
            for invalid in 0..=4 {
                let inputs = std::array::from_fn::<_, 4, _>(|lane| IidmInput {
                    speed: if lane == invalid {
                        f32::NAN
                    } else {
                        lane as f32 * 7.0
                    },
                    desired: 25.0,
                    gap: (lane % 2 == 0).then_some(12.0),
                    min_gap: 2.0,
                    headway: 1.5,
                    accel: 3.0,
                    ..IidmInput::EMPTY
                });
                let mut batch = IidmBatch::EMPTY;
                for (lane, input) in inputs.into_iter().enumerate() {
                    if mask & (1 << lane) != 0 {
                        batch.write(lane, input);
                    }
                }
                let expected = std::array::from_fn(|lane| {
                    if mask & (1 << lane) == 0 {
                        None
                    } else {
                        bits(inputs[lane].scalar())
                    }
                });
                assert_eq!(batch.scalar().map(bits), expected);
                assert_eq!(batch.simd().map(bits), expected);
            }
        }
    }
}
