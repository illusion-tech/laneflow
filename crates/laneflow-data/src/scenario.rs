//! ScenarioManifest 与 SpatialPackage 的内存加载和原子规范化。

use std::collections::HashMap;

use laneflow_core::{EdgeHandle, LaneGraph};
use laneflow_spatial::{
    CANONICAL_POINT_COMPONENT_MAX_METERS, CANONICAL_POINT_COMPONENT_MIN_METERS, CanonicalFrameId,
    CanonicalPoint3F32,
};
use serde::de::DeserializeOwned;
use serde_json::error::Category;
use sha2::{Digest, Sha256};

use crate::scenario_error::{ArtifactRole, ScenarioDocument, ScenarioError};
use crate::scenario_wire::{WireArtifactDescriptor, WireScenarioManifest, WireSpatialPackage};
use crate::wire::WireVersionHeader;
use crate::{LoadedPackage, from_json_slice};

/// ScenarioManifest loader 接受的唯一当前版本。
pub const CURRENT_SCENARIO_MANIFEST_FORMAT_VERSION: &str = "0.1";

/// SpatialPackage loader 接受的唯一当前版本。
pub const CURRENT_SPATIAL_FORMAT_VERSION: &str = "0.1";

/// Traffic package descriptor 的固定 media type。
pub const TRAFFIC_PACKAGE_MEDIA_TYPE: &str = "application/vnd.laneflow.traffic+json";

/// Spatial package descriptor 的固定 media type。
pub const SPATIAL_PACKAGE_MEDIA_TYPE: &str = "application/vnd.laneflow.spatial+json";

const MAX_PORTABLE_ARTIFACT_SIZE: u64 = 9_007_199_254_740_991;
const MIN_CENTERLINE_POINT_COUNT: usize = 2;

/// 调用方已经读取到内存中的具名制品。
#[derive(Clone, Copy, Debug)]
pub struct NamedArtifact<'a> {
    artifact_ref: &'a str,
    bytes: &'a [u8],
}

