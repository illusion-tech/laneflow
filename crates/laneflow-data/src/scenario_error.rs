//! ScenarioManifest、配套制品与 SpatialPackage 规范化错误。

use std::fmt;

use laneflow_current_source::{
    CurrentArtifactRole, CurrentDocumentRole, CurrentSourceError, CurrentSourceErrorPayload,
    CurrentSourceIssueContext, CurrentSourceSpan,
};
use laneflow_spatial::SpatialError;

use crate::DataError;

/// 场景加载过程中可以产生 JSON 诊断的文档。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScenarioDocument {
    /// ScenarioManifest 文档。
    Manifest,
    /// SpatialPackage 文档。
    Spatial,
}

impl fmt::Display for ScenarioDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Manifest => "manifest",
            Self::Spatial => "spatial",
        })
    }
}

/// ScenarioManifest 中的制品角色。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactRole {
    /// Traffic Data package。
    Traffic,
    /// SpatialPackage。
    Spatial,
}

impl fmt::Display for ArtifactRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Traffic => "traffic",
            Self::Spatial => "spatial",
        })
    }
}

/// 场景清单、配套制品与空间包的结构化加载错误。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ScenarioError {
    /// Manifest 或 Spatial JSON token、UTF-8、EOF 或 trailing content 无效。
    #[error("{document} JSON syntax 无效：path={path}, line={line}, column={column}：{source}")]
    JsonSyntax {
        document: ScenarioDocument,
        path: String,
        line: usize,
        column: usize,
        #[source]
        source: serde_json::Error,
    },
    /// Manifest 或 Spatial JSON 字段缺失、类型错误或包含 unknown field。
    #[error("{document} JSON shape 无效：path={path}, line={line}, column={column}：{source}")]
    JsonShape {
        document: ScenarioDocument,
        path: String,
        line: usize,
        column: usize,
        #[source]
        source: serde_json::Error,
    },
    /// Manifest 或 SpatialPackage 版本不是当前版本。
    #[error("不支持 {document} format version：expected=`{expected}`, actual=`{actual}`")]
    UnsupportedFormatVersion {
        document: ScenarioDocument,
        expected: &'static str,
        actual: String,
    },
    /// `artifactRef` 为空。
    #[error("artifactRef 不能为空：path={path}")]
    EmptyArtifactRef { path: String },
    /// Traffic 与 Spatial descriptor 使用了相同的 `artifactRef`。
    #[error("traffic 与 spatial artifactRef 必须不同：`{artifact_ref}`")]
    ConflictingManifestArtifactRef { artifact_ref: String },
    /// 调用方制品集合重复声明同一个 `artifactRef`。
    #[error("调用方制品集合重复 artifactRef：path={path}, artifactRef=`{artifact_ref}`")]
    DuplicateProvidedArtifactRef { path: String, artifact_ref: String },
    /// Manifest 引用的制品不存在。
    #[error("缺少 {role} 制品：artifactRef=`{artifact_ref}`")]
    MissingArtifact {
        role: ArtifactRole,
        artifact_ref: String,
    },
    /// Descriptor media type 与角色不匹配。
    #[error("mediaType 无效：path={path}, expected=`{expected}`, actual=`{actual}`")]
    InvalidMediaType {
        path: &'static str,
        expected: &'static str,
        actual: String,
    },
    /// Descriptor digest 不满足首版 SHA-256 语法。
    #[error("digest 无效：path={path}, expected=`sha256:<64 lowercase hex>`, actual=`{actual}`")]
    InvalidDigest { path: &'static str, actual: String },
    /// Descriptor size 超出 JSON portable integer 范围。
    #[error("制品 size 超出范围：path={path}, actual={actual}, max={max}")]
    ArtifactSizeOutOfRange {
        path: &'static str,
        actual: u64,
        max: u64,
    },
    /// 原始制品长度与 descriptor 不匹配。
    #[error(
        "{role} 制品 size 不匹配：artifactRef=`{artifact_ref}`, expected={expected}, actual={actual}"
    )]
    ArtifactSizeMismatch {
        role: ArtifactRole,
        artifact_ref: String,
        expected: u64,
        actual: u64,
    },
    /// 原始制品 SHA-256 与 descriptor 不匹配。
    #[error(
        "{role} 制品 digest 不匹配：artifactRef=`{artifact_ref}`, expected=`{expected}`, actual=`{actual}`"
    )]
    ArtifactDigestMismatch {
        role: ArtifactRole,
        artifact_ref: String,
        expected: String,
        actual: String,
    },
    /// Traffic package 未通过现有 Data loader。
    #[error("traffic 制品加载失败：artifactRef=`{artifact_ref}`：{source}")]
    TrafficPackage {
        artifact_ref: String,
        #[source]
        source: Box<DataError>,
    },
    /// `frameId` 或规范化点违反 Spatial domain invariant。
    #[error("Spatial domain validation 失败：path={path}：{source}")]
    SpatialDomain {
        path: String,
        #[source]
        source: SpatialError,
    },
    /// 中心线点数不足。
    #[error("中心线点数不足：path={path}, min={min}, actual={actual}")]
    InsufficientCenterlinePoints {
        path: String,
        min: usize,
        actual: usize,
    },
    /// 原始高保真坐标不是有限数。
    #[error("坐标必须是有限数：path={path}, actual={value:?}")]
    NonFiniteCoordinate { path: String, value: f64 },
    /// 原始高保真坐标超出 canonical frame 范围。
    #[error(
        "坐标超出 canonical frame 范围：path={path}, actual={value:?}, range=[{min:?}, {max:?}]"
    )]
    CoordinateOutOfRange {
        path: String,
        value: f64,
        min: f64,
        max: f64,
    },
    /// Spatial edge 引用了 Traffic lane graph 中不存在的 external ID。
    #[error("Spatial edge 引用未知 trafficEdgeId：path={path}, trafficEdgeId=`{traffic_edge_id}`")]
    UnknownTrafficEdge {
        path: String,
        traffic_edge_id: String,
    },
    /// Spatial package 重复绑定同一个 Traffic edge。
    #[error("Spatial edge 重复 trafficEdgeId：path={path}, trafficEdgeId=`{traffic_edge_id}`")]
    DuplicateTrafficEdge {
        path: String,
        traffic_edge_id: String,
    },
    /// Traffic lane graph 中的 edge 没有 Spatial 绑定。
    #[error("Traffic edge 缺少 Spatial 绑定：path={path}, trafficEdgeId=`{traffic_edge_id}`")]
    MissingTrafficEdge {
        path: &'static str,
        traffic_edge_id: String,
    },
}

