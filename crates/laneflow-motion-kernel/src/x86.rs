use super::projection::{ROOT_SHRINK_HARD, ROOT_SHRINK_SOFT};
use super::{
    BoundaryInput, BoundaryOutput, Columns, EdgeInput, EdgeOutput, EdgeStats, Input, LimitInput,
    LimitOutput, Phase, Stats, finish_values, motion_values_valid, raw_valid,
    stop_solve_participates,
};
use std::arch::x86_64::*;

#[target_feature(enable = "avx2")]
unsafe fn unsigned4(values: &[f64]) -> __m128i {
    // SAFETY: 仅由 finish4 的四行完整范围调用；结果仅在受检范围掩码内发布。
    let values = unsafe { _mm256_loadu_pd(values.as_ptr()) };
    let high = _mm256_cmp_pd::<_CMP_GE_OQ>(values, _mm256_set1_pd(2_147_483_648.0));
    let adjusted = _mm256_blendv_pd(
        values,
        _mm256_sub_pd(values, _mm256_set1_pd(2_147_483_648.0)),
        high,
    );
    let bits = _mm256_movemask_pd(high);
    let sign = _mm_set_epi32(
        if bits & 8 != 0 { i32::MIN } else { 0 },
        if bits & 4 != 0 { i32::MIN } else { 0 },
        if bits & 2 != 0 { i32::MIN } else { 0 },
        if bits & 1 != 0 { i32::MIN } else { 0 },
    );
    _mm_or_si128(_mm256_cvttpd_epi32(adjusted), sign)
}

#[target_feature(enable = "avx2")]
#[allow(clippy::too_many_arguments)]
unsafe fn finish4<const TRACK: bool>(
    input: &Input<'_>,
    output: &mut Columns<'_>,
    start: usize,
    delta_s: f32,
    um: &[f64],
    speed: &[f64],
    valid: &[bool],
    stats: &mut Stats,
) {
    // SAFETY: 后端只遍历完整四行；所有列已在 safe run 验证，所有载入/写入范围
    // start..start+4 都在这些独占输出内。u64 大提案和非有限通道保留精确标量回退。
    unsafe {
        let rounded = unsigned4(um);
        let carry = _mm_cvtepu16_epi32(_mm_loadl_epi64(input.carry_um.as_ptr().add(start).cast()));
        let total = _mm_add_epi32(rounded, carry);
        // 全 u32 / 1000：ceil(2^38 / 1000)，偶/奇通道分别用完整 u64 乘积。
        let magic = _mm_set1_epi32(274_877_907);
        let even = _mm_srli_epi64::<38>(_mm_mul_epu32(total, magic));
        let odd = _mm_slli_epi64::<32>(_mm_srli_epi64::<38>(_mm_mul_epu32(
            _mm_srli_epi64::<32>(total),
            magic,
        )));
        let proposed = _mm_or_si128(even, odd);
        let room = _mm_loadu_si128(input.hard_room_mm.as_ptr().add(start).cast());
        let travel = _mm_min_epu32(proposed, room);
        let exhausted = _mm_cmpeq_epi32(travel, room);
        let remainder = _mm_andnot_si128(
            exhausted,
            _mm_sub_epi32(total, _mm_mullo_epi32(travel, _mm_set1_epi32(1_000))),
        );
        let old_progress = _mm_loadu_si128(input.progress_mm.as_ptr().add(start).cast());
        let progress = _mm_add_epi32(old_progress, travel);
        let sign = _mm_set1_epi32(i32::MIN);
        let overflow = _mm_cmpgt_epi32(
            _mm_xor_si128(old_progress, sign),
            _mm_xor_si128(progress, sign),
        );
        let progress = _mm_or_si128(progress, overflow);
        let limit = _mm_loadu_si128(input.committed_limit_mm_s.as_ptr().add(start).cast());
        let stop_cap = _mm_loadu_si128(input.exhausted_speed_cap_mm_s.as_ptr().add(start).cast());
        let projected = unsigned4(speed);
        // F3' 复审修：过渡耗尽通道（exhausted_cap_dominates）速度取
        // min(cap, limit)，不与候选投影再 min；其余耗尽通道维持
        // min(投影, cap, limit)。与标量 finish_values 逐位一致。
        // R4-1 复审修：布尔列每行 1 字节，四行只有 4 字节——只读 4 字节
        // （_mm_loadu_si32）再零扩展；不能用 8 字节的 loadl_epi64（恰长
        // 输入的末组会越过分配读 4 字节）。
        let dominant = _mm_cvtepu8_epi32(_mm_loadu_si32(
            input.exhausted_cap_dominates.as_ptr().add(start).cast(),
        ));
        let dominant_mask = _mm_cmpeq_epi32(dominant, _mm_set1_epi32(1));
        let exhausted_value = _mm_blendv_epi8(
            _mm_min_epu32(_mm_min_epu32(projected, stop_cap), limit),
            _mm_min_epu32(stop_cap, limit),
            dominant_mask,
        );
        // 非耗尽通道才与投影取 min；耗尽通道的投影合并已在 exhausted_value
        // 内按主导/非主导分别完成。
        let speed_value =
            _mm_blendv_epi8(_mm_min_epu32(projected, limit), exhausted_value, exhausted);
        let exhausted_bits = _mm_movemask_ps(_mm_castsi128_ps(exhausted));
        let fast: [i32; 4] = std::array::from_fn(|lane| {
            let row = start + lane;
            let numeric_valid = delta_s > 0.0
                && delta_s.is_finite()
                && input.max_accel[row] > 0.0
                && input.comfort_decel[row] > 0.0
                && input.emergency_decel[row] > 0.0
                && valid[lane];
            let travel_convertible = input.hard_room_mm[row] == 0
                || (um[lane] >= 0.0
                    && um[lane] <= f64::from(u32::MAX - u32::from(input.carry_um[row])));
            let forced_zero =
                exhausted_bits & (1 << lane) != 0 && input.exhausted_speed_cap_mm_s[row] == 0;
            let convertible = travel_convertible
                && (forced_zero || (speed[lane] >= 0.0 && speed[lane] <= f64::from(u32::MAX)));
            if input.enabled[row] && numeric_valid && convertible {
                -1
            } else {
                0
            }
        });
        let mask = _mm_loadu_si128(fast.as_ptr().cast());
        let old = _mm_loadu_si128(output.travel_mm.as_ptr().add(start).cast());
        _mm_storeu_si128(
            output.travel_mm.as_mut_ptr().add(start).cast(),
            _mm_blendv_epi8(old, travel, mask),
        );
        let old = _mm_loadu_si128(output.progress_mm.as_ptr().add(start).cast());
        _mm_storeu_si128(
            output.progress_mm.as_mut_ptr().add(start).cast(),
            _mm_blendv_epi8(old, progress, mask),
        );
        let old = _mm_loadu_si128(output.speed_mm_s.as_ptr().add(start).cast());
        _mm_storeu_si128(
            output.speed_mm_s.as_mut_ptr().add(start).cast(),
            _mm_blendv_epi8(old, speed_value, mask),
        );
        let packed = _mm_packus_epi32(remainder, _mm_setzero_si128());
        let packed_mask = _mm_packs_epi32(mask, _mm_setzero_si128());
        let old = _mm_loadl_epi64(output.carry_um.as_ptr().add(start).cast());
        _mm_storel_epi64(
            output.carry_um.as_mut_ptr().add(start).cast(),
            _mm_blendv_epi8(old, packed, packed_mask),
        );
        for lane in 0..4 {
            let row = start + lane;
            if fast[lane] != 0 {
                output.valid[row] = true;
                output.exhausted[row] = exhausted_bits & (1 << lane) != 0;
                if TRACK {
                    stats.integer_vector_lanes += 1;
                }
            } else {
                finish_values(input, output, row, valid[lane], um[lane], speed[lane]);
            }
        }
    }
}

