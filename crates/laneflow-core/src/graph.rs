//! Lane graph 输入、handle registry 与 resolved traversal 原语。

use indexmap::{IndexMap, IndexSet};

use crate::{error::CoreError, handle::EdgeHandle, id::validate_external_id};

/// edge boundary 与最小 edge length 校验使用的统一 epsilon。
pub const EDGE_BOUNDARY_EPSILON: f64 = 1.0e-9;

/// lane edge 的长度，单位为 engine-agnostic distance unit。
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct EdgeLength(f64);

impl EdgeLength {
    /// 创建经过校验的 edge length。
    pub fn try_new(value: f64) -> Result<Self, CoreError> {
        if !value.is_finite() || value <= EDGE_BOUNDARY_EPSILON {
            return Err(CoreError::InvalidLaneEdgeLength {
                edge_length: value,
                min_exclusive: EDGE_BOUNDARY_EPSILON,
            });
        }

        Ok(Self(value))
    }

    /// 返回底层数值。
    pub const fn value(self) -> f64 {
        self.0
    }
}

/// lane edge 输入定义。
#[derive(Clone, Debug, PartialEq)]
pub struct LaneEdge {
    id: String,
    length: EdgeLength,
    next_edge_ids: Vec<String>,
}

impl LaneEdge {
    /// 创建 lane edge。跨 edge 引用由 `LaneGraph::try_new` 校验。
    pub fn new<I, S>(id: impl Into<String>, length: EdgeLength, next_edge_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            id: id.into(),
            length,
            next_edge_ids: next_edge_ids.into_iter().map(Into::into).collect(),
        }
    }

    /// 返回 lane edge id。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 返回 lane edge length。
    pub const fn length(&self) -> EdgeLength {
        self.length
    }

    /// 返回可连接的 next edge id 列表。
    pub fn next_edge_ids(&self) -> &[String] {
        &self.next_edge_ids
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ResolvedLaneEdge {
    definition: LaneEdge,
    next_edges: Vec<EdgeHandle>,
}

/// Lane graph runtime registry。
#[derive(Clone, Debug, PartialEq)]
pub struct LaneGraph {
    edges: Vec<ResolvedLaneEdge>,
    edge_handles: IndexMap<String, EdgeHandle>,
}

impl LaneGraph {
    /// 创建空 lane graph。
    pub fn empty() -> Self {
        Self {
            edges: Vec::new(),
            edge_handles: IndexMap::new(),
        }
    }

    /// 创建并校验 lane graph。
    pub fn try_new<I>(edges: I) -> Result<Self, CoreError>
    where
        I: IntoIterator<Item = LaneEdge>,
    {
        let mut edge_handles = IndexMap::new();
        let mut edge_definitions = Vec::new();

        for edge in edges {
            validate_external_id("laneGraph.edges[].id", edge.id())?;
            if edge_handles.contains_key(edge.id()) {
                return Err(CoreError::DuplicateLaneEdgeId {
                    edge_id: edge.id().to_owned(),
                });
            }
            let edge_id = edge.id().to_owned();
            let handle = EdgeHandle::new(edge_definitions.len());
            edge_handles.insert(edge_id, handle);
            edge_definitions.push(edge);
        }

        let mut resolved_edges = Vec::with_capacity(edge_definitions.len());
        for edge in edge_definitions {
            let mut connection_targets = IndexSet::new();
            let mut next_edges = Vec::with_capacity(edge.next_edge_ids.len());
            for next_edge_id in &edge.next_edge_ids {
                validate_external_id("laneGraph.edges[].connections[].to", next_edge_id)?;
                if !connection_targets.insert(next_edge_id.as_str()) {
                    return Err(CoreError::DuplicateLaneEdgeConnection {
                        edge_id: edge.id.clone(),
                        next_edge_id: next_edge_id.clone(),
                    });
                }
                let next_edge = edge_handles.get(next_edge_id).copied().ok_or_else(|| {
                    CoreError::UnknownNextLaneEdge {
                        edge_id: edge.id.clone(),
                        next_edge_id: next_edge_id.clone(),
                    }
                })?;
                next_edges.push(next_edge);
            }

            resolved_edges.push(ResolvedLaneEdge {
                definition: edge,
                next_edges,
            });
        }

        Ok(Self {
            edges: resolved_edges,
            edge_handles,
        })
    }

    /// 返回指定 external ID 的 edge handle。
    pub fn edge_handle(&self, id: &str) -> Option<EdgeHandle> {
        self.edge_handles.get(id).copied()
    }

    /// 返回 edge handle 对应的 external ID。
    pub fn edge_external_id(&self, handle: EdgeHandle) -> Option<&str> {
        self.edge(handle).map(LaneEdge::id)
    }

    /// 返回指定 lane edge。
    pub fn edge(&self, handle: EdgeHandle) -> Option<&LaneEdge> {
        self.edges.get(handle.index()).map(|edge| &edge.definition)
    }

    /// 返回指定 external ID 的 lane edge。
    pub fn edge_by_id(&self, id: &str) -> Option<&LaneEdge> {
        self.edge_handle(id).and_then(|handle| self.edge(handle))
    }

    /// 返回 graph 是否允许从 `from` 连接到 `to`。
    pub fn can_traverse(&self, from: EdgeHandle, to: EdgeHandle) -> bool {
        self.edges
            .get(from.index())
            .is_some_and(|edge| edge.next_edges.contains(&to))
    }

    /// 返回 graph 是否允许从 `from` external ID 连接到 `to` external ID。
    pub fn can_traverse_by_id(&self, from: &str, to: &str) -> bool {
        let Some(from) = self.edge_handle(from) else {
            return false;
        };
        let Some(to) = self.edge_handle(to) else {
            return false;
        };

        self.can_traverse(from, to)
    }

    /// 返回指定 edge 的长度。
    pub fn edge_length(&self, handle: EdgeHandle) -> Option<EdgeLength> {
        self.edge(handle).map(LaneEdge::length)
    }

    /// 返回指定 external ID 的 edge 长度。
    pub fn edge_length_by_id(&self, id: &str) -> Option<EdgeLength> {
        self.edge_by_id(id).map(LaneEdge::length)
    }

    /// 返回指定 edge 的 outgoing edge handle 列表。
    pub fn next_edges(&self, handle: EdgeHandle) -> Option<&[EdgeHandle]> {
        self.edges
            .get(handle.index())
            .map(|edge| edge.next_edges.as_slice())
    }

    /// 返回所有 lane edge，顺序与初始化输入一致。
    pub fn edges(&self) -> impl ExactSizeIterator<Item = &LaneEdge> {
        self.edges.iter().map(|edge| &edge.definition)
    }
}
