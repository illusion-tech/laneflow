//! 官方来源编译到原子已验证输出的公共入口。
//!
//! [`Compiler::compile`] 是唯一能够构造 [`ValidatedCanonicalLir`]、
//! [`ValidatedSourceMapInput`] 和 [`CompilationOutput`] 的路径。当前实现是干净单工作线程
//! 确定性预言机：每个阶段成功后才提交下一阶段，任一错误只返回
//! [`DiagnosticBundle`]；来源伴随数据在 AST/HIR/MIR 释放前冻结。

use laneflow_static_contract::{
    AuthoringLaneId, AuthoringLaneOrdinal, FacilityBandId, FacilityBandOrdinal, FieldTag,
    JunctionId, JunctionOrdinal, LaneEdgeId, LaneEdgeOrdinal, LaneGroupId, LaneGroupOrdinal,
    ManeuverGateId, ManeuverGateOrdinal, ManeuverPathId, ManeuverPathOrdinal, MovementId,
    MovementOrdinal, ParkingAreaId, ParkingAreaOrdinal, ParkingSpaceId, ParkingSpaceOrdinal,
    RoadCorridorId, RoadCorridorOrdinal, RoadSectionId, RoadSectionOrdinal, SignalAspect,
    SignalControllerId, SignalControllerOrdinal, SignalGroupId, SignalGroupOrdinal, SignalPhaseId,
    SignalPhaseOrdinal, StaticRouteId, StaticRouteOrdinal, StopLineId, StopLineOrdinal,
    WaitingZoneId, WaitingZoneOrdinal,
};

use crate::hir::build_hir;
use crate::lir::{
    LirAuthoringLane, LirCorridorElement, LirFacilityBand, LirGateOccurrence, LirIdentityField,
    LirJunction, LirJunctionInternalEdge, LirLaneEdge, LirLaneGroup, LirManeuverGate,
    LirManeuverOccurrence, LirManeuverPath, LirMovement, LirParkingArea, LirParkingSpace,
    LirRoadCorridor, LirRoadSection, LirRouteOccurrenceRef, LirSignalControl, LirSignalController,
    LirSignalGroup, LirSignalPhase, LirSignalPhaseState, LirStaticRoute, LirStopLine, LirUnit,
    LirWaitingZone, LirWaitingZoneOccurrence, freeze_lir,
};
use crate::mir::lower_to_mir;
use crate::source_map::{ValidatedSourceMapInput, freeze_source_map};
use crate::{CompilationUnit, Diagnostic, DiagnosticBundle};

/// 已完成 #292 当前支持子集全部静态语义验证的 Canonical LIR。
///
/// 字段保持私有，调用方只能按规范稳定顺序读取有类型表、身份字段和关系区间。不存在从
/// 裸表、未验证 MIR 或自报状态构造本类型的入口。
pub struct ValidatedCanonicalLir {
    inner: LirUnit,
}

impl ValidatedCanonicalLir {
    /// 按完整 Identity v1 前像规范顺序遍历全部车道图边。
    pub fn lane_edges(&self) -> impl ExactSizeIterator<Item = CanonicalLaneEdgeView<'_>> {
        self.inner
            .lane_edges
            .iter()
            .map(|edge| CanonicalLaneEdgeView {
                lir: &self.inner,
                edge,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取车道图边。
    ///
    /// 序号来自其他编译结果时可能命中错误实体；跨编译关联必须先使用 `LaneEdgeId`。
    #[must_use]
    pub fn lane_edge(&self, ordinal: LaneEdgeOrdinal) -> Option<CanonicalLaneEdgeView<'_>> {
        self.inner
            .lane_edges
            .get(ordinal.index())
            .map(|edge| CanonicalLaneEdgeView {
                lir: &self.inner,
                edge,
            })
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部道路走廊。
    pub fn road_corridors(&self) -> impl ExactSizeIterator<Item = CanonicalRoadCorridorView<'_>> {
        self.inner
            .road_corridors
            .iter()
            .map(|record| CanonicalRoadCorridorView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取道路走廊。
    #[must_use]
    pub fn road_corridor(
        &self,
        ordinal: RoadCorridorOrdinal,
    ) -> Option<CanonicalRoadCorridorView<'_>> {
        self.inner
            .road_corridors
            .get(ordinal.index())
            .map(|record| CanonicalRoadCorridorView {
                lir: &self.inner,
                record,
            })
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部道路区段。
    pub fn road_sections(&self) -> impl ExactSizeIterator<Item = CanonicalRoadSectionView<'_>> {
        self.inner
            .road_sections
            .iter()
            .map(|record| CanonicalRoadSectionView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取道路区段。
    #[must_use]
    pub fn road_section(
        &self,
        ordinal: RoadSectionOrdinal,
    ) -> Option<CanonicalRoadSectionView<'_>> {
        self.inner
            .road_sections
            .get(ordinal.index())
            .map(|record| CanonicalRoadSectionView {
                lir: &self.inner,
                record,
            })
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部编制车道。
    pub fn authoring_lanes(&self) -> impl ExactSizeIterator<Item = CanonicalAuthoringLaneView<'_>> {
        self.inner
            .authoring_lanes
            .iter()
            .map(|record| CanonicalAuthoringLaneView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取编制车道。
    #[must_use]
    pub fn authoring_lane(
        &self,
        ordinal: AuthoringLaneOrdinal,
    ) -> Option<CanonicalAuthoringLaneView<'_>> {
        self.inner
            .authoring_lanes
            .get(ordinal.index())
            .map(|record| CanonicalAuthoringLaneView {
                lir: &self.inner,
                record,
            })
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部车道组。
    pub fn lane_groups(&self) -> impl ExactSizeIterator<Item = CanonicalLaneGroupView<'_>> {
        self.inner
            .lane_groups
            .iter()
            .map(|record| CanonicalLaneGroupView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取车道组。
    #[must_use]
    pub fn lane_group(&self, ordinal: LaneGroupOrdinal) -> Option<CanonicalLaneGroupView<'_>> {
        self.inner
            .lane_groups
            .get(ordinal.index())
            .map(|record| CanonicalLaneGroupView {
                lir: &self.inner,
                record,
            })
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部设施带。
    pub fn facility_bands(&self) -> impl ExactSizeIterator<Item = CanonicalFacilityBandView<'_>> {
        self.inner
            .facility_bands
            .iter()
            .map(|record| CanonicalFacilityBandView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取设施带。
    #[must_use]
    pub fn facility_band(
        &self,
        ordinal: FacilityBandOrdinal,
    ) -> Option<CanonicalFacilityBandView<'_>> {
        self.inner
            .facility_bands
            .get(ordinal.index())
            .map(|record| CanonicalFacilityBandView {
                lir: &self.inner,
                record,
            })
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部路口。
    pub fn junctions(&self) -> impl ExactSizeIterator<Item = CanonicalJunctionView<'_>> {
        self.inner
            .junctions
            .iter()
            .map(|record| CanonicalJunctionView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取路口。
    #[must_use]
    pub fn junction(&self, ordinal: JunctionOrdinal) -> Option<CanonicalJunctionView<'_>> {
        self.inner
            .junctions
            .get(ordinal.index())
            .map(|record| CanonicalJunctionView {
                lir: &self.inner,
                record,
            })
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部转向动作。
    pub fn movements(&self) -> impl ExactSizeIterator<Item = CanonicalMovementView<'_>> {
        self.inner
            .movements
            .iter()
            .map(|record| CanonicalMovementView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取转向动作。
    #[must_use]
    pub fn movement(&self, ordinal: MovementOrdinal) -> Option<CanonicalMovementView<'_>> {
        self.inner
            .movements
            .get(ordinal.index())
            .map(|record| CanonicalMovementView {
                lir: &self.inner,
                record,
            })
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部机动路径。
    pub fn maneuver_paths(&self) -> impl ExactSizeIterator<Item = CanonicalManeuverPathView<'_>> {
        self.inner
            .maneuver_paths
            .iter()
            .map(|record| CanonicalManeuverPathView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取机动路径。
    #[must_use]
    pub fn maneuver_path(
        &self,
        ordinal: ManeuverPathOrdinal,
    ) -> Option<CanonicalManeuverPathView<'_>> {
        self.inner
            .maneuver_paths
            .get(ordinal.index())
            .map(|record| CanonicalManeuverPathView {
                lir: &self.inner,
                record,
            })
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部停止线。
    pub fn stop_lines(&self) -> impl ExactSizeIterator<Item = CanonicalStopLineView<'_>> {
        self.inner
            .stop_lines
            .iter()
            .map(|record| CanonicalStopLineView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取停止线。
    #[must_use]
    pub fn stop_line(&self, ordinal: StopLineOrdinal) -> Option<CanonicalStopLineView<'_>> {
        self.inner
            .stop_lines
            .get(ordinal.index())
            .map(|record| CanonicalStopLineView {
                lir: &self.inner,
                record,
            })
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部机动门。
    pub fn maneuver_gates(&self) -> impl ExactSizeIterator<Item = CanonicalManeuverGateView<'_>> {
        self.inner
            .maneuver_gates
            .iter()
            .map(|record| CanonicalManeuverGateView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取机动门。
    #[must_use]
    pub fn maneuver_gate(
        &self,
        ordinal: ManeuverGateOrdinal,
    ) -> Option<CanonicalManeuverGateView<'_>> {
        self.inner
            .maneuver_gates
            .get(ordinal.index())
            .map(|record| CanonicalManeuverGateView {
                lir: &self.inner,
                record,
            })
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部等待区。
    pub fn waiting_zones(&self) -> impl ExactSizeIterator<Item = CanonicalWaitingZoneView<'_>> {
        self.inner
            .waiting_zones
            .iter()
            .map(|record| CanonicalWaitingZoneView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取等待区。
    #[must_use]
    pub fn waiting_zone(
        &self,
        ordinal: WaitingZoneOrdinal,
    ) -> Option<CanonicalWaitingZoneView<'_>> {
        self.inner
            .waiting_zones
            .get(ordinal.index())
            .map(|record| CanonicalWaitingZoneView {
                lir: &self.inner,
                record,
            })
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部信号组。
    pub fn signal_groups(&self) -> impl ExactSizeIterator<Item = CanonicalSignalGroupView<'_>> {
        self.inner
            .signal_groups
            .iter()
            .map(|record| CanonicalSignalGroupView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取信号组。
    #[must_use]
    pub fn signal_group(
        &self,
        ordinal: SignalGroupOrdinal,
    ) -> Option<CanonicalSignalGroupView<'_>> {
        self.inner
            .signal_groups
            .get(ordinal.index())
            .map(|record| CanonicalSignalGroupView {
                lir: &self.inner,
                record,
            })
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部固定时制信号控制器。
    pub fn signal_controllers(
        &self,
    ) -> impl ExactSizeIterator<Item = CanonicalSignalControllerView<'_>> {
        self.inner
            .signal_controllers
            .iter()
            .map(|record| CanonicalSignalControllerView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取信号控制器。
    #[must_use]
    pub fn signal_controller(
        &self,
        ordinal: SignalControllerOrdinal,
    ) -> Option<CanonicalSignalControllerView<'_>> {
        self.inner
            .signal_controllers
            .get(ordinal.index())
            .map(|record| CanonicalSignalControllerView {
                lir: &self.inner,
                record,
            })
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部所有者局部（owner-local）信号相位。
    pub fn signal_phases(&self) -> impl ExactSizeIterator<Item = CanonicalSignalPhaseView<'_>> {
        self.inner
            .signal_phases
            .iter()
            .map(|record| CanonicalSignalPhaseView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取信号相位。
    #[must_use]
    pub fn signal_phase(
        &self,
        ordinal: SignalPhaseOrdinal,
    ) -> Option<CanonicalSignalPhaseView<'_>> {
        self.inner
            .signal_phases
            .get(ordinal.index())
            .map(|record| CanonicalSignalPhaseView {
                lir: &self.inner,
                record,
            })
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部停车区域。
    pub fn parking_areas(&self) -> impl ExactSizeIterator<Item = CanonicalParkingAreaView<'_>> {
        self.inner
            .parking_areas
            .iter()
            .map(|record| CanonicalParkingAreaView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取停车区域。
    #[must_use]
    pub fn parking_area(
        &self,
        ordinal: ParkingAreaOrdinal,
    ) -> Option<CanonicalParkingAreaView<'_>> {
        self.inner
            .parking_areas
            .get(ordinal.index())
            .map(|record| CanonicalParkingAreaView {
                lir: &self.inner,
                record,
            })
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部停车位。
    pub fn parking_spaces(&self) -> impl ExactSizeIterator<Item = CanonicalParkingSpaceView<'_>> {
        self.inner
            .parking_spaces
            .iter()
            .map(|record| CanonicalParkingSpaceView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取停车位。
    #[must_use]
    pub fn parking_space(
        &self,
        ordinal: ParkingSpaceOrdinal,
    ) -> Option<CanonicalParkingSpaceView<'_>> {
        self.inner
            .parking_spaces
            .get(ordinal.index())
            .map(|record| CanonicalParkingSpaceView {
                lir: &self.inner,
                record,
            })
    }

    /// 按 `LaneEdgeOrdinal` 遍历全部派生的路口内部边所有权。
    ///
    /// 验证阶段已证明每条内部边最多属于一个路口，并且不会同时承担任一路径的入口或出口
    /// 边角色；同一路口的多条路径仍可合法共享同一内部边。
    pub fn junction_internal_edges(
        &self,
    ) -> impl ExactSizeIterator<Item = CanonicalJunctionInternalEdgeView<'_>> {
        self.inner
            .junction_internal_edges
            .iter()
            .map(|record| CanonicalJunctionInternalEdgeView { record })
    }

    /// 返回一条车道图边的派生路口内部所有者；边不承担内部角色时返回 `None`。
    #[must_use]
    pub fn junction_internal_owner(&self, edge: LaneEdgeOrdinal) -> Option<JunctionOrdinal> {
        self.inner
            .junction_internal_edges
            .binary_search_by_key(&edge, |relation| relation.edge)
            .ok()
            .map(|index| self.inner.junction_internal_edges[index].junction)
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部静态路线。
    pub fn static_routes(&self) -> impl ExactSizeIterator<Item = CanonicalStaticRouteView<'_>> {
        self.inner
            .static_routes
            .iter()
            .map(|record| CanonicalStaticRouteView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取静态路线。
    #[must_use]
    pub fn static_route(
        &self,
        ordinal: StaticRouteOrdinal,
    ) -> Option<CanonicalStaticRouteView<'_>> {
        self.inner
            .static_routes
            .get(ordinal.index())
            .map(|record| CanonicalStaticRouteView {
                lir: &self.inner,
                record,
            })
    }
}

/// Canonical LIR 中一条 `LaneEdge` 记录的借用视图。
#[derive(Clone, Copy)]
pub struct CanonicalLaneEdgeView<'a> {
    lir: &'a LirUnit,
    edge: &'a LirLaneEdge,
}

impl CanonicalLaneEdgeView<'_> {
    /// 返回当前表中的有类型逻辑序号。
    #[must_use]
    pub const fn ordinal(&self) -> LaneEdgeOrdinal {
        self.edge.ordinal
    }

    /// 返回由完整 Identity v1 前像派生的稳定标识。
    #[must_use]
    pub const fn stable_id(&self) -> LaneEdgeId {
        self.edge.stable_id
    }

    /// 按 Identity v1 登记顺序遍历完整规范身份字段。
    pub fn identity_fields(&self) -> impl ExactSizeIterator<Item = CanonicalIdentityFieldView<'_>> {
        self.lir.identity_fields[self.edge.identity_fields.as_usize_range()]
            .iter()
            .map(|field| CanonicalIdentityFieldView {
                identity_field_bytes: &self.lir.identity_field_bytes,
                field,
            })
    }

    /// 返回交通权威长度，单位为米。
    #[must_use]
    pub const fn length_meters(&self) -> f64 {
        self.edge.length_meters
    }

    /// 返回基础道路限速，单位为米每秒。
    #[must_use]
    pub const fn speed_limit_meters_per_second(&self) -> f64 {
        self.edge.speed_limit_meters_per_second
    }

    /// 返回按领域顺序冻结的下游边有类型序号。
    #[must_use]
    pub fn successors(&self) -> &[LaneEdgeOrdinal] {
        &self.lir.lane_edge_successors[self.edge.successors.as_usize_range()]
    }

    /// 遍历引用此边的静态路线边出现项；重复边访问会产生多个不同路线内下标。
    pub fn static_route_occurrences(
        &self,
    ) -> impl ExactSizeIterator<Item = CanonicalStaticRouteOccurrenceRef> + '_ {
        occurrence_refs(
            &self.lir.lane_edge_route_occurrences
                [self.edge.static_route_occurrences.as_usize_range()],
        )
    }
}

macro_rules! impl_stable_entity_view {
    ($view:ident, $record:ty, $ordinal:ty, $id:ty) => {
        /// Canonical LIR 中一个已验证稳定实体的借用视图。
        #[derive(Clone, Copy)]
        pub struct $view<'a> {
            lir: &'a LirUnit,
            record: &'a $record,
        }

        impl $view<'_> {
            /// 返回当前实体表中的有类型逻辑序号。
            #[must_use]
            pub const fn ordinal(&self) -> $ordinal {
                self.record.ordinal
            }

            /// 返回由完整 Identity v1 前像派生的有类型稳定标识。
            #[must_use]
            pub const fn stable_id(&self) -> $id {
                self.record.stable_id
            }

            /// 按 Identity v1 登记顺序遍历完整规范身份字段。
            pub fn identity_fields(
                &self,
            ) -> impl ExactSizeIterator<Item = CanonicalIdentityFieldView<'_>> {
                self.lir.identity_fields[self.record.identity_fields.as_usize_range()]
                    .iter()
                    .map(|field| CanonicalIdentityFieldView {
                        identity_field_bytes: &self.lir.identity_field_bytes,
                        field,
                    })
            }
        }
    };
}

impl_stable_entity_view!(
    CanonicalRoadCorridorView,
    LirRoadCorridor,
    RoadCorridorOrdinal,
    RoadCorridorId
);
impl_stable_entity_view!(
    CanonicalRoadSectionView,
    LirRoadSection,
    RoadSectionOrdinal,
    RoadSectionId
);
impl_stable_entity_view!(
    CanonicalAuthoringLaneView,
    LirAuthoringLane,
    AuthoringLaneOrdinal,
    AuthoringLaneId
);
impl_stable_entity_view!(
    CanonicalLaneGroupView,
    LirLaneGroup,
    LaneGroupOrdinal,
    LaneGroupId
);
impl_stable_entity_view!(
    CanonicalFacilityBandView,
    LirFacilityBand,
    FacilityBandOrdinal,
    FacilityBandId
);
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
    CanonicalSignalGroupView,
    LirSignalGroup,
    SignalGroupOrdinal,
    SignalGroupId
);
impl_stable_entity_view!(
    CanonicalSignalControllerView,
    LirSignalController,
    SignalControllerOrdinal,
    SignalControllerId
);
impl_stable_entity_view!(
    CanonicalSignalPhaseView,
    LirSignalPhase,
    SignalPhaseOrdinal,
    SignalPhaseId
);
impl_stable_entity_view!(
    CanonicalParkingAreaView,
    LirParkingArea,
    ParkingAreaOrdinal,
    ParkingAreaId
);
impl_stable_entity_view!(
    CanonicalParkingSpaceView,
    LirParkingSpace,
    ParkingSpaceOrdinal,
    ParkingSpaceId
);
impl_stable_entity_view!(
    CanonicalStaticRouteView,
    LirStaticRoute,
    StaticRouteOrdinal,
    StaticRouteId
);

/// 一个稳定实体在静态路线中的反向出现项。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CanonicalStaticRouteOccurrenceRef {
    static_route: StaticRouteOrdinal,
    occurrence_index: u32,
}

impl CanonicalStaticRouteOccurrenceRef {
    /// 返回拥有该出现项的静态路线。
    #[must_use]
    pub const fn static_route(self) -> StaticRouteOrdinal {
        self.static_route
    }

    /// 返回对应关系表中、所属路线内的零基出现项下标。
    #[must_use]
    pub const fn occurrence_index(self) -> u32 {
        self.occurrence_index
    }
}

fn occurrence_refs<'a>(
    records: &'a [LirRouteOccurrenceRef],
) -> impl ExactSizeIterator<Item = CanonicalStaticRouteOccurrenceRef> + 'a {
    records
        .iter()
        .map(|record| CanonicalStaticRouteOccurrenceRef {
            static_route: record.static_route,
            occurrence_index: record.occurrence_index,
        })
}

/// 道路走廊有序横断面中的一项有类型成员。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CanonicalCorridorElement {
    /// 一个承载编制车道的有方向道路区段。
    RoadSection(RoadSectionOrdinal),
    /// 一个不进入遍历图的非方向设施带。
    FacilityBand(FacilityBandOrdinal),
}

impl CanonicalRoadCorridorView<'_> {
    /// 返回定义横断面参考方向、且已证明属于本走廊的道路区段。
    #[must_use]
    pub const fn reference_section(&self) -> RoadSectionOrdinal {
        self.record.reference_section
    }

    /// 按走廊参考方向从左到右遍历横断面成员；该顺序具有领域语义。
    pub fn elements(&self) -> impl ExactSizeIterator<Item = CanonicalCorridorElement> + '_ {
        self.lir.corridor_elements[self.record.elements.as_usize_range()]
            .iter()
            .map(|element| match element {
                LirCorridorElement::RoadSection(ordinal) => {
                    CanonicalCorridorElement::RoadSection(*ordinal)
                }
                LirCorridorElement::FacilityBand(ordinal) => {
                    CanonicalCorridorElement::FacilityBand(*ordinal)
                }
            })
    }
}

impl CanonicalRoadSectionView<'_> {
    /// 返回唯一拥有本区段的道路走廊。
    #[must_use]
    pub const fn road_corridor(&self) -> RoadCorridorOrdinal {
        self.record.road_corridor
    }

    /// 返回已验证为 lane-bearing 类别的物理设施 token。
    #[must_use]
    pub fn kind_id(&self) -> &str {
        &self.record.kind_id
    }

    /// 返回按走廊参考方向从左到右排列的编制车道序号。
    #[must_use]
    pub fn lanes(&self) -> &[AuthoringLaneOrdinal] {
        &self.lir.road_section_lanes[self.record.lanes.as_usize_range()]
    }
}

impl CanonicalAuthoringLaneView<'_> {
    /// 返回唯一拥有本编制车道的道路区段。
    #[must_use]
    pub const fn road_section(&self) -> RoadSectionOrdinal {
        self.record.road_section
    }

    /// 返回沿行驶方向排列、已证明直接连通的车道图边覆盖链。
    #[must_use]
    pub fn edge_chain(&self) -> &[LaneEdgeOrdinal] {
        &self.lir.authoring_lane_edges[self.record.edge_chain.as_usize_range()]
    }

    /// 返回可选车道组；存在时已证明与本车道属于同一道路区段。
    #[must_use]
    pub const fn lane_group(&self) -> Option<LaneGroupOrdinal> {
        self.record.lane_group
    }
}

impl CanonicalLaneGroupView<'_> {
    /// 返回唯一拥有本组的道路区段。
    #[must_use]
    pub const fn road_section(&self) -> RoadSectionOrdinal {
        self.record.road_section
    }

