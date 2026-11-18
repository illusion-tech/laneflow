//! 显式编译配置档；来源处理、规范容量与操作资源分别建模（ADR 0035）。
//!
//! 具名配置档组合三种政策，外部制品消费入口只接收规范容量与操作预算。字段私有，
//! 不提供无限制配置或隐式 Default；格式边界由调用方的 FormatLimits 独立执行。
use laneflow_static_contract::{
    CanonicalNetworkCapacity, CanonicalNetworkDimension, FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES,
};
#[derive(Clone, Copy)]
enum ProfileId {
    SingleNetwork10k,
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
    /// 最终规范路网登记的关系出现记录数。
    CanonicalRelationCount,
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
    /// 操作预算：一次 MIR 策略校验累计允许的查询、关系访问与扫描单位。
    PolicyValidationWorkUnits,
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
            Self::CanonicalRelationCount => "max_canonical_relation_count",
            Self::SymbolCount => "max_symbol_count",
            Self::StringItemCount => "max_string_item_count",
            Self::SingleStringBytes => "max_single_string_bytes",
            Self::TotalStringBytes => "max_total_string_bytes",
            Self::DiagnosticCount => "max_diagnostic_count",
            Self::CompilerControlledLiveBytes => "max_compiler_controlled_live_bytes",
            Self::RetainedCapacityBytes => "max_retained_capacity_bytes",
            Self::PolicyValidationWorkUnits => "max_policy_validation_work_units",
        }
    }
}

