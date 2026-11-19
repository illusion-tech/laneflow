//! 含微米余数的提交车身区间严格分离判据。
//!
//! 首个跨 harness 共同性质（设计 §5）：每个 Active 车辆的提交车身沿其实际
//! 路线展开为带物理边身份的非空半开区间 `[back_um, front_um)`，同空间的
//! 不同 owner 非空区间不得相交，恰好相接合法；`min_gap` 不属于该性质。
//! owner 是 `(逻辑身份, incarnation)` 的完整值：任一分量不同即视为不同
//! owner，同 owner 的重复片段自排除，但不能遮蔽它与别的 owner 的重叠。
//!
//! 两条执行路径共用同一相交判据：任意片段集合用按 `(space, lo, hi, owner)`
//! 稳定排序后的线性扫描（每空间保留终点最远的两个不同 owner 候选），受检
//! 一维走廊内的有序相邻检查也调用同一半开判据；两条路径只在候选枚举和域
//! 证明上不同，不分别内联另一套重叠判断。

/// 事实错误：车身区间不得为空（新鲜摆放的 `[0,0]` 退化入口点不属于本性质
/// 的检查对象，由适配器在采集侧排除）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EmptyInterval {
    /// 违规区间在传入切片中的原始下标。
    pub index: usize,
}

/// 车身区间的 owner：`(逻辑身份, incarnation)` 完整值。
///
/// 逻辑身份载荷由 harness 紧凑映射，但须无碰撞、可核对且包含代次；不以可
/// 回收槽位或 Runtime 私有 generation 冒充宿主身份。
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct BodyOwner {
    /// harness 映射的逻辑身份（无碰撞）。
    pub logical: u64,
    /// 同一逻辑身份的代次。
    pub incarnation: u32,
}

/// 一个受检车身片段：`space` 内的半开区间 `[back_um, front_um)`。
///
/// `space` 是需互检的物理空间身份（Urban 为物理边原始号；Synth W 为受检
/// 一维走廊分组），单位为微米。
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct BodyInterval {
    /// 物理空间身份；不同空间不互检。
    pub space: u32,
    /// 区间起点（微米，含）。
    pub back_um: u64,
    /// 区间终点（微米，不含）；须大于 `back_um`。
    pub front_um: u64,
    /// 片段 owner 的完整身份。
    pub owner: BodyOwner,
}

impl BodyInterval {
    /// 半开区间的中点无关排序键：`(space, back, front, owner)`。
    fn sort_key(self) -> (u32, u64, u64, BodyOwner) {
        (self.space, self.back_um, self.front_um, self.owner)
    }
}

/// 半开区间相交的纯判据：`[a_back, a_front)` 与 `[b_back, b_front)` 有公共
/// 点当且仅当 `a_back < b_front && b_back < a_front`；恰好相接（一方终点
/// 等于另一方起点）不构成相交。
pub fn half_open_overlap(a_back: u64, a_front: u64, b_back: u64, b_front: u64) -> bool {
    a_back < b_front && b_back < a_front
}

/// 两个片段的分离判据：同一空间、owner 完整值不同且区间相交才判违反。
/// 空（或退化）区间不参与：调用方只应提交非空片段。
pub fn intervals_overlap(a: BodyInterval, b: BodyInterval) -> bool {
    a.space == b.space
        && a.owner != b.owner
        && half_open_overlap(a.back_um, a.front_um, b.back_um, b.front_um)
}

/// 集合扫描发现的首个相交片段对，`earlier` / `later` 是排序后切片中的
/// 下标且 `earlier < later`；诊断信息由调用方按 owner 与空间身份组装。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BodyOverlap {
    pub earlier: usize,
    pub later: usize,
}

/// 候选：某空间内按处理序见到的、终点最远的片段。并列时保留先到者
/// （严格大于才替换），保证给定排序输入时首个报错与候选选择确定。
#[derive(Clone, Copy)]
struct Candidate {
    interval: BodyInterval,
    index: usize,
}

impl Candidate {
    fn end(self) -> u64 {
        self.interval.front_um
    }
}

