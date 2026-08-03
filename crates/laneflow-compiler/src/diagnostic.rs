//! 与渲染文本解耦的结构化编译诊断。
//!
//! 规范判断依赖 [`DiagnosticCode`]、严重程度、有类型载荷和来源位置，而不是
//! [`Display`](core::fmt::Display) 生成的中文句子。诊断按完整结构值排序；收集器即使
//! 达到保留上限也继续检查安全候选，并最终保留全局规范顺序最小的前缀，避免遍历顺序
//! 改变对外可见结果。任一错误阶段只返回 [`DiagnosticBundle`]，不携带部分阶段输出。

use core::fmt;
use std::sync::Arc;

use laneflow_static_contract::{EntityKind, StableId128};

use crate::CompileLimitDimension;
use crate::declaration::ScalarViolation;
use crate::identity::CanonicalIdentityViolation;

/// 来源文档内受检的一基行列位置。
///
/// 合成前端取 Rust 调用位置，后续文本前端可提供真实文本位置。零值不会由当前公共
/// 构造路径产生，但本值本身不负责重新验证范围。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourcePosition {
    line: u32,
    column: u32,
}

impl SourcePosition {
    /// 返回一基行号。
    #[must_use]
    pub const fn line(self) -> u32 {
        self.line
    }

    /// 返回一基列号。
    #[must_use]
    pub const fn column(self) -> u32 {
        self.column
    }
}

/// 与机器路径无关的来源范围。
///
/// `source_document_key` 是来源模块提供的稳定键，不是宿主文件系统路径；范围的起止
/// 位置都采用一基 `u32` 行列。位置服务诊断与源映射，不参与实体身份。
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceSpan {
    source_document_key: Arc<str>,
    start: SourcePosition,
    end: SourcePosition,
}

impl SourceSpan {
    /// 为包内官方前端建立单点范围；调用者负责传入已验证的文档键与一基位置。
    pub(crate) fn point(source_document_key: Arc<str>, line: u32, column: u32) -> Self {
        let position = SourcePosition { line, column };
        Self {
            source_document_key,
            start: position,
            end: position,
        }
    }

    /// 把合成 DSL 的 Rust 调用点转换为与机器路径无关的来源单点。
    pub(crate) fn at_caller(
        source_document_key: Arc<str>,
        caller: &'static std::panic::Location<'static>,
    ) -> Self {
        Self::point(source_document_key, caller.line(), caller.column())
    }

    /// 返回来源模块内稳定的文档键。
    #[must_use]
    pub fn source_document_key(&self) -> &str {
        &self.source_document_key
    }

    /// 返回包含范围的起始位置。
    #[must_use]
    pub const fn start(&self) -> SourcePosition {
        self.start
    }

    /// 返回包含范围的结束位置；单点范围与 `start` 相同。
    #[must_use]
    pub const fn end(&self) -> SourcePosition {
        self.end
    }
}

/// 稳定诊断代码。
///
/// 对外稳定标识是 [`DiagnosticCode::as_str`] 返回的字符串，不是 Rust 枚举判别值。
/// 枚举为 `non_exhaustive`，调用方必须允许后续版本新增代码。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum DiagnosticCode {
    /// 来源模块头字段违反文本或资源约束。
    InvalidSourceHeaderField,
    /// 导入命名空间不是合法外部 token。
    InvalidImportNamespace,
    /// 同一来源模块重复声明相同导入。
    DuplicateImport,
    /// 编译单元包含两个相同 authoring namespace 的模块。
    DuplicateModuleNamespace,
    /// 编译单元包含两个声明相同 `sourceDocumentKey` 的模块。
    DuplicateSourceDocumentKey,
    /// 显式导入在完整编译单元中没有目标模块。
    UnknownImport,
    /// 一个或多个显式导入边形成循环。
    ImportCycle,
    /// 声明稳定键不是合法外部 token。
    InvalidDeclarationKey,
    /// 同一模块、同一实体种类重复声明稳定键。
    DuplicateDeclaration,
    /// 引用中显式模块命名空间不是合法外部 token。
    InvalidReferenceNamespace,
    /// 引用的目标声明键不是合法外部 token。
    InvalidReferenceKey,
    /// 跨模块引用没有对应的显式导入。
    UnimportedReferenceModule,
    /// 导入闭合后仍找不到引用的目标声明。
    UnknownReferenceTarget,
    /// 车道图边长度不是满足当前契约的有限 `f64` 米值。
    InvalidLaneEdgeLength,
    /// 基础道路限速不是严格为正的有限 `f64` 米每秒值。
    InvalidLaneEdgeSpeedLimit,
    /// 同一车道图边重复列出相同下游目标。
    DuplicateLaneEdgeSuccessor,
    /// 编译器构造的规范身份字段不满足 Identity v1 登记表。
    InvalidCanonicalIdentity,
    /// 同一完整规范身份在编译单元中出现多次。
    DuplicateCanonicalIdentity,
    /// 不同完整规范身份派生出相同 StableId128。
    IdentityDigestCollision,
    /// 候选输入或阶段工作集超过显式编译资源配置档。
    CompileLimitExceeded,
}