impl ScenarioError {
    /// 把 scenario source 错误映射回现有 loader 错误形状；携带 `ScenarioTraffic`
    /// 上下文的 Traffic wire/version issue 先还原为 `DataError` 再包进
    /// `TrafficPackage`，保持既有公共错误面不变。
    pub(crate) fn from_current_source(error: CurrentSourceError) -> Self {
        let issues = error.into_issues();
        debug_assert_eq!(issues.len(), 1, "production-compatible source 立即失败");
        let issue = issues
            .into_iter()
            .next()
            .expect("CurrentSourceError 至少含一项 issue");
        let (payload, document, context, path, span) = issue.into_parts().into_components();
        let path = path
            .expect("production-compatible issue 必携带规范 path")
            .into_string();
        if let CurrentSourceIssueContext::ScenarioTraffic { artifact_ref } = context {
            return Self::TrafficPackage {
                artifact_ref: artifact_ref.into_string(),
                source: Box::new(DataError::from_traffic_payload(path, payload, span)),
            };
        }
        // document 只在 JSON/version variant 上有公共意义；制品配对 variant
        // （Missing/Size/DigestMismatch）在现有错误面上不携带 document，其
        // document 值不会被观察。
        let document = document.expect("production-compatible issue 必携带 document");
        let scenario_document = match document {
            CurrentDocumentRole::Manifest => ScenarioDocument::Manifest,
            CurrentDocumentRole::Spatial => ScenarioDocument::Spatial,
            CurrentDocumentRole::Traffic => match &payload {
                CurrentSourceErrorPayload::MissingArtifact { .. }
                | CurrentSourceErrorPayload::ArtifactSizeMismatch { .. }
                | CurrentSourceErrorPayload::ArtifactDigestMismatch { .. } => {
                    ScenarioDocument::Manifest
                }
                payload => unreachable!(
                    "scenario Traffic JSON/version issue 必携带 ScenarioTraffic 上下文：{}",
                    payload.stable_code()
                ),
            },
        };
        match payload {
            CurrentSourceErrorPayload::JsonSyntax { source } => {
                let (line, column) = json_issue_position(span);
                Self::JsonSyntax {
                    document: scenario_document,
                    path,
                    line,
                    column,
                    source,
                }
            }
            CurrentSourceErrorPayload::JsonShape { source } => {
                let (line, column) = json_issue_position(span);
                Self::JsonShape {
                    document: scenario_document,
                    path,
                    line,
                    column,
                    source,
                }
            }
            CurrentSourceErrorPayload::UnsupportedFormatVersion { expected, actual } => {
                Self::UnsupportedFormatVersion {
                    document: scenario_document,
                    expected,
                    actual: actual.into_string(),
                }
            }
            CurrentSourceErrorPayload::EmptyArtifactReference => Self::EmptyArtifactRef { path },
            CurrentSourceErrorPayload::ConflictingManifestArtifactReference { artifact_ref } => {
                Self::ConflictingManifestArtifactRef {
                    artifact_ref: artifact_ref.into_string(),
                }
            }
            CurrentSourceErrorPayload::DuplicateProvidedArtifactReference { artifact_ref } => {
                Self::DuplicateProvidedArtifactRef {
                    path,
                    artifact_ref: artifact_ref.into_string(),
                }
            }
            CurrentSourceErrorPayload::MissingArtifact { role, artifact_ref } => {
                Self::MissingArtifact {
                    role: artifact_role(role),
                    artifact_ref: artifact_ref.into_string(),
                }
            }
            CurrentSourceErrorPayload::InvalidMediaType { expected, actual } => {
                Self::InvalidMediaType {
                    path: static_descriptor_path(&path),
                    expected,
                    actual: actual.into_string(),
                }
            }
            CurrentSourceErrorPayload::InvalidDigest { actual } => Self::InvalidDigest {
                path: static_descriptor_path(&path),
                actual: actual.into_string(),
            },
            CurrentSourceErrorPayload::ArtifactSizeOutOfRange { actual, max } => {
                Self::ArtifactSizeOutOfRange {
                    path: static_descriptor_path(&path),
                    actual,
                    max,
                }
            }
            CurrentSourceErrorPayload::ArtifactSizeMismatch {
                role,
                artifact_ref,
                expected,
                actual,
            } => Self::ArtifactSizeMismatch {
                role: artifact_role(role),
                artifact_ref: artifact_ref.into_string(),
                expected,
                actual,
            },
            CurrentSourceErrorPayload::ArtifactDigestMismatch {
                role,
                artifact_ref,
                expected,
                actual,
            } => Self::ArtifactDigestMismatch {
                role: artifact_role(role),
                artifact_ref: artifact_ref.into_string(),
                expected: expected.into_string(),
                actual: actual.into_string(),
            },
        }
    }
}

fn artifact_role(role: CurrentArtifactRole) -> ArtifactRole {
    match role {
        CurrentArtifactRole::Traffic => ArtifactRole::Traffic,
        CurrentArtifactRole::Spatial => ArtifactRole::Spatial,
    }
}

/// JSON issue 的一基位置：只读 span 的显式 `start`（shape payload 的 serde
/// 错误内部位置恒为 0:0）。
fn json_issue_position(span: Option<CurrentSourceSpan>) -> (usize, usize) {
    let start = span.expect("production JSON issue 必携带 span").start();
    (start.line() as usize, start.column() as usize)
}

/// descriptor payload 的 path 在 source 侧就是固定静态串，映射回既有
/// `&'static str` path 字段。
fn static_descriptor_path(path: &str) -> &'static str {
    match path {
        "traffic.mediaType" => "traffic.mediaType",
        "spatial.mediaType" => "spatial.mediaType",
        "traffic.digest" => "traffic.digest",
        "spatial.digest" => "spatial.digest",
        "traffic.size" => "traffic.size",
        "spatial.size" => "spatial.size",
        other => unreachable!("descriptor payload 路径必须是固定静态串：{other}"),
    }
}
