//! 官方来源编译到原子已验证输出的公共入口。
//!
//! [`Compiler::compile`] 是唯一能够构造 [`ValidatedCanonicalLir`]、
//! [`ValidatedSourceMapInput`] 和 [`CompilationOutput`] 的路径。当前实现是干净单工作线程
//! 确定性预言机：每个阶段成功后才提交下一阶段，任一错误只返回
//! [`DiagnosticBundle`]；来源伴随数据在 AST/HIR/MIR 释放前冻结。

use laneflow_static_contract::{
    AccessRuleOrdinal, AuthoringLaneOrdinal, CanonicalFrameOrdinal, FacilityBandOrdinal,
    JunctionOrdinal, LaneEdgeOrdinal, LaneGroupOrdinal, ManeuverGateOrdinal, ManeuverPathOrdinal,
    MovementOrdinal, ParkingFacilityOrdinal, ParkingSpaceOrdinal, ParticipantClassOrdinal,
    RoadCorridorOrdinal, RoadSectionOrdinal, SignalControllerOrdinal, SignalGroupOrdinal,
    SignalPhaseOrdinal, StopLineOrdinal, VehicleProfileOrdinal, WaitingZoneOrdinal,
};

use crate::hir::build_hir;
use crate::lir::{LirUnit, freeze_lir};
use crate::mir::lower_to_mir;
use crate::source_map::freeze_source_map;
use crate::{CompilationUnit, DiagnosticBundle};

#[cfg(test)]
use crate::Diagnostic;
#[cfg(test)]
use laneflow_static_contract::{AccessEffect, FieldTag, SignalAspect};

mod output;
mod views;