/// 在任意片段集合中找出首个相交对：先按 `(space, back, front, owner)` 稳定
/// 排序，再逐空间线性扫描。
///
/// 扫描维护每个空间的两个候选——终点最远的片段 `best`，以及 owner 与
/// `best` 不同者中终点最远的片段 `second`。处理当前片段 `x` 时，owner 与
/// `x` 不同的最远终点必然是二者之一：若 `best.owner != x.owner` 则为
/// `best`（全局最远），否则为 `second`（其 owner 与 `best` 即 `x` 的 owner
/// 相异）。因排序保证候选起点不晚于 `x` 起点，相交判定为
/// `candidate.front_um > x.back_um`（严格比较，相接合法）。该结构使同
/// owner 的较短嵌套片段不会遮蔽更长片段与其他 owner 的重叠。
///
/// 返回排序后切片中的首个相交对；空（`back >= front`）片段属事实错误，
/// 立即拒绝。集合收集与排序成本由调用方承担，不隐藏在判据的 O(1) 后面。
pub fn first_overlap(bodies: &mut [BodyInterval]) -> Result<Option<BodyOverlap>, EmptyInterval> {
    for (index, interval) in bodies.iter().enumerate() {
        if interval.back_um >= interval.front_um {
            return Err(EmptyInterval { index });
        }
    }
    bodies.sort_by_key(|&interval| interval.sort_key());
    let mut best: Option<Candidate> = None;
    let mut second: Option<Candidate> = None;
    for (index, &interval) in bodies.iter().enumerate() {
        if best.is_some_and(|b| b.interval.space != interval.space) {
            best = None;
            second = None;
        }
        // 选出 owner 不同的最远终点并作严格相交比较。
        let candidate = if best.is_some_and(|b| b.interval.owner == interval.owner) {
            second
        } else {
            best
        };
        if let Some(candidate) = candidate {
            debug_assert!(candidate.interval.back_um <= interval.back_um);
            if candidate.end() > interval.back_um {
                return Ok(Some(BodyOverlap {
                    earlier: candidate.index,
                    later: index,
                }));
            }
        }
        // 更新两个候选：终点严格更远才替换（并列保留先到者）。
        if best.is_none_or(|b| interval.front_um > b.end()) {
            // 新 best 到来时，旧 best 是与它相异 owner 中的最远者；
            // 若 owner 相同，则旧 second 仍是相异 owner 的最远者。
            second = match best {
                Some(old) if old.interval.owner != interval.owner => best,
                _ => second,
            };
            best = Some(Candidate { interval, index });
        } else if let Some(best) = best
            && interval.owner != best.interval.owner
            && second.is_none_or(|s| interval.front_um > s.end())
        {
            second = Some(Candidate { interval, index });
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(logical: u64, incarnation: u32) -> BodyOwner {
        BodyOwner {
            logical,
            incarnation,
        }
    }

    fn interval(space: u32, back: u64, front: u64, owner: BodyOwner) -> BodyInterval {
        BodyInterval {
            space,
            back_um: back,
            front_um: front,
            owner,
        }
    }

    /// 独立两两枚举参考：小规模集合的预期答案，不以生产 occupancy 查询
    /// 作为预期。
    fn pairwise_first(bodies: &[BodyInterval]) -> Option<(usize, usize)> {
        let mut sorted: Vec<(usize, BodyInterval)> = bodies.iter().copied().enumerate().collect();
        sorted.sort_by_key(|&(_, interval)| interval.sort_key());
        let order: Vec<usize> = sorted.iter().map(|&(index, _)| index).collect();
        let sorted_bodies: Vec<BodyInterval> =
            sorted.iter().map(|&(_, interval)| interval).collect();
        for later in 0..sorted_bodies.len() {
            for earlier in 0..later {
                if intervals_overlap(sorted_bodies[earlier], sorted_bodies[later]) {
                    return Some((order[earlier], order[later]));
                }
            }
        }
        None
    }

    #[test]
    fn nested_same_owner_segments_do_not_mask_foreign_overlap() {
        // 设计 §5.3 的必拒嵌套反例：A[0,100)、A[50,70)、B[80,90)。只比较
        // 排序后相邻片段并跳过同 owner 会漏检 A[0,100) 与 B[80,90)。
        let a = owner(1, 0);
        let b = owner(2, 0);
        let mut bodies = vec![
            interval(7, 0, 100, a),
            interval(7, 50, 70, a),
            interval(7, 80, 90, b),
        ];
        let overlap = first_overlap(&mut bodies).unwrap().expect("必须发现重叠");
        assert_eq!(
            (bodies[overlap.earlier].owner, bodies[overlap.later].owner),
            (a, b),
            "报错对必须是 A 的长片段与 B"
        );
        assert_eq!(
            (
                bodies[overlap.earlier].back_um,
                bodies[overlap.later].back_um
            ),
            (0, 80)
        );
    }

    #[test]
    fn same_owner_repeats_are_self_excluded() {
        let a = owner(1, 0);
        let mut bodies = vec![
            interval(7, 0, 100, a),
            interval(7, 50, 70, a),
            interval(7, 99, 150, a),
        ];
        assert_eq!(first_overlap(&mut bodies).unwrap(), None);
    }

    #[test]
    fn touching_is_legal_strict_comparison_only() {
        let a = owner(1, 0);
        let b = owner(2, 0);
        let mut bodies = vec![interval(7, 0, 100, a), interval(7, 100, 200, b)];
        assert_eq!(first_overlap(&mut bodies).unwrap(), None);
        // 相接的另一侧：前片段终点压到后片段起点之后 1 微米即违反。
        let mut bodies = vec![interval(7, 0, 101, a), interval(7, 100, 200, b)];
        assert!(first_overlap(&mut bodies).unwrap().is_some());
    }

    #[test]
    fn different_spaces_never_conflict() {
        let a = owner(1, 0);
        let b = owner(2, 0);
        let mut bodies = vec![interval(1, 0, 100, a), interval(2, 0, 100, b)];
        assert_eq!(first_overlap(&mut bodies).unwrap(), None);
    }

    #[test]
    fn different_incarnation_means_different_owner() {
        // 同逻辑身份、不同代次：视为不同 owner，重叠必须报出。
        let old = owner(1, 0);
        let new = owner(1, 1);
        let mut bodies = vec![interval(7, 0, 100, old), interval(7, 90, 120, new)];
        assert!(first_overlap(&mut bodies).unwrap().is_some());
    }

    #[test]
    fn duplicate_physical_edges_from_distinct_occurrences_meet_in_one_space() {
        // 相同物理边的不同 occurrence 进入同一物理边比较：同一 space 值下
        // 不同 owner 的重叠必须报出，不因来源 occurrence 不同而豁免。
        let a = owner(1, 0);
        let b = owner(2, 0);
        let mut bodies = vec![interval(9, 10, 60, a), interval(9, 10, 60, b)];
        assert!(first_overlap(&mut bodies).unwrap().is_some());
    }

    #[test]
    fn empty_interval_is_a_fact_error() {
        let a = owner(1, 0);
        let mut bodies = vec![interval(7, 5, 5, a)];
        assert_eq!(first_overlap(&mut bodies), Err(EmptyInterval { index: 0 }));
        let mut bodies = vec![interval(7, 0, 100, a), interval(7, 60, 50, a)];
        assert_eq!(first_overlap(&mut bodies), Err(EmptyInterval { index: 1 }));
    }

    /// 简单可复现的伪随机源（xorshift64*），不引入外部依赖。
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn below(&mut self, bound: u64) -> u64 {
            self.next() % bound
        }
    }

    #[test]
    fn sweep_agrees_with_pairwise_enumeration_on_random_small_sets() {
        // 覆盖嵌套、同 owner、重复边、相接、不同 incarnation 与输入排列：
        // 随机小集合上，扫描的“是否存在相交”及“报出的那一对确实相交”均与
        // 独立两两枚举一致。
        let mut rng = Lcg(0x1457_0000_0000_0001);
        for _case in 0..2_000 {
            let len = 1 + rng.below(8) as usize;
            let mut bodies = Vec::with_capacity(len);
            for _ in 0..len {
                let space = rng.below(3) as u32;
                let owner = owner(rng.below(4), rng.below(2) as u32);
                let lo = rng.below(200);
                let hi = lo + 1 + rng.below(60);
                bodies.push(interval(space, lo, hi, owner));
            }
            let expected = pairwise_first(&bodies);
            let mut swept = bodies.clone();
            let found = first_overlap(&mut swept).unwrap();
            match (found, expected) {
                (None, None) => {}
                (Some(overlap), Some(_)) => {
                    // 报出的对必须真实相交（与自身排序副本核对）。
                    let a = swept[overlap.earlier];
                    let b = swept[overlap.later];
                    assert!(intervals_overlap(a, b), "虚报：{a:?} 与 {b:?}");
                    assert!(overlap.earlier < overlap.later);
                }
                (found, expected) => {
                    panic!(
                        "扫描与两两枚举不一致：found={found:?} expected={expected:?} bodies={bodies:?}"
                    )
                }
            }
        }
    }

    #[test]
    fn permutations_yield_the_same_first_overlap() {
        // 同一集合打乱输入顺序：排序后扫描结果（按片段值定位）必须一致，
        // 保障不同 worker/分块切分下首错确定。
        let mut rng = Lcg(0x1457_0000_0000_0002);
        for _case in 0..500 {
            let len = 2 + rng.below(6) as usize;
            let mut bodies = Vec::with_capacity(len);
            for _ in 0..len {
                let space = rng.below(2) as u32;
                let owner = owner(rng.below(3), rng.below(2) as u32);
                let lo = rng.below(100);
                let hi = lo + 1 + rng.below(40);
                bodies.push(interval(space, lo, hi, owner));
            }
            let mut reference = bodies.clone();
            let expected = first_overlap(&mut reference)
                .unwrap()
                .map(|o| (reference[o.earlier], reference[o.later]));
            for _shuffle in 0..4 {
                for i in (1..bodies.len()).rev() {
                    bodies.swap(i, rng.below(i as u64 + 1) as usize);
                }
                let mut shuffled = bodies.clone();
                let found = first_overlap(&mut shuffled)
                    .unwrap()
                    .map(|o| (shuffled[o.earlier], shuffled[o.later]));
                assert_eq!(found, expected, "输入排列改变了首错：{bodies:?}");
            }
        }
    }

    #[test]
    fn tied_end_same_owner_keeps_earliest_candidate_deterministically() {
        // 同 owner、终点并列的长片段到来时不得替换先到者：报错对钉在排序
        // 更靠前的片段上，给定排序输入时首错确定。
        let a = owner(1, 0);
        let b = owner(2, 0);
        let mut bodies = vec![
            interval(7, 0, 100, a),
            interval(7, 50, 100, a),
            interval(7, 99, 120, b),
        ];
        let overlap = first_overlap(&mut bodies).unwrap().expect("必须发现重叠");
        assert_eq!(bodies[overlap.earlier].back_um, 0, "并列终点保留先到者");
        assert_eq!(bodies[overlap.later].owner, b);
    }

    #[test]
    fn zero_length_not_required_and_length_crossing_short_edges_not_modeled_here() {
        // 本判据只回答“非空半开区间是否相交”；车长跨多条短边的展开、起点
        // 截断与单位换算由事实适配器负责（设计 §5.2），此处不重复建模。
        let a = owner(1, 0);
        assert!(half_open_overlap(0, 10, 5, 15));
        assert!(!half_open_overlap(0, 10, 10, 20));
        assert!(!half_open_overlap(0, 10, 20, 30));
        assert!(intervals_overlap(
            interval(1, 0, 10, a),
            interval(1, 5, 15, owner(2, 0))
        ));
        assert!(!intervals_overlap(
            interval(1, 0, 10, a),
            interval(1, 5, 15, a)
        ));
    }
}
