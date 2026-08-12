//! 高层中间表示（HIR）到中层中间表示（MIR）的确定性降级阶段。
//!
//! HIR 已完成模块、车道 / 路口拓扑、横断面所有者与符号解析；本阶段不再接受文本引用，而是把
//! 模块、稳定实体和 owner-local 关系冻结为目标布局中立的连续表。HIR 与 MIR 使用
//! 不同的键标记，并通过显式映射表转换，避免碰巧相同的 `u32` 被跨阶段复用。
//!
//! MIR 仍是 crate 私有编译阶段，不是静态镜像 ABI 或公共制品格式。它保留稳定键、
//! `f64` 交通标量和来源位置；后续 LIR 验证/冻结完成前，调用方不得把这些表视为已验证
//! 发布输出。

use std::sync::Arc;

use laneflow_static_contract::{
    AccessEffect, AccessRuleId, AuthoringLaneId, CanonicalFrameId, FacilityBandId, JunctionId,
    LaneEdgeId, LaneGroupId, ManeuverGateId, ManeuverPathId, MovementId, ParkingAreaId,
    ParkingSpaceId, ParticipantClassId, RoadCorridorId, RoadSectionId, SignalAspect,
    SignalControllerId, SignalGroupId, SignalPhaseId, StaticRouteId, StopLineId, VehicleProfileId,
    WaitingZoneId,
};

use crate::arena::{ArenaKey, ArenaKeyOverflow, TableRange, TypedArena};
use crate::diagnostic::DiagnosticCollector;
use crate::geometry_profile::GeometryCompilationProfiles;
use crate::hir::{HirAccessTarget, HirCorridorElement, HirLaneEdgeKey, HirSignalControl, HirUnit};
use crate::module::ResolvedSourceLocation;
use crate::{CompilationUnit, CompileLimitDimension, Diagnostic, DiagnosticBundle, SourceLocation};

/// 区分 MIR 模块表键的零尺寸阶段标记。
pub(crate) enum MirModuleTag {}
/// 区分 MIR 车道图边表键的零尺寸阶段标记。
pub(crate) enum MirLaneEdgeTag {}
pub(crate) enum MirRoadCorridorTag {}
pub(crate) enum MirRoadSectionTag {}
pub(crate) enum MirAuthoringLaneTag {}
pub(crate) enum MirLaneGroupTag {}
pub(crate) enum MirFacilityBandTag {}
pub(crate) enum MirJunctionTag {}
pub(crate) enum MirMovementTag {}
pub(crate) enum MirManeuverPathTag {}
pub(crate) enum MirStopLineTag {}
pub(crate) enum MirManeuverGateTag {}
pub(crate) enum MirWaitingZoneTag {}
pub(crate) enum MirStaticRouteTag {}
pub(crate) enum MirSignalGroupTag {}
pub(crate) enum MirSignalControllerTag {}
pub(crate) enum MirSignalPhaseTag {}
pub(crate) enum MirParkingAreaTag {}
pub(crate) enum MirParkingSpaceTag {}
pub(crate) enum MirParticipantClassTag {}
pub(crate) enum MirVehicleProfileTag {}
pub(crate) enum MirCanonicalFrameTag {}
pub(crate) enum MirAccessRuleTag {}

/// 仅在当前 `MirUnit` 模块表内有效的致密键。
pub(crate) type MirModuleKey = ArenaKey<MirModuleTag>;
/// 仅在当前 `MirUnit` 车道图边表内有效的致密键。
pub(crate) type MirLaneEdgeKey = ArenaKey<MirLaneEdgeTag>;
pub(crate) type MirRoadCorridorKey = ArenaKey<MirRoadCorridorTag>;
pub(crate) type MirRoadSectionKey = ArenaKey<MirRoadSectionTag>;
pub(crate) type MirAuthoringLaneKey = ArenaKey<MirAuthoringLaneTag>;
pub(crate) type MirLaneGroupKey = ArenaKey<MirLaneGroupTag>;
pub(crate) type MirFacilityBandKey = ArenaKey<MirFacilityBandTag>;
pub(crate) type MirJunctionKey = ArenaKey<MirJunctionTag>;
pub(crate) type MirMovementKey = ArenaKey<MirMovementTag>;
pub(crate) type MirManeuverPathKey = ArenaKey<MirManeuverPathTag>;
pub(crate) type MirStopLineKey = ArenaKey<MirStopLineTag>;
pub(crate) type MirManeuverGateKey = ArenaKey<MirManeuverGateTag>;
pub(crate) type MirWaitingZoneKey = ArenaKey<MirWaitingZoneTag>;
pub(crate) type MirStaticRouteKey = ArenaKey<MirStaticRouteTag>;
pub(crate) type MirSignalGroupKey = ArenaKey<MirSignalGroupTag>;
pub(crate) type MirSignalControllerKey = ArenaKey<MirSignalControllerTag>;
pub(crate) type MirSignalPhaseKey = ArenaKey<MirSignalPhaseTag>;
pub(crate) type MirParkingAreaKey = ArenaKey<MirParkingAreaTag>;
pub(crate) type MirParkingSpaceKey = ArenaKey<MirParkingSpaceTag>;
pub(crate) type MirParticipantClassKey = ArenaKey<MirParticipantClassTag>;
pub(crate) type MirVehicleProfileKey = ArenaKey<MirVehicleProfileTag>;
pub(crate) type MirCanonicalFrameKey = ArenaKey<MirCanonicalFrameTag>;
pub(crate) type MirAccessRuleKey = ArenaKey<MirAccessRuleTag>;

/// MIR 中保留的模块身份与来源上下文。
pub(crate) struct MirModule {
    /// 模块稳定 authoring namespace。
    pub(crate) authoring_namespace_id: Arc<str>,
    /// 模块声明位置。
    pub(crate) source_span: SourceLocation,
}

/// MIR 平坦连接表中的一条有类型车道图边连接。
pub(crate) struct MirLaneEdgeConnection {
    /// 当前 `MirUnit::lane_edges` 中的目标键。
    pub(crate) target: MirLaneEdgeKey,
    /// 原始引用位置，供后续诊断与源映射使用。
    pub(crate) source_span: SourceLocation,
}

/// 已冻结模块归属和连续连接区间的车道图边 MIR 记录。
pub(crate) struct MirLaneEdge {
    /// 拥有声明的 MIR 模块；不能用原始值当作 HIR 模块键。
    pub(crate) module: MirModuleKey,
    /// 模块内稳定键；不由 MIR 致密下标派生。
    pub(crate) stable_key: Arc<str>,
    /// 从 HIR 原样携带的 Identity v1 有类型稳定标识。
    pub(crate) stable_id: LaneEdgeId,
    /// 交通权威长度，单位为米并保持 `f64`。
    pub(crate) length_meters: f64,
    /// 基础道路限速，单位为米每秒并保持 `f64`。
    pub(crate) speed_limit_meters_per_second: f64,
    /// 此边在 `MirUnit::lane_edge_connections` 中的半开连续区间。
    pub(crate) connections: TableRange<MirLaneEdgeConnection>,
    /// 原始声明位置。
    pub(crate) source_span: SourceLocation,
}

pub(crate) enum MirCorridorElement {
    RoadSection {
        road_section: MirRoadSectionKey,
        source_location: ResolvedSourceLocation,
    },
    FacilityBand {
        facility_band: MirFacilityBandKey,
        source_location: ResolvedSourceLocation,
    },
}

pub(crate) struct MirRoadCorridor {
    pub(crate) module: MirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: RoadCorridorId,
    pub(crate) reference_section: MirRoadSectionKey,
    pub(crate) elements: TableRange<MirCorridorElement>,
    pub(crate) source_span: SourceLocation,
}

pub(crate) struct MirRoadSection {
    pub(crate) module: MirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: RoadSectionId,
    pub(crate) road_corridor: MirRoadCorridorKey,
    pub(crate) kind_id: Arc<str>,
    pub(crate) lanes: TableRange<MirAuthoringLane>,
    pub(crate) source_span: SourceLocation,
}

pub(crate) struct MirAuthoringLaneEdge {
    pub(crate) target: MirLaneEdgeKey,
    pub(crate) source_span: SourceLocation,
}

pub(crate) struct MirAuthoringLane {
    pub(crate) module: MirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: AuthoringLaneId,
    pub(crate) road_section: MirRoadSectionKey,
    pub(crate) edge_chain: TableRange<MirAuthoringLaneEdge>,
    pub(crate) lane_group: Option<MirLaneGroupKey>,
    pub(crate) lane_group_source_location: Option<ResolvedSourceLocation>,
    pub(crate) source_span: SourceLocation,
}

pub(crate) struct MirLaneGroupMember {
    pub(crate) lane: MirAuthoringLaneKey,
}

pub(crate) struct MirLaneGroup {
    pub(crate) module: MirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: LaneGroupId,
    pub(crate) road_section: MirRoadSectionKey,
    pub(crate) members: TableRange<MirLaneGroupMember>,
    pub(crate) source_span: SourceLocation,
}

pub(crate) struct MirFacilityBand {
    pub(crate) module: MirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: FacilityBandId,
    pub(crate) road_corridor: MirRoadCorridorKey,
    pub(crate) kind_id: Arc<str>,
    pub(crate) source_span: SourceLocation,
}

pub(crate) struct MirJunction {
    pub(crate) module: MirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: JunctionId,
    pub(crate) movements: TableRange<MirJunctionMovement>,
    pub(crate) source_span: SourceLocation,
}

pub(crate) struct MirMovement {
    pub(crate) module: MirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: MovementId,
    pub(crate) junction: MirJunctionKey,
    pub(crate) junction_source_location: Option<ResolvedSourceLocation>,
    pub(crate) directed_entry_approach_key: Arc<str>,
    pub(crate) directed_exit_approach_key: Arc<str>,
    pub(crate) maneuver_paths: TableRange<MirMovementManeuverPath>,
    pub(crate) source_span: SourceLocation,
}

pub(crate) struct MirJunctionMovement {
    pub(crate) movement: MirMovementKey,
}

pub(crate) struct MirMovementManeuverPath {
    pub(crate) maneuver_path: MirManeuverPathKey,
}

