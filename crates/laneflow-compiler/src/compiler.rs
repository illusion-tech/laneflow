//! 官方来源编译到原子已验证输出的公共入口。
//!
//! [`Compiler::compile`] 是唯一能够构造 [`ValidatedCanonicalLir`]、
//! [`ValidatedSourceMapInput`] 和 [`CompilationOutput`] 的路径。当前实现是干净单工作线程
//! 确定性预言机：每个阶段成功后才提交下一阶段，任一错误只返回
//! [`DiagnosticBundle`]；来源伴随数据在 AST/HIR/MIR 释放前冻结。

use laneflow_static_contract::{
    AccessEffect, AccessRuleId, AccessRuleOrdinal, AuthoringLaneId, AuthoringLaneOrdinal,
    CanonicalFrameId, CanonicalFrameOrdinal, FacilityBandId, FacilityBandOrdinal, FieldTag,
    JunctionId, JunctionOrdinal, LaneEdgeId, LaneEdgeOrdinal, LaneGroupId, LaneGroupOrdinal,
    ManeuverGateId, ManeuverGateOrdinal, ManeuverPathId, ManeuverPathOrdinal, MovementId,
    MovementOrdinal, ParkingAreaId, ParkingAreaOrdinal, ParkingSpaceId, ParkingSpaceOrdinal,
    ParticipantClassId, ParticipantClassOrdinal, RoadCorridorId, RoadCorridorOrdinal,
    RoadSectionId, RoadSectionOrdinal, SignalAspect, SignalControllerId, SignalControllerOrdinal,
    SignalGroupId, SignalGroupOrdinal, SignalPhaseId, SignalPhaseOrdinal, StaticRouteId,
    StaticRouteOrdinal, StopLineId, StopLineOrdinal, VehicleProfileId, VehicleProfileOrdinal,
    WaitingZoneId, WaitingZoneOrdinal,
};

use crate::hir::build_hir;
use crate::lir::{
    LirAccessRule, LirAccessTarget, LirAuthoringLane, LirCanonicalFrame, LirCanonicalPoint3F32,
    LirCorridorElement, LirFacilityBand, LirFacilityBandGeometry, LirGateOccurrence,
    LirIdentityField, LirJunction, LirJunctionInternalEdge, LirLaneEdge, LirLaneEdgeGeometry,
    LirLaneGroup, LirManeuverGate, LirManeuverOccurrence, LirManeuverPath, LirMovement,
    LirParkingArea, LirParkingSpace, LirParticipantClass, LirRoadCorridor, LirRoadSection,
    LirRouteOccurrenceRef, LirSignalControl, LirSignalController, LirSignalGroup, LirSignalPhase,
    LirSignalPhaseState, LirSpatialSegment, LirStaticRoute, LirStopLine, LirUnit,
    LirVehicleProfile, LirWaitingZone, LirWaitingZoneOccurrence, freeze_lir,
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

    /// 按完整 Identity v1 前像规范顺序遍历全部参与者类别。
    pub fn participant_classes(
        &self,
    ) -> impl ExactSizeIterator<Item = CanonicalParticipantClassView<'_>> {
        self.inner
            .participant_classes
            .iter()
            .map(|record| CanonicalParticipantClassView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取参与者类别。
    #[must_use]
    pub fn participant_class(
        &self,
        ordinal: ParticipantClassOrdinal,
    ) -> Option<CanonicalParticipantClassView<'_>> {
        self.inner
            .participant_classes
            .get(ordinal.index())
            .map(|record| CanonicalParticipantClassView {
                lir: &self.inner,
                record,
            })
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部车辆配置。
    pub fn vehicle_profiles(
        &self,
    ) -> impl ExactSizeIterator<Item = CanonicalVehicleProfileView<'_>> {
        self.inner
            .vehicle_profiles
            .iter()
            .map(|record| CanonicalVehicleProfileView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取车辆配置。
    #[must_use]
    pub fn vehicle_profile(
        &self,
        ordinal: VehicleProfileOrdinal,
    ) -> Option<CanonicalVehicleProfileView<'_>> {
        self.inner
            .vehicle_profiles
            .get(ordinal.index())
            .map(|record| CanonicalVehicleProfileView {
                lir: &self.inner,
                record,
            })
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部规范坐标框架。
    pub fn canonical_frames(&self) -> impl ExactSizeIterator<Item = CanonicalFrameView<'_>> {
        self.inner
            .canonical_frames
            .iter()
            .map(|record| CanonicalFrameView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取规范坐标框架。
    #[must_use]
    pub fn canonical_frame(
        &self,
        ordinal: CanonicalFrameOrdinal,
    ) -> Option<CanonicalFrameView<'_>> {
        self.inner
            .canonical_frames
            .get(ordinal.index())
            .map(|record| CanonicalFrameView {
                lir: &self.inner,
                record,
            })
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部静态准入规则。
    pub fn access_rules(&self) -> impl ExactSizeIterator<Item = CanonicalAccessRuleView<'_>> {
        self.inner
            .access_rules
            .iter()
            .map(|record| CanonicalAccessRuleView {
                lir: &self.inner,
                record,
            })
    }

    /// 通过当前 LIR 实例的有类型序号读取静态准入规则。
    #[must_use]
    pub fn access_rule(&self, ordinal: AccessRuleOrdinal) -> Option<CanonicalAccessRuleView<'_>> {
        self.inner
            .access_rules
            .get(ordinal.index())
            .map(|record| CanonicalAccessRuleView {
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

    /// 返回与本边同 ordinal 对齐的规范空间几何；headless LIR 返回 `None`。
    #[must_use]
    pub fn spatial_geometry(&self) -> Option<CanonicalLaneEdgeGeometryView<'_>> {
        self.lir
            .lane_edge_geometries
            .get(self.edge.ordinal.index())
            .map(|geometry| CanonicalLaneEdgeGeometryView {
                lir: self.lir,
                lane_edge: self.edge.ordinal,
                geometry,
            })
    }
}

/// 一条 `LaneEdge` 的只读规范中心线及预计算采样表。
#[derive(Clone, Copy)]
pub struct CanonicalLaneEdgeGeometryView<'a> {
    lir: &'a LirUnit,
    lane_edge: LaneEdgeOrdinal,
    geometry: &'a LirLaneEdgeGeometry,
}

impl CanonicalLaneEdgeGeometryView<'_> {
    #[must_use]
    pub const fn lane_edge(&self) -> LaneEdgeOrdinal {
        self.lane_edge
    }

    #[must_use]
    pub const fn canonical_frame(&self) -> CanonicalFrameOrdinal {
        self.geometry.canonical_frame
    }

    #[must_use]
    pub const fn arc_length_meters(&self) -> f32 {
        self.geometry.arc_length_meters
    }

    pub fn points(&self) -> impl ExactSizeIterator<Item = CanonicalPoint3F32> + '_ {
        self.lir.canonical_points[self.geometry.points.as_usize_range()]
            .iter()
            .copied()
            .map(CanonicalPoint3F32::from)
    }

    pub fn segments(&self) -> impl ExactSizeIterator<Item = CanonicalSpatialSegment> + '_ {
        self.lir.spatial_segments[self.geometry.segments.as_usize_range()]
            .iter()
            .copied()
            .map(CanonicalSpatialSegment::from)
    }
}

/// 已量化到 canonical frame 的只读 `f32` 点，单位为米。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalPoint3F32 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl From<LirCanonicalPoint3F32> for CanonicalPoint3F32 {
    fn from(point: LirCanonicalPoint3F32) -> Self {
        Self {
            x: point.x,
            y: point.y,
            z: point.z,
        }
    }
}

/// 中心线采样使用的单段累计弧长和正交局部基。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalSpatialSegment {
    pub length_meters: f32,
    pub cumulative_end_meters: f32,
    pub tangent: [f32; 3],
    pub up: [f32; 3],
}

impl From<LirSpatialSegment> for CanonicalSpatialSegment {
    fn from(segment: LirSpatialSegment) -> Self {
        Self {
            length_meters: segment.length_meters,
            cumulative_end_meters: segment.cumulative_end_meters,
            tangent: segment.tangent,
            up: segment.up,
        }
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
    CanonicalParticipantClassView,
    LirParticipantClass,
    ParticipantClassOrdinal,
    ParticipantClassId
);
impl_stable_entity_view!(
    CanonicalVehicleProfileView,
    LirVehicleProfile,
    VehicleProfileOrdinal,
    VehicleProfileId
);
impl_stable_entity_view!(
    CanonicalFrameView,
    LirCanonicalFrame,
    CanonicalFrameOrdinal,
    CanonicalFrameId
);
impl_stable_entity_view!(
    CanonicalAccessRuleView,
    LirAccessRule,
    AccessRuleOrdinal,
    AccessRuleId
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

    /// 返回 non-traversable 设施带的规范空间几何；headless LIR 返回 `None`。
    #[must_use]
    pub fn spatial_geometry(&self) -> Option<CanonicalFacilityBandGeometryView<'_>> {
        self.lir
            .facility_band_geometries
            .binary_search_by_key(&self.record.ordinal.raw(), |geometry| {
                geometry.facility_band.raw()
            })
            .ok()
            .map(|index| CanonicalFacilityBandGeometryView {
                lir: self.lir,
                geometry: &self.lir.facility_band_geometries[index],
            })
    }
}