impl<'a> NamedArtifact<'a> {
    /// 创建一个由不透明引用和原始 bytes 组成的调用方制品视图。
    pub const fn new(artifact_ref: &'a str, bytes: &'a [u8]) -> Self {
        Self {
            artifact_ref,
            bytes,
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
}

/// 已完成 Traffic/Spatial 配对与原子规范化的场景输入。
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedScenario {
    traffic: LoadedPackage,
    spatial: LoadedSpatialPackage,
}

impl LoadedScenario {
    /// 返回现有 Traffic Data loader 的规范化结果。
    pub const fn traffic(&self) -> &LoadedPackage {
        &self.traffic
    }

    /// 返回只含受检 F32 点的 Spatial 规范化结果。
    pub const fn spatial(&self) -> &LoadedSpatialPackage {
        &self.spatial
    }

    /// 拆分为 Traffic 与 Spatial 两个完整结果。
    pub fn into_parts(self) -> (LoadedPackage, LoadedSpatialPackage) {
        (self.traffic, self.spatial)
    }
}

/// 已绑定到当前 Traffic lane graph 的 SpatialPackage 规范化结果。
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedSpatialPackage {
    frame_id: CanonicalFrameId,
    edges: Vec<LoadedSpatialEdge>,
}

impl LoadedSpatialPackage {
    /// 返回空间包的 canonical frame ID。
    pub const fn frame_id(&self) -> &CanonicalFrameId {
        &self.frame_id
    }

    /// 返回按 `LaneGraph::edges()` 稳定顺序排列的完整 edge 输入。
    pub fn edges(&self) -> &[LoadedSpatialEdge] {
        &self.edges
    }
}

/// 一条已解析为 Core handle、但尚未执行 #135 几何构建的中心线输入。
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedSpatialEdge {
    edge: EdgeHandle,
    points: Vec<CanonicalPoint3F32>,
}

impl LoadedSpatialEdge {
    /// 返回已解析的 Core edge handle。
    pub const fn edge(&self) -> EdgeHandle {
        self.edge
    }

    /// 返回有向中心线的受检 canonical F32 点。
    pub fn points(&self) -> &[CanonicalPoint3F32] {
        &self.points
    }
}

/// 从 manifest JSON bytes 和调用方提供的具名原始制品集合加载完整场景。
///
/// # Errors
///
/// Manifest/Spatial syntax、shape、version、descriptor、原始 bytes identity、Traffic loader、
/// 坐标转换或 edge coverage 任一步失败时返回 `ScenarioError`。本函数不读取文件、不联网，
/// 也不返回部分规范化结果。
pub fn from_scenario_json_slice(
    manifest_input: &[u8],
    artifacts: &[NamedArtifact<'_>],
) -> Result<LoadedScenario, ScenarioError> {
    let header: WireVersionHeader = deserialize_json(manifest_input, ScenarioDocument::Manifest)?;
    if header.format_version != CURRENT_SCENARIO_MANIFEST_FORMAT_VERSION {
        return Err(ScenarioError::UnsupportedFormatVersion {
            document: ScenarioDocument::Manifest,
            expected: CURRENT_SCENARIO_MANIFEST_FORMAT_VERSION,
            actual: header.format_version,
        });
    }

    let manifest: WireScenarioManifest =
        deserialize_json(manifest_input, ScenarioDocument::Manifest)?;
    debug_assert_eq!(manifest.format_version, header.format_version);
    let traffic_descriptor = validate_descriptor(
        ArtifactRole::Traffic,
        "traffic",
        TRAFFIC_PACKAGE_MEDIA_TYPE,
        &manifest.traffic,
    )?;
    let spatial_descriptor = validate_descriptor(
        ArtifactRole::Spatial,
        "spatial",
        SPATIAL_PACKAGE_MEDIA_TYPE,
        &manifest.spatial,
    )?;
    if traffic_descriptor.artifact_ref == spatial_descriptor.artifact_ref {
        return Err(ScenarioError::ConflictingManifestArtifactRef {
            artifact_ref: traffic_descriptor.artifact_ref.to_owned(),
        });
    }

    let artifacts = collect_artifacts(artifacts)?;
    let traffic_bytes = verify_artifact(traffic_descriptor, &artifacts)?;
    let spatial_bytes = verify_artifact(spatial_descriptor, &artifacts)?;

    let traffic =
        from_json_slice(traffic_bytes).map_err(|source| ScenarioError::TrafficPackage {
            artifact_ref: traffic_descriptor.artifact_ref.to_owned(),
            source: Box::new(source),
        })?;
    let spatial = load_spatial(spatial_bytes, traffic.initial_traffic_data().lane_graph())?;

    Ok(LoadedScenario { traffic, spatial })
}

/// 从 manifest JSON string 和调用方提供的具名原始制品集合加载完整场景。
///
/// # Errors
///
/// 与 `from_scenario_json_slice` 相同。
pub fn from_scenario_json_str(
    manifest_input: &str,
    artifacts: &[NamedArtifact<'_>],
) -> Result<LoadedScenario, ScenarioError> {
    from_scenario_json_slice(manifest_input.as_bytes(), artifacts)
}

#[derive(Clone, Copy)]
struct ValidatedDescriptor<'a> {
    role: ArtifactRole,
    artifact_ref: &'a str,
    digest: [u8; 32],
    size: u64,
}

fn validate_descriptor<'a>(
    role: ArtifactRole,
    path: &'static str,
    expected_media_type: &'static str,
    descriptor: &'a WireArtifactDescriptor,
) -> Result<ValidatedDescriptor<'a>, ScenarioError> {
    if descriptor.artifact_ref.is_empty() {
        return Err(ScenarioError::EmptyArtifactRef {
            path: format!("{path}.artifactRef"),
        });
    }
    if descriptor.media_type != expected_media_type {
        return Err(ScenarioError::InvalidMediaType {
            path: match role {
                ArtifactRole::Traffic => "traffic.mediaType",
                ArtifactRole::Spatial => "spatial.mediaType",
            },
            expected: expected_media_type,
            actual: descriptor.media_type.clone(),
        });
    }
    if descriptor.size > MAX_PORTABLE_ARTIFACT_SIZE {
        return Err(ScenarioError::ArtifactSizeOutOfRange {
            path: match role {
                ArtifactRole::Traffic => "traffic.size",
                ArtifactRole::Spatial => "spatial.size",
            },
            actual: descriptor.size,
            max: MAX_PORTABLE_ARTIFACT_SIZE,
        });
    }
    let digest = parse_digest(
        match role {
            ArtifactRole::Traffic => "traffic.digest",
            ArtifactRole::Spatial => "spatial.digest",
        },
        &descriptor.digest,
    )?;

    Ok(ValidatedDescriptor {
        role,
        artifact_ref: &descriptor.artifact_ref,
        digest,
        size: descriptor.size,
    })
}

fn collect_artifacts<'a>(
    artifacts: &[NamedArtifact<'a>],
) -> Result<HashMap<&'a str, &'a [u8]>, ScenarioError> {
    let mut by_ref = HashMap::with_capacity(artifacts.len());
    for (index, artifact) in artifacts.iter().copied().enumerate() {
        if artifact.artifact_ref.is_empty() {
            return Err(ScenarioError::EmptyArtifactRef {
                path: format!("artifacts[{index}].artifactRef"),
            });
        }
        if by_ref
            .insert(artifact.artifact_ref, artifact.bytes)
            .is_some()
        {
            return Err(ScenarioError::DuplicateProvidedArtifactRef {
                path: format!("artifacts[{index}].artifactRef"),
                artifact_ref: artifact.artifact_ref.to_owned(),
            });
        }
    }
    Ok(by_ref)
}

fn verify_artifact<'a>(
    descriptor: ValidatedDescriptor<'_>,
    artifacts: &'a HashMap<&str, &'a [u8]>,
) -> Result<&'a [u8], ScenarioError> {
    let bytes = artifacts
        .get(descriptor.artifact_ref)
        .copied()
        .ok_or_else(|| ScenarioError::MissingArtifact {
            role: descriptor.role,
            artifact_ref: descriptor.artifact_ref.to_owned(),
        })?;
    let actual_size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if actual_size != descriptor.size {
        return Err(ScenarioError::ArtifactSizeMismatch {
            role: descriptor.role,
            artifact_ref: descriptor.artifact_ref.to_owned(),
            expected: descriptor.size,
            actual: actual_size,
        });
    }

    let actual_digest: [u8; 32] = Sha256::digest(bytes).into();
    if actual_digest != descriptor.digest {
        return Err(ScenarioError::ArtifactDigestMismatch {
            role: descriptor.role,
            artifact_ref: descriptor.artifact_ref.to_owned(),
            expected: encode_digest(&descriptor.digest),
            actual: encode_digest(&actual_digest),
        });
    }
    Ok(bytes)
}

