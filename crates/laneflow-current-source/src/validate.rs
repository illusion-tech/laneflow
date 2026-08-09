//! production-compatible current source 能力入口。

use std::collections::HashMap;
use std::fmt;

use crate::digest::{MAX_PORTABLE_ARTIFACT_SIZE, encode_digest, parse_digest, sha256_digest};
use crate::error::{
    CurrentArtifactRole, CurrentDocumentRole, CurrentSourceError, CurrentSourceErrorPayload,
    CurrentSourceIssue, CurrentSourceIssueContext, CurrentSourceSpan,
};
use crate::parse::{self, ParseFailure};
use crate::scenario_wire::{WireArtifactDescriptor, WireScenarioManifest, WireSpatialPackage};
use crate::wire::WirePackage;
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

/// 版本闸口（恰好一个合法字符串 `formatVersion` 才参与版本裁决）后单遍解析
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
    let wire = parse::parse_traffic(traffic_bytes)
        .map_err(|failure| context.parse_failure(traffic_bytes, failure))?;
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
    let manifest = parse::parse_manifest(manifest_bytes)
        .map_err(|failure| manifest_context.parse_failure(manifest_bytes, failure))?;
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
    let traffic = parse::parse_traffic(traffic_bytes)
        .map_err(|failure| traffic_context.parse_failure(traffic_bytes, failure))?;
    debug_assert_eq!(traffic.format_version(), CURRENT_TRAFFIC_FORMAT_VERSION);

    let spatial_context = IssueContext {
        document: CurrentDocumentRole::Spatial,
        context: CurrentSourceIssueContext::None,
    };
    let spatial = parse::parse_spatial(spatial_bytes)
        .map_err(|failure| spatial_context.parse_failure(spatial_bytes, failure))?;
    debug_assert_eq!(spatial.format_version(), CURRENT_SPATIAL_FORMAT_VERSION);

    // descriptor 借用 manifest wire；move 进 bundle 前先把 owned 摘要拷出；
    // Manifest 精确摘要对其原始 bytes 恰好计算一次。
    #[cfg(debug_assertions)]
    crate::counters::record_digest(CurrentDocumentRole::Manifest);
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
            Some(self.document),
            self.context.clone(),
            Some(path.into().into_boxed_str()),
            payload,
            None,
        ))
    }

    /// JSON issue 构造：syntax 以 serde 一基位置造单点 span；shape 以延迟候选
    /// 锚点对原始字节做一次 allocation-free 前缀扫描造区间 span；payload 为
    /// `Error::custom` 形态（内部位置 0:0，位置只由 span 承载）。
    fn parse_failure(&self, input: &[u8], failure: ParseFailure) -> CurrentSourceError {
        match failure {
            ParseFailure::Syntax { path, source } => {
                let span = parse::point_span(source.line(), source.column());
                self.error_json(
                    path,
                    Some(span),
                    CurrentSourceErrorPayload::JsonSyntax { source },
                )
            }
            ParseFailure::Shape(candidate) => {
                let span = parse::range_span(input, candidate.anchor);
                let source = <serde_json::Error as serde::de::Error>::custom(candidate.message);
                self.error_json(
                    candidate.path,
                    Some(span),
                    CurrentSourceErrorPayload::JsonShape { source },
                )
            }
            ParseFailure::UnsupportedVersion { expected, actual } => self.error(
                "$",
                CurrentSourceErrorPayload::UnsupportedFormatVersion {
                    expected,
                    actual: actual.into_boxed_str(),
                },
            ),
        }
    }

    fn error_json(
        &self,
        path: impl Into<String>,
        span: Option<CurrentSourceSpan>,
        payload: CurrentSourceErrorPayload,
    ) -> CurrentSourceError {
        CurrentSourceError::single(CurrentSourceIssue::new(
            Some(self.document),
            self.context.clone(),
            Some(path.into().into_boxed_str()),
            payload,
            span,
        ))
    }
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

    #[cfg(debug_assertions)]
    crate::counters::record_digest(match descriptor.role {
        CurrentArtifactRole::Traffic => CurrentDocumentRole::Traffic,
        CurrentArtifactRole::Spatial => CurrentDocumentRole::Spatial,
    });
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

/// 单遍证明（docs/design/current-package-import.md §7）：每份文档恰好一次根
/// deserializer 驱动、恰好一次 SHA-256 计算。计数器状态为线程局部且本测试
/// 先 `reset`，与并行用例无干扰。
#[cfg(all(test, debug_assertions))]
mod single_pass_counter_tests {
    use super::{validate_scenario_compatible, validate_traffic_compatible};
    use crate::counters;
    use crate::{CurrentArtifactInput, CurrentDocumentRole};

    const TRAFFIC_REF: &str = "v0.10-empty-signals-and-parking.laneflow.json";
    const SPATIAL_REF: &str = "v0.1-campus.spatial.json";
    const TRAFFIC: &[u8] =
        include_bytes!("../../../examples/data/v0.10-empty-signals-and-parking.laneflow.json");
    const SPATIAL: &[u8] = include_bytes!("../../../examples/data/v0.1-campus.spatial.json");
    const MANIFEST: &[u8] = include_bytes!("../../../examples/data/v0.1-campus.scenario.json");

    #[test]
    fn counters_pin_one_root_driver_and_one_digest_per_document() {
        counters::reset();
        validate_traffic_compatible(TRAFFIC).expect("traffic fixture 必须校验通过");
        let snapshot = counters::snapshot();
        assert_eq!(
            snapshot.root_drivers, 1,
            "traffic-only 单文档恰好一次根驱动"
        );
        assert!(
            snapshot.digests.is_empty(),
            "traffic-only facade 不计算任何摘要"
        );
        assert!(snapshot.replays > 0, "record token 经 replay 解码");

        counters::reset();
        let artifacts = [
            CurrentArtifactInput::new(TRAFFIC_REF, TRAFFIC, None),
            CurrentArtifactInput::new(SPATIAL_REF, SPATIAL, None),
        ];
        validate_scenario_compatible(MANIFEST, &artifacts).expect("scenario fixture 必须校验通过");
        let snapshot = counters::snapshot();
        assert_eq!(
            snapshot.root_drivers, 3,
            "manifest + traffic + spatial 各一次根驱动"
        );
        // 顺序 = 代码调用序：verify_artifact(traffic) → verify_artifact(spatial)
        // → manifest 精确摘要；「每 token 至多 replay 一次」由计数器硬断言覆盖。
        assert_eq!(
            snapshot.digests,
            vec![
                CurrentDocumentRole::Traffic,
                CurrentDocumentRole::Spatial,
                CurrentDocumentRole::Manifest,
            ],
            "每份文档摘要恰好一次"
        );
    }
}
