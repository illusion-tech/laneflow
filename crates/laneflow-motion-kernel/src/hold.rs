//! 停车可行性保持（整数几何 §5.1，#196）。
//!
//! 只做整数与精确有理运算：档位减速度是受检 `f32`，按其精确二进制值
//! `m·2^k` 参与运算，不经过 `f32` / `f64` 舍入后的商。标量与向量后端、
//! Runtime 标量路径共用本模块，判定逐位一致。

/// 一次求值同时考虑的停止目标数（信号等停止目标与有限路终）。
pub const HOLD_TARGETS: usize = 2;

/// 没有该类目标。
pub const NO_HOLD_TARGET: u32 = u32::MAX;

/// 拍初已提交状态与 profile 档位。
#[derive(Clone, Copy, Debug)]
pub struct HoldState {
    pub speed_mm_s: u32,
    pub carry_um: u16,
    pub comfort_decel: f32,
    pub emergency_decel: f32,
    /// 步长（ms）；合法步长为 `4..=1000`。
    pub delta_ms: u32,
}

/// 所选候选的类别，供诊断区分。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HoldKind {
    /// 常规候选通过提交判据（可能清除了差带余数）。
    Regular,
    /// 梯形收紧候选。
    Trapezoid,
    /// 以实际减速度 `A ∈ [V/Δt, b*]` 拍内停稳。
    InTickStop,
    /// 静止末端到位，或静止且没有可行推进（外部约束压住时）。
    Static,
}

/// §5.1 改写后的提交；`None` 表示不负保持义务，或常规候选原样通过、未清除余数且未到位。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HoldCommit {
    /// 本拍有效余数 `C_eff`。
    pub carry_um: u16,
    /// 候选终速（耗尽与终态清零之前）。
    pub speed_mm_s: u32,
    /// 量化后的行程增量 `z`（µm，不含 `C_eff`）。
    pub travel_um: u64,
    /// `C_eff + z` 恰好到达最近目标（终速为零）。
    pub arrives: bool,
    pub kind: HoldKind,
}

/// 由 `f32` 步长恢复整数毫秒；合法步长是整数毫秒，恢复精确。
#[must_use]
pub fn delta_ms(delta_s: f32) -> u32 {
    let ms = (f64::from(delta_s) * 1_000.0).round();
    if ms.is_finite() && ms >= 0.0 && ms <= f64::from(u32::MAX) {
        ms as u32
    } else {
        0
    }
}

/// 正有限 `f32` 的精确分解 `value = mantissa · 2^exponent`。
fn decompose(value: f32) -> Option<(u64, i32)> {
    if !(value.is_finite() && value > 0.0) {
        return None;
    }
    let bits = value.to_bits();
    let exponent_bits = ((bits >> 23) & 0xff) as i32;
    let fraction = u64::from(bits & 0x7f_ffff);
    Some(if exponent_bits == 0 {
        (fraction, -149)
    } else {
        (fraction | 0x80_0000, exponent_bits - 150)
    })
}

fn ceil_div(numerator: u128, denominator: u128) -> u128 {
    numerator.div_ceil(denominator)
}

fn saturate(value: u128) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

/// `Φ_b(V) = ⌈V² / (2b)⌉`（µm）：`(mm/s)² ÷ (m/s²) = µm`。非法减速度饱和为 `u64::MAX`。
#[must_use]
pub fn stop_distance_um(speed_mm_s: u32, decel: f32) -> u64 {
    braking_distance_to_speed_um(speed_mm_s, 0, decel)
}

