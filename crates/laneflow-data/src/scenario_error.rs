//! ScenarioManifest、配套制品与 SpatialPackage 规范化错误。

use std::fmt;

use laneflow_spatial::SpatialError;
use serde_json::error::Category;

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
    pub(crate) fn from_path_error(
        document: ScenarioDocument,
        error: serde_path_to_error::Error<serde_json::Error>,
    ) -> Self {
        let path = normalize_path(error.path().to_string());
        let source = error.into_inner();
        let category = source.classify();
        Self::from_json_error(document, path, source, category)
    }

    pub(crate) fn from_json_error(
        document: ScenarioDocument,
        path: String,
        source: serde_json::Error,
        category: Category,
    ) -> Self {
        let line = source.line();
        let column = source.column();
        match category {
            Category::Data => Self::JsonShape {
                document,
                path,
                line,
                column,
                source,
            },
            Category::Io | Category::Syntax | Category::Eof => Self::JsonSyntax {
                document,
                path,
                line,
                column,
                source,
            },
        }
    }
}

fn normalize_path(path: String) -> String {
    if path.is_empty() || path == "." {
        "$".to_owned()
    } else {
        path
    }
}
