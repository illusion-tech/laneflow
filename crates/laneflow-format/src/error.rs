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
    InvalidLimitConfiguration {
        dimension: LimitDimension,
        requested: u64,
        hard_limit: u64,
    },
    UnsupportedVersion {
        structure: FormatStructure,
        actual: u64,
        expected: u64,
    },
    LimitExceeded {
        dimension: LimitDimension,
        actual: u64,
        limit: u64,
    },
    Truncated {
        structure: FormatStructure,
        offset: u64,
        needed: u64,
        available: u64,
    },
    LengthMismatch {
        structure: FormatStructure,
        declared: u64,
        actual: u64,
    },
    ArithmeticOverflow {
        structure: FormatStructure,
    },
    GapOrOverlap {
        expected_offset: u64,
        actual_offset: u64,
    },
    UnknownKind {
        structure: FormatStructure,
        code: u64,
    },
    NonCanonicalOrder {
        structure: FormatStructure,
        previous: u64,
        current: u64,
    },
    NonCanonicalValue {
        structure: FormatStructure,
        offset: u64,
    },
    DigestMismatch {
        structure: FormatStructure,
    },
    BindingMismatch {
        structure: FormatStructure,
    },
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