impl DiagnosticCode {
    /// 返回跨渲染语言稳定的代码字符串。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidSourceHeaderField => "LF-COMP-SOURCE-HEADER-FIELD",
            Self::InvalidImportNamespace => "LF-COMP-IMPORT-NAMESPACE",
            Self::DuplicateImport => "LF-COMP-DUPLICATE-IMPORT",
            Self::DuplicateModuleNamespace => "LF-COMP-DUPLICATE-MODULE-NAMESPACE",
            Self::DuplicateSourceDocumentKey => "LF-COMP-DUPLICATE-SOURCE-DOCUMENT-KEY",
            Self::UnknownImport => "LF-COMP-UNKNOWN-IMPORT",
            Self::ImportCycle => "LF-COMP-IMPORT-CYCLE",
            Self::InvalidDeclarationKey => "LF-COMP-DECLARATION-KEY",
            Self::DuplicateDeclaration => "LF-COMP-DUPLICATE-DECLARATION",
            Self::InvalidReferenceNamespace => "LF-COMP-REFERENCE-NAMESPACE",
            Self::InvalidReferenceKey => "LF-COMP-REFERENCE-KEY",
            Self::UnimportedReferenceModule => "LF-COMP-UNIMPORTED-REFERENCE-MODULE",
            Self::UnknownReferenceTarget => "LF-COMP-UNKNOWN-REFERENCE-TARGET",
            Self::InvalidLaneEdgeLength => "LF-COMP-LANE-EDGE-LENGTH",
            Self::InvalidLaneEdgeSpeedLimit => "LF-COMP-LANE-EDGE-SPEED-LIMIT",
            Self::DuplicateLaneEdgeSuccessor => "LF-COMP-DUPLICATE-LANE-EDGE-SUCCESSOR",
            Self::InvalidCanonicalIdentity => "LF-COMP-INVALID-CANONICAL-IDENTITY",
            Self::DuplicateCanonicalIdentity => "LF-COMP-DUPLICATE-CANONICAL-IDENTITY",
            Self::IdentityDigestCollision => "LF-COMP-IDENTITY-DIGEST-COLLISION",
            Self::CompileLimitExceeded => "LF-COMP-RESOURCE-LIMIT",
        }
    }
}

/// 诊断严重程度。数值顺序同时是规范排序顺序。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
#[non_exhaustive]
pub enum DiagnosticSeverity {
    /// 当前阶段不能提交输出。
    Error = 1,
    /// 输出仍可提交，但调用方应向作者展示的问题。
    Warning = 2,
    /// 不改变成功与否的补充说明。
    Note = 3,
}

/// 来源模块头中由调用方提供的字段。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum SourceHeaderField {
    /// 声明身份使用的 authoring namespace。
    AuthoringNamespaceId,
    /// 与机器路径无关的来源文档键。
    SourceDocumentKey,
    /// 生成器构建标识。
    GeneratorBuildId,
    /// 来源沿袭展示文本。
    Provenance,
}

impl SourceHeaderField {
    /// 返回诊断载荷使用的稳定 lowerCamelCase 字段名。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AuthoringNamespaceId => "authoringNamespaceId",
            Self::SourceDocumentKey => "sourceDocumentKey",
            Self::GeneratorBuildId => "generatorBuildId",
            Self::Provenance => "provenance",
        }
    }
}

/// 来源文本字段的有类型失败原因。
///
/// 所有位置与长度都按 UTF-8 原始字节计；ASCII 校验失败后不会把字符索引误报为字节
/// 索引。枚举保持结构化数据，显示文本不是机器契约。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum SourceTextViolation {
    /// 必填字段为空。
    Empty,
    /// UTF-8 字节数超过所选资源配置档的单字符串上限。
    TooLong { limit: u64, observed: u64 },
    /// 指定零基字节位置不是 ASCII。
    NonAscii { byte_index: u64 },
    /// token 首字节不是 ASCII 字母或数字。
    InvalidFirstByte { byte: u8 },
    /// token 在指定零基位置包含不在允许集合内的 ASCII 字节。
    InvalidTokenByte { byte_index: u64, byte: u8 },
    /// 可见文本包含控制字节；空格不属于此错误。
    ControlByte { byte_index: u64, byte: u8 },
}

