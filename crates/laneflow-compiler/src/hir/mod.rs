//! Typed AST 到高层中间表示（HIR）的符号解析阶段。
//!
//! 输入 [`CompilationUnit`] 已闭合模块导入图并冻结依赖优先顺序。本阶段据此建立连续
//! 模块表与分实体符号表，把 `(module namespace, typed entity address)` 引用解析为阶段私有
//! `u32` 键，并保留来源位置供后续诊断/源映射使用。声明先全部登记、再统一解析引用，
//! 因此前向引用和自环合法；横断面子阶段在派生子实体身份前证明唯一所有者树，路口
//! 子阶段则闭合父子身份、完整机动路径与内部边角色。
//!
//! HIR 表顺序是规范顺序：模块沿用编译单元顺序，模块内声明按稳定键排序，导入和连接
//! 也使用已显式规范化的序列。`HashMap` 仅作查找，绝不能通过迭代哈希表决定诊断或
//! 后续布局。所有键、区间和类型均为 crate 私有，不能跨阶段或进入持久制品。
//!
//! 构建前的记录数统计与内存/限额预检由 [`plan`] 子模块承接；本模块保留编排、共享定义、
//! 领域 façade 再导出与原子装配。

mod base;
mod plan;

mod access;
mod control;
mod cross_section;
mod junction;
mod parking;
mod signal;
mod spatial;

#[cfg(test)]
mod tests;

use crate::arena::{ArenaKey, ArenaKeyOverflow};
use crate::declaration::{LaneEdgeDeclaration, TypedAstDeclaration};
use crate::geometry_profile::GeometryCompilationProfiles;
use crate::{CompilationUnit, CompileLimitDimension, Diagnostic, DiagnosticBundle, SourceLocation};

use base::{
    CanonicalLaneEdgeSource, HirBase, SymbolTable, derive_identity, register_owner,
    resolve_reference,
};
pub(crate) use base::{HirImport, HirLaneEdge, HirLaneEdgeReference, HirModule};
use plan::{
    AccessCounts, ControlCounts, CrossSectionCounts, HirBuildPlan, JunctionCounts, ParkingCounts,
    SignalCounts, SpatialCounts,
};

pub(crate) use access::{
    HirAccessRule, HirAccessRuleParticipantClass, HirAccessTarget, HirParticipantClass,
    HirVehicleProfile,
};
pub(crate) use control::{
    HirManeuverGate, HirManeuverPathGate, HirManeuverPathWaitingZone, HirStopLine,
    HirStopLineManeuverGate, HirWaitingZone,
};
pub(crate) use cross_section::{
    HirAuthoringLane, HirAuthoringLaneEdge, HirCorridorElement, HirFacilityBand, HirLaneGroup,
    HirLaneGroupMember, HirRoadCorridor, HirRoadSection,
};
pub(crate) use junction::{
    HirJunction, HirJunctionInternalEdge, HirJunctionMovement, HirManeuverPath,
    HirManeuverPathEdge, HirMovement, HirMovementManeuverPath,
};
pub(crate) use parking::{HirParkingArea, HirParkingAreaSpace, HirParkingSpace};
pub(crate) use signal::{
    HirSignalControl, HirSignalController, HirSignalControllerGroup, HirSignalGroup,
    HirSignalGroupManeuverGate, HirSignalPhase, HirSignalPhaseState,
};
pub(crate) use spatial::{
    HirCanonicalFrame, HirCanonicalPoint3F32, HirFacilityBandGeometry, HirGeometrySourceRange,
    HirLaneEdgeGeometry, HirSpatialSegment,
};

use access::{AccessCandidate, AccessHir, build_access_hir};
use control::{ControlHir, build_control_hir};
use cross_section::{CanonicalAuthoringLaneSource, CrossSectionHir, build_cross_section_hir};
use junction::{HirDeclaredJunctionEdge, JunctionHir, ManeuverPathSequence, build_junction_hir};
use parking::{ParkingHir, build_parking_hir, close_parking_anchors_to_emitted_length_mm};
use signal::{SignalHir, build_signal_hir};
#[cfg(test)]
use spatial::canonical_point_distance;
use spatial::{
    PendingSpatialGeometry, SpatialFrameAssignment, SpatialHir, SpatialHirContext,
    build_spatial_hir,
};