/// 一条 non-traversable `FacilityBand` 的只读规范中心线。
#[derive(Clone, Copy)]
pub struct CanonicalFacilityBandGeometryView<'a> {
    lir: &'a LirUnit,
    geometry: &'a LirFacilityBandGeometry,
}

impl CanonicalFacilityBandGeometryView<'_> {
    #[must_use]
    pub const fn facility_band(&self) -> FacilityBandOrdinal {
        self.geometry.facility_band
    }

    #[must_use]
    pub const fn canonical_frame(&self) -> CanonicalFrameOrdinal {
        self.geometry.canonical_frame
    }

    pub fn points(&self) -> impl ExactSizeIterator<Item = CanonicalPoint3F32> + '_ {
        self.lir.canonical_points[self.geometry.points.as_usize_range()]
            .iter()
            .copied()
            .map(CanonicalPoint3F32::from)
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

impl CanonicalParticipantClassView<'_> {
    /// 返回可选单继承父类；`None` 表示分类法根类别。
    #[must_use]
    pub const fn parent(&self) -> Option<ParticipantClassOrdinal> {
        self.record.parent
    }

    /// 返回继承深度；根类别深度为 `0`。
    #[must_use]
    pub const fn depth(&self) -> u32 {
        self.record.depth
    }

    /// 返回用于常数时间后代判断的 Euler tour 半开区间。
    #[must_use]
    pub const fn subtree_interval(&self) -> (u32, u32) {
        (self.record.subtree_enter, self.record.subtree_exit)
    }

    /// 判断另一个类别序号是否位于本类别的传递子树中（包含自身）。
    #[must_use]
    pub fn contains(&self, other: ParticipantClassOrdinal) -> bool {
        self.lir
            .participant_classes
            .get(other.index())
            .is_some_and(|candidate| {
                self.record.subtree_enter <= candidate.subtree_enter
                    && candidate.subtree_enter < self.record.subtree_exit
            })
    }
}

impl CanonicalVehicleProfileView<'_> {
    /// 返回该车辆配置唯一引用的参与者类别。
    #[must_use]
    pub const fn participant_class(&self) -> ParticipantClassOrdinal {
        self.record.participant_class
    }

    /// 返回车辆长度，单位为米。
    #[must_use]
    pub const fn length_meters(&self) -> f64 {
        self.record.length_meters
    }

    /// 返回自由流期望速度，单位为米每秒。
    #[must_use]
    pub const fn desired_speed_meters_per_second(&self) -> f64 {
        self.record.desired_speed_meters_per_second
    }

    /// 返回行为最小间距，单位为米。
    #[must_use]
    pub const fn min_gap_meters(&self) -> f64 {
        self.record.min_gap_meters
    }

    /// 返回期望时间间隔，单位为秒。
    #[must_use]
    pub const fn time_headway_seconds(&self) -> f64 {
        self.record.time_headway_seconds
    }

    /// 返回最大舒适加速度，单位为米每二次方秒。
    #[must_use]
    pub const fn max_acceleration_meters_per_second_squared(&self) -> f64 {
        self.record.max_acceleration_meters_per_second_squared
    }

    /// 返回舒适减速度幅值，单位为米每二次方秒。
    #[must_use]
    pub const fn comfortable_deceleration_meters_per_second_squared(&self) -> f64 {
        self.record
            .comfortable_deceleration_meters_per_second_squared
    }

    /// 返回紧急减速度幅值，单位为米每二次方秒。
    #[must_use]
    pub const fn emergency_deceleration_meters_per_second_squared(&self) -> f64 {
        self.record.emergency_deceleration_meters_per_second_squared
    }
}

/// Canonical LIR 中一条准入规则的有类型静态目标。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CanonicalAccessTarget {
    /// 单条车道图边。
    LaneEdge(LaneEdgeOrdinal),
    /// 车道组；运行时投影可按预编译覆盖关系展开到边。
    LaneGroup(LaneGroupOrdinal),
    /// 道路区段；运行时投影可按预编译覆盖关系展开到边。
    RoadSection(RoadSectionOrdinal),
    /// 保持独立准入平面的机动路径。
    ManeuverPath(ManeuverPathOrdinal),
}

/// 一条准入规则所携带法规来源的借用视图。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CanonicalAccessRegulationView<'a> {
    jurisdiction: &'a str,
    version: &'a str,
    source: Option<&'a str>,
}

impl<'a> CanonicalAccessRegulationView<'a> {
    /// 返回法域。
    #[must_use]
    pub const fn jurisdiction(self) -> &'a str {
        self.jurisdiction
    }

    /// 返回法规版本。
    #[must_use]
    pub const fn version(self) -> &'a str {
        self.version
    }

    /// 返回可选来源说明。
    #[must_use]
    pub const fn source(self) -> Option<&'a str> {
        self.source
    }
}

impl CanonicalAccessRuleView<'_> {
    /// 返回规则目标；边平面与机动路径平面分别组合，不能跨平面相互覆盖。
    #[must_use]
    pub const fn target(&self) -> CanonicalAccessTarget {
        match self.record.target {
            LirAccessTarget::LaneEdge(target) => CanonicalAccessTarget::LaneEdge(target),
            LirAccessTarget::LaneGroup(target) => CanonicalAccessTarget::LaneGroup(target),
            LirAccessTarget::RoadSection(target) => CanonicalAccessTarget::RoadSection(target),
            LirAccessTarget::ManeuverPath(target) => CanonicalAccessTarget::ManeuverPath(target),
        }
    }

    /// 返回规则在当前准入平面内的允许或拒绝效果。
    #[must_use]
    pub const fn effect(&self) -> AccessEffect {
        self.record.effect
    }

    /// 返回按规范类别序号排序、去重后的非空类别集合。
    #[must_use]
    pub fn participant_classes(&self) -> &[ParticipantClassOrdinal] {
        &self.lir.access_rule_participant_classes[self.record.participant_classes.as_usize_range()]
    }

    /// 返回可选法规来源；该信息不参与规则优先级计算。
    #[must_use]
    pub fn regulation(&self) -> Option<CanonicalAccessRegulationView<'_>> {
        self.record
            .regulation
            .as_ref()
            .map(|regulation| CanonicalAccessRegulationView {
                jurisdiction: &regulation.jurisdiction,
                version: &regulation.version,
                source: regulation.source.as_deref(),
            })
    }

    /// 返回类别与目标具体度相同后的显式优先级。
    #[must_use]
    pub const fn priority(&self) -> i32 {
        self.record.priority
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
    metrics: CompilationMetrics,
}

/// 一次成功生产编译的只读资源与确定性观测值。
///
/// 这些值来自编译器实际完成的 HIR→MIR→Canonical LIR 管线，不包含前端构造、当前态
/// 投影或证据序列化。字节数是编译器内部资源模型使用的逻辑值，不等同于操作系统进程
/// 工作集，也不是静态镜像或后继可移植制品的文件大小。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompilationMetrics {
    lir_record_count: u64,
    output_logical_bytes: u64,
    compiler_controlled_peak_bytes: u64,
    semantic_fingerprint: [u8; 32],
}

impl CompilationMetrics {
    /// 返回 Canonical LIR 的实体、关系与出现项逻辑记录总数。
    #[must_use]
    pub const fn lir_record_count(self) -> u64 {
        self.lir_record_count
    }

    /// 返回目标布局中立的 Canonical LIR 逻辑输出字节数。
    #[must_use]
    pub const fn output_logical_bytes(self) -> u64 {
        self.output_logical_bytes
    }

    /// 返回本次编译资源模型计算的编译器控制峰值字节数。
    ///
    /// 该值覆盖同一阶段同时存续的来源、IR、暂存区和输出容量，但不包含标准库、系统
    /// 分配器元数据或进程内其他组件的内存。
    #[must_use]
    pub const fn compiler_controlled_peak_bytes(self) -> u64 {
        self.compiler_controlled_peak_bytes
    }

    /// 返回当前编译器版本对完整 Canonical LIR 语义计算的确定性指纹。
    ///
    /// 该指纹用于同版本重复编译和性能证据核对；它不是制品完整性摘要、路网修订 ID
    /// 或跨格式版本兼容承诺，调用方不得用它替代后继版本化制品描述符。
    #[must_use]
    pub const fn semantic_fingerprint(self) -> [u8; 32] {
        self.semantic_fingerprint
    }
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

