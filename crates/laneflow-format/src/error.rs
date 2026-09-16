//! 线格式失败的稳定分类。

/// 可由调用方收紧、但不得超过 v1 格式天花板的资源维度。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LimitDimension {
    /// 单个对象的总字节长度。
    ObjectBytes,
    /// 单个 section 内的物理 chunk 数。
    ChunksPerSection,
    /// 单个表 chunk 的总字节长度。
    TableChunkBytes,
    /// 单个表 chunk 内的 row 数。
    RowsPerChunk,
    /// 单行 row 内的字段数。
    FieldsPerRow,
    /// Identity v1 ASCII 值的字节长度。
    IdentityAsciiBytes,
    /// 单个 UTF-8 字段值的字节长度。
    Utf8FieldBytes,
    /// 单个 chunk 内全部 UTF-8 字段值的累计字节长度（预算按 chunk 重建）。
    TotalUtf8Bytes,
    /// 单个向量值的元素个数。
    VectorItems,
    /// 单个 chunk 内全部向量值的累计字节数（预算按 chunk 重建）。
    TotalVectorBytes,
    /// record 向量值的嵌套行深度。
    RecordVectorDepth,
    /// 单个 LFSM SourceLocation chunk 内的 row 数。
    SourceLocationRowsPerChunk,
    /// 三个对象同时暂存当前 chunk 的内存字节量。
    StagedChunkBytes,
}

/// 用于定位错误的线格式结构。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FormatStructure {
    /// 对象前导头部。
    ObjectPreamble,
    /// 对象的 section 目录。
    SectionDirectory,
    /// section 目录中的单个条目。
    SectionDirectoryEntry,
    /// 单个 section。
    Section,
    /// section 内的表 chunk 目录。
    ChunkDirectory,
    /// 表 chunk 目录中的单个条目。
    ChunkDirectoryEntry,
    /// 单张表及其表头。
    Table,
    /// 表的行集合（行数与行区字节长度）。
    TableRows,
    /// 单行 row 及其行头。
    Row,
    /// 行内字段集合（字段数、tag 序与行形状必填约束）。
    RowFields,
    /// 单个字段及其字段头。
    Field,
    /// 字段值的取值与规范编码。
    FieldValue,
    /// `OrdinalVectorU32` 字段值的向量结构。
    OrdinalVector,
    /// `RecordVector` 字段值的嵌套行结构。
    RecordVector,
}

/// 对外稳定的错误分类；详细数值不参与协议兼容性。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FormatErrorClass {
    /// 调用方限制配置为 0 或高于格式天花板。
    InvalidLimitConfiguration,
    /// 读到的格式版本不等于当前登记值。
    UnsupportedVersion,
    /// 实测值超过调用方配置的资源上限。
    LimitExceeded,
    /// 声明的字节范围越过缓冲区末尾。
    Truncated,
    /// 声明的长度或计数与实际不一致。
    LengthMismatch,
    /// 解析或预算计算的 checked 算术溢出。
    ArithmeticOverflow,
    /// 相邻 section 或 chunk 的偏移/序号不连续。
    GapOrOverlap,
    /// kind 或类型码不在静态 registry 登记范围内。
    UnknownKind,
    /// 要求严格递增的序列乱序或重复。
    NonCanonicalOrder,
    /// 取值违背规范编码。
    NonCanonicalValue,
    /// chunk 摘要与实际字节重算不符。
    DigestMismatch,
    /// 解析结构与静态 registry 登记不一致；跨对象绑定不匹配经
    /// `PostEmissionCheckError` 专属变体报告，不属本类。
    BindingMismatch,
}