#[cfg(test)]
use crate::SpatialGeometryViolation;
#[cfg(test)]
use crate::declaration::{
    CanonicalPoint3F32Input, LaneEdgeGeometryAuthority, TypedAstEntityAddress,
};
#[cfg(test)]
use crate::diagnostic::JunctionEdgeSetViolation;
#[cfg(test)]
use laneflow_static_contract::{
    AccessEffect, EntityKind, SPATIAL_JOIN_POSITION_TOLERANCE_METERS, SignalAspect,
};
#[cfg(test)]
use std::sync::Arc;

/// 区分 HIR 模块表键的零尺寸阶段标记。
pub(crate) enum HirModuleTag {}
/// 区分 HIR 车道图边表键的零尺寸阶段标记。
pub(crate) enum HirLaneEdgeTag {}
pub(crate) enum HirRoadCorridorTag {}
pub(crate) enum HirRoadSectionTag {}
pub(crate) enum HirAuthoringLaneTag {}
pub(crate) enum HirLaneGroupTag {}
pub(crate) enum HirFacilityBandTag {}
pub(crate) enum HirJunctionTag {}
pub(crate) enum HirMovementTag {}
pub(crate) enum HirManeuverPathTag {}
pub(crate) enum HirStopLineTag {}
pub(crate) enum HirManeuverGateTag {}
pub(crate) enum HirWaitingZoneTag {}
pub(crate) enum HirSignalGroupTag {}
pub(crate) enum HirSignalControllerTag {}
pub(crate) enum HirSignalPhaseTag {}
pub(crate) enum HirParkingAreaTag {}
pub(crate) enum HirParkingSpaceTag {}
pub(crate) enum HirParticipantClassTag {}
pub(crate) enum HirVehicleProfileTag {}
pub(crate) enum HirCanonicalFrameTag {}
pub(crate) enum HirAccessRuleTag {}

/// 仅在当前 `HirUnit` 模块表内有效的致密键。
pub(crate) type HirModuleKey = ArenaKey<HirModuleTag>;
/// 仅在当前 `HirUnit` 车道图边表内有效的致密键。
pub(crate) type HirLaneEdgeKey = ArenaKey<HirLaneEdgeTag>;
pub(crate) type HirRoadCorridorKey = ArenaKey<HirRoadCorridorTag>;
pub(crate) type HirRoadSectionKey = ArenaKey<HirRoadSectionTag>;
pub(crate) type HirAuthoringLaneKey = ArenaKey<HirAuthoringLaneTag>;
pub(crate) type HirLaneGroupKey = ArenaKey<HirLaneGroupTag>;
pub(crate) type HirFacilityBandKey = ArenaKey<HirFacilityBandTag>;
pub(crate) type HirJunctionKey = ArenaKey<HirJunctionTag>;
pub(crate) type HirMovementKey = ArenaKey<HirMovementTag>;
pub(crate) type HirManeuverPathKey = ArenaKey<HirManeuverPathTag>;
pub(crate) type HirStopLineKey = ArenaKey<HirStopLineTag>;
pub(crate) type HirManeuverGateKey = ArenaKey<HirManeuverGateTag>;
pub(crate) type HirWaitingZoneKey = ArenaKey<HirWaitingZoneTag>;
pub(crate) type HirSignalGroupKey = ArenaKey<HirSignalGroupTag>;
pub(crate) type HirSignalControllerKey = ArenaKey<HirSignalControllerTag>;
pub(crate) type HirParkingAreaKey = ArenaKey<HirParkingAreaTag>;
pub(crate) type HirParkingSpaceKey = ArenaKey<HirParkingSpaceTag>;
pub(crate) type HirParticipantClassKey = ArenaKey<HirParticipantClassTag>;
pub(crate) type HirVehicleProfileKey = ArenaKey<HirVehicleProfileTag>;
pub(crate) type HirCanonicalFrameKey = ArenaKey<HirCanonicalFrameTag>;
pub(crate) type HirAccessRuleKey = ArenaKey<HirAccessRuleTag>;

