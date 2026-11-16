//! 编译资源上限的具名生产配置档。
//!
//! 同一份 [`CompileLimits`] 从来源构造一直传递到各编译阶段。配置档只声明容量维度与
//! 宿主预算维度（ADR 0035）：容量维度约束输入与中间记录规模，宿主预算维度约束编译器
//! 控制的总存续字节与保留容量。暂存、输出等编译器内部字节量由资源账本报告，不是限额。
//! 字段保持私有，防止调用方拼出未经校准的“无限制”配置，也避免把阶段内部计数布局冻结成
//! 公共兼容接口。

use laneflow_static_contract::FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES;

/// P100 生产编译资源配置档的稳定标识符。
const P100_PROFILE_ID: &str = "LF-COMP-P100";
/// 单个百万级现实混合静态路网的生产资源配置档标识符。
const SINGLE_NETWORK_1M_PROFILE_ID: &str = "LF-COMP-SINGLE-NETWORK-1M";

#[derive(Clone, Copy)]
enum CompileLimitsProfile {
    P100,
    SingleNetwork1m,
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
    /// 完整逻辑路网中的稳定静态实体数。
    StableEntityCount,
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
    /// 宿主预算：编译器控制且同时存续的受控请求字节。
    CompilerControlledLiveBytes,
    /// 宿主预算：可复用编译器实例在一次编译后允许保留的容量字节数。
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
            Self::StableEntityCount => "max_stable_entity_count",
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
            Self::CompilerControlledLiveBytes => "max_compiler_controlled_live_bytes",
            Self::RetainedCapacityBytes => "max_retained_capacity_bytes",
        }
    }
}

/// 一次编译使用的显式、不可变资源上限。
///
/// 只含容量维度与宿主预算维度，每个具名配置档都定义全部维度。字段保持私有，避免把
/// 编译器内部阶段布局冻结为公共兼容面。生产调用方必须显式选择具名配置档；本类型有意
/// 不实现 [`Default`]。
#[derive(Clone)]
pub struct CompileLimits {
    profile: CompileLimitsProfile,
    max_module_count: u32,
    max_source_document_count: u32,
    max_import_edge_count: u32,
    max_source_bytes_per_module: u64,
    max_source_bytes_total: u64,
    max_declaration_count: u32,
    max_stable_entity_count: u32,
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
    max_single_string_bytes: u64,
    max_total_string_bytes: u64,
    max_diagnostic_count: u32,
    max_compiler_controlled_live_bytes: u64,
    max_retained_capacity_bytes: u64,
}

impl CompileLimits {
    /// 选择 P100 生产编译资源配置档 `LF-COMP-P100`。
    ///
    /// 计数维度、来源字节与字符串字节来自 #308 九个压力分层的逐维上包络，宿主预算取
    /// 同一校准的最大建议值；来源文档数等于 522 个模块乘以每模块三个来源文档。返回值
    /// 是完整快照，改变任一维度须同时更新编译器基础 §5.3 的配置档表。
    #[must_use]
    pub const fn p100() -> Self {
        Self {
            profile: CompileLimitsProfile::P100,
            max_module_count: 522,
            max_source_document_count: 1_566,
            max_import_edge_count: 1_032,
            max_source_bytes_per_module: 542_741,
            max_source_bytes_total: 542_741,
            max_declaration_count: 11_265,
            max_stable_entity_count: 11_265,
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
            max_compiler_controlled_live_bytes: 43_269_120,
            max_retained_capacity_bytes: 36_925_688,
        }
    }

    /// 选择单个百万级现实混合静态路网的具名生产配置档 `LF-COMP-SINGLE-NETWORK-1M`。
    ///
    /// 该配置档约束一个逻辑路网修订的完整编译、file-backed emission 与共享静态构建
    /// 输入；它不是对任意理论实体组合的性能承诺，也不要求按所有上限预分配内存。
    #[must_use]
    pub const fn single_network_1m() -> Self {
        Self {
            profile: CompileLimitsProfile::SingleNetwork1m,
            max_module_count: 65_536,
            max_source_document_count: 196_608,
            max_import_edge_count: 262_144,
            max_source_bytes_per_module: 536_870_912,
            max_source_bytes_total: 536_870_912,
            max_declaration_count: 1_500_000,
            max_stable_entity_count: 1_000_000,
            max_typed_ast_record_count: 8_000_000,
            max_hir_record_count: 8_000_000,
            max_mir_record_count: 8_000_000,
            max_lir_record_count: 8_000_000,
            max_reference_count: 16_000_000,
            max_relation_occurrence_count: 16_000_000,
            max_identity_field_occurrence_count: 8_000_000,
            max_maneuver_gate_count: 1_000_000,
            max_waiting_zone_count: 1_000_000,
            max_geometry_point_count: 16_000_000,
            max_symbol_count: 2_000_000,
            max_string_item_count: 8_000_000,
            max_single_string_bytes: 4_096,
            max_total_string_bytes: 536_870_912,
            max_diagnostic_count: 16,
            max_compiler_controlled_live_bytes: 6_442_450_944,
            max_retained_capacity_bytes: 536_870_912,
        }
    }

