/// 首个生产编译资源上限配置档的稳定标识符。
const P100_INITIAL_V1_PROFILE_ID: &str = "LF-COMP-P100-INITIAL-v1";

/// 编译资源上限诊断使用的有类型维度。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum CompileLimitDimension {
    ModuleCount,
    ImportEdgeCount,
    SourceBytesPerModule,
    SourceBytesTotal,
    DeclarationCount,
    TypedAstRecordCount,
    HirRecordCount,
    MirRecordCount,
    LirRecordCount,
    ReferenceCount,
    RelationOccurrenceCount,
    IdentityFieldOccurrenceCount,
    RouteOccurrenceCount,
    ManeuverGateCount,
    WaitingZoneCount,
    GeometryPointCount,
    SymbolCount,
    StringItemCount,
    SingleStringBytes,
    TotalStringBytes,
    DiagnosticCount,
    StageScratchBytes,
    OutputBytes,
    CompilerControlledLiveBytes,
    RetainedCapacityBytes,
}

impl CompileLimitDimension {
    /// 返回设计文档登记的精确私有配置字段名。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ModuleCount => "max_module_count",
            Self::ImportEdgeCount => "max_import_edge_count",
            Self::SourceBytesPerModule => "max_source_bytes_per_module",
            Self::SourceBytesTotal => "max_source_bytes_total",
            Self::DeclarationCount => "max_declaration_count",
            Self::TypedAstRecordCount => "max_typed_ast_record_count",
            Self::HirRecordCount => "max_hir_record_count",
            Self::MirRecordCount => "max_mir_record_count",
            Self::LirRecordCount => "max_lir_record_count",
            Self::ReferenceCount => "max_reference_count",
            Self::RelationOccurrenceCount => "max_relation_occurrence_count",
            Self::IdentityFieldOccurrenceCount => "max_identity_field_occurrence_count",
            Self::RouteOccurrenceCount => "max_route_occurrence_count",
            Self::ManeuverGateCount => "max_maneuver_gate_count",
            Self::WaitingZoneCount => "max_waiting_zone_count",
            Self::GeometryPointCount => "max_geometry_point_count",
            Self::SymbolCount => "max_symbol_count",
            Self::StringItemCount => "max_string_item_count",
            Self::SingleStringBytes => "max_single_string_bytes",
            Self::TotalStringBytes => "max_total_string_bytes",
            Self::DiagnosticCount => "max_diagnostic_count",
            Self::StageScratchBytes => "max_stage_scratch_bytes",
            Self::OutputBytes => "max_output_bytes",
            Self::CompilerControlledLiveBytes => "max_compiler_controlled_live_bytes",
            Self::RetainedCapacityBytes => "max_retained_capacity_bytes",
        }
    }
}