/// HIR 阶段成功后一次性冻结的连续只读表集合。
///
/// 构造完成时所有引用均已解析，所有 `TableRange` 都落在对应平坦表内。字段中的键只对
/// 本实例有效。`controlled_live_bytes` 统计成功返回后由 HIR 自身持有的阶段字节；
/// `peak_controlled_live_bytes` 另保存资源预检已经计算的输入、查找表和暂存区共存峰值。
#[derive(Debug, PartialEq)]
pub(crate) struct HirUnit {
    /// 全编译单元唯一的道路几何编译档；无 RoadEditing 规范几何时为 `None`。
    pub(crate) geometry_profiles: Option<GeometryCompilationProfiles>,
    pub(crate) modules: Box<[HirModule]>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) imports: Box<[HirImport]>,
    pub(crate) lane_edges: Box<[HirLaneEdge]>,
    pub(crate) lane_edge_references: Box<[HirLaneEdgeReference]>,
    pub(crate) road_corridors: Box<[HirRoadCorridor]>,
    pub(crate) corridor_elements: Box<[HirCorridorElement]>,
    pub(crate) road_sections: Box<[HirRoadSection]>,
    pub(crate) authoring_lanes: Box<[HirAuthoringLane]>,
    pub(crate) authoring_lane_edges: Box<[HirAuthoringLaneEdge]>,
    pub(crate) lane_groups: Box<[HirLaneGroup]>,
    pub(crate) lane_group_members: Box<[HirLaneGroupMember]>,
    pub(crate) facility_bands: Box<[HirFacilityBand]>,
    pub(crate) junctions: Box<[HirJunction]>,
    pub(crate) movements: Box<[HirMovement]>,
    pub(crate) junction_movements: Box<[HirJunctionMovement]>,
    pub(crate) maneuver_paths: Box<[HirManeuverPath]>,
    pub(crate) movement_maneuver_paths: Box<[HirMovementManeuverPath]>,
    pub(crate) maneuver_path_edges: Box<[HirManeuverPathEdge]>,
    pub(crate) junction_internal_edges: Box<[HirJunctionInternalEdge]>,
    pub(crate) stop_lines: Box<[HirStopLine]>,
    pub(crate) maneuver_gates: Box<[HirManeuverGate]>,
    pub(crate) waiting_zones: Box<[HirWaitingZone]>,
    pub(crate) maneuver_path_gates: Box<[HirManeuverPathGate]>,
    pub(crate) maneuver_path_waiting_zones: Box<[HirManeuverPathWaitingZone]>,
    pub(crate) stop_line_maneuver_gates: Box<[HirStopLineManeuverGate]>,
    pub(crate) signal_groups: Box<[HirSignalGroup]>,
    pub(crate) signal_controllers: Box<[HirSignalController]>,
    pub(crate) signal_controller_groups: Box<[HirSignalControllerGroup]>,
    pub(crate) signal_phases: Box<[HirSignalPhase]>,
    pub(crate) signal_phase_states: Box<[HirSignalPhaseState]>,
    pub(crate) signal_group_maneuver_gates: Box<[HirSignalGroupManeuverGate]>,
    pub(crate) parking_areas: Box<[HirParkingArea]>,
    pub(crate) parking_spaces: Box<[HirParkingSpace]>,
    pub(crate) parking_area_spaces: Box<[HirParkingAreaSpace]>,
    pub(crate) participant_classes: Box<[HirParticipantClass]>,
    pub(crate) vehicle_profiles: Box<[HirVehicleProfile]>,
    pub(crate) canonical_frames: Box<[HirCanonicalFrame]>,
    pub(crate) lane_edge_geometries: Box<[HirLaneEdgeGeometry]>,
    pub(crate) facility_band_geometries: Box<[HirFacilityBandGeometry]>,
    pub(crate) geometry_source_ranges: Box<[HirGeometrySourceRange]>,
    pub(crate) canonical_points: Box<[HirCanonicalPoint3F32]>,
    pub(crate) spatial_segments: Box<[HirSpatialSegment]>,
    pub(crate) access_rules: Box<[HirAccessRule]>,
    pub(crate) access_rule_participant_classes: Box<[HirAccessRuleParticipantClass]>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) hir_record_count: u64,
    pub(crate) controlled_live_bytes: u64,
    pub(crate) peak_controlled_live_bytes: u64,
}