/// 按 `decel` 的精确二进制值匀减速，从 `from_mm_s` 降到 `limit_mm_s` 所需的行程，
/// 向上取整到微米。速度已达标时返回零，须减速但减速度无效或距离超界时返回 `u64::MAX`。
#[must_use]
pub fn braking_distance_to_speed_um(from_mm_s: u32, limit_mm_s: u32, decel: f32) -> u64 {
    if from_mm_s <= limit_mm_s {
        return 0;
    }
    let Some((mantissa, exponent)) = decompose(decel) else {
        return u64::MAX;
    };
    let squared_drop = u64::from(from_mm_s).pow(2) - u64::from(limit_mm_s).pow(2);
    // ⌈(v² − l²)/(2b)⌉，将精确的 2b = mantissa · 2^(exponent + 1) 分配到分子或分母。
    let shift = exponent + 1;
    if shift >= 0 {
        if shift >= 64 {
            // 平方差小于 2^64，任一正比值在此处向上取整都是 1。
            return 1;
        }
        saturate(ceil_div(
            u128::from(squared_drop),
            u128::from(mantissa) << shift,
        ))
    } else {
        let left = -shift;
        // 运行时受检的速度与减速度都落在此分支，前视二分只需原生整数除法。
        if left < 64 && squared_drop <= (u64::MAX >> left) {
            return (squared_drop << left).div_ceil(mantissa);
        }
        let squared_drop = u128::from(squared_drop);
        if left >= 128 || squared_drop > (u128::MAX >> left) {
            // 分子超出 u128；除以最多 24 位的有效数仍大于 u64::MAX。
            return u64::MAX;
        }
        saturate(ceil_div(squared_drop << left, u128::from(mantissa)))
    }
}

/// `b·Δt`（mm/s，`m/s² · ms = mm/s`）的精确上取整与 `V ≤ b·Δt` 判定。
struct DecelStep {
    /// `⌈b·Δt⌉`
    ceil: u64,
    /// `b·Δt` 是否为整数（此时 `ceil` 即精确值）。
    exact: bool,
}

fn decel_step(decel: f32, delta_ms: u32) -> Option<DecelStep> {
    let (mantissa, exponent) = decompose(decel)?;
    let product = u128::from(mantissa) * u128::from(delta_ms);
    if exponent >= 0 {
        if exponent > 60 {
            return Some(DecelStep {
                ceil: u64::MAX,
                exact: true,
            });
        }
        return Some(DecelStep {
            ceil: saturate(product << exponent),
            exact: true,
        });
    }
    let right = -exponent;
    if right >= 100 {
        return Some(DecelStep {
            ceil: u64::from(product > 0),
            exact: product == 0,
        });
    }
    let denominator = 1u128 << right;
    Some(DecelStep {
        ceil: saturate(ceil_div(product, denominator)),
        exact: product % denominator == 0,
    })
}

impl DecelStep {
    /// `V ≤ b·Δt`
    fn covers(&self, speed_mm_s: u32) -> bool {
        let speed = u64::from(speed_mm_s);
        if self.exact {
            speed <= self.ceil
        } else {
            speed < self.ceil
        }
    }

    /// `⌊V − b·Δt⌋`（仅在 `V > b·Δt` 时调用）。
    fn floor_below(&self, speed_mm_s: u32) -> u32 {
        (u64::from(speed_mm_s) - self.ceil) as u32
    }
}

#[derive(Clone, Copy)]
struct Obligation {
    /// `1000·D`
    distance_um: u64,
    decel: f32,
}

/// 远离全部目标时的快速放行：`1000·D` 明显大于常规行程、最大余数与
/// 舒适停车距离之和时，舒适档成立、不在差带且常规候选通过，结果不变。
fn clearly_clear(
    distance_mm: u32,
    state: &HoldState,
    regular_speed_mm_s: u32,
    regular_travel_um: u64,
) -> bool {
    let fastest = f64::from(state.speed_mm_s.max(regular_speed_mm_s));
    let comfort_stop = fastest * fastest / (2.0 * f64::from(state.comfort_decel));
    let bound = regular_travel_um as f64 + 1_000.0 + comfort_stop * (1.0 + 1e-12) + 2.0;
    bound.is_finite() && f64::from(distance_mm) * 1_000.0 > bound
}