    /// 返回本次成功编译的资源与确定性观测值。
    ///
    /// 调用者可以在停表后读取该值并形成基线；读取不会遍历或复制 LIR。
    #[must_use]
    pub const fn metrics(&self) -> CompilationMetrics {
        self.metrics
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

    /// 返回当前实例跨编译保留的容量字节数。
    ///
    /// 首版干净单工作线程编译器不保留 arena、缓存或其他堆容量，因此恒为零。后继若
    /// 引入容量复用，必须让该值覆盖所有由 `Compiler` 拥有、会影响宿主长期内存预算的
    /// 保留容量。
    #[must_use]
    pub const fn retained_capacity_bytes(&self) -> u64 {
        0
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
        let hir_peak_controlled_live_bytes = hir.peak_controlled_live_bytes;
        let mir = lower_to_mir(&unit, &hir)?;
        let mir_peak_controlled_live_bytes = mir.peak_controlled_live_bytes;
        // MIR 已拥有后继阶段所需的完整语义与来源位置；尽早释放 HIR，避免把阶段共存
        // 时间延长到 LIR/source-map 冻结并破坏资源峰值模型。
        drop(hir);
        let frozen_lir = freeze_lir(&unit, &mir)?;
        let lir_record_count = frozen_lir.lir.lir_record_count;
        let output_logical_bytes = frozen_lir.lir.output_bytes;
        let semantic_fingerprint = frozen_lir.lir.semantic_digest;
        let lir_peak_controlled_live_bytes = frozen_lir.lir.peak_controlled_live_bytes;
        let source_map_input = freeze_source_map(unit, &mir, &frozen_lir)?;
        let metrics = CompilationMetrics {
            lir_record_count,
            output_logical_bytes,
            compiler_controlled_peak_bytes: hir_peak_controlled_live_bytes
                .max(mir_peak_controlled_live_bytes)
                .max(lir_peak_controlled_live_bytes)
                .max(source_map_input.peak_controlled_live_bytes()),
            semantic_fingerprint,
        };
        drop(mir);
        let crate::lir::LirFreezeOutput { lir, .. } = frozen_lir;
        Ok(CompilationOutput {
            lir: ValidatedCanonicalLir { inner: lir },
            source_map_input,
            diagnostics: Box::default(),
            metrics,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declaration::{
        CompiledFacilityBandGeometry, EdgeLength, OwnedEntityReference, TypedAstDeclaration,
    };
    use crate::{
        AccessCapability, AccessRegulationInput, AccessRuleInput, AccessRuleTargetInput,
        AuthoringLaneInput, CanonicalFrameInput, CanonicalPoint3F32Input, CompilationUnitBuilder,
        CompileLimitDimension, CompileLimits, CorridorElementReference, DiagnosticCode,
        DiagnosticPayload, FacilityBandInput, FacilityBandReference, IidmVehicleProfileInput,
        JunctionInput, JunctionReference, LaneEdgeGeometryInput, LaneEdgeInput, LaneEdgeReference,
        LaneGroupInput, LaneGroupReference, ManeuverGateInput, ManeuverGateReference,
        ManeuverPathInput, ManeuverPathReference, MovementInput, MovementReference,
        ParkingAreaInput, ParkingAreaReference, ParkingLaneAnchorInput, ParkingSpaceGeometryInput,
        ParkingSpaceInput, ParticipantClassInput, ParticipantClassReference, RoadCorridorInput,
        RoadSectionInput, RoadSectionReference, SignalControlInput, SignalControllerInput,
        SignalGroupInput, SignalGroupReference, SignalGroupStateInput, SignalPhaseInput,
        SourceModuleDescriptor, SourceModuleHeader, SourceModuleHeaderInput, SourceRelationRole,
        SourceSpan, StaticRouteInput, StopLineInput, StopLineReference, SyntheticModule,
        SyntheticModuleBuilder, VehicleProfileInput, WaitingZoneInput,
    };
    use laneflow_static_contract::CanonicalFrameKind;
    use std::sync::Arc;

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

    fn spatial_cross_section_unit(
        permuted: bool,
        facility_a_z: f32,
        include_facility_geometry: bool,
    ) -> CompilationUnit {
        spatial_cross_section_unit_with_frame(
            permuted,
            facility_a_z,
            include_facility_geometry,
            false,
        )
    }

    fn spatial_cross_section_unit_with_frame(
        permuted: bool,
        facility_a_z: f32,
        include_facility_geometry: bool,
        imported_facility_frame: bool,
    ) -> CompilationUnit {
        let limits = CompileLimits::p100_initial_v1();
        let header = SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: "city/spatial-cross-section",
                source_document_key: "spatial-cross-section.document",
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
        if imported_facility_frame {
            builder.add_import("city/base").unwrap();
        }
        let lane_points = [
            CanonicalPoint3F32Input {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            CanonicalPoint3F32Input {
                x: 10.0,
                y: 0.0,
                z: 0.0,
            },
        ];
        let lane_geometry = [LaneEdgeGeometryInput {
            lane_edge: LaneEdgeReference::local("edge-main"),
            centerline_points: &lane_points,
        }];
        let corridor_elements = [
            CorridorElementReference::facility_band(FacilityBandReference::local("band-z")),
            CorridorElementReference::road_section(RoadSectionReference::local("carriageway")),
            CorridorElementReference::facility_band(FacilityBandReference::local("band-a")),
        ];
        let add_edge = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: "edge-main",
                    length_meters: 10.0,
                    speed_limit_meters_per_second: 12.0,
                    successors: &[],
                })
                .unwrap();
        };
        let add_bands = |builder: &mut SyntheticModuleBuilder, reverse: bool| {
            let keys = if reverse {
                ["band-z", "band-a"]
            } else {
                ["band-a", "band-z"]
            };
            for key in keys {
                builder
                    .add_facility_band(FacilityBandInput {
                        facility_band_key: key,
                        kind_id: "sidewalk",
                    })
                    .unwrap();
            }
        };
        let add_section = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_road_section(RoadSectionInput {
                    road_section_key: "carriageway",
                    kind_id: "motorLane",
                    lanes: &[AuthoringLaneInput {
                        authoring_lane_key: "lane-main",
                        edge_chain: &[LaneEdgeReference::local("edge-main")],
                        lane_group: None,
                    }],
                })
                .unwrap();
        };
        let add_corridor = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_road_corridor(RoadCorridorInput {
                    road_corridor_key: "main-road",
                    reference_section: RoadSectionReference::local("carriageway"),
                    elements: &corridor_elements,
                })
                .unwrap();
        };
        let add_frame = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_canonical_frame(CanonicalFrameInput {
                    canonical_frame_key: "frame-main",
                    lane_edge_geometries: &lane_geometry,
                })
                .unwrap();
        };

        if permuted {
            add_corridor(&mut builder);
            add_frame(&mut builder);
            add_bands(&mut builder, true);
            add_section(&mut builder);
            add_edge(&mut builder);
        } else {
            add_edge(&mut builder);
            add_bands(&mut builder, false);
            add_section(&mut builder);
            add_corridor(&mut builder);
            add_frame(&mut builder);
        }

        let source_module = builder.finish().unwrap();
        let mut unit = if imported_facility_frame {
            let base_header = SourceModuleHeader::new(
                SourceModuleHeaderInput {
                    authoring_namespace_id: "city/base",
                    source_document_key: "base.document",
                    generator_build_id: "git:0123456789abcdef",
                    parameters_and_inputs_digest: [0x11; 32],
                    frontend_options_digest: [0x22; 32],
                    random_seed: Some(42),
                    provenance: "repository:laneflow",
                },
                &limits,
            )
            .unwrap();
            let mut base = SyntheticModuleBuilder::new(base_header, &limits).unwrap();
            base.add_canonical_frame(CanonicalFrameInput {
                canonical_frame_key: "world",
                lane_edge_geometries: &[],
            })
            .unwrap();
            unit([source_module, base.finish().unwrap()])
        } else {
            unit([source_module])
        };
        if include_facility_geometry {
            let module = unit
                .modules
                .iter_mut()
                .find(|module| {
                    module.descriptor().authoring_namespace_id() == "city/spatial-cross-section"
                })
                .expect("fixture contains its cross-section module");
            let namespace: Arc<str> = module.descriptor().authoring_namespace_id().into();
            for declaration in &mut module.declarations {
                let TypedAstDeclaration::FacilityBand(band) = declaration else {
                    continue;
                };
                let z = if band.header.stable_key.as_ref() == "band-a" {
                    facility_a_z
                } else {
                    4.0
                };
                band.compiled_geometry = Some(CompiledFacilityBandGeometry {
                    length: EdgeLength::try_new(10.0).unwrap(),
                    canonical_frame: OwnedEntityReference::<CanonicalFrameKind>::new(
                        if imported_facility_frame {
                            Arc::from("city/base")
                        } else {
                            Arc::clone(&namespace)
                        },
                        if imported_facility_frame {
                            Arc::from("world")
                        } else {
                            Arc::from("frame-main")
                        },
                        band.header.span.clone(),
                    ),
                    centerline_points: [
                        CanonicalPoint3F32Input { x: 0.0, y: 0.0, z },
                        CanonicalPoint3F32Input { x: 10.0, y: 0.0, z },
                    ]
                    .into(),
                    source_ranges: Box::new([]),
                });
            }
        }
        unit
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