#[target_feature(enable = "avx2")]
fn select256(mask: __m256, yes: __m256, no: __m256) -> __m256 {
    _mm256_blendv_ps(no, yes, mask)
}
#[target_feature(enable = "avx512f")]
fn select512(mask: __mmask16, yes: __m512, no: __m512) -> __m512 {
    _mm512_mask_blend_ps(mask, no, yes)
}
fn and512(a: __mmask16, b: __mmask16) -> __mmask16 {
    a & b
}
fn float_bits512(mask: __mmask16) -> u32 {
    u32::from(mask)
}
#[target_feature(enable = "avx2")]
fn float_bits2(mask: __m256) -> u32 {
    _mm256_movemask_ps(mask) as u32
}

macro_rules! boundary_step {
    ($input:ident, $output:ident, $delta:ident, $width:literal, $set:ident, $load:ident, $store:ident, $uint:ident, $mask:ident,
        $mul:ident, $div:ident, $min:ident, $cmp:ident, $and:ident, $select:ident, $bits:ident) => {{
        let end = $output.active.len() / $width * $width;
        let zero = $set(0.0);
        let dt = $set($delta);
        for start in (0..end).step_by($width) {
            // SAFETY: safe boundary 验证切片长度；每轮 start+width 在完整范围内。
            unsafe {
                let active = $mask($output.active, start);
                let speed = $div($uint($input.speed_mm_s, start), $set(1_000.0));
                let distance = $div($uint($input.distance_mm, start), $set(1_000.0));
                let limit = $div($uint($input.limit_mm_s, start), $set(1_000.0));
                let next = $load($input.next_speed_m_s.as_ptr().add(start));
                let travel = $load($output.travel_m.as_ptr().add(start));
                let nonzero = $cmp::<_CMP_NEQ_OQ>(distance, zero);
                let within = $cmp::<_CMP_LT_OQ>(distance, travel);
                let clamp = $and(
                    active,
                    $and(
                        nonzero,
                        $and(
                            within,
                            $and(
                                $and(
                                    $cmp::<_CMP_LT_OQ>(limit, speed),
                                    $cmp::<_CMP_LT_OQ>(limit, next),
                                ),
                                $cmp::<_CMP_LE_OQ>($mul($mul($set(0.5), speed), dt), distance),
                            ),
                        ),
                    ),
                );
                $store(
                    $output.travel_m.as_mut_ptr().add(start),
                    $select(clamp, $min(travel, distance), travel),
                );
                let continues = $select(nonzero, $select(within, $set(1.0), zero), $set(1.0));
                let flags = $bits($and(active, $cmp::<_CMP_NEQ_OQ>(continues, zero)));
                for row in 0..$width {
                    $output.active[start + row] = flags & (1 << row) != 0;
                }
            }
        }
        end
    }};
}

#[target_feature(enable = "avx2")]
pub(super) unsafe fn boundary2(
    input: &BoundaryInput<'_>,
    output: &mut BoundaryOutput<'_>,
    delta_s: f32,
) -> usize {
    boundary_step!(
        input,
        output,
        delta_s,
        8,
        _mm256_set1_ps,
        _mm256_loadu_ps,
        _mm256_storeu_ps,
        u32x8,
        mask256,
        _mm256_mul_ps,
        _mm256_div_ps,
        _mm256_min_ps,
        _mm256_cmp_ps,
        _mm256_and_ps,
        select256,
        float_bits2
    )
}

#[target_feature(enable = "avx512f")]
pub(super) unsafe fn boundary512(
    input: &BoundaryInput<'_>,
    output: &mut BoundaryOutput<'_>,
    delta_s: f32,
) -> usize {
    boundary_step!(
        input,
        output,
        delta_s,
        16,
        _mm512_set1_ps,
        _mm512_loadu_ps,
        _mm512_storeu_ps,
        u32x16,
        mask512,
        _mm512_mul_ps,
        _mm512_div_ps,
        _mm512_min_ps,
        _mm512_cmp_ps_mask,
        and512,
        select512,
        float_bits512
    )
}