/// 来源处理限额：编制输入与编译阶段记录的处理边界。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceProcessingLimits {
    max_module_count: u32,
    max_source_document_count: u32,
    max_import_edge_count: u32,
    max_source_bytes_per_module: u64,
    max_source_bytes_total: u64,
    max_declaration_count: u32,
    max_typed_ast_record_count: u32,
    max_hir_record_count: u32,
    max_mir_record_count: u32,
    max_lir_record_count: u32,
    max_reference_count: u32,
    max_relation_occurrence_count: u32,
    max_identity_field_occurrence_count: u32,
    max_symbol_count: u32,
    max_string_item_count: u32,
    max_single_string_bytes: u64,
    max_total_string_bytes: u64,
    max_diagnostic_count: u32,
}
/// 来源处理限额的独立维度。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum SourceProcessingDimension {
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
}
impl SourceProcessingDimension {
    /// 全部登记维度，按固定顺序排列。
    pub const ALL: [Self; 18] = [
        Self::ModuleCount,
        Self::SourceDocumentCount,
        Self::ImportEdgeCount,
        Self::SourceBytesPerModule,
        Self::SourceBytesTotal,
        Self::DeclarationCount,
        Self::TypedAstRecordCount,
        Self::HirRecordCount,
        Self::MirRecordCount,
        Self::LirRecordCount,
        Self::ReferenceCount,
        Self::RelationOccurrenceCount,
        Self::IdentityFieldOccurrenceCount,
        Self::SymbolCount,
        Self::StringItemCount,
        Self::SingleStringBytes,
        Self::TotalStringBytes,
        Self::DiagnosticCount,
    ];
    /// 设计文档登记的字段名。
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
            Self::SymbolCount => "max_symbol_count",
            Self::StringItemCount => "max_string_item_count",
            Self::SingleStringBytes => "max_single_string_bytes",
            Self::TotalStringBytes => "max_total_string_bytes",
            Self::DiagnosticCount => "max_diagnostic_count",
        }
    }
}
impl SourceProcessingLimits {
    /// 查询本领域某维度的有限上限。
    #[must_use]
    pub const fn get(&self, dimension: SourceProcessingDimension) -> u64 {
        match dimension {
            SourceProcessingDimension::ModuleCount => self.max_module_count as u64,
            SourceProcessingDimension::SourceDocumentCount => self.max_source_document_count as u64,
            SourceProcessingDimension::ImportEdgeCount => self.max_import_edge_count as u64,
            SourceProcessingDimension::SourceBytesPerModule => self.max_source_bytes_per_module,
            SourceProcessingDimension::SourceBytesTotal => self.max_source_bytes_total,
            SourceProcessingDimension::DeclarationCount => self.max_declaration_count as u64,
            SourceProcessingDimension::TypedAstRecordCount => {
                self.max_typed_ast_record_count as u64
            }
            SourceProcessingDimension::HirRecordCount => self.max_hir_record_count as u64,
            SourceProcessingDimension::MirRecordCount => self.max_mir_record_count as u64,
            SourceProcessingDimension::LirRecordCount => self.max_lir_record_count as u64,
            SourceProcessingDimension::ReferenceCount => self.max_reference_count as u64,
            SourceProcessingDimension::RelationOccurrenceCount => {
                self.max_relation_occurrence_count as u64
            }
            SourceProcessingDimension::IdentityFieldOccurrenceCount => {
                self.max_identity_field_occurrence_count as u64
            }
            SourceProcessingDimension::SymbolCount => self.max_symbol_count as u64,
            SourceProcessingDimension::StringItemCount => self.max_string_item_count as u64,
            SourceProcessingDimension::SingleStringBytes => self.max_single_string_bytes,
            SourceProcessingDimension::TotalStringBytes => self.max_total_string_bytes,
            SourceProcessingDimension::DiagnosticCount => self.max_diagnostic_count as u64,
        }
    }
}
impl From<SourceProcessingDimension> for CompileLimitDimension {
    fn from(value: SourceProcessingDimension) -> Self {
        match value {
            SourceProcessingDimension::ModuleCount => Self::ModuleCount,
            SourceProcessingDimension::SourceDocumentCount => Self::SourceDocumentCount,
            SourceProcessingDimension::ImportEdgeCount => Self::ImportEdgeCount,
            SourceProcessingDimension::SourceBytesPerModule => Self::SourceBytesPerModule,
            SourceProcessingDimension::SourceBytesTotal => Self::SourceBytesTotal,
            SourceProcessingDimension::DeclarationCount => Self::DeclarationCount,
            SourceProcessingDimension::TypedAstRecordCount => Self::TypedAstRecordCount,
            SourceProcessingDimension::HirRecordCount => Self::HirRecordCount,
            SourceProcessingDimension::MirRecordCount => Self::MirRecordCount,
            SourceProcessingDimension::LirRecordCount => Self::LirRecordCount,
            SourceProcessingDimension::ReferenceCount => Self::ReferenceCount,
            SourceProcessingDimension::RelationOccurrenceCount => Self::RelationOccurrenceCount,
            SourceProcessingDimension::IdentityFieldOccurrenceCount => {
                Self::IdentityFieldOccurrenceCount
            }
            SourceProcessingDimension::SymbolCount => Self::SymbolCount,
            SourceProcessingDimension::StringItemCount => Self::StringItemCount,
            SourceProcessingDimension::SingleStringBytes => Self::SingleStringBytes,
            SourceProcessingDimension::TotalStringBytes => Self::TotalStringBytes,
            SourceProcessingDimension::DiagnosticCount => Self::DiagnosticCount,
        }
    }
}

/// 操作资源预算：一次操作的受控内存、保留容量与策略访问工作量。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationResourceBudget {
    max_compiler_controlled_live_bytes: u64,
    max_retained_capacity_bytes: u64,
    max_policy_validation_work_units: u32,
}
/// 操作资源预算的独立维度。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum OperationResourceDimension {
    /// 宿主预算：编译器控制且同时存续的受控请求字节。
    CompilerControlledLiveBytes,
    /// 宿主预算：可复用编译器实例在一次编译后允许保留的容量字节数。
    RetainedCapacityBytes,
    /// 操作预算：一次 MIR 策略校验累计允许的查询、关系访问与扫描单位。
    PolicyValidationWorkUnits,
}
impl OperationResourceDimension {
    /// 全部登记维度，按固定顺序排列。
    pub const ALL: [Self; 3] = [
        Self::CompilerControlledLiveBytes,
        Self::RetainedCapacityBytes,
        Self::PolicyValidationWorkUnits,
    ];
    /// 设计文档登记的字段名。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CompilerControlledLiveBytes => "max_compiler_controlled_live_bytes",
            Self::RetainedCapacityBytes => "max_retained_capacity_bytes",
            Self::PolicyValidationWorkUnits => "max_policy_validation_work_units",
        }
    }
}
impl OperationResourceBudget {
    /// 查询本领域某维度的有限上限。
    #[must_use]
    pub const fn get(&self, dimension: OperationResourceDimension) -> u64 {
        match dimension {
            OperationResourceDimension::CompilerControlledLiveBytes => {
                self.max_compiler_controlled_live_bytes
            }
            OperationResourceDimension::RetainedCapacityBytes => self.max_retained_capacity_bytes,
            OperationResourceDimension::PolicyValidationWorkUnits => {
                self.max_policy_validation_work_units as u64
            }
        }
    }
}
impl From<OperationResourceDimension> for CompileLimitDimension {
    fn from(value: OperationResourceDimension) -> Self {
        match value {
            OperationResourceDimension::CompilerControlledLiveBytes => {
                Self::CompilerControlledLiveBytes
            }
            OperationResourceDimension::RetainedCapacityBytes => Self::RetainedCapacityBytes,
            OperationResourceDimension::PolicyValidationWorkUnits => {
                Self::PolicyValidationWorkUnits
            }
        }
    }
}