    /// 返回非空且全部属于同一父区段的编制车道成员。
    #[must_use]
    pub fn members(&self) -> &[AuthoringLaneOrdinal] {
        &self.lir.lane_group_members[self.record.members.as_usize_range()]
    }
}

impl CanonicalFacilityBandView<'_> {
    /// 返回唯一拥有本设施带的道路走廊。
    #[must_use]
    pub const fn road_corridor(&self) -> RoadCorridorOrdinal {
        self.record.road_corridor
    }

    /// 返回已验证为 non-traversable 类别的物理设施 token。
    #[must_use]
    pub fn kind_id(&self) -> &str {
        &self.record.kind_id
    }
}

impl CanonicalJunctionView<'_> {
    /// 返回本路口拥有的非空转向动作集合。
    #[must_use]
    pub fn movements(&self) -> &[MovementOrdinal] {
        &self.lir.junction_movements[self.record.movements.as_usize_range()]
    }
}

impl CanonicalMovementView<'_> {
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

    /// 遍历完整匹配此机动路径的静态路线机动出现项。
    pub fn static_route_occurrences(
        &self,
    ) -> impl ExactSizeIterator<Item = CanonicalStaticRouteOccurrenceRef> + '_ {
        occurrence_refs(
            &self.lir.maneuver_path_route_occurrences
                [self.record.static_route_occurrences.as_usize_range()],
        )
    }
}

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

/// 已验证机动门的信号层控制绑定。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CanonicalSignalControl {
    /// 由指定固定时制信号组给出灯色约束。
    Group(SignalGroupOrdinal),
    /// 信号层不对该门施加约束；不等同于最终可通行。
    None,
}

impl CanonicalSignalGroupView<'_> {
    /// 返回唯一拥有本信号组的固定时制控制器。
    #[must_use]
    pub const fn controller(&self) -> SignalControllerOrdinal {
        self.record.controller
    }

    /// 返回由本组控制的非空机动门集合，按门的规范序号冻结。
    #[must_use]
    pub fn maneuver_gates(&self) -> &[ManeuverGateOrdinal] {
        &self.lir.signal_group_maneuver_gates[self.record.maneuver_gates.as_usize_range()]
    }
}

impl CanonicalSignalControllerView<'_> {
    /// 返回相对世界时间零点的规范循环偏移，单位为毫秒。
    #[must_use]
    pub const fn offset_ms(&self) -> u64 {
        self.record.offset_ms
    }

    /// 返回全部相位持续时间之和，单位为毫秒。
    #[must_use]
    pub const fn cycle_duration_ms(&self) -> u64 {
        self.record.cycle_duration_ms
    }

    /// 返回本控制器唯一拥有的信号组集合，按规范序号冻结。
    #[must_use]
    pub fn signal_groups(&self) -> &[SignalGroupOrdinal] {
        &self.lir.signal_controller_groups[self.record.signal_groups.as_usize_range()]
    }

    /// 返回定义固定时制循环程序的相位序列；该顺序具有执行语义。
    #[must_use]
    pub fn phases(&self) -> &[SignalPhaseOrdinal] {
        &self.lir.signal_controller_phases[self.record.phases.as_usize_range()]
    }
}

impl CanonicalSignalPhaseView<'_> {
    /// 返回唯一拥有本相位的信号控制器。
    #[must_use]
    pub const fn controller(&self) -> SignalControllerOrdinal {
        self.record.controller
    }

    /// 返回相位持续时间，单位为毫秒。
    #[must_use]
    pub const fn duration_ms(&self) -> u64 {
        self.record.duration_ms
    }

    /// 按控制器信号组规范顺序遍历完整灯色赋值。
    pub fn states(&self) -> impl ExactSizeIterator<Item = CanonicalSignalPhaseStateView<'_>> + '_ {
        self.lir.signal_phase_states[self.record.states.as_usize_range()]
            .iter()
            .map(|record| CanonicalSignalPhaseStateView { record })
    }
}

/// 固定时制相位对一个信号组的只读状态赋值。
#[derive(Clone, Copy)]
pub struct CanonicalSignalPhaseStateView<'a> {
    record: &'a LirSignalPhaseState,
}

impl CanonicalSignalPhaseStateView<'_> {
    /// 返回被赋值的信号组。
    #[must_use]
    pub const fn signal_group(self) -> SignalGroupOrdinal {
        self.record.signal_group
    }

    /// 返回本相位内的灯色指示；它不是最终通行权判定。
    #[must_use]
    pub const fn aspect(self) -> SignalAspect {
        self.record.aspect
    }
}

impl CanonicalParkingAreaView<'_> {
    /// 返回按规范停车位序号冻结的非空成员集合。
    #[must_use]
    pub fn parking_spaces(&self) -> &[ParkingSpaceOrdinal] {
        &self.lir.parking_area_spaces[self.record.parking_spaces.as_usize_range()]
    }
}

impl CanonicalParkingSpaceView<'_> {
    /// 返回可选停车区域组织归属；`None` 表示合法的独立停车位。
    #[must_use]
    pub const fn parking_area(&self) -> Option<ParkingAreaOrdinal> {
        self.record.parking_area
    }

    /// 返回驶入并提交停车动作前必须到达的车道图锚点。
    #[must_use]
    pub const fn entry(&self) -> CanonicalParkingLaneAnchor {
        CanonicalParkingLaneAnchor {
            lane_edge: self.record.entry.lane_edge,
            progress_meters: self.record.entry.progress_meters,
        }
    }

    /// 返回离开停车位后重新接入车道图的锚点。
    #[must_use]
    pub const fn exit(&self) -> CanonicalParkingLaneAnchor {
        CanonicalParkingLaneAnchor {
            lane_edge: self.record.exit.lane_edge,
            progress_meters: self.record.exit.progress_meters,
        }
    }

    /// 返回相对入口边正向切线解释的不可变矩形几何。
    #[must_use]
    pub const fn geometry(&self) -> CanonicalParkingSpaceGeometry {
        CanonicalParkingSpaceGeometry {
            lateral_offset_meters: self.record.geometry.lateral_offset_meters,
            heading_offset_radians: self.record.geometry.heading_offset_radians,
            length_meters: self.record.geometry.length_meters,
            width_meters: self.record.geometry.width_meters,
        }
    }
}

/// Canonical LIR 中一个已验证停车锚点的值视图。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalParkingLaneAnchor {
    lane_edge: LaneEdgeOrdinal,
    progress_meters: f64,
}

impl CanonicalParkingLaneAnchor {
    /// 返回锚点所在的车道图边。
    #[must_use]
    pub const fn lane_edge(self) -> LaneEdgeOrdinal {
        self.lane_edge
    }

    /// 返回从边起点量取的纵向进度，单位为米。
    #[must_use]
    pub const fn progress_meters(self) -> f64 {
        self.progress_meters
    }
}

/// Canonical LIR 中已验证停车位矩形几何的值视图。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalParkingSpaceGeometry {
    lateral_offset_meters: f64,
    heading_offset_radians: f64,
    length_meters: f64,
    width_meters: f64,
}

impl CanonicalParkingSpaceGeometry {
    /// 返回相对入口边中心线的横向偏移，单位为米；正值位于行驶方向左侧。
    #[must_use]
    pub const fn lateral_offset_meters(self) -> f64 {
        self.lateral_offset_meters
    }

    /// 返回相对入口边正向切线的逆时针朝向偏移，单位为弧度。
    #[must_use]
    pub const fn heading_offset_radians(self) -> f64 {
        self.heading_offset_radians
    }

    /// 返回沿停车朝向的泊位长度，单位为米。
    #[must_use]
    pub const fn length_meters(self) -> f64 {
        self.length_meters
    }