macro_rules! limit_step {
    ($input:ident, $output:ident, $delta:ident, $width:literal, $set:ident, $load:ident, $store:ident, $uint:ident, $mask:ident,
        $add:ident, $sub:ident, $mul:ident, $div:ident, $min:ident, $max:ident, $sqrt:ident, $cmp:ident, $and:ident, $select:ident, $bits:ident) => {{
        let end = $output.active.len() / $width * $width;
        let zero = $set(0.0); let two = $set(2.0); let dt = $set($delta); let half = $set(0.5);
        for start in (0..end).step_by($width) {
            // SAFETY: safe limit 验证全部列；每轮仅访问完整 width 范围，输出不别名。
            unsafe {
                let current = $div($uint($input.speed_mm_s, start), $set(1_000.0));
                let distance = $div($uint($input.distance_mm, start), $set(1_000.0));
                let limit = $max($div($uint($input.limit_mm_s, start), $set(1_000.0)), zero);
                let next = $load($output.next_speed_m_s.as_ptr().add(start));
                let active = $mask($output.active, start);
                let within = $select($cmp::<_CMP_EQ_OQ>(distance, zero), $set(1.0),
                    $select($cmp::<_CMP_LE_OQ>(distance, $load($input.window_m.as_ptr().add(start))), $set(1.0), zero));
                let work = $and(active, $cmp::<_CMP_NEQ_OQ>(within, zero));
                let solve = |decel| {
                    let linear = $max($min($min($sub($div($mul(two, distance), dt), current), limit), next), zero);
                    let b_dt = $mul(decel, dt);
                    let constant = $sub($sub($mul($mul(decel, current), dt), $mul(limit, limit)), $mul($mul(two, decel), distance));
                    let discriminant = $sub($mul(b_dt, b_dt), $mul($set(4.0), constant));
                    let quadratic = $select($cmp::<_CMP_GE_OQ>(discriminant, zero),
                        $min($div($add($sub(zero, b_dt), $sqrt(discriminant)), two), next), $set(f32::NEG_INFINITY));
                    let holds = |candidate| {
                        // 拍内停稳分支用档位减速度的实际停车位移，未停稳用梯形。
                        let stop_travel = $div($mul(current, current), $mul(two, decel));
                        let trap_travel = $mul($mul(half, $add(current, candidate)), dt);
                        let stopped = $and($cmp::<_CMP_LE_OQ>(candidate, zero), $cmp::<_CMP_LE_OQ>(current, b_dt));
                        let travel = $select(stopped, stop_travel, trap_travel);
                        let braking = $div($max($sub($mul(candidate, candidate), $mul(limit, limit)), zero), $mul(two, decel));
                        $cmp::<_CMP_LE_OQ>($add(travel, braking), distance)
                    };
                    // 本拍减速上界：低于 floor 的终速不在该档可行域内。
                    let floor = $max($sub(current, b_dt), zero);
                    let domain_ok = $cmp::<_CMP_LE_OQ>(floor, next);
                    // 根代回失败时按确定性安全方向微降重试（与标量逐位同序；
                    // 直接弃根退到 floor 会让停在目标前几十毫米的车永久不动）。
                    let soft = $mul(quadratic, $set(ROOT_SHRINK_SOFT));
                    let hard = $mul(quadratic, $set(ROOT_SHRINK_HARD));
                    let corrected = $select(holds(quadratic), quadratic,
                        $select(holds(soft), soft,
                            $select(holds(hard), hard, $set(f32::NEG_INFINITY))));
                    let better = $and($cmp::<_CMP_GT_OQ>(corrected, limit), $cmp::<_CMP_GT_OQ>(corrected, $set(f32::NEG_INFINITY)));
                    let best = $select(better, $max(linear, corrected), linear);
                    // floor 已验证可行且左侧随 u 单调，可行最大值不低于 floor。
                    let best = $max(best, floor);
                    let legal = $and($and($and($cmp::<_CMP_GT_OQ>(decel, zero), $cmp::<_CMP_GT_OQ>(dt, zero)), domain_ok),
                        $and(holds(floor), holds(best)));
                    ($min(best, next), legal)
                };
                let (comfort, comfort_valid) = solve($load($input.comfort_decel.as_ptr().add(start)));
                let (emergency, emergency_valid) = solve($load($input.emergency_decel.as_ptr().add(start)));
                // 两域皆空（#1272）：crossing 投影按 §8.1 收到目标限速，不再归零。
                let capped = $select(comfort_valid, comfort, $select(emergency_valid, emergency, limit));
                let candidate = $select($cmp::<_CMP_GE_OQ>(limit, next), next, capped);
                let candidate = $select($cmp::<_CMP_EQ_OQ>(distance, zero), $min(next, limit), candidate);
                $store($output.next_speed_m_s.as_mut_ptr().add(start), $select(work, candidate, next));
                let flags = $bits(work);
                for row in 0..$width { $output.active[start + row] = flags & (1 << row) != 0; }
            }
        }
        end
    }};
}

#[target_feature(enable = "avx2")]
pub(super) unsafe fn limit2(
    input: &LimitInput<'_>,
    output: &mut LimitOutput<'_>,
    delta_s: f32,
) -> usize {
    limit_step!(
        input,
        output,
        delta_s,
        8,
        _mm256_set1_ps,
        _mm256_loadu_ps,
        _mm256_storeu_ps,
        u32x8,
        mask256,
        _mm256_add_ps,
        _mm256_sub_ps,
        _mm256_mul_ps,
        _mm256_div_ps,
        _mm256_min_ps,
        _mm256_max_ps,
        _mm256_sqrt_ps,
        _mm256_cmp_ps,
        _mm256_and_ps,
        select256,
        float_bits2
    )
}

#[target_feature(enable = "avx512f")]
pub(super) unsafe fn limit512(
    input: &LimitInput<'_>,
    output: &mut LimitOutput<'_>,
    delta_s: f32,
) -> usize {
    limit_step!(
        input,
        output,
        delta_s,
        16,
        _mm512_set1_ps,
        _mm512_loadu_ps,
        _mm512_storeu_ps,
        u32x16,
        mask512,
        _mm512_add_ps,
        _mm512_sub_ps,
        _mm512_mul_ps,
        _mm512_div_ps,
        _mm512_min_ps,
        _mm512_max_ps,
        _mm512_sqrt_ps,
        _mm512_cmp_ps_mask,
        and512,
        select512,
        float_bits512
    )
}