    fn branched_control_builder(
        document: &str,
        include_right_path: bool,
    ) -> SyntheticModuleBuilder {
        let mut builder = junction_builder(document);
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "entry",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[
                    LaneEdgeReference::local("middle-left"),
                    LaneEdgeReference::local("middle-right"),
                ],
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "middle-left",
                length_meters: 8.0,
                speed_limit_meters_per_second: 8.0,
                successors: &[LaneEdgeReference::local("exit-left")],
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "middle-right",
                length_meters: 8.0,
                speed_limit_meters_per_second: 8.0,
                successors: &[LaneEdgeReference::local("exit-right")],
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "exit-left",
                length_meters: 12.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "exit-right",
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
                maneuver_path_key: "path-left",
                movement: MovementReference::local("movement-through"),
                entry_edge: LaneEdgeReference::local("entry"),
                internal_edges: &[LaneEdgeReference::local("middle-left")],
                exit_edge: LaneEdgeReference::local("exit-left"),
            })
            .unwrap();
        if include_right_path {
            builder
                .add_maneuver_path(ManeuverPathInput {
                    maneuver_path_key: "path-right",
                    movement: MovementReference::local("movement-through"),
                    entry_edge: LaneEdgeReference::local("entry"),
                    internal_edges: &[LaneEdgeReference::local("middle-right")],
                    exit_edge: LaneEdgeReference::local("exit-right"),
                })
                .unwrap();
        }
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

    fn access_builder(document: &str) -> SyntheticModuleBuilder {
        let limits = CompileLimits::p100_initial_v1();
        let header = SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: "city/access",
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
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge-main",
                length_meters: 20.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap();
        builder
    }

    fn canonical_iidm_profile() -> IidmVehicleProfileInput {
        IidmVehicleProfileInput {
            length_meters: 4.5,
            desired_speed_meters_per_second: 13.75,
            min_gap_meters: 2.0,
            time_headway_seconds: 1.4,
            max_acceleration_meters_per_second_squared: 1.8,
            comfortable_deceleration_meters_per_second_squared: 2.0,
            emergency_deceleration_meters_per_second_squared: 4.5,
        }
    }

    fn access_semantics_module(permuted: bool) -> SyntheticModule {
        let mut builder = access_builder("access-semantic.document");
        let add_root = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_participant_class(ParticipantClassInput {
                    participant_class_key: "road-user",
                    extends: None,
                })
                .unwrap();
        };
        let add_child = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_participant_class(ParticipantClassInput {
                    participant_class_key: "car",
                    extends: Some(ParticipantClassReference::local("road-user")),
                })
                .unwrap();
        };
        let add_allow = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_access_rule(AccessRuleInput {
                    access_rule_key: "allow-road-users",
                    target: AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local("edge-main")),
                    effect: AccessEffect::Allow,
                    participant_classes: &[ParticipantClassReference::local("road-user")],
                    regulation: Some(AccessRegulationInput {
                        jurisdiction: "CN-test",
                        version: "2026-01",
                        source: Some("fixture"),
                    }),
                    priority: 0,
                })
                .unwrap();
        };
        let add_deny = |builder: &mut SyntheticModuleBuilder| {
            builder
                .add_access_rule(AccessRuleInput {
                    access_rule_key: "deny-cars",
                    target: AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local("edge-main")),
                    effect: AccessEffect::Deny,
                    participant_classes: &[ParticipantClassReference::local("car")],
                    regulation: None,
                    priority: 0,
                })
                .unwrap();
        };
        if permuted {
            add_deny(&mut builder);
            add_allow(&mut builder);
            add_child(&mut builder);
            add_root(&mut builder);
        } else {
            add_root(&mut builder);
            add_child(&mut builder);
            add_allow(&mut builder);
            add_deny(&mut builder);
        }
        builder.finish().unwrap()
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
        let mut compiler = Compiler::new();
        let output = compiler.compile(input).unwrap();

        assert!(output.diagnostics().is_empty());
        let metrics = output.metrics();
        assert_eq!(
            metrics.lir_record_count(),
            output.lir.inner.lir_record_count
        );
        assert_eq!(
            metrics.output_logical_bytes(),
            output.lir.inner.output_bytes
        );
        assert!(metrics.compiler_controlled_peak_bytes() >= output.lir.inner.controlled_live_bytes);
        assert_eq!(
            metrics.semantic_fingerprint(),
            output.lir.inner.semantic_digest
        );
        assert_eq!(compiler.retained_capacity_bytes(), 0);
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

        let mut duplicate_stop_line = control_builder("stop-duplicate-edge.document");
        for key in ["stop-entry-a", "stop-entry-b"] {
            duplicate_stop_line
                .add_stop_line(StopLineInput {
                    stop_line_key: key,
                    lane_edge: LaneEdgeReference::local("entry"),
                })
                .unwrap();
        }
        assert!(
            compile_diagnostic_codes(duplicate_stop_line)
                .contains(&DiagnosticCode::DuplicateStopLineEdge)
        );

        let mut missing_gate = branched_control_builder("stop-missing-gate.document", true);
        missing_gate
            .add_stop_line(StopLineInput {
                stop_line_key: "stop-entry",
                lane_edge: LaneEdgeReference::local("entry"),
            })
            .unwrap()
            .add_maneuver_gate(ManeuverGateInput {
                maneuver_gate_key: "gate-left",
                maneuver_path: ManeuverPathReference::local("path-left"),
                transition_index: 0,
                stop_line: StopLineReference::local("stop-entry"),
                signal_control: SignalControlInput::None,
            })
            .unwrap();
        assert!(
            compile_diagnostic_codes(missing_gate)
                .contains(&DiagnosticCode::MissingManeuverGateCoverage)
        );

        let mut missing_path = branched_control_builder("stop-missing-path.document", false);
        missing_path
            .add_stop_line(StopLineInput {
                stop_line_key: "stop-entry",
                lane_edge: LaneEdgeReference::local("entry"),
            })
            .unwrap()
            .add_maneuver_gate(ManeuverGateInput {
                maneuver_gate_key: "gate-left",
                maneuver_path: ManeuverPathReference::local("path-left"),
                transition_index: 0,
                stop_line: StopLineReference::local("stop-entry"),
                signal_control: SignalControlInput::None,
            })
            .unwrap();
        assert!(
            compile_diagnostic_codes(missing_path)
                .contains(&DiagnosticCode::MissingManeuverPathCoverage)
        );
    }

    #[test]
    fn synthetic_maneuver_path_requires_successors_for_internal_sequence() {
        let mut builder = junction_builder("internal-route.document");
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "entry",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "internal",
                length_meters: 8.0,
                speed_limit_meters_per_second: 8.0,
                successors: &[],
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
                internal_edges: &[LaneEdgeReference::local("internal")],
                exit_edge: LaneEdgeReference::local("exit"),
            })
            .unwrap()
            .add_static_route(StaticRouteInput {
                static_route_key: "route-main",
                edge_sequence: &[
                    LaneEdgeReference::local("entry"),
                    LaneEdgeReference::local("internal"),
                    LaneEdgeReference::local("exit"),
                ],
            })
            .unwrap();

        let diagnostics = match Compiler::new().compile(unit([builder.finish().unwrap()])) {
            Ok(_) => panic!("Synthetic maneuver paths require explicit successor connectivity"),
            Err(diagnostics) => diagnostics,
        };
        assert!(
            diagnostics.diagnostics().iter().any(|diagnostic| {
                diagnostic.code() == DiagnosticCode::DisconnectedManeuverPath
            })
        );
    }

    #[test]
    fn path_owned_internal_transition_accepts_release_stop_without_explicit_successor() {
        let mut builder = junction_builder("internal-release-stop.document");
        let entry_chain = [LaneEdgeReference::local("entry")];
        let exit_chain = [LaneEdgeReference::local("exit")];
        let approach_lanes = [
            AuthoringLaneInput {
                authoring_lane_key: "lane-entry",
                edge_chain: &entry_chain,
                lane_group: None,
            },
            AuthoringLaneInput {
                authoring_lane_key: "lane-exit",
                edge_chain: &exit_chain,
                lane_group: None,
            },
        ];
        builder
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "entry",
                length_meters: 10.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[LaneEdgeReference::local("exit")],
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "middle",
                length_meters: 8.0,
                speed_limit_meters_per_second: 8.0,
                successors: &[],
            })
            .unwrap()
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "exit",
                length_meters: 12.0,
                speed_limit_meters_per_second: 10.0,
                successors: &[],
            })
            .unwrap()
            .add_road_section(RoadSectionInput {
                road_section_key: "section-main",
                kind_id: "motorLane",
                lanes: &approach_lanes,
            })
            .unwrap()
            .add_road_corridor(RoadCorridorInput {
                road_corridor_key: "corridor-main",
                reference_section: RoadSectionReference::local("section-main"),
                elements: &[CorridorElementReference::road_section(
                    RoadSectionReference::local("section-main"),
                )],
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
            .unwrap()
            .add_stop_line(StopLineInput {
                stop_line_key: "stop-middle",
                lane_edge: LaneEdgeReference::local("middle"),
            })
            .unwrap()
            .add_maneuver_gate(ManeuverGateInput {
                maneuver_gate_key: "gate-release",
                maneuver_path: ManeuverPathReference::local("path-main"),
                transition_index: 1,
                stop_line: StopLineReference::local("stop-middle"),
                signal_control: SignalControlInput::None,
            })
            .unwrap()
            .add_static_route(StaticRouteInput {
                static_route_key: "route-main",
                edge_sequence: &[
                    LaneEdgeReference::local("entry"),
                    LaneEdgeReference::local("middle"),
                    LaneEdgeReference::local("exit"),
                ],
            })
            .unwrap();

        let mut input = unit([builder.finish().unwrap()]);
        let junction = input.modules[0]
            .declarations
            .iter_mut()
            .find_map(|declaration| match declaration {
                TypedAstDeclaration::Junction(junction) => Some(junction),
                _ => None,
            })
            .unwrap();
        let namespace = Arc::<str>::from("city/junction");
        let document = Arc::<str>::from("internal-release-stop.document");
        let location = |column| SourceSpan::point(Arc::clone(&document), 1, column);
        junction.approach_edges = Box::new([
            OwnedEntityReference::new(Arc::clone(&namespace), Arc::from("entry"), location(1)),
            OwnedEntityReference::new(Arc::clone(&namespace), Arc::from("exit"), location(2)),
        ]);
        junction.internal_edges = Box::new([OwnedEntityReference::new(
            Arc::clone(&namespace),
            Arc::from("middle"),
            location(3),
        )]);

        let output = Compiler::new().compile(input).unwrap();
        assert_eq!(output.lir().maneuver_gates().count(), 1);
        assert_eq!(output.lir().static_routes().count(), 1);
        let route = output.lir().static_routes().next().unwrap();
        let maneuvers = route.maneuver_occurrences().collect::<Vec<_>>();
        let gates = route.gate_occurrences().collect::<Vec<_>>();
        assert_eq!(maneuvers.len(), 1);
        assert_eq!(maneuvers[0].entry_route_edge_index(), 0);
        assert_eq!(maneuvers[0].exit_route_edge_index(), 2);
        assert_eq!(maneuvers[0].gate_occurrence_range(), 0..1);
        assert_eq!(gates.len(), 1);
        assert_eq!(gates[0].maneuver_occurrence_index(), 0);
        assert_eq!(gates[0].from_route_edge_index(), 1);
        assert_eq!(gates[0].next_boundary_route_edge_index(), 2);
        assert_eq!(
            route.transition_gates().collect::<Vec<_>>(),
            [None, Some(gates[0].maneuver_gate())]
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

    #[test]
    fn compiler_freezes_vehicle_profile_values_identity_and_class_source() {
        let mut builder = access_builder("vehicle-profile.document");
        builder
            .add_participant_class(ParticipantClassInput {
                participant_class_key: "passenger-car",
                extends: None,
            })
            .unwrap()
            .add_vehicle_profile(VehicleProfileInput {
                vehicle_profile_key: "standard-car",
                participant_class: ParticipantClassReference::local("passenger-car"),
                iidm: canonical_iidm_profile(),
            })
            .unwrap();

        let output = Compiler::new()
            .compile(unit([builder.finish().unwrap()]))
            .unwrap();
        let profile = output.lir().vehicle_profiles().next().unwrap();
        assert_eq!(
            stable_key(profile.identity_fields(), FieldTag::VehicleProfileKey),
            "standard-car"
        );
        assert_eq!(profile.length_meters(), 4.5);
        assert_eq!(profile.desired_speed_meters_per_second(), 13.75);
        assert_eq!(profile.min_gap_meters(), 2.0);
        assert_eq!(profile.time_headway_seconds(), 1.4);
        assert_eq!(profile.max_acceleration_meters_per_second_squared(), 1.8);
        assert_eq!(
            profile.comfortable_deceleration_meters_per_second_squared(),
            2.0
        );
        assert_eq!(
            profile.emergency_deceleration_meters_per_second_squared(),
            4.5
        );
        assert_eq!(
            output
                .lir()
                .participant_class(profile.participant_class())
                .unwrap()
                .ordinal(),
            profile.participant_class()
        );
        assert_eq!(output.source_map_input().vehicle_profile_sources().len(), 1);
        let relation = output
            .source_map_input()
            .access_relation_sources()
            .find(|source| source.role() == SourceRelationRole::VehicleProfileParticipantClass)
            .unwrap();
        assert!(matches!(
            relation.owner(),
            crate::AccessRelationOwner::VehicleProfile(ordinal, stable_id)
                if ordinal == profile.ordinal() && stable_id == profile.stable_id()
        ));
    }

    #[test]
    fn compiler_freezes_canonical_frames_in_identity_order_with_sources() {
        let mut builder = access_builder("canonical-frame.document");
        builder
            .add_canonical_frame(CanonicalFrameInput {
                canonical_frame_key: "frame-z",
                lane_edge_geometries: &[],
            })
            .unwrap()
            .add_canonical_frame(CanonicalFrameInput {
                canonical_frame_key: "frame-a",
                lane_edge_geometries: &[],
            })
            .unwrap();

        let output = Compiler::new()
            .compile(unit([builder.finish().unwrap()]))
            .unwrap();
        let frames = output.lir().canonical_frames().collect::<Vec<_>>();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].ordinal().raw(), 0);
        assert_eq!(frames[1].ordinal().raw(), 1);
        assert_eq!(
            stable_key(frames[0].identity_fields(), FieldTag::CanonicalFrameKey),
            "frame-a"
        );
        assert_eq!(
            stable_key(frames[1].identity_fields(), FieldTag::CanonicalFrameKey),
            "frame-z"
        );
        let sources = output
            .source_map_input()
            .canonical_frame_sources()
            .collect::<Vec<_>>();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].ordinal(), frames[0].ordinal());
        assert_eq!(sources[0].stable_id(), frames[0].stable_id());
        assert_eq!(sources[1].ordinal(), frames[1].ordinal());
        assert_eq!(sources[1].stable_id(), frames[1].stable_id());
    }

    #[test]
    fn canonical_frame_identity_changes_lir_semantic_digest() {
        let compile = |key| {
            let mut builder = access_builder("canonical-frame-digest.document");
            builder
                .add_canonical_frame(CanonicalFrameInput {
                    canonical_frame_key: key,
                    lane_edge_geometries: &[],
                })
                .unwrap();
            Compiler::new()
                .compile(unit([builder.finish().unwrap()]))
                .unwrap()
        };

        let left = compile("frame-a");
        let right = compile("frame-b");
        assert_ne!(
            left.lir.inner.semantic_digest,
            right.lir.inner.semantic_digest
        );
    }

    #[test]
    fn compiler_validates_and_freezes_lane_edge_spatial_sampling_tables() {
        let points = [
            CanonicalPoint3F32Input {
                x: -0.0,
                y: 0.0,
                z: 0.0,
            },
            CanonicalPoint3F32Input {
                x: 8.0,
                y: 0.0,
                z: 0.0,
            },
            CanonicalPoint3F32Input {
                x: 20.0,
                y: 0.0,
                z: 0.0,
            },
        ];
        let geometries = [LaneEdgeGeometryInput {
            lane_edge: LaneEdgeReference::local("edge-main"),
            centerline_points: &points,
        }];
        let mut builder = access_builder("canonical-spatial.document");
        builder
            .add_canonical_frame(CanonicalFrameInput {
                canonical_frame_key: "frame-main",
                lane_edge_geometries: &geometries,
            })
            .unwrap();

        let output = Compiler::new()
            .compile(unit([builder.finish().unwrap()]))
            .unwrap();
        let edge = output.lir().lane_edges().next().unwrap();
        let geometry = edge.spatial_geometry().unwrap();
        assert_eq!(geometry.lane_edge(), edge.ordinal());
        assert_eq!(geometry.canonical_frame().raw(), 0);
        assert_eq!(geometry.arc_length_meters(), 20.0);
        let frozen_points = geometry.points().collect::<Vec<_>>();
        assert_eq!(frozen_points.len(), 3);
        assert_eq!(frozen_points[0].x.to_bits(), 0.0_f32.to_bits());
        let segments = geometry.segments().collect::<Vec<_>>();
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].length_meters, 8.0);
        assert_eq!(segments[1].cumulative_end_meters, 20.0);
        assert_eq!(segments[0].tangent, [1.0, 0.0, 0.0]);
        assert_eq!(segments[0].up, [0.0, 1.0, 0.0]);

        let relation = output
            .source_map_input()
            .spatial_relation_sources()
            .next()
            .unwrap();
        assert_eq!(relation.owner_ordinal(), geometry.canonical_frame());
        assert_eq!(
            relation.role(),
            SourceRelationRole::CanonicalFrameLaneEdgeGeometry
        );
        assert_eq!(relation.local_index(), 0);
    }

    #[test]
    fn compiler_freezes_non_traversable_facility_geometry_in_canonical_band_order() {
        let mut baseline_unit = spatial_cross_section_unit(false, 2.0, true);
        let baseline_module = &mut baseline_unit.modules[0];
        let mut band_a_index = None;
        let mut band_z_index = None;
        for (index, declaration) in baseline_module.declarations.iter_mut().enumerate() {
            let TypedAstDeclaration::FacilityBand(band) = declaration else {
                continue;
            };
            let (line, target) = if band.header.stable_key.as_ref() == "band-a" {
                (10, &mut band_a_index)
            } else {
                (20, &mut band_z_index)
            };
            band.header.span =
                SourceSpan::point(Arc::from("spatial-cross-section.document"), line, 1).into();
            *target = Some(index);
        }
        let band_a_index = band_a_index.expect("fixture contains band-a");
        let band_z_index = band_z_index.expect("fixture contains band-z");
        if band_a_index < band_z_index {
            baseline_module
                .declarations
                .swap(band_a_index, band_z_index);
        }
        let baseline = Compiler::new().compile(baseline_unit).unwrap();
        let permuted = Compiler::new()
            .compile(spatial_cross_section_unit(true, 2.0, true))
            .unwrap();
        let changed = Compiler::new()
            .compile(spatial_cross_section_unit(false, 3.0, true))
            .unwrap();
        let headless_facilities = Compiler::new()
            .compile(spatial_cross_section_unit(false, 2.0, false))
            .unwrap();
        let mut sparse_unit = spatial_cross_section_unit(false, 2.0, true);
        let sparse_module = &mut sparse_unit.modules[0];
        let sparse_band_a = sparse_module
            .declarations
            .iter_mut()
            .find_map(|declaration| match declaration {
                TypedAstDeclaration::FacilityBand(band)
                    if band.header.stable_key.as_ref() == "band-a" =>
                {
                    Some(band)
                }
                _ => None,
            })
            .expect("fixture contains band-a");
        sparse_band_a.compiled_geometry = None;
        let sparse = Compiler::new().compile(sparse_unit).unwrap();

        let bands = baseline.lir().facility_bands().collect::<Vec<_>>();
        assert_eq!(bands.len(), 2);
        let keys = bands
            .iter()
            .map(|band| {
                band.identity_fields()
                    .find(|field| field.tag() == FieldTag::FacilityBandKey)
                    .map(|field| String::from_utf8(field.value_bytes().to_vec()).unwrap())
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(keys, ["band-a", "band-z"]);

        let geometry = bands[0].spatial_geometry().unwrap();
        assert_eq!(geometry.facility_band(), bands[0].ordinal());
        assert_eq!(geometry.canonical_frame().raw(), 0);
        assert_eq!(
            geometry.points().collect::<Vec<_>>(),
            [
                CanonicalPoint3F32 {
                    x: 0.0,
                    y: 0.0,
                    z: 2.0,
                },
                CanonicalPoint3F32 {
                    x: 10.0,
                    y: 0.0,
                    z: 2.0,
                },
            ]
        );
        assert_eq!(baseline.lir.inner.lane_edge_geometries.len(), 1);
        assert_eq!(baseline.lir.inner.facility_band_geometries.len(), 2);
        assert_eq!(
            baseline
                .lir
                .inner
                .facility_band_geometries
                .iter()
                .map(|geometry| geometry.facility_band.raw())
                .collect::<Vec<_>>(),
            [0, 1]
        );
        let sparse_bands = sparse.lir().facility_bands().collect::<Vec<_>>();
        assert!(sparse_bands[0].spatial_geometry().is_none());
        assert_eq!(
            sparse_bands[1]
                .spatial_geometry()
                .expect("ordinal one keeps its sparse geometry")
                .facility_band(),
            sparse_bands[1].ordinal()
        );
        assert_eq!(
            sparse
                .lir
                .inner
                .facility_band_geometries
                .iter()
                .map(|geometry| geometry.facility_band.raw())
                .collect::<Vec<_>>(),
            [1]
        );
        assert_eq!(baseline.lir.inner.canonical_points.len(), 6);
        assert_eq!(baseline.lir.inner.spatial_segments.len(), 1);
        assert_eq!(
            baseline.lir.inner.spatial_segments.len(),
            headless_facilities.lir.inner.spatial_segments.len()
        );
        let baseline_edge = baseline
            .lir()
            .lane_edges()
            .next()
            .expect("fixture retains its lane edge");
        let baseline_lane = baseline_edge
            .spatial_geometry()
            .expect("fixture retains lane geometry");
        let changed_edge = changed
            .lir()
            .lane_edges()
            .next()
            .expect("changed fixture retains its lane edge");
        let changed_lane = changed_edge
            .spatial_geometry()
            .expect("changed fixture retains lane geometry");
        assert_eq!(
            baseline_lane.points().collect::<Vec<_>>(),
            changed_lane.points().collect::<Vec<_>>()
        );
        assert_eq!(
            baseline_lane.segments().collect::<Vec<_>>(),
            changed_lane.segments().collect::<Vec<_>>()
        );

        let spatial_relations = baseline
            .source_map_input()
            .spatial_relation_sources()
            .filter(|relation| {
                relation.role() == SourceRelationRole::CanonicalFrameFacilityBandGeometry
            })
            .map(|relation| {
                (
                    relation.role(),
                    relation.local_index(),
                    relation
                        .primary_source()
                        .text_range()
                        .map(|(start, _)| start.line()),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            spatial_relations,
            [
                (
                    SourceRelationRole::CanonicalFrameFacilityBandGeometry,
                    0,
                    Some(10),
                ),
                (
                    SourceRelationRole::CanonicalFrameFacilityBandGeometry,
                    1,
                    Some(20),
                ),
            ]
        );

        assert_eq!(
            baseline.metrics().semantic_fingerprint(),
            permuted.metrics().semantic_fingerprint()
        );
        assert_ne!(
            baseline.metrics().semantic_fingerprint(),
            changed.metrics().semantic_fingerprint()
        );
        assert_eq!(
            baseline.metrics().lir_record_count(),
            headless_facilities
                .metrics()
                .lir_record_count()
                .saturating_add(6)
        );
        // 两条 point-only facility geometry 行和四个点的目标布局逻辑量。
        assert_eq!(
            baseline.metrics().output_logical_bytes(),
            headless_facilities
                .metrics()
                .output_logical_bytes()
                .saturating_add(80)
        );
        assert!(
            headless_facilities
                .lir()
                .facility_bands()
                .all(|band| band.spatial_geometry().is_none())
        );
    }

    #[test]
    fn imported_facility_frame_keeps_the_band_module_as_its_relation_source() {
        let output = Compiler::new()
            .compile(spatial_cross_section_unit_with_frame(
                false, 2.0, true, true,
            ))
            .unwrap();
        let bands = output.lir().facility_bands().collect::<Vec<_>>();
        assert_eq!(
            output
                .lir
                .inner
                .facility_band_geometries
                .iter()
                .map(|geometry| geometry.facility_band.raw())
                .collect::<Vec<_>>(),
            [0, 1]
        );
        let geometry = bands[0]
            .spatial_geometry()
            .expect("fixture contains compiled FacilityBand geometry");
        let sources = output
            .source_map_input()
            .spatial_relation_sources()
            .filter(|source| {
                source.role() == SourceRelationRole::CanonicalFrameFacilityBandGeometry
            })
            .collect::<Vec<_>>();

        assert_eq!(sources.len(), 2);
        assert!(sources.iter().all(|source| {
            source.owner_ordinal() == geometry.canonical_frame()
                && source.primary_source().source_document_key() == "spatial-cross-section.document"
        }));
    }

    #[test]
    fn invalid_facility_geometry_fails_without_exposing_partial_lir() {
        let diagnostics = Compiler::new()
            .compile(spatial_cross_section_unit(false, f32::NAN, true))
            .err()
            .expect("non-finite FacilityBand geometry must fail");
        assert!(diagnostics.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == DiagnosticCode::InvalidFacilityBandGeometry
                && matches!(
                    diagnostic.payload(),
                    DiagnosticPayload::InvalidFacilityBandGeometry {
                        violation: crate::SpatialGeometryViolation::NonFiniteCoordinate { .. },
                        ..
                    }
                )
        }));
    }

    #[test]
    fn facility_geometry_without_any_canonical_frame_reports_the_reference_failure() {
        let mut input = unit([cross_section_module(false)]);
        let module = &mut input.modules[0];
        let namespace: Arc<str> = module.descriptor().authoring_namespace_id().into();
        let band = module
            .declarations
            .iter_mut()
            .find_map(|declaration| match declaration {
                TypedAstDeclaration::FacilityBand(band) => Some(band),
                _ => None,
            })
            .unwrap();
        band.compiled_geometry = Some(CompiledFacilityBandGeometry {
            length: EdgeLength::try_new(10.0).unwrap(),
            canonical_frame: OwnedEntityReference::<CanonicalFrameKind>::new(
                namespace,
                Arc::from("missing-frame"),
                band.header.span.clone(),
            ),
            centerline_points: [
                CanonicalPoint3F32Input {
                    x: 0.0,
                    y: 0.0,
                    z: 2.0,
                },
                CanonicalPoint3F32Input {
                    x: 10.0,
                    y: 0.0,
                    z: 2.0,
                },
            ]
            .into(),
            source_ranges: Box::new([]),
        });

        let diagnostics = Compiler::new()
            .compile(input)
            .err()
            .expect("compiled FacilityBand without a resolvable frame must fail");
        assert_eq!(
            diagnostics
                .diagnostics()
                .iter()
                .map(Diagnostic::code)
                .collect::<Vec<_>>(),
            [DiagnosticCode::UnknownReferenceTarget]
        );
    }

    #[test]
    fn spatial_geometry_rejects_length_mismatch_without_partial_output() {
        let points = [
            CanonicalPoint3F32Input {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            CanonicalPoint3F32Input {
                x: 19.0,
                y: 0.0,
                z: 0.0,
            },
        ];
        let geometries = [LaneEdgeGeometryInput {
            lane_edge: LaneEdgeReference::local("edge-main"),
            centerline_points: &points,
        }];
        let mut builder = access_builder("canonical-spatial-length.document");
        builder
            .add_canonical_frame(CanonicalFrameInput {
                canonical_frame_key: "frame-main",
                lane_edge_geometries: &geometries,
            })
            .unwrap();

        let diagnostics = Compiler::new()
            .compile(unit([builder.finish().unwrap()]))
            .err()
            .expect("mismatched geometry must fail");
        assert_eq!(diagnostics.diagnostics().len(), 1);
        assert_eq!(
            diagnostics.diagnostics()[0].code(),
            DiagnosticCode::InvalidSpatialGeometry
        );
        assert!(matches!(
            diagnostics.diagnostics()[0].payload(),
            DiagnosticPayload::InvalidSpatialGeometry {
                violation: crate::SpatialGeometryViolation::LengthMismatch { .. },
                ..
            }
        ));
    }

    #[test]
    fn spatial_geometry_set_order_does_not_change_lir_semantics() {
        let compile = |reverse: bool| {
            let limits = CompileLimits::p100_initial_v1();
            let header = SourceModuleHeader::new(
                SourceModuleHeaderInput {
                    authoring_namespace_id: "city/spatial-order",
                    source_document_key: if reverse {
                        "spatial-order-reverse.document"
                    } else {
                        "spatial-order.document"
                    },
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
            for key in ["edge-a", "edge-b"] {
                builder
                    .add_lane_edge(LaneEdgeInput {
                        lane_edge_key: key,
                        length_meters: 10.0,
                        speed_limit_meters_per_second: 10.0,
                        successors: &[],
                    })
                    .unwrap();
            }
            let points_a = [
                CanonicalPoint3F32Input {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                CanonicalPoint3F32Input {
                    x: 10.0,
                    y: 0.0,
                    z: 0.0,
                },
            ];
            let points_b = [
                CanonicalPoint3F32Input {
                    x: 20.0,
                    y: 0.0,
                    z: 0.0,
                },
                CanonicalPoint3F32Input {
                    x: 30.0,
                    y: 0.0,
                    z: 0.0,
                },
            ];
            let ordered = [
                LaneEdgeGeometryInput {
                    lane_edge: LaneEdgeReference::local("edge-a"),
                    centerline_points: &points_a,
                },
                LaneEdgeGeometryInput {
                    lane_edge: LaneEdgeReference::local("edge-b"),
                    centerline_points: &points_b,
                },
            ];
            let reversed = [ordered[1], ordered[0]];
            builder
                .add_canonical_frame(CanonicalFrameInput {
                    canonical_frame_key: "frame-main",
                    lane_edge_geometries: if reverse { &reversed } else { &ordered },
                })
                .unwrap();
            Compiler::new()
                .compile(unit([builder.finish().unwrap()]))
                .unwrap()
        };

        assert_eq!(
            compile(false).lir.inner.semantic_digest,
            compile(true).lir.inner.semantic_digest
        );
    }

    #[test]
    fn spatial_geometry_requires_complete_coverage_once_enabled() {
        let limits = CompileLimits::p100_initial_v1();
        let header = SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: "city/spatial-coverage",
                source_document_key: "spatial-coverage.document",
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
        for key in ["edge-a", "edge-b"] {
            builder
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: key,
                    length_meters: 10.0,
                    speed_limit_meters_per_second: 10.0,
                    successors: &[],
                })
                .unwrap();
        }
        let points = [
            CanonicalPoint3F32Input {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            CanonicalPoint3F32Input {
                x: 10.0,
                y: 0.0,
                z: 0.0,
            },
        ];
        let geometries = [LaneEdgeGeometryInput {
            lane_edge: LaneEdgeReference::local("edge-a"),
            centerline_points: &points,
        }];
        builder
            .add_canonical_frame(CanonicalFrameInput {
                canonical_frame_key: "frame-main",
                lane_edge_geometries: &geometries,
            })
            .unwrap();

        let diagnostics = Compiler::new()
            .compile(unit([builder.finish().unwrap()]))
            .err()
            .expect("partial coverage must fail");
        assert!(matches!(
            diagnostics.diagnostics()[0].payload(),
            DiagnosticPayload::InvalidSpatialGeometry {
                lane_edge_key,
                violation: crate::SpatialGeometryViolation::MissingEdgeBinding,
                ..
            } if lane_edge_key.as_ref() == "edge-b"
        ));
    }

    #[test]
    fn vehicle_profile_frontend_rejects_invalid_scalars_and_deceleration_order() {
        let mut invalid_scalar = access_builder("vehicle-profile-invalid-scalar.document");
        invalid_scalar
            .add_participant_class(ParticipantClassInput {
                participant_class_key: "car",
                extends: None,
            })
            .unwrap();
        let mut iidm = canonical_iidm_profile();
        iidm.min_gap_meters = -0.1;
        let diagnostics = match invalid_scalar.add_vehicle_profile(VehicleProfileInput {
            vehicle_profile_key: "invalid-gap",
            participant_class: ParticipantClassReference::local("car"),
            iidm,
        }) {
            Ok(_) => panic!("negative minGap must fail"),
            Err(diagnostics) => diagnostics,
        };
        assert_eq!(
            diagnostics.diagnostics()[0].code(),
            DiagnosticCode::InvalidVehicleProfileValue
        );

        let mut invalid_order = access_builder("vehicle-profile-invalid-order.document");
        invalid_order
            .add_participant_class(ParticipantClassInput {
                participant_class_key: "car",
                extends: None,
            })
            .unwrap();
        let mut iidm = canonical_iidm_profile();
        iidm.emergency_deceleration_meters_per_second_squared = 1.0;
        let diagnostics = match invalid_order.add_vehicle_profile(VehicleProfileInput {
            vehicle_profile_key: "invalid-order",
            participant_class: ParticipantClassReference::local("car"),
            iidm,
        }) {
            Ok(_) => panic!("invalid deceleration order must fail"),
            Err(diagnostics) => diagnostics,
        };
        assert_eq!(
            diagnostics.diagnostics()[0].code(),
            DiagnosticCode::InvalidVehicleProfileDecelerationOrder
        );
    }

    #[test]
    fn vehicle_profile_unknown_participant_class_fails_during_hir_resolution() {
        let mut builder = access_builder("vehicle-profile-unknown-class.document");
        builder
            .add_vehicle_profile(VehicleProfileInput {
                vehicle_profile_key: "standard-car",
                participant_class: ParticipantClassReference::local("missing"),
                iidm: canonical_iidm_profile(),
            })
            .unwrap();
        assert_eq!(
            compile_diagnostic_codes(builder),
            [DiagnosticCode::UnknownReferenceTarget]
        );
    }

    #[test]
    fn compiler_freezes_participant_hierarchy_access_rules_and_sources() {
        let output = Compiler::new()
            .compile(unit([access_semantics_module(false)]))
            .unwrap();
        assert_eq!(output.lir().participant_classes().len(), 2);
        assert_eq!(output.lir().access_rules().len(), 2);
        let classes = output
            .lir()
            .participant_classes()
            .map(|class| {
                (
                    stable_key(class.identity_fields(), FieldTag::ParticipantClassKey),
                    class.ordinal(),
                    class.parent(),
                    class.depth(),
                )
            })
            .collect::<Vec<_>>();
        let road_user = classes.iter().find(|class| class.0 == "road-user").unwrap();
        let car = classes.iter().find(|class| class.0 == "car").unwrap();
        assert_eq!(road_user.2, None);
        assert_eq!(road_user.3, 0);
        assert_eq!(car.2, Some(road_user.1));
        assert_eq!(car.3, 1);
        assert!(
            output
                .lir()
                .participant_class(road_user.1)
                .unwrap()
                .contains(car.1)
        );

        let allow = output
            .lir()
            .access_rules()
            .find(|rule| {
                stable_key(rule.identity_fields(), FieldTag::AccessRuleKey) == "allow-road-users"
            })
            .unwrap();
        assert_eq!(allow.effect(), AccessEffect::Allow);
        assert_eq!(allow.participant_classes(), &[road_user.1]);
        assert_eq!(allow.regulation().unwrap().jurisdiction(), "CN-test");
        assert!(matches!(allow.target(), CanonicalAccessTarget::LaneEdge(_)));

        let source_map = output.source_map_input();
        assert_eq!(source_map.participant_class_sources().len(), 2);
        assert_eq!(source_map.access_rule_sources().len(), 2);
        assert_eq!(source_map.access_relation_sources().len(), 5);
        assert_eq!(
            source_map
                .access_relation_sources()
                .map(|relation| relation.role())
                .collect::<Vec<_>>(),
            [
                SourceRelationRole::ParticipantClassExtends,
                SourceRelationRole::AccessRuleTarget,
                SourceRelationRole::AccessRuleParticipantClass,
                SourceRelationRole::AccessRuleTarget,
                SourceRelationRole::AccessRuleParticipantClass,
            ]
        );

        let permuted = Compiler::new()
            .compile(unit([access_semantics_module(true)]))
            .unwrap();
        assert_eq!(
            output.lir.inner.semantic_digest,
            permuted.lir.inner.semantic_digest
        );
    }

    #[test]
    fn access_validation_rejects_inheritance_cycles_and_exact_rule_ties() {
        let mut cycle = access_builder("access-cycle.document");
        cycle
            .add_participant_class(ParticipantClassInput {
                participant_class_key: "a",
                extends: Some(ParticipantClassReference::local("b")),
            })
            .unwrap()
            .add_participant_class(ParticipantClassInput {
                participant_class_key: "b",
                extends: Some(ParticipantClassReference::local("a")),
            })
            .unwrap();
        assert_eq!(
            compile_diagnostic_codes(cycle),
            [DiagnosticCode::ParticipantClassInheritanceCycle]
        );

        let mut ambiguity = access_builder("access-ambiguity.document");
        ambiguity
            .add_participant_class(ParticipantClassInput {
                participant_class_key: "all",
                extends: None,
            })
            .unwrap();
        for (key, effect) in [
            ("allow-all", AccessEffect::Allow),
            ("deny-all", AccessEffect::Deny),
        ] {
            ambiguity
                .add_access_rule(AccessRuleInput {
                    access_rule_key: key,
                    target: AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local("edge-main")),
                    effect,
                    participant_classes: &[ParticipantClassReference::local("all")],
                    regulation: None,
                    priority: 0,
                })
                .unwrap();
        }
        assert_eq!(
            compile_diagnostic_codes(ambiguity),
            [DiagnosticCode::AccessRuleAmbiguity]
        );
    }

    #[test]
    fn compiler_preserves_all_supported_access_target_planes() {
        let mut edge_targets = access_builder("access-edge-targets.document");
        edge_targets
            .add_lane_group(LaneGroupInput {
                lane_group_key: "group-main",
                road_section: RoadSectionReference::local("section-main"),
            })
            .unwrap()
            .add_road_section(RoadSectionInput {
                road_section_key: "section-main",
                kind_id: "motorLane",
                lanes: &[AuthoringLaneInput {
                    authoring_lane_key: "lane-main",
                    edge_chain: &[LaneEdgeReference::local("edge-main")],
                    lane_group: Some(LaneGroupReference::local("group-main")),
                }],
            })
            .unwrap()
            .add_road_corridor(RoadCorridorInput {
                road_corridor_key: "corridor-main",
                reference_section: RoadSectionReference::local("section-main"),
                elements: &[CorridorElementReference::road_section(
                    RoadSectionReference::local("section-main"),
                )],
            })
            .unwrap()
            .add_participant_class(ParticipantClassInput {
                participant_class_key: "all",
                extends: None,
            })
            .unwrap();
        for (key, target, effect) in [
            (
                "rule-edge",
                AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local("edge-main")),
                AccessEffect::Deny,
            ),
            (
                "rule-group",
                AccessRuleTargetInput::LaneGroup(LaneGroupReference::local("group-main")),
                AccessEffect::Allow,
            ),
            (
                "rule-section",
                AccessRuleTargetInput::RoadSection(RoadSectionReference::local("section-main")),
                AccessEffect::Deny,
            ),
        ] {
            edge_targets
                .add_access_rule(AccessRuleInput {
                    access_rule_key: key,
                    target,
                    effect,
                    participant_classes: &[ParticipantClassReference::local("all")],
                    regulation: None,
                    priority: 0,
                })
                .unwrap();
        }
        let edge_output = Compiler::new()
            .compile(unit([edge_targets.finish().unwrap()]))
            .unwrap();
        assert_eq!(
            edge_output
                .lir()
                .access_rules()
                .map(|rule| match rule.target() {
                    CanonicalAccessTarget::LaneEdge(_) => "edge",
                    CanonicalAccessTarget::LaneGroup(_) => "group",
                    CanonicalAccessTarget::RoadSection(_) => "section",
                    CanonicalAccessTarget::ManeuverPath(_) => "path",
                })
                .collect::<Vec<_>>(),
            ["edge", "group", "section"]
        );

        let mut path_target = junction_builder("access-path-target.document");
        path_target
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
                maneuver_path_key: "path-main",
                movement: MovementReference::local("movement-main"),
                entry_edge: LaneEdgeReference::local("entry"),
                internal_edges: &[],
                exit_edge: LaneEdgeReference::local("exit"),
            })
            .unwrap()
            .add_participant_class(ParticipantClassInput {
                participant_class_key: "all",
                extends: None,
            })
            .unwrap()
            .add_access_rule(AccessRuleInput {
                access_rule_key: "rule-path",
                target: AccessRuleTargetInput::ManeuverPath(ManeuverPathReference::local(
                    "path-main",
                )),
                effect: AccessEffect::Deny,
                participant_classes: &[ParticipantClassReference::local("all")],
                regulation: None,
                priority: 7,
            })
            .unwrap();
        let path_output = Compiler::new()
            .compile(unit([path_target.finish().unwrap()]))
            .unwrap();
        let path_rule = path_output.lir().access_rules().next().unwrap();
        assert!(matches!(
            path_rule.target(),
            CanonicalAccessTarget::ManeuverPath(_)
        ));
        assert_eq!(path_rule.priority(), 7);
    }

    #[test]
    fn access_validation_closes_shape_capability_reference_and_regulation_failures() {
        let mut empty = access_builder("access-empty-classes.document");
        empty
            .add_access_rule(AccessRuleInput {
                access_rule_key: "empty",
                target: AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local("edge-main")),
                effect: AccessEffect::Allow,
                participant_classes: &[],
                regulation: None,
                priority: 0,
            })
            .unwrap();
        assert_eq!(
            compile_diagnostic_codes(empty),
            [DiagnosticCode::EmptyAccessRuleParticipantClasses]
        );

        let mut unknown = access_builder("access-unknown-class.document");
        unknown
            .add_access_rule(AccessRuleInput {
                access_rule_key: "unknown",
                target: AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local("edge-main")),
                effect: AccessEffect::Allow,
                participant_classes: &[ParticipantClassReference::local("missing")],
                regulation: None,
                priority: 0,
            })
            .unwrap();
        assert_eq!(
            compile_diagnostic_codes(unknown),
            [DiagnosticCode::UnknownReferenceTarget]
        );

        let mut facility = access_builder("access-facility-band.document");
        facility
            .add_facility_band(FacilityBandInput {
                facility_band_key: "band-main",
                kind_id: "sidewalk",
            })
            .unwrap()
            .add_road_section(RoadSectionInput {
                road_section_key: "section-main",
                kind_id: "motorLane",
                lanes: &[AuthoringLaneInput {
                    authoring_lane_key: "lane-main",
                    edge_chain: &[LaneEdgeReference::local("edge-main")],
                    lane_group: None,
                }],
            })
            .unwrap()
            .add_road_corridor(RoadCorridorInput {
                road_corridor_key: "corridor-main",
                reference_section: RoadSectionReference::local("section-main"),
                elements: &[
                    CorridorElementReference::road_section(RoadSectionReference::local(
                        "section-main",
                    )),
                    CorridorElementReference::facility_band(FacilityBandReference::local(
                        "band-main",
                    )),
                ],
            })
            .unwrap()
            .add_participant_class(ParticipantClassInput {
                participant_class_key: "all",
                extends: None,
            })
            .unwrap()
            .add_access_rule(AccessRuleInput {
                access_rule_key: "band-rule",
                target: AccessRuleTargetInput::FacilityBand(FacilityBandReference::local(
                    "band-main",
                )),
                effect: AccessEffect::Allow,
                participant_classes: &[ParticipantClassReference::local("all")],
                regulation: None,
                priority: 0,
            })
            .unwrap();
        let diagnostics = match Compiler::new().compile(unit([facility.finish().unwrap()])) {
            Ok(_) => panic!("FacilityBand target must fail closed"),
            Err(diagnostics) => diagnostics,
        };
        assert!(matches!(
            diagnostics.diagnostics()[0].payload(),
            DiagnosticPayload::AccessCapabilityUnavailable {
                capability: AccessCapability::FacilityBandTarget,
                ..
            }
        ));

        let mut invalid_regulation = access_builder("access-invalid-regulation.document");
        invalid_regulation
            .add_participant_class(ParticipantClassInput {
                participant_class_key: "all",
                extends: None,
            })
            .unwrap()
            .add_access_rule(AccessRuleInput {
                access_rule_key: "invalid-regulation",
                target: AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local("edge-main")),
                effect: AccessEffect::Allow,
                participant_classes: &[ParticipantClassReference::local("all")],
                regulation: Some(AccessRegulationInput {
                    jurisdiction: "",
                    version: "2026-01",
                    source: None,
                }),
                priority: 0,
            })
            .unwrap();
        assert_eq!(
            compile_diagnostic_codes(invalid_regulation),
            [DiagnosticCode::InvalidAccessRegulationString]
        );

        let mut mismatch = access_builder("access-regulation-mismatch.document");
        mismatch
            .add_participant_class(ParticipantClassInput {
                participant_class_key: "all",
                extends: None,
            })
            .unwrap();
        for (key, jurisdiction) in [("rule-a", "CN-a"), ("rule-b", "CN-b")] {
            mismatch
                .add_access_rule(AccessRuleInput {
                    access_rule_key: key,
                    target: AccessRuleTargetInput::LaneEdge(LaneEdgeReference::local("edge-main")),
                    effect: AccessEffect::Allow,
                    participant_classes: &[ParticipantClassReference::local("all")],
                    regulation: Some(AccessRegulationInput {
                        jurisdiction,
                        version: "2026-01",
                        source: None,
                    }),
                    priority: 0,
                })
                .unwrap();
        }
        assert_eq!(
            compile_diagnostic_codes(mismatch),
            [DiagnosticCode::AccessRegulationMismatch]
        );
    }
}
