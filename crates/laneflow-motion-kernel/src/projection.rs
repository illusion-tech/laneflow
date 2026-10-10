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
    .unwrap_or(0.0);
}

/// 一次制动约束的精确 f32 线性/二次解；无可行解时返回 None。
pub fn max_next_speed_for_decel(
    current: f32,
    next: f32,
    delta: f32,
    distance: f32,
    limit: f32,
    decel: f32,
) -> Option<f32> {
    if decel <= 0.0 || delta <= 0.0 || 0.5 * current * delta > distance {
        return None;
    }
    let limit = limit.max(0.0);
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
    if quadratic > limit
        && speed_down_constraint_holds(current, quadratic, delta, distance, limit, decel)
    {
        best = best.max(quadratic);
    }
    speed_down_constraint_holds(current, best, delta, distance, limit, decel)
        .then_some(best.min(next).max(0.0))
}

/// 相同 f32 运算顺序的可行性判定，供摆放、标量预览和向量差分使用。
pub fn speed_down_constraint_holds(
    current: f32,
    next: f32,
    delta: f32,
    distance: f32,
    limit: f32,
    decel: f32,
) -> bool {
    let travel = 0.5 * (current + next) * delta;
    let braking = (next * next - limit * limit).max(0.0) / (2.0 * decel);
    travel + braking <= distance
}