pub(crate) struct MirManeuverPathEdge {
    pub(crate) target: MirLaneEdgeKey,
    pub(crate) source_span: SourceLocation,
}

pub(crate) struct MirManeuverPath {
    pub(crate) module: MirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: ManeuverPathId,
    pub(crate) movement: MirMovementKey,
    pub(crate) movement_source_location: Option<ResolvedSourceLocation>,
    pub(crate) edges: TableRange<MirManeuverPathEdge>,
    pub(crate) maneuver_gates: TableRange<MirManeuverPathGate>,
    pub(crate) waiting_zones: TableRange<MirManeuverPathWaitingZone>,
    pub(crate) source_span: SourceLocation,
}

pub(crate) struct MirManeuverPathGate {
    pub(crate) maneuver_gate: MirManeuverGateKey,
}

pub(crate) struct MirManeuverPathWaitingZone {
    pub(crate) waiting_zone: MirWaitingZoneKey,
}

pub(crate) struct MirStopLineManeuverGate {
    pub(crate) maneuver_gate: MirManeuverGateKey,
}

pub(crate) struct MirStopLine {
    pub(crate) module: MirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: StopLineId,
    pub(crate) lane_edge: MirLaneEdgeKey,
    pub(crate) maneuver_gates: TableRange<MirStopLineManeuverGate>,
    pub(crate) source_span: SourceLocation,
}

pub(crate) struct MirManeuverGate {
    pub(crate) module: MirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: ManeuverGateId,
    pub(crate) maneuver_path: MirManeuverPathKey,
    pub(crate) maneuver_path_source_location: Option<ResolvedSourceLocation>,
    pub(crate) transition_index: u32,
    pub(crate) stop_line: MirStopLineKey,
    pub(crate) stop_line_source_location: Option<ResolvedSourceLocation>,
    pub(crate) signal_control: MirSignalControl,
    pub(crate) source_span: SourceLocation,
}

#[derive(Clone)]
pub(crate) enum MirSignalControl {
    Group {
        signal_group: MirSignalGroupKey,
        source_location: ResolvedSourceLocation,
    },
    None,
}

pub(crate) struct MirSignalGroup {
    pub(crate) module: MirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: SignalGroupId,
    pub(crate) controller: MirSignalControllerKey,
    pub(crate) maneuver_gates: TableRange<MirSignalGroupManeuverGate>,
    pub(crate) source_span: SourceLocation,
}

pub(crate) struct MirSignalGroupManeuverGate {
    pub(crate) maneuver_gate: MirManeuverGateKey,
}

pub(crate) struct MirSignalControllerGroup {
    pub(crate) signal_group: MirSignalGroupKey,
    pub(crate) source_location: ResolvedSourceLocation,
}

pub(crate) struct MirSignalController {
    pub(crate) module: MirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: SignalControllerId,
    pub(crate) offset_ms: u64,
    pub(crate) cycle_duration_ms: u64,
    pub(crate) signal_groups: TableRange<MirSignalControllerGroup>,
    pub(crate) phases: TableRange<MirSignalPhase>,
    pub(crate) source_span: SourceLocation,
}

pub(crate) struct MirSignalPhase {
    pub(crate) module: MirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: SignalPhaseId,
    pub(crate) controller: MirSignalControllerKey,
    pub(crate) duration_ms: u64,
    pub(crate) states: TableRange<MirSignalPhaseState>,
    pub(crate) controller_relation_source_location: ResolvedSourceLocation,
    pub(crate) source_span: SourceLocation,
}

pub(crate) struct MirSignalPhaseState {
    pub(crate) signal_group: MirSignalGroupKey,
    pub(crate) aspect: SignalAspect,
    pub(crate) source_location: ResolvedSourceLocation,
}

pub(crate) struct MirParkingAreaSpace {
    pub(crate) parking_space: MirParkingSpaceKey,
}

pub(crate) struct MirParkingArea {
    pub(crate) module: MirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: ParkingAreaId,
    pub(crate) parking_spaces: TableRange<MirParkingAreaSpace>,
    pub(crate) source_span: SourceLocation,
}

#[derive(Clone)]
pub(crate) struct MirParkingLaneAnchor {
    pub(crate) lane_edge: MirLaneEdgeKey,
    pub(crate) progress_meters: f64,
    pub(crate) source_location: ResolvedSourceLocation,
}

#[derive(Clone, Copy)]
pub(crate) struct MirParkingSpaceGeometry {
    pub(crate) lateral_offset_meters: f64,
    pub(crate) heading_offset_radians: f64,
    pub(crate) length_meters: f64,
    pub(crate) width_meters: f64,
}

pub(crate) struct MirParkingSpace {
    pub(crate) module: MirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: ParkingSpaceId,
    pub(crate) parking_area: Option<MirParkingAreaKey>,
    pub(crate) parking_area_source_location: Option<ResolvedSourceLocation>,
    pub(crate) entry: MirParkingLaneAnchor,
    pub(crate) exit: MirParkingLaneAnchor,
    pub(crate) geometry: MirParkingSpaceGeometry,
    pub(crate) source_span: SourceLocation,
}

pub(crate) struct MirParticipantClass {
    pub(crate) module: MirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: ParticipantClassId,
    pub(crate) parent: Option<MirParticipantClassKey>,
    pub(crate) parent_source_span: Option<SourceLocation>,
    pub(crate) depth: u32,
    pub(crate) subtree_enter: u32,
    pub(crate) subtree_exit: u32,
    pub(crate) source_span: SourceLocation,
}

pub(crate) struct MirVehicleProfile {
    pub(crate) module: MirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: VehicleProfileId,
    pub(crate) participant_class: MirParticipantClassKey,
    pub(crate) participant_class_source_span: SourceLocation,
    pub(crate) length_meters: f64,
    pub(crate) desired_speed_meters_per_second: f64,
    pub(crate) min_gap_meters: f64,
    pub(crate) time_headway_seconds: f64,
    pub(crate) max_acceleration_meters_per_second_squared: f64,
    pub(crate) comfortable_deceleration_meters_per_second_squared: f64,
    pub(crate) emergency_deceleration_meters_per_second_squared: f64,
    pub(crate) source_span: SourceLocation,
}

pub(crate) struct MirCanonicalFrame {
    pub(crate) module: MirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: CanonicalFrameId,
    pub(crate) lane_edge_geometries: TableRange<MirLaneEdgeGeometry>,
    pub(crate) facility_band_geometries: TableRange<MirFacilityBandGeometry>,
    pub(crate) source_span: SourceLocation,
}

pub(crate) struct MirLaneEdgeGeometry {
    pub(crate) source_module: MirModuleKey,
    pub(crate) canonical_frame: MirCanonicalFrameKey,
    pub(crate) lane_edge: MirLaneEdgeKey,
    pub(crate) points: TableRange<MirCanonicalPoint3F32>,
    pub(crate) segments: TableRange<MirSpatialSegment>,
    pub(crate) source_ranges: TableRange<MirGeometrySourceRange>,
    pub(crate) arc_length_meters: f32,
    pub(crate) source_span: SourceLocation,
}

#[allow(
    dead_code,
    reason = "point ranges and source segment ordinals are consumed by the source-map emitter slice"
)]
pub(crate) struct MirGeometrySourceRange {
    pub(crate) source_module: MirModuleKey,
    pub(crate) points: TableRange<MirCanonicalPoint3F32>,
    pub(crate) source_segment_ordinal: u32,
    pub(crate) source: SourceLocation,
}

pub(crate) struct MirFacilityBandGeometry {
    pub(crate) canonical_frame: MirCanonicalFrameKey,
    pub(crate) facility_band: MirFacilityBandKey,
    pub(crate) points: TableRange<MirCanonicalPoint3F32>,
    pub(crate) source_ranges: TableRange<MirGeometrySourceRange>,
    pub(crate) source_span: SourceLocation,
}

#[derive(Clone, Copy)]
pub(crate) struct MirCanonicalPoint3F32 {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) z: f32,
}

#[derive(Clone, Copy)]
pub(crate) struct MirSpatialSegment {
    pub(crate) length_meters: f32,
    pub(crate) cumulative_end_meters: f32,
    pub(crate) tangent: [f32; 3],
    pub(crate) up: [f32; 3],
}

#[derive(Clone, Copy)]
pub(crate) enum MirAccessTarget {
    LaneEdge(MirLaneEdgeKey),
    LaneGroup(MirLaneGroupKey),
    RoadSection(MirRoadSectionKey),
    ManeuverPath(MirManeuverPathKey),
}

pub(crate) struct MirAccessRegulation {
    pub(crate) jurisdiction: Arc<str>,
    pub(crate) version: Arc<str>,
    pub(crate) source: Option<Arc<str>>,
}

pub(crate) struct MirAccessRuleParticipantClass {
    pub(crate) participant_class: MirParticipantClassKey,
    pub(crate) source_span: SourceLocation,
}

pub(crate) struct MirAccessRule {
    pub(crate) module: MirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: AccessRuleId,
    pub(crate) target: MirAccessTarget,
    pub(crate) target_source_span: SourceLocation,
    pub(crate) effect: AccessEffect,
    pub(crate) participant_classes: TableRange<MirAccessRuleParticipantClass>,
    pub(crate) regulation: Option<MirAccessRegulation>,
    pub(crate) priority: i32,
    pub(crate) source_span: SourceLocation,
}

pub(crate) struct MirWaitingZone {
    pub(crate) module: MirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: WaitingZoneId,
    pub(crate) maneuver_path: MirManeuverPathKey,
    pub(crate) maneuver_path_source_location: Option<ResolvedSourceLocation>,
    pub(crate) entry_gate: MirManeuverGateKey,
    pub(crate) release_gate: MirManeuverGateKey,
    pub(crate) max_occupancy: u32,
    pub(crate) source_span: SourceLocation,
}

pub(crate) struct MirJunctionInternalEdge {
    pub(crate) edge: MirLaneEdgeKey,
    pub(crate) junction: MirJunctionKey,
    /// 选择为规范主要来源的路径所属模块。
    pub(crate) module: MirModuleKey,
    pub(crate) source_span: SourceLocation,
}