impl From<CanonicalNetworkDimension> for CompileLimitDimension {
    fn from(value: CanonicalNetworkDimension) -> Self {
        match value {
            CanonicalNetworkDimension::StableEntityCount => Self::StableEntityCount,
            CanonicalNetworkDimension::ManeuverGateCount => Self::ManeuverGateCount,
            CanonicalNetworkDimension::WaitingZoneCount => Self::WaitingZoneCount,
            CanonicalNetworkDimension::GeometryPointCount => Self::GeometryPointCount,
            CanonicalNetworkDimension::CanonicalRelationCount => Self::CanonicalRelationCount,
            _ => unreachable!("new canonical dimension requires compiler registration"),
        }
    }
}
/// 已完成同领域比较的拒绝事实，仅供诊断组装。
pub(crate) struct ResourceLimitExceeded {
    pub(crate) dimension: CompileLimitDimension,
    pub(crate) limit: u64,
    pub(crate) observed: u64,
}

impl ResourceLimitExceeded {
    fn check(dimension: CompileLimitDimension, limit: u64, observed: u64) -> Option<Self> {
        (observed > limit).then_some(Self {
            dimension,
            limit,
            observed,
        })
    }
}

impl SourceProcessingLimits {
    pub(crate) fn exceeded(
        &self,
        dimension: SourceProcessingDimension,
        observed: u64,
    ) -> Option<ResourceLimitExceeded> {
        ResourceLimitExceeded::check(dimension.into(), self.get(dimension), observed)
    }
}

impl OperationResourceBudget {
    pub(crate) fn exceeded(
        &self,
        dimension: OperationResourceDimension,
        observed: u64,
    ) -> Option<ResourceLimitExceeded> {
        ResourceLimitExceeded::check(dimension.into(), self.get(dimension), observed)
    }
}

/// 外部规范制品消费政策；类型中不含来源处理限额。
#[derive(Clone, Copy)]
pub struct CanonicalConsumerPolicy<'a> {
    capacity: &'a CanonicalNetworkCapacity,
    operation: &'a OperationResourceBudget,
}

impl<'a> CanonicalConsumerPolicy<'a> {
    /// 组合容量与操作资源政策，不改变两者数值。
    #[must_use]
    pub const fn new(
        capacity: &'a CanonicalNetworkCapacity,
        operation: &'a OperationResourceBudget,
    ) -> Self {
        Self {
            capacity,
            operation,
        }
    }

    /// 每份 LFCA 独立执行的规范路网容量。
    #[must_use]
    pub const fn capacity(self) -> &'a CanonicalNetworkCapacity {
        self.capacity
    }

    /// 一次消费操作共用的资源预算。
    #[must_use]
    pub const fn operation_budget(self) -> &'a OperationResourceBudget {
        self.operation
    }
}

