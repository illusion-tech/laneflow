//! 一个实际降速 occurrence 的制动求解；路线发现与排序属于 Runtime。

/// 全部列属于同一物理行范围，enabled 仅含本轮实际降速消费者。
pub struct LimitInput<'a> {
    pub speed_mm_s: &'a [u32],
    pub distance_mm: &'a [u32],
    pub limit_mm_s: &'a [u32],
    pub window_m: &'a [f32],
    pub comfort_decel: &'a [f32],
    pub emergency_decel: &'a [f32],
}

/// 在原位提案速度列施加本轮约束。
pub struct LimitOutput<'a> {
    pub next_speed_m_s: &'a mut [f32],
    pub active: &'a mut [bool],
}

/// 速度约束完成后的实际降速边界输入。
pub struct BoundaryInput<'a> {
    pub speed_mm_s: &'a [u32],
    pub distance_mm: &'a [u32],
    pub limit_mm_s: &'a [u32],
    pub next_speed_m_s: &'a [f32],
}

/// active 同时是工作掩码和“还要读取后续 occurrence”的结果。
pub struct BoundaryOutput<'a> {
    pub travel_m: &'a mut [f32],
    pub active: &'a mut [bool],
}

pub(super) fn boundary_scalar(
    input: &BoundaryInput<'_>,
    output: &mut BoundaryOutput<'_>,
    row: usize,
    delta_s: f32,
) {
    if !output.active[row] || input.distance_mm[row] == 0 {
        return;
    }
    let distance = input.distance_mm[row] as f32 / 1_000.0;
    let travel = output.travel_m[row];
    if distance >= travel {
        output.active[row] = false;
        return;
    }
    let speed = input.speed_mm_s[row] as f32 / 1_000.0;
    let limit = input.limit_mm_s[row] as f32 / 1_000.0;
    if limit < speed && limit < input.next_speed_m_s[row] && 0.5 * speed * delta_s <= distance {
        output.travel_m[row] = travel.min(distance);
    }
}

pub(super) fn scalar(
    input: &LimitInput<'_>,
    output: &mut LimitOutput<'_>,
    row: usize,
    delta_s: f32,
) {
    if !output.active[row] {
        return;
    }
    let next = output.next_speed_m_s[row];
    let limit = input.limit_mm_s[row] as f32 / 1_000.0;
    if input.distance_mm[row] == 0 {
        output.next_speed_m_s[row] = next.min(limit.max(0.0));
        return;
    }
    let distance = input.distance_mm[row] as f32 / 1_000.0;
    if distance > input.window_m[row] {
        output.active[row] = false;
        return;
    }
    if limit >= next {
        return;
    }
    let current = input.speed_mm_s[row] as f32 / 1_000.0;
    output.next_speed_m_s[row] = max_next_speed_for_decel(
        current,
        next,
        delta_s,
        distance,
        limit,
        input.comfort_decel[row],
    )
    .or_else(|| {
        max_next_speed_for_decel(
            current,
            next,
            delta_s,
            distance,
            limit,
            input.emergency_decel[row],
        )
    })
    // 两域皆空（#1272）：crossing 投影按 §8.1 收到目标限速，不再归零。
    .unwrap_or(limit);
}

/// 根代回约束失败时的确定性安全方向微降系数（f32 舍入可让正根越界
/// ~1 ULP；直接弃根退到 floor 会让停在目标前几十毫米的车永久不动——
/// 8 mm 卡死回归）。两级覆盖约 1e-6 与 1e-3 相对越界。
pub(crate) const ROOT_SHRINK_SOFT: f32 = 1.0 - 1.0 / (1 << 20) as f32;
pub(crate) const ROOT_SHRINK_HARD: f32 = 1.0 - 1.0 / (1 << 10) as f32;

/// 一次制动约束的精确 f32 线性/二次解；无可行解时返回 None。
///
/// 可行域同时约束本拍位移（含拍内停稳分支）与后续制动，以及本拍减速上界
/// `u >= max(0, v - b*dt)`（终速在零处饱和，仅检查终速无法约束实际减速度；
/// 停稳分支由档位减速度承担同一义务）。候选 `next` 低于该档下界时域为空，
/// 由调用方级联到更低档或具名投影；本函数不抬高候选。
pub fn max_next_speed_for_decel(
    current: f32,
    next: f32,
    delta: f32,
    distance: f32,
    limit: f32,
    decel: f32,
) -> Option<f32> {
    if decel <= 0.0 || delta <= 0.0 {
        return None;
    }
    let limit = limit.max(0.0);
    let floor = (current - decel * delta).max(0.0);
    if floor > next || !speed_down_constraint_holds(current, floor, delta, distance, limit, decel) {
        return None;
    }
    let linear = ((2.0 * distance / delta) - current)
        .min(limit)
        .min(next)
        .max(0.0);
    let b_dt = decel * delta;
    let constant = decel * current * delta - limit * limit - 2.0 * decel * distance;
    let discriminant = b_dt * b_dt - 4.0 * constant;
    let quadratic = if discriminant >= 0.0 {
        ((-b_dt + discriminant.sqrt()) / 2.0).min(next)
    } else {
        f32::NEG_INFINITY
    };
    let mut best = linear;
    if quadratic > limit {
        // 根代回失败时按确定性的安全方向微降重试；三级候选与 SIMD 逐位
        // 同序。全部失败才回落到 linear/floor。
        let holds = |candidate: f32| {
            speed_down_constraint_holds(current, candidate, delta, distance, limit, decel)
        };
        let soft = quadratic * ROOT_SHRINK_SOFT;
        let hard = quadratic * ROOT_SHRINK_HARD;
        let corrected = if holds(quadratic) {
            quadratic
        } else if holds(soft) {
            soft
        } else if holds(hard) {
            hard
        } else {
            f32::NEG_INFINITY
        };
        if corrected > limit {
            best = best.max(corrected);
        }
    }
    // floor 已验证可行且约束左侧随 u 单调不减，可行最大值不低于 floor。
    best = best.max(floor);
    speed_down_constraint_holds(current, best, delta, distance, limit, decel)
        .then_some(best.min(next))
}

/// 相同 f32 运算顺序的可行性判定，供摆放、标量预览和向量差分使用。
///
/// 拍内停稳（终速为 0 且档位减速度足以本拍停住）时，位移用与
/// vehicle-following §11.1 一致的实际加速度停车分支 `v^2/(2b)`；
/// 未停稳用梯形积分。
pub fn speed_down_constraint_holds(
    current: f32,
    next: f32,
    delta: f32,
    distance: f32,
    limit: f32,
    decel: f32,
) -> bool {
    let travel = if next <= 0.0 && current <= decel * delta {
        (current * current) / (2.0 * decel)
    } else {
        0.5 * (current + next) * delta
    };
    let braking = (next * next - limit * limit).max(0.0) / (2.0 * decel);
    travel + braking <= distance
}
