//! production-compatible current source 能力入口。

use std::collections::HashMap;
use std::fmt;

use serde::de::DeserializeOwned;
use serde_json::error::Category;

use crate::digest::{MAX_PORTABLE_ARTIFACT_SIZE, encode_digest, parse_digest, sha256_digest};
use crate::error::{
    CurrentArtifactRole, CurrentDocumentRole, CurrentSourceError, CurrentSourceErrorPayload,
    CurrentSourceIssue, CurrentSourceIssueContext,
};
use crate::scenario_wire::{WireArtifactDescriptor, WireScenarioManifest, WireSpatialPackage};
use crate::wire::{WirePackage, WireVersionHeader};
use crate::{
    CURRENT_SCENARIO_MANIFEST_FORMAT_VERSION, CURRENT_SPATIAL_FORMAT_VERSION,
    CURRENT_TRAFFIC_FORMAT_VERSION, SPATIAL_PACKAGE_MEDIA_TYPE, TRAFFIC_PACKAGE_MEDIA_TYPE,
};

/// 调用方已经读取到内存中的 current 文档输入。
#[derive(Clone, Copy, Debug)]
pub struct CurrentDocumentInput<'a> {
    bytes: &'a [u8],
    display_source: Option<&'a str>,
}

impl<'a> CurrentDocumentInput<'a> {
    /// 创建一个由原始 bytes 与可选展示来源组成的文档视图。
    pub const fn new(bytes: &'a [u8], display_source: Option<&'a str>) -> Self {
        Self {
            bytes,
            display_source,
        }
    }

    /// 返回原始文档 bytes。
    pub const fn bytes(self) -> &'a [u8] {
        self.bytes
    }

    /// 返回可选展示来源。
    pub const fn display_source(self) -> Option<&'a str> {
        self.display_source
    }
}

/// 调用方已经读取到内存中的具名制品输入。
#[derive(Clone, Copy, Debug)]
pub struct CurrentArtifactInput<'a> {
    artifact_ref: &'a str,
    bytes: &'a [u8],
    display_source: Option<&'a str>,
}

impl<'a> CurrentArtifactInput<'a> {
    /// 创建一个由不透明引用、原始 bytes 与可选展示来源组成的制品视图。
    pub const fn new(
        artifact_ref: &'a str,
        bytes: &'a [u8],
        display_source: Option<&'a str>,
    ) -> Self {
        Self {
            artifact_ref,
            bytes,
            display_source,
        }
    }

    /// 返回不透明、大小写敏感的制品引用。
    pub const fn artifact_ref(self) -> &'a str {
        self.artifact_ref
    }

    /// 返回用于 size 与 digest 校验的原始 bytes。
    pub const fn bytes(self) -> &'a [u8] {
        self.bytes
    }

    /// 返回可选展示来源。
    pub const fn display_source(self) -> Option<&'a str> {
        self.display_source
    }
}

/// 已通过版本闸口与完整 wire 校验的 Traffic package 能力。
///
/// 无公开字段、`Default`、Serde、`Clone` 或裸构造器；跨包消费固定为借用
/// accessor 与消费型 `into_parts(self)`。
pub struct ValidatedCurrentTrafficPackage {
    traffic: WirePackage,
}

impl ValidatedCurrentTrafficPackage {
    /// 返回已验证 Traffic wire 的借用视图。
    #[doc(hidden)]
    pub fn traffic_wire(&self) -> &WirePackage {
        &self.traffic
    }

    /// 消费能力并返回 Traffic parts 视图。
    #[doc(hidden)]
    pub fn into_parts(self) -> CurrentTrafficParts {
        CurrentTrafficParts {
            traffic: self.traffic,
        }
    }
}

impl fmt::Debug for ValidatedCurrentTrafficPackage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedCurrentTrafficPackage")
            .finish_non_exhaustive()
    }
}

/// `ValidatedCurrentTrafficPackage` 的消费型 parts；字段私有，无 `Clone`、
/// Serde 或反向构造入口。
#[doc(hidden)]
pub struct CurrentTrafficParts {
    traffic: WirePackage,
}