#[target_feature(enable = "avx2")]
fn eq2(a: __m256i, b: __m256i) -> __m256i {
    _mm256_cmpeq_epi32(a, b)
}
#[target_feature(enable = "avx512f")]
fn eq512(a: __m512i, b: __m512i) -> __m512i {
    _mm512_maskz_set1_epi32(_mm512_cmpeq_epi32_mask(a, b), -1)
}
#[target_feature(enable = "avx2")]
fn bool2(values: &[bool], start: usize) -> __m256i {
    _mm256_castps_si256(mask256(values, start))
}
#[target_feature(enable = "avx512f")]
fn bool512(values: &[bool], start: usize) -> __m512i {
    _mm512_maskz_set1_epi32(mask512(values, start), -1)
}
#[target_feature(enable = "avx2")]
fn blend2(old: __m256i, next: __m256i, mask: __m256i) -> __m256i {
    _mm256_blendv_epi8(old, next, mask)
}
#[target_feature(enable = "avx512f")]
fn blend512(old: __m512i, next: __m512i, mask: __m512i) -> __m512i {
    _mm512_mask_blend_epi32(
        _mm512_cmpneq_epi32_mask(mask, _mm512_setzero_si512()),
        old,
        next,
    )
}
#[target_feature(enable = "avx2")]
fn bits2(mask: __m256i) -> u32 {
    _mm256_movemask_ps(_mm256_castsi256_ps(mask)) as u32
}
#[target_feature(enable = "avx512f")]
fn bits512(mask: __m512i) -> u32 {
    u32::from(_mm512_cmpneq_epi32_mask(mask, _mm512_setzero_si512()))
}

macro_rules! edge_step {
    ($input:ident, $output:ident, $stats:ident, $width:literal, $set:ident, $load:ident, $store:ident,
        $min:ident, $add:ident, $sub:ident, $and:ident, $or:ident, $xor:ident, $eq:ident, $mask:ident, $blend:ident, $bits:ident) => {{
        let end = $output.active.len() / $width * $width;
        let zero = $set(0);
        let all = $set(-1);
        for start in (0..end).step_by($width) {
            let carry: [bool; $width] =
                std::array::from_fn(|row| $input.carry_um[start + row] != 0);
            // SAFETY: safe advance 已验证等长和独占；start+width <= end <= len。
            unsafe {
                let active = $mask($output.active, start);
                let length = $load($input.edge_length_mm.as_ptr().add(start).cast());
                let progress = $load($output.progress_mm.as_ptr().add(start).cast());
                let remaining = $load($output.remaining_mm.as_ptr().add(start).cast());
                let cursor = $load($output.route_cursor.as_ptr().add(start).cast());
                let leftover = $sub(length, $min(length, progress));
                let take = $min(leftover, remaining);
                let boundary = $eq(take, leftover);
                let hop = $and(
                    active,
                    $and(
                        boundary,
                        $and($mask($input.can_hop, start), $mask($input.has_next, start)),
                    ),
                );
                let stopped = $and(active, $and(boundary, $xor(hop, all)));
                let next_remaining = $blend(zero, $sub(remaining, take), hop);
                let added = $add(progress, take);
                let overflow = $and(
                    $eq($min(progress, added), added),
                    $xor($eq(progress, added), all),
                );
                let next_progress =
                    $blend($blend($or(added, overflow), length, stopped), zero, hop);
                let cursor_overflow = $and(hop, $eq(cursor, all));
                let next_cursor = $add(cursor, $and(hop, $set(1)));
                let still_active = $and(
                    $and(hop, $xor(cursor_overflow, all)),
                    $or($xor($eq(next_remaining, zero), all), $mask(&carry, 0)),
                );
                $store(
                    $output.progress_mm.as_mut_ptr().add(start).cast(),
                    $blend(
                        progress,
                        next_progress,
                        $and(active, $xor(cursor_overflow, all)),
                    ),
                );
                $store(
                    $output.remaining_mm.as_mut_ptr().add(start).cast(),
                    $blend(
                        remaining,
                        next_remaining,
                        $and(active, $xor(cursor_overflow, all)),
                    ),
                );
                $store(
                    $output.route_cursor.as_mut_ptr().add(start).cast(),
                    $blend(
                        cursor,
                        next_cursor,
                        $and(active, $xor(cursor_overflow, all)),
                    ),
                );
                let active_bits = $bits(still_active);
                let invalid_bits = $bits(cursor_overflow);
                $stats.hops += ($bits(hop) & !invalid_bits).count_ones() as usize;
                for row in 0..$width {
                    $output.active[start + row] = active_bits & (1 << row) != 0;
                    $output.valid[start + row] &= invalid_bits & (1 << row) == 0;
                }
            }
        }
        end
    }};
}

#[target_feature(enable = "avx2")]
pub(super) unsafe fn advance2(
    input: &EdgeInput<'_>,
    output: &mut EdgeOutput<'_>,
    stats: &mut EdgeStats,
) -> usize {
    edge_step!(
        input,
        output,
        stats,
        8,
        _mm256_set1_epi32,
        _mm256_loadu_si256,
        _mm256_storeu_si256,
        _mm256_min_epu32,
        _mm256_add_epi32,
        _mm256_sub_epi32,
        _mm256_and_si256,
        _mm256_or_si256,
        _mm256_xor_si256,
        eq2,
        bool2,
        blend2,
        bits2
    )
}

#[target_feature(enable = "avx512f")]
pub(super) unsafe fn advance512(
    input: &EdgeInput<'_>,
    output: &mut EdgeOutput<'_>,
    stats: &mut EdgeStats,
) -> usize {
    edge_step!(
        input,
        output,
        stats,
        16,
        _mm512_set1_epi32,
        _mm512_loadu_si512,
        _mm512_storeu_si512,
        _mm512_min_epu32,
        _mm512_add_epi32,
        _mm512_sub_epi32,
        _mm512_and_si512,
        _mm512_or_si512,
        _mm512_xor_si512,
        eq512,
        bool512,
        blend512,
        bits512
    )
}

