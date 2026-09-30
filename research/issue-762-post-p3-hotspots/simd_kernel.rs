// #805：同布局标量与 SIMD 共享四车输入；无近似倒数、FMA 或额外量化。
#[derive(Clone, Copy, Debug)]
struct IidmInput {
    speed: f32,
    desired: f32,
    gap: Option<f32>,
    min_gap: f32,
    headway: f32,
    accel: f32,
    comfort: f32,
    emergency: f32,
    delta: f32,
}

impl IidmInput {
    const EMPTY: Self = Self {
        speed: 0.0, desired: 1.0, gap: None, min_gap: 0.0,
        headway: 1.0, accel: 1.0, comfort: 1.0, emergency: 1.0, delta: 0.033,
    };

    fn scalar(self) -> Option<(f32, f32)> {
        iidm_step(self.speed, self.desired, self.gap, self.min_gap, self.headway,
            self.accel, self.comfort, self.emergency, self.delta)
    }

    fn vectorizable(self) -> bool {
        [self.speed, self.desired, self.min_gap, self.headway, self.accel,
            self.comfort, self.emergency, self.delta].into_iter().all(f32::is_finite)
            && self.speed >= 0.0 && self.desired > 0.0 && self.delta > 0.0
            && self.accel > 0.0 && self.comfort > 0.0 && self.emergency > 0.0
            && self.gap.is_none_or(|gap| gap.is_finite() && gap > 0.0)
    }
}

#[derive(Clone, Copy)]
struct IidmBatch {
    speed: [f32; 4], desired: [f32; 4], gap: [f32; 4], min_gap: [f32; 4],
    headway: [f32; 4], accel: [f32; 4], delta: [f32; 4], gap_mask: [bool; 4],
    fallback: [Option<IidmInput>; 4],
}

impl IidmBatch {
    fn gather(inputs: [IidmInput; 4]) -> Self {
        let fallback = inputs.map(|input| (!input.vectorizable()).then_some(input));
        let safe = std::array::from_fn::<_, 4, _>(|i| {
            if fallback[i].is_some() { IidmInput::EMPTY } else { inputs[i] }
        });
        Self {
            speed: safe.map(|x| x.speed), desired: safe.map(|x| x.desired),
            gap: safe.map(|x| x.gap.unwrap_or(1.0)), min_gap: safe.map(|x| x.min_gap),
            headway: safe.map(|x| x.headway), accel: safe.map(|x| x.accel),
            delta: safe.map(|x| x.delta), gap_mask: safe.map(|x| x.gap.is_some()),
            fallback,
        }
    }

    #[allow(dead_code)]
    #[inline(never)]
    fn scalar(self) -> [Option<(f32, f32)>; 4] {
        std::array::from_fn(|i| {
            if let Some(input) = self.fallback[i] { return input.scalar(); }
            let q = (self.speed[i] / self.desired[i]).max(0.0);
            let speed_term = q.powi(4);
            let s_star = self.min_gap[i] + self.speed[i] * self.headway[i];
            let gap_term = if self.gap_mask[i] {
                (s_star / self.gap[i]).max(0.0).powi(2)
            } else { 0.0 };
            let accel = self.accel[i] * (1.0 - speed_term - gap_term);
            let next = (self.speed[i] + accel * self.delta[i]).max(0.0)
                .min(self.desired[i].max(0.0));
            let travel = ((self.speed[i] + next) * 0.5 * self.delta[i]).max(0.0);
            (travel.is_finite() && next.is_finite()).then_some((travel, next))
        })
    }

    #[inline(never)]
    #[allow(dead_code)]
    fn simd(self) -> [Option<(f32, f32)>; 4] {
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
        let mask = f32x4::from(self.gap_mask.map(|yes| {
            f32::from_bits(if yes { u32::MAX } else { 0 })
        }));
        let gap_term = mask.blend(gap_term, zero);
        let accel = f32x4::from(self.accel) * (f32x4::ONE - speed_term - gap_term);
        let next = (v + accel * dt).max(zero).min(d.max(zero));
        let travel = ((v + next) * f32x4::HALF * dt).max(zero);
        let next = next.to_array();
        let travel = travel.to_array();
        std::array::from_fn(|i| {
            if let Some(input) = self.fallback[i] { return input.scalar(); }
            (travel[i].is_finite() && next[i].is_finite()).then_some((travel[i], next[i]))
        })
    }
}

