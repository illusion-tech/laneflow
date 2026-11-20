//! 独立计数构建的局部账本；逐车计数只写块内值，主线程按原块序相加。

use serde::Serialize;

/// 这些是已有循环的访问次数，不是 CPU 指令数；成功拍才进入成本输出。
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub(crate) struct WorkloadCounts {
    pub pre_step_scans: u64,
    pub pre_step_blocks: u64,
    pub pre_step_slot_visits: u64,
    pub pre_step_vehicle_lookups: u64,
    pub state_slot_visits: u64,
    pub state_scans: u64,
    pub state_blocks: u64,
    pub state_vehicle_lookups: u64,
    pub crossed_route_hop_visits: u64,
    pub body_route_occurrence_visits: u64,
    pub body_fragments: u64,
    pub body_sort_input_rows: u64,
    pub body_sort_calls: u64,
    pub body_merge_output_rows: u64,
    pub body_merge_passes: u64,
    pub body_nonempty_scan_rows: u64,
    pub body_nonempty_scans: u64,
    pub body_overlap_scan_rows: u64,
    pub body_overlap_scans: u64,
    pub body_sorted_debug_pairs: u64,
    pub waiting_decision_encode_rows: u64,
    pub waiting_decision_encode_blocks: u64,
    pub waiting_decision_evidence_rows: u64,
    pub waiting_decision_evidence_scans: u64,
    pub conflict_decision_encode_rows: u64,
    pub conflict_decision_encode_blocks: u64,
    pub conflict_decision_evidence_rows: u64,
    pub conflict_decision_evidence_scans: u64,
    pub claim_owner_check_rows: u64,
    pub claim_owner_check_blocks: u64,
    pub claim_copy_rows: u64,
    pub claim_exclusivity_rows: u64,
    pub claim_exclusivity_scans: u64,
    pub claim_encode_rows: u64,
    pub claim_encode_scans: u64,
    pub waiting_zone_rows: u64,
    pub signal_group_rows: u64,
    pub parking_target_rows: u64,
    pub parking_facility_rows: u64,
    pub parking_space_binding_rows: u64,
}

impl WorkloadCounts {
    pub(crate) fn add(&mut self, other: Self) {
        macro_rules! sum {
            ($($field:ident),+ $(,)?) => { $(self.$field += other.$field;)+ };
        }
        sum!(
            pre_step_scans,
            pre_step_blocks,
            state_scans,
            state_blocks,
            body_sort_calls,
            body_merge_passes,
            body_nonempty_scans,
            body_overlap_scans,
            waiting_decision_encode_blocks,
            waiting_decision_evidence_scans,
            conflict_decision_encode_blocks,
            conflict_decision_evidence_scans,
            claim_owner_check_blocks,
            claim_copy_rows,
            claim_exclusivity_scans,
            claim_encode_scans,
            pre_step_slot_visits,
            pre_step_vehicle_lookups,
            state_slot_visits,
            state_vehicle_lookups,
            crossed_route_hop_visits,
            body_route_occurrence_visits,
            body_fragments,
            body_sort_input_rows,
            body_merge_output_rows,
            body_nonempty_scan_rows,
            body_overlap_scan_rows,
            body_sorted_debug_pairs,
            waiting_decision_encode_rows,
            waiting_decision_evidence_rows,
            conflict_decision_encode_rows,
            conflict_decision_evidence_rows,
            claim_owner_check_rows,
            claim_exclusivity_rows,
            claim_encode_rows,
            waiting_zone_rows,
            signal_group_rows,
            parking_target_rows,
            parking_facility_rows,
            parking_space_binding_rows,
        );
    }
}

/// allocator 的分配/释放/重分配独立列出；字节是分配流量，不是驻留内存。
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub(crate) struct AllocationCounts {
    pub allocations: usize,
    pub deallocations: usize,
    pub reallocations: usize,
    pub bytes_allocated: usize,
    pub bytes_deallocated: usize,
    pub bytes_reallocated: isize,
}

impl From<stats_alloc::Stats> for AllocationCounts {
    fn from(stats: stats_alloc::Stats) -> Self {
        Self {
            allocations: stats.allocations,
            deallocations: stats.deallocations,
            reallocations: stats.reallocations,
            bytes_allocated: stats.bytes_allocated,
            bytes_deallocated: stats.bytes_deallocated,
            bytes_reallocated: stats.bytes_reallocated,
        }
    }
}

impl AllocationCounts {
    fn add(&mut self, other: Self) {
        self.allocations += other.allocations;
        self.deallocations += other.deallocations;
        self.reallocations += other.reallocations;
        self.bytes_allocated += other.bytes_allocated;
        self.bytes_deallocated += other.bytes_deallocated;
        self.bytes_reallocated += other.bytes_reallocated;
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub(crate) struct ObservationAllocation {
    pub pre_step: AllocationCounts,
    pub post_step: AllocationCounts,
}

impl ObservationAllocation {
    pub(crate) fn add(&mut self, other: Self) {
        self.pre_step.add(other.pre_step);
        self.post_step.add(other.post_step);
    }
}

/// 每个值在成功拍之后采样，跨拍逐字段取最大；不是同一时点的 RSS。
/// Vec 容量用元素数；编码缓冲用字节；哈希表用可容纳条目数。
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub(crate) struct ScratchHighWater {
    pub blocks_len: usize,
    pub blocks_capacity: usize,
    pub block_encoding_bytes_capacity_sum: usize,
    pub block_json_bytes_capacity_sum: usize,
    pub block_body_capacity_sum: usize,
    pub block_state_capacity_sum: usize,
    pub body_capacity: usize,
    pub merge_capacity: usize,
    pub body_and_merge_capacity_sum: usize,
    pub runs_capacity: usize,
    pub claims_capacity: usize,
    pub zone_owners_capacity: usize,
    pub step_before_capacity: usize,
    pub scratch_red_capacity: usize,
}

impl ScratchHighWater {
    /// 逐字段记录新高出现次数；采样之间的真实 realloc 次数由 allocator 另记。
    pub(crate) fn include(&mut self, other: Self, increases: &mut Self) {
        macro_rules! maximum {
            ($($field:ident),+ $(,)?) => { $(
                if other.$field > self.$field {
                    self.$field = other.$field;
                    increases.$field += 1;
                }
            )+ };
        }
        maximum!(
            blocks_len,
            blocks_capacity,
            block_encoding_bytes_capacity_sum,
            block_json_bytes_capacity_sum,
            block_body_capacity_sum,
            block_state_capacity_sum,
            body_capacity,
            merge_capacity,
            body_and_merge_capacity_sum,
            runs_capacity,
            claims_capacity,
            zone_owners_capacity,
            step_before_capacity,
            scratch_red_capacity,
        );
    }
}

/// 检查库链接进入最终可执行文件时，计数 allocator 确实处于全局分配路径。
/// 探针在采样区间和整轮墙钟之外，分配通过 black_box 防止优化消除。
pub(crate) fn require_allocator() -> crate::Result<()> {
    let region = stats_alloc::Region::new(crate::allocation::ALLOCATOR);
    let probe = std::hint::black_box(vec![0_u8; std::hint::black_box(8_192)]);
    std::hint::black_box(&probe);
    let stats = region.change();
    if stats.allocations == 0 || stats.bytes_allocated < probe.len() {
        return Err(crate::invalid(
            "verification-cost counts 未安装全局计数分配器",
        ));
    }
    Ok(())
}
