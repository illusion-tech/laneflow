//! 停止线、机动门、等待区、静态路线及其出现项视图。

use super::{
    CanonicalIdentityFieldView, CanonicalSignalControl, CanonicalStaticRouteOccurrenceRef,
    impl_stable_entity_view, occurrence_refs,
};
use crate::lir::{
    LirGateOccurrence, LirManeuverGate, LirManeuverOccurrence, LirSignalControl, LirStaticRoute,
    LirStopLine, LirUnit, LirWaitingZone, LirWaitingZoneOccurrence,
};
use laneflow_static_contract::{
    LaneEdgeOrdinal, ManeuverGateId, ManeuverGateOrdinal, ManeuverPathOrdinal, StaticRouteId,
    StaticRouteOrdinal, StopLineId, StopLineOrdinal, WaitingZoneId, WaitingZoneOrdinal,
};

impl_stable_entity_view!(
    CanonicalStopLineView,
    LirStopLine,
    StopLineOrdinal,
    StopLineId
);
impl_stable_entity_view!(
    CanonicalManeuverGateView,
    LirManeuverGate,
    ManeuverGateOrdinal,
    ManeuverGateId
);
impl_stable_entity_view!(
    CanonicalWaitingZoneView,
    LirWaitingZone,
    WaitingZoneOrdinal,
    WaitingZoneId
);

impl_stable_entity_view!(
    CanonicalStaticRouteView,
    LirStaticRoute,
    StaticRouteOrdinal,
    StaticRouteId
);

impl CanonicalStopLineView<'_> {
    /// 返回停止线所在的车道图边；位置语义固定为该边末端。
    #[must_use]
    pub const fn lane_edge(&self) -> LaneEdgeOrdinal {
        self.record.lane_edge
    }

    /// 返回引用该停止线的机动门，顺序按机动门规范身份冻结。
    #[must_use]
    pub fn maneuver_gates(&self) -> &[ManeuverGateOrdinal] {
        &self.lir.stop_line_maneuver_gates[self.record.maneuver_gates.as_usize_range()]
    }
}

impl CanonicalManeuverGateView<'_> {
    /// 返回唯一拥有本机动门的机动路径。
    #[must_use]
    pub const fn maneuver_path(&self) -> ManeuverPathOrdinal {
        self.record.maneuver_path
    }

    /// 返回路径边序列中受控转换的起始边下标。
    #[must_use]
    pub const fn transition_index(&self) -> u32 {
        self.record.transition_index
    }

    /// 返回位于转换起始边末端的停止线。
    #[must_use]
    pub const fn stop_line(&self) -> StopLineOrdinal {
        self.record.stop_line
    }

    /// 返回信号层控制绑定；`None` 不代表其他通行权约束已经放行。
    #[must_use]
    pub const fn signal_control(&self) -> CanonicalSignalControl {
        match self.record.signal_control {
            LirSignalControl::Group(group) => CanonicalSignalControl::Group(group),
            LirSignalControl::None => CanonicalSignalControl::None,
        }
    }

    /// 遍历匹配此机动门的静态路线门出现项。
    pub fn static_route_occurrences(
        &self,
    ) -> impl ExactSizeIterator<Item = CanonicalStaticRouteOccurrenceRef> + '_ {
        occurrence_refs(
            &self.lir.maneuver_gate_route_occurrences
                [self.record.static_route_occurrences.as_usize_range()],
        )
    }
}

impl CanonicalWaitingZoneView<'_> {
    /// 返回唯一拥有本等待区的机动路径。
    #[must_use]
    pub const fn maneuver_path(&self) -> ManeuverPathOrdinal {
        self.record.maneuver_path
    }

    /// 返回界定等待区起点的入口门。
    #[must_use]
    pub const fn entry_gate(&self) -> ManeuverGateOrdinal {
        self.record.entry_gate
    }

    /// 返回界定等待区终点的释放门。
    #[must_use]
    pub const fn release_gate(&self) -> ManeuverGateOrdinal {
        self.record.release_gate
    }

    /// 返回允许同时占用等待区的最大交通参与单元数；该值已证明大于零。
    #[must_use]
    pub const fn max_occupancy(&self) -> u32 {
        self.record.max_occupancy
    }

    /// 遍历匹配此等待区的静态路线等待区出现项。
    pub fn static_route_occurrences(
        &self,
    ) -> impl ExactSizeIterator<Item = CanonicalStaticRouteOccurrenceRef> + '_ {
        occurrence_refs(
            &self.lir.waiting_zone_route_occurrences
                [self.record.static_route_occurrences.as_usize_range()],
        )
    }
}