#[target_feature(enable = "avx2")]
unsafe fn u32x8(values: &[u32], start: usize) -> __m256 {
    // SAFETY: 唯一调用点已证明 start+8 <= 所有输入列长度；使用无对齐要求的载入。
    let value = unsafe { _mm256_loadu_si256(values.as_ptr().add(start).cast()) };
    // 分成两个精确的 16 bit 部分，避免 i32 转换或先舍入低 31 bit 的二次舍入。
    let high = _mm256_cvtepi32_ps(_mm256_srli_epi32::<16>(value));
    let low = _mm256_cvtepi32_ps(_mm256_and_si256(value, _mm256_set1_epi32(0xffff)));
    _mm256_add_ps(_mm256_mul_ps(high, _mm256_set1_ps(65_536.0)), low)
}

#[target_feature(enable = "avx512f")]
unsafe fn u32x16(values: &[u32], start: usize) -> __m512 {
    // SAFETY: 唯一调用点已证明 start+16 <= 列长度；unsigned conversion 保留 u32 全域。
    _mm512_cvtepu32_ps(unsafe { _mm512_loadu_si512(values.as_ptr().add(start).cast()) })
}

#[target_feature(enable = "avx2")]
fn mask256(values: &[bool], start: usize) -> __m256 {
    let bits: [i32; 8] = std::array::from_fn(|row| if values[start + row] { -1 } else { 0 });
    // SAFETY: bits 本地数组恰有 8 个 i32，无对齐要求，不泄露指针。
    _mm256_castsi256_ps(unsafe { _mm256_loadu_si256(bits.as_ptr().cast()) })
}
fn mask512(values: &[bool], start: usize) -> __mmask16 {
    (0..16).fold(0, |mask, row| {
        mask | (u16::from(values[start + row]) << row)
    })
}