/// 一次编译使用的显式、不可变资源上限。
///
/// 字段保持私有，避免把编译器内部阶段布局冻结为公共兼容面。生产调用方必须显式选择
/// 具名配置档；本类型有意不实现 [`Default`]。
#[derive(Clone)]
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

    pub(crate) const fn value(&self, dimension: CompileLimitDimension) -> u64 {
        match dimension {
            CompileLimitDimension::ModuleCount => self.max_module_count as u64,
            CompileLimitDimension::ImportEdgeCount => self.max_import_edge_count as u64,
            CompileLimitDimension::SourceBytesPerModule => self.max_source_bytes_per_module as u64,
            CompileLimitDimension::SourceBytesTotal => self.max_source_bytes_total as u64,
            CompileLimitDimension::DeclarationCount => self.max_declaration_count as u64,
            CompileLimitDimension::TypedAstRecordCount => self.max_typed_ast_record_count as u64,
            CompileLimitDimension::HirRecordCount => self.max_hir_record_count as u64,
            CompileLimitDimension::MirRecordCount => self.max_mir_record_count as u64,
            CompileLimitDimension::LirRecordCount => self.max_lir_record_count as u64,
            CompileLimitDimension::ReferenceCount => self.max_reference_count as u64,
            CompileLimitDimension::RelationOccurrenceCount => {
                self.max_relation_occurrence_count as u64
            }
            CompileLimitDimension::IdentityFieldOccurrenceCount => {
                self.max_identity_field_occurrence_count as u64
            }
            CompileLimitDimension::RouteOccurrenceCount => self.max_route_occurrence_count as u64,
            CompileLimitDimension::ManeuverGateCount => self.max_maneuver_gate_count as u64,
            CompileLimitDimension::WaitingZoneCount => self.max_waiting_zone_count as u64,
            CompileLimitDimension::GeometryPointCount => self.max_geometry_point_count as u64,
            CompileLimitDimension::SymbolCount => self.max_symbol_count as u64,
            CompileLimitDimension::StringItemCount => self.max_string_item_count as u64,
            CompileLimitDimension::SingleStringBytes => self.max_single_string_bytes as u64,
            CompileLimitDimension::TotalStringBytes => self.max_total_string_bytes as u64,
            CompileLimitDimension::DiagnosticCount => self.max_diagnostic_count as u64,
            CompileLimitDimension::StageScratchBytes => self.max_stage_scratch_bytes as u64,
            CompileLimitDimension::OutputBytes => self.max_output_bytes as u64,
            CompileLimitDimension::CompilerControlledLiveBytes => {
                self.max_compiler_controlled_live_bytes as u64
            }
            CompileLimitDimension::RetainedCapacityBytes => self.max_retained_capacity_bytes as u64,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_test_string_limits(
        mut self,
        total_string_bytes: u32,
        compiler_controlled_live_bytes: u32,
    ) -> Self {
        self.max_total_string_bytes = total_string_bytes;
        self.max_compiler_controlled_live_bytes = compiler_controlled_live_bytes;
        self
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

    #[test]
    fn typed_dimensions_map_to_every_private_limit_field() {
        let limits = CompileLimits::p100_initial_v1();
        let expected = [
            (CompileLimitDimension::ModuleCount, 522),
            (CompileLimitDimension::ImportEdgeCount, 1_032),
            (CompileLimitDimension::SourceBytesPerModule, 542_741),
            (CompileLimitDimension::SourceBytesTotal, 542_741),
            (CompileLimitDimension::DeclarationCount, 11_265),
            (CompileLimitDimension::TypedAstRecordCount, 58_387),
            (CompileLimitDimension::HirRecordCount, 58_387),
            (CompileLimitDimension::MirRecordCount, 38_112),
            (CompileLimitDimension::LirRecordCount, 38_112),
            (CompileLimitDimension::ReferenceCount, 37_920),
            (CompileLimitDimension::RelationOccurrenceCount, 10_032),
            (CompileLimitDimension::IdentityFieldOccurrenceCount, 29_184),
            (CompileLimitDimension::RouteOccurrenceCount, 1_920),
            (CompileLimitDimension::ManeuverGateCount, 2_304),
            (CompileLimitDimension::WaitingZoneCount, 1_536),
            (CompileLimitDimension::GeometryPointCount, 22_368),
            (CompileLimitDimension::SymbolCount, 11_265),
            (CompileLimitDimension::StringItemCount, 36_894),
            (CompileLimitDimension::SingleStringBytes, 53),
            (CompileLimitDimension::TotalStringBytes, 991_537),
            (CompileLimitDimension::DiagnosticCount, 16),
            (CompileLimitDimension::StageScratchBytes, 304_896),
            (CompileLimitDimension::OutputBytes, 2_782_758),
            (
                CompileLimitDimension::CompilerControlledLiveBytes,
                43_269_120,
            ),
            (CompileLimitDimension::RetainedCapacityBytes, 36_925_688),
        ];

        for (dimension, expected_value) in expected {
            assert!(dimension.as_str().starts_with("max_"));
            assert_eq!(limits.value(dimension), expected_value);
        }
    }
}