#[derive(Clone, Copy)]
struct CanonicalDeclarationSource<K> {
    source_module_index: u32,
    declaration_index: u32,
    hir_key: K,
}

/// 部件装配（parts/finish）：基础构造与八领域部件在零错误后的聚合输入。
///
/// 任一阶段返回 Err 时整体放弃，不产生部分 `HirUnit`；只有全部部件就绪才调用
/// `finish` 原子装配。
struct HirParts {
    base: HirBase,
    cross_section: CrossSectionHir,
    junction: JunctionHir,
    control: ControlHir,
    signal: SignalHir,
    parking: ParkingHir,
    spatial: SpatialHir,
    access: AccessHir,
}

impl HirParts {
    /// 原子装配 `HirUnit`：消费全部部件并转为平坦 boxed 表；全部可失败阶段在此之前
    /// 已经完成，本函数不再引入新的失败点。
    fn finish(self, plan: &HirBuildPlan) -> HirUnit {
        HirUnit {
            geometry_profiles: self.spatial.geometry_profiles,
            modules: self.base.modules.into_boxed_slice(),
            imports: self.base.imports.into_boxed_slice(),
            lane_edges: self.base.lane_edges.into_boxed_slice(),
            lane_edge_references: self.base.lane_edge_references.into_boxed_slice(),
            road_corridors: self.cross_section.road_corridors,
            corridor_elements: self.cross_section.corridor_elements,
            road_sections: self.cross_section.road_sections,
            authoring_lanes: self.cross_section.authoring_lanes,
            authoring_lane_edges: self.cross_section.authoring_lane_edges,
            lane_groups: self.cross_section.lane_groups,
            lane_group_members: self.cross_section.lane_group_members,
            facility_bands: self.cross_section.facility_bands,
            junctions: self.junction.junctions,
            movements: self.junction.movements,
            junction_movements: self.junction.junction_movements,
            maneuver_paths: self.junction.maneuver_paths,
            movement_maneuver_paths: self.junction.movement_maneuver_paths,
            maneuver_path_edges: self.junction.maneuver_path_edges,
            junction_internal_edges: self.junction.junction_internal_edges,
            stop_lines: self.control.stop_lines,
            maneuver_gates: self.control.maneuver_gates,
            waiting_zones: self.control.waiting_zones,
            maneuver_path_gates: self.control.maneuver_path_gates,
            maneuver_path_waiting_zones: self.control.maneuver_path_waiting_zones,
            stop_line_maneuver_gates: self.control.stop_line_maneuver_gates,
            signal_groups: self.signal.signal_groups,
            signal_controllers: self.signal.signal_controllers,
            signal_controller_groups: self.signal.signal_controller_groups,
            signal_phases: self.signal.signal_phases,
            signal_phase_states: self.signal.signal_phase_states,
            signal_group_maneuver_gates: self.signal.signal_group_maneuver_gates,
            parking_areas: self.parking.parking_areas,
            parking_spaces: self.parking.parking_spaces,
            parking_area_spaces: self.parking.parking_area_spaces,
            canonical_frames: self.spatial.canonical_frames,
            lane_edge_geometries: self.spatial.lane_edge_geometries,
            facility_band_geometries: self.spatial.facility_band_geometries,
            geometry_source_ranges: self.spatial.geometry_source_ranges,
            canonical_points: self.spatial.canonical_points,
            spatial_segments: self.spatial.spatial_segments,
            participant_classes: self.access.participant_classes,
            vehicle_profiles: self.access.vehicle_profiles,
            access_rules: self.access.access_rules,
            access_rule_participant_classes: self.access.access_rule_participant_classes,
            hir_record_count: plan.memory.hir_record_count,
            controlled_live_bytes: plan.memory.persistent_bytes,
            peak_controlled_live_bytes: plan.memory.controlled_live_bytes,
        }
    }
}