impl CurrentTrafficParts {
    pub fn traffic_wire(&self) -> &WirePackage {
        &self.traffic
    }

    pub fn into_traffic_wire(self) -> WirePackage {
        self.traffic
    }
}

/// 已完成 Manifest 配对、制品 size/digest 校验与三份 wire 解析的 scenario
/// source 能力。
///
/// 原子拥有三份已验证 wire 内容与三份精确文档摘要（Manifest/Traffic/Spatial
/// 各对原始 bytes 计算一次 SHA-256）；无公开字段、`Default`、Serde、`Clone`
/// 或裸构造器。
pub struct ValidatedCurrentSourceBundle {
    manifest: WireScenarioManifest,
    traffic: WirePackage,
    spatial: WireSpatialPackage,
    manifest_digest: [u8; 32],
    traffic_digest: [u8; 32],
    spatial_digest: [u8; 32],
}

impl ValidatedCurrentSourceBundle {
    /// 返回已验证 Manifest wire 的借用视图。
    #[doc(hidden)]
    pub fn manifest(&self) -> &WireScenarioManifest {
        &self.manifest
    }

    /// 返回已验证 Traffic wire 的借用视图。
    #[doc(hidden)]
    pub fn traffic_wire(&self) -> &WirePackage {
        &self.traffic
    }

    /// 返回已验证 Spatial wire 的借用视图。
    #[doc(hidden)]
    pub fn spatial_wire(&self) -> &WireSpatialPackage {
        &self.spatial
    }

    /// 返回 Manifest 文档已计算的精确摘要。
    #[doc(hidden)]
    pub const fn manifest_digest(&self) -> [u8; 32] {
        self.manifest_digest
    }

    /// 返回 Traffic 制品已校验的精确摘要。
    #[doc(hidden)]
    pub const fn traffic_digest(&self) -> [u8; 32] {
        self.traffic_digest
    }

    /// 返回 Spatial 制品已校验的精确摘要。
    #[doc(hidden)]
    pub const fn spatial_digest(&self) -> [u8; 32] {
        self.spatial_digest
    }

    /// 消费能力并返回 scenario parts 视图。
    #[doc(hidden)]
    pub fn into_parts(self) -> CurrentSourceParts {
        CurrentSourceParts {
            manifest: self.manifest,
            traffic: self.traffic,
            spatial: self.spatial,
            manifest_digest: self.manifest_digest,
            traffic_digest: self.traffic_digest,
            spatial_digest: self.spatial_digest,
        }
    }
}

impl fmt::Debug for ValidatedCurrentSourceBundle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedCurrentSourceBundle")
            .finish_non_exhaustive()
    }
}

/// `ValidatedCurrentSourceBundle` 的消费型 parts；字段私有，无 `Clone`、
/// Serde 或反向构造入口。
#[doc(hidden)]
pub struct CurrentSourceParts {
    manifest: WireScenarioManifest,
    traffic: WirePackage,
    spatial: WireSpatialPackage,
    manifest_digest: [u8; 32],
    traffic_digest: [u8; 32],
    spatial_digest: [u8; 32],
}

impl CurrentSourceParts {
    pub fn manifest(&self) -> &WireScenarioManifest {
        &self.manifest
    }

    pub fn traffic_wire(&self) -> &WirePackage {
        &self.traffic
    }

    pub fn spatial_wire(&self) -> &WireSpatialPackage {
        &self.spatial
    }

    pub const fn manifest_digest(&self) -> [u8; 32] {
        self.manifest_digest
    }

    pub const fn traffic_digest(&self) -> [u8; 32] {
        self.traffic_digest
    }

    pub const fn spatial_digest(&self) -> [u8; 32] {
        self.spatial_digest
    }

    /// 拆出三份 owned wire 文档。
    pub fn into_documents(self) -> (WireScenarioManifest, WirePackage, WireSpatialPackage) {
        (self.manifest, self.traffic, self.spatial)
    }
}

