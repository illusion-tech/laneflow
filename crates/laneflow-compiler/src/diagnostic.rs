use core::fmt;
use std::sync::Arc;

use laneflow_static_contract::EntityKind;

use crate::CompileLimitDimension;
use crate::declaration::ScalarViolation;

/// 来源文档内受检的一基行列位置。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourcePosition {
    line: u32,
    column: u32,
}

impl SourcePosition {
    #[must_use]
    pub const fn line(self) -> u32 {
        self.line
    }

    #[must_use]
    pub const fn column(self) -> u32 {
        self.column
    }
}

/// 与机器路径无关的来源范围。
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceSpan {
    source_document_key: Arc<str>,
    start: SourcePosition,
    end: SourcePosition,
}

impl SourceSpan {
    pub(crate) fn point(source_document_key: Arc<str>, line: u32, column: u32) -> Self {
        let position = SourcePosition { line, column };
        Self {
            source_document_key,
            start: position,
            end: position,
        }
    }

    pub(crate) fn at_caller(
        source_document_key: Arc<str>,
        caller: &'static std::panic::Location<'static>,
    ) -> Self {
        Self::point(source_document_key, caller.line(), caller.column())
    }

    #[must_use]
    pub fn source_document_key(&self) -> &str {
        &self.source_document_key
    }

    #[must_use]
    pub const fn start(&self) -> SourcePosition {
        self.start
    }

    #[must_use]
    pub const fn end(&self) -> SourcePosition {
        self.end
    }
}

/// 稳定诊断代码。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum DiagnosticCode {
    InvalidSourceHeaderField,
    InvalidImportNamespace,
    DuplicateImport,
    DuplicateModuleNamespace,
    UnknownImport,
    ImportCycle,
    InvalidDeclarationKey,
    DuplicateDeclaration,
    InvalidReferenceNamespace,
    InvalidReferenceKey,
    UnimportedReferenceModule,
    UnknownReferenceTarget,
    InvalidLaneEdgeLength,
    InvalidLaneEdgeSpeedLimit,
    DuplicateLaneEdgeSuccessor,
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
            Self::CompileLimitExceeded => "LF-COMP-RESOURCE-LIMIT",
        }
    }
}

/// 诊断严重程度。数值顺序同时是规范排序顺序。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
#[non_exhaustive]
pub enum DiagnosticSeverity {
    Error = 1,
    Warning = 2,
    Note = 3,
}

/// 来源模块头中由调用方提供的字段。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum SourceHeaderField {
    AuthoringNamespaceId,
    SourceDocumentKey,
    GeneratorBuildId,
    Provenance,
}

impl SourceHeaderField {
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
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum SourceTextViolation {
    Empty,
    TooLong { limit: u64, observed: u64 },
    NonAscii { byte_index: u64 },
    InvalidFirstByte { byte: u8 },
    InvalidTokenByte { byte_index: u64, byte: u8 },
    ControlByte { byte_index: u64, byte: u8 },
}

/// 诊断的有类型载荷。
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum DiagnosticPayload {
    InvalidSourceHeaderField {
        field: SourceHeaderField,
        violation: SourceTextViolation,
    },
    CompileLimitExceeded {
        dimension: CompileLimitDimension,
        limit: u64,
        observed: u64,
    },
    InvalidImportNamespace {
        violation: SourceTextViolation,
    },
    DuplicateImport {
        namespace: Box<str>,
    },
    DuplicateModuleNamespace {
        namespace: Box<str>,
    },
    UnknownImport {
        namespace: Box<str>,
    },
    ImportCycle {
        namespaces: Box<[Box<str>]>,
    },
    InvalidDeclarationKey {
        entity_kind: EntityKind,
        violation: SourceTextViolation,
    },
    DuplicateDeclaration {
        entity_kind: EntityKind,
        stable_key: Box<str>,
    },
    InvalidReferenceNamespace {
        violation: SourceTextViolation,
    },
    InvalidReferenceKey {
        entity_kind: EntityKind,
        violation: SourceTextViolation,
    },
    UnimportedReferenceModule {
        namespace: Box<str>,
    },
    UnknownReferenceTarget {
        entity_kind: EntityKind,
        source_key: Box<str>,
        target_namespace: Box<str>,
        target_key: Box<str>,
    },
    InvalidLaneEdgeLength {
        stable_key: Box<str>,
        value_bits: u64,
        violation: ScalarViolation,
    },
    InvalidLaneEdgeSpeedLimit {
        stable_key: Box<str>,
        value_bits: u64,
        violation: ScalarViolation,
    },
    DuplicateLaneEdgeSuccessor {
        stable_key: Box<str>,
        target_namespace: Box<str>,
        target_key: Box<str>,
    },
}

/// 一条不可变结构化诊断。
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

    #[must_use]
    pub const fn code(&self) -> DiagnosticCode {
        self.code
    }

    #[must_use]
    pub const fn severity(&self) -> DiagnosticSeverity {
        self.severity
    }

    #[must_use]
    pub const fn payload(&self) -> &DiagnosticPayload {
        &self.payload
    }

    #[must_use]
    pub const fn primary_span(&self) -> Option<&SourceSpan> {
        self.primary_span.as_ref()
    }

    #[must_use]
    pub fn related_spans(&self) -> &[SourceSpan] {
        &self.related_spans
    }

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
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    #[must_use]
    pub const fn diagnostics_truncated(&self) -> bool {
        self.diagnostics_truncated
    }

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