#[cfg(test)]
mod iidm_simd_tests {
    use super::*;
    fn bits(result: Option<(f32, f32)>) -> Option<(u32, u32)> {
        result.map(|(travel, speed)| (travel.to_bits(), speed.to_bits()))
    }

    #[test]
    fn mixed_boundaries_and_nonfinite_lanes_preserve_scalar_bits() {
        let values = [0.0, -0.0, 0.001, 0.033, 1.0, 25.0, 100.0,
            f32::MAX, f32::MIN_POSITIVE, f32::NAN, f32::INFINITY, -1.0];
        for &speed in &values {
            for &desired in &values {
                for gap in [None, Some(0.0), Some(-1.0), Some(0.001), Some(10.0),
                    Some(f32::NAN), Some(f32::INFINITY)] {
                    let a = IidmInput { speed, desired, gap, ..IidmInput::EMPTY };
                    let inputs = [a, IidmInput::EMPTY,
                        IidmInput { accel: 0.0, ..a }, IidmInput { delta: f32::NAN, ..a }];
                    let batch = IidmBatch::gather(inputs);
                    let reference = inputs.map(|x| bits(x.scalar()));
                    assert_eq!(batch.scalar().map(bits), reference);
                    assert_eq!(batch.simd().map(bits), reference);
                }
            }
        }
    }

    #[test]
    fn million_valid_inputs_preserve_scalar_bits() {
        let mut state = 0x1234_5678_u32;
        let mut unit = || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 8) as f32 / 16_777_216.0
        };
        for _ in 0..250_000 {
            let inputs = std::array::from_fn(|_| IidmInput {
                speed: unit() * 100.0, desired: unit() * 100.0,
                gap: Some(0.001 + unit() * 1_000.0), min_gap: unit() * 128.0,
                headway: 0.001 + unit() * 60.0, accel: 0.5 + unit() * 49.5,
                comfort: 0.5 + unit() * 49.5, emergency: 50.0,
                delta: 0.004 + unit() * 0.996,
            });
            let batch = IidmBatch::gather(inputs);
            let reference = inputs.map(|x| bits(x.scalar()));
            assert_eq!(batch.scalar().map(bits), reference);
            assert_eq!(batch.simd().map(bits), reference);
        }
    }

    #[test]
    #[ignore = "isolated kernel timing; run after all builds"]
    fn iidm_kernel_bench() {
        let inputs: Vec<[IidmInput; 4]> = (0..1_024).map(|batch| {
            std::array::from_fn(|lane| IidmInput {
                speed: ((batch * 4 + lane) % 100) as f32 * 0.2,
                desired: 25.0, gap: (lane % 2 == 0).then_some(15.0 + batch as f32 * 0.01),
                min_gap: 2.0, headway: 1.5, accel: 3.0,
                ..IidmInput::EMPTY
            })
        }).collect();
        let packed: Vec<_> = inputs.iter().copied().map(IidmBatch::gather).collect();
        for group in 0..3 {
            for offset in 0..3 {
                let arm = (group + offset) % 3;
                let mut digest = 0_u64;
                let start = std::time::Instant::now();
                for _ in 0..100 {
                    for i in 0..inputs.len() {
                        let result = match arm {
                            0 => std::hint::black_box(inputs[i]).map(IidmInput::scalar),
                            1 => std::hint::black_box(packed[i]).scalar(),
                            _ => std::hint::black_box(packed[i]).simd(),
                        };
                        for (travel, speed) in result.into_iter().flatten() {
                            digest = digest.wrapping_add(u64::from(travel.to_bits())
                                ^ u64::from(speed.to_bits()));
                        }
                    }
                }
                println!("IIDM_BENCH {{\"group\":{group},\"arm\":{arm},\"vehicles\":409600,\"elapsed_ns\":{},\"digest\":{digest}}}", start.elapsed().as_nanos());
            }
        }
        println!("IIDM_LAYOUT {{\"input_bytes\":{},\"batch_bytes\":{},\"prepared_bytes\":{},\"prepared_vehicle_bytes\":{}}}",
            std::mem::size_of::<IidmInput>(), std::mem::size_of::<IidmBatch>(),
            std::mem::size_of::<PreparedActiveMotion<'_>>(), std::mem::size_of::<PreparedVehicleMotion<'_>>());
    }
}