/// 整数几何 §5.1：按拍初状态取保持档位、清除差带余数，核验常规候选；
/// 未通过时在收紧候选中取终速最高、同速行程最大者。
///
/// `targets` 为到各停止目标的路线距离（mm），[`NO_HOLD_TARGET`] 表示没有；
/// `regular_speed_mm_s` 为常规候选量化后、耗尽与终态清零之前的终速（同时是
/// 搜索上界 `q_上`），`regular_travel_um` 为其量化行程（硬约束截断之前）。
#[must_use]
pub fn hold_stop_feasibility(
    state: &HoldState,
    targets: [u32; HOLD_TARGETS],
    regular_speed_mm_s: u32,
    regular_travel_um: u64,
) -> Option<HoldCommit> {
    if state.delta_ms == 0
        || !(state.comfort_decel.is_finite() && state.comfort_decel > 0.0)
        || !(state.emergency_decel.is_finite() && state.emergency_decel > 0.0)
    {
        return None;
    }
    let speed = state.speed_mm_s;
    // 精确停车距离延迟到出现未被快速放行的目标时才求值。
    let mut stops: Option<(u64, u64)> = None;
    let mut obligations = [None; HOLD_TARGETS];
    let mut carry = state.carry_um;
    let mut strict = state.comfort_decel;
    for (slot, &distance_mm) in obligations.iter_mut().zip(targets.iter()) {
        if distance_mm == NO_HOLD_TARGET
            || distance_mm == 0
            || clearly_clear(distance_mm, state, regular_speed_mm_s, regular_travel_um)
        {
            continue;
        }
        let distance_um = u64::from(distance_mm) * 1_000;
        let (comfort_stop, emergency_stop) = *stops.get_or_insert_with(|| {
            (
                stop_distance_um(speed, state.comfort_decel),
                stop_distance_um(speed, state.emergency_decel),
            )
        });
        let (decel, stop) = if comfort_stop <= distance_um {
            (state.comfort_decel, comfort_stop)
        } else if emergency_stop <= distance_um {
            (state.emergency_decel, emergency_stop)
        } else {
            continue;
        };
        if decel > strict {
            strict = decel;
        }
        // 差带入口：连续判据成立而含余数的整数判据不成立。
        if stop > distance_um - u64::from(state.carry_um) {
            carry = 0;
        }
        *slot = Some(Obligation { distance_um, decel });
    }
    if obligations.iter().all(Option::is_none) {
        return None;
    }
    let budget = |obligation: &Obligation| obligation.distance_um - u64::from(carry);
    let feasible = |speed_mm_s: u32, travel_um: u64| {
        obligations.iter().flatten().all(|obligation| {
            travel_um
                .checked_add(stop_distance_um(speed_mm_s, obligation.decel))
                .is_some_and(|cost| cost <= budget(obligation))
        })
    };
    if feasible(regular_speed_mm_s, regular_travel_um) {
        // 常规候选本身零速恰好到达负保持义务的目标时，数值无需改写，
        // 但到位必须交给完成阶段，使到位零速优先于过渡耗尽速度。
        let arrives = regular_speed_mm_s == 0
            && obligations
                .iter()
                .flatten()
                .any(|obligation| regular_travel_um == budget(obligation));
        return (arrives || carry != state.carry_um).then_some(HoldCommit {
            carry_um: carry,
            speed_mm_s: regular_speed_mm_s,
            travel_um: regular_travel_um,
            arrives,
            kind: HoldKind::Regular,
        });
    }
    let nearest = obligations
        .iter()
        .flatten()
        .map(budget)
        .min()
        .expect("at least one obligation");
    let delta = u64::from(state.delta_ms);
    let trapezoid = |speed_mm_s: u32| (u64::from(speed) + u64::from(speed_mm_s)) * delta / 2;
    let step = decel_step(strict, state.delta_ms)?;
    let upper = regular_speed_mm_s;
    let covers = step.covers(speed);
    let lower = if covers {
        1
    } else {
        step.floor_below(speed).min(upper)
    };
    if lower <= upper && feasible(lower, trapezoid(lower)) {
        // 代价随终速单调不减：二分取最高可行终速。
        let (mut good, mut bad) = (lower, upper.saturating_add(1));
        while bad - good > 1 {
            let middle = good + (bad - good) / 2;
            if feasible(middle, trapezoid(middle)) {
                good = middle;
            } else {
                bad = middle;
            }
        }
        return Some(HoldCommit {
            carry_um: carry,
            speed_mm_s: good,
            travel_um: trapezoid(good),
            arrives: false,
            kind: HoldKind::Trapezoid,
        });
    }
    if !covers {
        // 存在性论证保证 `q_下` 可行；防御性保留同一候选。
        debug_assert!(false, "lower trapezoid candidate must satisfy the hold");
        return Some(HoldCommit {
            carry_um: carry,
            speed_mm_s: lower,
            travel_um: trapezoid(lower),
            arrives: false,
            kind: HoldKind::Trapezoid,
        });
    }
    if speed > 0 {
        // 实际行程 s = min(R, V·Δt/2)，z = ⌊s⌋；R ≤ V·Δt/2 时恰好到位。
        let half = u64::from(speed) * delta;
        let arrives = nearest * 2 <= half;
        return Some(HoldCommit {
            carry_um: carry,
            speed_mm_s: 0,
            travel_um: if arrives { nearest } else { half / 2 },
            arrives,
            kind: HoldKind::InTickStop,
        });
    }
    let arrives = nearest <= delta / 2;
    Some(HoldCommit {
        carry_um: carry,
        speed_mm_s: 0,
        travel_um: if arrives { nearest } else { 0 },
        arrives,
        kind: HoldKind::Static,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(speed: u32, carry: u16, comfort: f32, emergency: f32, delta_ms: u32) -> HoldState {
        HoldState {
            speed_mm_s: speed,
            carry_um: carry,
            comfort_decel: comfort,
            emergency_decel: emergency,
            delta_ms,
        }
    }

    #[test]
    fn stop_distance_is_exact_ceiling() {
        assert_eq!(stop_distance_um(0, 2.0), 0);
        assert_eq!(stop_distance_um(48, 6.0), 192);
        assert_eq!(stop_distance_um(48, 2.0), 576);
        assert_eq!(stop_distance_um(5, 1.0), 13);
        assert_eq!(stop_distance_um(6635, 6.0), 3_668_603);
        assert_eq!(stop_distance_um(1, 0.5), 1);
        // 0.1 不是二进制有限小数：按精确 f32 值向上取整。
        let exact = 1_000_000.0_f64 / (2.0 * f64::from(0.1_f32));
        assert_eq!(stop_distance_um(1_000, 0.1), exact.ceil() as u64);
    }

    #[test]
    fn brake_to_speed_rounds_the_difference_once_and_preserves_binary32_precision() {
        // 两次停车距离各自进位会错误得到 3 µm。平方差 7 除以 2 应进位到 4。
        assert_eq!(braking_distance_to_speed_um(4, 3, 1.0), 4);
        assert_eq!(braking_distance_to_speed_um(2, 3, 1.0), 0);
        assert_eq!(braking_distance_to_speed_um(3, 2, f32::NAN), u64::MAX);
        assert_eq!(braking_distance_to_speed_um(3, 2, f32::MAX), 1);
        // b=2^-42 的精确结果仍可表示，b=2^-149 则饱和。
        assert_eq!(
            braking_distance_to_speed_um(3, 2, 2.0_f32.powi(-42)),
            5_u64 << 41
        );
        assert_eq!(
            braking_distance_to_speed_um(3, 2, f32::from_bits(1)),
            u64::MAX
        );
        assert_eq!(
            braking_distance_to_speed_um(u32::MAX, 0, 0.5),
            u64::from(u32::MAX).pow(2)
        );
    }

    #[test]
    fn decel_step_matches_exact_product() {
        let step = decel_step(6.0, 16).unwrap();
        assert!(step.exact && step.ceil == 96);
        assert!(step.covers(96) && !step.covers(97));
        assert_eq!(step.floor_below(6635), 6539);
        let step = decel_step(0.1, 5).unwrap();
        assert!(!step.exact && step.ceil == 1);
        assert!(step.covers(0) && !step.covers(1));
    }

    #[test]
    fn band_entry_clears_carry_and_keeps_regular_candidate() {
        // #196 评审的差带入口：清除余数后整数判据成立。
        let held = hold_stop_feasibility(
            &state(6635, 614, 2.0, 6.0, 16),
            [3669, NO_HOLD_TARGET],
            6539,
            105_392,
        )
        .unwrap();
        assert_eq!(held.carry_um, 0);
        let budget = 3_669_000 - u64::from(held.carry_um);
        assert!(held.travel_um + stop_distance_um(held.speed_mm_s, 6.0) <= budget);
    }

    #[test]
    fn stationary_fixed_point_progresses() {
        // #1504 首轮反例的固定点：(0, 1, 560)，常规候选 (32, 256)。
        let held =
            hold_stop_feasibility(&state(0, 560, 2.0, 6.0, 16), [1, NO_HOLD_TARGET], 32, 256)
                .unwrap();
        assert_eq!((held.speed_mm_s, held.travel_um), (28, 224));
        assert_eq!(held.kind, HoldKind::Trapezoid);
    }

    #[test]
    fn rejected_arrival_does_not_lower_search_bound() {
        // #1504 复审：(0, 1, 880)，搜索上界取耗尽清零之前的 32。
        let held =
            hold_stop_feasibility(&state(0, 880, 2.0, 6.0, 16), [1, NO_HOLD_TARGET], 32, 256)
                .unwrap();
        assert_eq!((held.speed_mm_s, held.travel_um), (11, 88));
    }

    #[test]
    fn in_tick_stop_uses_actual_travel() {
        // #1504 复审：V = 5、b = 1、Δt = 5、R = 13，实际行程 12.5 µm → z = 12。
        let held = hold_stop_feasibility(&state(5, 987, 1.0, 6.0, 5), [1, NO_HOLD_TARGET], 10, 38)
            .unwrap();
        assert_eq!(held.kind, HoldKind::InTickStop);
        assert_eq!(
            (held.speed_mm_s, held.travel_um, held.arrives),
            (0, 12, false)
        );
        // 剩余 1 µm 由静止末端到位处理。
        let held =
            hold_stop_feasibility(&state(0, 999, 1.0, 6.0, 5), [1, NO_HOLD_TARGET], 7, 17).unwrap();
        assert_eq!(held.kind, HoldKind::Static);
        assert_eq!((held.travel_um, held.arrives), (1, true));
    }

    #[test]
    fn in_tick_stop_arrives_exactly() {
        let held =
            hold_stop_feasibility(&state(28, 784, 2.0, 6.0, 16), [1, NO_HOLD_TARGET], 60, 704)
                .unwrap();
        assert_eq!(held.kind, HoldKind::InTickStop);
        assert_eq!((held.travel_um, held.arrives), (216, true));
    }

    #[test]
    fn regular_arrival_is_reported_without_rewrite() {
        // #1506 评审：V = 1000、D = 125、b_c = 2、b_e = 4、Δt = 250，常规候选
        // (0, 125000) 恰好到位；余数不变也必须报告到位。
        let held = hold_stop_feasibility(
            &state(1_000, 0, 2.0, 4.0, 250),
            [125, NO_HOLD_TARGET],
            0,
            125_000,
        )
        .unwrap();
        assert_eq!(
            held,
            HoldCommit {
                carry_um: 0,
                speed_mm_s: 0,
                travel_um: 125_000,
                arrives: true,
                kind: HoldKind::Regular,
            }
        );
        // 清除余数后常规候选恰好到位：同样报告到位。
        let held = hold_stop_feasibility(
            &state(1_000, 400, 2.0, 4.0, 250),
            [125, NO_HOLD_TARGET],
            0,
            125_000,
        )
        .unwrap();
        assert_eq!((held.carry_um, held.arrives), (0, true));
        // 零速未到位：不改写。
        assert_eq!(
            hold_stop_feasibility(
                &state(1_000, 0, 2.0, 4.0, 250),
                [126, NO_HOLD_TARGET],
                0,
                125_000
            ),
            None
        );
    }

    #[test]
    fn far_targets_and_unstoppable_vehicles_have_no_obligation() {
        assert_eq!(
            hold_stop_feasibility(
                &state(10_000, 0, 2.0, 6.0, 16),
                [500_000, 900_000],
                10_020,
                160_160
            ),
            None
        );
        // 紧急档也停不住：不负保持义务。
        assert_eq!(
            hold_stop_feasibility(
                &state(12_500, 0, 2.0, 6.0, 16),
                [11_600, NO_HOLD_TARGET],
                12_520,
                200_160
            ),
            None
        );
    }
}
