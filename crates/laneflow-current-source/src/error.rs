//! Current source 结构化错误面（production-compatible 子集）。

use serde_json::error::Category;

/// 可以产生 JSON 诊断的 current 文档。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CurrentDocumentRole {
    /// ScenarioManifest 文档。
    Manifest,
    /// Traffic package 文档。
    Traffic,
    /// SpatialPackage 文档。
    Spatial,
}

/// ScenarioManifest 描述的制品角色。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CurrentArtifactRole {
    /// Traffic package。
    Traffic,
    /// SpatialPackage。
    Spatial,
}

/// issue 的调用上下文。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CurrentSourceIssueContext {
    /// Traffic-only façade 与其他文档 issue。
    None,
    /// scenario 路径中已完成 Manifest 绑定的 Traffic wire/version issue。
    ScenarioTraffic {
        /// 已完成 Manifest 绑定的 Traffic `artifactRef`。
        artifact_ref: Box<str>,
    },
}

/// 单项 source 校验失败 payload。
///
/// 只含 production-compatible 路径可达的 variant；strict profile 的资源与输入契约
/// variant 归 #297 后续切片。
#[derive(Debug)]
pub enum CurrentSourceErrorPayload {
    /// JSON token、UTF-8、EOF 或 trailing content 无效。
    JsonSyntax {
        /// 立即失败的原始 serde 错误（自带真实 line/column）。
        source: serde_json::Error,
    },
    /// JSON 字段缺失、类型错误、显式 `null` 或包含 unknown/duplicate field。
    JsonShape {
        /// 立即失败的原始 serde 错误（自带真实 line/column）。
        source: serde_json::Error,
    },
    /// `formatVersion` 不是当前接受的版本。
    UnsupportedFormatVersion {
        /// 当前接受的唯一版本。
        expected: &'static str,
        /// 文档实际声明的版本。
        actual: Box<str>,
    },
    /// descriptor 或调用方制品的 `artifactRef` 为空。
    EmptyArtifactReference,
    /// Traffic 与 Spatial descriptor 使用了相同的 `artifactRef`。
    ConflictingManifestArtifactReference {
        /// 冲突的 `artifactRef`。
        artifact_ref: Box<str>,
    },
    /// 调用方制品集合重复声明同一个 `artifactRef`。
    DuplicateProvidedArtifactReference {
        /// 重复的 `artifactRef`。
        artifact_ref: Box<str>,
    },
    /// Manifest 引用的制品不存在于调用方制品集合。
    MissingArtifact {
        /// 缺失制品的角色。
        role: CurrentArtifactRole,
        /// 缺失的 `artifactRef`。
        artifact_ref: Box<str>,
    },
    /// descriptor media type 与角色不匹配。
    InvalidMediaType {
        /// 角色要求的固定 media type。
        expected: &'static str,
        /// descriptor 实际声明的 media type。
        actual: Box<str>,
    },
    /// descriptor digest 不满足 `sha256:<64 lowercase hex>` 词法。
    InvalidDigest {
        /// descriptor 实际声明的 digest。
        actual: Box<str>,
    },
    /// descriptor size 超出 JSON portable integer 范围。
    ArtifactSizeOutOfRange {
        /// descriptor 实际声明的 size。
        actual: u64,
        /// portable 上限。
        max: u64,
    },
    /// 原始制品长度与 descriptor size 不匹配。
    ArtifactSizeMismatch {
        /// 制品角色。
        role: CurrentArtifactRole,
        /// 不匹配的 `artifactRef`。
        artifact_ref: Box<str>,
        /// descriptor 声明的 size。
        expected: u64,
        /// 原始 bytes 的实际长度。
        actual: u64,
    },
    /// 原始制品 SHA-256 与 descriptor digest 不匹配。
    ArtifactDigestMismatch {
        /// 制品角色。
        role: CurrentArtifactRole,
        /// 不匹配的 `artifactRef`。
        artifact_ref: Box<str>,
        /// descriptor 声明的 digest（`sha256:<hex>` 展示形式）。
        expected: Box<str>,
        /// 原始 bytes 的实际摘要（`sha256:<hex>` 展示形式）。
        actual: Box<str>,
    },
}

