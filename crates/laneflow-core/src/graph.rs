//! v0.1 最小 lane graph traversal 原语。

use indexmap::IndexMap;

use crate::error::CoreError;

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

/// v0.1 最小 lane edge。
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

/// v0.1 最小 lane graph。
#[derive(Clone, Debug, PartialEq)]
pub struct LaneGraph {
    edges: IndexMap<String, LaneEdge>,
}

impl LaneGraph {
    /// 创建空 lane graph。
    pub fn empty() -> Self {
        Self {
            edges: IndexMap::new(),
        }
    }

    /// 创建并校验 lane graph。
    pub fn try_new<I>(edges: I) -> Result<Self, CoreError>
    where
        I: IntoIterator<Item = LaneEdge>,
    {
        let mut edge_map = IndexMap::new();

        for edge in edges {
            let edge_id = edge.id.clone();
            if edge_map.contains_key(&edge_id) {
                return Err(CoreError::DuplicateLaneEdgeId { edge_id });
            }
            edge_map.insert(edge_id, edge);
        }

        for edge in edge_map.values() {
            for next_edge_id in &edge.next_edge_ids {
                if !edge_map.contains_key(next_edge_id) {
                    return Err(CoreError::UnknownNextLaneEdge {
                        edge_id: edge.id.clone(),
                        next_edge_id: next_edge_id.clone(),
                    });
                }
            }
        }

        Ok(Self { edges: edge_map })
    }

    /// 返回指定 lane edge。
    pub fn edge(&self, id: &str) -> Option<&LaneEdge> {
        self.edges.get(id)
    }

    /// 返回 graph 是否允许从 `from` 连接到 `to`。
    pub fn can_traverse(&self, from: &str, to: &str) -> bool {
        self.edge(from)
            .is_some_and(|edge| edge.next_edge_ids.iter().any(|edge_id| edge_id == to))
    }

    /// 返回指定 edge 的长度。
    pub fn edge_length(&self, id: &str) -> Option<EdgeLength> {
        self.edge(id).map(LaneEdge::length)
    }

    /// 返回所有 lane edge，顺序与初始化输入一致。
    pub fn edges(&self) -> impl ExactSizeIterator<Item = &LaneEdge> {
        self.edges.values()
    }
}