/// 建立模块/符号表，解析车道拓扑，并闭合横断面所有者树与稳定身份。
///
/// # Errors
///
/// 当 HIR 记录数、阶段暂存区、编译器控制存续字节或 `u32` 表边界超过所选配置档，
/// 任一来源位置未登记或跨模块错绑，或任一目标稳定键不存在时，返回规范有序诊断。
/// 失败不会返回部分 HIR。
pub(crate) fn build_hir(unit: &CompilationUnit) -> Result<HirUnit, DiagnosticBundle> {
    validate_source_document_ownership(unit)?;

    // 在任何与记录数成正比的阶段分配前，同时预检持久表、lookup 预算和阶段最大暂存区。
    // scratch 取互斥工作集的最大值而非总和，live peak 则包含输入与当时存续的全部集合。
    let plan = HirBuildPlan::analyze(unit);
    plan.check_limits(unit)?;

    let (mut base, mut identities) = HirBase::build(unit, &plan)?;

    let cross_section = build_cross_section_hir(
        unit,
        &plan.cross_section,
        &base.module_lookup,
        &base.lane_edges,
        &base.lane_edge_references,
        &base.lane_edge_symbols,
        &mut identities,
    )?;
    let mut junction = build_junction_hir(
        unit,
        &plan.junction,
        &base.module_lookup,
        &base.lane_edges,
        &base.lane_edge_references,
        &base.lane_edge_symbols,
        &cross_section.authoring_lane_edges,
        &mut identities,
    )?;
    // 仅有的两条跨领域写边之一：control 回写 junction 机动路径的 maneuver_gates /
    // waiting_zones 区间；读者仅 MIR。
    let mut control = build_control_hir(
        unit,
        &plan.control,
        &base.module_lookup,
        &base.lane_edges,
        &base.lane_edge_references,
        &base.lane_edge_symbols,
        &mut junction.maneuver_paths,
        &junction.maneuver_path_edges,
        &mut identities,
    )?;
    // 仅有的两条跨领域写边之二：signal 回写 control 机动门的 signal_control；读者仅 MIR。
    let signal = build_signal_hir(
        unit,
        &plan.signal,
        &base.module_lookup,
        &mut control.maneuver_gates,
        &mut identities,
    )?;
    let parking = build_parking_hir(
        unit,
        &plan.parking,
        &base.module_lookup,
        &base.lane_edges,
        &base.lane_edge_symbols,
        plan.spatial.lane_edge_geometries > 0,
        &mut identities,
    )?;
    let spatial = build_spatial_hir(
        unit,
        &plan.spatial,
        &base.module_lookup,
        SpatialHirContext {
            lane_edges: &mut base.lane_edges,
            lane_edge_references: &base.lane_edge_references,
            lane_edge_symbols: &base.lane_edge_symbols,
            facility_bands: &cross_section.facility_bands,
            maneuver_paths: &junction.maneuver_paths,
            maneuver_path_edges: &junction.maneuver_path_edges,
            junction_internal_edges: &junction.junction_internal_edges,
        },
        &mut identities,
    )?;
    close_parking_anchors_to_emitted_length_mm(
        &parking,
        &base.lane_edges,
        unit.limits.value(CompileLimitDimension::DiagnosticCount),
    )?;
    let access = build_access_hir(
        unit,
        &plan.access,
        &base.module_lookup,
        &base.lane_edges,
        &cross_section,
        &junction.maneuver_paths,
        &mut identities,
    )?;
    // 完整规范前像只服务本阶段的重复/碰撞判断。此后各表仅保留 16 字节有类型 ID，
    // 避免在 HIR 与 MIR 中复制可由稳定键和父项重建的 identity envelope。
    drop(identities);

    Ok(HirParts {
        base,
        cross_section,
        junction,
        control,
        signal,
        parking,
        spatial,
        access,
    }
    .finish(&plan))
}

