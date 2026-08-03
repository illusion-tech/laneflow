/// 首个生产编译资源上限配置档的稳定标识符。
const P100_INITIAL_V1_PROFILE_ID: &str = "LF-COMP-P100-INITIAL-v1";

/// 一次编译使用的显式、不可变资源上限。
///
/// 字段保持私有，避免把编译器内部阶段布局冻结为公共兼容面。生产调用方必须显式选择
/// 具名配置档；本类型有意不实现 [`Default`]。
pub struct CompileLimits {
    max_module_count: u32,
    max_import_edge_count: u32,
    max_source_bytes_per_module: u32,
    max_source_bytes_total: u32,
    max_declaration_count: u32,
    max_typed_ast_record_count: u32,
    max_hir_record_count: u32,
    max_mir_record_count: u32,
    max_lir_record_count: u32,
    max_reference_count: u32,
    max_relation_occurrence_count: u32,
    max_identity_field_occurrence_count: u32,
    max_route_occurrence_count: u32,
    max_maneuver_gate_count: u32,
    max_waiting_zone_count: u32,
    max_geometry_point_count: u32,
    max_symbol_count: u32,
    max_string_item_count: u32,
    max_single_string_bytes: u32,
    max_total_string_bytes: u32,
    max_diagnostic_count: u32,
    max_stage_scratch_bytes: u32,
    max_output_bytes: u32,
    max_compiler_controlled_live_bytes: u32,
    max_retained_capacity_bytes: u32,
}

impl Clone for CompileLimits {
    fn clone(&self) -> Self {
        Self {
            max_module_count: self.max_module_count,
            max_import_edge_count: self.max_import_edge_count,
            max_source_bytes_per_module: self.max_source_bytes_per_module,
            max_source_bytes_total: self.max_source_bytes_total,
            max_declaration_count: self.max_declaration_count,
            max_typed_ast_record_count: self.max_typed_ast_record_count,
            max_hir_record_count: self.max_hir_record_count,
            max_mir_record_count: self.max_mir_record_count,
            max_lir_record_count: self.max_lir_record_count,
            max_reference_count: self.max_reference_count,
            max_relation_occurrence_count: self.max_relation_occurrence_count,
            max_identity_field_occurrence_count: self.max_identity_field_occurrence_count,
            max_route_occurrence_count: self.max_route_occurrence_count,
            max_maneuver_gate_count: self.max_maneuver_gate_count,
            max_waiting_zone_count: self.max_waiting_zone_count,
            max_geometry_point_count: self.max_geometry_point_count,
            max_symbol_count: self.max_symbol_count,
            max_string_item_count: self.max_string_item_count,
            max_single_string_bytes: self.max_single_string_bytes,
            max_total_string_bytes: self.max_total_string_bytes,
            max_diagnostic_count: self.max_diagnostic_count,
            max_stage_scratch_bytes: self.max_stage_scratch_bytes,
            max_output_bytes: self.max_output_bytes,
            max_compiler_controlled_live_bytes: self.max_compiler_controlled_live_bytes,
            max_retained_capacity_bytes: self.max_retained_capacity_bytes,
        }
    }
}

impl CompileLimits {
    /// 选择 #292 G1 冻结的首个生产资源配置档。
    #[must_use]
    pub const fn p100_initial_v1() -> Self {
        Self {
            max_module_count: 522,
            max_import_edge_count: 1_032,
            max_source_bytes_per_module: 542_741,
            max_source_bytes_total: 542_741,
            max_declaration_count: 11_265,
            max_typed_ast_record_count: 58_387,
            max_hir_record_count: 58_387,
            max_mir_record_count: 38_112,
            max_lir_record_count: 38_112,
            max_reference_count: 37_920,
            max_relation_occurrence_count: 10_032,
            max_identity_field_occurrence_count: 29_184,
            max_route_occurrence_count: 1_920,
            max_maneuver_gate_count: 2_304,
            max_waiting_zone_count: 1_536,
            max_geometry_point_count: 22_368,
            max_symbol_count: 11_265,
            max_string_item_count: 36_894,
            max_single_string_bytes: 53,
            max_total_string_bytes: 991_537,
            max_diagnostic_count: 16,
            max_stage_scratch_bytes: 304_896,
            max_output_bytes: 2_782_758,
            max_compiler_controlled_live_bytes: 43_269_120,
            max_retained_capacity_bytes: 36_925_688,
        }
    }

    /// 返回调用方显式选择的稳定配置档标识符。
    #[must_use]
    pub const fn profile_id(&self) -> &'static str {
        P100_INITIAL_V1_PROFILE_ID
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p100_initial_v1_matches_accepted_exact_limits() {
        let limits = CompileLimits::p100_initial_v1();

        assert_eq!(limits.profile_id(), "LF-COMP-P100-INITIAL-v1");
        assert_eq!(limits.max_module_count, 522);
        assert_eq!(limits.max_import_edge_count, 1_032);
        assert_eq!(limits.max_source_bytes_per_module, 542_741);
        assert_eq!(limits.max_source_bytes_total, 542_741);
        assert_eq!(limits.max_declaration_count, 11_265);
        assert_eq!(limits.max_typed_ast_record_count, 58_387);
        assert_eq!(limits.max_hir_record_count, 58_387);
        assert_eq!(limits.max_mir_record_count, 38_112);
        assert_eq!(limits.max_lir_record_count, 38_112);
        assert_eq!(limits.max_reference_count, 37_920);
        assert_eq!(limits.max_relation_occurrence_count, 10_032);
        assert_eq!(limits.max_identity_field_occurrence_count, 29_184);
        assert_eq!(limits.max_route_occurrence_count, 1_920);
        assert_eq!(limits.max_maneuver_gate_count, 2_304);
        assert_eq!(limits.max_waiting_zone_count, 1_536);
        assert_eq!(limits.max_geometry_point_count, 22_368);
        assert_eq!(limits.max_symbol_count, 11_265);
        assert_eq!(limits.max_string_item_count, 36_894);
        assert_eq!(limits.max_single_string_bytes, 53);
        assert_eq!(limits.max_total_string_bytes, 991_537);
        assert_eq!(limits.max_diagnostic_count, 16);
        assert_eq!(limits.max_stage_scratch_bytes, 304_896);
        assert_eq!(limits.max_output_bytes, 2_782_758);
        assert_eq!(limits.max_compiler_controlled_live_bytes, 43_269_120);
        assert_eq!(limits.max_retained_capacity_bytes, 36_925_688);
    }
}