impl CanonicalStaticRouteView<'_> {
    /// 返回编制期权威有序车道图边序列；重复序号表示同一边的不同路线出现项。
    #[must_use]
    pub fn edges(&self) -> &[LaneEdgeOrdinal] {
        &self.lir.static_route_edges[self.record.edges.as_usize_range()]
    }

    /// 按相邻边转换顺序返回可选预编译机动门。
    pub fn transition_gates(
        &self,
    ) -> impl ExactSizeIterator<Item = Option<ManeuverGateOrdinal>> + '_ {
        self.lir.static_route_transitions[self.record.transitions.as_usize_range()]
            .iter()
            .map(|transition| transition.maneuver_gate)
    }

    /// 按入口路线边下标遍历完整机动路径出现项。
    pub fn maneuver_occurrences(
        &self,
    ) -> impl ExactSizeIterator<Item = CanonicalManeuverOccurrenceView<'_>> + '_ {
        let gate_start = self.record.gate_occurrences.start();
        let waiting_start = self.record.waiting_zone_occurrences.start();
        self.lir.maneuver_occurrences[self.record.maneuver_occurrences.as_usize_range()]
            .iter()
            .map(move |record| {
                CanonicalManeuverOccurrenceView::from_parts(record, gate_start, waiting_start)
            })
    }

    /// 按路线内出现顺序遍历机动门出现项。
    pub fn gate_occurrences(
        &self,
    ) -> impl ExactSizeIterator<Item = CanonicalGateOccurrenceView<'_>> + '_ {
        self.lir.gate_occurrences[self.record.gate_occurrences.as_usize_range()]
            .iter()
            .map(CanonicalGateOccurrenceView::from_record)
    }

    /// 按路线内出现顺序遍历等待区出现项。
    pub fn waiting_zone_occurrences(
        &self,
    ) -> impl ExactSizeIterator<Item = CanonicalWaitingZoneOccurrenceView<'_>> + '_ {
        self.lir.waiting_zone_occurrences[self.record.waiting_zone_occurrences.as_usize_range()]
            .iter()
            .map(CanonicalWaitingZoneOccurrenceView::from_record)
    }
}

/// 静态路线中一次完整 `ManeuverPath` 匹配的只读视图。
#[derive(Clone, Copy)]
pub struct CanonicalManeuverOccurrenceView<'a> {
    record: &'a LirManeuverOccurrence,
    route_gate_start: u32,
    route_waiting_start: u32,
}

impl<'a> CanonicalManeuverOccurrenceView<'a> {
    pub(in crate::compiler) const fn from_parts(
        record: &'a LirManeuverOccurrence,
        route_gate_start: u32,
        route_waiting_start: u32,
    ) -> Self {
        Self {
            record,
            route_gate_start,
            route_waiting_start,
        }
    }
}

impl CanonicalManeuverOccurrenceView<'_> {
    /// 返回本次完整匹配对应的规范机动路径。
    #[must_use]
    pub const fn maneuver_path(self) -> ManeuverPathOrdinal {
        self.record.maneuver_path
    }

    /// 返回机动入口边在所属静态路线边序列中的下标。
    #[must_use]
    pub const fn entry_route_edge_index(self) -> u32 {
        self.record.entry_route_edge_index
    }

    /// 返回机动出口边在所属静态路线边序列中的下标。
    #[must_use]
    pub const fn exit_route_edge_index(self) -> u32 {
        self.record.exit_route_edge_index
    }

    /// 返回该机动出现项在所属路线门出现项表中的半开区间。
    #[must_use]
    pub fn gate_occurrence_range(self) -> core::ops::Range<u32> {
        let start = self
            .record
            .gate_occurrences
            .start()
            .saturating_sub(self.route_gate_start);
        start..start.saturating_add(self.record.gate_occurrences.len())
    }

    /// 返回该机动出现项在所属路线等待区出现项表中的半开区间。
    #[must_use]
    pub fn waiting_zone_occurrence_range(self) -> core::ops::Range<u32> {
        let start = self
            .record
            .waiting_zone_occurrences
            .start()
            .saturating_sub(self.route_waiting_start);
        start..start.saturating_add(self.record.waiting_zone_occurrences.len())
    }
}

