//! ScenarioManifest 与 SpatialPackage 的内存加载和原子规范化。
//!
//! wire 校验、版本闸口、SHA-256 摘要与 Manifest 配对已由
//! `laneflow-current-source` 原子完成；本模块只保留 Traffic → Spatial 顺序的
//! current Core/Spatial 规范化。

use std::collections::HashMap;

use laneflow_core::{EdgeHandle, LaneGraph};
use laneflow_current_source::scenario_wire::{WireScenarioManifest, WireSpatialPackage};
use laneflow_current_source::wire::WirePackage;
use laneflow_current_source::{CurrentArtifactInput, validate_scenario_compatible};
use laneflow_spatial::{
    CANONICAL_POINT_COMPONENT_MAX_METERS, CANONICAL_POINT_COMPONENT_MIN_METERS, CanonicalFrameId,
    CanonicalPoint3F32,
};

use crate::scenario_error::ScenarioError;
use crate::{LoadedPackage, normalize};

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
    let inputs = artifacts
        .iter()
        .map(|artifact| CurrentArtifactInput::new(artifact.artifact_ref(), artifact.bytes(), None))
        .collect::<Vec<_>>();
    let (manifest, traffic_wire, spatial_wire) =
        validate_scenario_compatible(manifest_input, &inputs)
            .map_err(ScenarioError::from_current_source)?
            .into_parts()
            .into_documents();

    // source 原子成功后仍按 Traffic → Spatial 执行 current Core/Spatial 规范化；
    // Spatial 绑定依赖 Traffic 规范化产出的 LaneGraph。三份 owned DTO 各自在
    // 取出所需数据后随消费函数返回即释放，不整棵存活到规范化结束。
    let traffic = normalize_traffic(traffic_wire, into_traffic_artifact_ref(manifest))?;
    let spatial = normalize_spatial(spatial_wire, traffic.initial_traffic_data().lane_graph())?;

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

/// 取出 Traffic descriptor 的 owned `artifactRef`；Manifest DTO 随本函数返回即释放。
fn into_traffic_artifact_ref(manifest: WireScenarioManifest) -> String {
    manifest.traffic().artifact_ref().to_owned()
}

/// 规范化 Traffic wire；Traffic DTO 随本函数返回即释放。
fn normalize_traffic(
    traffic_wire: WirePackage,
    artifact_ref: String,
) -> Result<LoadedPackage, ScenarioError> {
    normalize(&traffic_wire).map_err(|source| ScenarioError::TrafficPackage {
        artifact_ref,
        source: Box::new(source),
    })
}

fn normalize_spatial(
    wire: WireSpatialPackage,
    lane_graph: &LaneGraph,
) -> Result<LoadedSpatialPackage, ScenarioError> {
    let frame_id = CanonicalFrameId::try_new(wire.frame_id().to_owned()).map_err(|source| {
        ScenarioError::SpatialDomain {
            path: "frameId".to_owned(),
            source,
        }
    })?;
    let wire_edges = wire.into_edges();
    let mut by_handle = HashMap::with_capacity(wire_edges.len());

    for (edge_index, wire_edge) in wire_edges.into_iter().enumerate() {
        let (traffic_edge_id, wire_points) = wire_edge.into_parts();
        let points_path = format!("edges[{edge_index}].centerline.points");
        if wire_points.len() < MIN_CENTERLINE_POINT_COUNT {
            return Err(ScenarioError::InsufficientCenterlinePoints {
                path: points_path,
                min: MIN_CENTERLINE_POINT_COUNT,
                actual: wire_points.len(),
            });
        }

        let mut points = Vec::with_capacity(wire_points.len());
        for (point_index, point) in wire_points.into_iter().enumerate() {
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
        let edge = lane_graph.edge_handle(&traffic_edge_id).ok_or_else(|| {
            ScenarioError::UnknownTrafficEdge {
                path: edge_path.clone(),
                traffic_edge_id: traffic_edge_id.clone(),
            }
        })?;
        if by_handle
            .insert(edge, LoadedSpatialEdge { edge, points })
            .is_some()
        {
            return Err(ScenarioError::DuplicateTrafficEdge {
                path: edge_path,
                traffic_edge_id,
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