/// 诊断的有类型载荷。
///
/// 载荷保留复现和机器判断所需的原始结构，例如计数维度、目标命名空间与浮点位模式；
/// 调用方不应解析 [`Diagnostic`] 的显示字符串来恢复这些信息。
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum DiagnosticPayload {
    /// 模块头字段及其文本失败原因。
    InvalidSourceHeaderField {
        field: SourceHeaderField,
        violation: SourceTextViolation,
    },
    /// 超限维度、配置值与候选观测值。
    CompileLimitExceeded {
        dimension: CompileLimitDimension,
        limit: u64,
        observed: u64,
    },
    /// 导入命名空间的文本失败原因。
    InvalidImportNamespace {
        /// 命名空间违反的精确文本规则。
        violation: SourceTextViolation,
    },
    /// 重复导入的规范命名空间。
    DuplicateImport {
        /// 在同一来源模块内第二次出现的命名空间。
        namespace: Box<str>,
    },
    /// 重复来源模块的 authoring namespace。
    DuplicateModuleNamespace {
        /// 在编译单元内发生冲突的 authoring namespace。
        namespace: Box<str>,
    },
    /// 在编译单元内不能唯一定位来源位置的重复文档键。
    DuplicateSourceDocumentKey {
        /// 两个来源模块共同声明的 `sourceDocumentKey`。
        source_document_key: Box<str>,
    },
    /// 在编译单元中没有目标模块的导入命名空间。
    UnknownImport {
        /// 没有对应来源模块的目标命名空间。
        namespace: Box<str>,
    },
    /// 一条规范选择的导入循环；顺序用于稳定展示见证，不代表全部可能回路。
    ImportCycle {
        /// 按规范选择的循环模块序列；首项不会在末尾重复。
        namespaces: Box<[Box<str>]>,
    },
    /// 非法声明稳定键所属实体种类及失败原因。
    InvalidDeclarationKey {
        entity_kind: EntityKind,
        violation: SourceTextViolation,
    },
    /// 模块内发生冲突的实体种类与稳定键。
    DuplicateDeclaration {
        entity_kind: EntityKind,
        stable_key: Box<str>,
    },
    /// 引用模块命名空间的文本失败原因。
    InvalidReferenceNamespace {
        /// 显式目标命名空间违反的精确文本规则。
        violation: SourceTextViolation,
    },
    /// 非法目标键所属实体种类及失败原因。
    InvalidReferenceKey {
        entity_kind: EntityKind,
        violation: SourceTextViolation,
    },
    /// 未经显式导入就被引用的模块命名空间。
    UnimportedReferenceModule {
        /// 被引用但未出现在当前模块导入集合中的命名空间。
        namespace: Box<str>,
    },
    /// 来源声明及无法解析的完整目标二元组。
    UnknownReferenceTarget {
        entity_kind: EntityKind,
        source_key: Box<str>,
        target_namespace: Box<str>,
        target_key: Box<str>,
    },
    /// 车道图边稳定键、非法长度位模式及数值约束原因。
    InvalidLaneEdgeLength {
        stable_key: Box<str>,
        /// 非法 `f64` 的原始 IEEE 754 位模式，避免 NaN 与格式化差异破坏确定性。
        value_bits: u64,
        violation: ScalarViolation,
    },
    /// 车道图边稳定键、非法限速位模式及数值约束原因。
    InvalidLaneEdgeSpeedLimit {
        stable_key: Box<str>,
        /// 非法 `f64` 的原始 IEEE 754 位模式。
        value_bits: u64,
        violation: ScalarViolation,
    },
    /// 来源车道图边与重复的完整目标二元组。
    DuplicateLaneEdgeSuccessor {
        stable_key: Box<str>,
        target_namespace: Box<str>,
        target_key: Box<str>,
    },
    /// 实体种类、来源稳定键及不能形成 Identity v1 前像的精确原因。
    InvalidCanonicalIdentity {
        entity_kind: EntityKind,
        stable_key: Box<str>,
        violation: CanonicalIdentityViolation,
    },
    /// 重复完整身份的实体种类和已派生摘要。
    DuplicateCanonicalIdentity {
        entity_kind: EntityKind,
        stable_id: StableId128,
    },
    /// 发生 BLAKE3-128 摘要碰撞的实体种类和冲突摘要。
    IdentityDigestCollision {
        entity_kind: EntityKind,
        stable_id: StableId128,
    },
}

/// 一条不可变结构化诊断。
///
/// 排序同时考虑规范模块顺序、来源位置、代码、严重程度、载荷、稳定键和关联位置。
/// `primary_span` 指向主要失败位置，`related_spans` 用于重复声明、跨模块引用等需要同时
/// 展示上下文的情况。
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Diagnostic {
    canonical_module_order: u32,
    primary_span: Option<SourceSpan>,
    code: DiagnosticCode,
    severity: DiagnosticSeverity,
    payload: DiagnosticPayload,
    stable_key: Option<Box<str>>,
    related_spans: Box<[SourceSpan]>,
}

impl Diagnostic {
    pub(crate) fn invalid_source_header_field(
        field: SourceHeaderField,
        violation: SourceTextViolation,
    ) -> Self {
        Self {
            canonical_module_order: 0,
            primary_span: None,
            code: DiagnosticCode::InvalidSourceHeaderField,
            severity: DiagnosticSeverity::Error,
            payload: DiagnosticPayload::InvalidSourceHeaderField { field, violation },
            stable_key: None,
            related_spans: Box::default(),
        }
    }