fn load_spatial(
    input: &[u8],
    lane_graph: &LaneGraph,
) -> Result<LoadedSpatialPackage, ScenarioError> {
    let header: WireVersionHeader = deserialize_json(input, ScenarioDocument::Spatial)?;
    if header.format_version != CURRENT_SPATIAL_FORMAT_VERSION {
        return Err(ScenarioError::UnsupportedFormatVersion {
            document: ScenarioDocument::Spatial,
            expected: CURRENT_SPATIAL_FORMAT_VERSION,
            actual: header.format_version,
        });
    }

    let wire: WireSpatialPackage = deserialize_json(input, ScenarioDocument::Spatial)?;
    debug_assert_eq!(wire.format_version, header.format_version);
    normalize_spatial(wire, lane_graph)
}

fn normalize_spatial(
    wire: WireSpatialPackage,
    lane_graph: &LaneGraph,
) -> Result<LoadedSpatialPackage, ScenarioError> {
    let frame_id = CanonicalFrameId::try_new(wire.frame_id).map_err(|source| {
        ScenarioError::SpatialDomain {
            path: "frameId".to_owned(),
            source,
        }
    })?;
    let mut by_handle = HashMap::with_capacity(wire.edges.len());

    for (edge_index, wire_edge) in wire.edges.into_iter().enumerate() {
        let points_path = format!("edges[{edge_index}].centerline.points");
        if wire_edge.centerline.points.len() < MIN_CENTERLINE_POINT_COUNT {
            return Err(ScenarioError::InsufficientCenterlinePoints {
                path: points_path,
                min: MIN_CENTERLINE_POINT_COUNT,
                actual: wire_edge.centerline.points.len(),
            });
        }

        let mut points = Vec::with_capacity(wire_edge.centerline.points.len());
        for (point_index, point) in wire_edge.centerline.points.into_iter().enumerate() {
            let mut converted = [0.0_f32; 3];
            for (axis_index, value) in point.into_iter().enumerate() {
                let path =
                    format!("edges[{edge_index}].centerline.points[{point_index}][{axis_index}]");
                converted[axis_index] = checked_coordinate(value, path)?;
            }
            let point = CanonicalPoint3F32::try_new(converted[0], converted[1], converted[2])
                .map_err(|source| ScenarioError::SpatialDomain {
                    path: format!("edges[{edge_index}].centerline.points[{point_index}]"),
                    source,
                })?;
            points.push(point);
        }

        let edge_path = format!("edges[{edge_index}].trafficEdgeId");
        let edge = lane_graph
            .edge_handle(&wire_edge.traffic_edge_id)
            .ok_or_else(|| ScenarioError::UnknownTrafficEdge {
                path: edge_path.clone(),
                traffic_edge_id: wire_edge.traffic_edge_id.clone(),
            })?;
        if by_handle
            .insert(edge, LoadedSpatialEdge { edge, points })
            .is_some()
        {
            return Err(ScenarioError::DuplicateTrafficEdge {
                path: edge_path,
                traffic_edge_id: wire_edge.traffic_edge_id,
            });
        }
    }

    let mut edges = Vec::with_capacity(by_handle.len());
    for edge_definition in lane_graph.edges() {
        let edge = lane_graph
            .edge_handle(edge_definition.id())
            .expect("LaneGraph::edges must resolve through its own registry");
        let normalized =
            by_handle
                .remove(&edge)
                .ok_or_else(|| ScenarioError::MissingTrafficEdge {
                    path: "edges",
                    traffic_edge_id: edge_definition.id().to_owned(),
                })?;
        edges.push(normalized);
    }
    debug_assert!(by_handle.is_empty());

    Ok(LoadedSpatialPackage { frame_id, edges })
}

