//! 编译资源上限的具名生产配置档。
//!
//! 同一份 [`CompileLimits`] 从来源构造一直传递到各编译阶段，使所有与输入规模成正比
//! 的复制、表构造和暂存区分配都能在分配前失败关闭。字段保持私有，防止调用方拼出
//! 未经校准的“无限制”配置，也避免把阶段内部计数布局冻结成公共兼容接口。

/// 首个生产编译资源上限配置档的稳定标识符。
const P100_INITIAL_V1_PROFILE_ID: &str = "LF-COMP-P100-INITIAL-v1";
/// 首个显式限定多文档逻辑模块的生产配置档标识符。
const P100_INITIAL_V2_PROFILE_ID: &str = "LF-COMP-P100-INITIAL-v2";

#[derive(Clone, Copy)]
enum CompileLimitsProfile {
    P100InitialV1,
    P100InitialV2,
}

/// 编译资源上限诊断使用的有类型维度。
///
/// `*Count` 维度以记录或出现次数计，`*Bytes` 维度以字节计。枚举值本身不作为线格式
/// 代码；稳定诊断字段名由 [`CompileLimitDimension::as_str`] 提供。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum CompileLimitDimension {
    /// 编译单元中的来源模块数。
    ModuleCount,
    /// 编译单元中独立登记的来源文档数。
    SourceDocumentCount,
    /// 模块图中的显式导入边数。
    ImportEdgeCount,
    /// 单个模块的规范来源记录字节数。
    SourceBytesPerModule,
    /// 编译单元全部规范来源记录的累计字节数。
    SourceBytesTotal,
    /// 全部领域声明数。
    DeclarationCount,
    /// Typed AST 的逻辑记录数。
    TypedAstRecordCount,
    /// HIR 的逻辑记录数。
    HirRecordCount,
    /// MIR 的逻辑记录数。
    MirRecordCount,
    /// Canonical LIR 的逻辑记录数。
    LirRecordCount,
    /// 有类型引用出现次数。
    ReferenceCount,
    /// 多值关系中的成员出现次数。
    RelationOccurrenceCount,
    /// 规范身份字段出现次数。
    IdentityFieldOccurrenceCount,
    /// ManeuverGate 声明数。
    ManeuverGateCount,
    /// WaitingZone 声明数。
    WaitingZoneCount,
    /// 几何点记录数。
    GeometryPointCount,
    /// 需要进入符号表的声明数。
    SymbolCount,
    /// 资源模型计入的逻辑字符串项数；相同文本的多次语义出现分别计数。
    StringItemCount,
    /// 单个受检字符串允许的最大字节数。
    SingleStringBytes,
    /// 资源模型计入的逻辑字符串累计字节数。
    TotalStringBytes,
    /// 一次失败最多保留的规范有序诊断数。
    DiagnosticCount,
    /// 任一编译阶段同时需要的私有暂存区字节数。
    StageScratchBytes,
    /// 编译输出受控字节数。
    OutputBytes,
    /// 编译器拥有且在某一阶段同时存续的峰值字节预算。
    CompilerControlledLiveBytes,
    /// 可复用编译器实例在一次编译后允许保留的容量字节数。
    RetainedCapacityBytes,
}