macro_rules! arithmetic {
    ($input:ident, $output:ident, $delta:ident, $phase:ident, $stats:ident, $track:ident, $quantize:ident, $width:literal,
     $set:ident, $load:ident, $store:ident, $uint:ident, $mask:ident,
     $add:ident, $sub:ident, $mul:ident, $div:ident, $max:ident, $min:ident,
     $sqrt:ident, $cmp:ident, $and:ident, $select:ident) => {{
        let end = $input.enabled.len() / $width * $width;
        let zero = $set(0.0); let one = $set(1.0); let two = $set(2.0); let half = $set(0.5);
        let dt = $set($delta); let inf = $set(f32::INFINITY); let neg_inf = $set(f32::NEG_INFINITY);
        for start in (0..end).step_by($width) {
            if $phase == Phase::Direct && !$input.enabled[start..start + $width].iter().any(|&v| v) {
                $output.valid[start..start + $width].fill(false);
                continue;
            }
            // SAFETY: run 验证全部长度；每轮 start+width <= end <= len。所有 load/store
            // 只访问这些切片；输出是独占 &mut，不能与输入或其它输出产生 Rust 别名。
            unsafe {
                let speed = $div($uint($input.speed_mm_s, start), $set(1_000.0));
                let desired = $div($uint($input.desired_mm_s, start), $set(1_000.0));
                let leader = $load($input.leader_m.as_ptr().add(start));
                let present = $mask($input.has_leader, start);
                let min_gap = $load($input.min_gap_m.as_ptr().add(start));
                let reused = $mask($input.has_proposal, start);
                let project = matches!($phase, Phase::Project | Phase::FloatProject);
                let all_reused = project || (start..start + $width).all(|row|
                    $input.has_proposal[row] || $phase == Phase::Direct && !$input.enabled[row]);
                let raw_speed = if all_reused {
                    $load($input.proposal_speed_m_s.as_ptr().add(start))
                } else {
                    let ratio = $max($div(speed, desired), zero);
                    let square = $mul(ratio, ratio);
                    let speed_term = $select($cmp::<_CMP_LE_OQ>(desired, zero), one, $mul(square, square));
                    let headway = $load($input.time_headway.as_ptr().add(start));
                    let gap_ratio = $max($div($add(min_gap, $mul(speed, headway)), leader), zero);
                    let gap_term = $select(present, $mul(gap_ratio, gap_ratio), zero);
                    let accel = $mul($load($input.max_accel.as_ptr().add(start)), $sub($sub(one, speed_term), gap_term));
                    let raw_speed = $min($max($add(speed, $mul(accel, dt)), zero), $max(desired, zero));
                    // F6 复审修 6：reused 是最外层选择——有 leader 的行也必须
                    // 优先消费调用方缓存提案（与标量 scalar_row 同序）。
                    let masked = $select(present, $select($cmp::<_CMP_LE_OQ>(leader, zero), zero, raw_speed), raw_speed);
                    $select(reused, $load($input.proposal_speed_m_s.as_ptr().add(start)), masked)
                };
                // #1272 候选形成与停止目标速度归约（与 scalar_row 同式；仅提案
                // 诞生处，Project 读到的提案已经过归约不得重放地板）：紧急下界
                // 先于空间目标（F3：过渡舒适档仅在实际耗尽时生效）。
                let comfort = $load($input.comfort_decel.as_ptr().add(start));
                let emergency = $load($input.emergency_decel.as_ptr().add(start));
                let stop_target = $min($load($input.stop_m.as_ptr().add(start)), $load($input.route_end_m.as_ptr().add(start)));
                let raw_speed = if !project {
                    let floor = $max($sub(speed, $mul(emergency, dt)), zero);
                    // R5 复审修（codex P2）：非有限提案（如缓存里的
                    // NEG_INFINITY）不得被地板抬成有限值——标量路径对非
                    // 有限提案保持拒绝，向量行先判有限再应用地板。
                    let finite = $and($cmp::<_CMP_LE_OQ>(neg_inf, raw_speed), $cmp::<_CMP_LE_OQ>(raw_speed, inf));
                    let lifted_unchecked = $select($cmp::<_CMP_LT_OQ>(raw_speed, floor), floor, raw_speed);
                    let lifted = $select(finite, lifted_unchecked, raw_speed);
                    // R6-3 复审修：整组不存在可能参与速度归约的目标（无效或
                    // 严格处于保守制动窗外）时跳过两档根式——窗外求解根不
                    // 低于已抬升提案，逐行 min 不改变结果；判据与 scalar_row
                    // 同式（stop_solve_participates）。紧急下界归一化与后续
                    // 投影义务保持不变；混合组继续用精确求解器。
                    let group_solve = (start..start + $width).any(|row| {
                        stop_solve_participates(
                            $input.speed_mm_s[row],
                            $input.desired_mm_s[row],
                            $input.max_accel[row],
                            $input.comfort_decel[row],
                            $delta,
                            $input.stop_m[row].min($input.route_end_m[row]),
                        )
                    });
                    if !group_solve {
                        lifted
                    } else {
                    let has_target = $and($and($cmp::<_CMP_GT_OQ>(stop_target, zero), $cmp::<_CMP_LT_OQ>(stop_target, inf)),
                        $cmp::<_CMP_GT_OQ>(lifted, zero));
                    let solve_stop = |decel: _| {
                        // 与 max_next_speed_for_decel（L=0）逐式同序：线性根恒 0；二次根
                        // 仅在判别式非负时求根（sqrt 负数产生 NaN，min 会把它变成
                        // 第二操作数）；可行域含本拍减速上界与拍内停稳分支。
                        let b_dt = $mul(decel, dt);
                        let constant = $sub($sub($mul($mul(decel, speed), dt), zero), $mul($mul(two, decel), stop_target));
                        let discriminant = $sub($mul(b_dt, b_dt), $mul(two, $mul(two, constant)));
                        let sqrt_disc = $sqrt($max(discriminant, zero));
                        let quadratic = $select($cmp::<_CMP_GE_OQ>(discriminant, zero),
                            $min($div($add($sub(zero, b_dt), sqrt_disc), two), lifted), $set(f32::NEG_INFINITY));
                        let holds = |candidate: _| {
                            let stop_branch = $div($mul(speed, speed), $mul(two, decel));
                            let trapezoid = $max($mul($mul($add(speed, candidate), half), dt), zero);
                            let stopped = $and($cmp::<_CMP_LE_OQ>(candidate, zero), $cmp::<_CMP_LE_OQ>(speed, b_dt));
                            let braking = $div($mul(candidate, candidate), $mul(two, decel));
                            $cmp::<_CMP_LE_OQ>($add($select(stopped, stop_branch, trapezoid), braking), stop_target)
                        };
                        let tier_floor = $max($sub(speed, b_dt), zero);
                        let domain = $cmp::<_CMP_LE_OQ>(tier_floor, lifted);
                        // 根代回失败时按确定性安全方向微降重试（与标量同序）。
                        let soft = $mul(quadratic, $set(ROOT_SHRINK_SOFT));
                        let hard = $mul(quadratic, $set(ROOT_SHRINK_HARD));
                        let corrected = $select(holds(quadratic), quadratic,
                            $select(holds(soft), soft,
                                $select(holds(hard), hard, $set(f32::NEG_INFINITY))));
                        let root_ok = $and($cmp::<_CMP_GT_OQ>(corrected, zero), $cmp::<_CMP_GT_OQ>(corrected, $set(f32::NEG_INFINITY)));
                        let best = $max($select(root_ok, corrected, zero), tier_floor);
                        let legal = $and($and($and($cmp::<_CMP_GT_OQ>(decel, zero), $cmp::<_CMP_GT_OQ>(dt, zero)), domain), holds(best));
                        ($min(best, lifted), legal)
                    };
                    let (comfort_cap, comfort_ok) = solve_stop(comfort);
                    let (emergency_cap, emergency_ok) = solve_stop(emergency);
                    // 两域皆空（具名投影）：保持档位下界推进（过渡用服务/舒适档），
                    // 位移由硬空间收到目标，线上速度交耗尽层按主导源分档。
                    let cascade = $select(comfort_ok, comfort_cap, $select(emergency_ok, emergency_cap, floor));
                    $select(has_target, $min(lifted, cascade), lifted)
                    }
                } else {
                    raw_speed
                };
                // 归约后位移按一致三元组重算；未触及行与原梯形逐位相同。
                let comfort_stop = $div($mul(speed, speed), $mul(two, comfort));
                let emergency_stop = $div($mul(speed, speed), $mul(two, emergency));
                let use_comfort = $and($cmp::<_CMP_LE_OQ>(raw_speed, zero),
                    $and($cmp::<_CMP_LE_OQ>(speed, $mul(comfort, dt)), $cmp::<_CMP_LE_OQ>(comfort_stop, stop_target)));
                let use_emergency = $and($cmp::<_CMP_LE_OQ>(raw_speed, zero),
                    $and($cmp::<_CMP_LE_OQ>(speed, $mul(emergency, dt)), $cmp::<_CMP_LE_OQ>(emergency_stop, stop_target)));
                let raw_travel = $select($and(use_comfort, $cmp::<_CMP_GT_OQ>(speed, zero)), comfort_stop,
                    $select(use_emergency, emergency_stop,
                        $max($mul($mul($add(speed, raw_speed), half), dt), zero)));
                let travel = if $phase != Phase::Proposal {
                    let leader_room = $max($sub(leader, min_gap), zero);
                    let reintegrated = raw_travel;
                    let mut travel = $select(present, $min(reintegrated, leader_room), reintegrated);
                    travel = $min(travel, $max($load($input.stop_m.as_ptr().add(start)), zero));
                    travel = $min(travel, $max($load($input.route_end_m.as_ptr().add(start)), zero));
                    travel = $max($min(travel, $load($input.envelope_m.as_ptr().add(start))), zero);
                    travel
                } else { zero };
                if let Some(floats) = $output.floats.as_mut() {
                    if $phase != Phase::Proposal { $store(floats.travel_m.as_mut_ptr().add(start), travel); }
                    $store(floats.proposal_speed_m_s.as_mut_ptr().add(start), raw_speed);
                    $store(floats.proposal_travel_m.as_mut_ptr().add(start), raw_travel);
                    if $phase == Phase::Proposal && let Some(window) = floats.window_m.as_mut() {
                        // 已证速度上界取窗：floor 可把候选抬到 desired 之上，
                        // 取 max(v, ·) 覆盖归一化后的共同候选。
                        let upper = $max($max($min($add(speed, $mul($load($input.max_accel.as_ptr().add(start)), dt)), $max(desired, zero)), speed), zero);
                        let value = $add($mul(dt, $add(speed, upper)), $div($mul(upper, upper), comfort));
                        $store(window.as_mut_ptr().add(start), value);
                    }
                } else {
                    // 仅本子批的有效性检查/受检回退需要栈值；量化继续消费同一向量寄存器，
                    // 不写回再读取世界范围的浮点列，不生成跨屏障中间提案。
                    let mut travels = [0.0; $width];
                    let mut speeds = [0.0; $width];
                    let mut proposals = [0.0; $width];
                    $store(travels.as_mut_ptr(), travel);
                    $store(speeds.as_mut_ptr(), raw_speed);
                    $store(proposals.as_mut_ptr(), raw_travel);
                    let valid: [bool; $width] = std::array::from_fn(|lane|
                        motion_values_valid($input, start + lane, $delta, proposals[lane], speeds[lane], travels[lane]));
                    $quantize::<$track>($input, $output, start, $delta, raw_speed, travel, &valid, $stats);
                }
                if $track {
                    for row in start..start + $width {
                        $stats.record_proposal_source($input.enabled[row], $input.has_proposal[row] || project);
                    }
                }
            }
        }
        end
    }};
}