    pub(crate) fn compile_limit_exceeded(
        dimension: CompileLimitDimension,
        limit: u64,
        observed: u64,
    ) -> Self {
        Self {
            canonical_module_order: 0,
            primary_span: None,
            code: DiagnosticCode::CompileLimitExceeded,
            severity: DiagnosticSeverity::Error,
            payload: DiagnosticPayload::CompileLimitExceeded {
                dimension,
                limit,
                observed,
            },
            stable_key: None,
            related_spans: Box::default(),
        }
    }

    pub(crate) fn invalid_import_namespace(
        violation: SourceTextViolation,
        primary_span: SourceSpan,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidImportNamespace,
            DiagnosticPayload::InvalidImportNamespace { violation },
            Some(primary_span),
            Box::default(),
            None,
        )
    }

    pub(crate) fn duplicate_import(
        namespace: &str,
        primary_span: SourceSpan,
        related_span: SourceSpan,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::DuplicateImport,
            DiagnosticPayload::DuplicateImport {
                namespace: namespace.into(),
            },
            Some(primary_span),
            Box::new([related_span]),
            Some(namespace.into()),
        )
    }

    pub(crate) fn duplicate_module_namespace(
        namespace: &str,
        primary_span: SourceSpan,
        related_span: SourceSpan,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::DuplicateModuleNamespace,
            DiagnosticPayload::DuplicateModuleNamespace {
                namespace: namespace.into(),
            },
            Some(primary_span),
            Box::new([related_span]),
            Some(namespace.into()),
        )
    }

    pub(crate) fn duplicate_source_document_key(
        source_document_key: &str,
        primary_span: SourceSpan,
        related_span: SourceSpan,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::DuplicateSourceDocumentKey,
            DiagnosticPayload::DuplicateSourceDocumentKey {
                source_document_key: source_document_key.into(),
            },
            Some(primary_span),
            Box::new([related_span]),
            Some(source_document_key.into()),
        )
    }

    pub(crate) fn unknown_import(namespace: &str, primary_span: SourceSpan) -> Self {
        Self::error_with_context(
            DiagnosticCode::UnknownImport,
            DiagnosticPayload::UnknownImport {
                namespace: namespace.into(),
            },
            Some(primary_span),
            Box::default(),
            Some(namespace.into()),
        )
    }

    pub(crate) fn import_cycle(namespaces: &[&str], spans: Box<[SourceSpan]>) -> Self {
        let stable_key = namespaces.first().copied().map(Into::into);
        let mut spans = spans.into_vec();
        let primary_span = if spans.is_empty() {
            None
        } else {
            Some(spans.remove(0))
        };
        Self::error_with_context(
            DiagnosticCode::ImportCycle,
            DiagnosticPayload::ImportCycle {
                namespaces: namespaces
                    .iter()
                    .map(|namespace| (*namespace).into())
                    .collect(),
            },
            primary_span,
            spans.into_boxed_slice(),
            stable_key,
        )
    }

    pub(crate) fn invalid_declaration_key(
        entity_kind: EntityKind,
        violation: SourceTextViolation,
        primary_span: SourceSpan,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidDeclarationKey,
            DiagnosticPayload::InvalidDeclarationKey {
                entity_kind,
                violation,
            },
            Some(primary_span),
            Box::default(),
            None,
        )
    }

    pub(crate) fn duplicate_declaration(
        entity_kind: EntityKind,
        stable_key: &str,
        primary_span: SourceSpan,
        related_span: SourceSpan,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::DuplicateDeclaration,
            DiagnosticPayload::DuplicateDeclaration {
                entity_kind,
                stable_key: stable_key.into(),
            },
            Some(primary_span),
            Box::new([related_span]),
            Some(stable_key.into()),
        )
    }

    pub(crate) fn invalid_reference_key(
        entity_kind: EntityKind,
        violation: SourceTextViolation,
        primary_span: SourceSpan,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidReferenceKey,
            DiagnosticPayload::InvalidReferenceKey {
                entity_kind,
                violation,
            },
            Some(primary_span),
            Box::default(),
            None,
        )
    }

    pub(crate) fn invalid_reference_namespace(
        violation: SourceTextViolation,
        primary_span: SourceSpan,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidReferenceNamespace,
            DiagnosticPayload::InvalidReferenceNamespace { violation },
            Some(primary_span),
            Box::default(),
            None,
        )
    }

    pub(crate) fn unimported_reference_module(namespace: &str, primary_span: SourceSpan) -> Self {
        Self::error_with_context(
            DiagnosticCode::UnimportedReferenceModule,
            DiagnosticPayload::UnimportedReferenceModule {
                namespace: namespace.into(),
            },
            Some(primary_span),
            Box::default(),
            Some(namespace.into()),
        )
    }

    pub(crate) fn unknown_reference_target(
        entity_kind: EntityKind,
        source_key: &str,
        target_namespace: &str,
        target_key: &str,
        primary_span: SourceSpan,
        source_declaration_span: SourceSpan,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::UnknownReferenceTarget,
            DiagnosticPayload::UnknownReferenceTarget {
                entity_kind,
                source_key: source_key.into(),
                target_namespace: target_namespace.into(),
                target_key: target_key.into(),
            },
            Some(primary_span),
            Box::new([source_declaration_span]),
            Some(source_key.into()),
        )
    }

    pub(crate) fn invalid_lane_edge_length(
        stable_key: &str,
        value: f64,
        violation: ScalarViolation,
        primary_span: SourceSpan,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidLaneEdgeLength,
            DiagnosticPayload::InvalidLaneEdgeLength {
                stable_key: stable_key.into(),
                value_bits: value.to_bits(),
                violation,
            },
            Some(primary_span),
            Box::default(),
            Some(stable_key.into()),
        )
    }

    pub(crate) fn invalid_lane_edge_speed_limit(
        stable_key: &str,
        value: f64,
        violation: ScalarViolation,
        primary_span: SourceSpan,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidLaneEdgeSpeedLimit,
            DiagnosticPayload::InvalidLaneEdgeSpeedLimit {
                stable_key: stable_key.into(),
                value_bits: value.to_bits(),
                violation,
            },
            Some(primary_span),
            Box::default(),
            Some(stable_key.into()),
        )
    }

    pub(crate) fn duplicate_lane_edge_successor(
        stable_key: &str,
        target_namespace: &str,
        target_key: &str,
        primary_span: SourceSpan,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::DuplicateLaneEdgeSuccessor,
            DiagnosticPayload::DuplicateLaneEdgeSuccessor {
                stable_key: stable_key.into(),
                target_namespace: target_namespace.into(),
                target_key: target_key.into(),
            },
            Some(primary_span),
            Box::default(),
            Some(stable_key.into()),
        )
    }

    pub(crate) fn invalid_canonical_identity(
        entity_kind: EntityKind,
        stable_key: &str,
        violation: CanonicalIdentityViolation,
        primary_span: SourceSpan,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::InvalidCanonicalIdentity,
            DiagnosticPayload::InvalidCanonicalIdentity {
                entity_kind,
                stable_key: stable_key.into(),
                violation,
            },
            Some(primary_span),
            Box::default(),
            Some(stable_key.into()),
        )
    }

    pub(crate) fn duplicate_canonical_identity(
        entity_kind: EntityKind,
        stable_key: &str,
        stable_id: StableId128,
        primary_span: SourceSpan,
        existing_span: SourceSpan,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::DuplicateCanonicalIdentity,
            DiagnosticPayload::DuplicateCanonicalIdentity {
                entity_kind,
                stable_id,
            },
            Some(primary_span),
            Box::new([existing_span]),
            Some(stable_key.into()),
        )
    }

    pub(crate) fn identity_digest_collision(
        entity_kind: EntityKind,
        stable_key: &str,
        stable_id: StableId128,
        primary_span: SourceSpan,
        existing_span: SourceSpan,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::IdentityDigestCollision,
            DiagnosticPayload::IdentityDigestCollision {
                entity_kind,
                stable_id,
            },
            Some(primary_span),
            Box::new([existing_span]),
            Some(stable_key.into()),
        )
    }

    pub(crate) const fn set_canonical_module_order(&mut self, order: u32) {
        self.canonical_module_order = order;
    }

    pub(crate) fn compile_limit_exceeded_at(
        dimension: CompileLimitDimension,
        limit: u64,
        observed: u64,
        primary_span: Option<SourceSpan>,
        stable_key: Option<Box<str>>,
    ) -> Self {
        Self::error_with_context(
            DiagnosticCode::CompileLimitExceeded,
            DiagnosticPayload::CompileLimitExceeded {
                dimension,
                limit,
                observed,
            },
            primary_span,
            Box::default(),
            stable_key,
        )
    }

    fn error_with_context(
        code: DiagnosticCode,
        payload: DiagnosticPayload,
        primary_span: Option<SourceSpan>,
        related_spans: Box<[SourceSpan]>,
        stable_key: Option<Box<str>>,
    ) -> Self {
        Self {
            canonical_module_order: 0,
            primary_span,
            code,
            severity: DiagnosticSeverity::Error,
            payload,
            stable_key,
            related_spans,
        }
    }

    /// 返回跨语言渲染稳定的诊断代码。
    #[must_use]
    pub const fn code(&self) -> DiagnosticCode {
        self.code
    }

    /// 返回该诊断对阶段提交的影响级别。
    #[must_use]
    pub const fn severity(&self) -> DiagnosticSeverity {
        self.severity
    }

    /// 返回机器可消费的有类型载荷。
    #[must_use]
    pub const fn payload(&self) -> &DiagnosticPayload {
        &self.payload
    }

    /// 返回主要来源位置；资源或头级错误可能没有具体位置。
    #[must_use]
    pub const fn primary_span(&self) -> Option<&SourceSpan> {
        self.primary_span.as_ref()
    }

    /// 返回与主要错误相关的其他声明或引用位置。
    #[must_use]
    pub fn related_spans(&self) -> &[SourceSpan] {
        &self.related_spans
    }

    /// 返回用于规范排序和快速定位的稳定键（若该诊断与键相关）。
    #[must_use]
    pub fn stable_key(&self) -> Option<&str> {
        self.stable_key.as_deref()
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: ", self.code.as_str())?;
        match &self.payload {
            DiagnosticPayload::InvalidSourceHeaderField { field, violation } => {
                write!(
                    formatter,
                    "来源模块头字段 {} 非法：{}",
                    field.as_str(),
                    SourceTextViolationDisplay(*violation)
                )
            }
            DiagnosticPayload::CompileLimitExceeded {
                dimension,
                limit,
                observed,
            } => write!(
                formatter,
                "编译资源维度 {} 超过上限：允许 {limit}，实际 {observed}",
                dimension.as_str()
            ),
            DiagnosticPayload::InvalidImportNamespace { violation } => write!(
                formatter,
                "导入模块命名空间非法：{}",
                SourceTextViolationDisplay(*violation)
            ),
            DiagnosticPayload::DuplicateImport { namespace } => {
                write!(formatter, "来源模块重复导入 {namespace}")
            }
            DiagnosticPayload::DuplicateModuleNamespace { namespace } => {
                write!(formatter, "编译单元包含重复模块命名空间 {namespace}")
            }
            DiagnosticPayload::DuplicateSourceDocumentKey {
                source_document_key,
            } => write!(
                formatter,
                "编译单元包含重复来源文档键 {source_document_key}"
            ),
            DiagnosticPayload::UnknownImport { namespace } => {
                write!(formatter, "导入目标模块 {namespace} 不存在")
            }
            DiagnosticPayload::ImportCycle { namespaces } => write!(
                formatter,
                "来源模块导入形成循环：{}",
                namespaces
                    .iter()
                    .map(AsRef::as_ref)
                    .collect::<Vec<&str>>()
                    .join(" -> ")
            ),
            DiagnosticPayload::InvalidDeclarationKey {
                entity_kind,
                violation,
            } => write!(
                formatter,
                "{} 声明的稳定键非法：{}",
                entity_kind.slug(),
                SourceTextViolationDisplay(*violation)
            ),
            DiagnosticPayload::DuplicateDeclaration {
                entity_kind,
                stable_key,
            } => write!(
                formatter,
                "来源模块重复声明 {} 稳定键 {stable_key}",
                entity_kind.slug()
            ),
            DiagnosticPayload::InvalidReferenceNamespace { violation } => write!(
                formatter,
                "引用目标模块命名空间非法：{}",
                SourceTextViolationDisplay(*violation)
            ),
            DiagnosticPayload::InvalidReferenceKey {
                entity_kind,
                violation,
            } => write!(
                formatter,
                "指向 {} 声明的引用键非法：{}",
                entity_kind.slug(),
                SourceTextViolationDisplay(*violation)
            ),
            DiagnosticPayload::UnimportedReferenceModule { namespace } => {
                write!(
                    formatter,
                    "引用目标模块 {namespace} 未被当前来源模块显式导入"
                )
            }
            DiagnosticPayload::UnknownReferenceTarget {
                entity_kind,
                source_key,
                target_namespace,
                target_key,
            } => write!(
                formatter,
                "{} 声明 {source_key} 引用了不存在的目标 {target_namespace}:{target_key}",
                entity_kind.slug()
            ),
            DiagnosticPayload::InvalidLaneEdgeLength {
                stable_key,
                value_bits,
                violation,
            } => write!(
                formatter,
                "车道图边 {stable_key} 的长度 {} 非法：{}",
                f64::from_bits(*value_bits),
                ScalarViolationDisplay(*violation)
            ),
            DiagnosticPayload::InvalidLaneEdgeSpeedLimit {
                stable_key,
                value_bits,
                violation,
            } => write!(
                formatter,
                "车道图边 {stable_key} 的基础道路限速 {} 非法：{}",
                f64::from_bits(*value_bits),
                ScalarViolationDisplay(*violation)
            ),
            DiagnosticPayload::DuplicateLaneEdgeSuccessor {
                stable_key,
                target_namespace,
                target_key,
            } => write!(
                formatter,
                "车道图边 {stable_key} 重复声明下游连接 {target_namespace}:{target_key}"
            ),
            DiagnosticPayload::InvalidCanonicalIdentity {
                entity_kind,
                stable_key,
                violation,
            } => write!(
                formatter,
                "{} 声明 {stable_key} 的规范身份非法：{}",
                entity_kind.slug(),
                CanonicalIdentityViolationDisplay(*violation)
            ),
            DiagnosticPayload::DuplicateCanonicalIdentity {
                entity_kind,
                stable_id,
            } => write!(
                formatter,
                "{} 完整规范身份重复，StableId128 为 {stable_id:x}",
                entity_kind.slug()
            ),
            DiagnosticPayload::IdentityDigestCollision {
                entity_kind,
                stable_id,
            } => write!(
                formatter,
                "{} 的不同规范身份产生相同 StableId128 {stable_id:x}",
                entity_kind.slug()
            ),
        }
    }
}

