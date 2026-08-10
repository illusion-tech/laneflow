//! Typed AST 到高层中间表示（HIR）的符号解析阶段。
//!
//! 输入 [`CompilationUnit`] 已闭合模块导入图并冻结依赖优先顺序。本阶段据此建立连续
//! 模块表与分实体符号表，把 `(module namespace, stable key)` 引用解析为阶段私有
//! `u32` 键，并保留来源位置供后续诊断/源映射使用。声明先全部登记、再统一解析引用，
//! 因此前向引用和自环合法；横断面子阶段在派生子实体身份前证明唯一所有者树，路口
//! 子阶段则闭合父子身份、完整机动路径与内部边排他角色。
//!
//! HIR 表顺序是规范顺序：模块沿用编译单元顺序，模块内声明按稳定键排序，导入和连接
//! 也使用已显式规范化的序列。`HashMap` 仅作查找，绝不能通过迭代哈希表决定诊断或
//! 后续布局。所有键、区间和类型均为 crate 私有，不能跨阶段或进入持久制品。

use core::hash::{Hash, Hasher};
use std::collections::HashMap;
use std::sync::Arc;

use laneflow_static_contract::{
    AccessEffect, AccessRuleId, AuthoringLaneId, CanonicalFrameId, EntityKind, FacilityBandId,
    FieldTag, JunctionId, LaneEdgeId, LaneGroupId, MIN_PARKING_EXTENT_EXCLUSIVE_METERS,
    MIN_PARKING_LATERAL_OFFSET_ABS_EXCLUSIVE_METERS, ManeuverGateId, ManeuverPathId, MovementId,
    PARKING_ANCHOR_ENDPOINT_CLEARANCE_METERS, PARKING_HEADING_OFFSET_MAXIMUM_RADIANS,
    PARKING_HEADING_OFFSET_MINIMUM_RADIANS, ParkingAreaId, ParkingSpaceId, ParticipantClassId,
    RoadCorridorId, RoadSectionId, SPATIAL_CORE_LENGTH_QUANTIZATION_ALLOWANCE_METERS,
    SPATIAL_JOIN_POSITION_TOLERANCE_METERS, SPATIAL_LENGTH_ABS_TOLERANCE_METERS,
    SPATIAL_LENGTH_REL_TOLERANCE, SPATIAL_MIN_PROJECTED_UP_LENGTH,
    SPATIAL_MIN_SEGMENT_LENGTH_METERS, SignalAspect, SignalControllerId, SignalGroupId,
    SignalPhaseId, StableId128, StaticRouteId, StopLineId, VehicleProfileId, WaitingZoneId,
};

use crate::arena::{ArenaKey, ArenaKeyOverflow, TableRange, TypedArena};
use crate::declaration::{
    LaneEdgeDeclaration, MAX_PORTABLE_SIGNAL_TIME_MS, OwnedAccessRegulation, OwnedAccessRuleTarget,
    OwnedCorridorElementReference, OwnedEntityReference, OwnedSignalControl, TypedAstDeclaration,
};
use crate::diagnostic::DiagnosticCollector;
use crate::identity::{
    IdentityFieldInput, IdentityRegistrationError, IdentityRegistry, RegisteredCanonicalIdentity,
    encode_canonical_identity,
};
use crate::module::ResolvedSourceLocation;
use crate::{
    AccessCapability, AccessPlane, AccessRegulationField, CompilationUnit, CompileLimitDimension,
    Diagnostic, DiagnosticBundle, ParkingAnchorRole, ParkingGeometryField,
    ParkingGeometryViolation, SourceLocation, SpatialGeometryViolation, WaitingZoneGateRole,
};

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
pub(crate) enum HirStaticRouteTag {}
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
pub(crate) type HirStaticRouteKey = ArenaKey<HirStaticRouteTag>;
pub(crate) type HirSignalGroupKey = ArenaKey<HirSignalGroupTag>;
pub(crate) type HirSignalControllerKey = ArenaKey<HirSignalControllerTag>;
pub(crate) type HirParkingAreaKey = ArenaKey<HirParkingAreaTag>;
pub(crate) type HirParkingSpaceKey = ArenaKey<HirParkingSpaceTag>;
pub(crate) type HirParticipantClassKey = ArenaKey<HirParticipantClassTag>;
pub(crate) type HirVehicleProfileKey = ArenaKey<HirVehicleProfileTag>;
pub(crate) type HirCanonicalFrameKey = ArenaKey<HirCanonicalFrameTag>;
pub(crate) type HirAccessRuleKey = ArenaKey<HirAccessRuleTag>;

/// 已解析为 HIR 模块键的显式导入边。
pub(crate) struct HirImport {
    /// 被导入模块；目标在规范模块顺序中位于当前模块之前。
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) target: HirModuleKey,
    /// 原始导入声明位置。
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) source_span: SourceLocation,
}

/// HIR 模块记录及其在平坦导入表中的连续区间。
pub(crate) struct HirModule {
    /// 声明身份与跨模块解析使用的稳定命名空间。
    pub(crate) authoring_namespace_id: Arc<str>,
    /// 此模块在 `HirUnit::imports` 中的半开区间。
    pub(crate) imports: TableRange<HirImport>,
    /// 模块声明位置。
    pub(crate) source_span: SourceLocation,
}

/// 已解析为 HIR 车道图边键的下游引用。
pub(crate) struct HirLaneEdgeReference {
    /// 当前 `HirUnit::lane_edges` 中的目标键。
    pub(crate) target: HirLaneEdgeKey,
    /// 原始引用位置。
    pub(crate) source_span: SourceLocation,
}

/// 完成模块归属和下游符号解析的车道图边 HIR 记录。
pub(crate) struct HirLaneEdge {
    /// 拥有此声明的 HIR 模块。
    pub(crate) module: HirModuleKey,
    /// 模块内稳定键；不是 HIR 致密下标。
    pub(crate) stable_key: Arc<str>,
    /// 由 `(authoringNamespaceId, laneEdgeKey)` 的完整 Identity v1 前像派生。
    pub(crate) stable_id: LaneEdgeId,
    /// 交通权威长度，单位为米并保留来源 `f64` 精度。
    pub(crate) length_meters: f64,
    /// 基础道路限速，单位为米每秒并保留来源 `f64` 精度。
    pub(crate) speed_limit_meters_per_second: f64,
    /// 此边在 `HirUnit::lane_edge_references` 中的连续下游引用区间。
    pub(crate) successors: TableRange<HirLaneEdgeReference>,
    /// 原始声明位置。
    pub(crate) source_span: SourceLocation,
}

/// 道路走廊有序横断面中的已解析异构成员。
pub(crate) enum HirCorridorElement {
    RoadSection {
        road_section: HirRoadSectionKey,
        source_location: ResolvedSourceLocation,
    },
    FacilityBand {
        facility_band: HirFacilityBandKey,
        source_location: ResolvedSourceLocation,
    },
}

/// 已证明参考区段成员性与成员唯一所有权的道路走廊。
pub(crate) struct HirRoadCorridor {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: RoadCorridorId,
    pub(crate) reference_section: HirRoadSectionKey,
    pub(crate) elements: TableRange<HirCorridorElement>,
    pub(crate) source_span: SourceLocation,
}

/// 已闭合到唯一道路走廊父项的道路区段。
pub(crate) struct HirRoadSection {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: RoadSectionId,
    pub(crate) road_corridor: HirRoadCorridorKey,
    pub(crate) kind_id: Arc<str>,
    pub(crate) lanes: TableRange<HirAuthoringLane>,
    pub(crate) source_span: SourceLocation,
}

/// 编制车道覆盖链中的一项已解析车道图边及其来源位置。
pub(crate) struct HirAuthoringLaneEdge {
    pub(crate) target: HirLaneEdgeKey,
    pub(crate) source_span: SourceLocation,
}

/// 已解析父区段、覆盖链和可选车道组的编制车道。
pub(crate) struct HirAuthoringLane {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: AuthoringLaneId,
    pub(crate) road_section: HirRoadSectionKey,
    pub(crate) edge_chain: TableRange<HirAuthoringLaneEdge>,
    pub(crate) lane_group: Option<HirLaneGroupKey>,
    pub(crate) lane_group_source_location: Option<ResolvedSourceLocation>,
    pub(crate) source_span: SourceLocation,
}

/// 车道组成员表中的一条编制车道引用。
#[derive(Clone, Copy)]
pub(crate) struct HirLaneGroupMember {
    pub(crate) lane: HirAuthoringLaneKey,
}

/// 已证明所有成员与父区段一致且非空的车道组。
pub(crate) struct HirLaneGroup {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: LaneGroupId,
    pub(crate) road_section: HirRoadSectionKey,
    pub(crate) members: TableRange<HirLaneGroupMember>,
    pub(crate) source_span: SourceLocation,
}

/// 已闭合到唯一道路走廊父项的非遍历设施带。
pub(crate) struct HirFacilityBand {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: FacilityBandId,
    pub(crate) road_corridor: HirRoadCorridorKey,
    pub(crate) kind_id: Arc<str>,
    pub(crate) source_span: SourceLocation,
}

/// 已解析出非空通行流向成员区间的路口。
pub(crate) struct HirJunction {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: JunctionId,
    pub(crate) movements: TableRange<HirJunctionMovement>,
    pub(crate) source_span: SourceLocation,
}

/// 已闭合到唯一路口父项并保留 Identity v1 有向引道键的通行流向。
pub(crate) struct HirMovement {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: MovementId,
    pub(crate) junction: HirJunctionKey,
    pub(crate) junction_source_location: Option<ResolvedSourceLocation>,
    pub(crate) directed_entry_approach_key: Arc<str>,
    pub(crate) directed_exit_approach_key: Arc<str>,
    pub(crate) maneuver_paths: TableRange<HirMovementManeuverPath>,
    pub(crate) source_span: SourceLocation,
}

#[derive(Clone, Copy)]
pub(crate) struct HirJunctionMovement {
    pub(crate) movement: HirMovementKey,
}

#[derive(Clone, Copy)]
pub(crate) struct HirMovementManeuverPath {
    pub(crate) maneuver_path: HirManeuverPathKey,
}

/// 一条机动路径完整遍历序列中的已解析车道图边。
pub(crate) struct HirManeuverPathEdge {
    pub(crate) target: HirLaneEdgeKey,
    pub(crate) source_span: SourceLocation,
}

/// 已解析父项、入口/内部/出口边和全局唯一遍历序列的机动路径。
pub(crate) struct HirManeuverPath {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: ManeuverPathId,
    pub(crate) movement: HirMovementKey,
    pub(crate) movement_source_location: Option<ResolvedSourceLocation>,
    /// 完整序列 `entry + internal + exit`；首尾是边界边，中间区间是内部边。
    pub(crate) edges: TableRange<HirManeuverPathEdge>,
    /// 按 `transition_index` 严格递增的机动门成员区间。
    pub(crate) maneuver_gates: TableRange<HirManeuverPathGate>,
    /// 按入口转换、释放转换和稳定 ID 排序的等待区成员区间。
    pub(crate) waiting_zones: TableRange<HirManeuverPathWaitingZone>,
    pub(crate) source_span: SourceLocation,
}

/// 机动路径规范门序列中的一项。
#[derive(Clone, Copy)]
pub(crate) struct HirManeuverPathGate {
    pub(crate) maneuver_gate: HirManeuverGateKey,
}

/// 机动路径规范等待区序列中的一项。
#[derive(Clone, Copy)]
pub(crate) struct HirManeuverPathWaitingZone {
    pub(crate) waiting_zone: HirWaitingZoneKey,
}

/// 停止线到引用它的机动门的反向关系项。
#[derive(Clone, Copy)]
pub(crate) struct HirStopLineManeuverGate {
    pub(crate) maneuver_gate: HirManeuverGateKey,
}

/// 已解析边位置并证明至少被一个机动门使用的停止线。
pub(crate) struct HirStopLine {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: StopLineId,
    pub(crate) lane_edge: HirLaneEdgeKey,
    pub(crate) maneuver_gates: TableRange<HirStopLineManeuverGate>,
    pub(crate) source_span: SourceLocation,
}

/// 已闭合到合法路径转换和同边停止线的机动门。
pub(crate) struct HirManeuverGate {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: ManeuverGateId,
    pub(crate) maneuver_path: HirManeuverPathKey,
    pub(crate) maneuver_path_source_location: Option<ResolvedSourceLocation>,
    pub(crate) transition_index: u32,
    pub(crate) stop_line: HirStopLineKey,
    pub(crate) stop_line_source_location: Option<ResolvedSourceLocation>,
    /// 信号层绑定；`None` 不改变其他通行权层的约束。
    pub(crate) signal_control: HirSignalControl,
    pub(crate) source_span: SourceLocation,
}

#[derive(Clone)]
pub(crate) enum HirSignalControl {
    Group {
        signal_group: HirSignalGroupKey,
        source_location: ResolvedSourceLocation,
    },
    None,
}

/// 由一个固定时制控制器唯一拥有、并至少控制一个机动门的信号组。
pub(crate) struct HirSignalGroup {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: SignalGroupId,
    pub(crate) controller: HirSignalControllerKey,
    pub(crate) maneuver_gates: TableRange<HirSignalGroupManeuverGate>,
    pub(crate) source_span: SourceLocation,
}

/// 一个信号组控制的机动门反向关系项。
#[derive(Clone, Copy)]
pub(crate) struct HirSignalGroupManeuverGate {
    pub(crate) maneuver_gate: HirManeuverGateKey,
}

/// 控制器有序信号组列表中的一项。
#[derive(Clone)]
pub(crate) struct HirSignalControllerGroup {
    pub(crate) signal_group: HirSignalGroupKey,
    pub(crate) source_location: ResolvedSourceLocation,
}

/// 固定时制控制器的不可变循环程序。
pub(crate) struct HirSignalController {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: SignalControllerId,
    pub(crate) offset_ms: u64,
    pub(crate) cycle_duration_ms: u64,
    pub(crate) signal_groups: TableRange<HirSignalControllerGroup>,
    pub(crate) phases: TableRange<HirSignalPhase>,
    pub(crate) source_span: SourceLocation,
}

/// 控制器所有者局部（owner-local）的一个有序固定时制相位。
pub(crate) struct HirSignalPhase {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: SignalPhaseId,
    pub(crate) controller: HirSignalControllerKey,
    pub(crate) duration_ms: u64,
    /// 状态按所属控制器的 `signal_groups` 顺序规范化，而非按输入顺序保存。
    pub(crate) states: TableRange<HirSignalPhaseState>,
    pub(crate) source_span: SourceLocation,
}

/// 一个相位对其控制器信号组的完整灯色赋值。
#[derive(Clone)]
pub(crate) struct HirSignalPhaseState {
    pub(crate) signal_group: HirSignalGroupKey,
    pub(crate) aspect: SignalAspect,
    pub(crate) source_location: ResolvedSourceLocation,
}

/// 停车区域的一个规范停车位成员。
#[derive(Clone, Copy)]
pub(crate) struct HirParkingAreaSpace {
    pub(crate) parking_space: HirParkingSpaceKey,
}

/// 已证明至少拥有一个停车位成员的停车区域。
pub(crate) struct HirParkingArea {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: ParkingAreaId,
    pub(crate) parking_spaces: TableRange<HirParkingAreaSpace>,
    pub(crate) source_span: SourceLocation,
}

/// 已解析到车道图边严格内部位置的停车锚点。
#[derive(Clone)]
pub(crate) struct HirParkingLaneAnchor {
    pub(crate) lane_edge: HirLaneEdgeKey,
    pub(crate) progress_meters: f64,
    pub(crate) source_location: ResolvedSourceLocation,
}

/// 已验证的停车位矩形几何；数值保持来源 `f64` 精度。
#[derive(Clone, Copy)]
pub(crate) struct HirParkingSpaceGeometry {
    pub(crate) lateral_offset_meters: f64,
    pub(crate) heading_offset_radians: f64,
    pub(crate) length_meters: f64,
    pub(crate) width_meters: f64,
}

/// 已闭合可选区域归属、入口/出口锚点和矩形几何的停车位。
pub(crate) struct HirParkingSpace {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: ParkingSpaceId,
    pub(crate) parking_area: Option<HirParkingAreaKey>,
    pub(crate) parking_area_source_location: Option<ResolvedSourceLocation>,
    pub(crate) entry: HirParkingLaneAnchor,
    pub(crate) exit: HirParkingLaneAnchor,
    pub(crate) geometry: HirParkingSpaceGeometry,
    pub(crate) source_span: SourceLocation,
}

/// 已解析父类并编译单继承层级信息的参与者类别。
pub(crate) struct HirParticipantClass {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: ParticipantClassId,
    pub(crate) parent: Option<HirParticipantClassKey>,
    pub(crate) parent_source_span: Option<SourceLocation>,
    /// 根类别为 0；准入规则以更深类别作为更高 specificity。
    pub(crate) depth: u32,
    /// Euler tour 半开子树区间 `[enter, exit)`。
    pub(crate) subtree_enter: u32,
    pub(crate) subtree_exit: u32,
    pub(crate) source_span: SourceLocation,
}

/// 已解析唯一参与者类别、并保持 current Core IIDM `f64` 语义的车辆配置。
pub(crate) struct HirVehicleProfile {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: VehicleProfileId,
    pub(crate) participant_class: HirParticipantClassKey,
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

/// 已冻结稳定身份的规范坐标框架。
///
/// 该记录故意不保存轴向、单位或宿主放置：这些语义分别由全局 canonical frame
/// 契约和 Adapter 边界拥有，不能成为同一 `frameId` 下的可变配置。
pub(crate) struct HirCanonicalFrame {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: CanonicalFrameId,
    pub(crate) lane_edge_geometries: TableRange<HirLaneEdgeGeometry>,
    pub(crate) source_span: SourceLocation,
}

/// 规范坐标框架内的一条中心线；点与线段区间均按行驶方向排列。
pub(crate) struct HirLaneEdgeGeometry {
    pub(crate) canonical_frame: HirCanonicalFrameKey,
    pub(crate) lane_edge: HirLaneEdgeKey,
    pub(crate) points: TableRange<HirCanonicalPoint3F32>,
    pub(crate) segments: TableRange<HirSpatialSegment>,
    pub(crate) arc_length_meters: f32,
    pub(crate) source_span: SourceLocation,
}

#[derive(Clone, Copy)]
pub(crate) struct HirCanonicalPoint3F32 {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) z: f32,
}

#[derive(Clone, Copy)]
pub(crate) struct HirSpatialSegment {
    pub(crate) length_meters: f32,
    pub(crate) cumulative_end_meters: f32,
    pub(crate) tangent: [f32; 3],
    pub(crate) up: [f32; 3],
}

/// HIR 中已解析且保持求值平面边界的准入目标。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum HirAccessTarget {
    LaneEdge(HirLaneEdgeKey),
    LaneGroup(HirLaneGroupKey),
    RoadSection(HirRoadSectionKey),
    ManeuverPath(HirManeuverPathKey),
}

/// 已验证的法规来源信息；该值参与规范 LIR，但不参与准入组合键。
pub(crate) struct HirAccessRegulation {
    pub(crate) jurisdiction: Arc<str>,
    pub(crate) version: Arc<str>,
    pub(crate) source: Option<Arc<str>>,
}

/// 一条准入规则引用的参与者类别。
pub(crate) struct HirAccessRuleParticipantClass {
    pub(crate) participant_class: HirParticipantClassKey,
    pub(crate) source_span: SourceLocation,
}

/// 完成静态引用解析和组合歧义验证的准入规则。
pub(crate) struct HirAccessRule {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: AccessRuleId,
    pub(crate) target: HirAccessTarget,
    pub(crate) target_source_span: SourceLocation,
    pub(crate) effect: AccessEffect,
    pub(crate) participant_classes: TableRange<HirAccessRuleParticipantClass>,
    pub(crate) regulation: Option<HirAccessRegulation>,
    pub(crate) priority: i32,
    pub(crate) source_span: SourceLocation,
}

/// 已证明门所有权、严格正向区间和同路径内部不重叠的等待区。
pub(crate) struct HirWaitingZone {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: WaitingZoneId,
    pub(crate) maneuver_path: HirManeuverPathKey,
    pub(crate) maneuver_path_source_location: Option<ResolvedSourceLocation>,
    pub(crate) entry_gate: HirManeuverGateKey,
    pub(crate) release_gate: HirManeuverGateKey,
    pub(crate) max_occupancy: u32,
    pub(crate) source_span: SourceLocation,
}

/// 从全部路径派生的路口内部边排他所有者。
pub(crate) struct HirJunctionInternalEdge {
    pub(crate) edge: HirLaneEdgeKey,
    pub(crate) junction: HirJunctionKey,
    /// 首次建立该排他声明的路径，供诊断回链、规范来源选择与路线闭包使用。
    pub(crate) source_path: HirManeuverPathKey,
    pub(crate) source_span: SourceLocation,
}

/// 静态路线有序边序列中的一次出现；同一 `LaneEdge` 可以出现多次。
pub(crate) struct HirStaticRouteEdge {
    pub(crate) target: HirLaneEdgeKey,
    pub(crate) source_span: SourceLocation,
}

/// 静态路线相邻边转换上预编译的可选机动门。
pub(crate) struct HirStaticRouteTransition {
    pub(crate) maneuver_gate: Option<HirManeuverGateKey>,
}

/// 一条完整机动路径在静态路线中的一次匹配。
pub(crate) struct HirManeuverOccurrence {
    pub(crate) maneuver_path: HirManeuverPathKey,
    pub(crate) entry_route_edge_index: u32,
    pub(crate) exit_route_edge_index: u32,
    pub(crate) gate_occurrences: TableRange<HirGateOccurrence>,
    pub(crate) waiting_zone_occurrences: TableRange<HirWaitingZoneOccurrence>,
}

/// 一个 `ManeuverGate` 在某次路线机动中的预编译出现项。
pub(crate) struct HirGateOccurrence {
    pub(crate) maneuver_gate: HirManeuverGateKey,
    pub(crate) maneuver_occurrence_index: u32,
    pub(crate) from_route_edge_index: u32,
    pub(crate) next_gate_occurrence_index: Option<u32>,
    pub(crate) next_boundary_route_edge_index: u32,
    pub(crate) waiting_zone_occurrence_index: Option<u32>,
}

/// 一个 `WaitingZone` 在某次路线机动中的预编译出现项。
pub(crate) struct HirWaitingZoneOccurrence {
    pub(crate) waiting_zone: HirWaitingZoneKey,
    pub(crate) maneuver_occurrence_index: u32,
    pub(crate) entry_gate_occurrence_index: u32,
    pub(crate) release_gate_occurrence_index: u32,
    pub(crate) entry_route_edge_index: u32,
    pub(crate) release_route_edge_index: u32,
}

/// 已解析边序列并闭合全部路口控制出现项的静态路线。
pub(crate) struct HirStaticRoute {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: StaticRouteId,
    pub(crate) edges: TableRange<HirStaticRouteEdge>,
    pub(crate) transitions: TableRange<HirStaticRouteTransition>,
    pub(crate) maneuver_occurrences: TableRange<HirManeuverOccurrence>,
    pub(crate) gate_occurrences: TableRange<HirGateOccurrence>,
    pub(crate) waiting_zone_occurrences: TableRange<HirWaitingZoneOccurrence>,
    pub(crate) source_span: SourceLocation,
}

/// HIR 阶段成功后一次性冻结的连续只读表集合。
///
/// 构造完成时所有引用均已解析，所有 `TableRange` 都落在对应平坦表内。字段中的键只对
/// 本实例有效。`controlled_live_bytes` 统计成功返回后由 HIR 自身持有的阶段字节；
/// `peak_controlled_live_bytes` 另保存资源预检已经计算的输入、查找表和暂存区共存峰值。
pub(crate) struct HirUnit {
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
    pub(crate) canonical_points: Box<[HirCanonicalPoint3F32]>,
    pub(crate) spatial_segments: Box<[HirSpatialSegment]>,
    pub(crate) access_rules: Box<[HirAccessRule]>,
    pub(crate) access_rule_participant_classes: Box<[HirAccessRuleParticipantClass]>,
    pub(crate) static_routes: Box<[HirStaticRoute]>,
    pub(crate) static_route_edges: Box<[HirStaticRouteEdge]>,
    pub(crate) static_route_transitions: Box<[HirStaticRouteTransition]>,
    pub(crate) maneuver_occurrences: Box<[HirManeuverOccurrence]>,
    pub(crate) gate_occurrences: Box<[HirGateOccurrence]>,
    pub(crate) waiting_zone_occurrences: Box<[HirWaitingZoneOccurrence]>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) hir_record_count: u64,
    pub(crate) controlled_live_bytes: u64,
    pub(crate) peak_controlled_live_bytes: u64,
}

/// 按 HIR 模块隔离的有类型符号查找索引；不提供规范遍历能力。
struct SymbolTable<K> {
    by_module: Vec<HashMap<Arc<str>, K>>,
}

impl<K: Copy> SymbolTable<K> {
    fn new(module_declaration_counts: impl IntoIterator<Item = usize>) -> Self {
        Self {
            by_module: module_declaration_counts
                .into_iter()
                .map(HashMap::with_capacity)
                .collect(),
        }
    }

    fn insert(&mut self, module: HirModuleKey, stable_key: Arc<str>, key: K) {
        let previous = self.by_module[module.index()].insert(stable_key, key);
        debug_assert!(
            previous.is_none(),
            "Typed AST rejected duplicate declarations"
        );
    }

    fn get(&self, module: HirModuleKey, stable_key: &str) -> Option<K> {
        self.by_module[module.index()].get(stable_key).copied()
    }
}

#[derive(Clone, Copy)]
/// 把规范 HIR 键映回 Typed AST 物理位置的阶段暂存记录。
///
/// HIR 键不能冒充来源模块/声明下标；显式保存两者可在声明排序后仍准确读取来源记录。
struct CanonicalLaneEdgeSource {
    source_module_index: u32,
    declaration_index: u32,
    hir_key: HirLaneEdgeKey,
}

#[derive(Clone, Copy)]
struct CanonicalDeclarationSource<K> {
    source_module_index: u32,
    declaration_index: u32,
    hir_key: K,
}

#[derive(Clone, Copy)]
struct CanonicalAuthoringLaneSource {
    source_module_index: u32,
    declaration_index: u32,
    lane_index: u32,
    hir_key: HirAuthoringLaneKey,
}

#[derive(Default)]
struct CrossSectionHir {
    road_corridors: Box<[HirRoadCorridor]>,
    corridor_elements: Box<[HirCorridorElement]>,
    road_sections: Box<[HirRoadSection]>,
    authoring_lanes: Box<[HirAuthoringLane]>,
    authoring_lane_edges: Box<[HirAuthoringLaneEdge]>,
    lane_groups: Box<[HirLaneGroup]>,
    lane_group_members: Box<[HirLaneGroupMember]>,
    facility_bands: Box<[HirFacilityBand]>,
}

#[derive(Default)]
struct CrossSectionCounts {
    road_corridors: u64,
    corridor_elements: u64,
    road_sections: u64,
    authoring_lanes: u64,
    authoring_lane_edges: u64,
    lane_groups: u64,
    facility_bands: u64,
}

impl CrossSectionCounts {
    fn entity_count(&self) -> u64 {
        self.road_corridors
            .saturating_add(self.road_sections)
            .saturating_add(self.authoring_lanes)
            .saturating_add(self.lane_groups)
            .saturating_add(self.facility_bands)
    }
}

#[derive(Default)]
struct JunctionHir {
    junctions: Box<[HirJunction]>,
    movements: Box<[HirMovement]>,
    junction_movements: Box<[HirJunctionMovement]>,
    maneuver_paths: Box<[HirManeuverPath]>,
    movement_maneuver_paths: Box<[HirMovementManeuverPath]>,
    maneuver_path_edges: Box<[HirManeuverPathEdge]>,
    junction_internal_edges: Box<[HirJunctionInternalEdge]>,
}

#[derive(Default)]
struct JunctionCounts {
    junctions: u64,
    movements: u64,
    maneuver_paths: u64,
    maneuver_path_edges: u64,
}

#[derive(Default)]
struct ControlHir {
    stop_lines: Box<[HirStopLine]>,
    maneuver_gates: Box<[HirManeuverGate]>,
    waiting_zones: Box<[HirWaitingZone]>,
    maneuver_path_gates: Box<[HirManeuverPathGate]>,
    maneuver_path_waiting_zones: Box<[HirManeuverPathWaitingZone]>,
    stop_line_maneuver_gates: Box<[HirStopLineManeuverGate]>,
}

#[derive(Default)]
struct ControlCounts {
    stop_lines: u64,
    maneuver_gates: u64,
    waiting_zones: u64,
}

#[derive(Default)]
struct SignalHir {
    signal_groups: Box<[HirSignalGroup]>,
    signal_controllers: Box<[HirSignalController]>,
    signal_controller_groups: Box<[HirSignalControllerGroup]>,
    signal_phases: Box<[HirSignalPhase]>,
    signal_phase_states: Box<[HirSignalPhaseState]>,
    signal_group_maneuver_gates: Box<[HirSignalGroupManeuverGate]>,
}

#[derive(Default)]
struct SignalCounts {
    groups: u64,
    controllers: u64,
    controller_groups: u64,
    phases: u64,
    phase_states: u64,
    controlled_gates: u64,
}

#[derive(Default)]
struct ParkingHir {
    parking_areas: Box<[HirParkingArea]>,
    parking_spaces: Box<[HirParkingSpace]>,
    parking_area_spaces: Box<[HirParkingAreaSpace]>,
}

#[derive(Default)]
struct ParkingCounts {
    areas: u64,
    spaces: u64,
    memberships: u64,
}

#[derive(Default)]
struct SpatialHir {
    canonical_frames: Box<[HirCanonicalFrame]>,
    lane_edge_geometries: Box<[HirLaneEdgeGeometry]>,
    canonical_points: Box<[HirCanonicalPoint3F32]>,
    spatial_segments: Box<[HirSpatialSegment]>,
}

#[derive(Default)]
struct SpatialCounts {
    canonical_frames: u64,
    lane_edge_geometries: u64,
    canonical_points: u64,
    spatial_segments: u64,
}

#[derive(Default)]
struct AccessHir {
    participant_classes: Box<[HirParticipantClass]>,
    vehicle_profiles: Box<[HirVehicleProfile]>,
    access_rules: Box<[HirAccessRule]>,
    access_rule_participant_classes: Box<[HirAccessRuleParticipantClass]>,
}

#[derive(Default)]
struct AccessCounts {
    participant_classes: u64,
    vehicle_profiles: u64,
    access_rules: u64,
    rule_class_references: u64,
}

#[derive(Clone, Copy)]
struct AccessCandidate {
    plane: AccessPlane,
    target_kind: EntityKind,
    target_index: u32,
    participant_class: HirParticipantClassKey,
    priority: i32,
    effect: AccessEffect,
    rule: HirAccessRuleKey,
}

/// 同一编译单元内首条有效法规来源，用于给后续不一致规则提供稳定的对照与关联位置。
struct FirstAccessRegulation {
    jurisdiction: Arc<str>,
    version: Arc<str>,
    rule_key: Arc<str>,
    source_span: SourceLocation,
}

impl AccessCounts {
    fn entity_count(&self) -> u64 {
        self.participant_classes
            .saturating_add(self.access_rules)
            .saturating_add(self.vehicle_profiles)
    }
}

impl ParkingCounts {
    fn entity_count(&self) -> u64 {
        self.areas.saturating_add(self.spaces)
    }
}

impl SignalCounts {
    fn entity_count(&self) -> u64 {
        self.groups
            .saturating_add(self.controllers)
            .saturating_add(self.phases)
    }
}

#[derive(Default)]
struct RouteHir {
    static_routes: Box<[HirStaticRoute]>,
    static_route_edges: Box<[HirStaticRouteEdge]>,
    static_route_transitions: Box<[HirStaticRouteTransition]>,
    maneuver_occurrences: Box<[HirManeuverOccurrence]>,
    gate_occurrences: Box<[HirGateOccurrence]>,
    waiting_zone_occurrences: Box<[HirWaitingZoneOccurrence]>,
}