/// 由宿主显式选择的编译配置档：三份独立领域政策的不可变组合。
#[derive(Clone)]
pub struct CompileProfile {
    profile: ProfileId,
    source: SourceProcessingLimits,
    canonical: CanonicalNetworkCapacity,
    operation: OperationResourceBudget,
}
impl CompileProfile {
    /// 从具名配置档取得外部制品消费政策。
    #[must_use]
    pub const fn consumer_policy(&self) -> CanonicalConsumerPolicy<'_> {
        CanonicalConsumerPolicy::new(&self.canonical, &self.operation)
    }

    /// 来源或中间表示可证明的规范数量下界；最终 LIR 仍独立完整准入。
    pub(crate) fn early_canonical_exceeded(
        &self,
        dimension: CanonicalNetworkDimension,
        observed: u64,
    ) -> Option<ResourceLimitExceeded> {
        ResourceLimitExceeded::check(dimension.into(), self.canonical.get(dimension), observed)
    }
    /// 选择单个一万稳定实体路网的完整编译配置档。
    #[must_use]
    pub const fn single_network_10k_stable_entities() -> Self {
        Self {
            profile: ProfileId::SingleNetwork10k,
            source: SourceProcessingLimits {
                max_module_count: 18,
                max_source_document_count: 18,
                max_import_edge_count: 17,
                max_source_bytes_per_module: 6_802_717,
                max_source_bytes_total: 8_728_106,
                max_declaration_count: 10_000,
                max_typed_ast_record_count: 381_284,
                max_hir_record_count: 535_364,
                max_mir_record_count: 395_015,
                max_lir_record_count: 396_669,
                max_reference_count: 77_602,
                max_relation_occurrence_count: 105_919,
                max_identity_field_occurrence_count: 26_333,
                max_symbol_count: 10_000,
                max_string_item_count: 164_951,
                max_single_string_bytes: 1_024,
                max_total_string_bytes: 4_815_555,
                max_diagnostic_count: 16,
            },
            canonical: CanonicalNetworkCapacity::new(10_000, 1_784, 399, 71_773, 128_511),
            operation: OperationResourceBudget {
                max_compiler_controlled_live_bytes: 268_435_456,
                max_retained_capacity_bytes: 16_777_216,
                max_policy_validation_work_units: 1_000_000,
            },
        }
    }

    /// 选择单个一百万稳定实体路网的完整编译配置档。
    #[must_use]
    pub const fn single_network_1m_stable_entities() -> Self {
        Self {
            profile: ProfileId::SingleNetwork1m,
            source: SourceProcessingLimits {
                max_module_count: 65_536,
                max_source_document_count: 196_608,
                max_import_edge_count: 262_144,
                max_source_bytes_per_module: 536_870_912,
                max_source_bytes_total: 536_870_912,
                max_declaration_count: 1_500_000,
                max_typed_ast_record_count: 8_000_000,
                max_hir_record_count: 8_000_000,
                max_mir_record_count: 8_000_000,
                max_lir_record_count: 8_000_000,
                max_reference_count: 16_000_000,
                max_relation_occurrence_count: 16_000_000,
                max_identity_field_occurrence_count: 8_000_000,
                max_symbol_count: 2_000_000,
                max_string_item_count: 8_000_000,
                max_single_string_bytes: 4_096,
                max_total_string_bytes: 536_870_912,
                max_diagnostic_count: 16,
            },
            canonical: CanonicalNetworkCapacity::new(
                1_000_000, 1_000_000, 1_000_000, 1_406_986, 16_000_000,
            ),
            operation: OperationResourceBudget {
                max_compiler_controlled_live_bytes: 6_442_450_944,
                max_retained_capacity_bytes: 536_870_912,
                max_policy_validation_work_units: 16_000_000,
            },
        }
    }

    /// 来源处理限额，只用于官方前端与编译管线。
    #[must_use]
    pub const fn source_limits(&self) -> &SourceProcessingLimits {
        &self.source
    }
    /// 生产端和消费端共用的规范路网容量。
    #[must_use]
    pub const fn canonical_capacity(&self) -> &CanonicalNetworkCapacity {
        &self.canonical
    }
    /// 本配置档提供的默认操作资源预算。
    #[must_use]
    pub const fn operation_budget(&self) -> &OperationResourceBudget {
        &self.operation
    }
    /// 单个受检编制字符串的字节上限。
    #[must_use]
    pub const fn max_single_string_bytes(&self) -> u64 {
        self.source.max_single_string_bytes
    }
    pub(crate) const fn identity_ascii_bytes_limit(&self) -> u64 {
        if self.source.max_single_string_bytes < FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES {
            self.source.max_single_string_bytes
        } else {
            FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES
        }
    }
    /// 显式选择的稳定配置档标识，不写入制品字节。
    #[must_use]
    pub const fn profile_id(&self) -> &'static str {
        match self.profile {
            ProfileId::SingleNetwork10k => "LF-COMP-SINGLE-NETWORK-10K-STABLE-ENTITIES",
            ProfileId::SingleNetwork1m => "LF-COMP-SINGLE-NETWORK-1M-STABLE-ENTITIES",
        }
    }
    /// 测试按诊断维度取值；生产比较只使用各领域的类型。
    #[cfg(test)]
    pub(crate) const fn value(&self, dimension: CompileLimitDimension) -> u64 {
        match dimension {
            CompileLimitDimension::ModuleCount => {
                self.source.get(SourceProcessingDimension::ModuleCount)
            }
            CompileLimitDimension::SourceDocumentCount => self
                .source
                .get(SourceProcessingDimension::SourceDocumentCount),
            CompileLimitDimension::ImportEdgeCount => {
                self.source.get(SourceProcessingDimension::ImportEdgeCount)
            }
            CompileLimitDimension::SourceBytesPerModule => self
                .source
                .get(SourceProcessingDimension::SourceBytesPerModule),
            CompileLimitDimension::SourceBytesTotal => {
                self.source.get(SourceProcessingDimension::SourceBytesTotal)
            }
            CompileLimitDimension::DeclarationCount => {
                self.source.get(SourceProcessingDimension::DeclarationCount)
            }
            CompileLimitDimension::StableEntityCount => self
                .canonical
                .get(CanonicalNetworkDimension::StableEntityCount),
            CompileLimitDimension::TypedAstRecordCount => self
                .source
                .get(SourceProcessingDimension::TypedAstRecordCount),
            CompileLimitDimension::HirRecordCount => {
                self.source.get(SourceProcessingDimension::HirRecordCount)
            }
            CompileLimitDimension::MirRecordCount => {
                self.source.get(SourceProcessingDimension::MirRecordCount)
            }
            CompileLimitDimension::LirRecordCount => {
                self.source.get(SourceProcessingDimension::LirRecordCount)
            }
            CompileLimitDimension::ReferenceCount => {
                self.source.get(SourceProcessingDimension::ReferenceCount)
            }
            CompileLimitDimension::RelationOccurrenceCount => self
                .source
                .get(SourceProcessingDimension::RelationOccurrenceCount),
            CompileLimitDimension::IdentityFieldOccurrenceCount => self
                .source
                .get(SourceProcessingDimension::IdentityFieldOccurrenceCount),
            CompileLimitDimension::ManeuverGateCount => self
                .canonical
                .get(CanonicalNetworkDimension::ManeuverGateCount),
            CompileLimitDimension::WaitingZoneCount => self
                .canonical
                .get(CanonicalNetworkDimension::WaitingZoneCount),
            CompileLimitDimension::GeometryPointCount => self
                .canonical
                .get(CanonicalNetworkDimension::GeometryPointCount),
            CompileLimitDimension::SymbolCount => {
                self.source.get(SourceProcessingDimension::SymbolCount)
            }
            CompileLimitDimension::StringItemCount => {
                self.source.get(SourceProcessingDimension::StringItemCount)
            }
            CompileLimitDimension::SingleStringBytes => self
                .source
                .get(SourceProcessingDimension::SingleStringBytes),
            CompileLimitDimension::TotalStringBytes => {
                self.source.get(SourceProcessingDimension::TotalStringBytes)
            }
            CompileLimitDimension::DiagnosticCount => {
                self.source.get(SourceProcessingDimension::DiagnosticCount)
            }
            CompileLimitDimension::CompilerControlledLiveBytes => self
                .operation
                .get(OperationResourceDimension::CompilerControlledLiveBytes),
            CompileLimitDimension::RetainedCapacityBytes => self
                .operation
                .get(OperationResourceDimension::RetainedCapacityBytes),
            CompileLimitDimension::PolicyValidationWorkUnits => self
                .operation
                .get(OperationResourceDimension::PolicyValidationWorkUnits),
            CompileLimitDimension::CanonicalRelationCount => self
                .canonical
                .get(CanonicalNetworkDimension::CanonicalRelationCount),
        }
    }
    /// 测试辅助：覆盖总字符串字节与编译器控制存续字节上限。
    #[cfg(test)]
    pub(crate) fn with_test_string_limits(
        mut self,
        total_string_bytes: u32,
        compiler_controlled_live_bytes: u32,
    ) -> Self {
        self.source.max_total_string_bytes = total_string_bytes.into();
        self.operation.max_compiler_controlled_live_bytes = compiler_controlled_live_bytes.into();
        self
    }

    /// 测试辅助：覆盖单字符串字节上限。
    #[cfg(test)]
    pub(crate) fn with_test_single_string_limit(mut self, single_string_bytes: u32) -> Self {
        self.source.max_single_string_bytes = single_string_bytes.into();
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
        self.source.max_hir_record_count = hir_record_count;
        self.source.max_mir_record_count = mir_record_count;
        self.operation.max_compiler_controlled_live_bytes = compiler_controlled_live_bytes.into();
        self
    }

    /// 测试辅助：覆盖 LIR 记录数与存续字节上限。
    #[cfg(test)]
    pub(crate) fn with_test_lir_limits(
        mut self,
        lir_record_count: u32,
        compiler_controlled_live_bytes: u32,
    ) -> Self {
        self.source.max_lir_record_count = lir_record_count;
        self.operation.max_compiler_controlled_live_bytes = compiler_controlled_live_bytes.into();
        self
    }

    /// 测试辅助：覆盖每模块来源字节与总来源字节上限。
    #[cfg(test)]
    pub(crate) fn with_test_source_byte_limits(mut self, per_module: u32, total: u32) -> Self {
        self.source.max_source_bytes_per_module = per_module.into();
        self.source.max_source_bytes_total = total.into();
        self
    }

    /// 测试辅助：覆盖编译器跨操作保留容量的宿主预算。
    #[cfg(test)]
    pub(crate) fn with_test_retained_limit(mut self, retained_bytes: u32) -> Self {
        self.operation.max_retained_capacity_bytes = retained_bytes.into();
        self
    }

    /// 测试辅助：仅覆盖策略访问预算，不改来源关系或内存预算。
    #[cfg(test)]
    pub(crate) fn with_test_policy_work_limit(mut self, work_units: u32) -> Self {
        self.operation.max_policy_validation_work_units = work_units;
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
            CompileLimitDimension::ModuleCount => self.source.max_module_count = limit,
            CompileLimitDimension::SourceDocumentCount => {
                self.source.max_source_document_count = limit
            }
            CompileLimitDimension::ImportEdgeCount => self.source.max_import_edge_count = limit,
            CompileLimitDimension::SourceBytesTotal => {
                self.source.max_source_bytes_total = limit.into()
            }
            CompileLimitDimension::DeclarationCount => self.source.max_declaration_count = limit,
            CompileLimitDimension::StableEntityCount => {
                self = self.with_test_canonical_limit(
                    CanonicalNetworkDimension::StableEntityCount,
                    u64::from(limit),
                )
            }
            CompileLimitDimension::TypedAstRecordCount => {
                self.source.max_typed_ast_record_count = limit
            }
            CompileLimitDimension::ReferenceCount => self.source.max_reference_count = limit,
            CompileLimitDimension::RelationOccurrenceCount => {
                self.source.max_relation_occurrence_count = limit
            }
            CompileLimitDimension::IdentityFieldOccurrenceCount => {
                self.source.max_identity_field_occurrence_count = limit
            }
            CompileLimitDimension::SymbolCount => self.source.max_symbol_count = limit,
            CompileLimitDimension::StringItemCount => self.source.max_string_item_count = limit,
            CompileLimitDimension::TotalStringBytes => {
                self.source.max_total_string_bytes = limit.into()
            }
            CompileLimitDimension::ManeuverGateCount => {
                self = self.with_test_canonical_limit(
                    CanonicalNetworkDimension::ManeuverGateCount,
                    u64::from(limit),
                )
            }
            CompileLimitDimension::WaitingZoneCount => {
                self = self.with_test_canonical_limit(
                    CanonicalNetworkDimension::WaitingZoneCount,
                    u64::from(limit),
                )
            }
            CompileLimitDimension::GeometryPointCount => {
                self = self.with_test_canonical_limit(
                    CanonicalNetworkDimension::GeometryPointCount,
                    u64::from(limit),
                )
            }
            CompileLimitDimension::CompilerControlledLiveBytes => {
                self.operation.max_compiler_controlled_live_bytes = limit.into()
            }
            _ => panic!("dimension is not enforced by common official-module admission"),
        }
        self
    }
    /// 测试辅助：仅覆盖一个规范容量维度。
    #[cfg(test)]
    pub(crate) fn with_test_canonical_limit(
        mut self,
        dimension: CanonicalNetworkDimension,
        limit: u64,
    ) -> Self {
        let value = |d| {
            if dimension == d {
                limit
            } else {
                self.canonical.get(d)
            }
        };
        self.canonical = CanonicalNetworkCapacity::new(
            value(CanonicalNetworkDimension::StableEntityCount),
            value(CanonicalNetworkDimension::ManeuverGateCount),
            value(CanonicalNetworkDimension::WaitingZoneCount),
            value(CanonicalNetworkDimension::CanonicalRelationCount),
            value(CanonicalNetworkDimension::GeometryPointCount),
        );
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registered_profiles_match_the_complete_policy() {
        for (profile, id, expected) in [
            (
                CompileProfile::single_network_10k_stable_entities(),
                "LF-COMP-SINGLE-NETWORK-10K-STABLE-ENTITIES",
                [
                    (CompileLimitDimension::ModuleCount, 18),
                    (CompileLimitDimension::SourceDocumentCount, 18),
                    (CompileLimitDimension::ImportEdgeCount, 17),
                    (CompileLimitDimension::SourceBytesPerModule, 6_802_717),
                    (CompileLimitDimension::SourceBytesTotal, 8_728_106),
                    (CompileLimitDimension::DeclarationCount, 10_000),
                    (CompileLimitDimension::StableEntityCount, 10_000),
                    (CompileLimitDimension::TypedAstRecordCount, 381_284),
                    (CompileLimitDimension::HirRecordCount, 535_364),
                    (CompileLimitDimension::MirRecordCount, 395_015),
                    (CompileLimitDimension::LirRecordCount, 396_669),
                    (CompileLimitDimension::ReferenceCount, 77_602),
                    (CompileLimitDimension::RelationOccurrenceCount, 105_919),
                    (CompileLimitDimension::IdentityFieldOccurrenceCount, 26_333),
                    (CompileLimitDimension::ManeuverGateCount, 1_784),
                    (CompileLimitDimension::WaitingZoneCount, 399),
                    (CompileLimitDimension::GeometryPointCount, 128_511),
                    (CompileLimitDimension::SymbolCount, 10_000),
                    (CompileLimitDimension::StringItemCount, 164_951),
                    (CompileLimitDimension::SingleStringBytes, 1_024),
                    (CompileLimitDimension::TotalStringBytes, 4_815_555),
                    (CompileLimitDimension::DiagnosticCount, 16),
                    (
                        CompileLimitDimension::CompilerControlledLiveBytes,
                        268_435_456,
                    ),
                    (CompileLimitDimension::RetainedCapacityBytes, 16_777_216),
                    (CompileLimitDimension::PolicyValidationWorkUnits, 1_000_000),
                    (CompileLimitDimension::CanonicalRelationCount, 71_773),
                ],
            ),
            (
                CompileProfile::single_network_1m_stable_entities(),
                "LF-COMP-SINGLE-NETWORK-1M-STABLE-ENTITIES",
                [
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
                    (CompileLimitDimension::PolicyValidationWorkUnits, 16_000_000),
                    (CompileLimitDimension::CanonicalRelationCount, 1_406_986),
                ],
            ),
        ] {
            assert_eq!(profile.profile_id(), id);
            for (dimension, expected) in expected {
                assert_eq!(profile.value(dimension), expected, "{}", dimension.as_str());
            }
            assert_eq!(
                profile.identity_ascii_bytes_limit(),
                FORMAT_HARD_MAX_IDENTITY_ASCII_BYTES
            );
        }
    }
    #[test]
    fn identity_ascii_limit_keeps_the_independent_format_ceiling() {
        assert_eq!(
            CompileProfile::single_network_10k_stable_entities()
                .with_test_single_string_limit(17)
                .identity_ascii_bytes_limit(),
            17
        );
    }
}