struct CanonicalIdentityViolationDisplay(CanonicalIdentityViolation);

impl fmt::Display for CanonicalIdentityViolationDisplay {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            CanonicalIdentityViolation::FieldCountMismatch { expected, actual } => {
                write!(formatter, "字段数不匹配，要求 {expected}，实际 {actual}")
            }
            CanonicalIdentityViolation::UnknownFieldTag { position, tag } => {
                write!(formatter, "字段位置 {position} 使用未知标签 {tag}")
            }
            CanonicalIdentityViolation::UnexpectedFieldTag {
                position,
                expected,
                actual,
            } => write!(
                formatter,
                "字段位置 {position} 要求标签 {expected}，实际为 {actual}"
            ),
            CanonicalIdentityViolation::InvalidAsciiField { tag, violation } => write!(
                formatter,
                "标签 {tag} 的 ASCII 值非法：{}",
                SourceTextViolationDisplay(violation)
            ),
            CanonicalIdentityViolation::InvalidStableIdLength { tag, actual } => write!(
                formatter,
                "标签 {tag} 的 StableId128 必须为 16 字节，实际为 {actual}"
            ),
            CanonicalIdentityViolation::FieldByteLengthOverflow { tag, actual } => write!(
                formatter,
                "标签 {tag} 的字段字节数不能写入 u32，实际为 {actual}"
            ),
            CanonicalIdentityViolation::CanonicalByteLengthOverflow { actual } => write!(
                formatter,
                "规范身份总字节数不能由当前平台表示，实际为 {actual}"
            ),
        }
    }
}