/// 结构预检错误。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FormatError {
    /// 调用方配置的资源维度为 0（仅对象字节、每段 chunk、每 chunk 行与来源位置
    /// 行四个维度禁止为 0）或高于 v1 格式天花板而不做静默 clamp
    /// （`FormatLimits::try_new`）。
    InvalidLimitConfiguration {
        /// 被拒绝的资源维度。
        dimension: LimitDimension,
        /// 调用方请求的配置值。
        requested: u64,
        /// 该维度的上限；对 `ObjectBytes`/`ChunksPerSection` 这是调用方预算
        /// （配置默认 `u64::MAX`/`u32::MAX`），其余维度为 v1 格式硬天花板。
        hard_limit: u64,
    },
    /// 前导格式版本、section 格式版本或表 schema 版本不等于当前格式登记值
    /// （`preflight_object_framing`、`preflight_table_structure`；
    /// `preflight_object_registry`/`preflight_object_values` 内嵌两者同样可达）。
    UnsupportedVersion {
        /// 版本不符的结构位置。
        structure: FormatStructure,
        /// 实际读到的版本号。
        actual: u64,
        /// 当前格式登记的版本号。
        expected: u64,
    },
    /// 某资源维度的实测值超过调用方配置上限（读取侧四个 `preflight_*` 入口与
    /// `RegistryCheckedObjectView::check_value_domains`；写入侧
    /// `measure_object`/`prepare_object`/`encode_object` 同样可达）。
    LimitExceeded {
        /// 超限的资源维度。
        dimension: LimitDimension,
        /// 实测值。
        actual: u64,
        /// 调用方配置的上限。
        limit: u64,
    },
    /// 声明的字节范围越过对象缓冲区末尾，或表头不足固定字节数（读取侧四个
    /// `preflight_*` 入口；registry 预检与 `check_value_domains` 的重解析中
    /// 同类构造点均为预检已证实的防御性死分支）。
    Truncated {
        /// 发生截断的结构位置。
        structure: FormatStructure,
        /// 请求读取的起始偏移（字节）。
        offset: u64,
        /// 完成本次读取所需的字节数。
        needed: u64,
        /// 从该偏移起实际可用的字节数。
        available: u64,
    },
    /// 声明的长度或计数与实际不一致，含对象长度、目录/section/chunk 跨度、行与字段边界、
    /// 字段定宽及输出缓冲区精确长度（读取侧四个 `preflight_*` 入口与
    /// `RegistryCheckedObjectView::check_value_domains`；写入侧
    /// `measure_object`/`prepare_object`/`encode_object`/`encode_prepared_object`。
    /// registry 预检已证实的字段不会在 `RegistryCheckedFieldView::value` 触发本变体）。
    LengthMismatch {
        /// 长度不符的结构位置。
        structure: FormatStructure,
        /// 声明的长度或计数。
        declared: u64,
        /// 实际的长度或计数；写入侧例外——`encode_prepared_object` 把调用方
        /// 提供的缓冲区长度存入 `declared`、预检要求的精确长度存入 `actual`。
        actual: u64,
    },
    /// 解析偏移换算或预算/长度累加的 checked 算术溢出（读取侧四个 `preflight_*` 入口；
    /// 预检证实的字段在 `RegistryCheckedFieldView::value` 的零偏移定宽解码与
    /// `check_value_domains` 的重复解析中均不会溢出；写入侧
    /// `measure_object`/`prepare_object`/`encode_object` 同样可达）。
    ArithmeticOverflow {
        /// 溢出发生的结构位置。
        structure: FormatStructure,
    },
    /// 相邻 section 或 chunk 的起始偏移、首行序号不紧接前一项结束位置
    /// （`preflight_object_framing`；`preflight_object_registry`/`preflight_object_values`
    /// 内嵌同样可达）。
    GapOrOverlap {
        /// 按前一项推算的期望位置；chunk 目录不连续时为行序号而非字节偏移。
        expected_offset: u64,
        /// 实际读到的位置；同上，可能为行序号。
        actual_offset: u64,
    },
    /// magic、section kind、表 kind、字段 tag、字段类型码、行判别值、策略成员 kind
    /// 或语义封闭枚举码（实体 kind、Identity tag、来源定位/语言/角色、property step
    /// kind）不在静态 registry 登记范围内（读取侧四个 `preflight_*` 入口与
    /// `RegistryCheckedObjectView::check_value_domains`；写入侧
    /// `measure_object`/`prepare_object`/`encode_object` 同样可达）。
    UnknownKind {
        /// 出现未登记码的结构位置。
        structure: FormatStructure,
        /// 读到的未登记 kind 码。
        code: u64,
    },
    /// 要求严格递增的序列出现乱序或重复，或值虽递增但与 registry 登记值不符
    /// （如首 section kind 跳号）：section kind、chunk 序号、行内字段 tag 等
    /// （读取侧四个 `preflight_*` 入口；写入侧 `measure_object`/`prepare_object`/
    /// `encode_object` 同样可达）。
    NonCanonicalOrder {
        /// 乱序发生的结构位置。
        structure: FormatStructure,
        /// 序列中前一项的值；section kind / chunk 序号失配时为登记的期望值
        /// （并无真实前驱项）。
        previous: u64,
        /// 读到的当前项的值；同上，失配时为实际值。
        current: u64,
    },
    /// 字段或头部取值违背规范编码：保留位非零、行数为 0、浮点非规范、UTF-8/ASCII 语法
    /// 非法、chunk 未按规范合并或数值区间与版本/绑定常量精确值核对失败（读取侧四个
    /// `preflight_*` 入口与 `RegistryCheckedObjectView::check_value_domains`——后者
    /// 覆盖对象种类专用的语义域；registry 预检已证实的 UTF-8 不会在
    /// `RegistryCheckedFieldView::value` 重解码触发；写入侧 `measure_object`/
    /// `prepare_object`/`encode_object` 同样可达）。
    NonCanonicalValue {
        /// 违背规范编码的结构位置；解析 section 或 table chunk 时为该切片内
        /// 局部偏移，非对象全局偏移。
        structure: FormatStructure,
        /// 违规取值的偏移；解析 section 或 table chunk 时为该切片内局部偏移，
        /// 写入侧错误无定位信息时为 0。
        offset: u64,
    },
    /// chunk 目录登记的 SHA-256 摘要与 chunk 实际字节重算不符（`preflight_object_registry`；
    /// `preflight_object_values` 内嵌同样可达）。
    DigestMismatch {
        /// 摘要不符的 chunk 结构位置。
        structure: FormatStructure,
    },
    /// 已解析结构与静态 registry 登记不一致：对象 magic、表 kind、行基数、字段类型、
    /// 嵌套行 schema、必填字段缺失或跨行键序不匹配，以及对象种类专用的同对象直接
    /// 绑定（同行存在性矩阵、跨行一致性闭环、Identity tag 序列、LFSD base-kind 行数
    /// 约束、LFCP 对象键摘要绑定）（读取侧四个 `preflight_*` 入口与
    /// `RegistryCheckedObjectView::check_value_domains`；registry 预检对
    /// record-vector 字段 schema 的构造保证使 `RegistryCheckedFieldView::value`
    /// 的嵌套行缺失分支不可达；`check_canonical_network_input` 的修订
    /// 声明行核对、`check_post_emission_bundle` 的 provenance/LFSM/LFSD 绑定行核对
    /// 与写入侧 `measure_object`/`prepare_object`/`encode_object` 同样可达）。
    BindingMismatch {
        /// 与登记不一致的结构位置。
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