    /// 单个受检编制字符串的配置档字节上限。
    #[must_use]
    pub const fn max_single_string_bytes(&self) -> u64 {
        self.max_single_string_bytes
    }

    /// Identity v1 ASCII 字段同时受配置档和可移植格式硬上限约束。
    pub(crate) const fn identity_ascii_bytes_limit(&self) -> u64 {
        if self.max_single_string_bytes < FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES {
            self.max_single_string_bytes
        } else {
            FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES
        }
    }

    /// 返回调用方显式选择的稳定配置档标识符。
    #[must_use]
    pub const fn profile_id(&self) -> &'static str {
        match self.profile {
            CompileLimitsProfile::P100 => P100_PROFILE_ID,
            CompileLimitsProfile::SingleNetwork1m => SINGLE_NETWORK_1M_PROFILE_ID,
        }
    }

    /// 返回某维度的精确上限，并统一提升为 `u64` 供饱和计数比较。
    pub(crate) const fn value(&self, dimension: CompileLimitDimension) -> u64 {
        match dimension {
            CompileLimitDimension::ModuleCount => self.max_module_count as u64,
            CompileLimitDimension::SourceDocumentCount => self.max_source_document_count as u64,
            CompileLimitDimension::ImportEdgeCount => self.max_import_edge_count as u64,
            CompileLimitDimension::SourceBytesPerModule => self.max_source_bytes_per_module,
            CompileLimitDimension::SourceBytesTotal => self.max_source_bytes_total,
            CompileLimitDimension::DeclarationCount => self.max_declaration_count as u64,
            CompileLimitDimension::StableEntityCount => self.max_stable_entity_count as u64,
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
            CompileLimitDimension::SingleStringBytes => self.max_single_string_bytes,
            CompileLimitDimension::TotalStringBytes => self.max_total_string_bytes,
            CompileLimitDimension::DiagnosticCount => self.max_diagnostic_count as u64,
            CompileLimitDimension::CompilerControlledLiveBytes => {
                self.max_compiler_controlled_live_bytes
            }
            CompileLimitDimension::RetainedCapacityBytes => self.max_retained_capacity_bytes,
        }
    }

    /// 测试辅助：覆盖总字符串字节与编译器控制存续字节上限。
    #[cfg(test)]
    pub(crate) fn with_test_string_limits(
        mut self,
        total_string_bytes: u32,
        compiler_controlled_live_bytes: u32,
    ) -> Self {
        self.max_total_string_bytes = total_string_bytes.into();
        self.max_compiler_controlled_live_bytes = compiler_controlled_live_bytes.into();
        self
    }

    /// 测试辅助：覆盖单字符串字节上限。
    #[cfg(test)]
    pub(crate) fn with_test_single_string_limit(mut self, single_string_bytes: u32) -> Self {
        self.max_single_string_bytes = single_string_bytes.into();
        self
    }

    /// 测试辅助：覆盖 HIR/MIR 记录数与存续字节上限。
    #[cfg(test)]
    pub(crate) fn with_test_pipeline_limits(
        mut self,
        hir_record_count: u32,
        mir_record_count: u32,
        compiler_controlled_live_bytes: u32,
    ) -> Self {
        self.max_hir_record_count = hir_record_count;
        self.max_mir_record_count = mir_record_count;
        self.max_compiler_controlled_live_bytes = compiler_controlled_live_bytes.into();
        self
    }

    /// 测试辅助：覆盖 LIR 记录数与存续字节上限。
    #[cfg(test)]
    pub(crate) fn with_test_lir_limits(
        mut self,
        lir_record_count: u32,
        compiler_controlled_live_bytes: u32,
    ) -> Self {
        self.max_lir_record_count = lir_record_count;
        self.max_compiler_controlled_live_bytes = compiler_controlled_live_bytes.into();
        self
    }

    /// 测试辅助：覆盖每模块来源字节与总来源字节上限。
    #[cfg(test)]
    pub(crate) fn with_test_source_byte_limits(mut self, per_module: u32, total: u32) -> Self {
        self.max_source_bytes_per_module = per_module.into();
        self.max_source_bytes_total = total.into();
        self
    }

    /// 测试辅助：按维度覆盖单条接入检查上限。
    #[cfg(test)]
    pub(crate) fn with_test_admission_limit(
        mut self,
        dimension: CompileLimitDimension,
        limit: u32,
    ) -> Self {
        match dimension {
            CompileLimitDimension::ModuleCount => self.max_module_count = limit,
            CompileLimitDimension::SourceDocumentCount => self.max_source_document_count = limit,
            CompileLimitDimension::ImportEdgeCount => self.max_import_edge_count = limit,
            CompileLimitDimension::SourceBytesTotal => self.max_source_bytes_total = limit.into(),
            CompileLimitDimension::DeclarationCount => self.max_declaration_count = limit,
            CompileLimitDimension::StableEntityCount => self.max_stable_entity_count = limit,
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
            CompileLimitDimension::TotalStringBytes => self.max_total_string_bytes = limit.into(),
            CompileLimitDimension::ManeuverGateCount => self.max_maneuver_gate_count = limit,
            CompileLimitDimension::WaitingZoneCount => self.max_waiting_zone_count = limit,
            CompileLimitDimension::GeometryPointCount => self.max_geometry_point_count = limit,
            CompileLimitDimension::CompilerControlledLiveBytes => {
                self.max_compiler_controlled_live_bytes = limit.into()
            }
            _ => panic!("dimension is not enforced by common official-module admission"),
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const P100_EXPECTED: [(CompileLimitDimension, u64); 24] = [
        (CompileLimitDimension::ModuleCount, 522),
        (CompileLimitDimension::SourceDocumentCount, 1_566),
        (CompileLimitDimension::ImportEdgeCount, 1_032),
        (CompileLimitDimension::SourceBytesPerModule, 542_741),
        (CompileLimitDimension::SourceBytesTotal, 542_741),
        (CompileLimitDimension::DeclarationCount, 11_265),
        (CompileLimitDimension::StableEntityCount, 11_265),
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
        (
            CompileLimitDimension::CompilerControlledLiveBytes,
            43_269_120,
        ),
        (CompileLimitDimension::RetainedCapacityBytes, 36_925_688),
    ];

    const SINGLE_NETWORK_1M_EXPECTED: [(CompileLimitDimension, u64); 24] = [
        (CompileLimitDimension::ModuleCount, 65_536),
        (CompileLimitDimension::SourceDocumentCount, 196_608),
        (CompileLimitDimension::ImportEdgeCount, 262_144),
        (CompileLimitDimension::SourceBytesPerModule, 536_870_912),
        (CompileLimitDimension::SourceBytesTotal, 536_870_912),
        (CompileLimitDimension::DeclarationCount, 1_500_000),
        (CompileLimitDimension::StableEntityCount, 1_000_000),
        (CompileLimitDimension::TypedAstRecordCount, 8_000_000),
        (CompileLimitDimension::HirRecordCount, 8_000_000),
        (CompileLimitDimension::MirRecordCount, 8_000_000),
        (CompileLimitDimension::LirRecordCount, 8_000_000),
        (CompileLimitDimension::ReferenceCount, 16_000_000),
        (CompileLimitDimension::RelationOccurrenceCount, 16_000_000),
        (
            CompileLimitDimension::IdentityFieldOccurrenceCount,
            8_000_000,
        ),
        (CompileLimitDimension::ManeuverGateCount, 1_000_000),
        (CompileLimitDimension::WaitingZoneCount, 1_000_000),
        (CompileLimitDimension::GeometryPointCount, 16_000_000),
        (CompileLimitDimension::SymbolCount, 2_000_000),
        (CompileLimitDimension::StringItemCount, 8_000_000),
        (CompileLimitDimension::SingleStringBytes, 4_096),
        (CompileLimitDimension::TotalStringBytes, 536_870_912),
        (CompileLimitDimension::DiagnosticCount, 16),
        (
            CompileLimitDimension::CompilerControlledLiveBytes,
            6_442_450_944,
        ),
        (CompileLimitDimension::RetainedCapacityBytes, 536_870_912),
    ];

    #[test]
    fn p100_matches_accepted_limits() {
        let limits = CompileLimits::p100();
        assert_eq!(limits.profile_id(), "LF-COMP-P100");
        for (dimension, expected) in P100_EXPECTED {
            assert!(dimension.as_str().starts_with("max_"));
            assert_eq!(limits.value(dimension), expected, "{}", dimension.as_str());
        }
    }

    #[test]
    fn single_network_1m_matches_accepted_limits() {
        let limits = CompileLimits::single_network_1m();
        assert_eq!(limits.profile_id(), "LF-COMP-SINGLE-NETWORK-1M");
        for (dimension, expected) in SINGLE_NETWORK_1M_EXPECTED {
            assert_eq!(limits.value(dimension), expected, "{}", dimension.as_str());
        }
    }

    #[test]
    fn identity_ascii_limit_never_exceeds_the_portable_format_hard_limit() {
        assert_eq!(
            CompileLimits::p100().identity_ascii_bytes_limit(),
            FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES
        );
        assert_eq!(
            CompileLimits::single_network_1m().identity_ascii_bytes_limit(),
            FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES
        );
        assert_eq!(
            CompileLimits::single_network_1m()
                .with_test_single_string_limit(17)
                .identity_ascii_bytes_limit(),
            17
        );
    }

    #[test]
    fn byte_limits_preserve_values_above_u32() {
        assert_eq!(
            CompileLimits::single_network_1m()
                .value(CompileLimitDimension::CompilerControlledLiveBytes),
            6_442_450_944
        );
    }
}