struct ScalarViolationDisplay(ScalarViolation);

impl fmt::Display for ScalarViolationDisplay {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            ScalarViolation::NotFinite => formatter.write_str("必须是有限数"),
            ScalarViolation::NotGreaterThan {
                exclusive_minimum_bits,
            } => write!(
                formatter,
                "必须严格大于 {}",
                f64::from_bits(exclusive_minimum_bits)
            ),
        }
    }
}

struct SourceTextViolationDisplay(SourceTextViolation);

impl fmt::Display for SourceTextViolationDisplay {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            SourceTextViolation::Empty => formatter.write_str("不得为空"),
            SourceTextViolation::TooLong { limit, observed } => {
                write!(formatter, "字节数超过上限，允许 {limit}，实际 {observed}")
            }
            SourceTextViolation::NonAscii { byte_index } => {
                write!(formatter, "字节位置 {byte_index} 不是 ASCII")
            }
            SourceTextViolation::InvalidFirstByte { byte } => {
                write!(formatter, "首字节 0x{byte:02x} 不是 ASCII 字母或数字")
            }
            SourceTextViolation::InvalidTokenByte { byte_index, byte } => write!(
                formatter,
                "字节位置 {byte_index} 包含非法 ASCII 令牌字节 0x{byte:02x}"
            ),
            SourceTextViolation::ControlByte { byte_index, byte } => {
                write!(formatter, "字节位置 {byte_index} 包含控制字节 0x{byte:02x}")
            }
        }
    }
}

