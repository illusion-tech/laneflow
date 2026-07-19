//! Core edge handle 到 Spatial 条目的 immutable registry 边界。

use std::{collections::HashMap, fmt};

use laneflow_core::{EdgeHandle, LaneGraph};

use crate::{CanonicalFrameId, SpatialError};

const MAX_REGISTRY_ENTRIES: usize = u32::MAX as usize;

/// 已完整绑定到一个 lane graph 的 opaque immutable Spatial registry。
///
/// 当前 #133 只冻结 frame identity、LaneGraph edge 顺序和私有 lookup；真实折线条目与
/// 公开构造入口由 #135 在拥有完整绑定语义后加入。
#[derive(Clone, PartialEq, Eq)]
pub struct SpatialRegistry {
    frame_id: CanonicalFrameId,
    edge_handles: Vec<EdgeHandle>,
    edge_slots: HashMap<EdgeHandle, u32>,
}

impl fmt::Debug for SpatialRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SpatialRegistry")
            .field("frame_id", &self.frame_id)
            .field("edge_handles", &self.edge_handles)
            .finish_non_exhaustive()
    }
}

impl SpatialRegistry {
    /// 返回本 registry 唯一的标准坐标框架 ID。
    pub const fn frame_id(&self) -> &CanonicalFrameId {
        &self.frame_id
    }

    /// 返回完整绑定的 edge 数量。
    pub const fn len(&self) -> usize {
        self.edge_handles.len()
    }

    /// 返回 registry 是否绑定空 lane graph。
    pub const fn is_empty(&self) -> bool {
        self.edge_handles.is_empty()
    }

    /// 返回目标 Core edge 是否存在已提交绑定。
    pub fn contains_edge(&self, edge: EdgeHandle) -> bool {
        self.edge_slots.contains_key(&edge)
    }

    /// 按 `LaneGraph::edges()` 的稳定顺序返回全部 Core edge handle。
    pub fn edge_handles(&self) -> impl ExactSizeIterator<Item = EdgeHandle> + '_ {
        self.edge_handles.iter().copied()
    }

    #[allow(dead_code, reason = "#135 将使用私有 dense slot 解析折线条目")]
    pub(crate) fn edge_slot(&self, edge: EdgeHandle) -> Option<u32> {
        self.edge_slots.get(&edge).copied()
    }
}

/// crate-private staged construction；公共 bound constructor 由 #134/#135 交付。
#[allow(dead_code, reason = "#134/#135 将在完整输入边界调用 staged builder")]
pub(crate) struct SpatialRegistryBuilder {
    frame_id: CanonicalFrameId,
    edge_handles: Vec<EdgeHandle>,
}

#[allow(dead_code, reason = "#134/#135 将在完整输入边界调用 staged builder")]
impl SpatialRegistryBuilder {
    pub(crate) fn new(frame_id: CanonicalFrameId) -> Self {
        Self {
            frame_id,
            edge_handles: Vec::new(),
        }
    }

    pub(crate) fn push_edge(&mut self, edge: EdgeHandle) {
        self.edge_handles.push(edge);
    }

    /// 只有所有 handle、完整覆盖和容量校验成功后才提交 immutable registry。
    pub(crate) fn commit(self, lane_graph: &LaneGraph) -> Result<SpatialRegistry, SpatialError> {
        validate_registry_capacity(self.edge_handles.len())?;

        let mut staged_edges = HashMap::with_capacity(self.edge_handles.len());
        for edge in self.edge_handles.iter().copied() {
            if lane_graph.edge(edge).is_none() {
                return Err(SpatialError::UnknownEdgeHandle { edge });
            }
            if staged_edges.insert(edge, ()).is_some() {
                return Err(SpatialError::DuplicateEdgeBinding { edge });
            }
        }

        let mut edge_handles = Vec::with_capacity(self.edge_handles.len());
        for edge_definition in lane_graph.edges() {
            let edge = lane_graph
                .edge_handle(edge_definition.id())
                .expect("LaneGraph::edges must resolve through its own registry");
            if !staged_edges.contains_key(&edge) {
                return Err(SpatialError::MissingEdgeBinding { edge });
            }
            edge_handles.push(edge);
        }

        let mut edge_slots = HashMap::with_capacity(edge_handles.len());
        for (index, edge) in edge_handles.iter().copied().enumerate() {
            let slot =
                u32::try_from(index).map_err(|_| registry_capacity_error(edge_handles.len()))?;
            edge_slots.insert(edge, slot);
        }

        Ok(SpatialRegistry {
            frame_id: self.frame_id,
            edge_handles,
            edge_slots,
        })
    }
}

fn validate_registry_capacity(actual: usize) -> Result<(), SpatialError> {
    if actual > MAX_REGISTRY_ENTRIES {
        return Err(registry_capacity_error(actual));
    }

    Ok(())
}

fn registry_capacity_error(actual: usize) -> SpatialError {
    SpatialError::RegistryCapacityExceeded {
        actual,
        max: MAX_REGISTRY_ENTRIES,
    }
}