#[derive(Default)]
struct RouteCounts {
    static_routes: u64,
    route_edges: u64,
    route_transitions: u64,
    largest_route_edges: u64,
}

impl ControlCounts {
    fn entity_count(&self) -> u64 {
        self.stop_lines
            .saturating_add(self.maneuver_gates)
            .saturating_add(self.waiting_zones)
    }
}

impl JunctionCounts {
    fn entity_count(&self) -> u64 {
        self.junctions
            .saturating_add(self.movements)
            .saturating_add(self.maneuver_paths)
    }
}

/// 只在完成路径表后借用的序列查找键；来源位置不参与遍历签名。
struct ManeuverPathSequence<'a>(&'a [HirManeuverPathEdge]);

impl PartialEq for ManeuverPathSequence<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.0
            .iter()
            .map(|edge| edge.target)
            .eq(other.0.iter().map(|edge| edge.target))
    }
}

impl Eq for ManeuverPathSequence<'_> {}

impl Hash for ManeuverPathSequence<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.len().hash(state);
        for edge in self.0 {
            edge.target.hash(state);
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
    let module_count = u64::try_from(unit.modules.len()).unwrap_or(u64::MAX);
    let lane_edge_count = lane_edge_count(unit);
    let lane_edge_reference_count = lane_edge_reference_count(unit);
    let cross_section_counts = cross_section_counts(unit);
    let junction_counts = junction_counts(unit);
    let control_counts = control_counts(unit);
    let signal_counts = signal_counts(unit);
    let parking_counts = parking_counts(unit);
    let spatial_counts = spatial_counts(unit);
    let access_counts = access_counts(unit);
    let route_counts = route_counts(unit);
    let cross_lookup_module_count = if cross_section_counts.entity_count() == 0 {
        0
    } else {
        module_count
    };
    let junction_lookup_module_count = if junction_counts.entity_count() == 0 {
        0
    } else {
        module_count
    };
    let control_lookup_module_count = if control_counts.entity_count() == 0 {
        0
    } else {
        module_count
    };
    let signal_lookup_module_count = if signal_counts.entity_count() == 0 {
        0
    } else {
        module_count
    };
    let parking_lookup_module_count = if parking_counts.entity_count() == 0 {
        0
    } else {
        module_count
    };
    let access_lookup_module_count = if access_counts.entity_count() == 0 {
        0
    } else {
        module_count
    };
    let hir_record_count = module_count
        .saturating_add(unit.import_edge_count)
        .saturating_add(unit.symbol_count)
        .saturating_add(unit.identity_field_occurrence_count)
        .saturating_add(unit.reference_count)
        .saturating_add(unit.relation_occurrence_count)
        .saturating_add(unit.geometry_point_count)
        .saturating_add(spatial_counts.spatial_segments)
        // 信号组到机动门的反向使用关系由 HIR 派生，Typed AST 只计正向绑定。
        .saturating_add(signal_counts.controlled_gates)
        // 区域归属在 Typed AST 中按停车位正向引用计数；区域成员表是 HIR 派生反向关系。
        .saturating_add(parking_counts.memberships)
        // 路线边引用已计入 CompilationUnit 关系数；转换以及三类派生出现项只在 HIR
        // 中产生，按单条边至多各生成一项的上界纳入预检。
        .saturating_add(route_counts.route_transitions)
        .saturating_add(route_counts.route_edges.saturating_mul(3));
    let canonical_source_scratch = requested_bytes::<CanonicalLaneEdgeSource>(lane_edge_count)
        .saturating_add(requested_bytes::<usize>(unit.declaration_count));
    let cross_section_scratch = if cross_section_counts.entity_count() == 0 {
        0
    } else {
        requested_bytes::<CanonicalDeclarationSource<HirRoadCorridorKey>>(
            cross_section_counts.road_corridors,
        )
        .saturating_add(requested_bytes::<
            CanonicalDeclarationSource<HirRoadSectionKey>,
        >(cross_section_counts.road_sections))
        .saturating_add(requested_bytes::<CanonicalAuthoringLaneSource>(
            cross_section_counts.authoring_lanes,
        ))
        .saturating_add(
            requested_bytes::<CanonicalDeclarationSource<HirLaneGroupKey>>(
                cross_section_counts.lane_groups,
            ),
        )
        .saturating_add(requested_bytes::<
            CanonicalDeclarationSource<HirFacilityBandKey>,
        >(cross_section_counts.facility_bands))
        .saturating_add(requested_bytes::<
            Option<(HirRoadCorridorKey, SourceLocation)>,
        >(
            cross_section_counts
                .road_sections
                .saturating_add(cross_section_counts.facility_bands),
        ))
        .saturating_add(requested_bytes::<Option<HirAuthoringLaneKey>>(
            lane_edge_count,
        ))
        .saturating_add(requested_bytes::<usize>(
            cross_section_counts.lane_groups.saturating_mul(2),
        ))
        .saturating_add(requested_bytes::<usize>(unit.declaration_count))
    };
    let import_sort_scratch = requested_bytes::<(&str, &SourceLocation)>(unit.import_edge_count);
    let junction_scratch = if junction_counts.entity_count() == 0 {
        0
    } else {
        requested_bytes::<CanonicalDeclarationSource<HirMovementKey>>(junction_counts.movements)
            .saturating_add(requested_bytes::<
                CanonicalDeclarationSource<HirManeuverPathKey>,
            >(junction_counts.maneuver_paths))
            .saturating_add(requested_bytes::<usize>(
                junction_counts
                    .junctions
                    .saturating_add(junction_counts.movements)
                    .saturating_mul(2),
            ))
            .saturating_add(requested_hash_table_bytes::<
                ManeuverPathSequence<'static>,
                HirManeuverPathKey,
            >(junction_counts.maneuver_paths))
            .saturating_add(requested_bytes::<Option<HirJunctionInternalEdge>>(
                lane_edge_count,
            ))
            .saturating_add(requested_bytes::<
                Option<(HirManeuverPathKey, SourceLocation)>,
            >(lane_edge_count))
            .saturating_add(requested_bytes::<usize>(unit.declaration_count))
    };
    let control_scratch = if control_counts.entity_count() == 0 {
        0
    } else {
        requested_bytes::<CanonicalDeclarationSource<HirStopLineKey>>(control_counts.stop_lines)
            .saturating_add(requested_bytes::<
                CanonicalDeclarationSource<HirManeuverGateKey>,
            >(control_counts.maneuver_gates))
            .saturating_add(requested_bytes::<
                CanonicalDeclarationSource<HirWaitingZoneKey>,
            >(control_counts.waiting_zones))
            .saturating_add(requested_bytes::<usize>(
                control_counts
                    .stop_lines
                    .saturating_add(junction_counts.maneuver_paths)
                    .saturating_mul(2),
            ))
            .saturating_add(requested_bytes::<Option<HirStopLineKey>>(lane_edge_count))
            .saturating_add(requested_bytes::<u8>(
                control_counts
                    .stop_lines
                    .saturating_add(junction_counts.maneuver_paths)
                    .saturating_add(lane_edge_reference_count),
            ))
            .saturating_add(requested_bytes::<HirManeuverGateKey>(
                control_counts.maneuver_gates.saturating_mul(2),
            ))
            .saturating_add(requested_bytes::<HirWaitingZoneKey>(
                control_counts.waiting_zones,
            ))
    };
    let route_scratch = if route_counts.static_routes == 0 {
        0
    } else {
        // 路线编译同时持有全局候选索引、按全部 LaneEdge 建立的角色索引，以及当前
        // 单条路线的局部输出；这里按这些集合真实的同时存续关系计算峰值。
        requested_bytes::<CanonicalDeclarationSource<HirStaticRouteKey>>(route_counts.static_routes)
            .saturating_add(requested_bytes::<Option<HirManeuverPathKey>>(
                lane_edge_count,
            ))
            .saturating_add(requested_bytes::<Option<usize>>(lane_edge_count))
            .saturating_add(requested_bytes::<(
                HirLaneEdgeKey,
                HirLaneEdgeKey,
                HirManeuverPathKey,
            )>(junction_counts.maneuver_paths))
            .saturating_add(requested_bytes::<HirStaticRouteEdge>(
                route_counts.largest_route_edges,
            ))
            .saturating_add(requested_bytes::<HirStaticRouteTransition>(
                route_counts.largest_route_edges.saturating_sub(1),
            ))
            .saturating_add(requested_bytes::<HirManeuverOccurrence>(
                route_counts.largest_route_edges,
            ))
            .saturating_add(requested_bytes::<HirGateOccurrence>(
                route_counts.largest_route_edges,
            ))
            .saturating_add(requested_bytes::<HirWaitingZoneOccurrence>(
                route_counts.largest_route_edges,
            ))
            .saturating_add(requested_bytes::<Option<HirManeuverPathKey>>(
                route_counts.largest_route_edges,
            ))
    };
    let signal_scratch = if signal_counts.entity_count() == 0 {
        0
    } else {
        requested_bytes::<CanonicalDeclarationSource<HirSignalGroupKey>>(signal_counts.groups)
            .saturating_add(requested_bytes::<
                CanonicalDeclarationSource<HirSignalControllerKey>,
            >(signal_counts.controllers))
            .saturating_add(requested_bytes::<
                Option<(HirSignalControllerKey, SourceLocation)>,
            >(signal_counts.groups))
            .saturating_add(requested_bytes::<Option<(SignalAspect, SourceLocation)>>(
                signal_counts.groups,
            ))
            .saturating_add(requested_bytes::<usize>(
                signal_counts.groups.saturating_mul(3),
            ))
            .saturating_add(requested_bytes::<HirSignalGroupKey>(
                signal_counts.controller_groups,
            ))
            .saturating_add(requested_bytes::<(HirSignalGroupKey, HirManeuverGateKey)>(
                signal_counts.controlled_gates,
            ))
            .saturating_add(requested_hash_table_bytes::<
                HirSignalGroupKey,
                SourceLocation,
            >(signal_counts.controller_groups))
            .saturating_add(requested_hash_table_bytes::<HirSignalGroupKey, usize>(
                signal_counts.controller_groups,
            ))
            .saturating_add(requested_hash_table_bytes::<Arc<str>, SourceLocation>(
                signal_counts.phases,
            ))
            .saturating_add(requested_bytes::<usize>(unit.declaration_count))
    };
    let parking_scratch = if parking_counts.entity_count() == 0 {
        0
    } else {
        requested_bytes::<CanonicalDeclarationSource<HirParkingAreaKey>>(parking_counts.areas)
            .saturating_add(requested_bytes::<
                CanonicalDeclarationSource<HirParkingSpaceKey>,
            >(parking_counts.spaces))
            .saturating_add(requested_bytes::<bool>(parking_counts.areas))
            .saturating_add(requested_bytes::<(HirParkingAreaKey, HirParkingSpaceKey)>(
                parking_counts.memberships,
            ))
            .saturating_add(requested_bytes::<usize>(unit.declaration_count))
    };
    let spatial_scratch = if spatial_counts.canonical_frames == 0 {
        0
    } else {
        requested_bytes::<usize>(unit.declaration_count).saturating_add(requested_bytes::<
            Option<(HirCanonicalFrameKey, usize)>,
        >(lane_edge_count))
    };
    let access_scratch = if access_counts.entity_count() == 0 {
        0
    } else {
        requested_bytes::<CanonicalDeclarationSource<HirParticipantClassKey>>(
            access_counts.participant_classes,
        )
        .saturating_add(requested_bytes::<
            CanonicalDeclarationSource<HirVehicleProfileKey>,
        >(access_counts.vehicle_profiles))
        .saturating_add(requested_bytes::<
            CanonicalDeclarationSource<HirAccessRuleKey>,
        >(access_counts.access_rules))
        .saturating_add(requested_bytes::<Option<HirParticipantClassKey>>(
            access_counts.participant_classes.saturating_mul(2),
        ))
        .saturating_add(requested_bytes::<u8>(access_counts.participant_classes))
        .saturating_add(requested_bytes::<(HirParticipantClassKey, bool)>(
            access_counts.participant_classes.saturating_mul(2),
        ))
        .saturating_add(requested_bytes::<HirAccessRuleParticipantClass>(
            access_counts.rule_class_references,
        ))
        .saturating_add(requested_bytes::<AccessCandidate>(
            access_counts.rule_class_references,
        ))
        .saturating_add(requested_bytes::<usize>(unit.declaration_count))
    };
    let (canonical_identity_bytes, largest_canonical_identity_bytes) = identity_byte_counts(unit);
    let stage_scratch_bytes = canonical_source_scratch
        .max(cross_section_scratch)
        .max(junction_scratch)
        .max(control_scratch)
        .max(signal_scratch)
        .max(parking_scratch)
        .max(spatial_scratch)
        .max(access_scratch)
        .max(route_scratch)
        .max(import_sort_scratch)
        .max(largest_canonical_identity_bytes);
    let hir_persistent_bytes = requested_bytes::<HirModule>(module_count)
        .saturating_add(requested_bytes::<HirImport>(unit.import_edge_count))
        .saturating_add(requested_bytes::<HirLaneEdge>(lane_edge_count))
        .saturating_add(requested_bytes::<HirLaneEdgeReference>(
            lane_edge_reference_count,
        ))
        .saturating_add(requested_bytes::<HirRoadCorridor>(
            cross_section_counts.road_corridors,
        ))
        .saturating_add(requested_bytes::<HirCorridorElement>(
            cross_section_counts.corridor_elements,
        ))
        .saturating_add(requested_bytes::<HirRoadSection>(
            cross_section_counts.road_sections,
        ))
        .saturating_add(requested_bytes::<HirAuthoringLane>(
            cross_section_counts.authoring_lanes,
        ))
        .saturating_add(requested_bytes::<HirAuthoringLaneEdge>(
            cross_section_counts.authoring_lane_edges,
        ))
        .saturating_add(requested_bytes::<HirLaneGroup>(
            cross_section_counts.lane_groups,
        ))
        .saturating_add(requested_bytes::<HirLaneGroupMember>(
            cross_section_counts.authoring_lanes,
        ))
        .saturating_add(requested_bytes::<HirFacilityBand>(
            cross_section_counts.facility_bands,
        ))
        .saturating_add(requested_bytes::<HirJunction>(junction_counts.junctions))
        .saturating_add(requested_bytes::<HirMovement>(junction_counts.movements))
        .saturating_add(requested_bytes::<HirJunctionMovement>(
            junction_counts.movements,
        ))
        .saturating_add(requested_bytes::<HirManeuverPath>(
            junction_counts.maneuver_paths,
        ))
        .saturating_add(requested_bytes::<HirMovementManeuverPath>(
            junction_counts.maneuver_paths,
        ))
        .saturating_add(requested_bytes::<HirManeuverPathEdge>(
            junction_counts.maneuver_path_edges,
        ))
        .saturating_add(requested_bytes::<HirJunctionInternalEdge>(
            lane_edge_count.min(junction_counts.maneuver_path_edges),
        ))
        .saturating_add(requested_bytes::<HirStopLine>(control_counts.stop_lines))
        .saturating_add(requested_bytes::<HirManeuverGate>(
            control_counts.maneuver_gates,
        ))
        .saturating_add(requested_bytes::<HirWaitingZone>(
            control_counts.waiting_zones,
        ))
        .saturating_add(requested_bytes::<HirManeuverPathGate>(
            control_counts.maneuver_gates,
        ))
        .saturating_add(requested_bytes::<HirManeuverPathWaitingZone>(
            control_counts.waiting_zones,
        ))
        .saturating_add(requested_bytes::<HirStopLineManeuverGate>(
            control_counts.maneuver_gates,
        ))
        .saturating_add(requested_bytes::<HirSignalGroup>(signal_counts.groups))
        .saturating_add(requested_bytes::<HirSignalController>(
            signal_counts.controllers,
        ))
        .saturating_add(requested_bytes::<HirSignalControllerGroup>(
            signal_counts.controller_groups,
        ))
        .saturating_add(requested_bytes::<HirSignalPhase>(signal_counts.phases))
        .saturating_add(requested_bytes::<HirSignalPhaseState>(
            signal_counts.phase_states,
        ))
        .saturating_add(requested_bytes::<HirSignalGroupManeuverGate>(
            signal_counts.controlled_gates,
        ))
        .saturating_add(requested_bytes::<HirParkingArea>(parking_counts.areas))
        .saturating_add(requested_bytes::<HirParkingSpace>(parking_counts.spaces))
        .saturating_add(requested_bytes::<HirParkingAreaSpace>(
            parking_counts.memberships,
        ))
        .saturating_add(requested_bytes::<HirCanonicalFrame>(
            spatial_counts.canonical_frames,
        ))
        .saturating_add(requested_bytes::<HirLaneEdgeGeometry>(
            spatial_counts.lane_edge_geometries,
        ))
        .saturating_add(requested_bytes::<HirCanonicalPoint3F32>(
            spatial_counts.canonical_points,
        ))
        .saturating_add(requested_bytes::<HirSpatialSegment>(
            spatial_counts.spatial_segments,
        ))
        .saturating_add(requested_bytes::<HirParticipantClass>(
            access_counts.participant_classes,
        ))
        .saturating_add(requested_bytes::<HirVehicleProfile>(
            access_counts.vehicle_profiles,
        ))
        .saturating_add(requested_bytes::<HirAccessRule>(access_counts.access_rules))
        .saturating_add(requested_bytes::<HirAccessRuleParticipantClass>(
            access_counts.rule_class_references,
        ))
        .saturating_add(requested_bytes::<HirStaticRoute>(
            route_counts.static_routes,
        ))
        .saturating_add(requested_bytes::<HirStaticRouteEdge>(
            route_counts.route_edges,
        ))
        .saturating_add(requested_bytes::<HirStaticRouteTransition>(
            route_counts.route_transitions,
        ))
        .saturating_add(requested_bytes::<HirManeuverOccurrence>(
            route_counts.route_edges,
        ))
        .saturating_add(requested_bytes::<HirGateOccurrence>(
            route_counts.route_edges,
        ))
        .saturating_add(requested_bytes::<HirWaitingZoneOccurrence>(
            route_counts.route_edges,
        ));
    let hir_lookup_bytes = requested_hash_table_bytes::<Arc<str>, HirModuleKey>(module_count)
        .saturating_add(requested_bytes::<HashMap<Arc<str>, HirLaneEdgeKey>>(
            module_count,
        ))
        .saturating_add(requested_bytes::<HashMap<Arc<str>, HirRoadSectionKey>>(
            cross_lookup_module_count,
        ))
        .saturating_add(requested_bytes::<HashMap<Arc<str>, HirLaneGroupKey>>(
            cross_lookup_module_count,
        ))
        .saturating_add(requested_bytes::<HashMap<Arc<str>, HirFacilityBandKey>>(
            cross_lookup_module_count,
        ))
        .saturating_add(requested_hash_table_bytes::<Arc<str>, HirLaneEdgeKey>(
            lane_edge_count,
        ))
        .saturating_add(requested_hash_table_bytes::<Arc<str>, HirRoadSectionKey>(
            cross_section_counts.road_sections,
        ))
        .saturating_add(requested_hash_table_bytes::<Arc<str>, HirLaneGroupKey>(
            cross_section_counts.lane_groups,
        ))
        .saturating_add(requested_hash_table_bytes::<Arc<str>, HirFacilityBandKey>(
            cross_section_counts.facility_bands,
        ))
        .saturating_add(requested_bytes::<HashMap<Arc<str>, HirJunctionKey>>(
            junction_lookup_module_count,
        ))
        .saturating_add(requested_bytes::<HashMap<Arc<str>, HirMovementKey>>(
            junction_lookup_module_count,
        ))
        .saturating_add(requested_hash_table_bytes::<Arc<str>, HirJunctionKey>(
            junction_counts.junctions,
        ))
        .saturating_add(requested_hash_table_bytes::<Arc<str>, HirMovementKey>(
            junction_counts.movements,
        ))
        .saturating_add(requested_bytes::<HashMap<Arc<str>, HirManeuverPathKey>>(
            control_lookup_module_count,
        ))
        .saturating_add(requested_bytes::<HashMap<Arc<str>, HirStopLineKey>>(
            control_lookup_module_count,
        ))
        .saturating_add(requested_bytes::<HashMap<Arc<str>, HirManeuverGateKey>>(
            control_lookup_module_count,
        ))
        .saturating_add(requested_hash_table_bytes::<Arc<str>, HirManeuverPathKey>(
            junction_counts.maneuver_paths,
        ))
        .saturating_add(requested_hash_table_bytes::<Arc<str>, HirStopLineKey>(
            control_counts.stop_lines,
        ))
        .saturating_add(requested_hash_table_bytes::<Arc<str>, HirManeuverGateKey>(
            control_counts.maneuver_gates,
        ))
        .saturating_add(requested_bytes::<HashMap<Arc<str>, HirSignalGroupKey>>(
            signal_lookup_module_count,
        ))
        .saturating_add(requested_hash_table_bytes::<Arc<str>, HirSignalGroupKey>(
            signal_counts.groups,
        ))
        .saturating_add(requested_bytes::<HashMap<Arc<str>, HirParkingAreaKey>>(
            parking_lookup_module_count,
        ))
        .saturating_add(requested_hash_table_bytes::<Arc<str>, HirParkingAreaKey>(
            parking_counts.areas,
        ))
        .saturating_add(
            requested_bytes::<HashMap<Arc<str>, HirParticipantClassKey>>(
                access_lookup_module_count,
            ),
        )
        .saturating_add(
            requested_hash_table_bytes::<Arc<str>, HirParticipantClassKey>(
                access_counts.participant_classes,
            ),
        )
        .saturating_add(requested_hash_table_bytes::<
            StableId128,
            RegisteredCanonicalIdentity,
        >(unit.declaration_count))
        .saturating_add(canonical_identity_bytes);
    let controlled_live_bytes = unit
        .controlled_live_bytes
        .saturating_add(hir_persistent_bytes)
        .saturating_add(hir_lookup_bytes)
        .saturating_add(stage_scratch_bytes);

    let primary_span = unit
        .modules
        .first()
        .map(|module| module.declaration_span().clone());
    let stable_key = unit
        .modules
        .first()
        .map(|module| module.descriptor().authoring_namespace_id().into());
    let mut limit_diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
    for (dimension, observed) in [
        (CompileLimitDimension::HirRecordCount, hir_record_count),
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
            limit_diagnostics.push(Diagnostic::compile_limit_exceeded_at(
                dimension,
                unit.limits.value(dimension),
                observed,
                primary_span.clone(),
                stable_key.clone(),
            ));
        }
    }
    if !limit_diagnostics.is_empty() {
        return Err(limit_diagnostics.finish());
    }

    let module_capacity = unit.modules.len();
    let import_capacity = count_to_usize(unit.import_edge_count, &unit.limits)?;
    let lane_edge_capacity = count_to_usize(lane_edge_count, &unit.limits)?;
    let reference_capacity = count_to_usize(lane_edge_reference_count, &unit.limits)?;
    // 第一阶段冻结模块键。CompilationUnit 已按依赖优先排序，因此 raw key 顺序可直接
    // 作为后续规范模块轴；module_lookup 只用于解析，不参与任何输出遍历。
    let mut modules = TypedArena::<HirModuleTag, HirModule>::with_capacity(module_capacity);
    let mut module_lookup = HashMap::with_capacity(module_capacity);
    for source_module in &unit.modules {
        let key = modules
            .push(HirModule {
                authoring_namespace_id: source_module.descriptor().authoring_namespace_arc(),
                imports: TableRange::empty(),
                source_span: source_module.declaration_span().clone(),
            })
            .map_err(|overflow| arena_overflow(overflow, &unit.limits, primary_span.clone()))?;
        module_lookup.insert(source_module.descriptor().authoring_namespace_arc(), key);
    }

    // 每个模块的导入单独按目标命名空间排序后追加到一个平坦表，TableRange 保留模块
    // 边界并避免每模块 Vec 的额外分配。
    let mut imports = Vec::with_capacity(import_capacity);
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key =
            HirModuleKey::from_raw(u32::try_from(module_index).map_err(|_| {
                arena_overflow(ArenaKeyOverflow, &unit.limits, primary_span.clone())
            })?);
        let start = imports.len();
        let mut canonical_imports: Vec<_> = source_module.import_records().collect();
        canonical_imports.sort_unstable_by(|left, right| left.0.cmp(right.0));
        for (target_namespace, source_span) in canonical_imports {
            let target = module_lookup[target_namespace];
            imports.push(HirImport {
                target,
                source_span: source_span.clone(),
            });
        }
        modules.get_mut(module_key).imports =
            TableRange::try_from_usize(start, imports.len().saturating_sub(start))
                .map_err(|overflow| arena_overflow(overflow, &unit.limits, primary_span.clone()))?;
    }

    // 先按 `(canonical module order, stable key)` 为全部声明分配键并建立完整符号表。
    // 这一步必须先于连接解析，才能让前向引用、自环和跨模块引用具有相同语义。
    let mut lane_edges =
        TypedArena::<HirLaneEdgeTag, HirLaneEdge>::with_capacity(lane_edge_capacity);
    let mut symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, TypedAstDeclaration::LaneEdge(_)))
            .count()
    }));
    let mut identities =
        IdentityRegistry::with_capacity(count_to_usize(unit.declaration_count, &unit.limits)?);
    let mut canonical_sources = Vec::with_capacity(lane_edge_capacity);
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key =
            HirModuleKey::from_raw(u32::try_from(module_index).map_err(|_| {
                arena_overflow(ArenaKeyOverflow, &unit.limits, primary_span.clone())
            })?);
        let mut declaration_indices: Vec<usize> = (0..source_module.declarations.len()).collect();
        declaration_indices.retain(|index| {
            matches!(
                source_module.declarations[*index],
                TypedAstDeclaration::LaneEdge(_)
            )
        });
        declaration_indices.sort_unstable_by(|left, right| {
            lane_edge_declaration(&source_module.declarations[*left])
                .expect("filtered declaration must be LaneEdge")
                .header
                .stable_key
                .cmp(
                    &lane_edge_declaration(&source_module.declarations[*right])
                        .expect("filtered declaration must be LaneEdge")
                        .header
                        .stable_key,
                )
        });
        for declaration_index in declaration_indices {
            let source = lane_edge_declaration(&source_module.declarations[declaration_index])
                .expect("filtered declaration must be LaneEdge");
            let fields = [
                IdentityFieldInput::new(
                    FieldTag::AuthoringNamespaceId,
                    source_module
                        .descriptor()
                        .authoring_namespace_id()
                        .as_bytes(),
                ),
                IdentityFieldInput::new(FieldTag::LaneEdgeKey, source.header.stable_key.as_bytes()),
            ];
            let identity = encode_canonical_identity(
                EntityKind::LaneEdge,
                &fields,
                unit.limits.value(CompileLimitDimension::SingleStringBytes),
            )
            .map_err(|violation| {
                let mut diagnostic = Diagnostic::invalid_canonical_identity(
                    EntityKind::LaneEdge,
                    &source.header.stable_key,
                    violation,
                    source.header.span.clone(),
                );
                diagnostic
                    .set_canonical_module_order(u32::try_from(module_index).unwrap_or(u32::MAX));
                DiagnosticBundle::single(diagnostic)
            })?;
            if let Err(error) = identities.register(&identity, &source.header.span) {
                let mut diagnostic = match error {
                    IdentityRegistrationError::Duplicate { existing_span } => {
                        Diagnostic::duplicate_canonical_identity(
                            identity.kind(),
                            &source.header.stable_key,
                            identity.stable_id(),
                            source.header.span.clone(),
                            existing_span,
                        )
                    }
                    IdentityRegistrationError::DigestCollision { existing_span } => {
                        Diagnostic::identity_digest_collision(
                            identity.kind(),
                            &source.header.stable_key,
                            identity.stable_id(),
                            source.header.span.clone(),
                            existing_span,
                        )
                    }
                };
                diagnostic
                    .set_canonical_module_order(u32::try_from(module_index).unwrap_or(u32::MAX));
                return Err(DiagnosticBundle::single(diagnostic));
            }
            let key = lane_edges
                .push(HirLaneEdge {
                    module: module_key,
                    stable_key: Arc::clone(&source.header.stable_key),
                    stable_id: LaneEdgeId::from_untyped(identity.stable_id()),
                    length_meters: source.length.value(),
                    speed_limit_meters_per_second: source.speed_limit.value(),
                    successors: TableRange::empty(),
                    source_span: source.header.span.clone(),
                })
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                })?;
            symbols.insert(module_key, Arc::clone(&source.header.stable_key), key);
            canonical_sources.push(CanonicalLaneEdgeSource {
                source_module_index: u32::try_from(module_index).map_err(|_| {
                    arena_overflow(
                        ArenaKeyOverflow,
                        &unit.limits,
                        Some(source.header.span.clone()),
                    )
                })?,
                declaration_index: u32::try_from(declaration_index).map_err(|_| {
                    arena_overflow(
                        ArenaKeyOverflow,
                        &unit.limits,
                        Some(source.header.span.clone()),
                    )
                })?,
                hir_key: key,
            });
        }
    }
    // 第二遍只解析已经规范化的引用序列。未知目标继续收集到有界诊断集合中；该边的
    // 临时区间不会在失败时泄漏，因为整个 HirUnit 仅在零错误后提交。
    let mut references = Vec::with_capacity(reference_capacity);
    let mut diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
    for source_location in canonical_sources {
        let module_index = usize::try_from(source_location.source_module_index)
            .expect("u32 module index must fit usize on supported targets");
        let source_module = &unit.modules[module_index];
        let source = lane_edge_declaration(
            &source_module.declarations[usize::try_from(source_location.declaration_index)
                .expect("u32 declaration index must fit usize on supported targets")],
        )
        .expect("canonical LaneEdge source must still name a LaneEdge");
        let start = references.len();
        for successor in &source.successors {
            let target_module = module_lookup[successor.module_namespace.as_ref()];
            let Some(target) = symbols.get(target_module, &successor.declaration_key) else {
                let mut diagnostic = Diagnostic::unknown_reference_target(
                    EntityKind::LaneEdge,
                    &source.header.stable_key,
                    &successor.module_namespace,
                    &successor.declaration_key,
                    successor.span.clone(),
                    source.header.span.clone(),
                );
                diagnostic.set_canonical_module_order(source_location.source_module_index);
                diagnostics.push(diagnostic);
                continue;
            };
            references.push(HirLaneEdgeReference {
                target,
                source_span: successor.span.clone(),
            });
        }
        lane_edges.get_mut(source_location.hir_key).successors =
            TableRange::try_from_usize(start, references.len().saturating_sub(start)).map_err(
                |overflow| arena_overflow(overflow, &unit.limits, Some(source.header.span.clone())),
            )?;
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    let cross_section = build_cross_section_hir(
        unit,
        &module_lookup,
        &lane_edges,
        &references,
        &symbols,
        &mut identities,
    )?;
    let mut junction = build_junction_hir(
        unit,
        &module_lookup,
        &lane_edges,
        &references,
        &symbols,
        &mut identities,
    )?;
    let mut control = build_control_hir(
        unit,
        &module_lookup,
        &lane_edges,
        &references,
        &symbols,
        &mut junction.maneuver_paths,
        &junction.maneuver_path_edges,
        &mut identities,
    )?;
    let signal = build_signal_hir(
        unit,
        &module_lookup,
        &mut control.maneuver_gates,
        &mut identities,
    )?;
    let parking = build_parking_hir(unit, &module_lookup, &lane_edges, &symbols, &mut identities)?;
    let spatial = build_spatial_hir(
        unit,
        &module_lookup,
        &lane_edges,
        &references,
        &symbols,
        &mut identities,
    )?;
    let access = build_access_hir(
        unit,
        &module_lookup,
        &lane_edges,
        &cross_section,
        &junction.maneuver_paths,
        &mut identities,
    )?;
    let route = build_route_hir(
        unit,
        &module_lookup,
        &lane_edges,
        &references,
        &symbols,
        &junction.maneuver_paths,
        &junction.maneuver_path_edges,
        &junction.junction_internal_edges,
        &control.stop_lines,
        &control.maneuver_gates,
        &control.waiting_zones,
        &control.maneuver_path_gates,
        &control.maneuver_path_waiting_zones,
        &mut identities,
    )?;
    // 完整规范前像只服务本阶段的重复/碰撞判断。此后各表仅保留 16 字节有类型 ID，
    // 避免在 HIR 与 MIR 中复制可由稳定键和父项重建的 identity envelope。
    drop(identities);

    debug_assert_eq!(modules.len(), module_capacity);
    debug_assert_eq!(lane_edges.len(), lane_edge_capacity);
    Ok(HirUnit {
        modules: modules.into_boxed_slice(),
        imports: imports.into_boxed_slice(),
        lane_edges: lane_edges.into_boxed_slice(),
        lane_edge_references: references.into_boxed_slice(),
        road_corridors: cross_section.road_corridors,
        corridor_elements: cross_section.corridor_elements,
        road_sections: cross_section.road_sections,
        authoring_lanes: cross_section.authoring_lanes,
        authoring_lane_edges: cross_section.authoring_lane_edges,
        lane_groups: cross_section.lane_groups,
        lane_group_members: cross_section.lane_group_members,
        facility_bands: cross_section.facility_bands,
        junctions: junction.junctions,
        movements: junction.movements,
        junction_movements: junction.junction_movements,
        maneuver_paths: junction.maneuver_paths,
        movement_maneuver_paths: junction.movement_maneuver_paths,
        maneuver_path_edges: junction.maneuver_path_edges,
        junction_internal_edges: junction.junction_internal_edges,
        stop_lines: control.stop_lines,
        maneuver_gates: control.maneuver_gates,
        waiting_zones: control.waiting_zones,
        maneuver_path_gates: control.maneuver_path_gates,
        maneuver_path_waiting_zones: control.maneuver_path_waiting_zones,
        stop_line_maneuver_gates: control.stop_line_maneuver_gates,
        signal_groups: signal.signal_groups,
        signal_controllers: signal.signal_controllers,
        signal_controller_groups: signal.signal_controller_groups,
        signal_phases: signal.signal_phases,
        signal_phase_states: signal.signal_phase_states,
        signal_group_maneuver_gates: signal.signal_group_maneuver_gates,
        parking_areas: parking.parking_areas,
        parking_spaces: parking.parking_spaces,
        parking_area_spaces: parking.parking_area_spaces,
        canonical_frames: spatial.canonical_frames,
        lane_edge_geometries: spatial.lane_edge_geometries,
        canonical_points: spatial.canonical_points,
        spatial_segments: spatial.spatial_segments,
        participant_classes: access.participant_classes,
        vehicle_profiles: access.vehicle_profiles,
        access_rules: access.access_rules,
        access_rule_participant_classes: access.access_rule_participant_classes,
        static_routes: route.static_routes,
        static_route_edges: route.static_route_edges,
        static_route_transitions: route.static_route_transitions,
        maneuver_occurrences: route.maneuver_occurrences,
        gate_occurrences: route.gate_occurrences,
        waiting_zone_occurrences: route.waiting_zone_occurrences,
        hir_record_count,
        controlled_live_bytes: hir_persistent_bytes,
        peak_controlled_live_bytes: controlled_live_bytes,
    })
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
        for declaration in &module.declarations {
            declaration.try_visit_source_locations(|span| {
                unit.resolve_source_document_for_module(module_ordinal, span)
                    .map(|_| ())
            })?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn build_cross_section_hir(
    unit: &CompilationUnit,
    module_lookup: &HashMap<Arc<str>, HirModuleKey>,
    lane_edges: &TypedArena<HirLaneEdgeTag, HirLaneEdge>,
    lane_edge_references: &[HirLaneEdgeReference],
    lane_edge_symbols: &SymbolTable<HirLaneEdgeKey>,
    identities: &mut IdentityRegistry,
) -> Result<CrossSectionHir, DiagnosticBundle> {
    let counts = cross_section_counts(unit);
    if counts.entity_count() == 0 {
        return Ok(CrossSectionHir::default());
    }
    // 只为会被引用解析访问的实体建立符号表，并按实体类别精确预留容量。RoadCorridor
    // 与 AuthoringLane 在本切片中没有按键引用消费者；为它们建立查找表只会增加峰值内存。
    let mut section_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, TypedAstDeclaration::RoadSection(_)))
            .count()
    }));
    let mut group_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, TypedAstDeclaration::LaneGroup(_)))
            .count()
    }));
    let mut band_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, TypedAstDeclaration::FacilityBand(_)))
            .count()
    }));

    let mut corridors = TypedArena::<HirRoadCorridorTag, HirRoadCorridor>::with_capacity(
        count_to_usize(counts.road_corridors, &unit.limits)?,
    );
    let mut sections = TypedArena::<HirRoadSectionTag, HirRoadSection>::with_capacity(
        count_to_usize(counts.road_sections, &unit.limits)?,
    );
    let mut lanes = TypedArena::<HirAuthoringLaneTag, HirAuthoringLane>::with_capacity(
        count_to_usize(counts.authoring_lanes, &unit.limits)?,
    );
    let mut groups = TypedArena::<HirLaneGroupTag, HirLaneGroup>::with_capacity(count_to_usize(
        counts.lane_groups,
        &unit.limits,
    )?);
    let mut bands = TypedArena::<HirFacilityBandTag, HirFacilityBand>::with_capacity(
        count_to_usize(counts.facility_bands, &unit.limits)?,
    );
    let mut corridor_sources = Vec::with_capacity(corridors_capacity(&counts, &unit.limits)?);
    let mut section_sources = Vec::with_capacity(sections_capacity(&counts, &unit.limits)?);
    let mut lane_sources = Vec::with_capacity(lanes_capacity(&counts, &unit.limits)?);
    let mut group_sources = Vec::with_capacity(groups_capacity(&counts, &unit.limits)?);
    let mut band_sources = Vec::with_capacity(bands_capacity(&counts, &unit.limits)?);

    // 首遍只登记符号与不依赖父项的 RoadCorridor identity。其余实体先保留零值占位，
    // 但在所有者/引用错误存在时不会逃逸出本函数；父项闭合后才写入真实 ID。
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key = HirModuleKey::from_raw(
            u32::try_from(module_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        let mut declaration_indices: Vec<_> = source_module
            .declarations
            .iter()
            .enumerate()
            .filter_map(|(index, declaration)| {
                matches!(
                    declaration,
                    TypedAstDeclaration::RoadCorridor(_)
                        | TypedAstDeclaration::RoadSection(_)
                        | TypedAstDeclaration::LaneGroup(_)
                        | TypedAstDeclaration::FacilityBand(_)
                )
                .then_some(index)
            })
            .collect();
        declaration_indices.sort_unstable_by(|left, right| {
            let left = declaration_header(&source_module.declarations[*left]);
            let right = declaration_header(&source_module.declarations[*right]);
            (left.entity_kind.code(), left.stable_key.as_bytes())
                .cmp(&(right.entity_kind.code(), right.stable_key.as_bytes()))
        });
        for source_declaration_index in declaration_indices {
            let source_module_index = u32::try_from(module_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?;
            let declaration_index = u32::try_from(source_declaration_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?;
            match &source_module.declarations[source_declaration_index] {
                TypedAstDeclaration::LaneEdge(_) => {
                    unreachable!("cross-section source filter admitted LaneEdge")
                }
                TypedAstDeclaration::RoadCorridor(source) => {
                    let fields = [
                        IdentityFieldInput::new(
                            FieldTag::AuthoringNamespaceId,
                            source_module
                                .descriptor()
                                .authoring_namespace_id()
                                .as_bytes(),
                        ),
                        IdentityFieldInput::new(
                            FieldTag::CorridorKey,
                            source.header.stable_key.as_bytes(),
                        ),
                    ];
                    let stable_id = RoadCorridorId::from_untyped(derive_identity(
                        unit,
                        identities,
                        module_index,
                        EntityKind::RoadCorridor,
                        &source.header.stable_key,
                        &source.header.span,
                        &fields,
                    )?);
                    let key = corridors
                        .push(HirRoadCorridor {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            stable_id,
                            reference_section: HirRoadSectionKey::from_raw(0),
                            elements: TableRange::empty(),
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    corridor_sources.push(CanonicalDeclarationSource {
                        source_module_index,
                        declaration_index,
                        hir_key: key,
                    });
                }
                TypedAstDeclaration::RoadSection(source) => {
                    let lane_start = lanes.len();
                    let section_key = sections
                        .push(HirRoadSection {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            stable_id: RoadSectionId::from_untyped(StableId128::ZERO),
                            road_corridor: HirRoadCorridorKey::from_raw(0),
                            kind_id: Arc::clone(&source.kind_id),
                            lanes: TableRange::empty(),
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    section_symbols.insert(
                        module_key,
                        Arc::clone(&source.header.stable_key),
                        section_key,
                    );
                    section_sources.push(CanonicalDeclarationSource {
                        source_module_index,
                        declaration_index,
                        hir_key: section_key,
                    });
                    for (lane_index, lane) in source.lanes.iter().enumerate() {
                        let lane_key = lanes
                            .push(HirAuthoringLane {
                                module: module_key,
                                stable_key: Arc::clone(&lane.header.stable_key),
                                stable_id: AuthoringLaneId::from_untyped(StableId128::ZERO),
                                road_section: section_key,
                                edge_chain: TableRange::empty(),
                                lane_group: None,
                                lane_group_source_location: None,
                                source_span: lane.header.span.clone(),
                            })
                            .map_err(|overflow| {
                                arena_overflow(
                                    overflow,
                                    &unit.limits,
                                    Some(lane.header.span.clone()),
                                )
                            })?;
                        lane_sources.push(CanonicalAuthoringLaneSource {
                            source_module_index,
                            declaration_index,
                            lane_index: u32::try_from(lane_index).map_err(|_| {
                                arena_overflow(
                                    ArenaKeyOverflow,
                                    &unit.limits,
                                    Some(lane.header.span.clone()),
                                )
                            })?,
                            hir_key: lane_key,
                        });
                    }
                    sections.get_mut(section_key).lanes = TableRange::try_from_usize(
                        lane_start,
                        lanes.len().saturating_sub(lane_start),
                    )
                    .map_err(|overflow| {
                        arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                    })?;
                }
                TypedAstDeclaration::LaneGroup(source) => {
                    let key = groups
                        .push(HirLaneGroup {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            stable_id: LaneGroupId::from_untyped(StableId128::ZERO),
                            road_section: HirRoadSectionKey::from_raw(0),
                            members: TableRange::empty(),
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    group_symbols.insert(module_key, Arc::clone(&source.header.stable_key), key);
                    group_sources.push(CanonicalDeclarationSource {
                        source_module_index,
                        declaration_index,
                        hir_key: key,
                    });
                }
                TypedAstDeclaration::FacilityBand(source) => {
                    let key = bands
                        .push(HirFacilityBand {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            stable_id: FacilityBandId::from_untyped(StableId128::ZERO),
                            road_corridor: HirRoadCorridorKey::from_raw(0),
                            kind_id: Arc::clone(&source.kind_id),
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    band_symbols.insert(module_key, Arc::clone(&source.header.stable_key), key);
                    band_sources.push(CanonicalDeclarationSource {
                        source_module_index,
                        declaration_index,
                        hir_key: key,
                    });
                }
                TypedAstDeclaration::Junction(_)
                | TypedAstDeclaration::Movement(_)
                | TypedAstDeclaration::ManeuverPath(_)
                | TypedAstDeclaration::StopLine(_)
                | TypedAstDeclaration::ManeuverGate(_)
                | TypedAstDeclaration::WaitingZone(_)
                | TypedAstDeclaration::StaticRoute(_)
                | TypedAstDeclaration::SignalGroup(_)
                | TypedAstDeclaration::SignalController(_)
                | TypedAstDeclaration::ParkingArea(_)
                | TypedAstDeclaration::ParkingSpace(_)
                | TypedAstDeclaration::ParticipantClass(_)
                | TypedAstDeclaration::VehicleProfile(_)
                | TypedAstDeclaration::CanonicalFrame(_)
                | TypedAstDeclaration::AccessRule(_) => {
                    unreachable!("cross-section source filter admitted junction declaration")
                }
            }
        }
    }

    let mut diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
    let mut corridor_elements =
        Vec::with_capacity(count_to_usize(counts.corridor_elements, &unit.limits)?);
    let mut section_owners: Vec<Option<(HirRoadCorridorKey, SourceLocation)>> =
        vec![None; sections.len()];
    let mut band_owners: Vec<Option<(HirRoadCorridorKey, SourceLocation)>> =
        vec![None; bands.len()];

    for location in &corridor_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let TypedAstDeclaration::RoadCorridor(source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical RoadCorridor source changed kind")
        };
        let reference_section = resolve_reference(
            module_lookup,
            &section_symbols,
            &source.reference_section,
            EntityKind::RoadCorridor,
            &source.header,
            location.source_module_index,
            &mut diagnostics,
        );
        let start = corridor_elements.len();
        let mut reference_is_member = false;
        for element in &source.elements {
            match element {
                OwnedCorridorElementReference::RoadSection(reference) => {
                    if let Some(target) = resolve_reference(
                        module_lookup,
                        &section_symbols,
                        reference,
                        EntityKind::RoadCorridor,
                        &source.header,
                        location.source_module_index,
                        &mut diagnostics,
                    ) {
                        reference_is_member |= reference_section == Some(target);
                        register_owner(
                            EntityKind::RoadSection,
                            target.index(),
                            &sections.get(target).stable_key,
                            location.hir_key,
                            &source.header,
                            &mut section_owners,
                            &corridors,
                            location.source_module_index,
                            &mut diagnostics,
                        );
                        corridor_elements.push(HirCorridorElement::RoadSection {
                            road_section: target,
                            source_location: unit.resolve_source_location_for_module(
                                location.source_module_index,
                                &reference.span,
                            )?,
                        });
                    }
                }
                OwnedCorridorElementReference::FacilityBand(reference) => {
                    if let Some(target) = resolve_reference(
                        module_lookup,
                        &band_symbols,
                        reference,
                        EntityKind::RoadCorridor,
                        &source.header,
                        location.source_module_index,
                        &mut diagnostics,
                    ) {
                        register_owner(
                            EntityKind::FacilityBand,
                            target.index(),
                            &bands.get(target).stable_key,
                            location.hir_key,
                            &source.header,
                            &mut band_owners,
                            &corridors,
                            location.source_module_index,
                            &mut diagnostics,
                        );
                        corridor_elements.push(HirCorridorElement::FacilityBand {
                            facility_band: target,
                            source_location: unit.resolve_source_location_for_module(
                                location.source_module_index,
                                &reference.span,
                            )?,
                        });
                    }
                }
            }
        }
        if let Some(reference_section) = reference_section {
            corridors.get_mut(location.hir_key).reference_section = reference_section;
            if !reference_is_member {
                let mut diagnostic = Diagnostic::invalid_corridor_reference_section(
                    &source.header.stable_key,
                    &source.reference_section.module_namespace,
                    &source.reference_section.declaration_key,
                    source.reference_section.span.clone(),
                    source.header.span.clone(),
                );
                diagnostic.set_canonical_module_order(location.source_module_index);
                diagnostics.push(diagnostic);
            }
        }
        corridors.get_mut(location.hir_key).elements =
            TableRange::try_from_usize(start, corridor_elements.len().saturating_sub(start))
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                })?;
    }

    for (key, section) in sections.iter() {
        if section_owners[key.index()].is_none() {
            let mut diagnostic = Diagnostic::missing_cross_section_owner(
                EntityKind::RoadSection,
                &section.stable_key,
                section.source_span.clone(),
            );
            diagnostic.set_canonical_module_order(section.module.raw());
            diagnostics.push(diagnostic);
        }
    }
    for (key, band) in bands.iter() {
        if band_owners[key.index()].is_none() {
            let mut diagnostic = Diagnostic::missing_cross_section_owner(
                EntityKind::FacilityBand,
                &band.stable_key,
                band.source_span.clone(),
            );
            diagnostic.set_canonical_module_order(band.module.raw());
            diagnostics.push(diagnostic);
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    // 父走廊已唯一闭合，此时才派生 RoadSection / FacilityBand identity。
    for location in &section_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let TypedAstDeclaration::RoadSection(source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical RoadSection source changed kind")
        };
        let owner = section_owners[location.hir_key.index()]
            .as_ref()
            .expect("owner diagnostics already rejected missing sections")
            .0;
        let owner_id = corridors.get(owner).stable_id;
        let fields = [
            IdentityFieldInput::new(
                FieldTag::AuthoringNamespaceId,
                source_module
                    .descriptor()
                    .authoring_namespace_id()
                    .as_bytes(),
            ),
            IdentityFieldInput::new(FieldTag::SectionKey, source.header.stable_key.as_bytes()),
            IdentityFieldInput::new(
                FieldTag::RoadCorridorStableId,
                owner_id.as_untyped().as_bytes(),
            ),
        ];
        let stable_id = RoadSectionId::from_untyped(derive_identity(
            unit,
            identities,
            location.source_module_index as usize,
            EntityKind::RoadSection,
            &source.header.stable_key,
            &source.header.span,
            &fields,
        )?);
        let section = sections.get_mut(location.hir_key);
        section.road_corridor = owner;
        section.stable_id = stable_id;
    }
    for location in &band_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let TypedAstDeclaration::FacilityBand(source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical FacilityBand source changed kind")
        };
        let owner = band_owners[location.hir_key.index()]
            .as_ref()
            .expect("owner diagnostics already rejected missing bands")
            .0;
        let owner_id = corridors.get(owner).stable_id;
        let fields = [
            IdentityFieldInput::new(
                FieldTag::AuthoringNamespaceId,
                source_module
                    .descriptor()
                    .authoring_namespace_id()
                    .as_bytes(),
            ),
            IdentityFieldInput::new(
                FieldTag::FacilityBandKey,
                source.header.stable_key.as_bytes(),
            ),
            IdentityFieldInput::new(
                FieldTag::RoadCorridorStableId,
                owner_id.as_untyped().as_bytes(),
            ),
        ];
        let stable_id = FacilityBandId::from_untyped(derive_identity(
            unit,
            identities,
            location.source_module_index as usize,
            EntityKind::FacilityBand,
            &source.header.stable_key,
            &source.header.span,
            &fields,
        )?);
        let band = bands.get_mut(location.hir_key);
        band.road_corridor = owner;
        band.stable_id = stable_id;
    }

    // LaneGroup 的父区段是其 identity 输入，必须先解析再处理引用它的编制车道。
    for location in &group_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let TypedAstDeclaration::LaneGroup(source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical LaneGroup source changed kind")
        };
        let Some(parent) = resolve_reference(
            module_lookup,
            &section_symbols,
            &source.road_section,
            EntityKind::LaneGroup,
            &source.header,
            location.source_module_index,
            &mut diagnostics,
        ) else {
            continue;
        };
        let parent_id = sections.get(parent).stable_id;
        let fields = [
            IdentityFieldInput::new(
                FieldTag::AuthoringNamespaceId,
                source_module
                    .descriptor()
                    .authoring_namespace_id()
                    .as_bytes(),
            ),
            IdentityFieldInput::new(FieldTag::LaneGroupKey, source.header.stable_key.as_bytes()),
            IdentityFieldInput::new(
                FieldTag::RoadSectionStableId,
                parent_id.as_untyped().as_bytes(),
            ),
        ];
        let stable_id = LaneGroupId::from_untyped(derive_identity(
            unit,
            identities,
            location.source_module_index as usize,
            EntityKind::LaneGroup,
            &source.header.stable_key,
            &source.header.span,
            &fields,
        )?);
        let group = groups.get_mut(location.hir_key);
        group.road_section = parent;
        group.stable_id = stable_id;
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    let mut lane_edges_flat =
        Vec::with_capacity(count_to_usize(counts.authoring_lane_edges, &unit.limits)?);
    let mut edge_owners: Vec<Option<HirAuthoringLaneKey>> = vec![None; lane_edges.len()];
    let mut group_member_counts = vec![0_usize; groups.len()];
    for location in &lane_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let TypedAstDeclaration::RoadSection(section_source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical AuthoringLane source parent changed kind")
        };
        let lane_source = &section_source.lanes[location.lane_index as usize];
        let parent = lanes.get(location.hir_key).road_section;
        let parent_id = sections.get(parent).stable_id;
        let fields = [
            IdentityFieldInput::new(
                FieldTag::AuthoringNamespaceId,
                source_module
                    .descriptor()
                    .authoring_namespace_id()
                    .as_bytes(),
            ),
            IdentityFieldInput::new(FieldTag::LaneKey, lane_source.header.stable_key.as_bytes()),
            IdentityFieldInput::new(
                FieldTag::RoadSectionStableId,
                parent_id.as_untyped().as_bytes(),
            ),
        ];
        let stable_id = AuthoringLaneId::from_untyped(derive_identity(
            unit,
            identities,
            location.source_module_index as usize,
            EntityKind::AuthoringLane,
            &lane_source.header.stable_key,
            &lane_source.header.span,
            &fields,
        )?);
        let start = lane_edges_flat.len();
        let mut predecessor = None;
        for reference in &lane_source.edge_chain {
            let Some(target) = resolve_reference(
                module_lookup,
                lane_edge_symbols,
                reference,
                EntityKind::AuthoringLane,
                &lane_source.header,
                location.source_module_index,
                &mut diagnostics,
            ) else {
                // 未知引用保留自身诊断，但不能把其两侧原本不相邻的边拼接后再检查连通性。
                predecessor = None;
                continue;
            };
            if let Some(first_owner) = edge_owners[target.index()] {
                let mut diagnostic = Diagnostic::multiple_authoring_lane_owners(
                    &lane_edges.get(target).stable_key,
                    &lanes.get(first_owner).stable_key,
                    &lane_source.header.stable_key,
                    reference.span.clone(),
                    lanes.get(first_owner).source_span.clone(),
                );
                diagnostic.set_canonical_module_order(location.source_module_index);
                diagnostics.push(diagnostic);
            } else {
                edge_owners[target.index()] = Some(location.hir_key);
            }
            if let Some((predecessor_key, predecessor_span)) = predecessor {
                let predecessor_record = lane_edges.get(predecessor_key);
                let connected = lane_edge_references
                    [predecessor_record.successors.as_usize_range()]
                .iter()
                .any(|candidate| candidate.target == target);
                if !connected {
                    let mut diagnostic = Diagnostic::disconnected_authoring_lane_edge_chain(
                        &lane_source.header.stable_key,
                        &predecessor_record.stable_key,
                        &lane_edges.get(target).stable_key,
                        reference.span.clone(),
                        predecessor_span,
                    );
                    diagnostic.set_canonical_module_order(location.source_module_index);
                    diagnostics.push(diagnostic);
                }
            }
            predecessor = Some((target, reference.span.clone()));
            lane_edges_flat.push(HirAuthoringLaneEdge {
                target,
                source_span: reference.span.clone(),
            });
        }

        let lane_group = lane_source.lane_group.as_ref().and_then(|reference| {
            resolve_reference(
                module_lookup,
                &group_symbols,
                reference,
                EntityKind::AuthoringLane,
                &lane_source.header,
                location.source_module_index,
                &mut diagnostics,
            )
        });
        if let Some(group_key) = lane_group {
            let group = groups.get(group_key);
            if group.road_section != parent {
                let mut diagnostic = Diagnostic::lane_group_parent_mismatch(
                    &lane_source.header.stable_key,
                    &group.stable_key,
                    &sections.get(parent).stable_key,
                    &sections.get(group.road_section).stable_key,
                    lane_source
                        .lane_group
                        .as_ref()
                        .expect("resolved lane group has source reference")
                        .span
                        .clone(),
                    group.source_span.clone(),
                );
                diagnostic.set_canonical_module_order(location.source_module_index);
                diagnostics.push(diagnostic);
            } else {
                group_member_counts[group_key.index()] =
                    group_member_counts[group_key.index()].saturating_add(1);
            }
        }
        let lane = lanes.get_mut(location.hir_key);
        lane.stable_id = stable_id;
        lane.edge_chain =
            TableRange::try_from_usize(start, lane_edges_flat.len().saturating_sub(start))
                .map_err(|overflow| {
                    arena_overflow(
                        overflow,
                        &unit.limits,
                        Some(lane_source.header.span.clone()),
                    )
                })?;
        lane.lane_group = lane_group;
        lane.lane_group_source_location = match (&lane_source.lane_group, lane_group) {
            (Some(reference), Some(_)) => Some(unit.resolve_source_location_for_module(
                location.source_module_index,
                &reference.span,
            )?),
            _ => None,
        };
    }

    for (group_key, group) in groups.iter() {
        if group_member_counts[group_key.index()] == 0 {
            diagnostics.push(Diagnostic::empty_lane_group(
                &group.stable_key,
                group.source_span.clone(),
            ));
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    // 先按 group key 计算连续范围，再按 lane key 递增顺序填充。这样维持与原车道遍历
    // 一致的成员顺序，同时避免为每个 LaneGroup 单独分配一个临时 Vec。
    let mut next_group_member = Vec::with_capacity(groups.len());
    let mut member_count = 0_usize;
    for (group_index, count) in group_member_counts.iter().copied().enumerate() {
        next_group_member.push(member_count);
        let group_key = HirLaneGroupKey::from_raw(
            u32::try_from(group_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        groups.get_mut(group_key).members = TableRange::try_from_usize(member_count, count)
            .map_err(|overflow| arena_overflow(overflow, &unit.limits, None))?;
        member_count = member_count.saturating_add(count);
    }
    let mut lane_group_members = if member_count == 0 {
        Vec::new()
    } else {
        let first_member = lanes
            .iter()
            .find_map(|(key, lane)| lane.lane_group.map(|_| key))
            .expect("positive validated group member count must name a lane");
        vec![HirLaneGroupMember { lane: first_member }; member_count]
    };
    for (lane_key, lane) in lanes.iter() {
        let Some(group_key) = lane.lane_group else {
            continue;
        };
        let destination = &mut next_group_member[group_key.index()];
        lane_group_members[*destination] = HirLaneGroupMember { lane: lane_key };
        *destination += 1;
    }
    debug_assert!(groups.iter().all(|(key, group)| {
        next_group_member[key.index()] == group.members.as_usize_range().end
    }));

    Ok(CrossSectionHir {
        road_corridors: corridors.into_boxed_slice(),
        corridor_elements: corridor_elements.into_boxed_slice(),
        road_sections: sections.into_boxed_slice(),
        authoring_lanes: lanes.into_boxed_slice(),
        authoring_lane_edges: lane_edges_flat.into_boxed_slice(),
        lane_groups: groups.into_boxed_slice(),
        lane_group_members: lane_group_members.into_boxed_slice(),
        facility_bands: bands.into_boxed_slice(),
    })
}

#[allow(clippy::too_many_lines)]
fn build_junction_hir(
    unit: &CompilationUnit,
    module_lookup: &HashMap<Arc<str>, HirModuleKey>,
    lane_edges: &TypedArena<HirLaneEdgeTag, HirLaneEdge>,
    lane_edge_references: &[HirLaneEdgeReference],
    lane_edge_symbols: &SymbolTable<HirLaneEdgeKey>,
    identities: &mut IdentityRegistry,
) -> Result<JunctionHir, DiagnosticBundle> {
    let counts = junction_counts(unit);
    if counts.entity_count() == 0 {
        return Ok(JunctionHir::default());
    }

    let junction_capacity = count_to_usize(counts.junctions, &unit.limits)?;
    let movement_capacity = count_to_usize(counts.movements, &unit.limits)?;
    let path_capacity = count_to_usize(counts.maneuver_paths, &unit.limits)?;
    let mut junctions = TypedArena::<HirJunctionTag, HirJunction>::with_capacity(junction_capacity);
    let mut movements = TypedArena::<HirMovementTag, HirMovement>::with_capacity(movement_capacity);
    let mut paths = TypedArena::<HirManeuverPathTag, HirManeuverPath>::with_capacity(path_capacity);
    let mut junction_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, TypedAstDeclaration::Junction(_)))
            .count()
    }));
    let mut movement_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, TypedAstDeclaration::Movement(_)))
            .count()
    }));
    let mut movement_sources = Vec::with_capacity(movement_capacity);
    let mut path_sources = Vec::with_capacity(path_capacity);

    // 三种声明都先按模块和稳定键分配完整符号。只有 Junction 不依赖父项，可以立即
    // 派生身份；Movement 与 ManeuverPath 先写入占位值，随后按父项顺序闭合。
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key = HirModuleKey::from_raw(
            u32::try_from(module_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        let module_order = u32::try_from(module_index).unwrap_or(u32::MAX);
        let mut declaration_indices: Vec<_> = (0..source_module.declarations.len()).collect();
        declaration_indices.sort_unstable_by(|left, right| {
            declaration_header(&source_module.declarations[*left])
                .stable_key
                .cmp(&declaration_header(&source_module.declarations[*right]).stable_key)
        });
        for declaration_index in declaration_indices {
            match &source_module.declarations[declaration_index] {
                TypedAstDeclaration::Junction(source) => {
                    let fields = [
                        IdentityFieldInput::new(
                            FieldTag::AuthoringNamespaceId,
                            source_module
                                .descriptor()
                                .authoring_namespace_id()
                                .as_bytes(),
                        ),
                        IdentityFieldInput::new(
                            FieldTag::JunctionKey,
                            source.header.stable_key.as_bytes(),
                        ),
                    ];
                    let stable_id = JunctionId::from_untyped(derive_identity(
                        unit,
                        identities,
                        module_index,
                        EntityKind::Junction,
                        &source.header.stable_key,
                        &source.header.span,
                        &fields,
                    )?);
                    let key = junctions
                        .push(HirJunction {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            stable_id,
                            movements: TableRange::empty(),
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    junction_symbols.insert(module_key, Arc::clone(&source.header.stable_key), key);
                }
                TypedAstDeclaration::Movement(source) => {
                    let key = movements
                        .push(HirMovement {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            stable_id: MovementId::from_untyped(StableId128::ZERO),
                            junction: HirJunctionKey::from_raw(0),
                            junction_source_location: None,
                            directed_entry_approach_key: Arc::clone(
                                &source.directed_entry_approach_key,
                            ),
                            directed_exit_approach_key: Arc::clone(
                                &source.directed_exit_approach_key,
                            ),
                            maneuver_paths: TableRange::empty(),
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    movement_symbols.insert(module_key, Arc::clone(&source.header.stable_key), key);
                    movement_sources.push(CanonicalDeclarationSource {
                        source_module_index: module_order,
                        declaration_index: u32::try_from(declaration_index).map_err(|_| {
                            arena_overflow(
                                ArenaKeyOverflow,
                                &unit.limits,
                                Some(source.header.span.clone()),
                            )
                        })?,
                        hir_key: key,
                    });
                }
                TypedAstDeclaration::ManeuverPath(source) => {
                    let key = paths
                        .push(HirManeuverPath {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            stable_id: ManeuverPathId::from_untyped(StableId128::ZERO),
                            movement: HirMovementKey::from_raw(0),
                            movement_source_location: None,
                            edges: TableRange::empty(),
                            maneuver_gates: TableRange::empty(),
                            waiting_zones: TableRange::empty(),
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    path_sources.push(CanonicalDeclarationSource {
                        source_module_index: module_order,
                        declaration_index: u32::try_from(declaration_index).map_err(|_| {
                            arena_overflow(
                                ArenaKeyOverflow,
                                &unit.limits,
                                Some(source.header.span.clone()),
                            )
                        })?,
                        hir_key: key,
                    });
                }
                _ => {}
            }
        }
    }

    let mut diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
    let mut junction_member_counts = vec![0_usize; junctions.len()];
    for location in &movement_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let source =
            movement_declaration(&source_module.declarations[location.declaration_index as usize])
                .expect("canonical Movement source must name a Movement");
        let Some(junction) = resolve_reference(
            module_lookup,
            &junction_symbols,
            &source.junction,
            EntityKind::Movement,
            &source.header,
            location.source_module_index,
            &mut diagnostics,
        ) else {
            continue;
        };
        let junction_id = junctions.get(junction).stable_id.into_untyped();
        let fields = [
            IdentityFieldInput::new(
                FieldTag::AuthoringNamespaceId,
                source_module
                    .descriptor()
                    .authoring_namespace_id()
                    .as_bytes(),
            ),
            IdentityFieldInput::new(FieldTag::MovementKey, source.header.stable_key.as_bytes()),
            IdentityFieldInput::new(
                FieldTag::DirectedEntryApproachKey,
                source.directed_entry_approach_key.as_bytes(),
            ),
            IdentityFieldInput::new(
                FieldTag::DirectedExitApproachKey,
                source.directed_exit_approach_key.as_bytes(),
            ),
            IdentityFieldInput::new(FieldTag::JunctionStableId, junction_id.as_bytes()),
        ];
        let stable_id = MovementId::from_untyped(derive_identity(
            unit,
            identities,
            location.source_module_index as usize,
            EntityKind::Movement,
            &source.header.stable_key,
            &source.header.span,
            &fields,
        )?);
        let movement = movements.get_mut(location.hir_key);
        movement.stable_id = stable_id;
        movement.junction = junction;
        movement.junction_source_location = Some(unit.resolve_source_location_for_module(
            location.source_module_index,
            &source.junction.span,
        )?);
        junction_member_counts[junction.index()] =
            junction_member_counts[junction.index()].saturating_add(1);
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    let mut path_edges =
        Vec::with_capacity(count_to_usize(counts.maneuver_path_edges, &unit.limits)?);
    let mut movement_member_counts = vec![0_usize; movements.len()];
    for location in &path_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let source = maneuver_path_declaration(
            &source_module.declarations[location.declaration_index as usize],
        )
        .expect("canonical ManeuverPath source must name a ManeuverPath");
        let movement = resolve_reference(
            module_lookup,
            &movement_symbols,
            &source.movement,
            EntityKind::ManeuverPath,
            &source.header,
            location.source_module_index,
            &mut diagnostics,
        );
        let start = path_edges.len();
        let mut predecessor: Option<(HirLaneEdgeKey, SourceLocation)> = None;
        let mut entry = None;
        let mut exit = None;
        for (index, reference) in core::iter::once(&source.entry_edge)
            .chain(source.internal_edges.iter())
            .chain(core::iter::once(&source.exit_edge))
            .enumerate()
        {
            let Some(target) = resolve_reference(
                module_lookup,
                lane_edge_symbols,
                reference,
                EntityKind::ManeuverPath,
                &source.header,
                location.source_module_index,
                &mut diagnostics,
            ) else {
                predecessor = None;
                continue;
            };
            if index == 0 {
                entry = Some(target);
            }
            if index == source.internal_edges.len().saturating_add(1) {
                exit = Some(target);
            }
            if let Some((predecessor_key, predecessor_span)) = predecessor {
                let predecessor_record = lane_edges.get(predecessor_key);
                let connected = lane_edge_references
                    [predecessor_record.successors.as_usize_range()]
                .iter()
                .any(|candidate| candidate.target == target);
                if !connected {
                    let mut diagnostic = Diagnostic::disconnected_maneuver_path(
                        &source.header.stable_key,
                        &predecessor_record.stable_key,
                        &lane_edges.get(target).stable_key,
                        reference.span.clone(),
                        predecessor_span,
                    );
                    diagnostic.set_canonical_module_order(location.source_module_index);
                    diagnostics.push(diagnostic);
                }
            }
            predecessor = Some((target, reference.span.clone()));
            path_edges.push(HirManeuverPathEdge {
                target,
                source_span: reference.span.clone(),
            });
        }
        let (Some(movement), Some(entry), Some(exit)) = (movement, entry, exit) else {
            continue;
        };
        let movement_id = movements.get(movement).stable_id.into_untyped();
        let entry_id = lane_edges.get(entry).stable_id.into_untyped();
        let exit_id = lane_edges.get(exit).stable_id.into_untyped();
        let fields = [
            IdentityFieldInput::new(
                FieldTag::AuthoringNamespaceId,
                source_module
                    .descriptor()
                    .authoring_namespace_id()
                    .as_bytes(),
            ),
            IdentityFieldInput::new(FieldTag::PathKey, source.header.stable_key.as_bytes()),
            IdentityFieldInput::new(FieldTag::MovementStableId, movement_id.as_bytes()),
            IdentityFieldInput::new(FieldTag::EntryEdgeStableId, entry_id.as_bytes()),
            IdentityFieldInput::new(FieldTag::ExitEdgeStableId, exit_id.as_bytes()),
        ];
        let stable_id = ManeuverPathId::from_untyped(derive_identity(
            unit,
            identities,
            location.source_module_index as usize,
            EntityKind::ManeuverPath,
            &source.header.stable_key,
            &source.header.span,
            &fields,
        )?);
        let path = paths.get_mut(location.hir_key);
        path.stable_id = stable_id;
        path.movement = movement;
        path.movement_source_location = Some(unit.resolve_source_location_for_module(
            location.source_module_index,
            &source.movement.span,
        )?);
        path.edges = TableRange::try_from_usize(start, path_edges.len().saturating_sub(start))
            .map_err(|overflow| {
                arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
            })?;
        movement_member_counts[movement.index()] =
            movement_member_counts[movement.index()].saturating_add(1);
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    // 完整路径序列的全局唯一性先于内部边角色派生，以保持 zero-internal 与普通路径的
    // 重复错误一致。HashMap 只查找已冻结切片，完整目标键比较封堵哈希碰撞。
    let mut sequence_index: HashMap<ManeuverPathSequence<'_>, HirManeuverPathKey> =
        HashMap::with_capacity(paths.len());
    for (path_key, path) in paths.iter() {
        let sequence = ManeuverPathSequence(&path_edges[path.edges.as_usize_range()]);
        if let Some(first_path_key) = sequence_index.get(&sequence).copied() {
            let first = paths.get(first_path_key);
            let first_junction = movements.get(first.movement).junction;
            let duplicate_junction = movements.get(path.movement).junction;
            let mut diagnostic = Diagnostic::duplicate_maneuver_path_sequence(
                &first.stable_key,
                &path.stable_key,
                &junctions.get(first_junction).stable_key,
                &junctions.get(duplicate_junction).stable_key,
                path.source_span.clone(),
                first.source_span.clone(),
            );
            diagnostic.set_canonical_module_order(path.module.raw());
            diagnostics.push(diagnostic);
        } else {
            sequence_index.insert(sequence, path_key);
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }
    drop(sequence_index);

    let mut internal_claims: Vec<Option<HirJunctionInternalEdge>> =
        (0..lane_edges.len()).map(|_| None).collect();
    let mut boundary_claims: Vec<Option<(HirManeuverPathKey, SourceLocation)>> =
        (0..lane_edges.len()).map(|_| None).collect();
    for (path_key, path) in paths.iter() {
        let edge_range = path.edges.as_usize_range();
        let edges = &path_edges[edge_range];
        let junction = movements.get(path.movement).junction;
        for (local_index, edge) in edges.iter().enumerate() {
            let is_boundary = local_index == 0 || local_index + 1 == edges.len();
            if is_boundary {
                if let Some(internal) = &internal_claims[edge.target.index()] {
                    let internal_path = paths.get(internal.source_path);
                    let mut diagnostic = Diagnostic::internal_boundary_role_conflict(
                        &lane_edges.get(edge.target).stable_key,
                        &internal_path.stable_key,
                        &path.stable_key,
                        edge.source_span.clone(),
                        internal.source_span.clone(),
                    );
                    diagnostic.set_canonical_module_order(path.module.raw());
                    diagnostics.push(diagnostic);
                } else if boundary_claims[edge.target.index()].is_none() {
                    boundary_claims[edge.target.index()] =
                        Some((path_key, edge.source_span.clone()));
                }
                continue;
            }
            if let Some((boundary_path_key, boundary_span)) = &boundary_claims[edge.target.index()]
            {
                let mut diagnostic = Diagnostic::internal_boundary_role_conflict(
                    &lane_edges.get(edge.target).stable_key,
                    &path.stable_key,
                    &paths.get(*boundary_path_key).stable_key,
                    edge.source_span.clone(),
                    boundary_span.clone(),
                );
                diagnostic.set_canonical_module_order(path.module.raw());
                diagnostics.push(diagnostic);
                continue;
            }
            if let Some(first) = &internal_claims[edge.target.index()] {
                if first.junction != junction {
                    let mut diagnostic = Diagnostic::internal_edge_junction_conflict(
                        &lane_edges.get(edge.target).stable_key,
                        &junctions.get(first.junction).stable_key,
                        &junctions.get(junction).stable_key,
                        &paths.get(first.source_path).stable_key,
                        &path.stable_key,
                        edge.source_span.clone(),
                        first.source_span.clone(),
                    );
                    diagnostic.set_canonical_module_order(path.module.raw());
                    diagnostics.push(diagnostic);
                } else if path.stable_id < paths.get(first.source_path).stable_id {
                    // 同一路口多条路径可共享内部边。来源映射选择 StableId 较小的路径作为
                    // 规范主要来源，避免声明排列改变同一派生关系的回链位置。
                    internal_claims[edge.target.index()] = Some(HirJunctionInternalEdge {
                        edge: edge.target,
                        junction,
                        source_path: path_key,
                        source_span: edge.source_span.clone(),
                    });
                }
            } else {
                internal_claims[edge.target.index()] = Some(HirJunctionInternalEdge {
                    edge: edge.target,
                    junction,
                    source_path: path_key,
                    source_span: edge.source_span.clone(),
                });
            }
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    for (junction_key, junction) in junctions.iter() {
        if junction_member_counts[junction_key.index()] == 0 {
            let mut diagnostic =
                Diagnostic::empty_junction(&junction.stable_key, junction.source_span.clone());
            diagnostic.set_canonical_module_order(junction.module.raw());
            diagnostics.push(diagnostic);
        }
    }
    for (movement_key, movement) in movements.iter() {
        if movement_member_counts[movement_key.index()] == 0 {
            let mut diagnostic =
                Diagnostic::empty_movement(&movement.stable_key, movement.source_span.clone());
            diagnostic.set_canonical_module_order(movement.module.raw());
            diagnostics.push(diagnostic);
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    let mut next_junction_member = Vec::with_capacity(junctions.len());
    let mut junction_member_total = 0_usize;
    for (index, count) in junction_member_counts.iter().copied().enumerate() {
        next_junction_member.push(junction_member_total);
        let key = HirJunctionKey::from_raw(
            u32::try_from(index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        junctions.get_mut(key).movements = TableRange::try_from_usize(junction_member_total, count)
            .map_err(|overflow| arena_overflow(overflow, &unit.limits, None))?;
        junction_member_total = junction_member_total.saturating_add(count);
    }
    let first_movement = movements
        .iter()
        .next()
        .map(|(key, _)| key)
        .unwrap_or(HirMovementKey::from_raw(0));
    let mut junction_movements = vec![
        HirJunctionMovement {
            movement: first_movement,
        };
        junction_member_total
    ];
    for (movement_key, movement) in movements.iter() {
        let destination = &mut next_junction_member[movement.junction.index()];
        junction_movements[*destination] = HirJunctionMovement {
            movement: movement_key,
        };
        *destination += 1;
    }

    let mut next_movement_member = Vec::with_capacity(movements.len());
    let mut movement_member_total = 0_usize;
    for (index, count) in movement_member_counts.iter().copied().enumerate() {
        next_movement_member.push(movement_member_total);
        let key = HirMovementKey::from_raw(
            u32::try_from(index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        movements.get_mut(key).maneuver_paths =
            TableRange::try_from_usize(movement_member_total, count)
                .map_err(|overflow| arena_overflow(overflow, &unit.limits, None))?;
        movement_member_total = movement_member_total.saturating_add(count);
    }
    let first_path = paths
        .iter()
        .next()
        .map(|(key, _)| key)
        .unwrap_or(HirManeuverPathKey::from_raw(0));
    let mut movement_maneuver_paths = vec![
        HirMovementManeuverPath {
            maneuver_path: first_path,
        };
        movement_member_total
    ];
    for (path_key, path) in paths.iter() {
        let destination = &mut next_movement_member[path.movement.index()];
        movement_maneuver_paths[*destination] = HirMovementManeuverPath {
            maneuver_path: path_key,
        };
        *destination += 1;
    }

    let junction_internal_edges = internal_claims
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .into_boxed_slice();
    Ok(JunctionHir {
        junctions: junctions.into_boxed_slice(),
        movements: movements.into_boxed_slice(),
        junction_movements: junction_movements.into_boxed_slice(),
        maneuver_paths: paths.into_boxed_slice(),
        movement_maneuver_paths: movement_maneuver_paths.into_boxed_slice(),
        maneuver_path_edges: path_edges.into_boxed_slice(),
        junction_internal_edges,
    })
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn build_control_hir(
    unit: &CompilationUnit,
    module_lookup: &HashMap<Arc<str>, HirModuleKey>,
    lane_edges: &TypedArena<HirLaneEdgeTag, HirLaneEdge>,
    lane_edge_references: &[HirLaneEdgeReference],
    lane_edge_symbols: &SymbolTable<HirLaneEdgeKey>,
    maneuver_paths: &mut [HirManeuverPath],
    maneuver_path_edges: &[HirManeuverPathEdge],
    identities: &mut IdentityRegistry,
) -> Result<ControlHir, DiagnosticBundle> {
    let counts = control_counts(unit);
    if counts.entity_count() == 0 {
        return Ok(ControlHir::default());
    }

    let mut path_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, TypedAstDeclaration::ManeuverPath(_)))
            .count()
    }));
    for (index, path) in maneuver_paths.iter().enumerate() {
        let key = HirManeuverPathKey::from_raw(
            u32::try_from(index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        path_symbols.insert(path.module, Arc::clone(&path.stable_key), key);
    }

    let mut stop_lines = TypedArena::<HirStopLineTag, HirStopLine>::with_capacity(count_to_usize(
        counts.stop_lines,
        &unit.limits,
    )?);
    let mut gates = TypedArena::<HirManeuverGateTag, HirManeuverGate>::with_capacity(
        count_to_usize(counts.maneuver_gates, &unit.limits)?,
    );
    let mut waiting_zones = TypedArena::<HirWaitingZoneTag, HirWaitingZone>::with_capacity(
        count_to_usize(counts.waiting_zones, &unit.limits)?,
    );
    let mut stop_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, TypedAstDeclaration::StopLine(_)))
            .count()
    }));
    let mut gate_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, TypedAstDeclaration::ManeuverGate(_)))
            .count()
    }));
    let mut stop_sources = Vec::with_capacity(count_to_usize(counts.stop_lines, &unit.limits)?);
    let mut gate_sources = Vec::with_capacity(count_to_usize(counts.maneuver_gates, &unit.limits)?);
    let mut waiting_sources =
        Vec::with_capacity(count_to_usize(counts.waiting_zones, &unit.limits)?);

    // 先登记全部控制对象符号，保证声明顺序不影响前向引用；依赖父项的身份先放零值，
    // 只有引用与领域约束全部闭合后才会离开本函数。
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key = HirModuleKey::from_raw(
            u32::try_from(module_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        let module_order = u32::try_from(module_index).unwrap_or(u32::MAX);
        let mut declaration_indices: Vec<_> = source_module
            .declarations
            .iter()
            .enumerate()
            .filter_map(|(index, declaration)| {
                matches!(
                    declaration,
                    TypedAstDeclaration::StopLine(_)
                        | TypedAstDeclaration::ManeuverGate(_)
                        | TypedAstDeclaration::WaitingZone(_)
                )
                .then_some(index)
            })
            .collect();
        declaration_indices.sort_unstable_by(|left, right| {
            let left = declaration_header(&source_module.declarations[*left]);
            let right = declaration_header(&source_module.declarations[*right]);
            (left.entity_kind.code(), left.stable_key.as_bytes())
                .cmp(&(right.entity_kind.code(), right.stable_key.as_bytes()))
        });
        for declaration_index in declaration_indices {
            let source_index = u32::try_from(declaration_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?;
            match &source_module.declarations[declaration_index] {
                TypedAstDeclaration::StopLine(source) => {
                    let fields = [
                        IdentityFieldInput::new(
                            FieldTag::AuthoringNamespaceId,
                            source_module
                                .descriptor()
                                .authoring_namespace_id()
                                .as_bytes(),
                        ),
                        IdentityFieldInput::new(
                            FieldTag::StopLineKey,
                            source.header.stable_key.as_bytes(),
                        ),
                    ];
                    let stable_id = StopLineId::from_untyped(derive_identity(
                        unit,
                        identities,
                        module_index,
                        EntityKind::StopLine,
                        &source.header.stable_key,
                        &source.header.span,
                        &fields,
                    )?);
                    let key = stop_lines
                        .push(HirStopLine {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            stable_id,
                            lane_edge: HirLaneEdgeKey::from_raw(0),
                            maneuver_gates: TableRange::empty(),
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    stop_symbols.insert(module_key, Arc::clone(&source.header.stable_key), key);
                    stop_sources.push(CanonicalDeclarationSource {
                        source_module_index: module_order,
                        declaration_index: source_index,
                        hir_key: key,
                    });
                }
                TypedAstDeclaration::ManeuverGate(source) => {
                    let key = gates
                        .push(HirManeuverGate {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            stable_id: ManeuverGateId::from_untyped(StableId128::ZERO),
                            maneuver_path: HirManeuverPathKey::from_raw(0),
                            maneuver_path_source_location: None,
                            transition_index: source.transition_index,
                            stop_line: HirStopLineKey::from_raw(0),
                            stop_line_source_location: None,
                            signal_control: HirSignalControl::None,
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    gate_symbols.insert(module_key, Arc::clone(&source.header.stable_key), key);
                    gate_sources.push(CanonicalDeclarationSource {
                        source_module_index: module_order,
                        declaration_index: source_index,
                        hir_key: key,
                    });
                }
                TypedAstDeclaration::WaitingZone(source) => {
                    let key = waiting_zones
                        .push(HirWaitingZone {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            stable_id: WaitingZoneId::from_untyped(StableId128::ZERO),
                            maneuver_path: HirManeuverPathKey::from_raw(0),
                            maneuver_path_source_location: None,
                            entry_gate: HirManeuverGateKey::from_raw(0),
                            release_gate: HirManeuverGateKey::from_raw(0),
                            max_occupancy: source.max_occupancy,
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    waiting_sources.push(CanonicalDeclarationSource {
                        source_module_index: module_order,
                        declaration_index: source_index,
                        hir_key: key,
                    });
                }
                _ => unreachable!("control source filter admitted unrelated declaration"),
            }
        }
    }

    let mut diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
    // StopLine 对 LaneEdge 是一对一关系。该表既在解析阶段发现重复所有者，也在后续
    // 覆盖校验中把候选 ManeuverPath 反查到唯一 StopLine，避免按停止线反复扫描路径。
    let mut stop_line_by_edge = vec![None; lane_edges.len()];
    for location in &stop_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let TypedAstDeclaration::StopLine(source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical StopLine source changed kind")
        };
        if let Some(edge) = resolve_reference(
            module_lookup,
            lane_edge_symbols,
            &source.lane_edge,
            EntityKind::StopLine,
            &source.header,
            location.source_module_index,
            &mut diagnostics,
        ) {
            if let Some(first_key) = stop_line_by_edge[edge.index()] {
                let first = stop_lines.get(first_key);
                let duplicate = stop_lines.get(location.hir_key);
                let mut diagnostic = Diagnostic::duplicate_stop_line_edge(
                    &lane_edges.get(edge).stable_key,
                    &first.stable_key,
                    &duplicate.stable_key,
                    duplicate.source_span.clone(),
                    first.source_span.clone(),
                );
                diagnostic.set_canonical_module_order(location.source_module_index);
                diagnostics.push(diagnostic);
            } else {
                stop_line_by_edge[edge.index()] = Some(location.hir_key);
            }
            stop_lines.get_mut(location.hir_key).lane_edge = edge;
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    let mut resolved_gate_keys = Vec::with_capacity(gates.len());
    for location in &gate_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let TypedAstDeclaration::ManeuverGate(source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical ManeuverGate source changed kind")
        };
        let path = resolve_reference(
            module_lookup,
            &path_symbols,
            &source.maneuver_path,
            EntityKind::ManeuverGate,
            &source.header,
            location.source_module_index,
            &mut diagnostics,
        );
        let stop_line = resolve_reference(
            module_lookup,
            &stop_symbols,
            &source.stop_line,
            EntityKind::ManeuverGate,
            &source.header,
            location.source_module_index,
            &mut diagnostics,
        );
        let (Some(path_key), Some(stop_line_key)) = (path, stop_line) else {
            continue;
        };
        let path = &maneuver_paths[path_key.index()];
        let transition_count = path.edges.len().saturating_sub(1);
        if source.transition_index >= transition_count {
            let mut diagnostic = Diagnostic::maneuver_gate_transition_out_of_range(
                &source.header.stable_key,
                &path.stable_key,
                source.transition_index,
                transition_count,
                source.header.span.clone(),
                path.source_span.clone(),
            );
            diagnostic.set_canonical_module_order(location.source_module_index);
            diagnostics.push(diagnostic);
            continue;
        }
        let from_edge = maneuver_path_edges[path.edges.as_usize_range()]
            [source.transition_index as usize]
            .target;
        let stop = stop_lines.get(stop_line_key);
        if stop.lane_edge != from_edge {
            let mut diagnostic = Diagnostic::maneuver_gate_stop_line_mismatch(
                &source.header.stable_key,
                &stop.stable_key,
                &lane_edges.get(from_edge).stable_key,
                &lane_edges.get(stop.lane_edge).stable_key,
                source.header.span.clone(),
                stop.source_span.clone(),
            );
            diagnostic.set_canonical_module_order(location.source_module_index);
            diagnostics.push(diagnostic);
            continue;
        }
        let path_id = path.stable_id.into_untyped();
        let fields = [
            IdentityFieldInput::new(
                FieldTag::AuthoringNamespaceId,
                source_module
                    .descriptor()
                    .authoring_namespace_id()
                    .as_bytes(),
            ),
            IdentityFieldInput::new(FieldTag::ManeuverPathStableId, path_id.as_bytes()),
            IdentityFieldInput::new(FieldTag::GateKey, source.header.stable_key.as_bytes()),
        ];
        let stable_id = ManeuverGateId::from_untyped(derive_identity(
            unit,
            identities,
            location.source_module_index as usize,
            EntityKind::ManeuverGate,
            &source.header.stable_key,
            &source.header.span,
            &fields,
        )?);
        let gate = gates.get_mut(location.hir_key);
        gate.stable_id = stable_id;
        gate.maneuver_path = path_key;
        gate.maneuver_path_source_location = Some(unit.resolve_source_location_for_module(
            location.source_module_index,
            &source.maneuver_path.span,
        )?);
        gate.stop_line = stop_line_key;
        gate.stop_line_source_location = Some(unit.resolve_source_location_for_module(
            location.source_module_index,
            &source.stop_line.span,
        )?);
        resolved_gate_keys.push(location.hir_key);
    }

    resolved_gate_keys.sort_unstable_by(|left, right| {
        let left = gates.get(*left);
        let right = gates.get(*right);
        (
            left.maneuver_path.raw(),
            left.transition_index,
            left.stable_key.as_bytes(),
        )
            .cmp(&(
                right.maneuver_path.raw(),
                right.transition_index,
                right.stable_key.as_bytes(),
            ))
    });
    for pair in resolved_gate_keys.windows(2) {
        let first = gates.get(pair[0]);
        let duplicate = gates.get(pair[1]);
        if first.maneuver_path == duplicate.maneuver_path
            && first.transition_index == duplicate.transition_index
        {
            let path = &maneuver_paths[first.maneuver_path.index()];
            let mut diagnostic = Diagnostic::duplicate_maneuver_gate_path_transition(
                &path.stable_key,
                first.transition_index,
                &first.stable_key,
                &duplicate.stable_key,
                duplicate.source_span.clone(),
                first.source_span.clone(),
            );
            diagnostic.set_canonical_module_order(duplicate.module.raw());
            diagnostics.push(diagnostic);
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    let mut path_gate_counts = vec![0_usize; maneuver_paths.len()];
    let mut stop_gate_counts = vec![0_usize; stop_lines.len()];
    // 使用 u8 标志，使实际容量与上方 scratch 字节预算保持一一对应。
    let mut path_has_entry_gate = vec![0_u8; maneuver_paths.len()];
    let mut stop_has_entry_gate = vec![0_u8; stop_lines.len()];
    for gate_key in &resolved_gate_keys {
        let gate = gates.get(*gate_key);
        path_gate_counts[gate.maneuver_path.index()] =
            path_gate_counts[gate.maneuver_path.index()].saturating_add(1);
        stop_gate_counts[gate.stop_line.index()] =
            stop_gate_counts[gate.stop_line.index()].saturating_add(1);
        if gate.transition_index == 0 {
            path_has_entry_gate[gate.maneuver_path.index()] = 1;
            stop_has_entry_gate[gate.stop_line.index()] = 1;
        }
    }

    // 每个 successor 引用是否至少有一条 ManeuverPath 以该转换起步。路径已在前一阶段
    // 验证连通，因此这里可以用首个转换建立线性覆盖表，并同时检查所有候选路径的入口门。
    let mut successor_has_path = vec![0_u8; lane_edge_references.len()];
    for (path_index, path) in maneuver_paths.iter().enumerate() {
        let path_edges = &maneuver_path_edges[path.edges.as_usize_range()];
        let [from, to, ..] = path_edges else {
            unreachable!("validated ManeuverPath must contain at least entry and exit edges")
        };
        let successor_range = lane_edges.get(from.target).successors.as_usize_range();
        let successor_offset = lane_edge_references[successor_range.clone()]
            .iter()
            .position(|successor| successor.target == to.target)
            .expect("validated ManeuverPath first transition must be connected");
        successor_has_path[successor_range.start + successor_offset] = 1;

        let Some(stop_key) = stop_line_by_edge[from.target.index()] else {
            continue;
        };
        if stop_has_entry_gate[stop_key.index()] == 0 || path_has_entry_gate[path_index] != 0 {
            continue;
        }
        let stop = stop_lines.get(stop_key);
        let mut diagnostic = Diagnostic::missing_maneuver_gate_coverage(
            &stop.stable_key,
            &lane_edges.get(from.target).stable_key,
            &path.stable_key,
            stop.source_span.clone(),
            path.source_span.clone(),
        );
        diagnostic.set_canonical_module_order(stop.module.raw());
        diagnostics.push(diagnostic);
    }
    for (stop_key, stop) in stop_lines.iter() {
        let successor_range = lane_edges.get(stop.lane_edge).successors.as_usize_range();
        if successor_range.is_empty() {
            let mut diagnostic = Diagnostic::orphan_stop_line(
                &stop.stable_key,
                &lane_edges.get(stop.lane_edge).stable_key,
                stop.source_span.clone(),
            );
            diagnostic.set_canonical_module_order(stop.module.raw());
            diagnostics.push(diagnostic);
        } else if stop_gate_counts[stop_key.index()] == 0 {
            let mut diagnostic = Diagnostic::unreferenced_stop_line(
                &stop.stable_key,
                &lane_edges.get(stop.lane_edge).stable_key,
                stop.source_span.clone(),
            );
            diagnostic.set_canonical_module_order(stop.module.raw());
            diagnostics.push(diagnostic);
        } else if stop_has_entry_gate[stop_key.index()] != 0 {
            for successor_index in successor_range {
                if successor_has_path[successor_index] != 0 {
                    continue;
                }
                let successor = lane_edge_references[successor_index].target;
                let mut diagnostic = Diagnostic::missing_maneuver_path_coverage(
                    &stop.stable_key,
                    &lane_edges.get(stop.lane_edge).stable_key,
                    &lane_edges.get(successor).stable_key,
                    stop.source_span.clone(),
                    lane_edges.get(successor).source_span.clone(),
                );
                diagnostic.set_canonical_module_order(stop.module.raw());
                diagnostics.push(diagnostic);
            }
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }
    drop(stop_line_by_edge);
    drop(path_has_entry_gate);
    drop(stop_has_entry_gate);
    drop(successor_has_path);

    let mut path_gate_total = 0_usize;
    for (index, count) in path_gate_counts.iter().copied().enumerate() {
        maneuver_paths[index].maneuver_gates =
            TableRange::try_from_usize(path_gate_total, count)
                .map_err(|overflow| arena_overflow(overflow, &unit.limits, None))?;
        path_gate_total = path_gate_total.saturating_add(count);
    }
    let mut maneuver_path_gates = Vec::with_capacity(path_gate_total);
    for gate_key in &resolved_gate_keys {
        maneuver_path_gates.push(HirManeuverPathGate {
            maneuver_gate: *gate_key,
        });
    }
    debug_assert_eq!(maneuver_path_gates.len(), path_gate_total);

    let mut stop_gate_order = resolved_gate_keys.clone();
    stop_gate_order.sort_unstable_by(|left, right| {
        let left = gates.get(*left);
        let right = gates.get(*right);
        (left.stop_line.raw(), left.stable_id).cmp(&(right.stop_line.raw(), right.stable_id))
    });
    let mut stop_gate_total = 0_usize;
    for (index, count) in stop_gate_counts.iter().copied().enumerate() {
        let key = HirStopLineKey::from_raw(u32::try_from(index).unwrap_or(u32::MAX));
        stop_lines.get_mut(key).maneuver_gates = TableRange::try_from_usize(stop_gate_total, count)
            .map_err(|overflow| arena_overflow(overflow, &unit.limits, None))?;
        stop_gate_total = stop_gate_total.saturating_add(count);
    }
    let stop_line_maneuver_gates = stop_gate_order
        .into_iter()
        .map(|maneuver_gate| HirStopLineManeuverGate { maneuver_gate })
        .collect::<Vec<_>>();

    let mut resolved_waiting_keys = Vec::with_capacity(waiting_zones.len());
    for location in &waiting_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let TypedAstDeclaration::WaitingZone(source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical WaitingZone source changed kind")
        };
        let path = resolve_reference(
            module_lookup,
            &path_symbols,
            &source.maneuver_path,
            EntityKind::WaitingZone,
            &source.header,
            location.source_module_index,
            &mut diagnostics,
        );
        let entry_gate = resolve_reference(
            module_lookup,
            &gate_symbols,
            &source.entry_gate,
            EntityKind::WaitingZone,
            &source.header,
            location.source_module_index,
            &mut diagnostics,
        );
        let release_gate = resolve_reference(
            module_lookup,
            &gate_symbols,
            &source.release_gate,
            EntityKind::WaitingZone,
            &source.header,
            location.source_module_index,
            &mut diagnostics,
        );
        let (Some(path_key), Some(entry_key), Some(release_key)) = (path, entry_gate, release_gate)
        else {
            continue;
        };
        let entry = gates.get(entry_key);
        let release = gates.get(release_key);
        let mut path_mismatch = false;
        for (role, gate) in [
            (WaitingZoneGateRole::Entry, entry),
            (WaitingZoneGateRole::Release, release),
        ] {
            if gate.maneuver_path != path_key {
                let mut diagnostic = Diagnostic::waiting_zone_gate_path_mismatch(
                    &source.header.stable_key,
                    role,
                    &gate.stable_key,
                    &maneuver_paths[path_key.index()].stable_key,
                    &maneuver_paths[gate.maneuver_path.index()].stable_key,
                    source.header.span.clone(),
                    gate.source_span.clone(),
                );
                diagnostic.set_canonical_module_order(location.source_module_index);
                diagnostics.push(diagnostic);
                path_mismatch = true;
            }
        }
        if path_mismatch {
            continue;
        }
        if entry.transition_index >= release.transition_index {
            let mut diagnostic = Diagnostic::invalid_waiting_zone_gate_order(
                &source.header.stable_key,
                entry.transition_index,
                release.transition_index,
                source.header.span.clone(),
            );
            diagnostic.set_canonical_module_order(location.source_module_index);
            diagnostics.push(diagnostic);
            continue;
        }
        let path_id = maneuver_paths[path_key.index()].stable_id.into_untyped();
        let fields = [
            IdentityFieldInput::new(
                FieldTag::AuthoringNamespaceId,
                source_module
                    .descriptor()
                    .authoring_namespace_id()
                    .as_bytes(),
            ),
            IdentityFieldInput::new(FieldTag::ManeuverPathStableId, path_id.as_bytes()),
            IdentityFieldInput::new(
                FieldTag::WaitingZoneKey,
                source.header.stable_key.as_bytes(),
            ),
        ];
        let stable_id = WaitingZoneId::from_untyped(derive_identity(
            unit,
            identities,
            location.source_module_index as usize,
            EntityKind::WaitingZone,
            &source.header.stable_key,
            &source.header.span,
            &fields,
        )?);
        let waiting = waiting_zones.get_mut(location.hir_key);
        waiting.stable_id = stable_id;
        waiting.maneuver_path = path_key;
        waiting.maneuver_path_source_location = Some(unit.resolve_source_location_for_module(
            location.source_module_index,
            &source.maneuver_path.span,
        )?);
        waiting.entry_gate = entry_key;
        waiting.release_gate = release_key;
        resolved_waiting_keys.push(location.hir_key);
    }
    resolved_waiting_keys.sort_unstable_by(|left, right| {
        let left = waiting_zones.get(*left);
        let right = waiting_zones.get(*right);
        let left_entry = gates.get(left.entry_gate).transition_index;
        let right_entry = gates.get(right.entry_gate).transition_index;
        let left_release = gates.get(left.release_gate).transition_index;
        let right_release = gates.get(right.release_gate).transition_index;
        (
            left.maneuver_path.raw(),
            left_entry,
            left_release,
            left.stable_id,
        )
            .cmp(&(
                right.maneuver_path.raw(),
                right_entry,
                right_release,
                right.stable_id,
            ))
    });
    let mut active: Option<(HirWaitingZoneKey, u32)> = None;
    for waiting_key in &resolved_waiting_keys {
        let waiting = waiting_zones.get(*waiting_key);
        let entry = gates.get(waiting.entry_gate).transition_index;
        let release = gates.get(waiting.release_gate).transition_index;
        if let Some((active_key, active_release)) = active {
            let first = waiting_zones.get(active_key);
            if first.maneuver_path == waiting.maneuver_path && entry < active_release {
                let mut diagnostic = Diagnostic::overlapping_waiting_zones(
                    &maneuver_paths[waiting.maneuver_path.index()].stable_key,
                    &first.stable_key,
                    &waiting.stable_key,
                    waiting.source_span.clone(),
                    first.source_span.clone(),
                );
                diagnostic.set_canonical_module_order(waiting.module.raw());
                diagnostics.push(diagnostic);
            }
            if first.maneuver_path != waiting.maneuver_path || release > active_release {
                active = Some((*waiting_key, release));
            }
        } else {
            active = Some((*waiting_key, release));
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    let mut path_waiting_counts = vec![0_usize; maneuver_paths.len()];
    for waiting_key in &resolved_waiting_keys {
        let path = waiting_zones.get(*waiting_key).maneuver_path;
        path_waiting_counts[path.index()] = path_waiting_counts[path.index()].saturating_add(1);
    }
    let mut waiting_total = 0_usize;
    for (index, count) in path_waiting_counts.iter().copied().enumerate() {
        maneuver_paths[index].waiting_zones = TableRange::try_from_usize(waiting_total, count)
            .map_err(|overflow| arena_overflow(overflow, &unit.limits, None))?;
        waiting_total = waiting_total.saturating_add(count);
    }
    let maneuver_path_waiting_zones = resolved_waiting_keys
        .iter()
        .copied()
        .map(|waiting_zone| HirManeuverPathWaitingZone { waiting_zone })
        .collect::<Vec<_>>();

    Ok(ControlHir {
        stop_lines: stop_lines.into_boxed_slice(),
        maneuver_gates: gates.into_boxed_slice(),
        waiting_zones: waiting_zones.into_boxed_slice(),
        maneuver_path_gates: maneuver_path_gates.into_boxed_slice(),
        maneuver_path_waiting_zones: maneuver_path_waiting_zones.into_boxed_slice(),
        stop_line_maneuver_gates: stop_line_maneuver_gates.into_boxed_slice(),
    })
}

#[allow(clippy::too_many_lines)]
fn build_signal_hir(
    unit: &CompilationUnit,
    module_lookup: &HashMap<Arc<str>, HirModuleKey>,
    maneuver_gates: &mut [HirManeuverGate],
    identities: &mut IdentityRegistry,
) -> Result<SignalHir, DiagnosticBundle> {
    let counts = signal_counts(unit);
    if counts.entity_count() == 0 && counts.controlled_gates == 0 {
        return Ok(SignalHir::default());
    }

    let mut groups = TypedArena::<HirSignalGroupTag, HirSignalGroup>::with_capacity(
        count_to_usize(counts.groups, &unit.limits)?,
    );
    let mut controllers = TypedArena::<HirSignalControllerTag, HirSignalController>::with_capacity(
        count_to_usize(counts.controllers, &unit.limits)?,
    );
    let mut phases = TypedArena::<HirSignalPhaseTag, HirSignalPhase>::with_capacity(
        count_to_usize(counts.phases, &unit.limits)?,
    );
    let mut group_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, TypedAstDeclaration::SignalGroup(_)))
            .count()
    }));
    let mut gate_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, TypedAstDeclaration::ManeuverGate(_)))
            .count()
    }));
    for (index, gate) in maneuver_gates.iter().enumerate() {
        let key = HirManeuverGateKey::from_raw(
            u32::try_from(index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        gate_symbols.insert(gate.module, Arc::clone(&gate.stable_key), key);
    }

    let mut group_sources = Vec::with_capacity(count_to_usize(counts.groups, &unit.limits)?);
    let mut controller_sources =
        Vec::with_capacity(count_to_usize(counts.controllers, &unit.limits)?);

    // 信号组和控制器先按规范模块顺序、模块内稳定键登记，随后才解析所有权和门绑定。
    // 因此控制器、相位或门都可以前向引用同一编译单元内的组。
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key = HirModuleKey::from_raw(
            u32::try_from(module_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        let module_order = u32::try_from(module_index).unwrap_or(u32::MAX);
        let mut declaration_indices: Vec<_> = source_module
            .declarations
            .iter()
            .enumerate()
            .filter_map(|(index, declaration)| {
                matches!(
                    declaration,
                    TypedAstDeclaration::SignalGroup(_) | TypedAstDeclaration::SignalController(_)
                )
                .then_some(index)
            })
            .collect();
        declaration_indices.sort_unstable_by(|left, right| {
            let left = declaration_header(&source_module.declarations[*left]);
            let right = declaration_header(&source_module.declarations[*right]);
            (left.entity_kind.code(), left.stable_key.as_bytes())
                .cmp(&(right.entity_kind.code(), right.stable_key.as_bytes()))
        });
        for declaration_index in declaration_indices {
            let source_index = u32::try_from(declaration_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?;
            match &source_module.declarations[declaration_index] {
                TypedAstDeclaration::SignalGroup(source) => {
                    let fields = [
                        IdentityFieldInput::new(
                            FieldTag::AuthoringNamespaceId,
                            source_module
                                .descriptor()
                                .authoring_namespace_id()
                                .as_bytes(),
                        ),
                        IdentityFieldInput::new(
                            FieldTag::SignalGroupKey,
                            source.header.stable_key.as_bytes(),
                        ),
                    ];
                    let stable_id = SignalGroupId::from_untyped(derive_identity(
                        unit,
                        identities,
                        module_index,
                        EntityKind::SignalGroup,
                        &source.header.stable_key,
                        &source.header.span,
                        &fields,
                    )?);
                    let key = groups
                        .push(HirSignalGroup {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            stable_id,
                            controller: HirSignalControllerKey::from_raw(0),
                            maneuver_gates: TableRange::empty(),
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    group_symbols.insert(module_key, Arc::clone(&source.header.stable_key), key);
                    group_sources.push(CanonicalDeclarationSource {
                        source_module_index: module_order,
                        declaration_index: source_index,
                        hir_key: key,
                    });
                }
                TypedAstDeclaration::SignalController(source) => {
                    let fields = [
                        IdentityFieldInput::new(
                            FieldTag::AuthoringNamespaceId,
                            source_module
                                .descriptor()
                                .authoring_namespace_id()
                                .as_bytes(),
                        ),
                        IdentityFieldInput::new(
                            FieldTag::SignalControllerKey,
                            source.header.stable_key.as_bytes(),
                        ),
                    ];
                    let stable_id = SignalControllerId::from_untyped(derive_identity(
                        unit,
                        identities,
                        module_index,
                        EntityKind::SignalController,
                        &source.header.stable_key,
                        &source.header.span,
                        &fields,
                    )?);
                    let key = controllers
                        .push(HirSignalController {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            stable_id,
                            offset_ms: source.offset_ms,
                            cycle_duration_ms: 0,
                            signal_groups: TableRange::empty(),
                            phases: TableRange::empty(),
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    controller_sources.push(CanonicalDeclarationSource {
                        source_module_index: module_order,
                        declaration_index: source_index,
                        hir_key: key,
                    });
                }
                _ => unreachable!("signal source filter admitted unrelated declaration"),
            }
        }
    }

    let mut diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
    let mut owners: Vec<Option<(HirSignalControllerKey, SourceLocation)>> =
        vec![None; groups.len()];
    let mut controller_group_rows =
        Vec::with_capacity(count_to_usize(counts.controller_groups, &unit.limits)?);
    let mut phase_states = Vec::with_capacity(count_to_usize(counts.phase_states, &unit.limits)?);

    for location in &controller_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let TypedAstDeclaration::SignalController(source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical SignalController source changed kind")
        };
        let module_order = location.source_module_index;
        let controller_key = location.hir_key;

        if source.signal_groups.is_empty() {
            let mut diagnostic = Diagnostic::empty_signal_controller_groups(
                &source.header.stable_key,
                source.header.span.clone(),
            );
            diagnostic.set_canonical_module_order(module_order);
            diagnostics.push(diagnostic);
        }
        if source.phases.is_empty() {
            let mut diagnostic = Diagnostic::empty_signal_controller_phases(
                &source.header.stable_key,
                source.header.span.clone(),
            );
            diagnostic.set_canonical_module_order(module_order);
            diagnostics.push(diagnostic);
        }

        let mut resolved_groups = Vec::with_capacity(source.signal_groups.len());
        let mut first_group_spans =
            HashMap::<HirSignalGroupKey, SourceLocation>::with_capacity(source.signal_groups.len());
        for reference in &source.signal_groups {
            let Some(group_key) = resolve_reference(
                module_lookup,
                &group_symbols,
                reference,
                EntityKind::SignalController,
                &source.header,
                module_order,
                &mut diagnostics,
            ) else {
                continue;
            };
            if let Some(first_span) = first_group_spans.get(&group_key) {
                let mut diagnostic = Diagnostic::duplicate_signal_controller_group(
                    &source.header.stable_key,
                    &groups.get(group_key).stable_key,
                    reference.span.clone(),
                    first_span.clone(),
                );
                diagnostic.set_canonical_module_order(module_order);
                diagnostics.push(diagnostic);
                continue;
            }
            first_group_spans.insert(group_key, reference.span.clone());
            if let Some((first_controller, first_span)) = &owners[group_key.index()] {
                let mut diagnostic = Diagnostic::signal_group_multiple_controllers(
                    &groups.get(group_key).stable_key,
                    &controllers.get(*first_controller).stable_key,
                    &source.header.stable_key,
                    reference.span.clone(),
                    first_span.clone(),
                );
                diagnostic.set_canonical_module_order(module_order);
                diagnostics.push(diagnostic);
            } else {
                owners[group_key.index()] = Some((controller_key, reference.span.clone()));
                groups.get_mut(group_key).controller = controller_key;
            }
            resolved_groups.push(group_key);
        }
        // 控制器的组声明是集合语义；这里按 StableId 建立 HIR 阶段局部确定顺序，
        // 只用于消除来源排列，不能把它当作最终 LIR 的完整身份顺序。
        resolved_groups.sort_unstable_by_key(|key| groups.get(*key).stable_id);
        let group_start = controller_group_rows.len();
        for signal_group in resolved_groups.iter().copied() {
            let source_span = first_group_spans
                .get(&signal_group)
                .expect("resolved controller group retains its first reference span");
            controller_group_rows.push(HirSignalControllerGroup {
                signal_group,
                source_location: unit
                    .resolve_source_location_for_module(module_order, source_span)?,
            });
        }
        controllers.get_mut(controller_key).signal_groups = TableRange::try_from_usize(
            group_start,
            controller_group_rows.len().saturating_sub(group_start),
        )
        .map_err(|overflow| {
            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
        })?;

        let group_positions: HashMap<_, _> = resolved_groups
            .iter()
            .copied()
            .enumerate()
            .map(|(position, key)| (key, position))
            .collect();
        let phase_start = phases.len();
        let mut phase_keys =
            HashMap::<Arc<str>, SourceLocation>::with_capacity(source.phases.len());
        let mut cycle_duration_ms = 0_u64;
        let mut cycle_overflow = false;
        let mut cycle_valid = true;
        for phase_source in &source.phases {
            if let Some(first_span) = phase_keys.get(&phase_source.header.stable_key) {
                let mut diagnostic = Diagnostic::duplicate_signal_phase_key(
                    &source.header.stable_key,
                    &phase_source.header.stable_key,
                    phase_source.header.span.clone(),
                    first_span.clone(),
                );
                diagnostic.set_canonical_module_order(module_order);
                diagnostics.push(diagnostic);
                continue;
            }
            phase_keys.insert(
                Arc::clone(&phase_source.header.stable_key),
                phase_source.header.span.clone(),
            );

            if phase_source.duration_ms == 0
                || phase_source.duration_ms > MAX_PORTABLE_SIGNAL_TIME_MS
            {
                cycle_valid = false;
                let mut diagnostic = Diagnostic::invalid_signal_phase_duration(
                    &source.header.stable_key,
                    &phase_source.header.stable_key,
                    phase_source.duration_ms,
                    MAX_PORTABLE_SIGNAL_TIME_MS,
                    phase_source.header.span.clone(),
                );
                diagnostic.set_canonical_module_order(module_order);
                diagnostics.push(diagnostic);
            } else if !cycle_overflow {
                match cycle_duration_ms.checked_add(phase_source.duration_ms) {
                    Some(sum) if sum <= MAX_PORTABLE_SIGNAL_TIME_MS => {
                        cycle_duration_ms = sum;
                    }
                    _ => {
                        cycle_overflow = true;
                        cycle_valid = false;
                        let mut diagnostic = Diagnostic::signal_cycle_duration_overflow(
                            &source.header.stable_key,
                            MAX_PORTABLE_SIGNAL_TIME_MS,
                            source.header.span.clone(),
                        );
                        diagnostic.set_canonical_module_order(module_order);
                        diagnostics.push(diagnostic);
                    }
                }
            }

            let fields = [
                IdentityFieldInput::new(
                    FieldTag::AuthoringNamespaceId,
                    source_module
                        .descriptor()
                        .authoring_namespace_id()
                        .as_bytes(),
                ),
                IdentityFieldInput::new(
                    FieldTag::SignalControllerStableId,
                    controllers
                        .get(controller_key)
                        .stable_id
                        .as_untyped()
                        .as_bytes(),
                ),
                IdentityFieldInput::new(
                    FieldTag::PhaseKey,
                    phase_source.header.stable_key.as_bytes(),
                ),
            ];
            let stable_id = SignalPhaseId::from_untyped(derive_identity(
                unit,
                identities,
                location.source_module_index as usize,
                EntityKind::SignalPhase,
                &phase_source.header.stable_key,
                &phase_source.header.span,
                &fields,
            )?);

            let mut states_by_position: Vec<Option<(SignalAspect, SourceLocation)>> =
                vec![None; resolved_groups.len()];
            for state in &phase_source.states {
                let Some(group_key) = resolve_reference(
                    module_lookup,
                    &group_symbols,
                    &state.signal_group,
                    EntityKind::SignalPhase,
                    &phase_source.header,
                    module_order,
                    &mut diagnostics,
                ) else {
                    continue;
                };
                let Some(&position) = group_positions.get(&group_key) else {
                    let mut diagnostic = Diagnostic::unknown_signal_phase_group(
                        &source.header.stable_key,
                        &phase_source.header.stable_key,
                        &groups.get(group_key).stable_key,
                        state.signal_group.span.clone(),
                        source.header.span.clone(),
                    );
                    diagnostic.set_canonical_module_order(module_order);
                    diagnostics.push(diagnostic);
                    continue;
                };
                if let Some((_, first_span)) = &states_by_position[position] {
                    let mut diagnostic = Diagnostic::duplicate_signal_phase_group(
                        &source.header.stable_key,
                        &phase_source.header.stable_key,
                        &groups.get(group_key).stable_key,
                        state.signal_group.span.clone(),
                        first_span.clone(),
                    );
                    diagnostic.set_canonical_module_order(module_order);
                    diagnostics.push(diagnostic);
                } else {
                    states_by_position[position] =
                        Some((state.aspect, state.signal_group.span.clone()));
                }
            }
            let state_start = phase_states.len();
            for (position, group_key) in resolved_groups.iter().copied().enumerate() {
                let Some((aspect, source_span)) = &states_by_position[position] else {
                    let mut diagnostic = Diagnostic::missing_signal_phase_group(
                        &source.header.stable_key,
                        &phase_source.header.stable_key,
                        &groups.get(group_key).stable_key,
                        phase_source.header.span.clone(),
                        groups.get(group_key).source_span.clone(),
                    );
                    diagnostic.set_canonical_module_order(module_order);
                    diagnostics.push(diagnostic);
                    // 失败路径不补虚构状态，保证阶段分配不会超过输入关系数预算。
                    continue;
                };
                phase_states.push(HirSignalPhaseState {
                    signal_group: group_key,
                    aspect: *aspect,
                    source_location: unit
                        .resolve_source_location_for_module(module_order, source_span)?,
                });
            }
            phases
                .push(HirSignalPhase {
                    module: controllers.get(controller_key).module,
                    stable_key: Arc::clone(&phase_source.header.stable_key),
                    stable_id,
                    controller: controller_key,
                    duration_ms: phase_source.duration_ms,
                    states: TableRange::try_from_usize(
                        state_start,
                        phase_states.len().saturating_sub(state_start),
                    )
                    .map_err(|overflow| {
                        arena_overflow(
                            overflow,
                            &unit.limits,
                            Some(phase_source.header.span.clone()),
                        )
                    })?,
                    source_span: phase_source.header.span.clone(),
                })
                .map_err(|overflow| {
                    arena_overflow(
                        overflow,
                        &unit.limits,
                        Some(phase_source.header.span.clone()),
                    )
                })?;
        }
        let controller = controllers.get_mut(controller_key);
        controller.cycle_duration_ms = cycle_duration_ms;
        controller.phases =
            TableRange::try_from_usize(phase_start, phases.len().saturating_sub(phase_start))
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                })?;
        if !source.phases.is_empty()
            && cycle_valid
            && (source.offset_ms > MAX_PORTABLE_SIGNAL_TIME_MS
                || source.offset_ms >= cycle_duration_ms)
        {
            let mut diagnostic = Diagnostic::invalid_signal_controller_offset(
                &source.header.stable_key,
                source.offset_ms,
                cycle_duration_ms,
                MAX_PORTABLE_SIGNAL_TIME_MS,
                source.header.span.clone(),
            );
            diagnostic.set_canonical_module_order(module_order);
            diagnostics.push(diagnostic);
        }
    }

    for location in &group_sources {
        if owners[location.hir_key.index()].is_none() {
            let group = groups.get(location.hir_key);
            let mut diagnostic =
                Diagnostic::unowned_signal_group(&group.stable_key, group.source_span.clone());
            diagnostic.set_canonical_module_order(location.source_module_index);
            diagnostics.push(diagnostic);
        }
    }

    // 正向门绑定完成后，按组/门稳定身份建立连续反向表；运行时不需要扫描全部门。
    let mut usages = Vec::<(HirSignalGroupKey, HirManeuverGateKey)>::with_capacity(count_to_usize(
        counts.controlled_gates,
        &unit.limits,
    )?);
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key = HirModuleKey::from_raw(
            u32::try_from(module_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        let module_order = u32::try_from(module_index).unwrap_or(u32::MAX);
        let mut declarations: Vec<_> = source_module
            .declarations
            .iter()
            .filter_map(|declaration| match declaration {
                TypedAstDeclaration::ManeuverGate(gate) => Some(gate),
                _ => None,
            })
            .collect();
        declarations
            .sort_unstable_by(|left, right| left.header.stable_key.cmp(&right.header.stable_key));
        for source in declarations {
            let gate_key = gate_symbols
                .get(module_key, &source.header.stable_key)
                .expect("control HIR must contain every ManeuverGate symbol");
            match &source.signal_control {
                OwnedSignalControl::None => {}
                OwnedSignalControl::Group(reference) => {
                    let Some(group_key) = resolve_reference(
                        module_lookup,
                        &group_symbols,
                        reference,
                        EntityKind::ManeuverGate,
                        &source.header,
                        module_order,
                        &mut diagnostics,
                    ) else {
                        continue;
                    };
                    maneuver_gates[gate_key.index()].signal_control = HirSignalControl::Group {
                        signal_group: group_key,
                        source_location: unit
                            .resolve_source_location_for_module(module_order, &reference.span)?,
                    };
                    usages.push((group_key, gate_key));
                }
            }
        }
    }
    usages.sort_unstable_by(|left, right| {
        (
            groups.get(left.0).stable_id,
            maneuver_gates[left.1.index()].stable_id,
        )
            .cmp(&(
                groups.get(right.0).stable_id,
                maneuver_gates[right.1.index()].stable_id,
            ))
    });
    // `usages` 按 StableId 排序，不能再假设其 owner 顺序与 arena key 相同。先从实际
    // 连续分组回填每个 group 的 range，避免 arena 顺序与 StableId 顺序不一致时把
    // 相邻 group 的成员切片错配给当前 group。
    let mut usage_ranges = vec![(0_usize, 0_usize); groups.len()];
    let mut usage_cursor = 0_usize;
    while usage_cursor < usages.len() {
        let group = usages[usage_cursor].0;
        let start = usage_cursor;
        while usage_cursor < usages.len() && usages[usage_cursor].0 == group {
            usage_cursor = usage_cursor.saturating_add(1);
        }
        usage_ranges[group.index()] = (start, usage_cursor.saturating_sub(start));
    }
    for (index, (start, count)) in usage_ranges.iter().copied().enumerate() {
        let group_key = HirSignalGroupKey::from_raw(
            u32::try_from(index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        groups.get_mut(group_key).maneuver_gates = TableRange::try_from_usize(start, count)
            .map_err(|overflow| arena_overflow(overflow, &unit.limits, None))?;
        if count == 0 {
            let group = groups.get(group_key);
            let mut diagnostic =
                Diagnostic::unused_signal_group(&group.stable_key, group.source_span.clone());
            diagnostic.set_canonical_module_order(group.module.raw());
            diagnostics.push(diagnostic);
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    Ok(SignalHir {
        signal_groups: groups.into_boxed_slice(),
        signal_controllers: controllers.into_boxed_slice(),
        signal_controller_groups: controller_group_rows.into_boxed_slice(),
        signal_phases: phases.into_boxed_slice(),
        signal_phase_states: phase_states.into_boxed_slice(),
        signal_group_maneuver_gates: usages
            .into_iter()
            .map(|(_, maneuver_gate)| HirSignalGroupManeuverGate { maneuver_gate })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    })
}

fn build_spatial_hir(
    unit: &CompilationUnit,
    module_lookup: &HashMap<Arc<str>, HirModuleKey>,
    lane_edges: &TypedArena<HirLaneEdgeTag, HirLaneEdge>,
    lane_edge_references: &[HirLaneEdgeReference],
    lane_edge_symbols: &SymbolTable<HirLaneEdgeKey>,
    identities: &mut IdentityRegistry,
) -> Result<SpatialHir, DiagnosticBundle> {
    let counts = spatial_counts(unit);
    if counts.canonical_frames == 0 {
        return Ok(SpatialHir::default());
    }

    let mut frames = TypedArena::<HirCanonicalFrameTag, HirCanonicalFrame>::with_capacity(
        count_to_usize(counts.canonical_frames, &unit.limits)?,
    );
    let mut geometries: Vec<HirLaneEdgeGeometry> =
        Vec::with_capacity(count_to_usize(counts.lane_edge_geometries, &unit.limits)?);
    let mut points = Vec::with_capacity(count_to_usize(counts.canonical_points, &unit.limits)?);
    let mut segments = Vec::with_capacity(count_to_usize(counts.spatial_segments, &unit.limits)?);
    let mut edge_bindings = vec![None::<(HirCanonicalFrameKey, usize)>; lane_edges.len()];
    let mut diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key = HirModuleKey::from_raw(
            u32::try_from(module_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        let mut declaration_indices: Vec<_> = source_module
            .declarations
            .iter()
            .enumerate()
            .filter_map(|(index, declaration)| {
                matches!(declaration, TypedAstDeclaration::CanonicalFrame(_)).then_some(index)
            })
            .collect();
        declaration_indices.sort_unstable_by(|left, right| {
            declaration_header(&source_module.declarations[*left])
                .stable_key
                .cmp(&declaration_header(&source_module.declarations[*right]).stable_key)
        });
        for declaration_index in declaration_indices {
            let TypedAstDeclaration::CanonicalFrame(source) =
                &source_module.declarations[declaration_index]
            else {
                unreachable!("canonical frame source filter admitted unrelated declaration")
            };
            let fields = [
                IdentityFieldInput::new(
                    FieldTag::AuthoringNamespaceId,
                    source_module
                        .descriptor()
                        .authoring_namespace_id()
                        .as_bytes(),
                ),
                IdentityFieldInput::new(
                    FieldTag::CanonicalFrameKey,
                    source.header.stable_key.as_bytes(),
                ),
            ];
            let stable_id = CanonicalFrameId::from_untyped(derive_identity(
                unit,
                identities,
                module_index,
                EntityKind::CanonicalFrame,
                &source.header.stable_key,
                &source.header.span,
                &fields,
            )?);
            let geometry_start = geometries.len();
            let frame_key = frames
                .push(HirCanonicalFrame {
                    module: module_key,
                    stable_key: Arc::clone(&source.header.stable_key),
                    stable_id,
                    lane_edge_geometries: TableRange::empty(),
                    source_span: source.header.span.clone(),
                })
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                })?;

            for geometry in &source.lane_edge_geometries {
                let target_module = module_lookup[geometry.lane_edge.module_namespace.as_ref()];
                let Some(lane_edge) =
                    lane_edge_symbols.get(target_module, &geometry.lane_edge.declaration_key)
                else {
                    let mut diagnostic = Diagnostic::unknown_reference_target(
                        EntityKind::LaneEdge,
                        &source.header.stable_key,
                        &geometry.lane_edge.module_namespace,
                        &geometry.lane_edge.declaration_key,
                        geometry.lane_edge.span.clone(),
                        source.header.span.clone(),
                    );
                    diagnostic.set_canonical_module_order(
                        u32::try_from(module_index).unwrap_or(u32::MAX),
                    );
                    diagnostics.push(diagnostic);
                    continue;
                };
                if let Some((_, existing_index)) = edge_bindings[lane_edge.index()] {
                    let mut diagnostic = Diagnostic::invalid_spatial_geometry(
                        Some(&source.header.stable_key),
                        &geometry.lane_edge.declaration_key,
                        None,
                        SpatialGeometryViolation::DuplicateEdgeBinding,
                        geometry.lane_edge.span.clone(),
                        Some(geometries[existing_index].source_span.clone()),
                    );
                    diagnostic.set_canonical_module_order(
                        u32::try_from(module_index).unwrap_or(u32::MAX),
                    );
                    diagnostics.push(diagnostic);
                    continue;
                }

                let point_start = points.len();
                points.extend(geometry.centerline_points.iter().map(|point| {
                    HirCanonicalPoint3F32 {
                        x: point.x,
                        y: point.y,
                        z: point.z,
                    }
                }));
                let segment_start = segments.len();
                let mut cumulative = 0.0_f32;
                let mut geometry_valid = true;
                for (segment_index, pair) in geometry.centerline_points.windows(2).enumerate() {
                    let delta = [
                        canonicalize_spatial_zero(pair[1].x - pair[0].x),
                        canonicalize_spatial_zero(pair[1].y - pair[0].y),
                        canonicalize_spatial_zero(pair[1].z - pair[0].z),
                    ];
                    let length = delta[0].hypot(delta[1]).hypot(delta[2]);
                    if length <= SPATIAL_MIN_SEGMENT_LENGTH_METERS {
                        let mut diagnostic = Diagnostic::invalid_spatial_geometry(
                            Some(&source.header.stable_key),
                            &geometry.lane_edge.declaration_key,
                            None,
                            SpatialGeometryViolation::DegenerateSegment {
                                segment_index: u32::try_from(segment_index).unwrap_or(u32::MAX),
                                length_bits: length.to_bits(),
                                minimum_bits: SPATIAL_MIN_SEGMENT_LENGTH_METERS.to_bits(),
                            },
                            geometry.lane_edge.span.clone(),
                            None,
                        );
                        diagnostic.set_canonical_module_order(
                            u32::try_from(module_index).unwrap_or(u32::MAX),
                        );
                        diagnostics.push(diagnostic);
                        geometry_valid = false;
                        break;
                    }
                    // 与 current Spatial 的受检单位向量构造保持逐位一致：先按最大绝对
                    // 分量缩放可避免平方求和溢出，也防止迁移投影重建出不同的局部基。
                    let tangent = normalize_spatial_vector(delta);
                    let projected_up = tangent[0].hypot(tangent[2]);
                    if projected_up < SPATIAL_MIN_PROJECTED_UP_LENGTH {
                        let mut diagnostic = Diagnostic::invalid_spatial_geometry(
                            Some(&source.header.stable_key),
                            &geometry.lane_edge.declaration_key,
                            None,
                            SpatialGeometryViolation::DegenerateProjectedUp {
                                segment_index: u32::try_from(segment_index).unwrap_or(u32::MAX),
                                projected_up_bits: projected_up.to_bits(),
                                minimum_bits: SPATIAL_MIN_PROJECTED_UP_LENGTH.to_bits(),
                            },
                            geometry.lane_edge.span.clone(),
                            None,
                        );
                        diagnostic.set_canonical_module_order(
                            u32::try_from(module_index).unwrap_or(u32::MAX),
                        );
                        diagnostics.push(diagnostic);
                        geometry_valid = false;
                        break;
                    }
                    let left = normalize_spatial_vector([tangent[2], 0.0, -tangent[0]]);
                    let raw_up = [
                        tangent[1] * left[2],
                        tangent[2] * left[0] - tangent[0] * left[2],
                        -tangent[1] * left[0],
                    ];
                    let up = normalize_spatial_vector(raw_up);
                    let next_cumulative = cumulative + length;
                    if !next_cumulative.is_finite() || next_cumulative <= cumulative {
                        let mut diagnostic = Diagnostic::invalid_spatial_geometry(
                            Some(&source.header.stable_key),
                            &geometry.lane_edge.declaration_key,
                            None,
                            SpatialGeometryViolation::ArcLengthAccumulationFailed {
                                segment_index: u32::try_from(segment_index).unwrap_or(u32::MAX),
                                accumulated_bits: cumulative.to_bits(),
                                segment_length_bits: length.to_bits(),
                            },
                            geometry.lane_edge.span.clone(),
                            None,
                        );
                        diagnostic.set_canonical_module_order(
                            u32::try_from(module_index).unwrap_or(u32::MAX),
                        );
                        diagnostics.push(diagnostic);
                        geometry_valid = false;
                        break;
                    }
                    segments.push(HirSpatialSegment {
                        length_meters: length,
                        cumulative_end_meters: next_cumulative,
                        tangent,
                        up,
                    });
                    cumulative = next_cumulative;
                }
                if !geometry_valid {
                    points.truncate(point_start);
                    segments.truncate(segment_start);
                    continue;
                }
                let lane_edge_length = lane_edges.get(lane_edge).length_meters;
                let tolerance = SPATIAL_LENGTH_ABS_TOLERANCE_METERS.max(
                    SPATIAL_LENGTH_REL_TOLERANCE * lane_edge_length.max(f64::from(cumulative)),
                ) + SPATIAL_CORE_LENGTH_QUANTIZATION_ALLOWANCE_METERS;
                if (lane_edge_length - f64::from(cumulative)).abs() > tolerance {
                    let mut diagnostic = Diagnostic::invalid_spatial_geometry(
                        Some(&source.header.stable_key),
                        &geometry.lane_edge.declaration_key,
                        None,
                        SpatialGeometryViolation::LengthMismatch {
                            lane_edge_length_bits: lane_edge_length.to_bits(),
                            geometry_length_bits: cumulative.to_bits(),
                            tolerance_bits: tolerance.to_bits(),
                        },
                        geometry.lane_edge.span.clone(),
                        None,
                    );
                    diagnostic.set_canonical_module_order(
                        u32::try_from(module_index).unwrap_or(u32::MAX),
                    );
                    diagnostics.push(diagnostic);
                    points.truncate(point_start);
                    segments.truncate(segment_start);
                    continue;
                }
                let geometry_index = geometries.len();
                geometries.push(HirLaneEdgeGeometry {
                    canonical_frame: frame_key,
                    lane_edge,
                    points: TableRange::try_from_usize(
                        point_start,
                        points.len().saturating_sub(point_start),
                    )
                    .map_err(|overflow| {
                        arena_overflow(
                            overflow,
                            &unit.limits,
                            Some(geometry.lane_edge.span.clone()),
                        )
                    })?,
                    segments: TableRange::try_from_usize(
                        segment_start,
                        segments.len().saturating_sub(segment_start),
                    )
                    .map_err(|overflow| {
                        arena_overflow(
                            overflow,
                            &unit.limits,
                            Some(geometry.lane_edge.span.clone()),
                        )
                    })?,
                    arc_length_meters: cumulative,
                    source_span: geometry.lane_edge.span.clone(),
                });
                edge_bindings[lane_edge.index()] = Some((frame_key, geometry_index));
            }
            frames.get_mut(frame_key).lane_edge_geometries = TableRange::try_from_usize(
                geometry_start,
                geometries.len().saturating_sub(geometry_start),
            )
            .map_err(|overflow| {
                arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
            })?;
        }
    }

    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    if !geometries.is_empty() {
        for (index, binding) in edge_bindings.iter().enumerate() {
            if binding.is_none() {
                let edge = lane_edges.get(HirLaneEdgeKey::from_raw(
                    u32::try_from(index).expect("LaneEdge arena length is bounded by u32"),
                ));
                diagnostics.push(Diagnostic::invalid_spatial_geometry(
                    None,
                    &edge.stable_key,
                    None,
                    SpatialGeometryViolation::MissingEdgeBinding,
                    edge.source_span.clone(),
                    None,
                ));
            }
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    for (edge_key, edge) in lane_edges.iter() {
        let Some((frame, geometry_index)) = edge_bindings[edge_key.index()] else {
            continue;
        };
        let geometry = &geometries[geometry_index];
        let end = points[geometry.points.as_usize_range().end - 1];
        for successor in &lane_edge_references[edge.successors.as_usize_range()] {
            let (successor_frame, successor_geometry_index) = edge_bindings
                [successor.target.index()]
            .expect("complete spatial coverage must bind every successor");
            let successor_geometry = &geometries[successor_geometry_index];
            let successor_edge = lane_edges.get(successor.target);
            if frame != successor_frame {
                diagnostics.push(Diagnostic::invalid_spatial_geometry(
                    Some(&frames.get(frame).stable_key),
                    &edge.stable_key,
                    Some(&successor_edge.stable_key),
                    SpatialGeometryViolation::ConnectedEdgesUseDifferentFrames,
                    geometry.source_span.clone(),
                    Some(successor.source_span.clone()),
                ));
                continue;
            }
            let start = points[successor_geometry.points.as_usize_range().start];
            let distance = (start.x - end.x)
                .hypot(start.y - end.y)
                .hypot(start.z - end.z);
            if distance > SPATIAL_JOIN_POSITION_TOLERANCE_METERS {
                diagnostics.push(Diagnostic::invalid_spatial_geometry(
                    Some(&frames.get(frame).stable_key),
                    &edge.stable_key,
                    Some(&successor_edge.stable_key),
                    SpatialGeometryViolation::DiscontinuousJoin {
                        distance_bits: distance.to_bits(),
                        tolerance_bits: SPATIAL_JOIN_POSITION_TOLERANCE_METERS.to_bits(),
                    },
                    geometry.source_span.clone(),
                    Some(successor.source_span.clone()),
                ));
            }
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    Ok(SpatialHir {
        canonical_frames: frames.into_boxed_slice(),
        lane_edge_geometries: geometries.into_boxed_slice(),
        canonical_points: points.into_boxed_slice(),
        spatial_segments: segments.into_boxed_slice(),
    })
}

fn normalize_spatial_vector(vector: [f32; 3]) -> [f32; 3] {
    let scale = vector[0].abs().max(vector[1].abs()).max(vector[2].abs());
    debug_assert!(scale > 0.0, "validated spatial direction must be non-zero");
    let scaled = [vector[0] / scale, vector[1] / scale, vector[2] / scale];
    let scaled_length = scaled[0].hypot(scaled[1]).hypot(scaled[2]);
    [
        canonicalize_spatial_zero(scaled[0] / scaled_length),
        canonicalize_spatial_zero(scaled[1] / scaled_length),
        canonicalize_spatial_zero(scaled[2] / scaled_length),
    ]
}

const fn canonicalize_spatial_zero(value: f32) -> f32 {
    if value == 0.0 { 0.0 } else { value }
}

#[allow(clippy::too_many_lines)]
fn build_parking_hir(
    unit: &CompilationUnit,
    module_lookup: &HashMap<Arc<str>, HirModuleKey>,
    lane_edges: &TypedArena<HirLaneEdgeTag, HirLaneEdge>,
    lane_edge_symbols: &SymbolTable<HirLaneEdgeKey>,
    identities: &mut IdentityRegistry,
) -> Result<ParkingHir, DiagnosticBundle> {
    let counts = parking_counts(unit);
    if counts.entity_count() == 0 {
        return Ok(ParkingHir::default());
    }

    let mut areas = TypedArena::<HirParkingAreaTag, HirParkingArea>::with_capacity(count_to_usize(
        counts.areas,
        &unit.limits,
    )?);
    let mut spaces = TypedArena::<HirParkingSpaceTag, HirParkingSpace>::with_capacity(
        count_to_usize(counts.spaces, &unit.limits)?,
    );
    let mut area_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, TypedAstDeclaration::ParkingArea(_)))
            .count()
    }));
    let mut area_sources = Vec::with_capacity(count_to_usize(counts.areas, &unit.limits)?);
    let mut space_sources =
        Vec::<(u32, u32)>::with_capacity(count_to_usize(counts.spaces, &unit.limits)?);

    // ParkingArea 必须先完整登记，ParkingSpace 的可选归属因而允许前向和跨模块引用。
    // 两类实体仍分别按模块和稳定键规范排序，来源声明顺序不会进入身份或布局语义。
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key = HirModuleKey::from_raw(
            u32::try_from(module_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        let module_order = u32::try_from(module_index).unwrap_or(u32::MAX);
        let mut area_indices: Vec<_> = source_module
            .declarations
            .iter()
            .enumerate()
            .filter_map(|(index, declaration)| {
                matches!(declaration, TypedAstDeclaration::ParkingArea(_)).then_some(index)
            })
            .collect();
        area_indices.sort_unstable_by(|left, right| {
            declaration_header(&source_module.declarations[*left])
                .stable_key
                .cmp(&declaration_header(&source_module.declarations[*right]).stable_key)
        });
        for declaration_index in area_indices {
            let TypedAstDeclaration::ParkingArea(source) =
                &source_module.declarations[declaration_index]
            else {
                unreachable!("parking area source filter admitted unrelated declaration")
            };
            let fields = [
                IdentityFieldInput::new(
                    FieldTag::AuthoringNamespaceId,
                    source_module
                        .descriptor()
                        .authoring_namespace_id()
                        .as_bytes(),
                ),
                IdentityFieldInput::new(
                    FieldTag::ParkingAreaKey,
                    source.header.stable_key.as_bytes(),
                ),
            ];
            let stable_id = ParkingAreaId::from_untyped(derive_identity(
                unit,
                identities,
                module_index,
                EntityKind::ParkingArea,
                &source.header.stable_key,
                &source.header.span,
                &fields,
            )?);
            let key = areas
                .push(HirParkingArea {
                    module: module_key,
                    stable_key: Arc::clone(&source.header.stable_key),
                    stable_id,
                    parking_spaces: TableRange::empty(),
                    source_span: source.header.span.clone(),
                })
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                })?;
            area_symbols.insert(module_key, Arc::clone(&source.header.stable_key), key);
            area_sources.push(CanonicalDeclarationSource {
                source_module_index: module_order,
                declaration_index: u32::try_from(declaration_index)
                    .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
                hir_key: key,
            });
        }

        let mut indices: Vec<_> = source_module
            .declarations
            .iter()
            .enumerate()
            .filter_map(|(index, declaration)| {
                matches!(declaration, TypedAstDeclaration::ParkingSpace(_)).then_some(index)
            })
            .collect();
        indices.sort_unstable_by(|left, right| {
            declaration_header(&source_module.declarations[*left])
                .stable_key
                .cmp(&declaration_header(&source_module.declarations[*right]).stable_key)
        });
        for declaration_index in indices {
            space_sources.push((
                module_order,
                u32::try_from(declaration_index)
                    .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
            ));
        }
    }

    let mut diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
    let mut area_has_member = vec![false; areas.len()];
    let mut memberships = Vec::<(HirParkingAreaKey, HirParkingSpaceKey)>::with_capacity(
        count_to_usize(counts.memberships, &unit.limits)?,
    );

    for (module_order, declaration_index) in space_sources {
        let module_index = module_order as usize;
        let source_module = &unit.modules[module_index];
        let TypedAstDeclaration::ParkingSpace(source) =
            &source_module.declarations[declaration_index as usize]
        else {
            unreachable!("canonical ParkingSpace source changed kind")
        };
        let module_key = HirModuleKey::from_raw(module_order);
        let fields = [
            IdentityFieldInput::new(
                FieldTag::AuthoringNamespaceId,
                source_module
                    .descriptor()
                    .authoring_namespace_id()
                    .as_bytes(),
            ),
            IdentityFieldInput::new(
                FieldTag::ParkingSpaceKey,
                source.header.stable_key.as_bytes(),
            ),
        ];
        let stable_id = ParkingSpaceId::from_untyped(derive_identity(
            unit,
            identities,
            module_index,
            EntityKind::ParkingSpace,
            &source.header.stable_key,
            &source.header.span,
            &fields,
        )?);

        let parking_area = source.parking_area.as_ref().and_then(|reference| {
            let area = resolve_reference(
                module_lookup,
                &area_symbols,
                reference,
                EntityKind::ParkingSpace,
                &source.header,
                module_order,
                &mut diagnostics,
            );
            if let Some(area) = area {
                // 区域孤立性由声明关系判断；成员自己的其他字段失败不应产生级联 orphan。
                area_has_member[area.index()] = true;
            }
            area
        });
        let entry_edge = resolve_reference(
            module_lookup,
            lane_edge_symbols,
            &source.entry.lane_edge,
            EntityKind::ParkingSpace,
            &source.header,
            module_order,
            &mut diagnostics,
        );
        let exit_edge = resolve_reference(
            module_lookup,
            lane_edge_symbols,
            &source.exit.lane_edge,
            EntityKind::ParkingSpace,
            &source.header,
            module_order,
            &mut diagnostics,
        );

        for (role, anchor, edge) in [
            (ParkingAnchorRole::Entry, &source.entry, entry_edge),
            (ParkingAnchorRole::Exit, &source.exit, exit_edge),
        ] {
            let Some(edge) = edge else { continue };
            let edge_length = lane_edges.get(edge).length_meters;
            let progress = anchor.progress_meters;
            if !progress.is_finite()
                || progress <= PARKING_ANCHOR_ENDPOINT_CLEARANCE_METERS
                || progress >= edge_length - PARKING_ANCHOR_ENDPOINT_CLEARANCE_METERS
            {
                let mut diagnostic = Diagnostic::invalid_parking_anchor_progress(
                    &source.header.stable_key,
                    role,
                    &lane_edges.get(edge).stable_key,
                    progress,
                    edge_length,
                    PARKING_ANCHOR_ENDPOINT_CLEARANCE_METERS,
                    anchor.lane_edge.span.clone(),
                );
                diagnostic.set_canonical_module_order(module_order);
                diagnostics.push(diagnostic);
            }
        }

        let geometry = source.geometry;
        for (field, value, violation) in [
            (
                ParkingGeometryField::LateralOffsetMeters,
                geometry.lateral_offset_meters,
                if !geometry.lateral_offset_meters.is_finite() {
                    Some(ParkingGeometryViolation::NotFinite)
                } else if geometry.lateral_offset_meters.abs()
                    <= MIN_PARKING_LATERAL_OFFSET_ABS_EXCLUSIVE_METERS
                {
                    Some(ParkingGeometryViolation::AbsoluteNotGreaterThan {
                        exclusive_minimum_bits: MIN_PARKING_LATERAL_OFFSET_ABS_EXCLUSIVE_METERS
                            .to_bits(),
                    })
                } else {
                    None
                },
            ),
            (
                ParkingGeometryField::HeadingOffsetRadians,
                geometry.heading_offset_radians,
                if !geometry.heading_offset_radians.is_finite() {
                    Some(ParkingGeometryViolation::NotFinite)
                } else if !(PARKING_HEADING_OFFSET_MINIMUM_RADIANS
                    ..PARKING_HEADING_OFFSET_MAXIMUM_RADIANS)
                    .contains(&geometry.heading_offset_radians)
                {
                    Some(ParkingGeometryViolation::OutsideHalfOpenRange {
                        minimum_inclusive_bits: PARKING_HEADING_OFFSET_MINIMUM_RADIANS.to_bits(),
                        maximum_exclusive_bits: PARKING_HEADING_OFFSET_MAXIMUM_RADIANS.to_bits(),
                    })
                } else {
                    None
                },
            ),
            (
                ParkingGeometryField::LengthMeters,
                geometry.length_meters,
                parking_extent_violation(geometry.length_meters),
            ),
            (
                ParkingGeometryField::WidthMeters,
                geometry.width_meters,
                parking_extent_violation(geometry.width_meters),
            ),
        ] {
            if let Some(violation) = violation {
                let mut diagnostic = Diagnostic::invalid_parking_space_geometry(
                    &source.header.stable_key,
                    field,
                    value,
                    violation,
                    source.header.span.clone(),
                );
                diagnostic.set_canonical_module_order(module_order);
                diagnostics.push(diagnostic);
            }
        }

        let (Some(entry_edge), Some(exit_edge)) = (entry_edge, exit_edge) else {
            continue;
        };
        let parking_area_source_location = match (&source.parking_area, parking_area) {
            (Some(reference), Some(_)) => {
                Some(unit.resolve_source_location_for_module(module_order, &reference.span)?)
            }
            _ => None,
        };
        let space_key = spaces
            .push(HirParkingSpace {
                module: module_key,
                stable_key: Arc::clone(&source.header.stable_key),
                stable_id,
                parking_area,
                parking_area_source_location,
                entry: HirParkingLaneAnchor {
                    lane_edge: entry_edge,
                    progress_meters: source.entry.progress_meters,
                    source_location: unit.resolve_source_location_for_module(
                        module_order,
                        &source.entry.lane_edge.span,
                    )?,
                },
                exit: HirParkingLaneAnchor {
                    lane_edge: exit_edge,
                    progress_meters: source.exit.progress_meters,
                    source_location: unit.resolve_source_location_for_module(
                        module_order,
                        &source.exit.lane_edge.span,
                    )?,
                },
                geometry: HirParkingSpaceGeometry {
                    lateral_offset_meters: geometry.lateral_offset_meters,
                    heading_offset_radians: geometry.heading_offset_radians,
                    length_meters: geometry.length_meters,
                    width_meters: geometry.width_meters,
                },
                source_span: source.header.span.clone(),
            })
            .map_err(|overflow| {
                arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
            })?;
        if let Some(area) = parking_area {
            memberships.push((area, space_key));
        }
    }

    for location in &area_sources {
        if !area_has_member[location.hir_key.index()] {
            let area = areas.get(location.hir_key);
            let mut diagnostic =
                Diagnostic::orphan_parking_area(&area.stable_key, area.source_span.clone());
            diagnostic.set_canonical_module_order(location.source_module_index);
            diagnostics.push(diagnostic);
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    // 归属是集合语义。按区域 StableId、停车位 StableId 冻结反向成员表，避免来源排列
    // 改变 LIR 或语义摘要；独立停车位不会出现在该表中。
    memberships.sort_unstable_by_key(|(area, space)| {
        (areas.get(*area).stable_id, spaces.get(*space).stable_id)
    });
    let area_spaces = memberships
        .iter()
        .map(|(_, parking_space)| HirParkingAreaSpace {
            parking_space: *parking_space,
        })
        .collect::<Vec<_>>();
    let mut area_ranges = vec![(0_usize, 0_usize); areas.len()];
    let mut cursor = 0_usize;
    while cursor < memberships.len() {
        let area = memberships[cursor].0;
        let start = cursor;
        while cursor < memberships.len() && memberships[cursor].0 == area {
            cursor = cursor.saturating_add(1);
        }
        area_ranges[area.index()] = (start, cursor.saturating_sub(start));
    }
    for (area_index, (start, count)) in area_ranges.iter().copied().enumerate() {
        let area_key = HirParkingAreaKey::from_raw(
            u32::try_from(area_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        let source_span = areas.get(area_key).source_span.clone();
        areas.get_mut(area_key).parking_spaces = TableRange::try_from_usize(start, count)
            .map_err(|overflow| arena_overflow(overflow, &unit.limits, Some(source_span)))?;
    }

    Ok(ParkingHir {
        parking_areas: areas.into_boxed_slice(),
        parking_spaces: spaces.into_boxed_slice(),
        parking_area_spaces: area_spaces.into_boxed_slice(),
    })
}

fn parking_extent_violation(value: f64) -> Option<ParkingGeometryViolation> {
    if !value.is_finite() {
        Some(ParkingGeometryViolation::NotFinite)
    } else if value <= MIN_PARKING_EXTENT_EXCLUSIVE_METERS {
        Some(ParkingGeometryViolation::NotGreaterThan {
            exclusive_minimum_bits: MIN_PARKING_EXTENT_EXCLUSIVE_METERS.to_bits(),
        })
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn build_access_hir(
    unit: &CompilationUnit,
    module_lookup: &HashMap<Arc<str>, HirModuleKey>,
    lane_edges: &TypedArena<HirLaneEdgeTag, HirLaneEdge>,
    cross_section: &CrossSectionHir,
    maneuver_paths: &[HirManeuverPath],
    identities: &mut IdentityRegistry,
) -> Result<AccessHir, DiagnosticBundle> {
    let counts = access_counts(unit);
    if counts.entity_count() == 0 {
        return Ok(AccessHir::default());
    }

    let mut class_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|item| matches!(item, TypedAstDeclaration::ParticipantClass(_)))
            .count()
    }));
    let mut edge_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|item| matches!(item, TypedAstDeclaration::LaneEdge(_)))
            .count()
    }));
    let mut group_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|item| matches!(item, TypedAstDeclaration::LaneGroup(_)))
            .count()
    }));
    let mut section_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|item| matches!(item, TypedAstDeclaration::RoadSection(_)))
            .count()
    }));
    let mut path_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|item| matches!(item, TypedAstDeclaration::ManeuverPath(_)))
            .count()
    }));
    let mut band_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|item| matches!(item, TypedAstDeclaration::FacilityBand(_)))
            .count()
    }));
    for (key, edge) in lane_edges.iter() {
        edge_symbols.insert(edge.module, Arc::clone(&edge.stable_key), key);
    }
    for (index, group) in cross_section.lane_groups.iter().enumerate() {
        group_symbols.insert(
            group.module,
            Arc::clone(&group.stable_key),
            HirLaneGroupKey::from_raw(u32::try_from(index).expect("HIR table is u32-bounded")),
        );
    }
    for (index, section) in cross_section.road_sections.iter().enumerate() {
        section_symbols.insert(
            section.module,
            Arc::clone(&section.stable_key),
            HirRoadSectionKey::from_raw(u32::try_from(index).expect("HIR table is u32-bounded")),
        );
    }
    for (index, path) in maneuver_paths.iter().enumerate() {
        path_symbols.insert(
            path.module,
            Arc::clone(&path.stable_key),
            HirManeuverPathKey::from_raw(u32::try_from(index).expect("HIR table is u32-bounded")),
        );
    }
    for (index, band) in cross_section.facility_bands.iter().enumerate() {
        band_symbols.insert(
            band.module,
            Arc::clone(&band.stable_key),
            HirFacilityBandKey::from_raw(u32::try_from(index).expect("HIR table is u32-bounded")),
        );
    }

    let mut classes = TypedArena::<HirParticipantClassTag, HirParticipantClass>::with_capacity(
        count_to_usize(counts.participant_classes, &unit.limits)?,
    );
    let mut class_sources =
        Vec::with_capacity(count_to_usize(counts.participant_classes, &unit.limits)?);
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key = HirModuleKey::from_raw(
            u32::try_from(module_index).expect("module table is u32-bounded"),
        );
        let mut declaration_indices: Vec<_> = source_module
            .declarations
            .iter()
            .enumerate()
            .filter_map(|(index, declaration)| {
                matches!(declaration, TypedAstDeclaration::ParticipantClass(_)).then_some(index)
            })
            .collect();
        declaration_indices.sort_unstable_by(|left, right| {
            declaration_header(&source_module.declarations[*left])
                .stable_key
                .cmp(&declaration_header(&source_module.declarations[*right]).stable_key)
        });
        for declaration_index in declaration_indices {
            let TypedAstDeclaration::ParticipantClass(source) =
                &source_module.declarations[declaration_index]
            else {
                unreachable!("filtered declaration must be ParticipantClass");
            };
            let fields = [
                IdentityFieldInput::new(
                    FieldTag::AuthoringNamespaceId,
                    source_module
                        .descriptor()
                        .authoring_namespace_id()
                        .as_bytes(),
                ),
                IdentityFieldInput::new(
                    FieldTag::ParticipantClassKey,
                    source.header.stable_key.as_bytes(),
                ),
            ];
            let stable_id = ParticipantClassId::from_untyped(derive_identity(
                unit,
                identities,
                module_index,
                EntityKind::ParticipantClass,
                &source.header.stable_key,
                &source.header.span,
                &fields,
            )?);
            let key = classes
                .push(HirParticipantClass {
                    module: module_key,
                    stable_key: Arc::clone(&source.header.stable_key),
                    stable_id,
                    parent: None,
                    parent_source_span: None,
                    depth: 0,
                    subtree_enter: 0,
                    subtree_exit: 0,
                    source_span: source.header.span.clone(),
                })
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                })?;
            class_symbols.insert(module_key, Arc::clone(&source.header.stable_key), key);
            class_sources.push(CanonicalDeclarationSource {
                source_module_index: u32::try_from(module_index)
                    .expect("module table is u32-bounded"),
                declaration_index: u32::try_from(declaration_index)
                    .expect("declaration table is u32-bounded"),
                hir_key: key,
            });
        }
    }

    let mut diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
    for location in &class_sources {
        let module_index = usize::try_from(location.source_module_index)
            .expect("u32 module index must fit usize on supported targets");
        let declaration_index = usize::try_from(location.declaration_index)
            .expect("u32 declaration index must fit usize on supported targets");
        let TypedAstDeclaration::ParticipantClass(source) =
            &unit.modules[module_index].declarations[declaration_index]
        else {
            unreachable!("canonical class source must still name ParticipantClass");
        };
        if let Some(parent) = &source.extends {
            classes.get_mut(location.hir_key).parent = resolve_reference(
                module_lookup,
                &class_symbols,
                parent,
                EntityKind::ParticipantClass,
                &source.header,
                location.source_module_index,
                &mut diagnostics,
            );
            classes.get_mut(location.hir_key).parent_source_span = Some(parent.span.clone());
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    // 单继承链用三色迭代遍历检测，避免极深分类法消耗线程栈。
    let mut state = vec![0_u8; classes.len()];
    for start_index in 0..classes.len() {
        if state[start_index] != 0 {
            continue;
        }
        let mut path = Vec::new();
        let mut cursor = HirParticipantClassKey::from_raw(
            u32::try_from(start_index).expect("HIR table is u32-bounded"),
        );
        let mut cycle_cursor = None;
        while state[cursor.index()] == 0 {
            state[cursor.index()] = 1;
            path.push(cursor);
            let Some(parent) = classes.get(cursor).parent else {
                break;
            };
            cursor = parent;
        }
        // 无父类的根节点也处于本轮的 visiting 状态；只有沿 parent 边重新进入
        // visiting 节点才构成环，不能仅凭最终状态判断。
        if classes
            .get(*path.last().expect("fresh traversal is non-empty"))
            .parent
            .is_some()
            && state[cursor.index()] == 1
        {
            cycle_cursor = Some(cursor);
        }
        if let Some(cursor) = cycle_cursor {
            let cycle_start = path.iter().position(|key| *key == cursor).unwrap_or(0);
            let cycle = &path[cycle_start..];
            let representative = cycle
                .iter()
                .copied()
                .min_by(|left, right| {
                    classes
                        .get(*left)
                        .stable_key
                        .cmp(&classes.get(*right).stable_key)
                })
                .expect("active traversal contains its cycle cursor");
            let related_spans = cycle
                .iter()
                .filter(|key| **key != representative)
                .map(|key| classes.get(*key).source_span.clone())
                .collect::<Vec<_>>()
                .into_boxed_slice();
            let mut diagnostic = Diagnostic::participant_class_inheritance_cycle(
                &classes.get(representative).stable_key,
                classes.get(representative).source_span.clone(),
                related_spans,
            );
            diagnostic.set_canonical_module_order(classes.get(representative).module.raw());
            diagnostics.push(diagnostic);
        }
        for key in path.into_iter().rev() {
            if classes
                .get(key)
                .parent
                .is_none_or(|parent| state[parent.index()] == 2)
            {
                classes.get_mut(key).depth = classes
                    .get(key)
                    .parent
                    .map_or(0, |parent| classes.get(parent).depth.saturating_add(1));
            }
            state[key.index()] = 2;
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    // first-child/next-sibling 数组让层级闭包只需连续内存，不为每个类别单独分配 Vec。
    let mut first_child = vec![None; classes.len()];
    let mut next_sibling = vec![None; classes.len()];
    let mut first_root = None;
    for (index, sibling_slot) in next_sibling.iter_mut().enumerate() {
        let key = HirParticipantClassKey::from_raw(
            u32::try_from(index).expect("HIR table is u32-bounded"),
        );
        if let Some(parent) = classes.get(key).parent {
            *sibling_slot = first_child[parent.index()];
            first_child[parent.index()] = Some(key);
        } else {
            *sibling_slot = first_root;
            first_root = Some(key);
        }
    }
    let mut stack = Vec::with_capacity(classes.len().saturating_mul(2));
    let mut root = first_root;
    while let Some(key) = root {
        stack.push((key, false));
        root = next_sibling[key.index()];
    }
    let mut euler = 0_u32;
    while let Some((key, exiting)) = stack.pop() {
        if exiting {
            classes.get_mut(key).subtree_exit = euler;
            continue;
        }
        classes.get_mut(key).subtree_enter = euler;
        euler = euler.checked_add(1).ok_or_else(|| {
            arena_overflow(
                ArenaKeyOverflow,
                &unit.limits,
                Some(classes.get(key).source_span.clone()),
            )
        })?;
        stack.push((key, true));
        let mut child = first_child[key.index()];
        while let Some(child_key) = child {
            stack.push((child_key, false));
            child = next_sibling[child_key.index()];
        }
    }

    // VehicleProfile 只消费已经闭合的分类法；它不会反向改变类别层级或把车辆参数
    // 提升为跨执行域能力。先登记规范身份，再统一解析类别，保留前向/跨模块引用。
    let mut profiles = TypedArena::<HirVehicleProfileTag, HirVehicleProfile>::with_capacity(
        count_to_usize(counts.vehicle_profiles, &unit.limits)?,
    );
    let mut profile_sources =
        Vec::with_capacity(count_to_usize(counts.vehicle_profiles, &unit.limits)?);
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key = HirModuleKey::from_raw(
            u32::try_from(module_index).expect("module table is u32-bounded"),
        );
        let mut declaration_indices: Vec<_> = source_module
            .declarations
            .iter()
            .enumerate()
            .filter_map(|(index, declaration)| {
                matches!(declaration, TypedAstDeclaration::VehicleProfile(_)).then_some(index)
            })
            .collect();
        declaration_indices.sort_unstable_by(|left, right| {
            declaration_header(&source_module.declarations[*left])
                .stable_key
                .cmp(&declaration_header(&source_module.declarations[*right]).stable_key)
        });
        for declaration_index in declaration_indices {
            let TypedAstDeclaration::VehicleProfile(source) =
                &source_module.declarations[declaration_index]
            else {
                unreachable!("filtered declaration must be VehicleProfile");
            };
            let fields = [
                IdentityFieldInput::new(
                    FieldTag::AuthoringNamespaceId,
                    source_module
                        .descriptor()
                        .authoring_namespace_id()
                        .as_bytes(),
                ),
                IdentityFieldInput::new(
                    FieldTag::VehicleProfileKey,
                    source.header.stable_key.as_bytes(),
                ),
            ];
            let stable_id = VehicleProfileId::from_untyped(derive_identity(
                unit,
                identities,
                module_index,
                EntityKind::VehicleProfile,
                &source.header.stable_key,
                &source.header.span,
                &fields,
            )?);
            let iidm = source.iidm;
            let key = profiles
                .push(HirVehicleProfile {
                    module: module_key,
                    stable_key: Arc::clone(&source.header.stable_key),
                    stable_id,
                    participant_class: HirParticipantClassKey::from_raw(0),
                    participant_class_source_span: source.participant_class.span.clone(),
                    length_meters: iidm.length_meters,
                    desired_speed_meters_per_second: iidm.desired_speed_meters_per_second,
                    min_gap_meters: iidm.min_gap_meters,
                    time_headway_seconds: iidm.time_headway_seconds,
                    max_acceleration_meters_per_second_squared: iidm
                        .max_acceleration_meters_per_second_squared,
                    comfortable_deceleration_meters_per_second_squared: iidm
                        .comfortable_deceleration_meters_per_second_squared,
                    emergency_deceleration_meters_per_second_squared: iidm
                        .emergency_deceleration_meters_per_second_squared,
                    source_span: source.header.span.clone(),
                })
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                })?;
            profile_sources.push(CanonicalDeclarationSource {
                source_module_index: u32::try_from(module_index)
                    .expect("module table is u32-bounded"),
                declaration_index: u32::try_from(declaration_index)
                    .expect("declaration table is u32-bounded"),
                hir_key: key,
            });
        }
    }
    for location in &profile_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let TypedAstDeclaration::VehicleProfile(source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical vehicle profile source changed kind");
        };
        if let Some(participant_class) = resolve_reference(
            module_lookup,
            &class_symbols,
            &source.participant_class,
            EntityKind::VehicleProfile,
            &source.header,
            location.source_module_index,
            &mut diagnostics,
        ) {
            profiles.get_mut(location.hir_key).participant_class = participant_class;
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    let mut rules = TypedArena::<HirAccessRuleTag, HirAccessRule>::with_capacity(count_to_usize(
        counts.access_rules,
        &unit.limits,
    )?);
    let mut rule_sources = Vec::with_capacity(count_to_usize(counts.access_rules, &unit.limits)?);
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key = HirModuleKey::from_raw(
            u32::try_from(module_index).expect("module table is u32-bounded"),
        );
        let mut declaration_indices: Vec<_> = source_module
            .declarations
            .iter()
            .enumerate()
            .filter_map(|(index, declaration)| {
                matches!(declaration, TypedAstDeclaration::AccessRule(_)).then_some(index)
            })
            .collect();
        declaration_indices.sort_unstable_by(|left, right| {
            declaration_header(&source_module.declarations[*left])
                .stable_key
                .cmp(&declaration_header(&source_module.declarations[*right]).stable_key)
        });
        for declaration_index in declaration_indices {
            let TypedAstDeclaration::AccessRule(source) =
                &source_module.declarations[declaration_index]
            else {
                unreachable!("filtered declaration must be AccessRule");
            };
            let fields = [
                IdentityFieldInput::new(
                    FieldTag::AuthoringNamespaceId,
                    source_module
                        .descriptor()
                        .authoring_namespace_id()
                        .as_bytes(),
                ),
                IdentityFieldInput::new(
                    FieldTag::AccessRuleKey,
                    source.header.stable_key.as_bytes(),
                ),
            ];
            let stable_id = AccessRuleId::from_untyped(derive_identity(
                unit,
                identities,
                module_index,
                EntityKind::AccessRule,
                &source.header.stable_key,
                &source.header.span,
                &fields,
            )?);
            let key = rules
                .push(HirAccessRule {
                    module: module_key,
                    stable_key: Arc::clone(&source.header.stable_key),
                    stable_id,
                    // 仅作首遍占位；目标解析失败时整个 HIR 不会提交。
                    target: HirAccessTarget::LaneEdge(HirLaneEdgeKey::from_raw(0)),
                    target_source_span: source.header.span.clone(),
                    effect: source.effect,
                    participant_classes: TableRange::empty(),
                    regulation: None,
                    priority: source.priority,
                    source_span: source.header.span.clone(),
                })
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                })?;
            rule_sources.push(CanonicalDeclarationSource {
                source_module_index: u32::try_from(module_index)
                    .expect("module table is u32-bounded"),
                declaration_index: u32::try_from(declaration_index)
                    .expect("declaration table is u32-bounded"),
                hir_key: key,
            });
        }
    }

    let mut rule_classes =
        Vec::with_capacity(count_to_usize(counts.rule_class_references, &unit.limits)?);
    let mut first_regulation: Option<FirstAccessRegulation> = None;
    for location in &rule_sources {
        let module_index = usize::try_from(location.source_module_index)
            .expect("u32 module index must fit usize on supported targets");
        let declaration_index = usize::try_from(location.declaration_index)
            .expect("u32 declaration index must fit usize on supported targets");
        let TypedAstDeclaration::AccessRule(source) =
            &unit.modules[module_index].declarations[declaration_index]
        else {
            unreachable!("canonical rule source must still name AccessRule");
        };
        let target = resolve_access_target(
            module_lookup,
            &edge_symbols,
            &group_symbols,
            &section_symbols,
            &path_symbols,
            &band_symbols,
            source,
            location.source_module_index,
            &mut diagnostics,
        );
        if let Some(target) = target {
            rules.get_mut(location.hir_key).target = target;
            rules.get_mut(location.hir_key).target_source_span = access_target_source_span(source);
        }

        if source.participant_classes.is_empty() {
            let mut diagnostic = Diagnostic::empty_access_rule_participant_classes(
                &source.header.stable_key,
                source.header.span.clone(),
            );
            diagnostic.set_canonical_module_order(location.source_module_index);
            diagnostics.push(diagnostic);
        }
        let start = rule_classes.len();
        let mut resolved_classes = Vec::with_capacity(source.participant_classes.len());
        for reference in &source.participant_classes {
            if let Some(participant_class) = resolve_reference(
                module_lookup,
                &class_symbols,
                reference,
                EntityKind::AccessRule,
                &source.header,
                location.source_module_index,
                &mut diagnostics,
            ) {
                resolved_classes.push((participant_class, reference.span.clone()));
            }
        }
        resolved_classes.sort_unstable_by_key(|(participant_class, _)| *participant_class);
        resolved_classes.dedup_by_key(|(participant_class, _)| *participant_class);
        rule_classes.extend(resolved_classes.into_iter().map(
            |(participant_class, source_span)| HirAccessRuleParticipantClass {
                participant_class,
                source_span,
            },
        ));
        rules.get_mut(location.hir_key).participant_classes =
            TableRange::try_from_usize(start, rule_classes.len().saturating_sub(start)).map_err(
                |overflow| arena_overflow(overflow, &unit.limits, Some(source.header.span.clone())),
            )?;

        if let Some(regulation) = &source.regulation {
            let valid = validate_access_regulation(
                regulation,
                source,
                location.source_module_index,
                &mut diagnostics,
            );
            if valid {
                if let Some(first) = &first_regulation {
                    if first.jurisdiction.as_ref() != regulation.jurisdiction.as_ref()
                        || first.version.as_ref() != regulation.version.as_ref()
                    {
                        let mut diagnostic = Diagnostic::access_regulation_mismatch(
                            &first.rule_key,
                            &first.jurisdiction,
                            &first.version,
                            &source.header.stable_key,
                            &regulation.jurisdiction,
                            &regulation.version,
                            source.header.span.clone(),
                            first.source_span.clone(),
                        );
                        diagnostic.set_canonical_module_order(location.source_module_index);
                        diagnostics.push(diagnostic);
                    }
                } else {
                    first_regulation = Some(FirstAccessRegulation {
                        jurisdiction: Arc::clone(&regulation.jurisdiction),
                        version: Arc::clone(&regulation.version),
                        rule_key: Arc::clone(&source.header.stable_key),
                        source_span: source.header.span.clone(),
                    });
                }
                rules.get_mut(location.hir_key).regulation = Some(HirAccessRegulation {
                    jurisdiction: Arc::clone(&regulation.jurisdiction),
                    version: Arc::clone(&regulation.version),
                    source: regulation.source.clone(),
                });
            }
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    validate_access_ambiguity(
        unit,
        lane_edges,
        cross_section,
        maneuver_paths,
        &classes,
        &rules,
        &rule_classes,
        &mut diagnostics,
    )?;
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    Ok(AccessHir {
        participant_classes: classes.into_boxed_slice(),
        vehicle_profiles: profiles.into_boxed_slice(),
        access_rules: rules.into_boxed_slice(),
        access_rule_participant_classes: rule_classes.into_boxed_slice(),
    })
}

#[allow(clippy::too_many_arguments)]
fn resolve_access_target(
    module_lookup: &HashMap<Arc<str>, HirModuleKey>,
    edge_symbols: &SymbolTable<HirLaneEdgeKey>,
    group_symbols: &SymbolTable<HirLaneGroupKey>,
    section_symbols: &SymbolTable<HirRoadSectionKey>,
    path_symbols: &SymbolTable<HirManeuverPathKey>,
    band_symbols: &SymbolTable<HirFacilityBandKey>,
    source: &crate::declaration::AccessRuleDeclaration,
    module_order: u32,
    diagnostics: &mut DiagnosticCollector,
) -> Option<HirAccessTarget> {
    match &source.target {
        OwnedAccessRuleTarget::LaneEdge(reference) => resolve_reference(
            module_lookup,
            edge_symbols,
            reference,
            EntityKind::AccessRule,
            &source.header,
            module_order,
            diagnostics,
        )
        .map(HirAccessTarget::LaneEdge),
        OwnedAccessRuleTarget::LaneGroup(reference) => resolve_reference(
            module_lookup,
            group_symbols,
            reference,
            EntityKind::AccessRule,
            &source.header,
            module_order,
            diagnostics,
        )
        .map(HirAccessTarget::LaneGroup),
        OwnedAccessRuleTarget::RoadSection(reference) => resolve_reference(
            module_lookup,
            section_symbols,
            reference,
            EntityKind::AccessRule,
            &source.header,
            module_order,
            diagnostics,
        )
        .map(HirAccessTarget::RoadSection),
        OwnedAccessRuleTarget::ManeuverPath(reference) => resolve_reference(
            module_lookup,
            path_symbols,
            reference,
            EntityKind::AccessRule,
            &source.header,
            module_order,
            diagnostics,
        )
        .map(HirAccessTarget::ManeuverPath),
        OwnedAccessRuleTarget::FacilityBand(reference) => {
            if resolve_reference(
                module_lookup,
                band_symbols,
                reference,
                EntityKind::AccessRule,
                &source.header,
                module_order,
                diagnostics,
            )
            .is_some()
            {
                let mut diagnostic = Diagnostic::access_capability_unavailable(
                    &source.header.stable_key,
                    AccessCapability::FacilityBandTarget,
                    reference.span.clone(),
                );
                diagnostic.set_canonical_module_order(module_order);
                diagnostics.push(diagnostic);
            }
            None
        }
    }
}

fn access_target_source_span(source: &crate::declaration::AccessRuleDeclaration) -> SourceLocation {
    match &source.target {
        OwnedAccessRuleTarget::LaneEdge(reference) => reference.span.clone(),
        OwnedAccessRuleTarget::LaneGroup(reference) => reference.span.clone(),
        OwnedAccessRuleTarget::RoadSection(reference) => reference.span.clone(),
        OwnedAccessRuleTarget::ManeuverPath(reference) => reference.span.clone(),
        OwnedAccessRuleTarget::FacilityBand(reference) => reference.span.clone(),
    }
}

fn validate_access_regulation(
    regulation: &OwnedAccessRegulation,
    source: &crate::declaration::AccessRuleDeclaration,
    module_order: u32,
    diagnostics: &mut DiagnosticCollector,
) -> bool {
    let mut valid = true;
    for (field, value) in [
        (
            AccessRegulationField::Jurisdiction,
            regulation.jurisdiction.as_ref(),
        ),
        (AccessRegulationField::Version, regulation.version.as_ref()),
    ] {
        let character_count = u32::try_from(value.chars().count()).unwrap_or(u32::MAX);
        if !(1..=128).contains(&character_count) {
            let mut diagnostic = Diagnostic::invalid_access_regulation_string(
                &source.header.stable_key,
                field,
                character_count,
                source.header.span.clone(),
            );
            diagnostic.set_canonical_module_order(module_order);
            diagnostics.push(diagnostic);
            valid = false;
        }
    }
    if let Some(value) = &regulation.source {
        let character_count = u32::try_from(value.chars().count()).unwrap_or(u32::MAX);
        if !(1..=128).contains(&character_count) {
            let mut diagnostic = Diagnostic::invalid_access_regulation_string(
                &source.header.stable_key,
                AccessRegulationField::Source,
                character_count,
                source.header.span.clone(),
            );
            diagnostic.set_canonical_module_order(module_order);
            diagnostics.push(diagnostic);
            valid = false;
        }
    }
    valid
}

#[allow(clippy::too_many_arguments)]
fn validate_access_ambiguity(
    unit: &CompilationUnit,
    lane_edges: &TypedArena<HirLaneEdgeTag, HirLaneEdge>,
    cross_section: &CrossSectionHir,
    maneuver_paths: &[HirManeuverPath],
    classes: &TypedArena<HirParticipantClassTag, HirParticipantClass>,
    rules: &TypedArena<HirAccessRuleTag, HirAccessRule>,
    rule_classes: &[HirAccessRuleParticipantClass],
    diagnostics: &mut DiagnosticCollector,
) -> Result<(), DiagnosticBundle> {
    let counts = access_counts(unit);
    let mut candidates =
        Vec::with_capacity(count_to_usize(counts.rule_class_references, &unit.limits)?);
    for (rule_key, rule) in rules.iter() {
        let (plane, target_kind, target_index) = match rule.target {
            HirAccessTarget::LaneEdge(target) => {
                (AccessPlane::Edge, EntityKind::LaneEdge, target.raw())
            }
            HirAccessTarget::LaneGroup(target) => {
                (AccessPlane::Edge, EntityKind::LaneGroup, target.raw())
            }
            HirAccessTarget::RoadSection(target) => {
                (AccessPlane::Edge, EntityKind::RoadSection, target.raw())
            }
            HirAccessTarget::ManeuverPath(target) => (
                AccessPlane::ManeuverPath,
                EntityKind::ManeuverPath,
                target.raw(),
            ),
        };
        // 单继承意味着同深度且相交的类别子树必有相同根；完整横断面所有者树又保证
        // 同 specificity 的两个不同 edge/group/section target 不会覆盖同一边。因此
        // 只需比较规则实际声明的 target 与 selector，无需展开全部边和全部后代类别。
        for selector in &rule_classes[rule.participant_classes.as_usize_range()] {
            candidates.push(AccessCandidate {
                plane,
                target_kind,
                target_index,
                participant_class: selector.participant_class,
                priority: rule.priority,
                effect: rule.effect,
                rule: rule_key,
            });
        }
    }
    candidates.sort_unstable_by(|left, right| {
        (
            left.plane,
            left.target_kind,
            left.target_index,
            left.participant_class,
            left.priority,
        )
            .cmp(&(
                right.plane,
                right.target_kind,
                right.target_index,
                right.participant_class,
                right.priority,
            ))
            .then_with(|| left.rule.cmp(&right.rule))
    });

    let mut cursor = 0;
    while cursor < candidates.len() {
        let first = candidates[cursor];
        let group_key = (
            first.plane,
            first.target_kind,
            first.target_index,
            first.participant_class,
            first.priority,
        );
        let mut allow = None;
        let mut deny = None;
        while cursor < candidates.len()
            && (
                candidates[cursor].plane,
                candidates[cursor].target_kind,
                candidates[cursor].target_index,
                candidates[cursor].participant_class,
                candidates[cursor].priority,
            ) == group_key
        {
            match candidates[cursor].effect {
                AccessEffect::Allow => {
                    allow.get_or_insert(candidates[cursor].rule);
                }
                AccessEffect::Deny => {
                    deny.get_or_insert(candidates[cursor].rule);
                }
                _ => unreachable!("AccessEffect extension requires compiler update"),
            }
            cursor += 1;
        }
        if let (Some(allow_rule), Some(deny_rule)) = (allow, deny) {
            let allow_rule = rules.get(allow_rule);
            let deny_rule = rules.get(deny_rule);
            let participant_class = classes.get(first.participant_class);
            let target_key = match first.target_kind {
                EntityKind::LaneEdge => lane_edges
                    .get(HirLaneEdgeKey::from_raw(first.target_index))
                    .stable_key
                    .as_ref(),
                EntityKind::LaneGroup => cross_section.lane_groups
                    [usize::try_from(first.target_index).expect("u32 group index must fit usize")]
                .stable_key
                .as_ref(),
                EntityKind::RoadSection => {
                    cross_section.road_sections[usize::try_from(first.target_index)
                        .expect("u32 section index must fit usize")]
                    .stable_key
                    .as_ref()
                }
                EntityKind::ManeuverPath => maneuver_paths
                    [usize::try_from(first.target_index).expect("u32 path index must fit usize")]
                .stable_key
                .as_ref(),
                _ => unreachable!("access candidate target kinds are closed"),
            };
            let mut diagnostic = Diagnostic::access_rule_ambiguity(
                first.plane,
                first.target_kind,
                target_key,
                &participant_class.stable_key,
                &allow_rule.stable_key,
                &deny_rule.stable_key,
                deny_rule.source_span.clone(),
                allow_rule.source_span.clone(),
            );
            diagnostic.set_canonical_module_order(deny_rule.module.raw());
            diagnostics.push(diagnostic);
        }
        while cursor < candidates.len()
            && (
                candidates[cursor].plane,
                candidates[cursor].target_kind,
                candidates[cursor].target_index,
                candidates[cursor].participant_class,
                candidates[cursor].priority,
            ) == group_key
        {
            cursor += 1;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn build_route_hir(
    unit: &CompilationUnit,
    module_lookup: &HashMap<Arc<str>, HirModuleKey>,
    lane_edges: &TypedArena<HirLaneEdgeTag, HirLaneEdge>,
    lane_edge_references: &[HirLaneEdgeReference],
    lane_edge_symbols: &SymbolTable<HirLaneEdgeKey>,
    maneuver_paths: &[HirManeuverPath],
    maneuver_path_edges: &[HirManeuverPathEdge],
    junction_internal_edges: &[HirJunctionInternalEdge],
    stop_lines: &[HirStopLine],
    maneuver_gates: &[HirManeuverGate],
    waiting_zones: &[HirWaitingZone],
    maneuver_path_gates: &[HirManeuverPathGate],
    maneuver_path_waiting_zones: &[HirManeuverPathWaitingZone],
    identities: &mut IdentityRegistry,
) -> Result<RouteHir, DiagnosticBundle> {
    let counts = route_counts(unit);
    if counts.static_routes == 0 {
        return Ok(RouteHir::default());
    }

    // 候选表按前两条边建立连续排序索引。路线扫描只做二分分段和切片遍历，既不依赖
    // HashMap 迭代顺序，也不会为每条路线重新建立路径查找表。
    let mut entry_candidates = Vec::with_capacity(maneuver_paths.len());
    for (index, path) in maneuver_paths.iter().enumerate() {
        let path_key = HirManeuverPathKey::from_raw(
            u32::try_from(index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        let edges = &maneuver_path_edges[path.edges.as_usize_range()];
        debug_assert!(
            edges.len() >= 2,
            "validated ManeuverPath must have boundaries"
        );
        entry_candidates.push((edges[0].target, edges[1].target, path_key));
    }
    entry_candidates.sort_unstable_by(|left, right| {
        (
            left.0.raw(),
            left.1.raw(),
            maneuver_paths[left.2.index()].stable_id,
        )
            .cmp(&(
                right.0.raw(),
                right.1.raw(),
                maneuver_paths[right.2.index()].stable_id,
            ))
    });

    // 角色索引把路线边界检查和最终覆盖检查降为 O(route edges)。路口 HIR 已证明内部
    // 边排他，因此每个槽最多只有一个 ManeuverPath 所有者。
    let mut internal_owner = vec![None; lane_edges.len()];
    for claim in junction_internal_edges {
        internal_owner[claim.edge.index()] = Some(claim.source_path);
    }
    let mut stop_line_by_edge = vec![None; lane_edges.len()];
    for (index, stop_line) in stop_lines.iter().enumerate() {
        let slot = &mut stop_line_by_edge[stop_line.lane_edge.index()];
        if slot.is_none_or(|existing: usize| stop_lines[existing].stable_id > stop_line.stable_id) {
            *slot = Some(index);
        }
    }

    let route_capacity = count_to_usize(counts.static_routes, &unit.limits)?;
    let edge_capacity = count_to_usize(counts.route_edges, &unit.limits)?;
    let transition_capacity = count_to_usize(counts.route_transitions, &unit.limits)?;
    let mut routes = TypedArena::<HirStaticRouteTag, HirStaticRoute>::with_capacity(route_capacity);
    let mut sources = Vec::with_capacity(route_capacity);

    // 先按模块规范顺序和 stable key 登记路线身份，使声明物理顺序不影响路线 ordinal。
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key = HirModuleKey::from_raw(
            u32::try_from(module_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        let mut declaration_indices = source_module
            .declarations
            .iter()
            .enumerate()
            .filter_map(|(index, declaration)| {
                matches!(declaration, TypedAstDeclaration::StaticRoute(_)).then_some(index)
            })
            .collect::<Vec<_>>();
        declaration_indices.sort_unstable_by(|left, right| {
            declaration_header(&source_module.declarations[*left])
                .stable_key
                .cmp(&declaration_header(&source_module.declarations[*right]).stable_key)
        });
        for declaration_index in declaration_indices {
            let TypedAstDeclaration::StaticRoute(source) =
                &source_module.declarations[declaration_index]
            else {
                unreachable!("route source filter admitted unrelated declaration")
            };
            let fields = [
                IdentityFieldInput::new(
                    FieldTag::AuthoringNamespaceId,
                    source_module
                        .descriptor()
                        .authoring_namespace_id()
                        .as_bytes(),
                ),
                IdentityFieldInput::new(FieldTag::RouteKey, source.header.stable_key.as_bytes()),
            ];
            let stable_id = StaticRouteId::from_untyped(derive_identity(
                unit,
                identities,
                module_index,
                EntityKind::StaticRoute,
                &source.header.stable_key,
                &source.header.span,
                &fields,
            )?);
            let route_key = routes
                .push(HirStaticRoute {
                    module: module_key,
                    stable_key: Arc::clone(&source.header.stable_key),
                    stable_id,
                    edges: TableRange::empty(),
                    transitions: TableRange::empty(),
                    maneuver_occurrences: TableRange::empty(),
                    gate_occurrences: TableRange::empty(),
                    waiting_zone_occurrences: TableRange::empty(),
                    source_span: source.header.span.clone(),
                })
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                })?;
            sources.push(CanonicalDeclarationSource {
                source_module_index: u32::try_from(module_index).map_err(|_| {
                    arena_overflow(
                        ArenaKeyOverflow,
                        &unit.limits,
                        Some(source.header.span.clone()),
                    )
                })?,
                declaration_index: u32::try_from(declaration_index).map_err(|_| {
                    arena_overflow(
                        ArenaKeyOverflow,
                        &unit.limits,
                        Some(source.header.span.clone()),
                    )
                })?,
                hir_key: route_key,
            });
        }
    }

    let mut route_edges = Vec::with_capacity(edge_capacity);
    let mut route_transitions = Vec::with_capacity(transition_capacity);
    let mut maneuver_occurrences = Vec::with_capacity(edge_capacity);
    let mut gate_occurrences = Vec::with_capacity(edge_capacity);
    let mut waiting_zone_occurrences = Vec::with_capacity(edge_capacity);
    let mut diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));

    for location in sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let TypedAstDeclaration::StaticRoute(source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical StaticRoute source changed kind")
        };
        let route_index = |index: usize| {
            u32::try_from(index).map_err(|_| {
                arena_overflow(
                    ArenaKeyOverflow,
                    &unit.limits,
                    Some(source.header.span.clone()),
                )
            })
        };
        let mut resolved_edges = Vec::with_capacity(source.edge_sequence.len());
        let mut route_has_error = false;
        for reference in &source.edge_sequence {
            if let Some(target) = resolve_reference(
                module_lookup,
                lane_edge_symbols,
                reference,
                EntityKind::StaticRoute,
                &source.header,
                location.source_module_index,
                &mut diagnostics,
            ) {
                resolved_edges.push(HirStaticRouteEdge {
                    target,
                    source_span: reference.span.clone(),
                });
            } else {
                route_has_error = true;
            }
        }
        if route_has_error {
            continue;
        }
        debug_assert!(!resolved_edges.is_empty(), "frontend rejects empty routes");

        for (index, pair) in resolved_edges.windows(2).enumerate() {
            let successors =
                &lane_edge_references[lane_edges.get(pair[0].target).successors.as_usize_range()];
            if !successors
                .iter()
                .any(|successor| successor.target == pair[1].target)
            {
                let mut diagnostic = Diagnostic::disconnected_static_route_edge(
                    &source.header.stable_key,
                    &lane_edges.get(pair[0].target).stable_key,
                    &lane_edges.get(pair[1].target).stable_key,
                    route_index(index.saturating_add(1))?,
                    pair[1].source_span.clone(),
                    pair[0].source_span.clone(),
                );
                diagnostic.set_canonical_module_order(location.source_module_index);
                diagnostics.push(diagnostic);
                route_has_error = true;
            }
        }
        let first_edge = resolved_edges[0].target;
        if let Some(path_key) = internal_owner[first_edge.index()] {
            let mut diagnostic = Diagnostic::static_route_starts_inside_junction(
                &source.header.stable_key,
                &lane_edges.get(first_edge).stable_key,
                resolved_edges[0].source_span.clone(),
                maneuver_paths[path_key.index()].source_span.clone(),
            );
            diagnostic.set_canonical_module_order(location.source_module_index);
            diagnostics.push(diagnostic);
            route_has_error = true;
        }
        let last_index = resolved_edges.len().saturating_sub(1);
        let last_edge = resolved_edges[last_index].target;
        if let Some(path_key) = internal_owner[last_edge.index()] {
            let mut diagnostic = Diagnostic::static_route_ends_inside_junction(
                &source.header.stable_key,
                &lane_edges.get(last_edge).stable_key,
                resolved_edges[last_index].source_span.clone(),
                maneuver_paths[path_key.index()].source_span.clone(),
            );
            diagnostic.set_canonical_module_order(location.source_module_index);
            diagnostics.push(diagnostic);
            route_has_error = true;
        }
        if let Some(stop_index) = stop_line_by_edge[last_edge.index()] {
            let stop_line = &stop_lines[stop_index];
            let mut diagnostic = Diagnostic::static_route_terminates_at_stop_line(
                &source.header.stable_key,
                &lane_edges.get(last_edge).stable_key,
                &stop_line.stable_key,
                resolved_edges[last_index].source_span.clone(),
                stop_line.source_span.clone(),
            );
            diagnostic.set_canonical_module_order(location.source_module_index);
            diagnostics.push(diagnostic);
            route_has_error = true;
        }
        if route_has_error {
            continue;
        }

        let mut local_transitions = (0..resolved_edges.len().saturating_sub(1))
            .map(|_| HirStaticRouteTransition {
                maneuver_gate: None,
            })
            .collect::<Vec<_>>();
        let mut local_maneuvers = Vec::with_capacity(resolved_edges.len());
        let mut local_gates = Vec::with_capacity(resolved_edges.len());
        let mut local_waiting = Vec::with_capacity(resolved_edges.len());
        let mut internal_coverage: Vec<Option<HirManeuverPathKey>> =
            vec![None; resolved_edges.len()];

        for entry_index in 0..resolved_edges.len().saturating_sub(1) {
            let pair = (
                resolved_edges[entry_index].target,
                resolved_edges[entry_index + 1].target,
            );
            let candidate_start = entry_candidates.partition_point(|candidate| {
                (candidate.0.raw(), candidate.1.raw()) < (pair.0.raw(), pair.1.raw())
            });
            let candidate_end = entry_candidates.partition_point(|candidate| {
                (candidate.0.raw(), candidate.1.raw()) <= (pair.0.raw(), pair.1.raw())
            });
            if candidate_start == candidate_end {
                continue;
            }
            let candidates = &entry_candidates[candidate_start..candidate_end];
            let mut full_matches = candidates.iter().filter_map(|candidate| {
                let path = &maneuver_paths[candidate.2.index()];
                let path_edges = &maneuver_path_edges[path.edges.as_usize_range()];
                (resolved_edges.len().saturating_sub(entry_index) >= path_edges.len()
                    && resolved_edges[entry_index..entry_index + path_edges.len()]
                        .iter()
                        .map(|edge| edge.target)
                        .eq(path_edges.iter().map(|edge| edge.target)))
                .then_some(candidate.2)
            });
            let Some(path_key) = full_matches.next() else {
                let candidate = candidates[0].2;
                let mut diagnostic = Diagnostic::static_route_maneuver_no_full_match(
                    &source.header.stable_key,
                    route_index(entry_index)?,
                    &lane_edges.get(pair.0).stable_key,
                    &lane_edges.get(pair.1).stable_key,
                    resolved_edges[entry_index + 1].source_span.clone(),
                    maneuver_paths[candidate.index()].source_span.clone(),
                );
                diagnostic.set_canonical_module_order(location.source_module_index);
                diagnostics.push(diagnostic);
                route_has_error = true;
                continue;
            };
            if let Some(second_path_key) = full_matches.next() {
                let first = &maneuver_paths[path_key.index()];
                let second = &maneuver_paths[second_path_key.index()];
                let mut diagnostic = Diagnostic::static_route_maneuver_multiple_full_matches(
                    &source.header.stable_key,
                    route_index(entry_index)?,
                    &first.stable_key,
                    &second.stable_key,
                    resolved_edges[entry_index].source_span.clone(),
                    first.source_span.clone(),
                    second.source_span.clone(),
                );
                diagnostic.set_canonical_module_order(location.source_module_index);
                diagnostics.push(diagnostic);
                route_has_error = true;
                continue;
            }

            let path = &maneuver_paths[path_key.index()];
            let path_edge_count = path.edges.as_usize_range().len();
            let exit_index = entry_index + path_edge_count.saturating_sub(1);
            for route_edge_index in entry_index + 1..exit_index {
                if let Some(first_path_key) = internal_coverage[route_edge_index] {
                    let first = &maneuver_paths[first_path_key.index()];
                    let mut diagnostic = Diagnostic::static_route_maneuver_internal_overlap(
                        &source.header.stable_key,
                        route_index(route_edge_index)?,
                        &lane_edges
                            .get(resolved_edges[route_edge_index].target)
                            .stable_key,
                        &first.stable_key,
                        &path.stable_key,
                        resolved_edges[route_edge_index].source_span.clone(),
                        first.source_span.clone(),
                        path.source_span.clone(),
                    );
                    diagnostic.set_canonical_module_order(location.source_module_index);
                    diagnostics.push(diagnostic);
                    route_has_error = true;
                } else {
                    internal_coverage[route_edge_index] = Some(path_key);
                }
            }

            let maneuver_index = local_maneuvers.len();
            let local_gate_start = local_gates.len();
            for member in &maneuver_path_gates[path.maneuver_gates.as_usize_range()] {
                let gate = &maneuver_gates[member.maneuver_gate.index()];
                let from_route_edge_index = entry_index + gate.transition_index as usize;
                local_transitions[from_route_edge_index].maneuver_gate = Some(member.maneuver_gate);
                local_gates.push(HirGateOccurrence {
                    maneuver_gate: member.maneuver_gate,
                    maneuver_occurrence_index: route_index(maneuver_index)?,
                    from_route_edge_index: route_index(from_route_edge_index)?,
                    next_gate_occurrence_index: None,
                    next_boundary_route_edge_index: route_index(exit_index)?,
                    waiting_zone_occurrence_index: None,
                });
            }
            let local_gate_end = local_gates.len();
            for gate_index in local_gate_start..local_gate_end {
                if gate_index + 1 < local_gate_end {
                    local_gates[gate_index].next_gate_occurrence_index =
                        Some(route_index(gate_index + 1)?);
                    local_gates[gate_index].next_boundary_route_edge_index =
                        local_gates[gate_index + 1].from_route_edge_index;
                }
            }

            let local_waiting_start = local_waiting.len();
            for member in &maneuver_path_waiting_zones[path.waiting_zones.as_usize_range()] {
                let waiting = &waiting_zones[member.waiting_zone.index()];
                let entry_gate_offset = maneuver_path_gates[path.maneuver_gates.as_usize_range()]
                    .iter()
                    .position(|gate| gate.maneuver_gate == waiting.entry_gate)
                    .expect("validated WaitingZone entry gate belongs to path");
                let release_gate_offset = maneuver_path_gates[path.maneuver_gates.as_usize_range()]
                    .iter()
                    .position(|gate| gate.maneuver_gate == waiting.release_gate)
                    .expect("validated WaitingZone release gate belongs to path");
                let entry_gate_index = local_gate_start + entry_gate_offset;
                let release_gate_index = local_gate_start + release_gate_offset;
                let waiting_index = local_waiting.len();
                local_gates[entry_gate_index].waiting_zone_occurrence_index =
                    Some(route_index(waiting_index)?);
                local_waiting.push(HirWaitingZoneOccurrence {
                    waiting_zone: member.waiting_zone,
                    maneuver_occurrence_index: route_index(maneuver_index)?,
                    entry_gate_occurrence_index: route_index(entry_gate_index)?,
                    release_gate_occurrence_index: route_index(release_gate_index)?,
                    entry_route_edge_index: local_gates[entry_gate_index].from_route_edge_index,
                    release_route_edge_index: local_gates[release_gate_index].from_route_edge_index,
                });
            }
            local_maneuvers.push(HirManeuverOccurrence {
                maneuver_path: path_key,
                entry_route_edge_index: route_index(entry_index)?,
                exit_route_edge_index: route_index(exit_index)?,
                gate_occurrences: TableRange::try_from_usize(
                    gate_occurrences.len() + local_gate_start,
                    local_gate_end.saturating_sub(local_gate_start),
                )
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                })?,
                waiting_zone_occurrences: TableRange::try_from_usize(
                    waiting_zone_occurrences.len() + local_waiting_start,
                    local_waiting.len().saturating_sub(local_waiting_start),
                )
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                })?,
            });
        }

        for (route_edge_index, route_edge) in resolved_edges.iter().enumerate() {
            let Some(owner_path) = internal_owner[route_edge.target.index()] else {
                continue;
            };
            if internal_coverage[route_edge_index].is_none() {
                let mut diagnostic = Diagnostic::static_route_internal_edge_uncovered(
                    &source.header.stable_key,
                    route_index(route_edge_index)?,
                    &lane_edges.get(route_edge.target).stable_key,
                    route_edge.source_span.clone(),
                    maneuver_paths[owner_path.index()].source_span.clone(),
                );
                diagnostic.set_canonical_module_order(location.source_module_index);
                diagnostics.push(diagnostic);
                route_has_error = true;
            }
        }
        if route_has_error {
            continue;
        }

        let edge_start = route_edges.len();
        let transition_start = route_transitions.len();
        let maneuver_start = maneuver_occurrences.len();
        let gate_start = gate_occurrences.len();
        let waiting_start = waiting_zone_occurrences.len();
        route_edges.extend(resolved_edges);
        route_transitions.extend(local_transitions);
        maneuver_occurrences.extend(local_maneuvers);
        gate_occurrences.extend(local_gates);
        waiting_zone_occurrences.extend(local_waiting);
        let route = routes.get_mut(location.hir_key);
        route.edges = TableRange::try_from_usize(edge_start, route_edges.len() - edge_start)
            .map_err(|overflow| {
                arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
            })?;
        route.transitions = TableRange::try_from_usize(
            transition_start,
            route_transitions.len() - transition_start,
        )
        .map_err(|overflow| {
            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
        })?;
        route.maneuver_occurrences =
            TableRange::try_from_usize(maneuver_start, maneuver_occurrences.len() - maneuver_start)
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                })?;
        route.gate_occurrences =
            TableRange::try_from_usize(gate_start, gate_occurrences.len() - gate_start).map_err(
                |overflow| arena_overflow(overflow, &unit.limits, Some(source.header.span.clone())),
            )?;
        route.waiting_zone_occurrences = TableRange::try_from_usize(
            waiting_start,
            waiting_zone_occurrences.len() - waiting_start,
        )
        .map_err(|overflow| {
            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
        })?;
    }

    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }
    Ok(RouteHir {
        static_routes: routes.into_boxed_slice(),
        static_route_edges: route_edges.into_boxed_slice(),
        static_route_transitions: route_transitions.into_boxed_slice(),
        maneuver_occurrences: maneuver_occurrences.into_boxed_slice(),
        gate_occurrences: gate_occurrences.into_boxed_slice(),
        waiting_zone_occurrences: waiting_zone_occurrences.into_boxed_slice(),
    })
}

