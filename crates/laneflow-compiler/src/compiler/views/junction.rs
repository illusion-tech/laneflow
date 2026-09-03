//! 路口、转向动作、机动路径与派生内部边视图。

use super::{CanonicalIdentityFieldView, impl_stable_entity_view};
use crate::lir::{LirJunction, LirJunctionInternalEdge, LirManeuverPath, LirMovement, LirUnit};
use laneflow_static_contract::{
    JunctionId, JunctionOrdinal, LaneEdgeOrdinal, ManeuverGateOrdinal, ManeuverPathId,
    ManeuverPathOrdinal, MovementId, MovementOrdinal, WaitingZoneOrdinal,
};

impl_stable_entity_view!(
    CanonicalJunctionView,
    LirJunction,
    JunctionOrdinal,
    JunctionId
);
impl_stable_entity_view!(
    CanonicalMovementView,
    LirMovement,
    MovementOrdinal,
    MovementId
);
impl_stable_entity_view!(
    CanonicalManeuverPathView,
    LirManeuverPath,
    ManeuverPathOrdinal,
    ManeuverPathId
);

impl CanonicalJunctionView<'_> {
    /// 返回本路口拥有的非空转向动作集合。
    #[must_use]
    pub fn movements(&self) -> &[MovementOrdinal] {
        &self.lir.junction_movements[self.record.movements.as_usize_range()]
    }
}

impl CanonicalMovementView<'_> {
    /// 返回来源显式声明的方向；缺失不代表直行。
    #[must_use]
    pub const fn turn_direction(&self) -> Option<crate::ManeuverDirection> {
        self.record.turn_direction
    }

    /// 返回唯一拥有本转向动作的路口。
    #[must_use]
    pub const fn junction(&self) -> JunctionOrdinal {
        self.record.junction
    }

    /// 返回参与 Identity v1 的有向入口接近键；该键由编制端显式提供，编译器不从几何推断。
    #[must_use]
    pub fn directed_entry_approach_key(&self) -> &str {
        &self.record.directed_entry_approach_key
    }

    /// 返回参与 Identity v1 的有向出口接近键；该键由编制端显式提供，编译器不从几何推断。
    #[must_use]
    pub fn directed_exit_approach_key(&self) -> &str {
        &self.record.directed_exit_approach_key
    }

    /// 返回本转向动作拥有的非空机动路径集合。
    #[must_use]
    pub fn maneuver_paths(&self) -> &[ManeuverPathOrdinal] {
        &self.lir.movement_maneuver_paths[self.record.maneuver_paths.as_usize_range()]
    }
}

impl CanonicalManeuverPathView<'_> {
    /// 返回唯一拥有本机动路径的转向动作。
    #[must_use]
    pub const fn movement(&self) -> MovementOrdinal {
        self.record.movement
    }

    /// 返回完整且已验证直接连通的 `entry + internal + exit` 车道图边序列。
    #[must_use]
    pub fn edges(&self) -> &[LaneEdgeOrdinal] {
        &self.lir.maneuver_path_edges[self.record.edges.as_usize_range()]
    }

    /// 返回完整路径序列的入口边。
    #[must_use]
    pub fn entry_edge(&self) -> LaneEdgeOrdinal {
        self.edges()[0]
    }

    /// 返回完整路径序列中可为空的内部边切片。
    #[must_use]
    pub fn internal_edges(&self) -> &[LaneEdgeOrdinal] {
        let edges = self.edges();
        &edges[1..edges.len() - 1]
    }

    /// 返回完整路径序列的出口边。
    #[must_use]
    pub fn exit_edge(&self) -> LaneEdgeOrdinal {
        let edges = self.edges();
        edges[edges.len() - 1]
    }

    /// 返回按 `transition_index` 严格递增冻结的机动门序号。
    #[must_use]
    pub fn maneuver_gates(&self) -> &[ManeuverGateOrdinal] {
        &self.lir.maneuver_path_gates[self.record.maneuver_gates.as_usize_range()]
    }

    /// 返回按入口转换、释放转换和稳定身份冻结的等待区序号。
    #[must_use]
    pub fn waiting_zones(&self) -> &[WaitingZoneOrdinal] {
        &self.lir.maneuver_path_waiting_zones[self.record.waiting_zones.as_usize_range()]
    }
}

/// Canonical LIR 中一条派生路口内部边所有权的借用视图。
#[derive(Clone, Copy)]
pub struct CanonicalJunctionInternalEdgeView<'a> {
    record: &'a LirJunctionInternalEdge,
}

impl<'a> CanonicalJunctionInternalEdgeView<'a> {
    pub(in crate::compiler) const fn from_record(record: &'a LirJunctionInternalEdge) -> Self {
        Self { record }
    }
}

impl CanonicalJunctionInternalEdgeView<'_> {
    /// 返回承担路口内部角色的车道图边。
    #[must_use]
    pub const fn edge(&self) -> LaneEdgeOrdinal {
        self.record.edge
    }

    /// 返回该内部边的唯一所有者路口。
    #[must_use]
    pub const fn junction(&self) -> JunctionOrdinal {
        self.record.junction
    }
}
