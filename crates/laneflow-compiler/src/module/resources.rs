use std::mem::size_of;

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

/// 编译单元准入过程中唯一的累计计数状态。
///
/// 候选值按值计算并完整通过限额校验后才替换构建器中的当前值，避免并行字段在新增
/// 维度时漏更新。受控存续由准入账本按授予记录；`module_payload_live_bytes` 只累计
/// 模块载荷的计划字节，`*_slot_capacity` 是 builder 模块表与索引按倍增预留的容量。
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
    pub(super) source_document_slot_capacity: u64,
    pub(super) admission_peak_live_bytes: u64,
}

impl AdmissionTotals {
    /// 返回叠加候选模块计数后的准入累计状态。
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
            source_document_slot_capacity: planned_module_slot_capacity(
                self.source_document_slot_capacity,
                self.source_document_count
                    .saturating_add(source_document_count),
            ),
            admission_peak_live_bytes: self.admission_peak_live_bytes,
        }
    }

    /// 汇总需对照限额配置档校验的计数维度观测值；受控存续由准入账本在计划准入时核对。
    #[inline]
    pub(super) fn limit_violations(
        self,
        profile: &crate::CompileProfile,
    ) -> [Option<crate::limits::ResourceLimitExceeded>; 15] {
        [
            profile.source_limits().exceeded(
                crate::SourceProcessingDimension::ModuleCount,
                self.module_count,
            ),
            profile.source_limits().exceeded(
                crate::SourceProcessingDimension::SourceBytesTotal,
                self.source_bytes_total,
            ),
            profile.source_limits().exceeded(
                crate::SourceProcessingDimension::ImportEdgeCount,
                self.import_edge_count,
            ),
            profile.source_limits().exceeded(
                crate::SourceProcessingDimension::DeclarationCount,
                self.declaration_count,
            ),
            profile.early_canonical_exceeded(
                laneflow_static_contract::CanonicalNetworkDimension::StableEntityCount,
                self.stable_entity_count,
            ),
            profile.source_limits().exceeded(
                crate::SourceProcessingDimension::TypedAstRecordCount,
                self.typed_ast_record_count,
            ),
            profile.source_limits().exceeded(
                crate::SourceProcessingDimension::ReferenceCount,
                self.reference_count,
            ),
            profile.source_limits().exceeded(
                crate::SourceProcessingDimension::RelationOccurrenceCount,
                self.relation_occurrence_count,
            ),
            profile.source_limits().exceeded(
                crate::SourceProcessingDimension::IdentityFieldOccurrenceCount,
                self.identity_field_occurrence_count,
            ),
            profile.source_limits().exceeded(
                crate::SourceProcessingDimension::SymbolCount,
                self.symbol_count,
            ),
            profile.source_limits().exceeded(
                crate::SourceProcessingDimension::StringItemCount,
                self.string_item_count,
            ),
            profile.source_limits().exceeded(
                crate::SourceProcessingDimension::TotalStringBytes,
                self.string_bytes,
            ),
            profile.early_canonical_exceeded(
                laneflow_static_contract::CanonicalNetworkDimension::ManeuverGateCount,
                self.maneuver_gate_count,
            ),
            profile.early_canonical_exceeded(
                laneflow_static_contract::CanonicalNetworkDimension::WaitingZoneCount,
                self.waiting_zone_count,
            ),
            profile.early_canonical_exceeded(
                laneflow_static_contract::CanonicalNetworkDimension::GeometryPointCount,
                self.geometry_point_count,
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

/// 按类型布局估算 `count` 个元素占用的字节数（饱和计算）。
pub(crate) fn size_bytes<T>(count: u64) -> u64 {
    count.saturating_mul(u64::try_from(size_of::<T>()).unwrap_or(u64::MAX))
}