fn derive_identity(
    unit: &CompilationUnit,
    identities: &mut IdentityRegistry,
    module_index: usize,
    entity_kind: EntityKind,
    stable_key: &str,
    source_span: &SourceLocation,
    fields: &[IdentityFieldInput<'_>],
) -> Result<StableId128, DiagnosticBundle> {
    let identity = encode_canonical_identity(
        entity_kind,
        fields,
        unit.limits.value(CompileLimitDimension::SingleStringBytes),
    )
    .map_err(|violation| {
        let mut diagnostic = Diagnostic::invalid_canonical_identity(
            entity_kind,
            stable_key,
            violation,
            source_span.clone(),
        );
        diagnostic.set_canonical_module_order(u32::try_from(module_index).unwrap_or(u32::MAX));
        DiagnosticBundle::single(diagnostic)
    })?;
    if let Err(error) = identities.register(&identity, source_span) {
        let mut diagnostic = match error {
            IdentityRegistrationError::Duplicate { existing_span } => {
                Diagnostic::duplicate_canonical_identity(
                    entity_kind,
                    stable_key,
                    identity.stable_id(),
                    source_span.clone(),
                    existing_span,
                )
            }
            IdentityRegistrationError::DigestCollision { existing_span } => {
                Diagnostic::identity_digest_collision(
                    entity_kind,
                    stable_key,
                    identity.stable_id(),
                    source_span.clone(),
                    existing_span,
                )
            }
        };
        diagnostic.set_canonical_module_order(u32::try_from(module_index).unwrap_or(u32::MAX));
        return Err(DiagnosticBundle::single(diagnostic));
    }
    Ok(identity.stable_id())
}

#[allow(clippy::too_many_arguments)]
fn register_owner(
    entity_kind: EntityKind,
    target_index: usize,
    target_key: &str,
    owner: HirRoadCorridorKey,
    owner_header: &crate::declaration::DeclarationHeader,
    owners: &mut [Option<(HirRoadCorridorKey, SourceLocation)>],
    corridors: &TypedArena<HirRoadCorridorTag, HirRoadCorridor>,
    module_order: u32,
    diagnostics: &mut DiagnosticCollector,
) {
    if let Some((first_owner, first_span)) = &owners[target_index] {
        let mut diagnostic = Diagnostic::multiple_cross_section_owners(
            entity_kind,
            target_key,
            &corridors.get(*first_owner).stable_key,
            &owner_header.stable_key,
            owner_header.span.clone(),
            first_span.clone(),
        );
        diagnostic.set_canonical_module_order(module_order);
        diagnostics.push(diagnostic);
    } else {
        owners[target_index] = Some((owner, owner_header.span.clone()));
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_reference<M, K: Copy>(
    module_lookup: &HashMap<Arc<str>, HirModuleKey>,
    symbols: &SymbolTable<K>,
    reference: &OwnedEntityReference<M>,
    source_kind: EntityKind,
    source_header: &crate::declaration::DeclarationHeader,
    module_order: u32,
    diagnostics: &mut DiagnosticCollector,
) -> Option<K>
where
    M: laneflow_static_contract::EntityKindMarker,
{
    let target_module = module_lookup[reference.module_namespace.as_ref()];
    let Some(target) = symbols.get(target_module, &reference.declaration_key) else {
        let mut diagnostic = Diagnostic::unknown_reference_target(
            source_kind,
            &source_header.stable_key,
            &reference.module_namespace,
            &reference.declaration_key,
            reference.span.clone(),
            source_header.span.clone(),
        );
        diagnostic.set_canonical_module_order(module_order);
        diagnostics.push(diagnostic);
        return None;
    };
    Some(target)
}

fn lane_edge_declaration(declaration: &TypedAstDeclaration) -> Option<&LaneEdgeDeclaration> {
    match declaration {
        TypedAstDeclaration::LaneEdge(declaration) => Some(declaration),
        _ => None,
    }
}

fn movement_declaration(
    declaration: &TypedAstDeclaration,
) -> Option<&crate::declaration::MovementDeclaration> {
    match declaration {
        TypedAstDeclaration::Movement(declaration) => Some(declaration),
        _ => None,
    }
}

fn maneuver_path_declaration(
    declaration: &TypedAstDeclaration,
) -> Option<&crate::declaration::ManeuverPathDeclaration> {
    match declaration {
        TypedAstDeclaration::ManeuverPath(declaration) => Some(declaration),
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
        TypedAstDeclaration::StaticRoute(declaration) => &declaration.header,
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

fn lane_edge_count(unit: &CompilationUnit) -> u64 {
    unit.modules
        .iter()
        .flat_map(|module| module.declarations.iter())
        .filter(|declaration| matches!(declaration, TypedAstDeclaration::LaneEdge(_)))
        .count()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn lane_edge_reference_count(unit: &CompilationUnit) -> u64 {
    unit.modules
        .iter()
        .flat_map(|module| module.declarations.iter())
        .filter_map(lane_edge_declaration)
        .fold(0_u64, |total, declaration| {
            total.saturating_add(u64::try_from(declaration.successors.len()).unwrap_or(u64::MAX))
        })
}

fn cross_section_counts(unit: &CompilationUnit) -> CrossSectionCounts {
    let mut counts = CrossSectionCounts::default();
    for declaration in unit
        .modules
        .iter()
        .flat_map(|module| module.declarations.iter())
    {
        match declaration {
            TypedAstDeclaration::LaneEdge(_) => {}
            TypedAstDeclaration::RoadCorridor(corridor) => {
                counts.road_corridors = counts.road_corridors.saturating_add(1);
                counts.corridor_elements = counts
                    .corridor_elements
                    .saturating_add(u64::try_from(corridor.elements.len()).unwrap_or(u64::MAX));
            }
            TypedAstDeclaration::RoadSection(section) => {
                counts.road_sections = counts.road_sections.saturating_add(1);
                counts.authoring_lanes = counts
                    .authoring_lanes
                    .saturating_add(u64::try_from(section.lanes.len()).unwrap_or(u64::MAX));
                counts.authoring_lane_edges =
                    counts
                        .authoring_lane_edges
                        .saturating_add(section.lanes.iter().fold(0_u64, |total, lane| {
                            total.saturating_add(
                                u64::try_from(lane.edge_chain.len()).unwrap_or(u64::MAX),
                            )
                        }));
            }
            TypedAstDeclaration::LaneGroup(_) => {
                counts.lane_groups = counts.lane_groups.saturating_add(1);
            }
            TypedAstDeclaration::FacilityBand(_) => {
                counts.facility_bands = counts.facility_bands.saturating_add(1);
            }
            TypedAstDeclaration::Junction(_)
            | TypedAstDeclaration::Movement(_)
            | TypedAstDeclaration::ManeuverPath(_)
            | TypedAstDeclaration::StopLine(_)
            | TypedAstDeclaration::ManeuverGate(_)
            | TypedAstDeclaration::WaitingZone(_)
            | TypedAstDeclaration::StaticRoute(_)
            | TypedAstDeclaration::SignalGroup(_)
            | TypedAstDeclaration::SignalController(_)
            | TypedAstDeclaration::ParkingArea(_)
            | TypedAstDeclaration::ParkingSpace(_)
            | TypedAstDeclaration::ParticipantClass(_)
            | TypedAstDeclaration::VehicleProfile(_)
            | TypedAstDeclaration::CanonicalFrame(_)
            | TypedAstDeclaration::AccessRule(_) => {}
        }
    }
    counts
}

fn junction_counts(unit: &CompilationUnit) -> JunctionCounts {
    let mut counts = JunctionCounts::default();
    for declaration in unit
        .modules
        .iter()
        .flat_map(|module| module.declarations.iter())
    {
        match declaration {
            TypedAstDeclaration::Junction(_) => {
                counts.junctions = counts.junctions.saturating_add(1);
            }
            TypedAstDeclaration::Movement(_) => {
                counts.movements = counts.movements.saturating_add(1);
            }
            TypedAstDeclaration::ManeuverPath(path) => {
                counts.maneuver_paths = counts.maneuver_paths.saturating_add(1);
                counts.maneuver_path_edges = counts.maneuver_path_edges.saturating_add(
                    u64::try_from(path.internal_edges.len())
                        .unwrap_or(u64::MAX)
                        .saturating_add(2),
                );
            }
            _ => {}
        }
    }
    counts
}

fn control_counts(unit: &CompilationUnit) -> ControlCounts {
    let mut counts = ControlCounts {
        maneuver_gates: unit.maneuver_gate_count,
        waiting_zones: unit.waiting_zone_count,
        ..ControlCounts::default()
    };
    for declaration in unit
        .modules
        .iter()
        .flat_map(|module| module.declarations.iter())
    {
        match declaration {
            TypedAstDeclaration::StopLine(_) => {
                counts.stop_lines = counts.stop_lines.saturating_add(1);
            }
            TypedAstDeclaration::ManeuverGate(_) | TypedAstDeclaration::WaitingZone(_) => {}
            _ => {}
        }
    }
    counts
}

fn route_counts(unit: &CompilationUnit) -> RouteCounts {
    let mut counts = RouteCounts::default();
    for declaration in unit
        .modules
        .iter()
        .flat_map(|module| module.declarations.iter())
    {
        if let TypedAstDeclaration::StaticRoute(route) = declaration {
            let edge_count = u64::try_from(route.edge_sequence.len()).unwrap_or(u64::MAX);
            counts.static_routes = counts.static_routes.saturating_add(1);
            counts.route_edges = counts.route_edges.saturating_add(edge_count);
            counts.route_transitions = counts
                .route_transitions
                .saturating_add(edge_count.saturating_sub(1));
            counts.largest_route_edges = counts.largest_route_edges.max(edge_count);
        }
    }
    debug_assert_eq!(counts.route_edges, unit.route_occurrence_count);
    counts
}

fn signal_counts(unit: &CompilationUnit) -> SignalCounts {
    let mut counts = SignalCounts::default();
    for declaration in unit
        .modules
        .iter()
        .flat_map(|module| module.declarations.iter())
    {
        match declaration {
            TypedAstDeclaration::SignalGroup(_) => {
                counts.groups = counts.groups.saturating_add(1);
            }
            TypedAstDeclaration::SignalController(controller) => {
                counts.controllers = counts.controllers.saturating_add(1);
                counts.controller_groups = counts.controller_groups.saturating_add(
                    u64::try_from(controller.signal_groups.len()).unwrap_or(u64::MAX),
                );
                counts.phases = counts
                    .phases
                    .saturating_add(u64::try_from(controller.phases.len()).unwrap_or(u64::MAX));
                counts.phase_states =
                    counts
                        .phase_states
                        .saturating_add(controller.phases.iter().fold(0_u64, |total, phase| {
                            total.saturating_add(
                                u64::try_from(phase.states.len()).unwrap_or(u64::MAX),
                            )
                        }));
            }
            TypedAstDeclaration::ManeuverGate(gate)
                if matches!(gate.signal_control, OwnedSignalControl::Group(_)) =>
            {
                counts.controlled_gates = counts.controlled_gates.saturating_add(1);
            }
            _ => {}
        }
    }
    counts
}

fn parking_counts(unit: &CompilationUnit) -> ParkingCounts {
    let mut counts = ParkingCounts::default();
    for declaration in unit
        .modules
        .iter()
        .flat_map(|module| module.declarations.iter())
    {
        match declaration {
            TypedAstDeclaration::ParkingArea(_) => {
                counts.areas = counts.areas.saturating_add(1);
            }
            TypedAstDeclaration::ParkingSpace(space) => {
                counts.spaces = counts.spaces.saturating_add(1);
                if space.parking_area.is_some() {
                    counts.memberships = counts.memberships.saturating_add(1);
                }
            }
            _ => {}
        }
    }
    counts
}

fn access_counts(unit: &CompilationUnit) -> AccessCounts {
    let mut counts = AccessCounts::default();
    for declaration in unit
        .modules
        .iter()
        .flat_map(|module| module.declarations.iter())
    {
        match declaration {
            TypedAstDeclaration::ParticipantClass(_) => {
                counts.participant_classes = counts.participant_classes.saturating_add(1);
            }
            TypedAstDeclaration::VehicleProfile(_) => {
                counts.vehicle_profiles = counts.vehicle_profiles.saturating_add(1);
            }
            TypedAstDeclaration::AccessRule(rule) => {
                counts.access_rules = counts.access_rules.saturating_add(1);
                counts.rule_class_references = counts.rule_class_references.saturating_add(
                    u64::try_from(rule.participant_classes.len()).unwrap_or(u64::MAX),
                );
            }
            _ => {}
        }
    }
    counts
}

fn spatial_counts(unit: &CompilationUnit) -> SpatialCounts {
    let mut counts = SpatialCounts::default();
    for declaration in unit
        .modules
        .iter()
        .flat_map(|module| module.declarations.iter())
    {
        if let TypedAstDeclaration::CanonicalFrame(frame) = declaration {
            counts.canonical_frames = counts.canonical_frames.saturating_add(1);
            counts.lane_edge_geometries = counts.lane_edge_geometries.saturating_add(
                u64::try_from(frame.lane_edge_geometries.len()).unwrap_or(u64::MAX),
            );
            for geometry in &frame.lane_edge_geometries {
                let points = u64::try_from(geometry.centerline_points.len()).unwrap_or(u64::MAX);
                counts.canonical_points = counts.canonical_points.saturating_add(points);
                counts.spatial_segments = counts
                    .spatial_segments
                    .saturating_add(points.saturating_sub(1));
            }
        }
    }
    counts
}

fn corridors_capacity(
    counts: &CrossSectionCounts,
    limits: &crate::CompileLimits,
) -> Result<usize, DiagnosticBundle> {
    count_to_usize(counts.road_corridors, limits)
}

fn sections_capacity(
    counts: &CrossSectionCounts,
    limits: &crate::CompileLimits,
) -> Result<usize, DiagnosticBundle> {
    count_to_usize(counts.road_sections, limits)
}

fn lanes_capacity(
    counts: &CrossSectionCounts,
    limits: &crate::CompileLimits,
) -> Result<usize, DiagnosticBundle> {
    count_to_usize(counts.authoring_lanes, limits)
}

fn groups_capacity(
    counts: &CrossSectionCounts,
    limits: &crate::CompileLimits,
) -> Result<usize, DiagnosticBundle> {
    count_to_usize(counts.lane_groups, limits)
}

fn bands_capacity(
    counts: &CrossSectionCounts,
    limits: &crate::CompileLimits,
) -> Result<usize, DiagnosticBundle> {
    count_to_usize(counts.facility_bands, limits)
}

fn identity_byte_counts(unit: &CompilationUnit) -> (u64, u64) {
    let mut total = 0_u64;
    let mut largest = 0_u64;
    for module in &unit.modules {
        let namespace_bytes =
            u64::try_from(module.descriptor().authoring_namespace_id().len()).unwrap_or(u64::MAX);
        for source_declaration in &module.declarations {
            let header = declaration_header(source_declaration);
            let bytes = match source_declaration {
                TypedAstDeclaration::LaneEdge(_)
                | TypedAstDeclaration::RoadCorridor(_)
                | TypedAstDeclaration::Junction(_)
                | TypedAstDeclaration::StaticRoute(_) => 22_u64
                    .saturating_add(namespace_bytes)
                    .saturating_add(u64::try_from(header.stable_key.len()).unwrap_or(u64::MAX)),
                TypedAstDeclaration::RoadSection(_)
                | TypedAstDeclaration::LaneGroup(_)
                | TypedAstDeclaration::FacilityBand(_) => 44_u64
                    .saturating_add(namespace_bytes)
                    .saturating_add(u64::try_from(header.stable_key.len()).unwrap_or(u64::MAX)),
                TypedAstDeclaration::Movement(movement) => 56_u64
                    .saturating_add(namespace_bytes)
                    .saturating_add(u64::try_from(header.stable_key.len()).unwrap_or(u64::MAX))
                    .saturating_add(
                        u64::try_from(movement.directed_entry_approach_key.len())
                            .unwrap_or(u64::MAX),
                    )
                    .saturating_add(
                        u64::try_from(movement.directed_exit_approach_key.len())
                            .unwrap_or(u64::MAX),
                    ),
                TypedAstDeclaration::ManeuverPath(_) => 88_u64
                    .saturating_add(namespace_bytes)
                    .saturating_add(u64::try_from(header.stable_key.len()).unwrap_or(u64::MAX)),
                TypedAstDeclaration::StopLine(_) => 22_u64
                    .saturating_add(namespace_bytes)
                    .saturating_add(u64::try_from(header.stable_key.len()).unwrap_or(u64::MAX)),
                TypedAstDeclaration::ManeuverGate(_) | TypedAstDeclaration::WaitingZone(_) => {
                    44_u64
                        .saturating_add(namespace_bytes)
                        .saturating_add(u64::try_from(header.stable_key.len()).unwrap_or(u64::MAX))
                }
                TypedAstDeclaration::SignalGroup(_)
                | TypedAstDeclaration::SignalController(_)
                | TypedAstDeclaration::ParkingArea(_)
                | TypedAstDeclaration::ParkingSpace(_)
                | TypedAstDeclaration::ParticipantClass(_)
                | TypedAstDeclaration::VehicleProfile(_)
                | TypedAstDeclaration::CanonicalFrame(_)
                | TypedAstDeclaration::AccessRule(_) => 22_u64
                    .saturating_add(namespace_bytes)
                    .saturating_add(u64::try_from(header.stable_key.len()).unwrap_or(u64::MAX)),
            };
            total = total.saturating_add(bytes);
            largest = largest.max(bytes);
            if let TypedAstDeclaration::RoadSection(section) = source_declaration {
                for lane in &section.lanes {
                    let lane_bytes = 44_u64.saturating_add(namespace_bytes).saturating_add(
                        u64::try_from(lane.header.stable_key.len()).unwrap_or(u64::MAX),
                    );
                    total = total.saturating_add(lane_bytes);
                    largest = largest.max(lane_bytes);
                }
            }
            if let TypedAstDeclaration::SignalController(controller) = source_declaration {
                for phase in &controller.phases {
                    let phase_bytes = 44_u64.saturating_add(namespace_bytes).saturating_add(
                        u64::try_from(phase.header.stable_key.len()).unwrap_or(u64::MAX),
                    );
                    total = total.saturating_add(phase_bytes);
                    largest = largest.max(phase_bytes);
                }
            }
        }
    }
    (total, largest)
}

fn requested_bytes<T>(count: u64) -> u64 {
    count.saturating_mul(u64::try_from(size_of::<T>()).unwrap_or(u64::MAX))
}

fn requested_hash_table_bytes<K, V>(entry_count: u64) -> u64 {
    if entry_count == 0 {
        return 0;
    }
    // 标准库不公开桶分配布局。预检为每个请求项预留八个桶，并额外计入每桶控制字节
    // 与一组尾部控制区，覆盖小表最小桶数和负载因子取整，而不依赖哈希表迭代顺序。
    // 真实生产基准仍须另报实际容量和进程内存，不能用本预算冒充测量。
    let bucket_bytes = u64::try_from(size_of::<(K, V)>())
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    entry_count
        .saturating_mul(8)
        .saturating_mul(bucket_bytes)
        .saturating_add(16)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CompilationUnitBuilder, CompileLimits, DiagnosticCode, DiagnosticPayload, LaneEdgeInput,
        LaneEdgeReference, SourceModuleHeader, SourceModuleHeaderInput, SyntheticModule,
        SyntheticModuleBuilder,
    };

    fn header(namespace: &str) -> SourceModuleHeader {
        SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: namespace,
                source_document_key: namespace,
                generator_build_id: "git:0123456789abcdef",
                parameters_and_inputs_digest: [0x11; 32],
                frontend_options_digest: [0x22; 32],
                random_seed: Some(42),
                provenance: "repository:laneflow",
            },
            &CompileLimits::p100_initial_v1(),
        )
        .unwrap()
    }

    fn module(
        namespace: &str,
        imports: &[&str],
        edges: &[(&str, &[LaneEdgeReference<'_>])],
    ) -> SyntheticModule {
        let limits = CompileLimits::p100_initial_v1();
        let mut builder = SyntheticModuleBuilder::new(header(namespace), &limits).unwrap();
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

    #[test]
    fn hir_resolves_local_and_imported_lane_edge_references_to_typed_keys() {
        let base = module("city/base", &[], &[("edge-b", &[])]);
        let app_successors = [
            LaneEdgeReference::imported("city/base", "edge-b"),
            LaneEdgeReference::local("edge-c"),
        ];
        let app = module(
            "city/app",
            &["city/base"],
            &[("edge-c", &[]), ("edge-a", &app_successors)],
        );
        let unit = unit([app, base]);
        let hir = build_hir(&unit).unwrap();

        assert_eq!(hir.modules.len(), 2);
        assert_eq!(hir.modules[0].authoring_namespace_id.as_ref(), "city/base");
        assert_eq!(hir.modules[1].authoring_namespace_id.as_ref(), "city/app");
        assert_eq!(hir.imports.len(), 1);
        assert_eq!(hir.imports[0].target.raw(), 0);
        assert_eq!(hir.imports[0].source_span.source_document_key(), "city/app");
        assert_eq!(hir.modules[1].imports.start(), 0);
        assert_eq!(hir.modules[1].imports.len(), 1);
        assert_eq!(hir.lane_edges.len(), 3);
        assert_eq!(
            hir.lane_edges
                .iter()
                .map(|edge| edge.stable_key.as_ref())
                .collect::<Vec<_>>(),
            ["edge-b", "edge-a", "edge-c"]
        );
        let edge_a = &hir.lane_edges[1];
        let targets = hir.lane_edge_references[edge_a.successors.as_usize_range()]
            .iter()
            .map(|reference| reference.target.raw())
            .collect::<Vec<_>>();
        assert_eq!(targets, [2, 0]);
        assert!(hir.modules[0].imports.is_empty());
        assert_eq!(hir.hir_record_count, 16);
    }

    #[test]
    fn hir_reports_every_unknown_target_in_canonical_module_order() {
        let z_successors = [LaneEdgeReference::local("missing-z")];
        let a_successors = [LaneEdgeReference::local("missing-a")];
        let unit = unit([
            module("city/z", &[], &[("edge-z", &z_successors)]),
            module("city/a", &[], &[("edge-a", &a_successors)]),
        ]);
        let diagnostics = match build_hir(&unit) {
            Ok(_) => panic!("unknown targets must reject HIR construction"),
            Err(diagnostics) => diagnostics,
        };

        assert_eq!(diagnostics.diagnostics().len(), 2);
        assert_eq!(
            diagnostics
                .diagnostics()
                .iter()
                .map(|diagnostic| diagnostic.stable_key().unwrap())
                .collect::<Vec<_>>(),
            ["edge-a", "edge-z"]
        );
        assert!(diagnostics.diagnostics().iter().all(|diagnostic| {
            diagnostic.code() == DiagnosticCode::UnknownReferenceTarget
                && diagnostic.primary_span().is_some()
                && diagnostic.related_locations().len() == 1
        }));
    }

    #[test]
    fn hir_symbol_and_reference_order_ignore_declaration_insertion_order() {
        let successors = [
            LaneEdgeReference::local("edge-c"),
            LaneEdgeReference::local("edge-b"),
        ];
        let left = unit([module(
            "city/a",
            &[],
            &[("edge-a", &successors), ("edge-b", &[]), ("edge-c", &[])],
        )]);
        let right = unit([module(
            "city/a",
            &[],
            &[("edge-c", &[]), ("edge-a", &successors), ("edge-b", &[])],
        )]);
        let left = build_hir(&left).unwrap();
        let right = build_hir(&right).unwrap();

        let projection = |hir: &HirUnit| {
            hir.lane_edges
                .iter()
                .map(|edge| {
                    (
                        edge.stable_key.to_string(),
                        hir.lane_edge_references[edge.successors.as_usize_range()]
                            .iter()
                            .map(|reference| reference.target.raw())
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(projection(&left), projection(&right));
        assert_eq!(
            left.lane_edges
                .iter()
                .map(|edge| edge.stable_id)
                .collect::<Vec<_>>(),
            right
                .lane_edges
                .iter()
                .map(|edge| edge.stable_id)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            projection(&left),
            [
                ("edge-a".into(), vec![1, 2]),
                ("edge-b".into(), vec![]),
                ("edge-c".into(), vec![]),
            ]
        );
    }

    #[test]
    fn hir_lane_edge_identity_uses_namespace_and_key_instead_of_dense_position() {
        let city_a = unit([module("city/a", &[], &[("edge-a", &[]), ("edge-b", &[])])]);
        let city_b = unit([module("city/b", &[], &[("edge-a", &[])])]);
        let city_a = build_hir(&city_a).unwrap();
        let city_b = build_hir(&city_b).unwrap();

        assert_ne!(
            city_a.lane_edges[0].stable_id,
            city_a.lane_edges[1].stable_id
        );
        assert_ne!(
            city_a.lane_edges[0].stable_id,
            city_b.lane_edges[0].stable_id
        );
        assert_eq!(
            city_a.lane_edges[0].stable_id.to_string(),
            format!(
                "lfid1_lane-edge_{:x}",
                city_a.lane_edges[0].stable_id.as_untyped()
            )
        );
    }

    #[test]
    fn hir_lane_edge_identity_ignores_non_identity_scalars_and_connections() {
        let baseline = unit([module("city/a", &[], &[("edge-a", &[]), ("edge-b", &[])])]);

        let limits = CompileLimits::p100_initial_v1();
        let mut changed = SyntheticModuleBuilder::new(header("city/a"), &limits).unwrap();
        changed
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge-a",
                length_meters: 99.0,
                speed_limit_meters_per_second: 2.0,
                successors: &[LaneEdgeReference::local("edge-b")],
            })
            .unwrap();
        changed
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge-b",
                length_meters: 1.0,
                speed_limit_meters_per_second: 1.0,
                successors: &[],
            })
            .unwrap();
        let changed = unit([changed.finish().unwrap()]);

        let baseline = build_hir(&baseline).unwrap();
        let changed = build_hir(&changed).unwrap();
        assert_eq!(baseline.lane_edges[0].stable_key.as_ref(), "edge-a");
        assert_eq!(changed.lane_edges[0].stable_key.as_ref(), "edge-a");
        assert_eq!(
            baseline.lane_edges[0].stable_id,
            changed.lane_edges[0].stable_id
        );
    }

    #[test]
    fn hir_checks_record_scratch_and_live_byte_limits_before_stage_allocation() {
        let mut unit = unit([module("city/a", &[], &[("edge-a", &[])])]);
        unit.limits = CompileLimits::p100_initial_v1().with_test_pipeline_limits(
            3,
            u32::MAX,
            u32::MAX,
            u32::MAX,
        );
        let record_failure = match build_hir(&unit) {
            Ok(_) => panic!("HIR record limit must fail closed"),
            Err(diagnostics) => diagnostics,
        };
        assert!(matches!(
            record_failure.diagnostics()[0].payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::HirRecordCount,
                limit: 3,
                observed: 4,
            }
        ));

        unit.limits = CompileLimits::p100_initial_v1().with_test_pipeline_limits(
            u32::MAX,
            u32::MAX,
            0,
            u32::MAX,
        );
        let scratch_failure = match build_hir(&unit) {
            Ok(_) => panic!("HIR scratch limit must fail closed"),
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

        let source_live_bytes = u32::try_from(unit.controlled_live_bytes).unwrap();
        unit.limits = CompileLimits::p100_initial_v1().with_test_pipeline_limits(
            u32::MAX,
            u32::MAX,
            u32::MAX,
            source_live_bytes,
        );
        let live_failure = match build_hir(&unit) {
            Ok(_) => panic!("HIR live byte limit must fail closed"),
            Err(diagnostics) => diagnostics,
        };
        assert!(live_failure.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::CompilerControlledLiveBytes,
                limit,
                observed,
            } if *limit == u64::from(source_live_bytes) && observed > limit
        )));
    }
}