/// 版本闸口（恰好一个合法字符串 `formatVersion` 才参与版本裁决）后完整解析
/// Traffic wire。Traffic-only 能力不虚构 Manifest 或 Spatial。
///
/// # Errors
///
/// JSON syntax/shape 或 format version 校验失败时返回单元素
/// `CurrentSourceError`。
pub fn validate_traffic_compatible(
    traffic_bytes: &[u8],
) -> Result<ValidatedCurrentTrafficPackage, CurrentSourceError> {
    let context = IssueContext {
        document: CurrentDocumentRole::Traffic,
        context: CurrentSourceIssueContext::None,
    };
    gate_format_version(traffic_bytes, CURRENT_TRAFFIC_FORMAT_VERSION, &context)?;
    let wire: WirePackage = deserialize_json(traffic_bytes, &context)?;
    debug_assert_eq!(wire.format_version(), CURRENT_TRAFFIC_FORMAT_VERSION);
    Ok(ValidatedCurrentTrafficPackage { traffic: wire })
}

/// 按冻结失败顺序校验 Manifest、配对调用方制品并解析 Traffic/Spatial wire。
///
/// 冻结顺序：Manifest syntax → `formatVersion` 头部 shape → unsupported
/// version → 其他 Manifest shape → Traffic descriptor → Spatial descriptor →
/// conflicting ref → provided refs（空/重复）→ Traffic size→digest → Spatial
/// size→digest → Traffic wire → Spatial wire。额外唯一制品只检查非空与唯一，
/// 不哈希、不解析、不复制；Manifest/Traffic/Spatial 摘要各计算一次。
///
/// # Errors
///
/// 任一冻结步骤失败时返回单元素 `CurrentSourceError`。
pub fn validate_scenario_compatible(
    manifest_bytes: &[u8],
    artifacts: &[CurrentArtifactInput<'_>],
) -> Result<ValidatedCurrentSourceBundle, CurrentSourceError> {
    let manifest_context = IssueContext {
        document: CurrentDocumentRole::Manifest,
        context: CurrentSourceIssueContext::None,
    };
    gate_format_version(
        manifest_bytes,
        CURRENT_SCENARIO_MANIFEST_FORMAT_VERSION,
        &manifest_context,
    )?;
    let manifest: WireScenarioManifest = deserialize_json(manifest_bytes, &manifest_context)?;
    debug_assert_eq!(
        manifest.format_version(),
        CURRENT_SCENARIO_MANIFEST_FORMAT_VERSION
    );

    let traffic_descriptor = validate_descriptor(
        CurrentArtifactRole::Traffic,
        "traffic",
        TRAFFIC_PACKAGE_MEDIA_TYPE,
        manifest.traffic(),
        &manifest_context,
    )?;
    let spatial_descriptor = validate_descriptor(
        CurrentArtifactRole::Spatial,
        "spatial",
        SPATIAL_PACKAGE_MEDIA_TYPE,
        manifest.spatial(),
        &manifest_context,
    )?;
    if traffic_descriptor.artifact_ref == spatial_descriptor.artifact_ref {
        return Err(manifest_context.error(
            "$",
            CurrentSourceErrorPayload::ConflictingManifestArtifactReference {
                artifact_ref: traffic_descriptor.artifact_ref.into(),
            },
        ));
    }

    let artifacts = collect_artifacts(artifacts, &manifest_context)?;
    let traffic_bytes = verify_artifact(traffic_descriptor, &artifacts, &manifest_context)?;
    let spatial_bytes = verify_artifact(spatial_descriptor, &artifacts, &manifest_context)?;

    let traffic_context = IssueContext {
        document: CurrentDocumentRole::Traffic,
        context: CurrentSourceIssueContext::ScenarioTraffic {
            artifact_ref: traffic_descriptor.artifact_ref.into(),
        },
    };
    gate_format_version(
        traffic_bytes,
        CURRENT_TRAFFIC_FORMAT_VERSION,
        &traffic_context,
    )?;
    let traffic: WirePackage = deserialize_json(traffic_bytes, &traffic_context)?;
    debug_assert_eq!(traffic.format_version(), CURRENT_TRAFFIC_FORMAT_VERSION);

    let spatial_context = IssueContext {
        document: CurrentDocumentRole::Spatial,
        context: CurrentSourceIssueContext::None,
    };
    gate_format_version(
        spatial_bytes,
        CURRENT_SPATIAL_FORMAT_VERSION,
        &spatial_context,
    )?;
    let spatial: WireSpatialPackage = deserialize_json(spatial_bytes, &spatial_context)?;
    debug_assert_eq!(spatial.format_version(), CURRENT_SPATIAL_FORMAT_VERSION);

    // descriptor 借用 manifest wire；move 进 bundle 前先把 owned 摘要拷出；
    // Manifest 精确摘要对其原始 bytes 恰好计算一次。
    let manifest_digest = sha256_digest(manifest_bytes);
    let traffic_digest = traffic_descriptor.digest;
    let spatial_digest = spatial_descriptor.digest;
    Ok(ValidatedCurrentSourceBundle {
        manifest,
        traffic,
        spatial,
        manifest_digest,
        traffic_digest,
        spatial_digest,
    })
}

/// 单文档 issue 的构造上下文（document + context）。
#[derive(Clone)]
struct IssueContext {
    document: CurrentDocumentRole,
    context: CurrentSourceIssueContext,
}

impl IssueContext {
    fn error(
        &self,
        path: impl Into<String>,
        payload: CurrentSourceErrorPayload,
    ) -> CurrentSourceError {
        CurrentSourceError::single(CurrentSourceIssue::new(
            self.document,
            self.context.clone(),
            path.into(),
            payload,
        ))
    }

    /// 按 serde category 分流 JSON payload：`Data` 归 shape，其余归 syntax，
    /// 与 `laneflow-data` 迁移前的分类逐字节一致。
    fn json(&self, path: String, source: serde_json::Error) -> CurrentSourceError {
        let payload = match source.classify() {
            Category::Data => CurrentSourceErrorPayload::JsonShape { source },
            Category::Io | Category::Syntax | Category::Eof => {
                CurrentSourceErrorPayload::JsonSyntax { source }
            }
        };
        self.error(path, payload)
    }
}

/// 带 `serde_path_to_error` 路径跟踪的单次完整 JSON 解析；trailing content
/// 在根 `$` 处以 syntax 失败。
fn deserialize_json<T>(input: &[u8], context: &IssueContext) -> Result<T, CurrentSourceError>
where
    T: DeserializeOwned,
{
    let mut deserializer = serde_json::Deserializer::from_slice(input);
    let value = serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
        let path = normalize_path(error.path().to_string());
        context.json(path, error.into_inner())
    })?;
    deserializer
        .end()
        .map_err(|source| context.json("$".to_owned(), source))?;
    Ok(value)
}