    /// 返回垂直停车朝向的泊位宽度，单位为米。
    #[must_use]
    pub const fn width_meters(self) -> f64 {
        self.width_meters
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
            .map(move |record| CanonicalManeuverOccurrenceView {
                record,
                route_gate_start: gate_start,
                route_waiting_start: waiting_start,
            })
    }

    /// 按路线内出现顺序遍历机动门出现项。
    pub fn gate_occurrences(
        &self,
    ) -> impl ExactSizeIterator<Item = CanonicalGateOccurrenceView<'_>> + '_ {
        self.lir.gate_occurrences[self.record.gate_occurrences.as_usize_range()]
            .iter()
            .map(|record| CanonicalGateOccurrenceView { record })
    }

    /// 按路线内出现顺序遍历等待区出现项。
    pub fn waiting_zone_occurrences(
        &self,
    ) -> impl ExactSizeIterator<Item = CanonicalWaitingZoneOccurrenceView<'_>> + '_ {
        self.lir.waiting_zone_occurrences[self.record.waiting_zone_occurrences.as_usize_range()]
            .iter()
            .map(|record| CanonicalWaitingZoneOccurrenceView { record })
    }
}

/// 静态路线中一次完整 `ManeuverPath` 匹配的只读视图。
#[derive(Clone, Copy)]
pub struct CanonicalManeuverOccurrenceView<'a> {
    record: &'a LirManeuverOccurrence,
    route_gate_start: u32,
    route_waiting_start: u32,
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

/// Canonical LIR 中一条派生路口内部边所有权的借用视图。
#[derive(Clone, Copy)]
pub struct CanonicalJunctionInternalEdgeView<'a> {
    record: &'a LirJunctionInternalEdge,
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

/// Canonical LIR 共享身份字段池中的一项借用视图。
#[derive(Clone, Copy)]
pub struct CanonicalIdentityFieldView<'a> {
    identity_field_bytes: &'a [u8],
    field: &'a LirIdentityField,
}

impl CanonicalIdentityFieldView<'_> {
    /// 返回 Identity v1 登记字段标签。
    #[must_use]
    pub const fn tag(&self) -> FieldTag {
        self.field.tag
    }

    /// 返回字段的完整规范值字节，不包含标签和长度前缀。
    #[must_use]
    pub fn value_bytes(&self) -> &[u8] {
        &self.identity_field_bytes[self.field.value_bytes.as_usize_range()]
    }
}

/// 一次成功编译原子拥有的已验证结果。
///
/// LIR 与来源伴随数据不能分别构造或重新配对；后继源映射后端必须从同一个实例同时借用
/// 二者。当前支持子集不产生 warning/note，因此 `diagnostics` 为空，但该成功契约保留
/// 非错误级诊断通道。
pub struct CompilationOutput {
    lir: ValidatedCanonicalLir,
    source_map_input: ValidatedSourceMapInput,
    diagnostics: Box<[Diagnostic]>,
}

impl CompilationOutput {
    /// 借用所有静态语义后端的唯一输入。
    #[must_use]
    pub const fn lir(&self) -> &ValidatedCanonicalLir {
        &self.lir
    }

    /// 借用仅供源映射/诊断后端使用的来源伴随数据。
    #[must_use]
    pub const fn source_map_input(&self) -> &ValidatedSourceMapInput {
        &self.source_map_input
    }

    /// 返回成功路径保留的非错误级诊断。
    #[must_use]
    pub const fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
}

/// 可复用的 LaneFlow 静态路网编译器。
///
/// 当前干净单线程预言机没有跨编译语义状态；因此失败后无需清理缓存，也不可能让上次
/// 编译污染下一次结果。后继若加入容量复用，仍必须维持这一可观察契约。
pub struct Compiler {
    _private: (),
}

// #292 G1 只冻结显式 `Compiler::new()`，没有授权额外的公共 `Default` 构造契约。
#[allow(clippy::new_without_default)]
impl Compiler {
    /// 建立一个没有隐式输入、线程配置或无限资源模式的编译器实例。
    #[must_use]
    pub const fn new() -> Self {
        Self { _private: () }
    }