pub use output::{CompilationMetrics, CompilationOutput};
pub use views::{
    CanonicalAccessRegulationView, CanonicalAccessRuleView, CanonicalAccessTarget,
    CanonicalAuthoringLaneView, CanonicalCorridorElement, CanonicalFacilityBandGeometryView,
    CanonicalFacilityBandView, CanonicalFrameView, CanonicalIdentityFieldView,
    CanonicalJunctionInternalEdgeView, CanonicalJunctionView, CanonicalLaneEdgeGeometryView,
    CanonicalLaneEdgeView, CanonicalLaneGroupView, CanonicalManeuverGateView,
    CanonicalManeuverPathView, CanonicalMovementView, CanonicalParkingFacilityView,
    CanonicalParkingLaneAnchor, CanonicalParkingSpaceGeometry, CanonicalParkingSpaceView,
    CanonicalParticipantClassView, CanonicalPoint3F32, CanonicalRoadCorridorView,
    CanonicalRoadSectionView, CanonicalSignalControl, CanonicalSignalControllerView,
    CanonicalSignalGroupView, CanonicalSignalPhaseStateView, CanonicalSignalPhaseView,
    CanonicalSpatialSegment, CanonicalStopLineView, CanonicalVehicleProfileView,
    CanonicalWaitingZoneView,
};

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
            .map(|edge| CanonicalLaneEdgeView::from_lir(&self.inner, edge))
    }

    /// 通过当前 LIR 实例的有类型序号读取车道图边。
    ///
    /// 序号来自其他编译结果时可能命中错误实体；跨编译关联必须先使用 `LaneEdgeId`。
    #[must_use]
    pub fn lane_edge(&self, ordinal: LaneEdgeOrdinal) -> Option<CanonicalLaneEdgeView<'_>> {
        self.inner
            .lane_edges
            .get(ordinal.index())
            .map(|edge| CanonicalLaneEdgeView::from_lir(&self.inner, edge))
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部道路走廊。
    pub fn road_corridors(&self) -> impl ExactSizeIterator<Item = CanonicalRoadCorridorView<'_>> {
        self.inner
            .road_corridors
            .iter()
            .map(|record| CanonicalRoadCorridorView::from_lir(&self.inner, record))
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
            .map(|record| CanonicalRoadCorridorView::from_lir(&self.inner, record))
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部道路区段。
    pub fn road_sections(&self) -> impl ExactSizeIterator<Item = CanonicalRoadSectionView<'_>> {
        self.inner
            .road_sections
            .iter()
            .map(|record| CanonicalRoadSectionView::from_lir(&self.inner, record))
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
            .map(|record| CanonicalRoadSectionView::from_lir(&self.inner, record))
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部编制车道。
    pub fn authoring_lanes(&self) -> impl ExactSizeIterator<Item = CanonicalAuthoringLaneView<'_>> {
        self.inner
            .authoring_lanes
            .iter()
            .map(|record| CanonicalAuthoringLaneView::from_lir(&self.inner, record))
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
            .map(|record| CanonicalAuthoringLaneView::from_lir(&self.inner, record))
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部车道组。
    pub fn lane_groups(&self) -> impl ExactSizeIterator<Item = CanonicalLaneGroupView<'_>> {
        self.inner
            .lane_groups
            .iter()
            .map(|record| CanonicalLaneGroupView::from_lir(&self.inner, record))
    }

    /// 通过当前 LIR 实例的有类型序号读取车道组。
    #[must_use]
    pub fn lane_group(&self, ordinal: LaneGroupOrdinal) -> Option<CanonicalLaneGroupView<'_>> {
        self.inner
            .lane_groups
            .get(ordinal.index())
            .map(|record| CanonicalLaneGroupView::from_lir(&self.inner, record))
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部设施带。
    pub fn facility_bands(&self) -> impl ExactSizeIterator<Item = CanonicalFacilityBandView<'_>> {
        self.inner
            .facility_bands
            .iter()
            .map(|record| CanonicalFacilityBandView::from_lir(&self.inner, record))
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
            .map(|record| CanonicalFacilityBandView::from_lir(&self.inner, record))
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部路口。
    pub fn junctions(&self) -> impl ExactSizeIterator<Item = CanonicalJunctionView<'_>> {
        self.inner
            .junctions
            .iter()
            .map(|record| CanonicalJunctionView::from_lir(&self.inner, record))
    }

    /// 通过当前 LIR 实例的有类型序号读取路口。
    #[must_use]
    pub fn junction(&self, ordinal: JunctionOrdinal) -> Option<CanonicalJunctionView<'_>> {
        self.inner
            .junctions
            .get(ordinal.index())
            .map(|record| CanonicalJunctionView::from_lir(&self.inner, record))
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部转向动作。
    pub fn movements(&self) -> impl ExactSizeIterator<Item = CanonicalMovementView<'_>> {
        self.inner
            .movements
            .iter()
            .map(|record| CanonicalMovementView::from_lir(&self.inner, record))
    }

    /// 通过当前 LIR 实例的有类型序号读取转向动作。
    #[must_use]
    pub fn movement(&self, ordinal: MovementOrdinal) -> Option<CanonicalMovementView<'_>> {
        self.inner
            .movements
            .get(ordinal.index())
            .map(|record| CanonicalMovementView::from_lir(&self.inner, record))
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部机动路径。
    pub fn maneuver_paths(&self) -> impl ExactSizeIterator<Item = CanonicalManeuverPathView<'_>> {
        self.inner
            .maneuver_paths
            .iter()
            .map(|record| CanonicalManeuverPathView::from_lir(&self.inner, record))
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
            .map(|record| CanonicalManeuverPathView::from_lir(&self.inner, record))
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部停止线。
    pub fn stop_lines(&self) -> impl ExactSizeIterator<Item = CanonicalStopLineView<'_>> {
        self.inner
            .stop_lines
            .iter()
            .map(|record| CanonicalStopLineView::from_lir(&self.inner, record))
    }

    /// 通过当前 LIR 实例的有类型序号读取停止线。
    #[must_use]
    pub fn stop_line(&self, ordinal: StopLineOrdinal) -> Option<CanonicalStopLineView<'_>> {
        self.inner
            .stop_lines
            .get(ordinal.index())
            .map(|record| CanonicalStopLineView::from_lir(&self.inner, record))
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部机动门。
    pub fn maneuver_gates(&self) -> impl ExactSizeIterator<Item = CanonicalManeuverGateView<'_>> {
        self.inner
            .maneuver_gates
            .iter()
            .map(|record| CanonicalManeuverGateView::from_lir(&self.inner, record))
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
            .map(|record| CanonicalManeuverGateView::from_lir(&self.inner, record))
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部等待区。
    pub fn waiting_zones(&self) -> impl ExactSizeIterator<Item = CanonicalWaitingZoneView<'_>> {
        self.inner
            .waiting_zones
            .iter()
            .map(|record| CanonicalWaitingZoneView::from_lir(&self.inner, record))
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
            .map(|record| CanonicalWaitingZoneView::from_lir(&self.inner, record))
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部信号组。
    pub fn signal_groups(&self) -> impl ExactSizeIterator<Item = CanonicalSignalGroupView<'_>> {
        self.inner
            .signal_groups
            .iter()
            .map(|record| CanonicalSignalGroupView::from_lir(&self.inner, record))
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
            .map(|record| CanonicalSignalGroupView::from_lir(&self.inner, record))
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部固定时制信号控制器。
    pub fn signal_controllers(
        &self,
    ) -> impl ExactSizeIterator<Item = CanonicalSignalControllerView<'_>> {
        self.inner
            .signal_controllers
            .iter()
            .map(|record| CanonicalSignalControllerView::from_lir(&self.inner, record))
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
            .map(|record| CanonicalSignalControllerView::from_lir(&self.inner, record))
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部所有者局部（owner-local）信号相位。
    pub fn signal_phases(&self) -> impl ExactSizeIterator<Item = CanonicalSignalPhaseView<'_>> {
        self.inner
            .signal_phases
            .iter()
            .map(|record| CanonicalSignalPhaseView::from_lir(&self.inner, record))
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
            .map(|record| CanonicalSignalPhaseView::from_lir(&self.inner, record))
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部停车区域。
    pub fn parking_facilities(
        &self,
    ) -> impl ExactSizeIterator<Item = CanonicalParkingFacilityView<'_>> {
        self.inner
            .parking_facilities
            .iter()
            .map(|record| CanonicalParkingFacilityView::from_lir(&self.inner, record))
    }

    /// 通过当前 LIR 实例的有类型序号读取停车区域。
    #[must_use]
    pub fn parking_facility(
        &self,
        ordinal: ParkingFacilityOrdinal,
    ) -> Option<CanonicalParkingFacilityView<'_>> {
        self.inner
            .parking_facilities
            .get(ordinal.index())
            .map(|record| CanonicalParkingFacilityView::from_lir(&self.inner, record))
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部停车位。
    pub fn parking_spaces(&self) -> impl ExactSizeIterator<Item = CanonicalParkingSpaceView<'_>> {
        self.inner
            .parking_spaces
            .iter()
            .map(|record| CanonicalParkingSpaceView::from_lir(&self.inner, record))
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
            .map(|record| CanonicalParkingSpaceView::from_lir(&self.inner, record))
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部参与者类别。
    pub fn participant_classes(
        &self,
    ) -> impl ExactSizeIterator<Item = CanonicalParticipantClassView<'_>> {
        self.inner
            .participant_classes
            .iter()
            .map(|record| CanonicalParticipantClassView::from_lir(&self.inner, record))
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
            .map(|record| CanonicalParticipantClassView::from_lir(&self.inner, record))
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部车辆配置。
    pub fn vehicle_profiles(
        &self,
    ) -> impl ExactSizeIterator<Item = CanonicalVehicleProfileView<'_>> {
        self.inner
            .vehicle_profiles
            .iter()
            .map(|record| CanonicalVehicleProfileView::from_lir(&self.inner, record))
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
            .map(|record| CanonicalVehicleProfileView::from_lir(&self.inner, record))
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部规范坐标框架。
    pub fn canonical_frames(&self) -> impl ExactSizeIterator<Item = CanonicalFrameView<'_>> {
        self.inner
            .canonical_frames
            .iter()
            .map(|record| CanonicalFrameView::from_lir(&self.inner, record))
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
            .map(|record| CanonicalFrameView::from_lir(&self.inner, record))
    }

    /// 按完整 Identity v1 前像规范顺序遍历全部静态准入规则。
    pub fn access_rules(&self) -> impl ExactSizeIterator<Item = CanonicalAccessRuleView<'_>> {
        self.inner
            .access_rules
            .iter()
            .map(|record| CanonicalAccessRuleView::from_lir(&self.inner, record))
    }

    /// 通过当前 LIR 实例的有类型序号读取静态准入规则。
    #[must_use]
    pub fn access_rule(&self, ordinal: AccessRuleOrdinal) -> Option<CanonicalAccessRuleView<'_>> {
        self.inner
            .access_rules
            .get(ordinal.index())
            .map(|record| CanonicalAccessRuleView::from_lir(&self.inner, record))
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
            .map(CanonicalJunctionInternalEdgeView::from_record)
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
        let selected_limits = unit.limits.clone();
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
        let metrics = CompilationMetrics::from_pipeline(
            lir_record_count,
            output_logical_bytes,
            hir_peak_controlled_live_bytes
                .max(mir_peak_controlled_live_bytes)
                .max(lir_peak_controlled_live_bytes)
                .max(source_map_input.peak_controlled_live_bytes()),
            semantic_fingerprint,
        );
        drop(mir);
        let crate::lir::LirFreezeOutput { lir, .. } = frozen_lir;
        Ok(CompilationOutput::from_success(
            ValidatedCanonicalLir { inner: lir },
            source_map_input,
            Box::default(),
            metrics,
            selected_limits,
        ))
    }
}

#[cfg(test)]
pub(crate) mod portable_fixture_tests;

#[cfg(test)]
mod tests;