fn checked_coordinate(value: f64, path: String) -> Result<f32, ScenarioError> {
    if !value.is_finite() {
        return Err(ScenarioError::NonFiniteCoordinate { path, value });
    }
    let min = f64::from(CANONICAL_POINT_COMPONENT_MIN_METERS);
    let max = f64::from(CANONICAL_POINT_COMPONENT_MAX_METERS);
    if !(min..=max).contains(&value) {
        return Err(ScenarioError::CoordinateOutOfRange {
            path,
            value,
            min,
            max,
        });
    }

    Ok(value as f32)
}

fn deserialize_json<T>(input: &[u8], document: ScenarioDocument) -> Result<T, ScenarioError>
where
    T: DeserializeOwned,
{
    let mut deserializer = serde_json::Deserializer::from_slice(input);
    let value = serde_path_to_error::deserialize(&mut deserializer)
        .map_err(|error| ScenarioError::from_path_error(document, error))?;
    deserializer.end().map_err(|source| {
        ScenarioError::from_json_error(document, "$".to_owned(), source, Category::Syntax)
    })?;
    Ok(value)
}

fn parse_digest(path: &'static str, value: &str) -> Result<[u8; 32], ScenarioError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(ScenarioError::InvalidDigest {
            path,
            actual: value.to_owned(),
        });
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ScenarioError::InvalidDigest {
            path,
            actual: value.to_owned(),
        });
    }

    let mut digest = [0_u8; 32];
    for (index, chunk) in hex.as_bytes().chunks_exact(2).enumerate() {
        digest[index] = (hex_value(chunk[0]) << 4) | hex_value(chunk[1]);
    }
    Ok(digest)
}

fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => unreachable!("parse_digest validates lowercase hexadecimal input"),
    }
}

fn encode_digest(digest: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut encoded = String::with_capacity("sha256:".len() + digest.len() * 2);
    encoded.push_str("sha256:");
    for byte in digest {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}