impl CompileLimitDimension {
    /// 返回设计文档登记的精确私有配置字段名。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ModuleCount => "max_module_count",
            Self::SourceDocumentCount => "max_source_document_count",
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
    profile: CompileLimitsProfile,
    max_module_count: u32,
    max_source_document_count: Option<u32>,
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
    ///
    /// 返回值是完整快照；后续校准若改变任一精确上限或维度集合，必须使用新的配置档
    /// 标识符，而不能原地改变 `LF-COMP-P100-INITIAL-v1` 的语义。
    #[must_use]
    pub const fn p100_initial_v1() -> Self {
        Self {
            profile: CompileLimitsProfile::P100InitialV1,
            max_module_count: 522,
            max_source_document_count: None,
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

    /// 选择 #315 G1 冻结的多文档生产资源配置档。
    ///
    /// v2 逐项继承 v1 的精确上限，只新增编译单元最多 1,566 份来源
    /// 文档的显式维度。v1 依然只接受每模块一份文档的形状。
    #[must_use]
    pub const fn p100_initial_v2() -> Self {
        let mut limits = Self::p100_initial_v1();
        limits.profile = CompileLimitsProfile::P100InitialV2;
        limits.max_source_document_count = Some(1_566);
        limits
    }

    /// 单个受检编制字符串 / Identity ASCII 字段的字节上限。
    #[must_use]
    pub const fn max_single_string_bytes(&self) -> u64 {
        self.max_single_string_bytes as u64
    }

    /// 返回调用方显式选择的稳定配置档标识符。
    #[must_use]
    pub const fn profile_id(&self) -> &'static str {
        match self.profile {
            CompileLimitsProfile::P100InitialV1 => P100_INITIAL_V1_PROFILE_ID,
            CompileLimitsProfile::P100InitialV2 => P100_INITIAL_V2_PROFILE_ID,
        }
    }

    /// 返回配置档是否显式支持独立来源文档数维度。
    pub(crate) const fn source_document_count_limit(&self) -> Option<u64> {
        match self.max_source_document_count {
            Some(limit) => Some(limit as u64),
            None => None,
        }
    }

    /// 返回某维度的精确上限，并统一提升为 `u64` 供饱和计数比较。
    pub(crate) const fn value(&self, dimension: CompileLimitDimension) -> u64 {
        match dimension {
            CompileLimitDimension::ModuleCount => self.max_module_count as u64,
            CompileLimitDimension::SourceDocumentCount => match self.max_source_document_count {
                Some(limit) => limit as u64,
                None => {
                    panic!("selected compile-limits profile has no source-document-count dimension")
                }
            },
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

    #[cfg(test)]
    pub(crate) fn with_test_single_string_limit(mut self, single_string_bytes: u32) -> Self {
        self.max_single_string_bytes = single_string_bytes;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_test_pipeline_limits(
        mut self,
        hir_record_count: u32,
        mir_record_count: u32,
        stage_scratch_bytes: u32,
        compiler_controlled_live_bytes: u32,
    ) -> Self {
        self.max_hir_record_count = hir_record_count;
        self.max_mir_record_count = mir_record_count;
        self.max_stage_scratch_bytes = stage_scratch_bytes;
        self.max_compiler_controlled_live_bytes = compiler_controlled_live_bytes;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_test_lir_limits(
        mut self,
        lir_record_count: u32,
        stage_scratch_bytes: u32,
        output_bytes: u32,
        compiler_controlled_live_bytes: u32,
    ) -> Self {
        self.max_lir_record_count = lir_record_count;
        self.max_stage_scratch_bytes = stage_scratch_bytes;
        self.max_output_bytes = output_bytes;
        self.max_compiler_controlled_live_bytes = compiler_controlled_live_bytes;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_test_source_byte_limits(mut self, per_module: u32, total: u32) -> Self {
        self.max_source_bytes_per_module = per_module;
        self.max_source_bytes_total = total;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_test_admission_limit(
        mut self,
        dimension: CompileLimitDimension,
        limit: u32,
    ) -> Self {
        match dimension {
            CompileLimitDimension::ModuleCount => self.max_module_count = limit,
            CompileLimitDimension::SourceDocumentCount => {
                self.max_source_document_count = Some(limit)
            }
            CompileLimitDimension::ImportEdgeCount => self.max_import_edge_count = limit,
            CompileLimitDimension::SourceBytesTotal => self.max_source_bytes_total = limit,
            CompileLimitDimension::DeclarationCount => self.max_declaration_count = limit,
            CompileLimitDimension::TypedAstRecordCount => self.max_typed_ast_record_count = limit,
            CompileLimitDimension::ReferenceCount => self.max_reference_count = limit,
            CompileLimitDimension::RelationOccurrenceCount => {
                self.max_relation_occurrence_count = limit
            }
            CompileLimitDimension::IdentityFieldOccurrenceCount => {
                self.max_identity_field_occurrence_count = limit
            }
            CompileLimitDimension::SymbolCount => self.max_symbol_count = limit,
            CompileLimitDimension::StringItemCount => self.max_string_item_count = limit,
            CompileLimitDimension::TotalStringBytes => self.max_total_string_bytes = limit,
            CompileLimitDimension::ManeuverGateCount => self.max_maneuver_gate_count = limit,
            CompileLimitDimension::WaitingZoneCount => self.max_waiting_zone_count = limit,
            CompileLimitDimension::GeometryPointCount => self.max_geometry_point_count = limit,
            CompileLimitDimension::StageScratchBytes => self.max_stage_scratch_bytes = limit,
            CompileLimitDimension::CompilerControlledLiveBytes => {
                self.max_compiler_controlled_live_bytes = limit
            }
            _ => panic!("dimension is not enforced by common official-module admission"),
        }
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
        assert_eq!(limits.max_source_document_count, None);
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
    fn p100_initial_v2_only_adds_the_accepted_source_document_limit() {
        let v1 = CompileLimits::p100_initial_v1();
        let v2 = CompileLimits::p100_initial_v2();

        assert_eq!(v2.profile_id(), "LF-COMP-P100-INITIAL-v2");
        assert_eq!(v1.source_document_count_limit(), None);
        assert_eq!(v2.source_document_count_limit(), Some(1_566));
        for dimension in [
            CompileLimitDimension::ModuleCount,
            CompileLimitDimension::ImportEdgeCount,
            CompileLimitDimension::SourceBytesPerModule,
            CompileLimitDimension::SourceBytesTotal,
            CompileLimitDimension::DeclarationCount,
            CompileLimitDimension::TypedAstRecordCount,
            CompileLimitDimension::HirRecordCount,
            CompileLimitDimension::MirRecordCount,
            CompileLimitDimension::LirRecordCount,
            CompileLimitDimension::ReferenceCount,
            CompileLimitDimension::RelationOccurrenceCount,
            CompileLimitDimension::IdentityFieldOccurrenceCount,
            CompileLimitDimension::ManeuverGateCount,
            CompileLimitDimension::WaitingZoneCount,
            CompileLimitDimension::GeometryPointCount,
            CompileLimitDimension::SymbolCount,
            CompileLimitDimension::StringItemCount,
            CompileLimitDimension::SingleStringBytes,
            CompileLimitDimension::TotalStringBytes,
            CompileLimitDimension::DiagnosticCount,
            CompileLimitDimension::StageScratchBytes,
            CompileLimitDimension::OutputBytes,
            CompileLimitDimension::CompilerControlledLiveBytes,
            CompileLimitDimension::RetainedCapacityBytes,
        ] {
            assert_eq!(v2.value(dimension), v1.value(dimension));
        }
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