#[cfg(test)]
mod tests {
    use laneflow_core::{EdgeLength, LaneEdge};

    use super::*;

    fn edge_length(value: f64) -> EdgeLength {
        EdgeLength::try_new(value).expect("valid edge length")
    }

    fn graph(ids: &[&str]) -> LaneGraph {
        LaneGraph::try_new(
            ids.iter()
                .map(|id| LaneEdge::new(*id, edge_length(1.0), std::iter::empty::<&str>())),
        )
        .expect("valid lane graph")
    }

    fn frame_id() -> CanonicalFrameId {
        CanonicalFrameId::try_new("campus/main").expect("valid frame ID")
    }

    fn builder_with_edges(edges: impl IntoIterator<Item = EdgeHandle>) -> SpatialRegistryBuilder {
        let mut builder = SpatialRegistryBuilder::new(frame_id());
        for edge in edges {
            builder.push_edge(edge);
        }
        builder
    }

    #[test]
    fn empty_lane_graph_commits_empty_registry() {
        let lane_graph = LaneGraph::empty();

        let registry = SpatialRegistryBuilder::new(frame_id())
            .commit(&lane_graph)
            .expect("empty graph has complete empty bindings");

        assert_eq!(registry.frame_id().as_str(), "campus/main");
        assert!(registry.is_empty());
        assert_eq!(registry.edge_handles().len(), 0);
    }

    #[test]
    fn registry_uses_lane_graph_order_and_private_dense_lookup() {
        let lane_graph = graph(&["A", "B", "C"]);
        let edge_a = lane_graph.edge_handle("A").expect("edge A");
        let edge_b = lane_graph.edge_handle("B").expect("edge B");
        let edge_c = lane_graph.edge_handle("C").expect("edge C");

        let registry = builder_with_edges([edge_c, edge_a, edge_b])
            .commit(&lane_graph)
            .expect("complete bindings");

        assert_eq!(
            registry.edge_handles().collect::<Vec<_>>(),
            [edge_a, edge_b, edge_c]
        );
        assert_eq!(registry.edge_slot(edge_a), Some(0));
        assert_eq!(registry.edge_slot(edge_b), Some(1));
        assert_eq!(registry.edge_slot(edge_c), Some(2));
        assert!(registry.contains_edge(edge_a));
        assert!(!format!("{registry:?}").contains("edge_slots"));
    }

    #[test]
    fn registry_rejects_unknown_handle_before_commit() {
        let lane_graph = graph(&["A"]);
        let foreign_graph = graph(&["foreign-A", "foreign-B"]);
        let unknown = foreign_graph
            .edge_handle("foreign-B")
            .expect("foreign edge");
        let before = lane_graph.clone();

        let error = builder_with_edges([unknown, unknown])
            .commit(&lane_graph)
            .expect_err("unknown must win before duplicate");

        assert_eq!(error, SpatialError::UnknownEdgeHandle { edge: unknown });
        assert_eq!(lane_graph, before);
    }

    #[test]
    fn same_ordinal_wrong_world_handle_remains_deliberately_indistinguishable() {
        let lane_graph = graph(&["A"]);
        let foreign_graph = graph(&["foreign-A"]);
        let local = lane_graph.edge_handle("A").expect("local edge");
        let foreign = foreign_graph
            .edge_handle("foreign-A")
            .expect("foreign edge");

        assert_eq!(foreign, local);
        let registry = builder_with_edges([foreign])
            .commit(&lane_graph)
            .expect("opaque handles do not carry world identity");
        assert!(registry.contains_edge(local));
    }

    #[test]
    fn registry_rejects_duplicate_handle_without_partial_result() {
        let lane_graph = graph(&["A", "B"]);
        let edge_a = lane_graph.edge_handle("A").expect("edge A");
        let before = lane_graph.clone();

        let error = builder_with_edges([edge_a, edge_a])
            .commit(&lane_graph)
            .expect_err("duplicate handle must fail");

        assert_eq!(error, SpatialError::DuplicateEdgeBinding { edge: edge_a });
        assert_eq!(lane_graph, before);
    }

    #[test]
    fn registry_rejects_first_missing_handle_in_lane_graph_order() {
        let lane_graph = graph(&["A", "B"]);
        let edge_a = lane_graph.edge_handle("A").expect("edge A");
        let edge_b = lane_graph.edge_handle("B").expect("edge B");
        let before = lane_graph.clone();

        let error = builder_with_edges([edge_b])
            .commit(&lane_graph)
            .expect_err("missing edge must fail");

        assert_eq!(error, SpatialError::MissingEdgeBinding { edge: edge_a });
        assert_eq!(lane_graph, before);
    }

    #[test]
    fn registry_capacity_check_covers_u32_boundary_without_allocating() {
        assert_eq!(validate_registry_capacity(u32::MAX as usize), Ok(()));

        if usize::BITS > u32::BITS {
            let actual = u32::MAX as usize + 1;
            assert_eq!(
                validate_registry_capacity(actual),
                Err(SpatialError::RegistryCapacityExceeded {
                    actual,
                    max: u32::MAX as usize,
                })
            );
        }
    }
}