/// 一次失败原子返回的规范有序诊断集合。
///
/// `diagnostics` 始终按规范顺序排列。当安全候选数超过配置档上限时只保留该顺序最小
/// 的前缀，并令 [`DiagnosticBundle::diagnostics_truncated`] 返回 `true`；这不表示编译
/// 可以继续，也不表示未保留候选未被检查。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticBundle {
    diagnostics: Box<[Diagnostic]>,
    diagnostics_truncated: bool,
}

impl DiagnosticBundle {
    pub(crate) fn single(diagnostic: Diagnostic) -> Self {
        Self {
            diagnostics: Box::new([diagnostic]),
            diagnostics_truncated: false,
        }
    }
    /// 返回按规范顺序保留的诊断切片。
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// 指示至少还有一个已发现诊断未被保留。
    #[must_use]
    pub const fn diagnostics_truncated(&self) -> bool {
        self.diagnostics_truncated
    }

    /// 判断保留的诊断中是否包含阻止阶段提交的错误。
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
    }
}

impl fmt::Display for DiagnosticBundle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.diagnostics.first() {
            Some(first) if self.diagnostics.len() == 1 && !self.diagnostics_truncated => {
                first.fmt(formatter)
            }
            Some(first) => write!(
                formatter,
                "{}（共保留 {} 项诊断{}）",
                first,
                self.diagnostics.len(),
                if self.diagnostics_truncated {
                    "，其余已按规范顺序截断"
                } else {
                    ""
                }
            ),
            None => formatter.write_str("诊断集合为空"),
        }
    }
}