/// 静态路线中一次 `ManeuverGate` 匹配的只读视图。
#[derive(Clone, Copy)]
pub struct CanonicalGateOccurrenceView<'a> {
    record: &'a LirGateOccurrence,
}

impl<'a> CanonicalGateOccurrenceView<'a> {
    pub(in crate::compiler) const fn from_record(record: &'a LirGateOccurrence) -> Self {
        Self { record }
    }
}

impl CanonicalGateOccurrenceView<'_> {
    /// 返回本次出现对应的规范机动门。
    #[must_use]
    pub const fn maneuver_gate(self) -> ManeuverGateOrdinal {
        self.record.maneuver_gate
    }

    /// 返回所属静态路线的机动出现项下标。
    #[must_use]
    pub const fn maneuver_occurrence_index(self) -> u32 {
        self.record.maneuver_occurrence_index
    }

    /// 返回门所在转换的起始边在静态路线边序列中的下标。
    #[must_use]
    pub const fn from_route_edge_index(self) -> u32 {
        self.record.from_route_edge_index
    }

    /// 返回同一机动内的下一门出现项；最后一道门没有后继。
    #[must_use]
    pub const fn next_gate_occurrence_index(self) -> Option<u32> {
        self.record.next_gate_occurrence_index
    }

    /// 返回当前门之后首个边界边在静态路线边序列中的下标。
    #[must_use]
    pub const fn next_boundary_route_edge_index(self) -> u32 {
        self.record.next_boundary_route_edge_index
    }

    /// 返回从当前门进入的等待区出现项；不存在等待区时为 `None`。
    #[must_use]
    pub const fn waiting_zone_occurrence_index(self) -> Option<u32> {
        self.record.waiting_zone_occurrence_index
    }
}

/// 静态路线中一次 `WaitingZone` 匹配的只读视图。
#[derive(Clone, Copy)]
pub struct CanonicalWaitingZoneOccurrenceView<'a> {
    record: &'a LirWaitingZoneOccurrence,
}

impl<'a> CanonicalWaitingZoneOccurrenceView<'a> {
    pub(in crate::compiler) const fn from_record(record: &'a LirWaitingZoneOccurrence) -> Self {
        Self { record }
    }
}

impl CanonicalWaitingZoneOccurrenceView<'_> {
    /// 返回本次出现对应的规范等待区。
    #[must_use]
    pub const fn waiting_zone(self) -> WaitingZoneOrdinal {
        self.record.waiting_zone
    }

    /// 返回所属静态路线的机动出现项下标。
    #[must_use]
    pub const fn maneuver_occurrence_index(self) -> u32 {
        self.record.maneuver_occurrence_index
    }

    /// 返回进入等待区的门在所属静态路线门出现项表中的下标。
    #[must_use]
    pub const fn entry_gate_occurrence_index(self) -> u32 {
        self.record.entry_gate_occurrence_index
    }

    /// 返回释放等待区的门在所属静态路线门出现项表中的下标。
    #[must_use]
    pub const fn release_gate_occurrence_index(self) -> u32 {
        self.record.release_gate_occurrence_index
    }

    /// 返回进入等待区前的边在静态路线边序列中的下标。
    #[must_use]
    pub const fn entry_route_edge_index(self) -> u32 {
        self.record.entry_route_edge_index
    }

    /// 返回通过释放门后抵达的边在静态路线边序列中的下标。
    #[must_use]
    pub const fn release_route_edge_index(self) -> u32 {
        self.record.release_route_edge_index
    }
}
