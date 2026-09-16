//! 线格式失败的稳定分类。

/// 可由调用方收紧、但不得超过 v1 格式天花板的资源维度。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LimitDimension {
    ObjectBytes,
    ChunksPerSection,
    TableChunkBytes,
    RowsPerChunk,
    FieldsPerRow,
    IdentityAsciiBytes,
    Utf8FieldBytes,
    TotalUtf8Bytes,
    VectorItems,
    TotalVectorBytes,
    RecordVectorDepth,
    SourceLocationRowsPerChunk,
    StagedChunkBytes,
}

/// 用于定位错误的线格式结构。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FormatStructure {
    ObjectPreamble,
    SectionDirectory,
    SectionDirectoryEntry,
    Section,
    ChunkDirectory,
    ChunkDirectoryEntry,
    Table,
    TableRows,
    Row,
    RowFields,
    Field,
    FieldValue,
    OrdinalVector,
    RecordVector,
}

/// 对外稳定的错误分类；详细数值不参与协议兼容性。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FormatErrorClass {
    InvalidLimitConfiguration,
    UnsupportedVersion,
    LimitExceeded,
    Truncated,
    LengthMismatch,
    ArithmeticOverflow,
    GapOrOverlap,
    UnknownKind,
    NonCanonicalOrder,
    NonCanonicalValue,
    DigestMismatch,
    BindingMismatch,
}

/// 结构预检错误。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FormatError {
    /// 调用方配置的某个资源维度为 0，或高于 v1 格式天花板而不做静默 clamp
    /// （`FormatLimits::try_new`）。
    InvalidLimitConfiguration {
        dimension: LimitDimension,
        requested: u64,
        hard_limit: u64,
    },
    /// 前导格式版本、section 格式版本或表 schema 版本不等于当前格式登记值
    /// （`preflight_object_framing`、`preflight_table_structure`；
    /// `preflight_object_registry`/`preflight_object_values` 内嵌两者同样可达）。
    UnsupportedVersion {
        structure: FormatStructure,
        actual: u64,
        expected: u64,
    },
    /// 某资源维度的实测值超过调用方配置上限（读取侧四个 `preflight_*` 入口；写入侧
    /// `measure_object`/`prepare_object`/`encode_object` 同样可达）。
    LimitExceeded {
        dimension: LimitDimension,
        actual: u64,
        limit: u64,
    },
    /// 声明的字节范围越过对象缓冲区末尾，或表头不足固定字节数（读取侧四个
    /// `preflight_*` 入口；registry 预检已证实的定长字段不会在
    /// `RegistryCheckedFieldView::value` 触发本变体）。
    Truncated {
        structure: FormatStructure,
        offset: u64,
        needed: u64,
        available: u64,
    },
    /// 声明的长度或计数与实际不一致，含对象长度、目录/section/chunk 跨度、行与字段边界、
    /// 字段定宽及输出缓冲区精确长度（读取侧四个 `preflight_*` 入口与
    /// `RegistryCheckedFieldView::value`；写入侧 `measure_object`/`prepare_object`/
    /// `encode_object`/`encode_prepared_object` 同样可达）。
    LengthMismatch {
        structure: FormatStructure,
        declared: u64,
        actual: u64,
    },
    /// 解析偏移换算或预算/长度累加的 checked 算术溢出（读取侧四个 `preflight_*` 入口与
    /// `RegistryCheckedFieldView::value`；写入侧 `measure_object`/`prepare_object`/
    /// `encode_object` 同样可达）。
    ArithmeticOverflow { structure: FormatStructure },
    /// 相邻 section 或 chunk 的起始偏移、首行序号不紧接前一项结束位置
    /// （`preflight_object_framing`；`preflight_object_registry`/`preflight_object_values`
    /// 内嵌同样可达）。
    GapOrOverlap {
        expected_offset: u64,
        actual_offset: u64,
    },
    /// magic、section kind、表 kind、字段 tag、字段类型码、行判别值或策略成员 kind 不在
    /// 静态 registry 登记范围内（读取侧四个 `preflight_*` 入口；写入侧
    /// `measure_object`/`prepare_object`/`encode_object` 同样可达）。
    UnknownKind {
        structure: FormatStructure,
        code: u64,
    },
    /// 要求严格递增的序列出现乱序或重复：section kind、chunk 序号、行内字段 tag 等
    /// （读取侧四个 `preflight_*` 入口；写入侧 `measure_object`/`prepare_object`/
    /// `encode_object` 同样可达）。
    NonCanonicalOrder {
        structure: FormatStructure,
        previous: u64,
        current: u64,
    },
    /// 字段或头部取值违背规范编码：保留位非零、行数为 0、浮点非规范、UTF-8/ASCII 语法
    /// 非法或 chunk 未按规范合并（读取侧四个 `preflight_*` 入口与
    /// `RegistryCheckedFieldView::value`；写入侧 `measure_object`/`prepare_object`/
    /// `encode_object` 同样可达）。
    NonCanonicalValue {
        structure: FormatStructure,
        offset: u64,
    },
    /// chunk 目录登记的 SHA-256 摘要与 chunk 实际字节重算不符（`preflight_object_registry`；
    /// `preflight_object_values` 内嵌同样可达）。
    DigestMismatch { structure: FormatStructure },
    /// 已解析结构与静态 registry 登记不一致：对象 magic、表 kind、行基数、字段类型、
    /// 嵌套行 schema、必填字段缺失或跨行键序不匹配（读取侧四个 `preflight_*` 入口与
    /// `RegistryCheckedFieldView::value`；`check_canonical_network_input` 的修订
    /// 声明行核对与写入侧 `measure_object`/`prepare_object`/`encode_object` 同样
    /// 可达）。
    BindingMismatch { structure: FormatStructure },
}

impl FormatError {
    /// 返回不随错误文本或具体数值变化的稳定分类。
    #[must_use]
    pub const fn class(self) -> FormatErrorClass {
        match self {
            Self::InvalidLimitConfiguration { .. } => FormatErrorClass::InvalidLimitConfiguration,
            Self::UnsupportedVersion { .. } => FormatErrorClass::UnsupportedVersion,
            Self::LimitExceeded { .. } => FormatErrorClass::LimitExceeded,
            Self::Truncated { .. } => FormatErrorClass::Truncated,
            Self::LengthMismatch { .. } => FormatErrorClass::LengthMismatch,
            Self::ArithmeticOverflow { .. } => FormatErrorClass::ArithmeticOverflow,
            Self::GapOrOverlap { .. } => FormatErrorClass::GapOrOverlap,
            Self::UnknownKind { .. } => FormatErrorClass::UnknownKind,
            Self::NonCanonicalOrder { .. } => FormatErrorClass::NonCanonicalOrder,
            Self::NonCanonicalValue { .. } => FormatErrorClass::NonCanonicalValue,
            Self::DigestMismatch { .. } => FormatErrorClass::DigestMismatch,
            Self::BindingMismatch { .. } => FormatErrorClass::BindingMismatch,
        }
    }
}