    /// 消费一个受检编译单元并运行当前已实现的 #292 领域子集编译管线。
    ///
    /// # Errors
    ///
    /// 任一阶段出现错误级语义诊断或超过 `CompilationUnit` 携带的资源配置档时，返回
    /// 规范排序的 [`DiagnosticBundle`]。错误路径不会返回部分 LIR 或部分源映射输入；
    /// 同一个 `Compiler` 可以立即用于下一次编译。
    pub fn compile(
        &mut self,
        unit: CompilationUnit,
    ) -> Result<CompilationOutput, DiagnosticBundle> {
        let hir = build_hir(&unit)?;
        let mir = lower_to_mir(&unit, &hir)?;
        // MIR 已拥有后继阶段所需的完整语义与来源位置；尽早释放 HIR，避免把阶段共存
        // 时间延长到 LIR/source-map 冻结并破坏资源峰值模型。
        drop(hir);
        let frozen_lir = freeze_lir(&unit, &mir)?;
        let source_map_input = freeze_source_map(unit, &mir, &frozen_lir)?;
        drop(mir);
        let crate::lir::LirFreezeOutput { lir, .. } = frozen_lir;
        Ok(CompilationOutput {
            lir: ValidatedCanonicalLir { inner: lir },
            source_map_input,
            diagnostics: Box::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AuthoringLaneInput, CompilationUnitBuilder, CompileLimitDimension, CompileLimits,
        CorridorElementReference, DiagnosticCode, DiagnosticPayload, FacilityBandInput,
        FacilityBandReference, JunctionInput, JunctionReference, LaneEdgeInput, LaneEdgeReference,
        LaneGroupInput, LaneGroupReference, ManeuverGateInput, ManeuverGateReference,
        ManeuverPathInput, ManeuverPathReference, MovementInput, MovementReference,
        ParkingAreaInput, ParkingAreaReference, ParkingLaneAnchorInput, ParkingSpaceGeometryInput,
        ParkingSpaceInput, RoadCorridorInput, RoadSectionInput, RoadSectionReference,
        SignalControlInput, SignalControllerInput, SignalGroupInput, SignalGroupReference,
        SignalGroupStateInput, SignalPhaseInput, SourceModuleDescriptor, SourceModuleHeader,
        SourceModuleHeaderInput, SourceRelationRole, StaticRouteInput, StopLineInput,
        StopLineReference, SyntheticModule, SyntheticModuleBuilder, WaitingZoneInput,
    };

    fn module(
        namespace: &str,
        document: &str,
        imports: &[&str],
        edges: &[(&str, f64, &[LaneEdgeReference<'_>])],
    ) -> SyntheticModule {
        let limits = CompileLimits::p100_initial_v1();
        let header = SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: namespace,
                source_document_key: document,
                generator_build_id: "git:0123456789abcdef",
                parameters_and_inputs_digest: [0x11; 32],
                frontend_options_digest: [0x22; 32],
                random_seed: Some(42),
                provenance: "repository:laneflow",
            },
            &limits,
        )
        .unwrap();
        let mut builder = SyntheticModuleBuilder::new(header, &limits).unwrap();
        for import in imports {
            builder.add_import(import).unwrap();
        }
        for (key, length_meters, successors) in edges {
            builder
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: key,
                    length_meters: *length_meters,
                    speed_limit_meters_per_second: 13.75,
                    successors,
                })
                .unwrap();
        }
        builder.finish().unwrap()
    }

    fn unit(modules: impl IntoIterator<Item = SyntheticModule>) -> CompilationUnit {
        let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
        for module in modules {
            builder.add_synthetic_module(module).unwrap();
        }
        builder.build().unwrap()
    }

    fn cross_section_module(permuted: bool) -> SyntheticModule {
        let limits = CompileLimits::p100_initial_v1();
        let header = SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: "city/cross-section",
                source_document_key: "cross-section.document",
                generator_build_id: "git:0123456789abcdef",
                parameters_and_inputs_digest: [0x11; 32],
                frontend_options_digest: [0x22; 32],
                random_seed: Some(42),
                provenance: "repository:laneflow",
            },
            &limits,
        )
        .unwrap();
        let mut builder = SyntheticModuleBuilder::new(header, &limits).unwrap();
        let add_edges = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "edge-a",
                    length_meters: 10.0,
                    speed_limit_meters_per_second: 12.0,
                    successors: &[LaneEdgeReference::local("edge-b")],
                })
                .unwrap()
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "edge-b",
                    length_meters: 12.0,
                    speed_limit_meters_per_second: 12.0,
                    successors: &[],
                })
                .unwrap();
        };
        let add_band = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_facility_band(FacilityBandInput {
                    facility_band_key: "sidewalk-left",
                    kind_id: "sidewalk",
                })
                .unwrap();
        };
        let add_group = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_lane_group(LaneGroupInput {
                    lane_group_key: "through",
                    road_section: RoadSectionReference::local("carriageway"),
                })
                .unwrap();
        };
        let add_section = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_road_section(RoadSectionInput {
                    road_section_key: "carriageway",
                    kind_id: "motorLane",
                    lanes: &[AuthoringLaneInput {
                        authoring_lane_key: "lane-main",
                        edge_chain: &[
                            LaneEdgeReference::local("edge-a"),
                            LaneEdgeReference::local("edge-b"),
                        ],
                        lane_group: Some(LaneGroupReference::local("through")),
                    }],
                })
                .unwrap();
        };
        let add_corridor = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_road_corridor(RoadCorridorInput {
                    road_corridor_key: "main-road",
                    reference_section: RoadSectionReference::local("carriageway"),
                    elements: &[
                        CorridorElementReference::facility_band(FacilityBandReference::local(
                            "sidewalk-left",
                        )),
                        CorridorElementReference::road_section(RoadSectionReference::local(
                            "carriageway",
                        )),
                    ],
                })
                .unwrap();
        };

        if permuted {
            add_corridor(&mut builder);
            add_section(&mut builder);
            add_group(&mut builder);
            add_band(&mut builder);
            add_edges(&mut builder);
        } else {
            add_edges(&mut builder);
            add_band(&mut builder);
            add_group(&mut builder);
            add_section(&mut builder);
            add_corridor(&mut builder);
        }
        builder.finish().unwrap()
    }

    fn junction_builder(document: &str) -> SyntheticModuleBuilder {
        let limits = CompileLimits::p100_initial_v1();
        let header = SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: "city/junction",
                source_document_key: document,
                generator_build_id: "git:0123456789abcdef",
                parameters_and_inputs_digest: [0x11; 32],
                frontend_options_digest: [0x22; 32],
                random_seed: Some(42),
                provenance: "repository:laneflow",
            },
            &limits,
        )
        .unwrap();
        SyntheticModuleBuilder::new(header, &limits).unwrap()
    }

    fn junction_module(permuted: bool, selected_internal: &'static str) -> SyntheticModule {
        let mut builder = junction_builder(if permuted {
            "junction-permuted.document"
        } else {
            "junction.document"
        });
        let add_edges = |builder: &mut SyntheticModuleBuilder| {
            let internal_successors = [LaneEdgeReference::local("exit")];
            let entry_successors = [
                LaneEdgeReference::local("internal-a"),
                LaneEdgeReference::local("internal-b"),
            ];
            builder
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "entry-a",
                    length_meters: 10.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &entry_successors,
                })
                .unwrap()
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "entry-b",
                    length_meters: 10.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &entry_successors,
                })
                .unwrap()
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "internal-a",
                    length_meters: 8.0,
                    speed_limit_meters_per_second: 8.0,
                    successors: &internal_successors,
                })
                .unwrap()
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "internal-b",
                    length_meters: 8.0,
                    speed_limit_meters_per_second: 8.0,
                    successors: &internal_successors,
                })
                .unwrap()
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "exit",
                    length_meters: 12.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[],
                })
                .unwrap();
        };
        let add_junction = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_junction(JunctionInput {
                    junction_key: "junction-main",
                })
                .unwrap();
        };
        let add_movement = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_movement(MovementInput {
                    movement_key: "movement-through",
                    junction: JunctionReference::local("junction-main"),
                    directed_entry_approach_key: "approach-westbound",
                    directed_exit_approach_key: "approach-eastbound",
                })
                .unwrap();
        };
        let add_path = |builder: &mut SyntheticModuleBuilder, key: &str, entry: &str| {
            let internal = [LaneEdgeReference::local(selected_internal)];
            builder
                .add_maneuver_path(ManeuverPathInput {
                    maneuver_path_key: key,
                    movement: MovementReference::local("movement-through"),
                    entry_edge: LaneEdgeReference::local(entry),
                    internal_edges: &internal,
                    exit_edge: LaneEdgeReference::local("exit"),
                })
                .unwrap();
        };

        if permuted {
            add_path(&mut builder, "path-b", "entry-b");
            add_path(&mut builder, "path-a", "entry-a");
            add_movement(&mut builder);
            add_junction(&mut builder);
            add_edges(&mut builder);
        } else {
            add_edges(&mut builder);
            add_junction(&mut builder);
            add_movement(&mut builder);
            add_path(&mut builder, "path-a", "entry-a");
            add_path(&mut builder, "path-b", "entry-b");
        }
        builder.finish().unwrap()
    }

    fn control_builder(document: &str) -> SyntheticModuleBuilder {
        let mut builder = junction_builder(document);
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "entry",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[LaneEdgeReference::local("middle")],
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "middle",
                length_meters: 8.0,
                speed_limit_meters_per_second: 8.0,
                successors: &[LaneEdgeReference::local("exit")],
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "exit",
                length_meters: 12.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_junction(JunctionInput {
                junction_key: "junction-main",
            })
            .unwrap()
            .add_movement(MovementInput {
                movement_key: "movement-through",
                junction: JunctionReference::local("junction-main"),
                directed_entry_approach_key: "approach-westbound",
                directed_exit_approach_key: "approach-eastbound",
            })
            .unwrap()
            .add_maneuver_path(ManeuverPathInput {
                maneuver_path_key: "path-main",
                movement: MovementReference::local("movement-through"),
                entry_edge: LaneEdgeReference::local("entry"),
                internal_edges: &[LaneEdgeReference::local("middle")],
                exit_edge: LaneEdgeReference::local("exit"),
            })
            .unwrap();
        builder
    }

    fn route_validation_builder(document: &str) -> SyntheticModuleBuilder {
        let mut builder = junction_builder(document);
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "entry",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[LaneEdgeReference::local("middle")],
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "other",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[LaneEdgeReference::local("middle")],
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "middle",
                length_meters: 8.0,
                speed_limit_meters_per_second: 8.0,
                successors: &[
                    LaneEdgeReference::local("exit"),
                    LaneEdgeReference::local("detour"),
                ],
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "exit",
                length_meters: 12.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "detour",
                length_meters: 12.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_junction(JunctionInput {
                junction_key: "junction-main",
            })
            .unwrap()
            .add_movement(MovementInput {
                movement_key: "movement-through",
                junction: JunctionReference::local("junction-main"),
                directed_entry_approach_key: "approach-westbound",
                directed_exit_approach_key: "approach-eastbound",
            })
            .unwrap()
            .add_maneuver_path(ManeuverPathInput {
                maneuver_path_key: "path-main",
                movement: MovementReference::local("movement-through"),
                entry_edge: LaneEdgeReference::local("entry"),
                internal_edges: &[LaneEdgeReference::local("middle")],
                exit_edge: LaneEdgeReference::local("exit"),
            })
            .unwrap();
        builder
    }

    fn add_valid_control(builder: &mut SyntheticModuleBuilder, permuted: bool) {
        let add_stops = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_stop_line(StopLineInput {
                    stop_line_key: "stop-entry",
                    lane_edge: LaneEdgeReference::local("entry"),
                })
                .unwrap()
                .add_stop_line(StopLineInput {
                    stop_line_key: "stop-middle",
                    lane_edge: LaneEdgeReference::local("middle"),
                })
                .unwrap();
        };
        let add_gates = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_maneuver_gate(ManeuverGateInput {
                    maneuver_gate_key: "gate-entry",
                    maneuver_path: ManeuverPathReference::local("path-main"),
                    transition_index: 0,
                    stop_line: StopLineReference::local("stop-entry"),
                    signal_control: SignalControlInput::None,
                })
                .unwrap()
                .add_maneuver_gate(ManeuverGateInput {
                    maneuver_gate_key: "gate-release",
                    maneuver_path: ManeuverPathReference::local("path-main"),
                    transition_index: 1,
                    stop_line: StopLineReference::local("stop-middle"),
                    signal_control: SignalControlInput::None,
                })
                .unwrap();
        };
        let add_waiting = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_waiting_zone(WaitingZoneInput {
                    waiting_zone_key: "waiting-main",
                    maneuver_path: ManeuverPathReference::local("path-main"),
                    entry_gate: ManeuverGateReference::local("gate-entry"),
                    release_gate: ManeuverGateReference::local("gate-release"),
                    max_occupancy: 3,
                })
                .unwrap();
        };
        if permuted {
            add_waiting(builder);
            add_gates(builder);
            add_stops(builder);
        } else {
            add_stops(builder);
            add_gates(builder);
            add_waiting(builder);
        }
    }

    fn signal_module(permuted: bool) -> SyntheticModule {
        let mut builder = control_builder(if permuted {
            "signal-permuted.document"
        } else {
            "signal.document"
        });
        let add_stops = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_stop_line(StopLineInput {
                    stop_line_key: "stop-entry",
                    lane_edge: LaneEdgeReference::local("entry"),
                })
                .unwrap()
                .add_stop_line(StopLineInput {
                    stop_line_key: "stop-middle",
                    lane_edge: LaneEdgeReference::local("middle"),
                })
                .unwrap();
        };
        let add_groups = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_signal_group(SignalGroupInput {
                    signal_group_key: "group-entry",
                })
                .unwrap()
                .add_signal_group(SignalGroupInput {
                    signal_group_key: "group-release",
                })
                .unwrap();
        };
        let add_gates = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_maneuver_gate(ManeuverGateInput {
                    maneuver_gate_key: "gate-entry",
                    maneuver_path: ManeuverPathReference::local("path-main"),
                    transition_index: 0,
                    stop_line: StopLineReference::local("stop-entry"),
                    signal_control: SignalControlInput::Group(SignalGroupReference::local(
                        "group-entry",
                    )),
                })
                .unwrap()
                .add_maneuver_gate(ManeuverGateInput {
                    maneuver_gate_key: "gate-release",
                    maneuver_path: ManeuverPathReference::local("path-main"),
                    transition_index: 1,
                    stop_line: StopLineReference::local("stop-middle"),
                    signal_control: SignalControlInput::Group(SignalGroupReference::local(
                        "group-release",
                    )),
                })
                .unwrap();
        };
        let add_controller = |builder: &mut SyntheticModuleBuilder, reverse_sets: bool| {
            let groups = if reverse_sets {
                [
                    SignalGroupReference::local("group-release"),
                    SignalGroupReference::local("group-entry"),
                ]
            } else {
                [
                    SignalGroupReference::local("group-entry"),
                    SignalGroupReference::local("group-release"),
                ]
            };
            let go_states = if reverse_sets {
                [
                    SignalGroupStateInput {
                        signal_group: SignalGroupReference::local("group-release"),
                        aspect: SignalAspect::Red,
                    },
                    SignalGroupStateInput {
                        signal_group: SignalGroupReference::local("group-entry"),
                        aspect: SignalAspect::Green,
                    },
                ]
            } else {
                [
                    SignalGroupStateInput {
                        signal_group: SignalGroupReference::local("group-entry"),
                        aspect: SignalAspect::Green,
                    },
                    SignalGroupStateInput {
                        signal_group: SignalGroupReference::local("group-release"),
                        aspect: SignalAspect::Red,
                    },
                ]
            };
            let clear_states = if reverse_sets {
                [
                    SignalGroupStateInput {
                        signal_group: SignalGroupReference::local("group-release"),
                        aspect: SignalAspect::Green,
                    },
                    SignalGroupStateInput {
                        signal_group: SignalGroupReference::local("group-entry"),
                        aspect: SignalAspect::Yellow,
                    },
                ]
            } else {
                [
                    SignalGroupStateInput {
                        signal_group: SignalGroupReference::local("group-entry"),
                        aspect: SignalAspect::Yellow,
                    },
                    SignalGroupStateInput {
                        signal_group: SignalGroupReference::local("group-release"),
                        aspect: SignalAspect::Green,
                    },
                ]
            };
            builder
                .add_signal_controller(SignalControllerInput {
                    signal_controller_key: "controller-main",
                    offset_ms: 1_000,
                    signal_groups: &groups,
                    phases: &[
                        SignalPhaseInput {
                            signal_phase_key: "phase-go",
                            duration_ms: 30_000,
                            states: &go_states,
                        },
                        SignalPhaseInput {
                            signal_phase_key: "phase-clear",
                            duration_ms: 5_000,
                            states: &clear_states,
                        },
                    ],
                })
                .unwrap();
        };
        if permuted {
            add_controller(&mut builder, true);
            add_gates(&mut builder);
            add_groups(&mut builder);
            add_stops(&mut builder);
        } else {
            add_stops(&mut builder);
            add_groups(&mut builder);
            add_gates(&mut builder);
            add_controller(&mut builder, false);
        }
        builder.finish().unwrap()
    }

    fn single_signal_group_builder(document: &str) -> SyntheticModuleBuilder {
        let mut builder = control_builder(document);
        builder
            .add_stop_line(StopLineInput {
                stop_line_key: "stop-entry",
                lane_edge: LaneEdgeReference::local("entry"),
            })
            .unwrap()
            .add_signal_group(SignalGroupInput {
                signal_group_key: "group-main",
            })
            .unwrap()
            .add_maneuver_gate(ManeuverGateInput {
                maneuver_gate_key: "gate-entry",
                maneuver_path: ManeuverPathReference::local("path-main"),
                transition_index: 0,
                stop_line: StopLineReference::local("stop-entry"),
                signal_control: SignalControlInput::Group(SignalGroupReference::local(
                    "group-main",
                )),
            })
            .unwrap();
        builder
    }

    fn stable_key<'a>(
        mut fields: impl Iterator<Item = CanonicalIdentityFieldView<'a>>,
        tag: FieldTag,
    ) -> String {
        fields
            .find(|field| field.tag() == tag)
            .map(|field| String::from_utf8(field.value_bytes().to_vec()).unwrap())
            .unwrap()
    }

    fn compile_diagnostic_codes(builder: SyntheticModuleBuilder) -> Vec<DiagnosticCode> {
        match Compiler::new().compile(unit([builder.finish().unwrap()])) {
            Ok(_) => panic!("expected junction topology validation failure"),
            Err(diagnostics) => diagnostics
                .diagnostics()
                .iter()
                .map(Diagnostic::code)
                .collect(),
        }
    }

    fn parking_builder(document: &str) -> SyntheticModuleBuilder {
        let limits = CompileLimits::p100_initial_v1();
        let header = SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: "city/parking",
                source_document_key: document,
                generator_build_id: "git:0123456789abcdef",
                parameters_and_inputs_digest: [0x11; 32],
                frontend_options_digest: [0x22; 32],
                random_seed: Some(42),
                provenance: "repository:laneflow",
            },
            &limits,
        )
        .unwrap();
        SyntheticModuleBuilder::new(header, &limits).unwrap()
    }

    fn add_parking_edges(builder: &mut SyntheticModuleBuilder) {
        for key in ["parking-entry", "parking-exit"] {
            builder
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: key,
                    length_meters: 20.0,
                    speed_limit_meters_per_second: 8.0,
                    successors: &[],
                })
                .unwrap();
        }
    }

    fn add_parking_space(builder: &mut SyntheticModuleBuilder, key: &str, area: Option<&str>) {
        builder
            .add_parking_space(ParkingSpaceInput {
                parking_space_key: key,
                parking_area: area.map(ParkingAreaReference::local),
                entry: ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local("parking-entry"),
                    progress_meters: 4.0,
                },
                exit: ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local("parking-exit"),
                    progress_meters: 6.0,
                },
                geometry: ParkingSpaceGeometryInput {
                    lateral_offset_meters: -3.0,
                    heading_offset_radians: 0.25,
                    length_meters: 5.5,
                    width_meters: 2.6,
                },
            })
            .unwrap();
    }

    fn parking_module(document: &str, area_key: &str, permuted: bool) -> SyntheticModule {
        let mut builder = parking_builder(document);
        if permuted {
            add_parking_space(&mut builder, "space-independent", None);
            add_parking_space(&mut builder, "space-owned", Some(area_key));
            builder
                .add_parking_area(ParkingAreaInput {
                    parking_area_key: area_key,
                })
                .unwrap();
            add_parking_edges(&mut builder);
        } else {
            add_parking_edges(&mut builder);
            builder
                .add_parking_area(ParkingAreaInput {
                    parking_area_key: area_key,
                })
                .unwrap();
            add_parking_space(&mut builder, "space-owned", Some(area_key));
            add_parking_space(&mut builder, "space-independent", None);
        }
        builder.finish().unwrap()
    }

    fn edge_key(edge: CanonicalLaneEdgeView<'_>) -> String {
        edge.identity_fields()
            .find(|field| field.tag() == FieldTag::LaneEdgeKey)
            .map(|field| String::from_utf8(field.value_bytes().to_vec()).unwrap())
            .unwrap()
    }

    #[test]
    fn compiler_atomically_returns_lir_source_map_and_success_diagnostics() {
        let successors = [LaneEdgeReference::imported("city/base", "edge-b")];
        let input = unit([
            module(
                "city/app",
                "app.document",
                &["city/base"],
                &[("edge-a", 10.0, &successors)],
            ),
            module("city/base", "base.document", &[], &[("edge-b", 20.0, &[])]),
        ]);
        let output = Compiler::new().compile(input).unwrap();

        assert!(output.diagnostics().is_empty());
        let edges = output.lir().lane_edges().collect::<Vec<_>>();
        assert_eq!(edges.len(), 2);
        assert_eq!(edge_key(edges[0]), "edge-a");
        assert_eq!(edges[0].ordinal().raw(), 0);
        assert_eq!(edges[0].successors(), [LaneEdgeOrdinal::from_raw(1)]);
        assert_eq!(edges[0].length_meters(), 10.0);
        assert_eq!(edges[0].speed_limit_meters_per_second(), 13.75);
        assert_eq!(
            output
                .lir()
                .lane_edge(edges[1].ordinal())
                .unwrap()
                .stable_id(),
            edges[1].stable_id()
        );

        let modules = output
            .source_map_input()
            .source_modules()
            .map(SourceModuleDescriptor::authoring_namespace_id)
            .collect::<Vec<_>>();
        assert_eq!(modules, ["city/base", "city/app"]);
        let documents = output
            .source_map_input()
            .source_documents()
            .map(|document| document.source_document_key())
            .collect::<Vec<_>>();
        assert_eq!(documents, ["base.document", "app.document"]);

        let entity_sources = output
            .source_map_input()
            .lane_edge_sources()
            .collect::<Vec<_>>();
        assert_eq!(entity_sources.len(), 2);
        for (edge, source) in edges.iter().zip(entity_sources) {
            assert_eq!(source.ordinal(), edge.ordinal());
            assert_eq!(source.stable_id(), edge.stable_id());
            assert!(source.contributing_sources().next().is_none());
        }
        assert_eq!(
            output
                .source_map_input()
                .lane_edge_successor_sources()
                .map(|source| (
                    source.owner_ordinal().raw(),
                    source.role(),
                    source.local_index(),
                    source.primary_source().source_document_key().to_owned(),
                ))
                .collect::<Vec<_>>(),
            [(
                0,
                SourceRelationRole::LaneEdgeSuccessor,
                0,
                "app.document".to_owned(),
            )]
        );
    }

    #[test]
    fn compiler_freezes_fixed_time_signal_program_bindings_and_source_relations() {
        let output = Compiler::new()
            .compile(unit([signal_module(false)]))
            .unwrap();
        let lir = output.lir();
        let groups = lir.signal_groups().collect::<Vec<_>>();
        let controller = lir.signal_controllers().next().unwrap();
        let phases = controller
            .phases()
            .iter()
            .map(|ordinal| lir.signal_phase(*ordinal).unwrap())
            .collect::<Vec<_>>();

        assert_eq!(groups.len(), 2);
        assert_eq!(controller.offset_ms(), 1_000);
        assert_eq!(controller.cycle_duration_ms(), 35_000);
        assert_eq!(controller.signal_groups().len(), 2);
        assert_eq!(
            phases
                .iter()
                .map(|phase| phase.duration_ms())
                .collect::<Vec<_>>(),
            [30_000, 5_000]
        );
        assert!(phases.iter().all(|phase| {
            phase.controller() == controller.ordinal()
                && phase
                    .states()
                    .map(|state| state.signal_group())
                    .collect::<Vec<_>>()
                    == controller.signal_groups()
        }));
        assert!(groups.iter().all(|group| {
            group.controller() == controller.ordinal() && group.maneuver_gates().len() == 1
        }));
        assert!(
            lir.maneuver_gates()
                .all(|gate| matches!(gate.signal_control(), CanonicalSignalControl::Group(_)))
        );
        assert_eq!(
            phases[0]
                .identity_fields()
                .map(|field| field.tag())
                .collect::<Vec<_>>(),
            [
                FieldTag::AuthoringNamespaceId,
                FieldTag::SignalControllerStableId,
                FieldTag::PhaseKey,
            ]
        );

        let source_map = output.source_map_input();
        assert_eq!(source_map.signal_group_sources().len(), 2);
        assert_eq!(source_map.signal_controller_sources().len(), 1);
        assert_eq!(source_map.signal_phase_sources().len(), 2);
        assert_eq!(source_map.signal_relation_sources().len(), 10);
        assert_eq!(
            source_map
                .signal_relation_sources()
                .fold([0_u32; 4], |mut counts, source| {
                    let index = match source.role() {
                        SourceRelationRole::SignalControllerGroup => 0,
                        SourceRelationRole::SignalControllerPhase => 1,
                        SourceRelationRole::SignalPhaseState => 2,
                        SourceRelationRole::ManeuverGateSignalGroup => 3,
                        _ => unreachable!("unexpected signal relation role"),
                    };
                    counts[index] += 1;
                    counts
                }),
            [2, 2, 4, 2]
        );
    }

    #[test]
    fn signal_set_permutation_does_not_change_lir_semantics() {
        let baseline = Compiler::new()
            .compile(unit([signal_module(false)]))
            .unwrap();
        let permuted = Compiler::new()
            .compile(unit([signal_module(true)]))
            .unwrap();
        assert_eq!(
            baseline.lir.inner.semantic_digest,
            permuted.lir.inner.semantic_digest
        );
    }

    #[test]
    fn signal_controller_rejects_empty_group_and_phase_programs() {
        let mut builder = control_builder("signal-invalid.document");
        builder
            .add_signal_controller(SignalControllerInput {
                signal_controller_key: "controller-empty",
                offset_ms: 0,
                signal_groups: &[],
                phases: &[],
            })
            .unwrap();
        assert_eq!(
            compile_diagnostic_codes(builder),
            [
                DiagnosticCode::EmptySignalControllerGroups,
                DiagnosticCode::EmptySignalControllerPhases,
            ]
        );
    }

    #[test]
    fn signal_program_validation_closes_phase_time_and_ownership_boundaries() {
        let valid_state = [SignalGroupStateInput {
            signal_group: SignalGroupReference::local("group-main"),
            aspect: SignalAspect::Red,
        }];

        let mut missing = single_signal_group_builder("signal-missing-state.document");
        missing
            .add_signal_controller(SignalControllerInput {
                signal_controller_key: "controller-main",
                offset_ms: 0,
                signal_groups: &[SignalGroupReference::local("group-main")],
                phases: &[SignalPhaseInput {
                    signal_phase_key: "phase-main",
                    duration_ms: 100,
                    states: &[],
                }],
            })
            .unwrap();
        assert_eq!(
            compile_diagnostic_codes(missing),
            [DiagnosticCode::MissingSignalPhaseGroup]
        );

        let duplicate_states = [valid_state[0], valid_state[0]];
        let mut duplicate = single_signal_group_builder("signal-duplicate-state.document");
        duplicate
            .add_signal_controller(SignalControllerInput {
                signal_controller_key: "controller-main",
                offset_ms: 0,
                signal_groups: &[
                    SignalGroupReference::local("group-main"),
                    SignalGroupReference::local("group-main"),
                ],
                phases: &[SignalPhaseInput {
                    signal_phase_key: "phase-main",
                    duration_ms: 100,
                    states: &duplicate_states,
                }],
            })
            .unwrap();
        let duplicate_codes = compile_diagnostic_codes(duplicate);
        assert!(duplicate_codes.contains(&DiagnosticCode::DuplicateSignalControllerGroup));
        assert!(duplicate_codes.contains(&DiagnosticCode::DuplicateSignalPhaseGroup));

        let mut invalid_duration = single_signal_group_builder("signal-invalid-duration.document");
        invalid_duration
            .add_signal_controller(SignalControllerInput {
                signal_controller_key: "controller-main",
                offset_ms: 0,
                signal_groups: &[SignalGroupReference::local("group-main")],
                phases: &[SignalPhaseInput {
                    signal_phase_key: "phase-main",
                    duration_ms: 0,
                    states: &valid_state,
                }],
            })
            .unwrap();
        assert_eq!(
            compile_diagnostic_codes(invalid_duration),
            [DiagnosticCode::InvalidSignalPhaseDuration]
        );

        let mut invalid_offset = single_signal_group_builder("signal-invalid-offset.document");
        invalid_offset
            .add_signal_controller(SignalControllerInput {
                signal_controller_key: "controller-main",
                offset_ms: 100,
                signal_groups: &[SignalGroupReference::local("group-main")],
                phases: &[SignalPhaseInput {
                    signal_phase_key: "phase-main",
                    duration_ms: 100,
                    states: &valid_state,
                }],
            })
            .unwrap();
        assert_eq!(
            compile_diagnostic_codes(invalid_offset),
            [DiagnosticCode::InvalidSignalControllerOffset]
        );

        let mut cycle_overflow = single_signal_group_builder("signal-cycle-overflow.document");
        cycle_overflow
            .add_signal_controller(SignalControllerInput {
                signal_controller_key: "controller-main",
                offset_ms: 0,
                signal_groups: &[SignalGroupReference::local("group-main")],
                phases: &[
                    SignalPhaseInput {
                        signal_phase_key: "phase-long",
                        duration_ms: 9_007_199_254_740_991,
                        states: &valid_state,
                    },
                    SignalPhaseInput {
                        signal_phase_key: "phase-overflow",
                        duration_ms: 1,
                        states: &valid_state,
                    },
                ],
            })
            .unwrap();
        assert_eq!(
            compile_diagnostic_codes(cycle_overflow),
            [DiagnosticCode::SignalCycleDurationOverflow]
        );

        let mut multiple_owner = single_signal_group_builder("signal-owner.document");
        for controller_key in ["controller-a", "controller-b"] {
            multiple_owner
                .add_signal_controller(SignalControllerInput {
                    signal_controller_key: controller_key,
                    offset_ms: 0,
                    signal_groups: &[SignalGroupReference::local("group-main")],
                    phases: &[SignalPhaseInput {
                        signal_phase_key: "phase-main",
                        duration_ms: 100,
                        states: &valid_state,
                    }],
                })
                .unwrap();
        }
        assert!(
            compile_diagnostic_codes(multiple_owner)
                .contains(&DiagnosticCode::SignalGroupMultipleControllers)
        );
    }

    #[test]
    fn signal_group_reference_failure_is_reported_even_without_signal_entities() {
        let mut builder = control_builder("signal-unknown-group.document");
        builder
            .add_stop_line(StopLineInput {
                stop_line_key: "stop-entry",
                lane_edge: LaneEdgeReference::local("entry"),
            })
            .unwrap()
            .add_maneuver_gate(ManeuverGateInput {
                maneuver_gate_key: "gate-entry",
                maneuver_path: ManeuverPathReference::local("path-main"),
                transition_index: 0,
                stop_line: StopLineReference::local("stop-entry"),
                signal_control: SignalControlInput::Group(SignalGroupReference::local(
                    "group-missing",
                )),
            })
            .unwrap();
        assert_eq!(
            compile_diagnostic_codes(builder),
            [DiagnosticCode::UnknownReferenceTarget]
        );
    }

    #[test]
    fn signal_validation_reports_local_identity_and_global_closure_failures() {
        let valid_state = [SignalGroupStateInput {
            signal_group: SignalGroupReference::local("group-main"),
            aspect: SignalAspect::Red,
        }];

        let mut duplicate_phase = single_signal_group_builder("signal-duplicate-phase.document");
        duplicate_phase
            .add_signal_controller(SignalControllerInput {
                signal_controller_key: "controller-main",
                offset_ms: 0,
                signal_groups: &[SignalGroupReference::local("group-main")],
                phases: &[
                    SignalPhaseInput {
                        signal_phase_key: "phase-main",
                        duration_ms: 100,
                        states: &valid_state,
                    },
                    SignalPhaseInput {
                        signal_phase_key: "phase-main",
                        duration_ms: 100,
                        states: &valid_state,
                    },
                ],
            })
            .unwrap();
        assert_eq!(
            compile_diagnostic_codes(duplicate_phase),
            [DiagnosticCode::DuplicateSignalPhaseKey]
        );

        let mut foreign_phase_group =
            single_signal_group_builder("signal-foreign-phase-group.document");
        foreign_phase_group
            .add_signal_group(SignalGroupInput {
                signal_group_key: "group-foreign",
            })
            .unwrap();
        let states = [
            valid_state[0],
            SignalGroupStateInput {
                signal_group: SignalGroupReference::local("group-foreign"),
                aspect: SignalAspect::Green,
            },
        ];
        foreign_phase_group
            .add_signal_controller(SignalControllerInput {
                signal_controller_key: "controller-main",
                offset_ms: 0,
                signal_groups: &[SignalGroupReference::local("group-main")],
                phases: &[SignalPhaseInput {
                    signal_phase_key: "phase-main",
                    duration_ms: 100,
                    states: &states,
                }],
            })
            .unwrap();
        let foreign_group_codes = compile_diagnostic_codes(foreign_phase_group);
        assert!(foreign_group_codes.contains(&DiagnosticCode::UnknownSignalPhaseGroup));
        assert!(foreign_group_codes.contains(&DiagnosticCode::UnownedSignalGroup));
        assert!(foreign_group_codes.contains(&DiagnosticCode::UnusedSignalGroup));

        let mut orphan_group = control_builder("signal-orphan-group.document");
        orphan_group
            .add_signal_group(SignalGroupInput {
                signal_group_key: "group-orphan",
            })
            .unwrap();
        let orphan_codes = compile_diagnostic_codes(orphan_group);
        assert!(orphan_codes.contains(&DiagnosticCode::UnownedSignalGroup));
        assert!(orphan_codes.contains(&DiagnosticCode::UnusedSignalGroup));
    }

    #[test]
    fn compiler_freezes_complete_cross_section_owner_tree_and_source_relations() {
        let output = Compiler::new()
            .compile(unit([cross_section_module(false)]))
            .unwrap();
        let lir = output.lir();
        let corridor = lir.road_corridors().next().unwrap();
        let section = lir.road_sections().next().unwrap();
        let lane = lir.authoring_lanes().next().unwrap();
        let group = lir.lane_groups().next().unwrap();
        let band = lir.facility_bands().next().unwrap();

        assert_eq!(corridor.reference_section(), section.ordinal());
        assert_eq!(
            corridor.elements().collect::<Vec<_>>(),
            [
                CanonicalCorridorElement::FacilityBand(band.ordinal()),
                CanonicalCorridorElement::RoadSection(section.ordinal()),
            ]
        );
        assert_eq!(section.road_corridor(), corridor.ordinal());
        assert_eq!(section.kind_id(), "motorLane");
        assert_eq!(section.lanes(), [lane.ordinal()]);
        assert_eq!(lane.road_section(), section.ordinal());
        assert_eq!(lane.edge_chain().len(), 2);
        assert_eq!(lane.lane_group(), Some(group.ordinal()));
        assert_eq!(group.road_section(), section.ordinal());
        assert_eq!(group.members(), [lane.ordinal()]);
        assert_eq!(band.road_corridor(), corridor.ordinal());
        assert_eq!(band.kind_id(), "sidewalk");

        let section_fields = section
            .identity_fields()
            .map(|field| (field.tag(), field.value_bytes().to_vec()))
            .collect::<Vec<_>>();
        assert_eq!(
            section_fields
                .iter()
                .map(|field| field.0)
                .collect::<Vec<_>>(),
            [
                FieldTag::AuthoringNamespaceId,
                FieldTag::SectionKey,
                FieldTag::RoadCorridorStableId,
            ]
        );
        assert_eq!(
            section_fields[2].1,
            corridor.stable_id().as_untyped().as_bytes()
        );

        let source_map = output.source_map_input();
        assert_eq!(source_map.road_corridor_sources().len(), 1);
        assert_eq!(source_map.road_section_sources().len(), 1);
        assert_eq!(source_map.authoring_lane_sources().len(), 1);
        assert_eq!(source_map.lane_group_sources().len(), 1);
        assert_eq!(source_map.facility_band_sources().len(), 1);
        assert_eq!(
            source_map
                .cross_section_relation_sources()
                .map(|source| {
                    (
                        source.owner().entity_kind(),
                        source.role(),
                        source.local_index(),
                    )
                })
                .collect::<Vec<_>>(),
            [
                (
                    laneflow_static_contract::EntityKind::RoadCorridor,
                    SourceRelationRole::RoadCorridorElement,
                    0,
                ),
                (
                    laneflow_static_contract::EntityKind::RoadCorridor,
                    SourceRelationRole::RoadCorridorElement,
                    1,
                ),
                (
                    laneflow_static_contract::EntityKind::RoadSection,
                    SourceRelationRole::RoadSectionLane,
                    0,
                ),
                (
                    laneflow_static_contract::EntityKind::AuthoringLane,
                    SourceRelationRole::AuthoringLaneEdge,
                    0,
                ),
                (
                    laneflow_static_contract::EntityKind::AuthoringLane,
                    SourceRelationRole::AuthoringLaneEdge,
                    1,
                ),
                (
                    laneflow_static_contract::EntityKind::LaneGroup,
                    SourceRelationRole::LaneGroupMember,
                    0,
                ),
            ]
        );
    }

    #[test]
    fn cross_section_lir_semantics_ignore_top_level_declaration_order() {
        let baseline = Compiler::new()
            .compile(unit([cross_section_module(false)]))
            .unwrap();
        let permuted = Compiler::new()
            .compile(unit([cross_section_module(true)]))
            .unwrap();

        assert_eq!(
            baseline.lir.inner.semantic_digest,
            permuted.lir.inner.semantic_digest
        );
        assert_eq!(
            baseline
                .lir()
                .road_corridors()
                .map(|corridor| corridor.stable_id())
                .collect::<Vec<_>>(),
            permuted
                .lir()
                .road_corridors()
                .map(|corridor| corridor.stable_id())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            baseline
                .lir()
                .authoring_lanes()
                .map(|lane| lane.stable_id())
                .collect::<Vec<_>>(),
            permuted
                .lir()
                .authoring_lanes()
                .map(|lane| lane.stable_id())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn compiler_freezes_complete_junction_topology_and_source_relations() {
        let output = Compiler::new()
            .compile(unit([junction_module(false, "internal-a")]))
            .unwrap();
        let lir = output.lir();
        let junction = lir.junctions().next().unwrap();
        let movement = lir.movements().next().unwrap();
        let paths = lir.maneuver_paths().collect::<Vec<_>>();
        let edges = lir
            .lane_edges()
            .map(|edge| (edge_key(edge), edge.ordinal()))
            .collect::<std::collections::HashMap<_, _>>();

        assert_eq!(junction.movements(), [movement.ordinal()]);
        assert_eq!(movement.junction(), junction.ordinal());
        assert_eq!(movement.directed_entry_approach_key(), "approach-westbound");
        assert_eq!(movement.directed_exit_approach_key(), "approach-eastbound");
        assert_eq!(movement.maneuver_paths().len(), 2);
        assert_eq!(paths.len(), 2);
        assert_eq!(
            stable_key(paths[0].identity_fields(), FieldTag::PathKey),
            "path-a"
        );
        assert_eq!(
            paths[0].edges(),
            [edges["entry-a"], edges["internal-a"], edges["exit"]]
        );
        assert_eq!(paths[0].entry_edge(), edges["entry-a"]);
        assert_eq!(paths[0].internal_edges(), [edges["internal-a"]]);
        assert_eq!(paths[0].exit_edge(), edges["exit"]);
        assert_eq!(
            lir.junction_internal_owner(edges["internal-a"]),
            Some(junction.ordinal())
        );
        assert_eq!(lir.junction_internal_owner(edges["entry-a"]), None);
        assert_eq!(
            lir.junction_internal_edges()
                .map(|relation| (relation.edge(), relation.junction()))
                .collect::<Vec<_>>(),
            [(edges["internal-a"], junction.ordinal())]
        );
        assert_eq!(
            movement
                .identity_fields()
                .map(|field| field.tag())
                .collect::<Vec<_>>(),
            [
                FieldTag::AuthoringNamespaceId,
                FieldTag::MovementKey,
                FieldTag::DirectedEntryApproachKey,
                FieldTag::DirectedExitApproachKey,
                FieldTag::JunctionStableId,
            ]
        );
        assert_eq!(
            paths[0]
                .identity_fields()
                .map(|field| field.tag())
                .collect::<Vec<_>>(),
            [
                FieldTag::AuthoringNamespaceId,
                FieldTag::PathKey,
                FieldTag::MovementStableId,
                FieldTag::EntryEdgeStableId,
                FieldTag::ExitEdgeStableId,
            ]
        );

        let source_map = output.source_map_input();
        assert_eq!(source_map.junction_sources().len(), 1);
        assert_eq!(source_map.movement_sources().len(), 1);
        assert_eq!(source_map.maneuver_path_sources().len(), 2);
        assert_eq!(
            source_map
                .junction_relation_sources()
                .map(|source| (
                    source.owner().entity_kind(),
                    source.role(),
                    source.local_index()
                ))
                .collect::<Vec<_>>(),
            [
                (
                    laneflow_static_contract::EntityKind::Junction,
                    SourceRelationRole::JunctionMovement,
                    0,
                ),
                (
                    laneflow_static_contract::EntityKind::Movement,
                    SourceRelationRole::MovementManeuverPath,
                    0,
                ),
                (
                    laneflow_static_contract::EntityKind::Movement,
                    SourceRelationRole::MovementManeuverPath,
                    1,
                ),
                (
                    laneflow_static_contract::EntityKind::ManeuverPath,
                    SourceRelationRole::ManeuverPathEdge,
                    0,
                ),
                (
                    laneflow_static_contract::EntityKind::ManeuverPath,
                    SourceRelationRole::ManeuverPathEdge,
                    1,
                ),
                (
                    laneflow_static_contract::EntityKind::ManeuverPath,
                    SourceRelationRole::ManeuverPathEdge,
                    2,
                ),
                (
                    laneflow_static_contract::EntityKind::ManeuverPath,
                    SourceRelationRole::ManeuverPathEdge,
                    0,
                ),
                (
                    laneflow_static_contract::EntityKind::ManeuverPath,
                    SourceRelationRole::ManeuverPathEdge,
                    1,
                ),
                (
                    laneflow_static_contract::EntityKind::ManeuverPath,
                    SourceRelationRole::ManeuverPathEdge,
                    2,
                ),
                (
                    laneflow_static_contract::EntityKind::Junction,
                    SourceRelationRole::JunctionInternalEdge,
                    0,
                ),
            ]
        );
    }

    #[test]
    fn compiler_accepts_a_direct_maneuver_path_without_internal_edges() {
        let mut builder = junction_builder("direct-path.document");
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "entry",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[LaneEdgeReference::local("exit")],
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "exit",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_junction(JunctionInput {
                junction_key: "junction-main",
            })
            .unwrap()
            .add_movement(MovementInput {
                movement_key: "movement-main",
                junction: JunctionReference::local("junction-main"),
                directed_entry_approach_key: "approach-entry",
                directed_exit_approach_key: "approach-exit",
            })
            .unwrap()
            .add_maneuver_path(ManeuverPathInput {
                maneuver_path_key: "path-direct",
                movement: MovementReference::local("movement-main"),
                entry_edge: LaneEdgeReference::local("entry"),
                internal_edges: &[],
                exit_edge: LaneEdgeReference::local("exit"),
            })
            .unwrap();

        let output = Compiler::new()
            .compile(unit([builder.finish().unwrap()]))
            .unwrap();
        let path = output.lir().maneuver_paths().next().unwrap();
        assert_eq!(path.edges().len(), 2);
        assert!(path.internal_edges().is_empty());
        assert_eq!(output.lir().junction_internal_edges().len(), 0);
    }

    #[test]
    fn junction_lir_is_deterministic_and_path_identity_excludes_internal_edges() {
        let baseline = Compiler::new()
            .compile(unit([junction_module(false, "internal-a")]))
            .unwrap();
        let permuted = Compiler::new()
            .compile(unit([junction_module(true, "internal-a")]))
            .unwrap();
        let different_internal = Compiler::new()
            .compile(unit([junction_module(false, "internal-b")]))
            .unwrap();

        assert_eq!(
            baseline.lir.inner.semantic_digest,
            permuted.lir.inner.semantic_digest
        );
        assert_eq!(
            baseline
                .lir()
                .maneuver_paths()
                .map(|path| path.stable_id())
                .collect::<Vec<_>>(),
            permuted
                .lir()
                .maneuver_paths()
                .map(|path| path.stable_id())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            baseline
                .lir()
                .maneuver_paths()
                .map(|path| path.stable_id())
                .collect::<Vec<_>>(),
            different_internal
                .lir()
                .maneuver_paths()
                .map(|path| path.stable_id())
                .collect::<Vec<_>>()
        );
        assert_ne!(
            baseline.lir.inner.semantic_digest,
            different_internal.lir.inner.semantic_digest
        );
    }

    #[test]
    fn compiler_rejects_junction_topology_semantic_failures_before_lir() {
        let add_junction = |builder: &mut SyntheticModuleBuilder, key: &'static str| {
            builder
                .add_junction(JunctionInput { junction_key: key })
                .unwrap();
        };
        let add_movement =
            |builder: &mut SyntheticModuleBuilder, key: &'static str, junction: &'static str| {
                builder
                    .add_movement(MovementInput {
                        movement_key: key,
                        junction: JunctionReference::local(junction),
                        directed_entry_approach_key: "approach-entry",
                        directed_exit_approach_key: "approach-exit",
                    })
                    .unwrap();
            };
        let add_edge = |builder: &mut SyntheticModuleBuilder,
                        key: &'static str,
                        successors: &[LaneEdgeReference<'static>]| {
            builder
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: key,
                    length_meters: 10.0,
                    speed_limit_meters_per_second: 10.0,
                    successors,
                })
                .unwrap();
        };

        let mut empty_junction = junction_builder("empty-junction.document");
        add_junction(&mut empty_junction, "junction-empty");
        assert!(compile_diagnostic_codes(empty_junction).contains(&DiagnosticCode::EmptyJunction));

        let mut empty_movement = junction_builder("empty-movement.document");
        add_junction(&mut empty_movement, "junction-main");
        add_movement(&mut empty_movement, "movement-empty", "junction-main");
        assert!(compile_diagnostic_codes(empty_movement).contains(&DiagnosticCode::EmptyMovement));

        let mut disconnected = junction_builder("disconnected-path.document");
        add_edge(&mut disconnected, "entry", &[]);
        add_edge(&mut disconnected, "exit", &[]);
        add_junction(&mut disconnected, "junction-main");
        add_movement(&mut disconnected, "movement-main", "junction-main");
        disconnected
            .add_maneuver_path(ManeuverPathInput {
                maneuver_path_key: "path-main",
                movement: MovementReference::local("movement-main"),
                entry_edge: LaneEdgeReference::local("entry"),
                internal_edges: &[],
                exit_edge: LaneEdgeReference::local("exit"),
            })
            .unwrap();
        assert!(
            compile_diagnostic_codes(disconnected)
                .contains(&DiagnosticCode::DisconnectedManeuverPath)
        );

        let mut duplicate = junction_builder("duplicate-path.document");
        add_edge(&mut duplicate, "entry", &[LaneEdgeReference::local("exit")]);
        add_edge(&mut duplicate, "exit", &[]);
        add_junction(&mut duplicate, "junction-main");
        add_movement(&mut duplicate, "movement-main", "junction-main");
        for path_key in ["path-a", "path-b"] {
            duplicate
                .add_maneuver_path(ManeuverPathInput {
                    maneuver_path_key: path_key,
                    movement: MovementReference::local("movement-main"),
                    entry_edge: LaneEdgeReference::local("entry"),
                    internal_edges: &[],
                    exit_edge: LaneEdgeReference::local("exit"),
                })
                .unwrap();
        }
        assert!(
            compile_diagnostic_codes(duplicate)
                .contains(&DiagnosticCode::DuplicateManeuverPathSequence)
        );

        let mut cross_junction = junction_builder("cross-junction-internal.document");
        add_edge(
            &mut cross_junction,
            "entry-a",
            &[LaneEdgeReference::local("internal")],
        );
        add_edge(
            &mut cross_junction,
            "entry-b",
            &[LaneEdgeReference::local("internal")],
        );
        add_edge(
            &mut cross_junction,
            "internal",
            &[
                LaneEdgeReference::local("exit-a"),
                LaneEdgeReference::local("exit-b"),
            ],
        );
        add_edge(&mut cross_junction, "exit-a", &[]);
        add_edge(&mut cross_junction, "exit-b", &[]);
        for suffix in ["a", "b"] {
            let junction_key = if suffix == "a" {
                "junction-a"
            } else {
                "junction-b"
            };
            let movement_key = if suffix == "a" {
                "movement-a"
            } else {
                "movement-b"
            };
            add_junction(&mut cross_junction, junction_key);
            add_movement(&mut cross_junction, movement_key, junction_key);
            let internal = [LaneEdgeReference::local("internal")];
            cross_junction
                .add_maneuver_path(ManeuverPathInput {
                    maneuver_path_key: if suffix == "a" { "path-a" } else { "path-b" },
                    movement: MovementReference::local(movement_key),
                    entry_edge: LaneEdgeReference::local(if suffix == "a" {
                        "entry-a"
                    } else {
                        "entry-b"
                    }),
                    internal_edges: &internal,
                    exit_edge: LaneEdgeReference::local(if suffix == "a" {
                        "exit-a"
                    } else {
                        "exit-b"
                    }),
                })
                .unwrap();
        }
        assert!(
            compile_diagnostic_codes(cross_junction)
                .contains(&DiagnosticCode::InternalEdgeJunctionConflict)
        );

        let mut boundary_conflict = junction_builder("internal-boundary-conflict.document");
        add_edge(
            &mut boundary_conflict,
            "entry",
            &[LaneEdgeReference::local("internal")],
        );
        add_edge(
            &mut boundary_conflict,
            "internal",
            &[
                LaneEdgeReference::local("exit-a"),
                LaneEdgeReference::local("exit-b"),
            ],
        );
        add_edge(&mut boundary_conflict, "exit-a", &[]);
        add_edge(&mut boundary_conflict, "exit-b", &[]);
        add_junction(&mut boundary_conflict, "junction-main");
        add_movement(&mut boundary_conflict, "movement-main", "junction-main");
        let internal = [LaneEdgeReference::local("internal")];
        boundary_conflict
            .add_maneuver_path(ManeuverPathInput {
                maneuver_path_key: "path-with-internal",
                movement: MovementReference::local("movement-main"),
                entry_edge: LaneEdgeReference::local("entry"),
                internal_edges: &internal,
                exit_edge: LaneEdgeReference::local("exit-a"),
            })
            .unwrap()
            .add_maneuver_path(ManeuverPathInput {
                maneuver_path_key: "path-with-boundary",
                movement: MovementReference::local("movement-main"),
                entry_edge: LaneEdgeReference::local("internal"),
                internal_edges: &[],
                exit_edge: LaneEdgeReference::local("exit-b"),
            })
            .unwrap();
        assert!(
            compile_diagnostic_codes(boundary_conflict)
                .contains(&DiagnosticCode::InternalBoundaryRoleConflict)
        );
    }

    #[test]
    fn movement_approach_identity_fields_reject_non_ascii_input_atomically() {
        let mut builder = junction_builder("invalid-approach.document");
        builder
            .add_junction(JunctionInput {
                junction_key: "junction-main",
            })
            .unwrap();
        let diagnostic = match builder.add_movement(MovementInput {
            movement_key: "movement-main",
            junction: JunctionReference::local("junction-main"),
            directed_entry_approach_key: "入口",
            directed_exit_approach_key: "approach-exit",
        }) {
            Ok(_) => panic!("non-ASCII identity field must reject the declaration"),
            Err(diagnostic) => diagnostic,
        };
        assert_eq!(
            diagnostic.diagnostics()[0].code(),
            DiagnosticCode::InvalidIdentityAsciiField
        );
        // 同一个稳定键仍可被合法声明，证明失败路径没有预占符号或部分提交资源计数。
        builder
            .add_movement(MovementInput {
                movement_key: "movement-main",
                junction: JunctionReference::local("junction-main"),
                directed_entry_approach_key: "approach-entry",
                directed_exit_approach_key: "approach-exit",
            })
            .unwrap();
    }

    #[test]
    fn compiler_rejects_cross_section_semantic_failures_before_lir() {
        let limits = CompileLimits::p100_initial_v1();
        let make_builder = || {
            let header = SourceModuleHeader::new(
                SourceModuleHeaderInput {
                    authoring_namespace_id: "city/failure",
                    source_document_key: "failure.document",
                    generator_build_id: "git:0123456789abcdef",
                    parameters_and_inputs_digest: [0x11; 32],
                    frontend_options_digest: [0x22; 32],
                    random_seed: None,
                    provenance: "repository:laneflow",
                },
                &limits,
            )
            .unwrap();
            SyntheticModuleBuilder::new(header, &limits).unwrap()
        };

        let mut missing_owner = make_builder();
        missing_owner
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge-a",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_road_section(RoadSectionInput {
                road_section_key: "section-a",
                kind_id: "motorLane",
                lanes: &[AuthoringLaneInput {
                    authoring_lane_key: "lane-a",
                    edge_chain: &[LaneEdgeReference::local("edge-a")],
                    lane_group: None,
                }],
            })
            .unwrap();
        let diagnostics = match Compiler::new().compile(unit([missing_owner.finish().unwrap()])) {
            Ok(_) => panic!("missing owner must reject compilation"),
            Err(diagnostics) => diagnostics,
        };
        assert_eq!(
            diagnostics.diagnostics()[0].code(),
            DiagnosticCode::MissingCrossSectionOwner
        );

        let mut disconnected = make_builder();
        disconnected
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge-a",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge-b",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_road_section(RoadSectionInput {
                road_section_key: "section-a",
                kind_id: "motorLane",
                lanes: &[AuthoringLaneInput {
                    authoring_lane_key: "lane-a",
                    edge_chain: &[
                        LaneEdgeReference::local("edge-a"),
                        LaneEdgeReference::local("edge-b"),
                    ],
                    lane_group: None,
                }],
            })
            .unwrap()
            .add_road_corridor(RoadCorridorInput {
                road_corridor_key: "corridor-a",
                reference_section: RoadSectionReference::local("section-a"),
                elements: &[CorridorElementReference::road_section(
                    RoadSectionReference::local("section-a"),
                )],
            })
            .unwrap();
        let diagnostics = match Compiler::new().compile(unit([disconnected.finish().unwrap()])) {
            Ok(_) => panic!("disconnected lane chain must reject compilation"),
            Err(diagnostics) => diagnostics,
        };
        assert!(diagnostics.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == DiagnosticCode::DisconnectedAuthoringLaneEdgeChain
        }));

        let mut unknown_middle = make_builder();
        unknown_middle
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge-a",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge-c",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_road_section(RoadSectionInput {
                road_section_key: "section-a",
                kind_id: "motorLane",
                lanes: &[AuthoringLaneInput {
                    authoring_lane_key: "lane-a",
                    edge_chain: &[
                        LaneEdgeReference::local("edge-a"),
                        LaneEdgeReference::local("missing"),
                        LaneEdgeReference::local("edge-c"),
                    ],
                    lane_group: None,
                }],
            })
            .unwrap()
            .add_road_corridor(RoadCorridorInput {
                road_corridor_key: "corridor-a",
                reference_section: RoadSectionReference::local("section-a"),
                elements: &[CorridorElementReference::road_section(
                    RoadSectionReference::local("section-a"),
                )],
            })
            .unwrap();
        let diagnostics = match Compiler::new().compile(unit([unknown_middle.finish().unwrap()])) {
            Ok(_) => panic!("unknown lane edge must reject compilation"),
            Err(diagnostics) => diagnostics,
        };
        assert!(
            diagnostics
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == DiagnosticCode::UnknownReferenceTarget)
        );
        assert!(diagnostics.diagnostics().iter().all(|diagnostic| {
            diagnostic.code() != DiagnosticCode::DisconnectedAuthoringLaneEdgeChain
        }));

        let mut multiple_owner = make_builder();
        multiple_owner
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge-a",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_road_section(RoadSectionInput {
                road_section_key: "section-a",
                kind_id: "motorLane",
                lanes: &[AuthoringLaneInput {
                    authoring_lane_key: "lane-a",
                    edge_chain: &[LaneEdgeReference::local("edge-a")],
                    lane_group: None,
                }],
            })
            .unwrap()
            .add_road_corridor(RoadCorridorInput {
                road_corridor_key: "corridor-a",
                reference_section: RoadSectionReference::local("section-a"),
                elements: &[CorridorElementReference::road_section(
                    RoadSectionReference::local("section-a"),
                )],
            })
            .unwrap()
            .add_road_corridor(RoadCorridorInput {
                road_corridor_key: "corridor-b",
                reference_section: RoadSectionReference::local("section-a"),
                elements: &[CorridorElementReference::road_section(
                    RoadSectionReference::local("section-a"),
                )],
            })
            .unwrap();
        let diagnostics = match Compiler::new().compile(unit([multiple_owner.finish().unwrap()])) {
            Ok(_) => panic!("multiple cross-section owners must reject compilation"),
            Err(diagnostics) => diagnostics,
        };
        assert!(
            diagnostics.diagnostics().iter().any(|diagnostic| {
                diagnostic.code() == DiagnosticCode::MultipleCrossSectionOwners
            })
        );

        let mut group_parent_mismatch = make_builder();
        group_parent_mismatch
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge-a",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge-b",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_lane_group(LaneGroupInput {
                lane_group_key: "group-a",
                road_section: RoadSectionReference::local("section-a"),
            })
            .unwrap()
            .add_road_section(RoadSectionInput {
                road_section_key: "section-a",
                kind_id: "motorLane",
                lanes: &[AuthoringLaneInput {
                    authoring_lane_key: "lane-a",
                    edge_chain: &[LaneEdgeReference::local("edge-a")],
                    lane_group: None,
                }],
            })
            .unwrap()
            .add_road_section(RoadSectionInput {
                road_section_key: "section-b",
                kind_id: "motorLane",
                lanes: &[AuthoringLaneInput {
                    authoring_lane_key: "lane-b",
                    edge_chain: &[LaneEdgeReference::local("edge-b")],
                    lane_group: Some(LaneGroupReference::local("group-a")),
                }],
            })
            .unwrap()
            .add_road_corridor(RoadCorridorInput {
                road_corridor_key: "corridor-a",
                reference_section: RoadSectionReference::local("section-a"),
                elements: &[
                    CorridorElementReference::road_section(RoadSectionReference::local(
                        "section-a",
                    )),
                    CorridorElementReference::road_section(RoadSectionReference::local(
                        "section-b",
                    )),
                ],
            })
            .unwrap();
        let diagnostics =
            match Compiler::new().compile(unit([group_parent_mismatch.finish().unwrap()])) {
                Ok(_) => panic!("lane-group parent mismatch must reject compilation"),
                Err(diagnostics) => diagnostics,
            };
        assert!(
            diagnostics
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == DiagnosticCode::LaneGroupParentMismatch)
        );
    }

    #[test]
    fn source_changes_do_not_change_lir_semantic_digest() {
        let left = unit([module(
            "city/a",
            "left.document",
            &[],
            &[("edge-a", 10.0, &[])],
        )]);
        let right = unit([module(
            "city/a",
            "right.document",
            &[],
            &[("edge-a", 10.0, &[])],
        )]);
        let mut compiler = Compiler::new();
        let left = compiler.compile(left).unwrap();
        let right = compiler.compile(right).unwrap();

        assert_eq!(
            left.lir.inner.semantic_digest,
            right.lir.inner.semantic_digest
        );
        assert_ne!(
            left.source_map_input()
                .lane_edge_sources()
                .next()
                .unwrap()
                .primary_source()
                .source_document_key(),
            right
                .source_map_input()
                .lane_edge_sources()
                .next()
                .unwrap()
                .primary_source()
                .source_document_key()
        );
    }

    #[test]
    fn thirty_two_failures_do_not_pollute_reused_compiler() {
        let missing = [LaneEdgeReference::local("missing")];
        let mut compiler = Compiler::new();
        for index in 0..32 {
            let failed = unit([module(
                &format!("failed/{index}"),
                &format!("failed-{index}.document"),
                &[],
                &[("edge-a", 10.0, &missing)],
            )]);
            let diagnostics = match compiler.compile(failed) {
                Ok(_) => panic!("expected failed compilation"),
                Err(diagnostics) => diagnostics,
            };
            assert!(matches!(
                diagnostics.diagnostics()[0].payload(),
                DiagnosticPayload::UnknownReferenceTarget { .. }
            ));
        }

        let recovered = unit([module(
            "city/a",
            "city-a.document",
            &[],
            &[("edge-a", 10.0, &[])],
        )]);
        let fresh = unit([module(
            "city/a",
            "city-a.document",
            &[],
            &[("edge-a", 10.0, &[])],
        )]);
        assert_eq!(
            compiler
                .compile(recovered)
                .unwrap()
                .lir
                .inner
                .semantic_digest,
            Compiler::new()
                .compile(fresh)
                .unwrap()
                .lir
                .inner
                .semantic_digest
        );
    }

    #[test]
    fn source_map_output_limit_fails_after_lir_without_exposing_partial_output() {
        let probe = unit([module(
            "city/a",
            "city-a.document",
            &[],
            &[("edge-a", 10.0, &[])],
        )]);
        let hir = build_hir(&probe).unwrap();
        let mir = lower_to_mir(&probe, &hir).unwrap();
        let lir_output_bytes = freeze_lir(&probe, &mir).unwrap().lir.output_bytes;

        let mut constrained = unit([module(
            "city/a",
            "city-a.document",
            &[],
            &[("edge-a", 10.0, &[])],
        )]);
        constrained.limits = CompileLimits::p100_initial_v1().with_test_lir_limits(
            u32::MAX,
            u32::MAX,
            u32::try_from(lir_output_bytes).unwrap(),
            u32::MAX,
        );
        let mut compiler = Compiler::new();
        let diagnostics = match compiler.compile(constrained) {
            Ok(_) => panic!("expected source-map output limit failure"),
            Err(diagnostics) => diagnostics,
        };
        assert!(diagnostics.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::OutputBytes,
                limit,
                observed,
            } if *limit == lir_output_bytes && observed > limit
        )));

        let recovered = unit([module(
            "city/recovered",
            "recovered.document",
            &[],
            &[("edge-a", 10.0, &[])],
        )]);
        assert!(compiler.compile(recovered).is_ok());
    }

    #[test]
    fn compiler_freezes_gate_stop_line_and_waiting_zone_closure() {
        let mut builder = control_builder("control.document");
        add_valid_control(&mut builder, false);
        let output = Compiler::new()
            .compile(unit([builder.finish().unwrap()]))
            .unwrap();
        let lir = output.lir();

        assert_eq!(lir.stop_lines().len(), 2);
        assert_eq!(lir.maneuver_gates().len(), 2);
        assert_eq!(lir.waiting_zones().len(), 1);
        let path = lir.maneuver_paths().next().unwrap();
        let gates = path.maneuver_gates();
        assert_eq!(gates.len(), 2);
        assert_eq!(lir.maneuver_gate(gates[0]).unwrap().transition_index(), 0);
        assert_eq!(lir.maneuver_gate(gates[1]).unwrap().transition_index(), 1);
        let waiting = lir.waiting_zones().next().unwrap();
        assert_eq!(waiting.maneuver_path(), path.ordinal());
        assert_eq!(waiting.entry_gate(), gates[0]);
        assert_eq!(waiting.release_gate(), gates[1]);
        assert_eq!(waiting.max_occupancy(), 3);
        assert_eq!(path.waiting_zones(), &[waiting.ordinal()]);

        for gate in lir.maneuver_gates() {
            let stop_line = lir.stop_line(gate.stop_line()).unwrap();
            assert_eq!(stop_line.maneuver_gates(), &[gate.ordinal()]);
            assert_eq!(
                stop_line.lane_edge(),
                path.edges()[gate.transition_index() as usize]
            );
        }

        let source_map = output.source_map_input();
        assert_eq!(source_map.stop_line_sources().len(), 2);
        assert_eq!(source_map.maneuver_gate_sources().len(), 2);
        assert_eq!(source_map.waiting_zone_sources().len(), 1);
        let roles = source_map
            .junction_relation_sources()
            .map(|source| source.role())
            .collect::<Vec<_>>();
        assert_eq!(
            roles
                .iter()
                .filter(|role| **role == SourceRelationRole::ManeuverPathGate)
                .count(),
            2
        );
        assert_eq!(
            roles
                .iter()
                .filter(|role| **role == SourceRelationRole::ManeuverPathWaitingZone)
                .count(),
            1
        );
        assert_eq!(
            roles
                .iter()
                .filter(|role| **role == SourceRelationRole::StopLineManeuverGate)
                .count(),
            2
        );
    }

    #[test]
    fn compiler_precompiles_static_route_control_occurrences_and_reverse_indexes() {
        let mut builder = control_builder("static-route.document");
        add_valid_control(&mut builder, false);
        builder
            .add_static_route(StaticRouteInput {
                static_route_key: "route-main",
                edge_sequence: &[
                    LaneEdgeReference::local("entry"),
                    LaneEdgeReference::local("middle"),
                    LaneEdgeReference::local("exit"),
                ],
            })
            .unwrap();
        let output = Compiler::new()
            .compile(unit([builder.finish().unwrap()]))
            .unwrap();
        let lir = output.lir();
        let route = lir.static_routes().next().unwrap();
        let path = lir.maneuver_paths().next().unwrap();
        let path_gates = path.maneuver_gates();
        let waiting = lir.waiting_zones().next().unwrap();

        assert_eq!(route.edges(), path.edges());
        assert_eq!(
            route.transition_gates().collect::<Vec<_>>(),
            [Some(path_gates[0]), Some(path_gates[1])]
        );
        let maneuvers = route.maneuver_occurrences().collect::<Vec<_>>();
        assert_eq!(maneuvers.len(), 1);
        assert_eq!(maneuvers[0].maneuver_path(), path.ordinal());
        assert_eq!(maneuvers[0].entry_route_edge_index(), 0);
        assert_eq!(maneuvers[0].exit_route_edge_index(), 2);
        assert_eq!(maneuvers[0].gate_occurrence_range(), 0..2);
        assert_eq!(maneuvers[0].waiting_zone_occurrence_range(), 0..1);

        let gates = route.gate_occurrences().collect::<Vec<_>>();
        assert_eq!(gates.len(), 2);
        assert_eq!(gates[0].maneuver_gate(), path_gates[0]);
        assert_eq!(gates[0].next_gate_occurrence_index(), Some(1));
        assert_eq!(gates[0].next_boundary_route_edge_index(), 1);
        assert_eq!(gates[0].waiting_zone_occurrence_index(), Some(0));
        assert_eq!(gates[1].maneuver_gate(), path_gates[1]);
        assert_eq!(gates[1].next_gate_occurrence_index(), None);
        assert_eq!(gates[1].next_boundary_route_edge_index(), 2);

        let waiting_occurrences = route.waiting_zone_occurrences().collect::<Vec<_>>();
        assert_eq!(waiting_occurrences.len(), 1);
        assert_eq!(waiting_occurrences[0].waiting_zone(), waiting.ordinal());
        assert_eq!(waiting_occurrences[0].entry_gate_occurrence_index(), 0);
        assert_eq!(waiting_occurrences[0].release_gate_occurrence_index(), 1);
        assert_eq!(waiting_occurrences[0].entry_route_edge_index(), 0);
        assert_eq!(waiting_occurrences[0].release_route_edge_index(), 1);

        for (edge_index, edge) in route.edges().iter().copied().enumerate() {
            assert_eq!(
                lir.lane_edge(edge)
                    .unwrap()
                    .static_route_occurrences()
                    .collect::<Vec<_>>(),
                [CanonicalStaticRouteOccurrenceRef {
                    static_route: route.ordinal(),
                    occurrence_index: edge_index as u32,
                }]
            );
        }
        assert_eq!(path.static_route_occurrences().len(), 1);
        assert_eq!(
            lir.maneuver_gate(path_gates[0])
                .unwrap()
                .static_route_occurrences()
                .len(),
            1
        );
        assert_eq!(waiting.static_route_occurrences().len(), 1);

        let source_map = output.source_map_input();
        assert_eq!(source_map.static_route_sources().len(), 1);
        let route_sources = source_map.route_relation_sources().collect::<Vec<_>>();
        assert_eq!(
            route_sources
                .iter()
                .map(|source| source.role())
                .collect::<Vec<_>>(),
            [
                SourceRelationRole::StaticRouteEdge,
                SourceRelationRole::StaticRouteEdge,
                SourceRelationRole::StaticRouteEdge,
                SourceRelationRole::StaticRouteManeuverOccurrence,
                SourceRelationRole::StaticRouteGateOccurrence,
                SourceRelationRole::StaticRouteGateOccurrence,
                SourceRelationRole::StaticRouteWaitingZoneOccurrence,
            ]
        );
        assert!(
            route_sources[..3]
                .iter()
                .all(|source| source.contributing_sources().len() == 0)
        );
        assert!(
            route_sources[3..]
                .iter()
                .all(|source| source.contributing_sources().len() == 1)
        );
    }

    #[test]
    fn static_route_preserves_repeated_edge_occurrences() {
        let mut builder = junction_builder("static-route-repeated-edge.document");
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "loop",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[
                    LaneEdgeReference::local("loop"),
                    LaneEdgeReference::local("exit"),
                ],
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "exit",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_static_route(StaticRouteInput {
                static_route_key: "route-loop",
                edge_sequence: &[
                    LaneEdgeReference::local("loop"),
                    LaneEdgeReference::local("loop"),
                    LaneEdgeReference::local("exit"),
                ],
            })
            .unwrap();

        let output = Compiler::new()
            .compile(unit([builder.finish().unwrap()]))
            .unwrap();
        let lir = output.lir();
        let route = lir.static_routes().next().unwrap();
        assert_eq!(route.edges()[0], route.edges()[1]);
        assert_ne!(route.edges()[1], route.edges()[2]);
        assert_eq!(
            lir.lane_edge(route.edges()[0])
                .unwrap()
                .static_route_occurrences()
                .collect::<Vec<_>>(),
            [
                CanonicalStaticRouteOccurrenceRef {
                    static_route: route.ordinal(),
                    occurrence_index: 0,
                },
                CanonicalStaticRouteOccurrenceRef {
                    static_route: route.ordinal(),
                    occurrence_index: 1,
                },
            ]
        );
        assert_eq!(
            lir.lane_edge(route.edges()[2])
                .unwrap()
                .static_route_occurrences()
                .collect::<Vec<_>>(),
            [CanonicalStaticRouteOccurrenceRef {
                static_route: route.ordinal(),
                occurrence_index: 2,
            }]
        );
    }

    #[test]
    fn static_route_limit_failure_preserves_the_builder() {
        let mut builder = junction_builder("static-route-limit.document");
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "loop",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[LaneEdgeReference::local("loop")],
            })
            .unwrap();
        let over_limit = vec![
            LaneEdgeReference::local("loop");
            usize::try_from(
                CompileLimits::p100_initial_v1().value(CompileLimitDimension::RouteOccurrenceCount)
            )
            .unwrap()
                + 1
        ];
        let diagnostics = match builder.add_static_route(StaticRouteInput {
            static_route_key: "route-over-limit",
            edge_sequence: &over_limit,
        }) {
            Ok(_) => panic!("route occurrence limit must fail before owning the input"),
            Err(diagnostics) => diagnostics,
        };
        assert!(matches!(
            diagnostics.diagnostics()[0].payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::RouteOccurrenceCount,
                limit: 1_920,
                observed: 1_921,
            }
        ));

        builder
            .add_static_route(StaticRouteInput {
                static_route_key: "route-valid-after-failure",
                edge_sequence: &[LaneEdgeReference::local("loop")],
            })
            .unwrap();
        assert!(
            Compiler::new()
                .compile(unit([builder.finish().unwrap()]))
                .is_ok()
        );
    }

    #[test]
    fn static_route_semantics_ignore_control_and_route_declaration_order() {
        let mut left = control_builder("static-route-left.document");
        add_valid_control(&mut left, false);
        left.add_static_route(StaticRouteInput {
            static_route_key: "route-main",
            edge_sequence: &[
                LaneEdgeReference::local("entry"),
                LaneEdgeReference::local("middle"),
                LaneEdgeReference::local("exit"),
            ],
        })
        .unwrap();

        let mut right = control_builder("static-route-right.document");
        right
            .add_static_route(StaticRouteInput {
                static_route_key: "route-main",
                edge_sequence: &[
                    LaneEdgeReference::local("entry"),
                    LaneEdgeReference::local("middle"),
                    LaneEdgeReference::local("exit"),
                ],
            })
            .unwrap();
        add_valid_control(&mut right, true);

        let left = Compiler::new()
            .compile(unit([left.finish().unwrap()]))
            .unwrap();
        let right = Compiler::new()
            .compile(unit([right.finish().unwrap()]))
            .unwrap();
        assert_eq!(
            left.lir.inner.semantic_digest,
            right.lir.inner.semantic_digest
        );
        assert_eq!(
            left.lir()
                .static_routes()
                .map(|route| (route.stable_id(), route.edges().to_vec()))
                .collect::<Vec<_>>(),
            right
                .lir()
                .static_routes()
                .map(|route| (route.stable_id(), route.edges().to_vec()))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn static_route_frontend_and_hir_reject_empty_disconnected_and_terminal_control() {
        let mut empty = junction_builder("static-route-empty.document");
        let diagnostics = match empty.add_static_route(StaticRouteInput {
            static_route_key: "route-empty",
            edge_sequence: &[],
        }) {
            Ok(_) => panic!("empty route must fail before mutation"),
            Err(diagnostics) => diagnostics,
        };
        assert_eq!(
            diagnostics.diagnostics()[0].code(),
            DiagnosticCode::EmptyStaticRoute
        );

        let mut disconnected = junction_builder("static-route-disconnected.document");
        disconnected
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "left",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "right",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_static_route(StaticRouteInput {
                static_route_key: "route-disconnected",
                edge_sequence: &[
                    LaneEdgeReference::local("left"),
                    LaneEdgeReference::local("right"),
                ],
            })
            .unwrap();
        assert!(
            compile_diagnostic_codes(disconnected)
                .contains(&DiagnosticCode::DisconnectedStaticRouteEdge)
        );

        let mut terminal = control_builder("static-route-terminal.document");
        add_valid_control(&mut terminal, false);
        terminal
            .add_static_route(StaticRouteInput {
                static_route_key: "route-terminal",
                edge_sequence: &[LaneEdgeReference::local("entry")],
            })
            .unwrap();
        assert!(
            compile_diagnostic_codes(terminal)
                .contains(&DiagnosticCode::StaticRouteTerminatesAtStopLine)
        );

        let mut boundaries = route_validation_builder("static-route-boundaries.document");
        boundaries
            .add_static_route(StaticRouteInput {
                static_route_key: "route-starts-inside",
                edge_sequence: &[
                    LaneEdgeReference::local("middle"),
                    LaneEdgeReference::local("exit"),
                ],
            })
            .unwrap()
            .add_static_route(StaticRouteInput {
                static_route_key: "route-ends-inside",
                edge_sequence: &[
                    LaneEdgeReference::local("entry"),
                    LaneEdgeReference::local("middle"),
                ],
            })
            .unwrap()
            .add_static_route(StaticRouteInput {
                static_route_key: "route-no-full-match",
                edge_sequence: &[
                    LaneEdgeReference::local("entry"),
                    LaneEdgeReference::local("middle"),
                    LaneEdgeReference::local("detour"),
                ],
            })
            .unwrap()
            .add_static_route(StaticRouteInput {
                static_route_key: "route-uncovered-internal",
                edge_sequence: &[
                    LaneEdgeReference::local("other"),
                    LaneEdgeReference::local("middle"),
                    LaneEdgeReference::local("exit"),
                ],
            })
            .unwrap();
        let codes = compile_diagnostic_codes(boundaries);
        assert!(codes.contains(&DiagnosticCode::StaticRouteStartsInsideJunction));
        assert!(codes.contains(&DiagnosticCode::StaticRouteEndsInsideJunction));
        assert!(codes.contains(&DiagnosticCode::StaticRouteManeuverNoFullMatch));
        assert!(codes.contains(&DiagnosticCode::StaticRouteInternalEdgeUncovered));
    }

    #[test]
    fn control_semantics_are_invariant_to_declaration_permutation() {
        let mut left = control_builder("control-left.document");
        add_valid_control(&mut left, false);
        let mut right = control_builder("control-right.document");
        add_valid_control(&mut right, true);
        let left = Compiler::new()
            .compile(unit([left.finish().unwrap()]))
            .unwrap();
        let right = Compiler::new()
            .compile(unit([right.finish().unwrap()]))
            .unwrap();

        assert_eq!(
            left.lir.inner.semantic_digest,
            right.lir.inner.semantic_digest
        );
        assert_eq!(
            left.lir()
                .maneuver_gates()
                .map(|gate| (gate.stable_id(), gate.transition_index()))
                .collect::<Vec<_>>(),
            right
                .lir()
                .maneuver_gates()
                .map(|gate| (gate.stable_id(), gate.transition_index()))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn control_closure_rejects_invalid_gate_and_stop_line_topology() {
        let mut out_of_range = control_builder("gate-range.document");
        out_of_range
            .add_stop_line(StopLineInput {
                stop_line_key: "stop-entry",
                lane_edge: LaneEdgeReference::local("entry"),
            })
            .unwrap()
            .add_maneuver_gate(ManeuverGateInput {
                maneuver_gate_key: "gate-invalid",
                maneuver_path: ManeuverPathReference::local("path-main"),
                transition_index: 2,
                stop_line: StopLineReference::local("stop-entry"),
                signal_control: SignalControlInput::None,
            })
            .unwrap();
        assert!(
            compile_diagnostic_codes(out_of_range)
                .contains(&DiagnosticCode::ManeuverGateTransitionOutOfRange)
        );

        let mut duplicate = control_builder("gate-duplicate.document");
        duplicate
            .add_stop_line(StopLineInput {
                stop_line_key: "stop-entry",
                lane_edge: LaneEdgeReference::local("entry"),
            })
            .unwrap();
        for key in ["gate-a", "gate-b"] {
            duplicate
                .add_maneuver_gate(ManeuverGateInput {
                    maneuver_gate_key: key,
                    maneuver_path: ManeuverPathReference::local("path-main"),
                    transition_index: 0,
                    stop_line: StopLineReference::local("stop-entry"),
                    signal_control: SignalControlInput::None,
                })
                .unwrap();
        }
        assert!(
            compile_diagnostic_codes(duplicate)
                .contains(&DiagnosticCode::DuplicateManeuverGatePathTransition)
        );

        let mut mismatch = control_builder("gate-mismatch.document");
        mismatch
            .add_stop_line(StopLineInput {
                stop_line_key: "stop-middle",
                lane_edge: LaneEdgeReference::local("middle"),
            })
            .unwrap()
            .add_maneuver_gate(ManeuverGateInput {
                maneuver_gate_key: "gate-entry",
                maneuver_path: ManeuverPathReference::local("path-main"),
                transition_index: 0,
                stop_line: StopLineReference::local("stop-middle"),
                signal_control: SignalControlInput::None,
            })
            .unwrap();
        assert!(
            compile_diagnostic_codes(mismatch)
                .contains(&DiagnosticCode::ManeuverGateStopLineMismatch)
        );

        let mut orphan = control_builder("stop-orphan.document");
        orphan
            .add_stop_line(StopLineInput {
                stop_line_key: "stop-exit",
                lane_edge: LaneEdgeReference::local("exit"),
            })
            .unwrap();
        assert!(compile_diagnostic_codes(orphan).contains(&DiagnosticCode::OrphanStopLine));

        let mut unreferenced = control_builder("stop-unreferenced.document");
        unreferenced
            .add_stop_line(StopLineInput {
                stop_line_key: "stop-entry",
                lane_edge: LaneEdgeReference::local("entry"),
            })
            .unwrap();
        assert!(
            compile_diagnostic_codes(unreferenced).contains(&DiagnosticCode::UnreferencedStopLine)
        );
    }

    #[test]
    fn waiting_zone_validation_rejects_zero_reverse_and_overlap() {
        let mut zero = control_builder("waiting-zero.document");
        let diagnostics = match zero.add_waiting_zone(WaitingZoneInput {
            waiting_zone_key: "waiting-zero",
            maneuver_path: ManeuverPathReference::local("path-main"),
            entry_gate: ManeuverGateReference::local("gate-entry"),
            release_gate: ManeuverGateReference::local("gate-release"),
            max_occupancy: 0,
        }) {
            Ok(_) => panic!("zero waiting-zone capacity must fail"),
            Err(diagnostics) => diagnostics,
        };
        assert_eq!(
            diagnostics.diagnostics()[0].code(),
            DiagnosticCode::InvalidWaitingZoneCapacity
        );

        let mut reverse = control_builder("waiting-reverse.document");
        add_valid_control(&mut reverse, false);
        reverse
            .add_waiting_zone(WaitingZoneInput {
                waiting_zone_key: "waiting-reverse",
                maneuver_path: ManeuverPathReference::local("path-main"),
                entry_gate: ManeuverGateReference::local("gate-release"),
                release_gate: ManeuverGateReference::local("gate-entry"),
                max_occupancy: 1,
            })
            .unwrap();
        assert!(
            compile_diagnostic_codes(reverse)
                .contains(&DiagnosticCode::InvalidWaitingZoneGateOrder)
        );

        let mut overlap = control_builder("waiting-overlap.document");
        add_valid_control(&mut overlap, false);
        overlap
            .add_waiting_zone(WaitingZoneInput {
                waiting_zone_key: "waiting-overlap",
                maneuver_path: ManeuverPathReference::local("path-main"),
                entry_gate: ManeuverGateReference::local("gate-entry"),
                release_gate: ManeuverGateReference::local("gate-release"),
                max_occupancy: 1,
            })
            .unwrap();
        assert!(
            compile_diagnostic_codes(overlap).contains(&DiagnosticCode::OverlappingWaitingZones)
        );
    }

    #[test]
    fn parking_static_contract_freezes_area_standalone_space_and_source_roles() {
        let output = Compiler::new()
            .compile(unit([parking_module(
                "parking.document",
                "area-main",
                false,
            )]))
            .unwrap();
        let area = output.lir().parking_areas().next().unwrap();
        let spaces = output.lir().parking_spaces().collect::<Vec<_>>();
        assert_eq!(spaces.len(), 2);
        let owned = spaces
            .iter()
            .copied()
            .find(|space| {
                stable_key(space.identity_fields(), FieldTag::ParkingSpaceKey) == "space-owned"
            })
            .unwrap();
        let independent = spaces
            .iter()
            .copied()
            .find(|space| {
                stable_key(space.identity_fields(), FieldTag::ParkingSpaceKey)
                    == "space-independent"
            })
            .unwrap();

        assert_eq!(area.parking_spaces(), [owned.ordinal()]);
        assert_eq!(owned.parking_area(), Some(area.ordinal()));
        assert_eq!(independent.parking_area(), None);
        assert_eq!(owned.entry().progress_meters(), 4.0);
        assert_eq!(owned.exit().progress_meters(), 6.0);
        assert_ne!(owned.entry().lane_edge(), owned.exit().lane_edge());
        assert_eq!(owned.geometry().lateral_offset_meters(), -3.0);
        assert_eq!(owned.geometry().heading_offset_radians(), 0.25);
        assert_eq!(owned.geometry().length_meters(), 5.5);
        assert_eq!(owned.geometry().width_meters(), 2.6);

        assert_eq!(output.source_map_input().parking_area_sources().len(), 1);
        assert_eq!(output.source_map_input().parking_space_sources().len(), 2);
        let roles = output
            .source_map_input()
            .parking_relation_sources()
            .map(|source| source.role())
            .collect::<Vec<_>>();
        assert_eq!(
            roles,
            [
                SourceRelationRole::ParkingSpaceArea,
                SourceRelationRole::ParkingSpaceEntry,
                SourceRelationRole::ParkingSpaceExit,
                SourceRelationRole::ParkingSpaceEntry,
                SourceRelationRole::ParkingSpaceExit,
            ]
        );
    }

    #[test]
    fn parking_identity_and_digest_obey_set_and_organizational_semantics() {
        let first = Compiler::new()
            .compile(unit([parking_module(
                "parking-a.document",
                "area-a",
                false,
            )]))
            .unwrap();
        let permuted = Compiler::new()
            .compile(unit([parking_module("parking-b.document", "area-a", true)]))
            .unwrap();
        assert_eq!(
            first.lir.inner.semantic_digest,
            permuted.lir.inner.semantic_digest
        );

        let reassigned = Compiler::new()
            .compile(unit([parking_module(
                "parking-c.document",
                "area-b",
                false,
            )]))
            .unwrap();
        let owned_id = |output: &CompilationOutput| {
            output
                .lir()
                .parking_spaces()
                .find(|space| {
                    stable_key(space.identity_fields(), FieldTag::ParkingSpaceKey) == "space-owned"
                })
                .unwrap()
                .stable_id()
        };
        assert_eq!(owned_id(&first), owned_id(&reassigned));
        assert_ne!(
            first.lir().parking_areas().next().unwrap().stable_id(),
            reassigned.lir().parking_areas().next().unwrap().stable_id()
        );
    }

    #[test]
    fn parking_validation_rejects_orphan_anchor_and_geometry_failures() {
        let mut orphan = parking_builder("parking-orphan.document");
        orphan
            .add_parking_area(ParkingAreaInput {
                parking_area_key: "area-orphan",
            })
            .unwrap();
        assert_eq!(
            compile_diagnostic_codes(orphan),
            [DiagnosticCode::OrphanParkingArea]
        );

        let mut invalid = parking_builder("parking-invalid.document");
        add_parking_edges(&mut invalid);
        invalid
            .add_parking_area(ParkingAreaInput {
                parking_area_key: "area-main",
            })
            .unwrap()
            .add_parking_space(ParkingSpaceInput {
                parking_space_key: "space-invalid",
                parking_area: Some(ParkingAreaReference::local("area-main")),
                entry: ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local("parking-entry"),
                    progress_meters: 0.0,
                },
                exit: ParkingLaneAnchorInput {
                    lane_edge: LaneEdgeReference::local("parking-exit"),
                    progress_meters: 20.0,
                },
                geometry: ParkingSpaceGeometryInput {
                    lateral_offset_meters: 0.0,
                    heading_offset_radians: core::f64::consts::PI,
                    length_meters: 0.0,
                    width_meters: f64::INFINITY,
                },
            })
            .unwrap();
        let codes = compile_diagnostic_codes(invalid);
        assert_eq!(
            codes
                .iter()
                .filter(|code| **code == DiagnosticCode::InvalidParkingAnchorProgress)
                .count(),
            2
        );
        assert_eq!(
            codes
                .iter()
                .filter(|code| **code == DiagnosticCode::InvalidParkingSpaceGeometry)
                .count(),
            4
        );
        assert!(!codes.contains(&DiagnosticCode::OrphanParkingArea));
    }
}