#[target_feature(enable = "avx2")]
#[allow(clippy::too_many_arguments)]
unsafe fn quantize_direct2<const TRACK: bool>(
    input: &Input<'_>,
    output: &mut Columns<'_>,
    start: usize,
    delta_s: f32,
    raw_speed: __m256,
    travel: __m256,
    valid: &[bool],
    stats: &mut Stats,
) {
    // SAFETY: arithmetic 只在长度证明后的完整八行调用。拆为两个四行向量，
    // 不重读浮点列；栈上仅保留 f64 受检量化值，finish4 保留全 u64 回退。
    unsafe {
        let speeds = [
            _mm256_castps256_ps128(raw_speed),
            _mm256_extractf128_ps::<1>(raw_speed),
        ];
        let travels = [
            _mm256_castps256_ps128(travel),
            _mm256_extractf128_ps::<1>(travel),
        ];
        for group in 0..2 {
            let mut um = [0.0; 4];
            let mut speed = [0.0; 4];
            _mm256_storeu_pd(
                um.as_mut_ptr(),
                _mm256_round_pd::<{ _MM_FROUND_TO_NEAREST_INT | _MM_FROUND_NO_EXC }>(
                    _mm256_mul_pd(_mm256_cvtps_pd(travels[group]), _mm256_set1_pd(1_000_000.0)),
                ),
            );
            _mm256_storeu_pd(
                speed.as_mut_ptr(),
                _mm256_round_pd::<{ _MM_FROUND_TO_NEAREST_INT | _MM_FROUND_NO_EXC }>(
                    _mm256_mul_pd(_mm256_cvtps_pd(speeds[group]), _mm256_set1_pd(1_000.0)),
                ),
            );
            finish4::<TRACK>(
                input,
                output,
                start + group * 4,
                delta_s,
                &um,
                &speed,
                &valid[group * 4..group * 4 + 4],
                stats,
            );
        }
    }
}

#[target_feature(enable = "avx512f,avx2")]
#[allow(clippy::too_many_arguments)]
unsafe fn quantize_direct512<const TRACK: bool>(
    input: &Input<'_>,
    output: &mut Columns<'_>,
    start: usize,
    delta_s: f32,
    raw_speed: __m512,
    travel: __m512,
    valid: &[bool],
    stats: &mut Stats,
) {
    // SAFETY: arithmetic 证明完整十六行；extractf64x4 仅要求 AVX512F，
    // 按位转换拆两个八行，不引入 AVX512DQ 的额外后端前置条件。
    unsafe {
        let speeds = [
            _mm512_castps512_ps256(raw_speed),
            _mm256_castpd_ps(_mm512_extractf64x4_pd::<1>(_mm512_castps_pd(raw_speed))),
        ];
        let travels = [
            _mm512_castps512_ps256(travel),
            _mm256_castpd_ps(_mm512_extractf64x4_pd::<1>(_mm512_castps_pd(travel))),
        ];
        for group in 0..2 {
            let mut um = [0.0; 8];
            let mut speed = [0.0; 8];
            _mm512_storeu_pd(
                um.as_mut_ptr(),
                _mm512_roundscale_pd::<{ _MM_FROUND_TO_NEAREST_INT | _MM_FROUND_NO_EXC }>(
                    _mm512_mul_pd(_mm512_cvtps_pd(travels[group]), _mm512_set1_pd(1_000_000.0)),
                ),
            );
            _mm512_storeu_pd(
                speed.as_mut_ptr(),
                _mm512_roundscale_pd::<{ _MM_FROUND_TO_NEAREST_INT | _MM_FROUND_NO_EXC }>(
                    _mm512_mul_pd(_mm512_cvtps_pd(speeds[group]), _mm512_set1_pd(1_000.0)),
                ),
            );
            for half in 0..2 {
                let lane = half * 4;
                finish4::<TRACK>(
                    input,
                    output,
                    start + group * 8 + lane,
                    delta_s,
                    &um[lane..lane + 4],
                    &speed[lane..lane + 4],
                    &valid[group * 8 + lane..group * 8 + lane + 4],
                    stats,
                );
            }
        }
    }
}