fn normalize_path(path: String) -> String {
    if path.is_empty() || path == "." {
        "$".to_owned()
    } else {
        path
    }
}

/// 头部版本闸口：先解析 `WireVersionHeader`，再裁决 unsupported version。
fn gate_format_version(
    input: &[u8],
    expected: &'static str,
    context: &IssueContext,
) -> Result<(), CurrentSourceError> {
    let header: WireVersionHeader = deserialize_json(input, context)?;
    if header.format_version != expected {
        return Err(context.error(
            "$",
            CurrentSourceErrorPayload::UnsupportedFormatVersion {
                expected,
                actual: header.format_version.into_boxed_str(),
            },
        ));
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct ValidatedDescriptor<'a> {
    role: CurrentArtifactRole,
    artifact_ref: &'a str,
    digest: [u8; 32],
    size: u64,
}

/// descriptor 语义校验：非空 ref → media type → portable size → digest 词法。
fn validate_descriptor<'a>(
    role: CurrentArtifactRole,
    path: &'static str,
    expected_media_type: &'static str,
    descriptor: &'a WireArtifactDescriptor,
    context: &IssueContext,
) -> Result<ValidatedDescriptor<'a>, CurrentSourceError> {
    if descriptor.artifact_ref().is_empty() {
        return Err(context.error(
            format!("{path}.artifactRef"),
            CurrentSourceErrorPayload::EmptyArtifactReference,
        ));
    }
    if descriptor.media_type() != expected_media_type {
        return Err(context.error(
            format!("{path}.mediaType"),
            CurrentSourceErrorPayload::InvalidMediaType {
                expected: expected_media_type,
                actual: descriptor.media_type().into(),
            },
        ));
    }
    if descriptor.size() > MAX_PORTABLE_ARTIFACT_SIZE {
        return Err(context.error(
            format!("{path}.size"),
            CurrentSourceErrorPayload::ArtifactSizeOutOfRange {
                actual: descriptor.size(),
                max: MAX_PORTABLE_ARTIFACT_SIZE,
            },
        ));
    }
    let digest = parse_digest(descriptor.digest()).ok_or_else(|| {
        context.error(
            format!("{path}.digest"),
            CurrentSourceErrorPayload::InvalidDigest {
                actual: descriptor.digest().into(),
            },
        )
    })?;

    Ok(ValidatedDescriptor {
        role,
        artifact_ref: descriptor.artifact_ref(),
        digest,
        size: descriptor.size(),
    })
}