impl std::error::Error for DiagnosticBundle {}

/// 有界保留、但不提前终止候选检查的诊断收集器。
pub(crate) struct DiagnosticCollector {
    retained: Vec<Diagnostic>,
    limit: usize,
    diagnostics_truncated: bool,
}

impl DiagnosticCollector {
    pub(crate) fn new(limit: u64) -> Self {
        let limit = usize::try_from(limit).unwrap_or(0);
        Self {
            retained: Vec::with_capacity(limit),
            limit,
            diagnostics_truncated: false,
        }
    }

    pub(crate) fn push(&mut self, diagnostic: Diagnostic) {
        if self.retained.len() < self.limit {
            self.retained.push(diagnostic);
            return;
        }

        self.diagnostics_truncated = true;
        // 不能在容量满时简单丢弃后续项：候选发现顺序不是规范顺序。用新候选替换当前
        // 最大项，才能在扫描结束后得到全体候选的规范最小前缀。
        if let Some((max_index, current_max)) = self
            .retained
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| left.cmp(right))
            && diagnostic < *current_max
        {
            self.retained[max_index] = diagnostic;
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.retained.is_empty() && !self.diagnostics_truncated
    }

    pub(crate) fn finish(mut self) -> DiagnosticBundle {
        self.retained.sort_unstable();
        DiagnosticBundle {
            diagnostics: self.retained.into_boxed_slice(),
            diagnostics_truncated: self.diagnostics_truncated,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_collector_retains_global_canonical_prefix() {
        let mut collector = DiagnosticCollector::new(16);
        let dimensions = [
            CompileLimitDimension::RetainedCapacityBytes,
            CompileLimitDimension::CompilerControlledLiveBytes,
            CompileLimitDimension::OutputBytes,
            CompileLimitDimension::StageScratchBytes,
            CompileLimitDimension::DiagnosticCount,
            CompileLimitDimension::TotalStringBytes,
            CompileLimitDimension::SingleStringBytes,
            CompileLimitDimension::StringItemCount,
            CompileLimitDimension::SymbolCount,
            CompileLimitDimension::GeometryPointCount,
            CompileLimitDimension::WaitingZoneCount,
            CompileLimitDimension::ManeuverGateCount,
            CompileLimitDimension::RouteOccurrenceCount,
            CompileLimitDimension::IdentityFieldOccurrenceCount,
            CompileLimitDimension::RelationOccurrenceCount,
            CompileLimitDimension::ReferenceCount,
            CompileLimitDimension::LirRecordCount,
            CompileLimitDimension::MirRecordCount,
            CompileLimitDimension::HirRecordCount,
            CompileLimitDimension::TypedAstRecordCount,
        ];

        for dimension in dimensions {
            collector.push(Diagnostic::compile_limit_exceeded(dimension, 1, 2));
        }

        let bundle = collector.finish();
        assert!(bundle.diagnostics_truncated());
        assert_eq!(bundle.diagnostics().len(), 16);
        assert!(
            bundle
                .diagnostics()
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        );
        assert!(bundle.diagnostics().iter().all(|diagnostic| {
            !matches!(
                diagnostic.payload(),
                DiagnosticPayload::CompileLimitExceeded {
                    dimension: CompileLimitDimension::RetainedCapacityBytes
                        | CompileLimitDimension::CompilerControlledLiveBytes
                        | CompileLimitDimension::OutputBytes
                        | CompileLimitDimension::StageScratchBytes,
                    ..
                }
            )
        }));
    }

    #[test]
    fn chinese_rendering_keeps_code_and_typed_values() {
        let diagnostic =
            Diagnostic::compile_limit_exceeded(CompileLimitDimension::ModuleCount, 522, 523);
        assert_eq!(diagnostic.code().as_str(), "LF-COMP-RESOURCE-LIMIT");
        assert_eq!(
            diagnostic.to_string(),
            "LF-COMP-RESOURCE-LIMIT: 编译资源维度 max_module_count 超过上限：允许 522，实际 523"
        );
    }

    #[test]
    fn source_span_value_uses_one_based_u32_positions() {
        let span = SourceSpan {
            source_document_key: Arc::from("generator.main"),
            start: SourcePosition { line: 7, column: 3 },
            end: SourcePosition {
                line: 7,
                column: 11,
            },
        };

        assert_eq!(span.source_document_key(), "generator.main");
        assert_eq!(span.start().line(), 7);
        assert_eq!(span.start().column(), 3);
        assert_eq!(span.end().line(), 7);
        assert_eq!(span.end().column(), 11);
    }
}