#[target_feature(enable = "avx2")]
pub(super) unsafe fn avx2<const TRACK: bool>(
    input: &Input<'_>,
    output: &mut Columns<'_>,
    delta_s: f32,
    phase: Phase,
    stats: &mut Stats,
) -> usize {
    let end = if phase == Phase::Quantize {
        input.enabled.len() / 8 * 8
    } else {
        arithmetic!(
            input,
            output,
            delta_s,
            phase,
            stats,
            TRACK,
            quantize_direct2,
            8,
            _mm256_set1_ps,
            _mm256_loadu_ps,
            _mm256_storeu_ps,
            u32x8,
            mask256,
            _mm256_add_ps,
            _mm256_sub_ps,
            _mm256_mul_ps,
            _mm256_div_ps,
            _mm256_max_ps,
            _mm256_min_ps,
            _mm256_sqrt_ps,
            _mm256_cmp_ps,
            _mm256_and_ps,
            select256
        )
    };
    if phase == Phase::Direct {
        return end;
    }
    if matches!(phase, Phase::Proposal | Phase::FloatProject) {
        let floats = output.floats.as_ref().expect("float barrier columns");
        for row in 0..end {
            output.valid[row] = raw_valid(
                input,
                row,
                delta_s,
                floats.proposal_travel_m[row],
                floats.proposal_speed_m_s[row],
            ) && (phase == Phase::Proposal || floats.travel_m[row].is_finite());
        }
        return end;
    }
    for start in (0..end).step_by(4) {
        let floats = output
            .floats
            .as_ref()
            .expect("quantization barrier columns");
        let valid: [bool; 4] = std::array::from_fn(|lane| {
            motion_values_valid(
                input,
                start + lane,
                delta_s,
                floats.proposal_travel_m[start + lane],
                floats.proposal_speed_m_s[start + lane],
                floats.travel_m[start + lane],
            )
        });
        let mut um = [0.0; 4];
        let mut speed = [0.0; 4];
        // SAFETY: start+4 <= end；栈输出恰 4 个 f64。量化指令固定 ties-even，
        // 不修改 MXCSR。范围检查和 u64/u32 受检转换由 finish_values 统一完成。
        unsafe {
            let travel = _mm256_cvtps_pd(_mm_loadu_ps(floats.travel_m.as_ptr().add(start)));
            let raw_speed =
                _mm256_cvtps_pd(_mm_loadu_ps(floats.proposal_speed_m_s.as_ptr().add(start)));
            _mm256_storeu_pd(
                um.as_mut_ptr(),
                _mm256_round_pd::<{ _MM_FROUND_TO_NEAREST_INT | _MM_FROUND_NO_EXC }>(
                    _mm256_mul_pd(travel, _mm256_set1_pd(1_000_000.0)),
                ),
            );
            _mm256_storeu_pd(
                speed.as_mut_ptr(),
                _mm256_round_pd::<{ _MM_FROUND_TO_NEAREST_INT | _MM_FROUND_NO_EXC }>(
                    _mm256_mul_pd(raw_speed, _mm256_set1_pd(1_000.0)),
                ),
            );
        }
        // SAFETY: 上面的量化块和 finish4 都使用同一完整四行范围。
        unsafe {
            finish4::<TRACK>(input, output, start, delta_s, &um, &speed, &valid, stats);
        }
    }
    end
}

#[target_feature(enable = "avx512f,avx2")]
pub(super) unsafe fn avx512<const TRACK: bool>(
    input: &Input<'_>,
    output: &mut Columns<'_>,
    delta_s: f32,
    phase: Phase,
    stats: &mut Stats,
) -> usize {
    let end = if phase == Phase::Quantize {
        input.enabled.len() / 16 * 16
    } else {
        arithmetic!(
            input,
            output,
            delta_s,
            phase,
            stats,
            TRACK,
            quantize_direct512,
            16,
            _mm512_set1_ps,
            _mm512_loadu_ps,
            _mm512_storeu_ps,
            u32x16,
            mask512,
            _mm512_add_ps,
            _mm512_sub_ps,
            _mm512_mul_ps,
            _mm512_div_ps,
            _mm512_max_ps,
            _mm512_min_ps,
            _mm512_sqrt_ps,
            _mm512_cmp_ps_mask,
            and512,
            select512
        )
    };
    if phase == Phase::Direct {
        return end;
    }
    if matches!(phase, Phase::Proposal | Phase::FloatProject) {
        let floats = output.floats.as_ref().expect("float barrier columns");
        for row in 0..end {
            output.valid[row] = raw_valid(
                input,
                row,
                delta_s,
                floats.proposal_travel_m[row],
                floats.proposal_speed_m_s[row],
            ) && (phase == Phase::Proposal || floats.travel_m[row].is_finite());
        }
        return end;
    }
    for start in (0..end).step_by(8) {
        let floats = output
            .floats
            .as_ref()
            .expect("quantization barrier columns");
        let valid: [bool; 8] = std::array::from_fn(|lane| {
            motion_values_valid(
                input,
                start + lane,
                delta_s,
                floats.proposal_travel_m[start + lane],
                floats.proposal_speed_m_s[start + lane],
                floats.travel_m[start + lane],
            )
        });
        let mut um = [0.0; 8];
        let mut speed = [0.0; 8];
        // SAFETY: start+8 <= end；栈输出恰 8 个 f64，其它边界与 AVX2 相同。
        unsafe {
            let travel = _mm512_cvtps_pd(_mm256_loadu_ps(floats.travel_m.as_ptr().add(start)));
            let raw_speed = _mm512_cvtps_pd(_mm256_loadu_ps(
                floats.proposal_speed_m_s.as_ptr().add(start),
            ));
            _mm512_storeu_pd(
                um.as_mut_ptr(),
                _mm512_roundscale_pd::<{ _MM_FROUND_TO_NEAREST_INT | _MM_FROUND_NO_EXC }>(
                    _mm512_mul_pd(travel, _mm512_set1_pd(1_000_000.0)),
                ),
            );
            _mm512_storeu_pd(
                speed.as_mut_ptr(),
                _mm512_roundscale_pd::<{ _MM_FROUND_TO_NEAREST_INT | _MM_FROUND_NO_EXC }>(
                    _mm512_mul_pd(raw_speed, _mm512_set1_pd(1_000.0)),
                ),
            );
        }
        // SAFETY: 八行量化拆为两个完整四行整数发布范围，没有跨切片写入。
        unsafe {
            finish4::<TRACK>(
                input,
                output,
                start,
                delta_s,
                &um[..4],
                &speed[..4],
                &valid[..4],
                stats,
            );
            finish4::<TRACK>(
                input,
                output,
                start + 4,
                delta_s,
                &um[4..],
                &speed[4..],
                &valid[4..],
                stats,
            );
        }
    }
    end
}
