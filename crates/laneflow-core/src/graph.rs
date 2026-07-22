//! Lane graph 输入、handle registry 与 resolved traversal 原语。

use indexmap::{IndexMap, IndexSet};

use crate::{
    error::CoreError, handle::EdgeHandle, id::validate_external_id,
    numeric_policy::MIN_EDGE_LENGTH_EXCLUSIVE_METERS,
};

/// lane edge 的长度，单位为 engine-agnostic distance unit。
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct EdgeLength(f64);

impl EdgeLength {
    /// 创建经过校验的 edge length。
    pub fn try_new(value: f64) -> Result<Self, CoreError> {
        if !value.is_finite() || value <= MIN_EDGE_LENGTH_EXCLUSIVE_METERS {
            return Err(CoreError::InvalidLaneEdgeLength {
                edge_length: value,
                min_exclusive: MIN_EDGE_LENGTH_EXCLUSIVE_METERS,
            });
        }

        Ok(Self(value))
    }

    /// 返回底层数值。
    pub const fn value(self) -> f64 {
        self.0
    }
}

/// lane edge 的基础道路限速，单位为 m/s。
///
/// 该值描述 immutable 道路事实；运行时纵向策略可以基于它派生 effective speed
/// ceiling，但不得修改该基础值。
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct SpeedLimit(f64);

impl SpeedLimit {
    /// 创建经过校验的基础道路限速。
    pub fn try_new(value: f64) -> Result<Self, CoreError> {
        if !value.is_finite() || value <= 0.0 {
            return Err(CoreError::InvalidSpeedLimit { speed_limit: value });
        }

        Ok(Self(value))
    }

    /// 返回底层 m/s 数值。
    pub const fn value(self) -> f64 {
        self.0
    }
}

/// lane edge 输入定义。
#[derive(Clone, Debug, PartialEq)]
pub struct LaneEdge {
    id: String,
    length: EdgeLength,
    speed_limit: SpeedLimit,
    next_edge_ids: Vec<String>,
}

impl LaneEdge {
    /// 创建 lane edge。跨 edge 引用由 `LaneGraph::try_new` 校验。
    pub fn new<I, S>(
        id: impl Into<String>,
        length: EdgeLength,
        speed_limit: SpeedLimit,
        next_edge_ids: I,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            id: id.into(),
            length,
            speed_limit,
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

    /// 返回 lane edge 的基础道路限速。
    pub const fn speed_limit(&self) -> SpeedLimit {
        self.speed_limit
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
                validate_external_id("laneGraph.edges[].connections[].toEdgeId", next_edge_id)?;
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

    /// 返回指定 edge 的基础道路限速。
    pub fn edge_speed_limit(&self, handle: EdgeHandle) -> Option<SpeedLimit> {
        self.edge(handle).map(LaneEdge::speed_limit)
    }

    /// 返回指定 external ID 的 edge 基础道路限速。
    pub fn edge_speed_limit_by_id(&self, id: &str) -> Option<SpeedLimit> {
        self.edge_by_id(id).map(LaneEdge::speed_limit)
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

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        let edge_bytes = self.edges.capacity() * std::mem::size_of::<ResolvedLaneEdge>()
            + self
                .edges
                .iter()
                .map(|edge| {
                    edge.definition.id.capacity()
                        + edge.definition.next_edge_ids.capacity() * std::mem::size_of::<String>()
                        + edge
                            .definition
                            .next_edge_ids
                            .iter()
                            .map(String::capacity)
                            .sum::<usize>()
                        + edge.next_edges.capacity() * std::mem::size_of::<EdgeHandle>()
                })
                .sum::<usize>();
        let handle_bytes = self.edge_handles.capacity()
            * std::mem::size_of::<(String, EdgeHandle)>()
            + self
                .edge_handles
                .keys()
                .map(String::capacity)
                .sum::<usize>();
        edge_bytes + handle_bytes
    }
}