impl CurrentSourceErrorPayload {
    /// 返回该 variant 的稳定字符串诊断码。
    pub const fn stable_code(&self) -> &'static str {
        match self {
            Self::JsonSyntax { .. } => "LF-CURRENT-SOURCE-JSON-SYNTAX",
            Self::JsonShape { .. } => "LF-CURRENT-SOURCE-JSON-SHAPE",
            Self::UnsupportedFormatVersion { .. } => "LF-CURRENT-SOURCE-UNSUPPORTED-FORMAT-VERSION",
            Self::EmptyArtifactReference => "LF-CURRENT-SOURCE-EMPTY-ARTIFACT-REFERENCE",
            Self::ConflictingManifestArtifactReference { .. } => {
                "LF-CURRENT-SOURCE-CONFLICTING-MANIFEST-ARTIFACT-REFERENCE"
            }
            Self::DuplicateProvidedArtifactReference { .. } => {
                "LF-CURRENT-SOURCE-DUPLICATE-PROVIDED-ARTIFACT-REFERENCE"
            }
            Self::MissingArtifact { .. } => "LF-CURRENT-SOURCE-MISSING-ARTIFACT",
            Self::InvalidMediaType { .. } => "LF-CURRENT-SOURCE-INVALID-MEDIA-TYPE",
            Self::InvalidDigest { .. } => "LF-CURRENT-SOURCE-INVALID-DIGEST",
            Self::ArtifactSizeOutOfRange { .. } => "LF-CURRENT-SOURCE-ARTIFACT-SIZE-OUT-OF-RANGE",
            Self::ArtifactSizeMismatch { .. } => "LF-CURRENT-SOURCE-ARTIFACT-SIZE-MISMATCH",
            Self::ArtifactDigestMismatch { .. } => "LF-CURRENT-SOURCE-ARTIFACT-DIGEST-MISMATCH",
        }
    }
}

/// 一条 source 校验 issue。
#[derive(Debug)]
pub struct CurrentSourceIssue {
    document: CurrentDocumentRole,
    context: CurrentSourceIssueContext,
    path: String,
    payload: CurrentSourceErrorPayload,
}

impl CurrentSourceIssue {
    pub(crate) fn new(
        document: CurrentDocumentRole,
        context: CurrentSourceIssueContext,
        path: String,
        payload: CurrentSourceErrorPayload,
    ) -> Self {
        Self {
            document,
            context,
            path,
            payload,
        }
    }

    /// 返回产生该 issue 的文档角色。
    pub const fn document(&self) -> CurrentDocumentRole {
        self.document
    }

    /// 返回该 issue 的调用上下文。
    pub const fn context(&self) -> &CurrentSourceIssueContext {
        &self.context
    }

    /// 返回规范 `$` path（根为 `$`）。
    pub fn path(&self) -> &str {
        &self.path
    }

    /// 返回该 issue 的 payload。
    pub const fn payload(&self) -> &CurrentSourceErrorPayload {
        &self.payload
    }

    /// 返回该 issue 的稳定字符串诊断码。
    pub const fn stable_code(&self) -> &'static str {
        self.payload.stable_code()
    }

    /// 返回 JSON issue 的 serde category；非 JSON payload 是 data 级语义失败，
    /// 固定返回 `Category::Data`。
    pub fn category(&self) -> Category {
        match &self.payload {
            CurrentSourceErrorPayload::JsonSyntax { source }
            | CurrentSourceErrorPayload::JsonShape { source } => source.classify(),
            _ => Category::Data,
        }
    }

    /// 消费 issue 并返回其 parts 视图。
    pub fn into_parts(self) -> CurrentSourceIssueParts {
        CurrentSourceIssueParts {
            document: self.document,
            context: self.context,
            path: self.path,
            payload: self.payload,
        }
    }
}

/// `CurrentSourceIssue` 的消费型 parts；字段私有。
#[derive(Debug)]
pub struct CurrentSourceIssueParts {
    document: CurrentDocumentRole,
    context: CurrentSourceIssueContext,
    path: String,
    payload: CurrentSourceErrorPayload,
}

impl CurrentSourceIssueParts {
    /// 拆出全部 owned 组件；这是取走不可 Clone `serde_json::Error` 的唯一
    /// owned bridge。
    #[allow(clippy::type_complexity)]
    pub fn into_components(
        self,
    ) -> (
        CurrentDocumentRole,
        CurrentSourceIssueContext,
        String,
        CurrentSourceErrorPayload,
    ) {
        (self.document, self.context, self.path, self.payload)
    }
}

/// 至少含一项 issue 的 source 校验错误 bundle。
///
/// production-compatible 路径全部立即失败，因此 bundle 恒为单元素。
#[derive(Debug, thiserror::Error)]
#[error("current source 校验失败（详见 issues()）")]
pub struct CurrentSourceError {
    issues: Vec<CurrentSourceIssue>,
}

impl CurrentSourceError {
    pub(crate) fn single(issue: CurrentSourceIssue) -> Self {
        Self {
            issues: vec![issue],
        }
    }

    /// 返回按全局规范顺序冻结的 issue 列表（永远非空）。
    pub fn issues(&self) -> &[CurrentSourceIssue] {
        &self.issues
    }

    /// 消费 bundle 并返回 owned issue 列表（永远非空）。
    pub fn into_issues(self) -> Vec<CurrentSourceIssue> {
        self.issues
    }
}