pub(crate) struct MirStaticRouteEdge {
    pub(crate) target: MirLaneEdgeKey,
    pub(crate) source_span: SourceLocation,
}

pub(crate) struct MirStaticRouteTransition {
    pub(crate) maneuver_gate: Option<MirManeuverGateKey>,
}

pub(crate) struct MirManeuverOccurrence {
    pub(crate) maneuver_path: MirManeuverPathKey,
    pub(crate) entry_route_edge_index: u32,
    pub(crate) exit_route_edge_index: u32,
    pub(crate) gate_occurrences: TableRange<MirGateOccurrence>,
    pub(crate) waiting_zone_occurrences: TableRange<MirWaitingZoneOccurrence>,
}

pub(crate) struct MirGateOccurrence {
    pub(crate) maneuver_gate: MirManeuverGateKey,
    pub(crate) maneuver_occurrence_index: u32,
    pub(crate) from_route_edge_index: u32,
    pub(crate) next_gate_occurrence_index: Option<u32>,
    pub(crate) next_boundary_route_edge_index: u32,
    pub(crate) waiting_zone_occurrence_index: Option<u32>,
}

pub(crate) struct MirWaitingZoneOccurrence {
    pub(crate) waiting_zone: MirWaitingZoneKey,
    pub(crate) maneuver_occurrence_index: u32,
    pub(crate) entry_gate_occurrence_index: u32,
    pub(crate) release_gate_occurrence_index: u32,
    pub(crate) entry_route_edge_index: u32,
    pub(crate) release_route_edge_index: u32,
}

pub(crate) struct MirStaticRoute {
    pub(crate) module: MirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: StaticRouteId,
    pub(crate) edges: TableRange<MirStaticRouteEdge>,
    pub(crate) transitions: TableRange<MirStaticRouteTransition>,
    pub(crate) maneuver_occurrences: TableRange<MirManeuverOccurrence>,
    pub(crate) gate_occurrences: TableRange<MirGateOccurrence>,
    pub(crate) waiting_zone_occurrences: TableRange<MirWaitingZoneOccurrence>,
    pub(crate) source_span: SourceLocation,
}

/// MIR 阶段成功后一次性冻结的目标布局中立表集合。
///
/// 每个连接区间都落在 `lane_edge_connections` 内，且所有目标键指向本实例的
/// `lane_edges`。`controlled_live_bytes` 只统计 MIR 成功返回后自身拥有的表；
/// `peak_controlled_live_bytes` 另保存 CompilationUnit、HIR 与键映射暂存区的共存峰值。
pub(crate) struct MirUnit {
    pub(crate) geometry_profiles: Option<GeometryCompilationProfiles>,
    pub(crate) modules: Box<[MirModule]>,
    pub(crate) lane_edges: Box<[MirLaneEdge]>,
    pub(crate) lane_edge_connections: Box<[MirLaneEdgeConnection]>,
    pub(crate) road_corridors: Box<[MirRoadCorridor]>,
    pub(crate) corridor_elements: Box<[MirCorridorElement]>,
    pub(crate) road_sections: Box<[MirRoadSection]>,
    pub(crate) authoring_lanes: Box<[MirAuthoringLane]>,
    pub(crate) authoring_lane_edges: Box<[MirAuthoringLaneEdge]>,
    pub(crate) lane_groups: Box<[MirLaneGroup]>,
    pub(crate) lane_group_members: Box<[MirLaneGroupMember]>,
    pub(crate) facility_bands: Box<[MirFacilityBand]>,
    pub(crate) junctions: Box<[MirJunction]>,
    pub(crate) movements: Box<[MirMovement]>,
    pub(crate) junction_movements: Box<[MirJunctionMovement]>,
    pub(crate) maneuver_paths: Box<[MirManeuverPath]>,
    pub(crate) movement_maneuver_paths: Box<[MirMovementManeuverPath]>,
    pub(crate) maneuver_path_edges: Box<[MirManeuverPathEdge]>,
    pub(crate) junction_internal_edges: Box<[MirJunctionInternalEdge]>,
    pub(crate) stop_lines: Box<[MirStopLine]>,
    pub(crate) maneuver_gates: Box<[MirManeuverGate]>,
    pub(crate) waiting_zones: Box<[MirWaitingZone]>,
    pub(crate) maneuver_path_gates: Box<[MirManeuverPathGate]>,
    pub(crate) maneuver_path_waiting_zones: Box<[MirManeuverPathWaitingZone]>,
    pub(crate) stop_line_maneuver_gates: Box<[MirStopLineManeuverGate]>,
    pub(crate) signal_groups: Box<[MirSignalGroup]>,
    pub(crate) signal_controllers: Box<[MirSignalController]>,
    pub(crate) signal_controller_groups: Box<[MirSignalControllerGroup]>,
    pub(crate) signal_phases: Box<[MirSignalPhase]>,
    pub(crate) signal_phase_states: Box<[MirSignalPhaseState]>,
    pub(crate) signal_group_maneuver_gates: Box<[MirSignalGroupManeuverGate]>,
    pub(crate) parking_areas: Box<[MirParkingArea]>,
    pub(crate) parking_spaces: Box<[MirParkingSpace]>,
    pub(crate) parking_area_spaces: Box<[MirParkingAreaSpace]>,
    pub(crate) participant_classes: Box<[MirParticipantClass]>,
    pub(crate) vehicle_profiles: Box<[MirVehicleProfile]>,
    pub(crate) canonical_frames: Box<[MirCanonicalFrame]>,
    pub(crate) lane_edge_geometries: Box<[MirLaneEdgeGeometry]>,
    pub(crate) geometry_source_ranges: Box<[MirGeometrySourceRange]>,
    pub(crate) facility_band_geometries: Box<[MirFacilityBandGeometry]>,
    pub(crate) canonical_points: Box<[MirCanonicalPoint3F32]>,
    pub(crate) spatial_segments: Box<[MirSpatialSegment]>,
    pub(crate) access_rules: Box<[MirAccessRule]>,
    pub(crate) access_rule_participant_classes: Box<[MirAccessRuleParticipantClass]>,
    pub(crate) static_routes: Box<[MirStaticRoute]>,
    pub(crate) static_route_edges: Box<[MirStaticRouteEdge]>,
    pub(crate) static_route_transitions: Box<[MirStaticRouteTransition]>,
    pub(crate) maneuver_occurrences: Box<[MirManeuverOccurrence]>,
    pub(crate) gate_occurrences: Box<[MirGateOccurrence]>,
    pub(crate) waiting_zone_occurrences: Box<[MirWaitingZoneOccurrence]>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) mir_record_count: u64,
    pub(crate) controlled_live_bytes: u64,
    pub(crate) peak_controlled_live_bytes: u64,
}