/// 在任何 HIR 语义诊断产生前，核对全部模块、导入、声明与关系位置的文档所有权。
///
/// 编译单元已经按规范模块顺序冻结，因此首次失败稳定地由模块顺序和声明内结构顺序
/// 决定。该遍历只复用冻结文档索引，不分配、不保留第二份位置表。
fn validate_source_document_ownership(unit: &CompilationUnit) -> Result<(), DiagnosticBundle> {
    for (module_index, module) in unit.modules.iter().enumerate() {
        let module_ordinal = u32::try_from(module_index)
            .expect("compile limits bound canonical module ordinals to u32");
        unit.resolve_source_document_for_module(module_ordinal, module.declaration_span())?;
        for (_, span) in module.import_records() {
            unit.resolve_source_document_for_module(module_ordinal, span)?;
        }
        for alignment in &module.road_alignments {
            alignment.try_visit_source_locations(|span| {
                unit.resolve_source_document_for_module(module_ordinal, span)
                    .map(|_| ())
            })?;
        }
        for declaration in &module.declarations {
            declaration.try_visit_source_locations(|span| {
                unit.resolve_source_document_for_module(module_ordinal, span)
                    .map(|_| ())
            })?;
        }
    }
    Ok(())
}

fn lane_edge_declaration(declaration: &TypedAstDeclaration) -> Option<&LaneEdgeDeclaration> {
    match declaration {
        TypedAstDeclaration::LaneEdge(declaration) => Some(declaration),
        _ => None,
    }
}

fn declaration_header(declaration: &TypedAstDeclaration) -> &crate::declaration::DeclarationHeader {
    match declaration {
        TypedAstDeclaration::LaneEdge(declaration) => &declaration.header,
        TypedAstDeclaration::RoadCorridor(declaration) => &declaration.header,
        TypedAstDeclaration::RoadSection(declaration) => &declaration.header,
        TypedAstDeclaration::LaneGroup(declaration) => &declaration.header,
        TypedAstDeclaration::FacilityBand(declaration) => &declaration.header,
        TypedAstDeclaration::Junction(declaration) => &declaration.header,
        TypedAstDeclaration::Movement(declaration) => &declaration.header,
        TypedAstDeclaration::ManeuverPath(declaration) => &declaration.header,
        TypedAstDeclaration::StopLine(declaration) => &declaration.header,
        TypedAstDeclaration::ManeuverGate(declaration) => &declaration.header,
        TypedAstDeclaration::WaitingZone(declaration) => &declaration.header,
        TypedAstDeclaration::SignalGroup(declaration) => &declaration.header,
        TypedAstDeclaration::SignalController(declaration) => &declaration.header,
        TypedAstDeclaration::ParkingArea(declaration) => &declaration.header,
        TypedAstDeclaration::ParkingSpace(declaration) => &declaration.header,
        TypedAstDeclaration::ParticipantClass(declaration) => &declaration.header,
        TypedAstDeclaration::VehicleProfile(declaration) => &declaration.header,
        TypedAstDeclaration::CanonicalFrame(declaration) => &declaration.header,
        TypedAstDeclaration::AccessRule(declaration) => &declaration.header,
    }
}

fn count_to_usize(count: u64, limits: &crate::CompileLimits) -> Result<usize, DiagnosticBundle> {
    usize::try_from(count).map_err(|_| arena_overflow(ArenaKeyOverflow, limits, None))
}

fn arena_overflow(
    _: ArenaKeyOverflow,
    limits: &crate::CompileLimits,
    primary_span: Option<SourceLocation>,
) -> DiagnosticBundle {
    DiagnosticBundle::single(Diagnostic::compile_limit_exceeded_at(
        CompileLimitDimension::HirRecordCount,
        limits.value(CompileLimitDimension::HirRecordCount),
        u64::from(u32::MAX) + 1,
        primary_span,
        None,
    ))
}
