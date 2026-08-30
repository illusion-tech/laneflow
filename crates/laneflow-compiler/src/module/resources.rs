use std::mem::size_of;

use crate::CompileLimitDimension;

/// 由具体官方前端一次性派生、供共同准入复核的模块资源计数。
pub(crate) struct ModuleResourceCounts {
    pub(crate) source_bytes: u64,
    pub(crate) declaration_count: u64,
    pub(crate) stable_entity_count: u64,
    pub(crate) typed_ast_record_count: u64,
    pub(crate) reference_count: u64,
    pub(crate) relation_occurrence_count: u64,
    pub(crate) identity_field_occurrence_count: u64,
    pub(crate) symbol_count: u64,
    pub(crate) string_item_count: u64,
    pub(crate) string_bytes: u64,
    pub(crate) maneuver_gate_count: u64,
    pub(crate) waiting_zone_count: u64,
    pub(crate) geometry_point_count: u64,
    pub(crate) geometry_source_range_count: u64,
    pub(crate) controlled_live_bytes: u64,
    /// 具体官方前端构造本候选模块期间的受控存续峰值，不含既有 builder。
    pub(crate) admission_peak_live_bytes: u64,
}

/// 编译单元准入过程中唯一的累计资源状态。
///
/// 候选值按值计算并完整通过限额校验后才替换构建器中的当前值，避免并行字段在新增
/// 维度时漏更新。`module_payload_live_bytes` 只累计模块载荷，不含共同模块/文档索引或
/// 构建器/结果容器；这些结构由准入层在候选基数确定后按阶段一次组装进存续与峰值账本。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct AdmissionTotals {
    pub(super) module_count: u64,
    pub(super) source_document_count: u64,
    pub(super) source_bytes_total: u64,
    pub(super) import_edge_count: u64,
    pub(super) declaration_count: u64,
    pub(super) stable_entity_count: u64,
    pub(super) typed_ast_record_count: u64,
    pub(super) reference_count: u64,
    pub(super) relation_occurrence_count: u64,
    pub(super) identity_field_occurrence_count: u64,
    pub(super) symbol_count: u64,
    pub(super) string_item_count: u64,
    pub(super) string_bytes: u64,
    pub(super) maneuver_gate_count: u64,
    pub(super) waiting_zone_count: u64,
    pub(super) geometry_point_count: u64,
    pub(super) geometry_source_range_count: u64,
    pub(super) module_payload_live_bytes: u64,
    pub(super) module_slot_capacity: u64,
    pub(super) admission_peak_live_bytes: u64,
}

impl AdmissionTotals {
    #[inline]
    pub(super) fn candidate_after(
        self,
        source_document_count: u64,
        import_edge_count: u64,
        counts: &ModuleResourceCounts,
    ) -> Self {
        let module_count = self.module_count.saturating_add(1);
        Self {
            module_count,
            source_document_count: self
                .source_document_count
                .saturating_add(source_document_count),
            source_bytes_total: self.source_bytes_total.saturating_add(counts.source_bytes),
            import_edge_count: self.import_edge_count.saturating_add(import_edge_count),
            declaration_count: self
                .declaration_count
                .saturating_add(counts.declaration_count),
            stable_entity_count: self
                .stable_entity_count
                .saturating_add(counts.stable_entity_count),
            typed_ast_record_count: self
                .typed_ast_record_count
                .saturating_add(counts.typed_ast_record_count),
            reference_count: self.reference_count.saturating_add(counts.reference_count),
            relation_occurrence_count: self
                .relation_occurrence_count
                .saturating_add(counts.relation_occurrence_count),
            identity_field_occurrence_count: self
                .identity_field_occurrence_count
                .saturating_add(counts.identity_field_occurrence_count),
            symbol_count: self.symbol_count.saturating_add(counts.symbol_count),
            string_item_count: self
                .string_item_count
                .saturating_add(counts.string_item_count),
            string_bytes: self.string_bytes.saturating_add(counts.string_bytes),
            maneuver_gate_count: self
                .maneuver_gate_count
                .saturating_add(counts.maneuver_gate_count),
            waiting_zone_count: self
                .waiting_zone_count
                .saturating_add(counts.waiting_zone_count),
            geometry_point_count: self
                .geometry_point_count
                .saturating_add(counts.geometry_point_count),
            geometry_source_range_count: self
                .geometry_source_range_count
                .saturating_add(counts.geometry_source_range_count),
            module_payload_live_bytes: self
                .module_payload_live_bytes
                .saturating_add(counts.controlled_live_bytes),
            module_slot_capacity: planned_module_slot_capacity(
                self.module_slot_capacity,
                module_count,
            ),
            admission_peak_live_bytes: self.admission_peak_live_bytes,
        }
    }

    #[inline]
    pub(super) fn limit_observations(
        self,
        controlled_live_bytes: u64,
    ) -> [(CompileLimitDimension, u64); 16] {
        [
            (CompileLimitDimension::ModuleCount, self.module_count),
            (
                CompileLimitDimension::SourceBytesTotal,
                self.source_bytes_total,
            ),
            (
                CompileLimitDimension::ImportEdgeCount,
                self.import_edge_count,
            ),
            (
                CompileLimitDimension::DeclarationCount,
                self.declaration_count,
            ),
            (
                CompileLimitDimension::StableEntityCount,
                self.stable_entity_count,
            ),
            (
                CompileLimitDimension::TypedAstRecordCount,
                self.typed_ast_record_count,
            ),
            (CompileLimitDimension::ReferenceCount, self.reference_count),
            (
                CompileLimitDimension::RelationOccurrenceCount,
                self.relation_occurrence_count,
            ),
            (
                CompileLimitDimension::IdentityFieldOccurrenceCount,
                self.identity_field_occurrence_count,
            ),
            (CompileLimitDimension::SymbolCount, self.symbol_count),
            (
                CompileLimitDimension::StringItemCount,
                self.string_item_count,
            ),
            (CompileLimitDimension::TotalStringBytes, self.string_bytes),
            (
                CompileLimitDimension::ManeuverGateCount,
                self.maneuver_gate_count,
            ),
            (
                CompileLimitDimension::WaitingZoneCount,
                self.waiting_zone_count,
            ),
            (
                CompileLimitDimension::GeometryPointCount,
                self.geometry_point_count,
            ),
            (
                CompileLimitDimension::CompilerControlledLiveBytes,
                controlled_live_bytes,
            ),
        ]
    }
}

fn planned_module_slot_capacity(current: u64, required: u64) -> u64 {
    if required <= current {
        return current;
    }
    required.next_power_of_two().max(4)
}

pub(crate) fn size_bytes<T>(count: u64) -> u64 {
    count.saturating_mul(u64::try_from(size_of::<T>()).unwrap_or(u64::MAX))
}

pub(super) fn requested_hash_table_bytes<K, V>(entry_count: u64) -> u64 {
    if entry_count == 0 {
        return 0;
    }
    // 与阶段工作集预算采用同一保守模型：为每项预留八个桶、每桶一个控制字节，
    // 再计入尾部控制区；不依赖标准库私有布局或实际哈希表迭代顺序。
    let bucket_bytes = u64::try_from(size_of::<(K, V)>())
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    entry_count
        .saturating_mul(8)
        .saturating_mul(bucket_bytes)
        .saturating_add(16)
}