/// 将已解析 HIR 降级为连续 MIR 表，并显式重映射全部阶段键。
///
/// # Errors
///
/// 当 MIR 记录、阶段暂存区、编译器控制存续字节或 `u32` 表边界超过所选配置档时，
/// 返回资源诊断且不返回部分 MIR。输入 HIR 只能由 `build_hir` 成功产生，因此本函数不
/// 重复执行文本符号解析。
pub(crate) fn lower_to_mir(
    unit: &CompilationUnit,
    hir: &HirUnit,
) -> Result<MirUnit, DiagnosticBundle> {
    // MIR record 指标计全部稳定实体与关系；模块元数据另计入分配和 live-byte 预检。
    // 在任何阶段表分配前先验证记录、暂存映射和 HIR/MIR 同时存续的峰值。
    let module_count = u64::try_from(hir.modules.len()).unwrap_or(u64::MAX);
    let lane_edge_count = u64::try_from(hir.lane_edges.len()).unwrap_or(u64::MAX);
    let connection_count = u64::try_from(hir.lane_edge_references.len()).unwrap_or(u64::MAX);
    let cross_record_count = [
        hir.road_corridors.len(),
        hir.corridor_elements.len(),
        hir.road_sections.len(),
        hir.authoring_lanes.len(),
        hir.authoring_lane_edges.len(),
        hir.lane_groups.len(),
        hir.lane_group_members.len(),
        hir.facility_bands.len(),
    ]
    .into_iter()
    .fold(0_u64, |total, count| {
        total.saturating_add(u64::try_from(count).unwrap_or(u64::MAX))
    });
    let junction_record_count = [
        hir.junctions.len(),
        hir.movements.len(),
        hir.junction_movements.len(),
        hir.maneuver_paths.len(),
        hir.movement_maneuver_paths.len(),
        hir.maneuver_path_edges.len(),
        hir.junction_internal_edges.len(),
    ]
    .into_iter()
    .fold(0_u64, |total, count| {
        total.saturating_add(u64::try_from(count).unwrap_or(u64::MAX))
    });
    let control_record_count = [
        hir.stop_lines.len(),
        hir.maneuver_gates.len(),
        hir.waiting_zones.len(),
        hir.maneuver_path_gates.len(),
        hir.maneuver_path_waiting_zones.len(),
        hir.stop_line_maneuver_gates.len(),
    ]
    .into_iter()
    .fold(0_u64, |total, count| {
        total.saturating_add(u64::try_from(count).unwrap_or(u64::MAX))
    });
    let route_record_count = [
        hir.static_routes.len(),
        hir.static_route_edges.len(),
        hir.static_route_transitions.len(),
        hir.maneuver_occurrences.len(),
        hir.gate_occurrences.len(),
        hir.waiting_zone_occurrences.len(),
    ]
    .into_iter()
    .fold(0_u64, |total, count| {
        total.saturating_add(u64::try_from(count).unwrap_or(u64::MAX))
    });
    let signal_record_count = [
        hir.signal_groups.len(),
        hir.signal_controllers.len(),
        hir.signal_controller_groups.len(),
        hir.signal_phases.len(),
        hir.signal_phase_states.len(),
        hir.signal_group_maneuver_gates.len(),
    ]
    .into_iter()
    .fold(0_u64, |total, count| {
        total.saturating_add(u64::try_from(count).unwrap_or(u64::MAX))
    });
    let parking_record_count = [
        hir.parking_areas.len(),
        hir.parking_spaces.len(),
        hir.parking_area_spaces.len(),
    ]
    .into_iter()
    .fold(0_u64, |total, count| {
        total.saturating_add(u64::try_from(count).unwrap_or(u64::MAX))
    });
    let access_record_count = [
        hir.participant_classes.len(),
        hir.vehicle_profiles.len(),
        hir.access_rules.len(),
        hir.access_rule_participant_classes.len(),
    ]
    .into_iter()
    .fold(0_u64, |total, count| {
        total.saturating_add(u64::try_from(count).unwrap_or(u64::MAX))
    });
    let mir_record_count = lane_edge_count
        .saturating_add(connection_count)
        .saturating_add(cross_record_count)
        .saturating_add(junction_record_count)
        .saturating_add(control_record_count)
        .saturating_add(signal_record_count)
        .saturating_add(parking_record_count)
        .saturating_add(u64::try_from(hir.canonical_frames.len()).unwrap_or(u64::MAX))
        .saturating_add(u64::try_from(hir.lane_edge_geometries.len()).unwrap_or(u64::MAX))
        .saturating_add(u64::try_from(hir.geometry_source_ranges.len()).unwrap_or(u64::MAX))
        .saturating_add(u64::try_from(hir.facility_band_geometries.len()).unwrap_or(u64::MAX))
        .saturating_add(u64::try_from(hir.canonical_points.len()).unwrap_or(u64::MAX))
        .saturating_add(u64::try_from(hir.spatial_segments.len()).unwrap_or(u64::MAX))
        .saturating_add(access_record_count)
        .saturating_add(route_record_count);
    let stage_scratch_bytes = requested_bytes::<MirModuleKey>(module_count)
        .saturating_add(requested_bytes::<MirLaneEdgeKey>(lane_edge_count))
        .saturating_add(requested_bytes::<u32>(
            cross_record_count.saturating_add(
                u64::try_from(hir.junctions.len())
                    .unwrap_or(u64::MAX)
                    .saturating_add(u64::try_from(hir.movements.len()).unwrap_or(u64::MAX))
                    .saturating_add(u64::try_from(hir.maneuver_paths.len()).unwrap_or(u64::MAX)),
            ),
        ))
        .saturating_add(requested_bytes::<u32>(
            u64::try_from(hir.stop_lines.len())
                .unwrap_or(u64::MAX)
                .saturating_add(u64::try_from(hir.maneuver_gates.len()).unwrap_or(u64::MAX))
                .saturating_add(u64::try_from(hir.waiting_zones.len()).unwrap_or(u64::MAX)),
        ))
        .saturating_add(requested_bytes::<u32>(
            u64::try_from(hir.static_routes.len()).unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<u32>(
            u64::try_from(hir.signal_groups.len())
                .unwrap_or(u64::MAX)
                .saturating_add(u64::try_from(hir.signal_controllers.len()).unwrap_or(u64::MAX)),
        ))
        .saturating_add(requested_bytes::<u32>(
            u64::try_from(hir.parking_areas.len())
                .unwrap_or(u64::MAX)
                .saturating_add(u64::try_from(hir.parking_spaces.len()).unwrap_or(u64::MAX)),
        ))
        .saturating_add(requested_bytes::<u32>(
            u64::try_from(hir.participant_classes.len())
                .unwrap_or(u64::MAX)
                .saturating_add(u64::try_from(hir.vehicle_profiles.len()).unwrap_or(u64::MAX))
                .saturating_add(u64::try_from(hir.access_rules.len()).unwrap_or(u64::MAX)),
        ))
        .saturating_add(requested_bytes::<u32>(
            u64::try_from(hir.canonical_frames.len()).unwrap_or(u64::MAX),
        ));
    let mir_owned_bytes = requested_bytes::<MirModule>(module_count)
        .saturating_add(requested_bytes::<MirLaneEdge>(lane_edge_count))
        .saturating_add(requested_bytes::<MirLaneEdgeConnection>(connection_count))
        .saturating_add(requested_bytes::<MirRoadCorridor>(
            hir.road_corridors.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirCorridorElement>(
            hir.corridor_elements.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirRoadSection>(
            hir.road_sections.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirAuthoringLane>(
            hir.authoring_lanes.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirAuthoringLaneEdge>(
            hir.authoring_lane_edges
                .len()
                .try_into()
                .unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirLaneGroup>(
            hir.lane_groups.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirLaneGroupMember>(
            hir.lane_group_members.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirFacilityBand>(
            hir.facility_bands.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirJunction>(
            hir.junctions.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirMovement>(
            hir.movements.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirJunctionMovement>(
            hir.junction_movements.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirManeuverPath>(
            hir.maneuver_paths.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirMovementManeuverPath>(
            hir.movement_maneuver_paths
                .len()
                .try_into()
                .unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirManeuverPathEdge>(
            hir.maneuver_path_edges.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirJunctionInternalEdge>(
            hir.junction_internal_edges
                .len()
                .try_into()
                .unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirStopLine>(
            hir.stop_lines.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirManeuverGate>(
            hir.maneuver_gates.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirWaitingZone>(
            hir.waiting_zones.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirManeuverPathGate>(
            hir.maneuver_path_gates.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirManeuverPathWaitingZone>(
            hir.maneuver_path_waiting_zones
                .len()
                .try_into()
                .unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirStopLineManeuverGate>(
            hir.stop_line_maneuver_gates
                .len()
                .try_into()
                .unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirSignalGroup>(
            hir.signal_groups.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirSignalController>(
            hir.signal_controllers.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirSignalControllerGroup>(
            hir.signal_controller_groups
                .len()
                .try_into()
                .unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirSignalPhase>(
            hir.signal_phases.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirSignalPhaseState>(
            hir.signal_phase_states.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirSignalGroupManeuverGate>(
            hir.signal_group_maneuver_gates
                .len()
                .try_into()
                .unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirParkingArea>(
            hir.parking_areas.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirParkingSpace>(
            hir.parking_spaces.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirParkingAreaSpace>(
            hir.parking_area_spaces.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirCanonicalFrame>(
            hir.canonical_frames.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirLaneEdgeGeometry>(
            hir.lane_edge_geometries
                .len()
                .try_into()
                .unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirGeometrySourceRange>(
            hir.geometry_source_ranges
                .len()
                .try_into()
                .unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirFacilityBandGeometry>(
            hir.facility_band_geometries
                .len()
                .try_into()
                .unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirCanonicalPoint3F32>(
            hir.canonical_points.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirSpatialSegment>(
            hir.spatial_segments.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirParticipantClass>(
            hir.participant_classes.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirVehicleProfile>(
            hir.vehicle_profiles.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirAccessRule>(
            hir.access_rules.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirAccessRuleParticipantClass>(
            hir.access_rule_participant_classes
                .len()
                .try_into()
                .unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirStaticRoute>(
            hir.static_routes.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirStaticRouteEdge>(
            hir.static_route_edges.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirStaticRouteTransition>(
            hir.static_route_transitions
                .len()
                .try_into()
                .unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirManeuverOccurrence>(
            hir.maneuver_occurrences
                .len()
                .try_into()
                .unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirGateOccurrence>(
            hir.gate_occurrences.len().try_into().unwrap_or(u64::MAX),
        ))
        .saturating_add(requested_bytes::<MirWaitingZoneOccurrence>(
            hir.waiting_zone_occurrences
                .len()
                .try_into()
                .unwrap_or(u64::MAX),
        ));
    let controlled_live_bytes = unit
        .controlled_live_bytes
        .saturating_add(hir.controlled_live_bytes)
        .saturating_add(mir_owned_bytes)
        .saturating_add(stage_scratch_bytes);
    let primary_span = hir.modules.first().map(|module| module.source_span.clone());
    let stable_key = hir
        .modules
        .first()
        .map(|module| module.authoring_namespace_id.as_ref().into());
    let mut diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
    for (dimension, observed) in [
        (CompileLimitDimension::MirRecordCount, mir_record_count),
        (
            CompileLimitDimension::StageScratchBytes,
            stage_scratch_bytes,
        ),
        (
            CompileLimitDimension::CompilerControlledLiveBytes,
            controlled_live_bytes,
        ),
    ] {
        if observed > unit.limits.value(dimension) {
            diagnostics.push(Diagnostic::compile_limit_exceeded_at(
                dimension,
                unit.limits.value(dimension),
                observed,
                primary_span.clone(),
                stable_key.clone(),
            ));
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    // 不依赖 HIR/MIR raw key 数值碰巧一致：每次插入都记录显式 stage-to-stage 映射。
    let mut modules = TypedArena::<MirModuleTag, MirModule>::with_capacity(hir.modules.len());
    let mut hir_module_to_mir = Vec::with_capacity(hir.modules.len());
    for module in &hir.modules {
        let mir_key = modules
            .push(MirModule {
                authoring_namespace_id: Arc::clone(&module.authoring_namespace_id),
                source_span: module.source_span.clone(),
            })
            .map_err(|overflow| arena_overflow(overflow, &unit.limits, primary_span.clone()))?;
        hir_module_to_mir.push(mir_key);
    }

    let edge_capacity = usize::try_from(lane_edge_count)
        .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, primary_span.clone()))?;
    let connection_capacity = usize::try_from(connection_count)
        .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, primary_span.clone()))?;
    let mut lane_edges = TypedArena::<MirLaneEdgeTag, MirLaneEdge>::with_capacity(edge_capacity);
    let mut hir_to_mir = Vec::with_capacity(edge_capacity);
    for edge in &hir.lane_edges {
        let module = hir_module_to_mir[edge.module.index()];
        let mir_key = lane_edges
            .push(MirLaneEdge {
                module,
                stable_key: Arc::clone(&edge.stable_key),
                stable_id: edge.stable_id,
                length_meters: edge.length_meters,
                speed_limit_meters_per_second: edge.speed_limit_meters_per_second,
                connections: TableRange::empty(),
                source_span: edge.source_span.clone(),
            })
            .map_err(|overflow| {
                arena_overflow(overflow, &unit.limits, Some(edge.source_span.clone()))
            })?;
        hir_to_mir.push(mir_key);
    }

    // 按 HIR 的规范边顺序追加连接，并以 TableRange 记录每条边的连续片段；这样后续遍历
    // 不需要哈希查找或每边独立分配。
    let mut connections = Vec::with_capacity(connection_capacity);
    for (hir_index, edge) in hir.lane_edges.iter().enumerate() {
        let mir_key = hir_to_mir[hir_index];
        let start = connections.len();
        for reference in &hir.lane_edge_references[edge.successors.as_usize_range()] {
            connections.push(MirLaneEdgeConnection {
                target: mir_key_for_hir(reference.target, &hir_to_mir),
                source_span: reference.source_span.clone(),
            });
        }
        lane_edges.get_mut(mir_key).connections =
            TableRange::try_from_usize(start, connections.len().saturating_sub(start)).map_err(
                |overflow| arena_overflow(overflow, &unit.limits, Some(edge.source_span.clone())),
            )?;
    }

    let corridor_mapping = dense_mapping::<MirRoadCorridorTag>(hir.road_corridors.len())?;
    let section_mapping = dense_mapping::<MirRoadSectionTag>(hir.road_sections.len())?;
    let lane_mapping = dense_mapping::<MirAuthoringLaneTag>(hir.authoring_lanes.len())?;
    let group_mapping = dense_mapping::<MirLaneGroupTag>(hir.lane_groups.len())?;
    let band_mapping = dense_mapping::<MirFacilityBandTag>(hir.facility_bands.len())?;

    let mut road_corridors = Vec::with_capacity(hir.road_corridors.len());
    for corridor in &hir.road_corridors {
        road_corridors.push(MirRoadCorridor {
            module: hir_module_to_mir[corridor.module.index()],
            stable_key: Arc::clone(&corridor.stable_key),
            stable_id: corridor.stable_id,
            reference_section: section_mapping[corridor.reference_section.index()],
            elements: remap_range(corridor.elements, &unit.limits, &corridor.source_span)?,
            source_span: corridor.source_span.clone(),
        });
    }
    let corridor_elements: Vec<MirCorridorElement> = hir
        .corridor_elements
        .iter()
        .map(|element| match element {
            HirCorridorElement::RoadSection {
                road_section,
                source_location,
            } => MirCorridorElement::RoadSection {
                road_section: section_mapping[road_section.index()],
                source_location: source_location.clone(),
            },
            HirCorridorElement::FacilityBand {
                facility_band,
                source_location,
            } => MirCorridorElement::FacilityBand {
                facility_band: band_mapping[facility_band.index()],
                source_location: source_location.clone(),
            },
        })
        .collect();
    let road_sections = hir
        .road_sections
        .iter()
        .map(|section| {
            Ok(MirRoadSection {
                module: hir_module_to_mir[section.module.index()],
                stable_key: Arc::clone(&section.stable_key),
                stable_id: section.stable_id,
                road_corridor: corridor_mapping[section.road_corridor.index()],
                kind_id: Arc::clone(&section.kind_id),
                lanes: remap_range(section.lanes, &unit.limits, &section.source_span)?,
                source_span: section.source_span.clone(),
            })
        })
        .collect::<Result<Vec<_>, DiagnosticBundle>>()?;
    let authoring_lanes = hir
        .authoring_lanes
        .iter()
        .map(|lane| {
            Ok(MirAuthoringLane {
                module: hir_module_to_mir[lane.module.index()],
                stable_key: Arc::clone(&lane.stable_key),
                stable_id: lane.stable_id,
                road_section: section_mapping[lane.road_section.index()],
                edge_chain: remap_range(lane.edge_chain, &unit.limits, &lane.source_span)?,
                lane_group: lane.lane_group.map(|key| group_mapping[key.index()]),
                lane_group_source_location: lane.lane_group_source_location.clone(),
                source_span: lane.source_span.clone(),
            })
        })
        .collect::<Result<Vec<_>, DiagnosticBundle>>()?;
    let authoring_lane_edges: Vec<MirAuthoringLaneEdge> = hir
        .authoring_lane_edges
        .iter()
        .map(|edge| MirAuthoringLaneEdge {
            target: hir_to_mir[edge.target.index()],
            source_span: edge.source_span.clone(),
        })
        .collect();
    let lane_groups = hir
        .lane_groups
        .iter()
        .map(|group| {
            Ok(MirLaneGroup {
                module: hir_module_to_mir[group.module.index()],
                stable_key: Arc::clone(&group.stable_key),
                stable_id: group.stable_id,
                road_section: section_mapping[group.road_section.index()],
                members: remap_range(group.members, &unit.limits, &group.source_span)?,
                source_span: group.source_span.clone(),
            })
        })
        .collect::<Result<Vec<_>, DiagnosticBundle>>()?;
    let lane_group_members: Vec<MirLaneGroupMember> = hir
        .lane_group_members
        .iter()
        .map(|member| MirLaneGroupMember {
            lane: lane_mapping[member.lane.index()],
        })
        .collect();
    let facility_bands: Vec<MirFacilityBand> = hir
        .facility_bands
        .iter()
        .map(|band| MirFacilityBand {
            module: hir_module_to_mir[band.module.index()],
            stable_key: Arc::clone(&band.stable_key),
            stable_id: band.stable_id,
            road_corridor: corridor_mapping[band.road_corridor.index()],
            kind_id: Arc::clone(&band.kind_id),
            source_span: band.source_span.clone(),
        })
        .collect();

    let junction_mapping = dense_mapping::<MirJunctionTag>(hir.junctions.len())?;
    let movement_mapping = dense_mapping::<MirMovementTag>(hir.movements.len())?;
    let maneuver_path_mapping = dense_mapping::<MirManeuverPathTag>(hir.maneuver_paths.len())?;
    let junctions = hir
        .junctions
        .iter()
        .map(|junction| {
            Ok(MirJunction {
                module: hir_module_to_mir[junction.module.index()],
                stable_key: Arc::clone(&junction.stable_key),
                stable_id: junction.stable_id,
                movements: remap_range(junction.movements, &unit.limits, &junction.source_span)?,
                source_span: junction.source_span.clone(),
            })
        })
        .collect::<Result<Vec<_>, DiagnosticBundle>>()?;
    let movements = hir
        .movements
        .iter()
        .map(|movement| {
            Ok(MirMovement {
                module: hir_module_to_mir[movement.module.index()],
                stable_key: Arc::clone(&movement.stable_key),
                stable_id: movement.stable_id,
                junction: junction_mapping[movement.junction.index()],
                junction_source_location: movement.junction_source_location.clone(),
                directed_entry_approach_key: Arc::clone(&movement.directed_entry_approach_key),
                directed_exit_approach_key: Arc::clone(&movement.directed_exit_approach_key),
                maneuver_paths: remap_range(
                    movement.maneuver_paths,
                    &unit.limits,
                    &movement.source_span,
                )?,
                source_span: movement.source_span.clone(),
            })
        })
        .collect::<Result<Vec<_>, DiagnosticBundle>>()?;
    let junction_movements = hir
        .junction_movements
        .iter()
        .map(|member| MirJunctionMovement {
            movement: movement_mapping[member.movement.index()],
        })
        .collect::<Vec<_>>();
    let maneuver_paths = hir
        .maneuver_paths
        .iter()
        .map(|path| {
            Ok(MirManeuverPath {
                module: hir_module_to_mir[path.module.index()],
                stable_key: Arc::clone(&path.stable_key),
                stable_id: path.stable_id,
                movement: movement_mapping[path.movement.index()],
                movement_source_location: path.movement_source_location.clone(),
                edges: remap_range(path.edges, &unit.limits, &path.source_span)?,
                maneuver_gates: remap_range(path.maneuver_gates, &unit.limits, &path.source_span)?,
                waiting_zones: remap_range(path.waiting_zones, &unit.limits, &path.source_span)?,
                source_span: path.source_span.clone(),
            })
        })
        .collect::<Result<Vec<_>, DiagnosticBundle>>()?;
    let movement_maneuver_paths = hir
        .movement_maneuver_paths
        .iter()
        .map(|member| MirMovementManeuverPath {
            maneuver_path: maneuver_path_mapping[member.maneuver_path.index()],
        })
        .collect::<Vec<_>>();
    let maneuver_path_edges = hir
        .maneuver_path_edges
        .iter()
        .map(|edge| MirManeuverPathEdge {
            target: hir_to_mir[edge.target.index()],
            source_span: edge.source_span.clone(),
        })
        .collect::<Vec<_>>();
    let junction_internal_edges = hir
        .junction_internal_edges
        .iter()
        .map(|relation| MirJunctionInternalEdge {
            edge: hir_to_mir[relation.edge.index()],
            junction: junction_mapping[relation.junction.index()],
            module: hir_module_to_mir[hir.maneuver_paths[relation.source_path.index()]
                .module
                .index()],
            source_span: relation.source_span.clone(),
        })
        .collect::<Vec<_>>();

    let signal_group_mapping = dense_mapping::<MirSignalGroupTag>(hir.signal_groups.len())?;
    let signal_controller_mapping =
        dense_mapping::<MirSignalControllerTag>(hir.signal_controllers.len())?;
    let stop_line_mapping = dense_mapping::<MirStopLineTag>(hir.stop_lines.len())?;
    let maneuver_gate_mapping = dense_mapping::<MirManeuverGateTag>(hir.maneuver_gates.len())?;
    let waiting_zone_mapping = dense_mapping::<MirWaitingZoneTag>(hir.waiting_zones.len())?;
    let stop_lines = hir
        .stop_lines
        .iter()
        .map(|stop_line| {
            Ok(MirStopLine {
                module: hir_module_to_mir[stop_line.module.index()],
                stable_key: Arc::clone(&stop_line.stable_key),
                stable_id: stop_line.stable_id,
                lane_edge: hir_to_mir[stop_line.lane_edge.index()],
                maneuver_gates: remap_range(
                    stop_line.maneuver_gates,
                    &unit.limits,
                    &stop_line.source_span,
                )?,
                source_span: stop_line.source_span.clone(),
            })
        })
        .collect::<Result<Vec<_>, DiagnosticBundle>>()?;
    let maneuver_gates = hir
        .maneuver_gates
        .iter()
        .map(|gate| MirManeuverGate {
            module: hir_module_to_mir[gate.module.index()],
            stable_key: Arc::clone(&gate.stable_key),
            stable_id: gate.stable_id,
            maneuver_path: maneuver_path_mapping[gate.maneuver_path.index()],
            maneuver_path_source_location: gate.maneuver_path_source_location.clone(),
            transition_index: gate.transition_index,
            stop_line: stop_line_mapping[gate.stop_line.index()],
            stop_line_source_location: gate.stop_line_source_location.clone(),
            signal_control: match &gate.signal_control {
                HirSignalControl::Group {
                    signal_group,
                    source_location,
                } => MirSignalControl::Group {
                    signal_group: signal_group_mapping[signal_group.index()],
                    source_location: source_location.clone(),
                },
                HirSignalControl::None => MirSignalControl::None,
            },
            source_span: gate.source_span.clone(),
        })
        .collect::<Vec<_>>();
    let waiting_zones = hir
        .waiting_zones
        .iter()
        .map(|waiting| MirWaitingZone {
            module: hir_module_to_mir[waiting.module.index()],
            stable_key: Arc::clone(&waiting.stable_key),
            stable_id: waiting.stable_id,
            maneuver_path: maneuver_path_mapping[waiting.maneuver_path.index()],
            maneuver_path_source_location: waiting.maneuver_path_source_location.clone(),
            entry_gate: maneuver_gate_mapping[waiting.entry_gate.index()],
            release_gate: maneuver_gate_mapping[waiting.release_gate.index()],
            max_occupancy: waiting.max_occupancy,
            source_span: waiting.source_span.clone(),
        })
        .collect::<Vec<_>>();
    let maneuver_path_gates = hir
        .maneuver_path_gates
        .iter()
        .map(|member| MirManeuverPathGate {
            maneuver_gate: maneuver_gate_mapping[member.maneuver_gate.index()],
        })
        .collect::<Vec<_>>();
    let maneuver_path_waiting_zones = hir
        .maneuver_path_waiting_zones
        .iter()
        .map(|member| MirManeuverPathWaitingZone {
            waiting_zone: waiting_zone_mapping[member.waiting_zone.index()],
        })
        .collect::<Vec<_>>();
    let stop_line_maneuver_gates = hir
        .stop_line_maneuver_gates
        .iter()
        .map(|member| MirStopLineManeuverGate {
            maneuver_gate: maneuver_gate_mapping[member.maneuver_gate.index()],
        })
        .collect::<Vec<_>>();

    let signal_groups = hir
        .signal_groups
        .iter()
        .map(|group| {
            Ok(MirSignalGroup {
                module: hir_module_to_mir[group.module.index()],
                stable_key: Arc::clone(&group.stable_key),
                stable_id: group.stable_id,
                controller: signal_controller_mapping[group.controller.index()],
                maneuver_gates: remap_range(
                    group.maneuver_gates,
                    &unit.limits,
                    &group.source_span,
                )?,
                source_span: group.source_span.clone(),
            })
        })
        .collect::<Result<Vec<_>, DiagnosticBundle>>()?;
    let signal_controllers = hir
        .signal_controllers
        .iter()
        .map(|controller| {
            Ok(MirSignalController {
                module: hir_module_to_mir[controller.module.index()],
                stable_key: Arc::clone(&controller.stable_key),
                stable_id: controller.stable_id,
                offset_ms: controller.offset_ms,
                cycle_duration_ms: controller.cycle_duration_ms,
                signal_groups: remap_range(
                    controller.signal_groups,
                    &unit.limits,
                    &controller.source_span,
                )?,
                phases: remap_range(controller.phases, &unit.limits, &controller.source_span)?,
                source_span: controller.source_span.clone(),
            })
        })
        .collect::<Result<Vec<_>, DiagnosticBundle>>()?;
    let signal_controller_groups = hir
        .signal_controller_groups
        .iter()
        .map(|member| MirSignalControllerGroup {
            signal_group: signal_group_mapping[member.signal_group.index()],
            source_location: member.source_location.clone(),
        })
        .collect::<Vec<_>>();
    let signal_phases = hir
        .signal_phases
        .iter()
        .map(|phase| {
            Ok(MirSignalPhase {
                module: hir_module_to_mir[phase.module.index()],
                stable_key: Arc::clone(&phase.stable_key),
                stable_id: phase.stable_id,
                controller: signal_controller_mapping[phase.controller.index()],
                duration_ms: phase.duration_ms,
                states: remap_range(phase.states, &unit.limits, &phase.source_span)?,
                controller_relation_source_location: phase
                    .controller_relation_source_location
                    .clone(),
                source_span: phase.source_span.clone(),
            })
        })
        .collect::<Result<Vec<_>, DiagnosticBundle>>()?;
    let signal_phase_states = hir
        .signal_phase_states
        .iter()
        .map(|state| MirSignalPhaseState {
            signal_group: signal_group_mapping[state.signal_group.index()],
            aspect: state.aspect,
            source_location: state.source_location.clone(),
        })
        .collect::<Vec<_>>();
    let signal_group_maneuver_gates = hir
        .signal_group_maneuver_gates
        .iter()
        .map(|member| MirSignalGroupManeuverGate {
            maneuver_gate: maneuver_gate_mapping[member.maneuver_gate.index()],
        })
        .collect::<Vec<_>>();

    let parking_area_mapping = dense_mapping::<MirParkingAreaTag>(hir.parking_areas.len())?;
    let parking_space_mapping = dense_mapping::<MirParkingSpaceTag>(hir.parking_spaces.len())?;
    let parking_areas = hir
        .parking_areas
        .iter()
        .map(|area| {
            Ok(MirParkingArea {
                module: hir_module_to_mir[area.module.index()],
                stable_key: Arc::clone(&area.stable_key),
                stable_id: area.stable_id,
                parking_spaces: remap_range(area.parking_spaces, &unit.limits, &area.source_span)?,
                source_span: area.source_span.clone(),
            })
        })
        .collect::<Result<Vec<_>, DiagnosticBundle>>()?;
    let parking_spaces = hir
        .parking_spaces
        .iter()
        .map(|space| MirParkingSpace {
            module: hir_module_to_mir[space.module.index()],
            stable_key: Arc::clone(&space.stable_key),
            stable_id: space.stable_id,
            parking_area: space
                .parking_area
                .map(|area| parking_area_mapping[area.index()]),
            parking_area_source_location: space.parking_area_source_location.clone(),
            entry: MirParkingLaneAnchor {
                lane_edge: hir_to_mir[space.entry.lane_edge.index()],
                progress_meters: space.entry.progress_meters,
                source_location: space.entry.source_location.clone(),
            },
            exit: MirParkingLaneAnchor {
                lane_edge: hir_to_mir[space.exit.lane_edge.index()],
                progress_meters: space.exit.progress_meters,
                source_location: space.exit.source_location.clone(),
            },
            geometry: MirParkingSpaceGeometry {
                lateral_offset_meters: space.geometry.lateral_offset_meters,
                heading_offset_radians: space.geometry.heading_offset_radians,
                length_meters: space.geometry.length_meters,
                width_meters: space.geometry.width_meters,
            },
            source_span: space.source_span.clone(),
        })
        .collect::<Vec<_>>();
    let parking_area_spaces = hir
        .parking_area_spaces
        .iter()
        .map(|member| MirParkingAreaSpace {
            parking_space: parking_space_mapping[member.parking_space.index()],
        })
        .collect::<Vec<_>>();

    let canonical_frame_mapping =
        dense_mapping::<MirCanonicalFrameTag>(hir.canonical_frames.len())?;
    let canonical_frames = hir
        .canonical_frames
        .iter()
        .map(|frame| {
            Ok(MirCanonicalFrame {
                module: hir_module_to_mir[frame.module.index()],
                stable_key: Arc::clone(&frame.stable_key),
                stable_id: frame.stable_id,
                lane_edge_geometries: remap_range(
                    frame.lane_edge_geometries,
                    &unit.limits,
                    &frame.source_span,
                )?,
                facility_band_geometries: remap_range(
                    frame.facility_band_geometries,
                    &unit.limits,
                    &frame.source_span,
                )?,
                source_span: frame.source_span.clone(),
            })
        })
        .collect::<Result<Vec<_>, DiagnosticBundle>>()?;
    let facility_band_geometries = hir
        .facility_band_geometries
        .iter()
        .map(|geometry| {
            Ok(MirFacilityBandGeometry {
                canonical_frame: canonical_frame_mapping[geometry.canonical_frame.index()],
                facility_band: band_mapping[geometry.facility_band.index()],
                points: remap_range(geometry.points, &unit.limits, &geometry.source_span)?,
                source_ranges: remap_range(
                    geometry.source_ranges,
                    &unit.limits,
                    &geometry.source_span,
                )?,
                source_span: geometry.source_span.clone(),
            })
        })
        .collect::<Result<Vec<_>, DiagnosticBundle>>()?;
    let lane_edge_geometries = hir
        .lane_edge_geometries
        .iter()
        .map(|geometry| {
            Ok(MirLaneEdgeGeometry {
                source_module: hir_module_to_mir[geometry.source_module.index()],
                canonical_frame: canonical_frame_mapping[geometry.canonical_frame.index()],
                lane_edge: hir_to_mir[geometry.lane_edge.index()],
                points: remap_range(geometry.points, &unit.limits, &geometry.source_span)?,
                segments: remap_range(geometry.segments, &unit.limits, &geometry.source_span)?,
                source_ranges: remap_range(
                    geometry.source_ranges,
                    &unit.limits,
                    &geometry.source_span,
                )?,
                arc_length_meters: geometry.arc_length_meters,
                source_span: geometry.source_span.clone(),
            })
        })
        .collect::<Result<Vec<_>, DiagnosticBundle>>()?;
    let mut geometry_source_ranges = Vec::with_capacity(hir.geometry_source_ranges.len());
    for range in &hir.geometry_source_ranges {
        geometry_source_ranges.push(MirGeometrySourceRange {
            source_module: hir_module_to_mir[range.source_module.index()],
            points: remap_range(range.points, &unit.limits, &range.source)?,
            source_segment_ordinal: range.source_segment_ordinal,
            source: range.source.clone(),
        });
    }
    let canonical_points = hir
        .canonical_points
        .iter()
        .map(|point| MirCanonicalPoint3F32 {
            x: point.x,
            y: point.y,
            z: point.z,
        })
        .collect::<Vec<_>>();
    let spatial_segments = hir
        .spatial_segments
        .iter()
        .map(|segment| MirSpatialSegment {
            length_meters: segment.length_meters,
            cumulative_end_meters: segment.cumulative_end_meters,
            tangent: segment.tangent,
            up: segment.up,
        })
        .collect::<Vec<_>>();

    let participant_class_mapping =
        dense_mapping::<MirParticipantClassTag>(hir.participant_classes.len())?;
    let participant_classes = hir
        .participant_classes
        .iter()
        .map(|participant_class| MirParticipantClass {
            module: hir_module_to_mir[participant_class.module.index()],
            stable_key: Arc::clone(&participant_class.stable_key),
            stable_id: participant_class.stable_id,
            parent: participant_class
                .parent
                .map(|parent| participant_class_mapping[parent.index()]),
            parent_source_span: participant_class.parent_source_span.clone(),
            depth: participant_class.depth,
            subtree_enter: participant_class.subtree_enter,
            subtree_exit: participant_class.subtree_exit,
            source_span: participant_class.source_span.clone(),
        })
        .collect::<Vec<_>>();
    let vehicle_profile_mapping =
        dense_mapping::<MirVehicleProfileTag>(hir.vehicle_profiles.len())?;
    let vehicle_profiles = hir
        .vehicle_profiles
        .iter()
        .map(|profile| MirVehicleProfile {
            module: hir_module_to_mir[profile.module.index()],
            stable_key: Arc::clone(&profile.stable_key),
            stable_id: profile.stable_id,
            participant_class: participant_class_mapping[profile.participant_class.index()],
            participant_class_source_span: profile.participant_class_source_span.clone(),
            length_meters: profile.length_meters,
            desired_speed_meters_per_second: profile.desired_speed_meters_per_second,
            min_gap_meters: profile.min_gap_meters,
            time_headway_seconds: profile.time_headway_seconds,
            max_acceleration_meters_per_second_squared: profile
                .max_acceleration_meters_per_second_squared,
            comfortable_deceleration_meters_per_second_squared: profile
                .comfortable_deceleration_meters_per_second_squared,
            emergency_deceleration_meters_per_second_squared: profile
                .emergency_deceleration_meters_per_second_squared,
            source_span: profile.source_span.clone(),
        })
        .collect::<Vec<_>>();
    debug_assert_eq!(vehicle_profile_mapping.len(), vehicle_profiles.len());
    let access_rule_mapping = dense_mapping::<MirAccessRuleTag>(hir.access_rules.len())?;
    let access_rules = hir
        .access_rules
        .iter()
        .map(|rule| {
            let target = match rule.target {
                HirAccessTarget::LaneEdge(target) => {
                    MirAccessTarget::LaneEdge(hir_to_mir[target.index()])
                }
                HirAccessTarget::LaneGroup(target) => {
                    MirAccessTarget::LaneGroup(group_mapping[target.index()])
                }
                HirAccessTarget::RoadSection(target) => {
                    MirAccessTarget::RoadSection(section_mapping[target.index()])
                }
                HirAccessTarget::ManeuverPath(target) => {
                    MirAccessTarget::ManeuverPath(maneuver_path_mapping[target.index()])
                }
            };
            Ok(MirAccessRule {
                module: hir_module_to_mir[rule.module.index()],
                stable_key: Arc::clone(&rule.stable_key),
                stable_id: rule.stable_id,
                target,
                target_source_span: rule.target_source_span.clone(),
                effect: rule.effect,
                participant_classes: remap_range(
                    rule.participant_classes,
                    &unit.limits,
                    &rule.source_span,
                )?,
                regulation: rule
                    .regulation
                    .as_ref()
                    .map(|regulation| MirAccessRegulation {
                        jurisdiction: Arc::clone(&regulation.jurisdiction),
                        version: Arc::clone(&regulation.version),
                        source: regulation.source.clone(),
                    }),
                priority: rule.priority,
                source_span: rule.source_span.clone(),
            })
        })
        .collect::<Result<Vec<_>, DiagnosticBundle>>()?;
    let access_rule_participant_classes = hir
        .access_rule_participant_classes
        .iter()
        .map(|selector| MirAccessRuleParticipantClass {
            participant_class: participant_class_mapping[selector.participant_class.index()],
            source_span: selector.source_span.clone(),
        })
        .collect::<Vec<_>>();
    debug_assert_eq!(access_rule_mapping.len(), access_rules.len());

    let static_route_mapping = dense_mapping::<MirStaticRouteTag>(hir.static_routes.len())?;
    debug_assert_eq!(static_route_mapping.len(), hir.static_routes.len());
    let static_routes = hir
        .static_routes
        .iter()
        .map(|route| {
            Ok(MirStaticRoute {
                module: hir_module_to_mir[route.module.index()],
                stable_key: Arc::clone(&route.stable_key),
                stable_id: route.stable_id,
                edges: remap_range(route.edges, &unit.limits, &route.source_span)?,
                transitions: remap_range(route.transitions, &unit.limits, &route.source_span)?,
                maneuver_occurrences: remap_range(
                    route.maneuver_occurrences,
                    &unit.limits,
                    &route.source_span,
                )?,
                gate_occurrences: remap_range(
                    route.gate_occurrences,
                    &unit.limits,
                    &route.source_span,
                )?,
                waiting_zone_occurrences: remap_range(
                    route.waiting_zone_occurrences,
                    &unit.limits,
                    &route.source_span,
                )?,
                source_span: route.source_span.clone(),
            })
        })
        .collect::<Result<Vec<_>, DiagnosticBundle>>()?;
    let static_route_edges = hir
        .static_route_edges
        .iter()
        .map(|edge| MirStaticRouteEdge {
            target: hir_to_mir[edge.target.index()],
            source_span: edge.source_span.clone(),
        })
        .collect::<Vec<_>>();
    let static_route_transitions = hir
        .static_route_transitions
        .iter()
        .map(|transition| MirStaticRouteTransition {
            maneuver_gate: transition
                .maneuver_gate
                .map(|key| maneuver_gate_mapping[key.index()]),
        })
        .collect::<Vec<_>>();
    let occurrence_span = hir
        .static_routes
        .first()
        .map(|route| &route.source_span)
        .or_else(|| hir.modules.first().map(|module| &module.source_span));
    let maneuver_occurrences = hir
        .maneuver_occurrences
        .iter()
        .map(|occurrence| {
            Ok(MirManeuverOccurrence {
                maneuver_path: maneuver_path_mapping[occurrence.maneuver_path.index()],
                entry_route_edge_index: occurrence.entry_route_edge_index,
                exit_route_edge_index: occurrence.exit_route_edge_index,
                gate_occurrences: remap_range_optional_span(
                    occurrence.gate_occurrences,
                    &unit.limits,
                    occurrence_span,
                )?,
                waiting_zone_occurrences: remap_range_optional_span(
                    occurrence.waiting_zone_occurrences,
                    &unit.limits,
                    occurrence_span,
                )?,
            })
        })
        .collect::<Result<Vec<_>, DiagnosticBundle>>()?;
    let gate_occurrences = hir
        .gate_occurrences
        .iter()
        .map(|occurrence| MirGateOccurrence {
            maneuver_gate: maneuver_gate_mapping[occurrence.maneuver_gate.index()],
            maneuver_occurrence_index: occurrence.maneuver_occurrence_index,
            from_route_edge_index: occurrence.from_route_edge_index,
            next_gate_occurrence_index: occurrence.next_gate_occurrence_index,
            next_boundary_route_edge_index: occurrence.next_boundary_route_edge_index,
            waiting_zone_occurrence_index: occurrence.waiting_zone_occurrence_index,
        })
        .collect::<Vec<_>>();
    let waiting_zone_occurrences = hir
        .waiting_zone_occurrences
        .iter()
        .map(|occurrence| MirWaitingZoneOccurrence {
            waiting_zone: waiting_zone_mapping[occurrence.waiting_zone.index()],
            maneuver_occurrence_index: occurrence.maneuver_occurrence_index,
            entry_gate_occurrence_index: occurrence.entry_gate_occurrence_index,
            release_gate_occurrence_index: occurrence.release_gate_occurrence_index,
            entry_route_edge_index: occurrence.entry_route_edge_index,
            release_route_edge_index: occurrence.release_route_edge_index,
        })
        .collect::<Vec<_>>();

    debug_assert_eq!(modules.len(), hir.modules.len());
    debug_assert_eq!(lane_edges.len(), edge_capacity);
    debug_assert_eq!(connections.len(), connection_capacity);
    Ok(MirUnit {
        geometry_profiles: hir.geometry_profiles,
        modules: modules.into_boxed_slice(),
        lane_edges: lane_edges.into_boxed_slice(),
        lane_edge_connections: connections.into_boxed_slice(),
        road_corridors: road_corridors.into_boxed_slice(),
        corridor_elements: corridor_elements.into_boxed_slice(),
        road_sections: road_sections.into_boxed_slice(),
        authoring_lanes: authoring_lanes.into_boxed_slice(),
        authoring_lane_edges: authoring_lane_edges.into_boxed_slice(),
        lane_groups: lane_groups.into_boxed_slice(),
        lane_group_members: lane_group_members.into_boxed_slice(),
        facility_bands: facility_bands.into_boxed_slice(),
        junctions: junctions.into_boxed_slice(),
        movements: movements.into_boxed_slice(),
        junction_movements: junction_movements.into_boxed_slice(),
        maneuver_paths: maneuver_paths.into_boxed_slice(),
        movement_maneuver_paths: movement_maneuver_paths.into_boxed_slice(),
        maneuver_path_edges: maneuver_path_edges.into_boxed_slice(),
        junction_internal_edges: junction_internal_edges.into_boxed_slice(),
        stop_lines: stop_lines.into_boxed_slice(),
        maneuver_gates: maneuver_gates.into_boxed_slice(),
        waiting_zones: waiting_zones.into_boxed_slice(),
        maneuver_path_gates: maneuver_path_gates.into_boxed_slice(),
        maneuver_path_waiting_zones: maneuver_path_waiting_zones.into_boxed_slice(),
        stop_line_maneuver_gates: stop_line_maneuver_gates.into_boxed_slice(),
        signal_groups: signal_groups.into_boxed_slice(),
        signal_controllers: signal_controllers.into_boxed_slice(),
        signal_controller_groups: signal_controller_groups.into_boxed_slice(),
        signal_phases: signal_phases.into_boxed_slice(),
        signal_phase_states: signal_phase_states.into_boxed_slice(),
        signal_group_maneuver_gates: signal_group_maneuver_gates.into_boxed_slice(),
        parking_areas: parking_areas.into_boxed_slice(),
        parking_spaces: parking_spaces.into_boxed_slice(),
        parking_area_spaces: parking_area_spaces.into_boxed_slice(),
        canonical_frames: canonical_frames.into_boxed_slice(),
        lane_edge_geometries: lane_edge_geometries.into_boxed_slice(),
        geometry_source_ranges: geometry_source_ranges.into_boxed_slice(),
        facility_band_geometries: facility_band_geometries.into_boxed_slice(),
        canonical_points: canonical_points.into_boxed_slice(),
        spatial_segments: spatial_segments.into_boxed_slice(),
        participant_classes: participant_classes.into_boxed_slice(),
        vehicle_profiles: vehicle_profiles.into_boxed_slice(),
        access_rules: access_rules.into_boxed_slice(),
        access_rule_participant_classes: access_rule_participant_classes.into_boxed_slice(),
        static_routes: static_routes.into_boxed_slice(),
        static_route_edges: static_route_edges.into_boxed_slice(),
        static_route_transitions: static_route_transitions.into_boxed_slice(),
        maneuver_occurrences: maneuver_occurrences.into_boxed_slice(),
        gate_occurrences: gate_occurrences.into_boxed_slice(),
        waiting_zone_occurrences: waiting_zone_occurrences.into_boxed_slice(),
        mir_record_count,
        controlled_live_bytes: mir_owned_bytes,
        peak_controlled_live_bytes: controlled_live_bytes,
    })
}

fn mir_key_for_hir(key: HirLaneEdgeKey, mapping: &[MirLaneEdgeKey]) -> MirLaneEdgeKey {
    mapping[key.index()]
}

fn dense_mapping<K>(count: usize) -> Result<Vec<ArenaKey<K>>, DiagnosticBundle> {
    (0..count)
        .map(|index| {
            u32::try_from(index).map(ArenaKey::from_raw).map_err(|_| {
                DiagnosticBundle::single(Diagnostic::compile_limit_exceeded_at(
                    CompileLimitDimension::MirRecordCount,
                    u64::from(u32::MAX),
                    u64::from(u32::MAX) + 1,
                    None::<SourceLocation>,
                    None,
                ))
            })
        })
        .collect()
}

fn remap_range<T, U>(
    range: TableRange<T>,
    limits: &crate::CompileLimits,
    source_span: &SourceLocation,
) -> Result<TableRange<U>, DiagnosticBundle> {
    TableRange::try_from_usize(range.start() as usize, range.len() as usize)
        .map_err(|overflow| arena_overflow(overflow, limits, Some(source_span.clone())))
}

fn remap_range_optional_span<T, U>(
    range: TableRange<T>,
    limits: &crate::CompileLimits,
    source_span: Option<&SourceLocation>,
) -> Result<TableRange<U>, DiagnosticBundle> {
    TableRange::try_from_usize(range.start() as usize, range.len() as usize)
        .map_err(|overflow| arena_overflow(overflow, limits, source_span.cloned()))
}

fn requested_bytes<T>(count: u64) -> u64 {
    count.saturating_mul(u64::try_from(size_of::<T>()).unwrap_or(u64::MAX))
}

fn arena_overflow(
    _: ArenaKeyOverflow,
    limits: &crate::CompileLimits,
    primary_span: Option<SourceLocation>,
) -> DiagnosticBundle {
    DiagnosticBundle::single(Diagnostic::compile_limit_exceeded_at(
        CompileLimitDimension::MirRecordCount,
        limits.value(CompileLimitDimension::MirRecordCount),
        u64::from(u32::MAX) + 1,
        primary_span,
        None,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::build_hir;
    use crate::{
        CompilationUnitBuilder, CompileLimits, DiagnosticPayload, LaneEdgeInput, LaneEdgeReference,
        SourceModuleHeader, SourceModuleHeaderInput, SyntheticModule, SyntheticModuleBuilder,
    };

    fn module(
        namespace: &str,
        imports: &[&str],
        edges: &[(&str, &[LaneEdgeReference<'_>])],
    ) -> SyntheticModule {
        let limits = CompileLimits::p100_initial_v1();
        let header = SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: namespace,
                source_document_key: namespace,
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
        for (key, successors) in edges {
            builder
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: key,
                    length_meters: 12.5,
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

    fn projection(mir: &MirUnit) -> Vec<(String, String, Vec<u32>)> {
        mir.lane_edges
            .iter()
            .map(|edge| {
                (
                    mir.modules[edge.module.index()]
                        .authoring_namespace_id
                        .to_string(),
                    edge.stable_key.to_string(),
                    mir.lane_edge_connections[edge.connections.as_usize_range()]
                        .iter()
                        .map(|connection| connection.target.raw())
                        .collect(),
                )
            })
            .collect()
    }

    #[test]
    fn mir_freezes_resolved_lane_edges_and_flat_connection_ranges() {
        let app_successors = [
            LaneEdgeReference::imported("city/base", "edge-b"),
            LaneEdgeReference::local("edge-c"),
        ];
        let unit = unit([
            module(
                "city/app",
                &["city/base"],
                &[("edge-c", &[]), ("edge-a", &app_successors)],
            ),
            module("city/base", &[], &[("edge-b", &[])]),
        ]);
        let hir = build_hir(&unit).unwrap();
        let mir = lower_to_mir(&unit, &hir).unwrap();

        assert_eq!(mir.modules.len(), 2);
        assert_eq!(mir.lane_edges.len(), 3);
        assert_eq!(mir.lane_edge_connections.len(), 2);
        assert_eq!(mir.mir_record_count, 5);
        assert_eq!(mir.modules[1].source_span.source_document_key(), "city/app");
        assert_eq!(mir.lane_edges[1].stable_id, hir.lane_edges[1].stable_id);
        assert_eq!(mir.lane_edges[1].length_meters, 12.5);
        assert_eq!(mir.lane_edges[1].speed_limit_meters_per_second, 13.75);
        assert_eq!(
            mir.lane_edges[1].source_span.source_document_key(),
            "city/app"
        );
        assert_eq!(
            mir.lane_edge_connections[0]
                .source_span
                .source_document_key(),
            "city/app"
        );
        assert_eq!(
            projection(&mir),
            [
                ("city/base".into(), "edge-b".into(), vec![]),
                ("city/app".into(), "edge-a".into(), vec![2, 0]),
                ("city/app".into(), "edge-c".into(), vec![]),
            ]
        );
    }

    #[test]
    fn mir_topology_is_identical_after_declaration_permutation() {
        let successors = [
            LaneEdgeReference::local("edge-c"),
            LaneEdgeReference::local("edge-b"),
        ];
        let left_unit = unit([module(
            "city/a",
            &[],
            &[("edge-a", &successors), ("edge-b", &[]), ("edge-c", &[])],
        )]);
        let right_unit = unit([module(
            "city/a",
            &[],
            &[("edge-c", &[]), ("edge-a", &successors), ("edge-b", &[])],
        )]);
        let left_hir = build_hir(&left_unit).unwrap();
        let right_hir = build_hir(&right_unit).unwrap();
        let left = lower_to_mir(&left_unit, &left_hir).unwrap();
        let right = lower_to_mir(&right_unit, &right_hir).unwrap();

        assert_eq!(projection(&left), projection(&right));
        assert_eq!(left.mir_record_count, right.mir_record_count);
    }

    #[test]
    fn mir_checks_record_scratch_and_live_byte_limits_before_stage_allocation() {
        let successors = [LaneEdgeReference::local("edge-a")];
        let mut unit = unit([module("city/a", &[], &[("edge-a", &successors)])]);
        let hir = build_hir(&unit).unwrap();

        unit.limits = CompileLimits::p100_initial_v1().with_test_pipeline_limits(
            u32::MAX,
            1,
            u32::MAX,
            u32::MAX,
        );
        let record_failure = match lower_to_mir(&unit, &hir) {
            Ok(_) => panic!("MIR record limit must fail closed"),
            Err(diagnostics) => diagnostics,
        };
        assert!(matches!(
            record_failure.diagnostics()[0].payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::MirRecordCount,
                limit: 1,
                observed: 2,
            }
        ));

        unit.limits = CompileLimits::p100_initial_v1().with_test_pipeline_limits(
            u32::MAX,
            u32::MAX,
            0,
            u32::MAX,
        );
        let scratch_failure = match lower_to_mir(&unit, &hir) {
            Ok(_) => panic!("MIR scratch limit must fail closed"),
            Err(diagnostics) => diagnostics,
        };
        assert!(
            scratch_failure
                .diagnostics()
                .iter()
                .any(|diagnostic| matches!(
                    diagnostic.payload(),
                    DiagnosticPayload::CompileLimitExceeded {
                        dimension: CompileLimitDimension::StageScratchBytes,
                        limit: 0,
                        observed,
                    } if *observed > 0
                ))
        );

        let input_live_bytes =
            u32::try_from(unit.controlled_live_bytes + hir.controlled_live_bytes).unwrap();
        unit.limits = CompileLimits::p100_initial_v1().with_test_pipeline_limits(
            u32::MAX,
            u32::MAX,
            u32::MAX,
            input_live_bytes,
        );
        let live_failure = match lower_to_mir(&unit, &hir) {
            Ok(_) => panic!("MIR live byte limit must fail closed"),
            Err(diagnostics) => diagnostics,
        };
        assert!(live_failure.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::CompilerControlledLiveBytes,
                limit,
                observed,
            } if *limit == u64::from(input_live_bytes) && observed > limit
        )));
    }
}
