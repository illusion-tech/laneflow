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
    pub(crate) const fn unit(&self) -> &LirUnit {
        &self.inner
    }

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
pub(crate) mod portable_fixture_tests;

#[cfg(test)]
mod tests;