/// 调用方制品集合预检：只检查非空与全集合唯一，不哈希、不解析、不复制。
fn collect_artifacts<'a>(
    artifacts: &[CurrentArtifactInput<'a>],
    context: &IssueContext,
) -> Result<HashMap<&'a str, &'a [u8]>, CurrentSourceError> {
    let mut by_ref = HashMap::with_capacity(artifacts.len());
    for (index, artifact) in artifacts.iter().copied().enumerate() {
        if artifact.artifact_ref.is_empty() {
            return Err(context.error(
                format!("artifacts[{index}].artifactRef"),
                CurrentSourceErrorPayload::EmptyArtifactReference,
            ));
        }
        if by_ref
            .insert(artifact.artifact_ref, artifact.bytes)
            .is_some()
        {
            return Err(context.error(
                format!("artifacts[{index}].artifactRef"),
                CurrentSourceErrorPayload::DuplicateProvidedArtifactReference {
                    artifact_ref: artifact.artifact_ref.into(),
                },
            ));
        }
    }
    Ok(by_ref)
}

/// 定位目标制品并按 size → digest 顺序校验原始 bytes；每份制品摘要只计算一次。
fn verify_artifact<'a>(
    descriptor: ValidatedDescriptor<'_>,
    artifacts: &'a HashMap<&str, &'a [u8]>,
    manifest_context: &IssueContext,
) -> Result<&'a [u8], CurrentSourceError> {
    let artifact_context = IssueContext {
        document: match descriptor.role {
            CurrentArtifactRole::Traffic => CurrentDocumentRole::Traffic,
            CurrentArtifactRole::Spatial => CurrentDocumentRole::Spatial,
        },
        context: CurrentSourceIssueContext::None,
    };
    let bytes = artifacts
        .get(descriptor.artifact_ref)
        .copied()
        .ok_or_else(|| {
            manifest_context.error(
                "$",
                CurrentSourceErrorPayload::MissingArtifact {
                    role: descriptor.role,
                    artifact_ref: descriptor.artifact_ref.into(),
                },
            )
        })?;
    let actual_size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if actual_size != descriptor.size {
        return Err(artifact_context.error(
            "$",
            CurrentSourceErrorPayload::ArtifactSizeMismatch {
                role: descriptor.role,
                artifact_ref: descriptor.artifact_ref.into(),
                expected: descriptor.size,
                actual: actual_size,
            },
        ));
    }

    let actual_digest = sha256_digest(bytes);
    if actual_digest != descriptor.digest {
        return Err(artifact_context.error(
            "$",
            CurrentSourceErrorPayload::ArtifactDigestMismatch {
                role: descriptor.role,
                artifact_ref: descriptor.artifact_ref.into(),
                expected: encode_digest(&descriptor.digest).into(),
                actual: encode_digest(&actual_digest).into(),
            },
        ));
    }
    Ok(bytes)
}
