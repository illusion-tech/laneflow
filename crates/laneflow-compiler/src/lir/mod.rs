//! 中层中间表示（MIR）到已验证规范低层中间表示（Canonical LIR）的冻结阶段。
//!
//! 稳定实体按各自完整 Identity v1 前像字节排序，表下标冻结为有类型逻辑序号；车道
//! 连接、横断面成员、覆盖链、路口路径和派生内部边所有权全部改写为同一 LIR 实例内的
//! 有类型序号。身份
//! 字段和值分别进入连续表与共享字节池；来源位置留给同次编译的源映射伴随数据，不进入
//! LIR 或语义摘要。
//!
//! `LirUnit` 仍是 crate 私有阶段结果。它不是可移植规范制品或静态镜像 ABI；本模块的
//! 语义摘要只用于验证干净编译的确定性，不能冒充后继制品摘要或路网修订摘要。

use core::cmp::Ordering;

use laneflow_static_contract::{
    AccessEffect, AccessRuleOrdinal, AuthoringLaneOrdinal, CanonicalFrameOrdinal, EntityKind,
    FacilityBandOrdinal, FieldTag, JunctionOrdinal, LaneEdgeId, LaneEdgeOrdinal, LaneGroupOrdinal,
    ManeuverGateOrdinal, ManeuverPathOrdinal, MovementOrdinal, ParkingAreaOrdinal,
    ParkingSpaceOrdinal, ParticipantClassOrdinal, RoadCorridorOrdinal, RoadSectionOrdinal,
    SignalAspect, SignalControllerOrdinal, SignalGroupOrdinal, SignalPhaseOrdinal,
    StaticRouteOrdinal, StopLineOrdinal, VehicleProfileOrdinal, WaitingZoneOrdinal,
};

use crate::arena::{ArenaKey, ArenaKeyOverflow, TableRange};
use crate::geometry_profile::GeometryCompilationProfiles;
use crate::mir::{
    MirAccessRuleKey, MirAccessTarget, MirAuthoringLaneKey, MirCanonicalFrameKey,
    MirCorridorElement, MirFacilityBandKey, MirJunctionKey, MirLaneEdgeConnection, MirLaneEdgeKey,
    MirLaneGroupKey, MirManeuverGateKey, MirManeuverPathKey, MirMovementKey, MirParkingAreaKey,
    MirParkingSpaceKey, MirParticipantClassKey, MirRoadCorridorKey, MirRoadSectionKey,
    MirSignalControl, MirSignalControllerGroup, MirSignalControllerKey, MirSignalGroupKey,
    MirSignalPhaseKey, MirSignalPhaseState, MirStaticRouteKey, MirStopLineKey, MirUnit,
    MirVehicleProfileKey, MirWaitingZoneKey,
};
use crate::{CompilationUnit, CompileLimitDimension, Diagnostic, DiagnosticBundle, SourceLocation};

/// 与公开制品版本轴无关的编译器私有摘要域。
const LIR_SEMANTIC_DIGEST_DOMAIN: &[u8] = b"LANEFLOW-COMPILER-LIR-SEMANTIC-V1\0";
/// `ordinal + stable_id + identity_range + length + speed + successor_range + route_range`。
const LIR_LANE_EDGE_LOGICAL_BYTES: u64 = 4 + 16 + 8 + 8 + 8 + 8 + 8;
/// `field_tag + value_range`；表归属已经给出实体种类，不在每项重复编码。
const LIR_IDENTITY_FIELD_LOGICAL_BYTES: u64 = 2 + 8;
const LIR_SUCCESSOR_LOGICAL_BYTES: u64 = 4;
const LIR_CORRIDOR_LOGICAL_BYTES: u64 = 4 + 16 + 8 + 4 + 8;
const LIR_SECTION_LOGICAL_BYTES: u64 = 4 + 16 + 8 + 4 + 4 + 8;
const LIR_LANE_LOGICAL_BYTES: u64 = 4 + 16 + 8 + 4 + 8 + 1 + 4;
const LIR_GROUP_LOGICAL_BYTES: u64 = 4 + 16 + 8 + 4 + 8;
const LIR_BAND_LOGICAL_BYTES: u64 = 4 + 16 + 8 + 4 + 4;
const LIR_JUNCTION_LOGICAL_BYTES: u64 = 4 + 16 + 8 + 8;
const LIR_MOVEMENT_LOGICAL_BYTES: u64 = 4 + 16 + 8 + 4 + 4 + 4 + 8;
const LIR_MANEUVER_PATH_LOGICAL_BYTES: u64 = 4 + 16 + 8 + 4 + 8 + 8 + 8 + 8;
const LIR_STOP_LINE_LOGICAL_BYTES: u64 = 4 + 16 + 8 + 4 + 8;
const LIR_MANEUVER_GATE_LOGICAL_BYTES: u64 = 4 + 16 + 8 + 4 + 4 + 4 + 1 + 4 + 8;
const LIR_WAITING_ZONE_LOGICAL_BYTES: u64 = 4 + 16 + 8 + 4 + 4 + 4 + 4 + 8;
const LIR_STATIC_ROUTE_LOGICAL_BYTES: u64 = 4 + 16 + 8 + 8 + 8 + 8 + 8 + 8;
const LIR_MANEUVER_OCCURRENCE_LOGICAL_BYTES: u64 = 4 + 4 + 4 + 8 + 8;
const LIR_GATE_OCCURRENCE_LOGICAL_BYTES: u64 = 4 + 4 + 4 + 1 + 4 + 4 + 1 + 4;
const LIR_WAITING_OCCURRENCE_LOGICAL_BYTES: u64 = 4 + 4 + 4 + 4 + 4 + 4;
const LIR_ROUTE_OCCURRENCE_REF_LOGICAL_BYTES: u64 = 4 + 4;
const LIR_JUNCTION_INTERNAL_EDGE_LOGICAL_BYTES: u64 = 4 + 4;
const LIR_TYPED_ORDINAL_LOGICAL_BYTES: u64 = 4;
const LIR_SIGNAL_GROUP_LOGICAL_BYTES: u64 = 4 + 16 + 8 + 4 + 8;
const LIR_SIGNAL_CONTROLLER_LOGICAL_BYTES: u64 = 4 + 16 + 8 + 8 + 8 + 8 + 8;
const LIR_SIGNAL_PHASE_LOGICAL_BYTES: u64 = 4 + 16 + 8 + 4 + 8 + 8;
const LIR_SIGNAL_PHASE_STATE_LOGICAL_BYTES: u64 = 4 + 1;
const LIR_PARKING_AREA_LOGICAL_BYTES: u64 = 4 + 16 + 8 + 8;
const LIR_PARKING_SPACE_LOGICAL_BYTES: u64 = 4 + 16 + 8 + 1 + 4 + (4 + 8) * 2 + 8 * 4;
const LIR_PARTICIPANT_CLASS_LOGICAL_BYTES: u64 = 4 + 16 + 8 + 1 + 4 + 4 + 4 + 4;
const LIR_VEHICLE_PROFILE_LOGICAL_BYTES: u64 = 4 + 16 + 8 + 4 + 8 * 7;
const LIR_CANONICAL_FRAME_LOGICAL_BYTES: u64 = 4 + 16 + 8;
const LIR_SPATIAL_GEOMETRY_LOGICAL_BYTES: u64 = 4 + 8 + 8 + 4;
const LIR_FACILITY_BAND_GEOMETRY_LOGICAL_BYTES: u64 = 4 + 4 + 8;
const LIR_CANONICAL_POINT_LOGICAL_BYTES: u64 = 4 * 3;
const LIR_SPATIAL_SEGMENT_LOGICAL_BYTES: u64 = 4 * 8;
// target 按 tag+ordinal 计；可选 regulation 按 presence、两个必需字符串区间和一个
// 可选来源区间的最大形状计。实际 UTF-8 内容在下方按字节数另行累加。
const LIR_ACCESS_RULE_LOGICAL_BYTES: u64 = 4 + 16 + 8 + (2 + 4) + 1 + 8 + (1 + 4 + 4 + 1 + 4) + 4;
const LIR_CORRIDOR_ELEMENT_LOGICAL_BYTES: u64 = 2 + 4;
const LIR_GEOMETRY_PROFILE_LOGICAL_BYTES: u64 = 2;
const LIR_SEMANTIC_DIGEST_BYTES: u64 = 32;

/// 一项规范身份字段；值位于 `LirUnit::identity_field_bytes` 的连续区间。
pub(crate) struct LirIdentityField {
    /// Identity v1 登记标签；同一实体内严格按登记顺序保存。
    pub(crate) tag: FieldTag,
    /// 字段原始规范字节，不包含标签和长度前缀。
    pub(crate) value_bytes: TableRange<u8>,
}

/// 已冻结的车道图边静态语义。
pub(crate) struct LirLaneEdge {
    /// 此记录在当前 `lane_edges` 表中的有类型逻辑序号。
    pub(crate) ordinal: LaneEdgeOrdinal,
    /// 由同一记录的完整 Identity v1 字段前像派生的稳定标识。
    pub(crate) stable_id: LaneEdgeId,
    /// 此实体在 `identity_fields` 中的完整、规范有序字段区间。
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    /// 交通权威长度，单位为米；输入阶段已证明有限且严格大于零。
    pub(crate) length_meters: f64,
    /// 基础道路限速，单位为米每秒；输入阶段已证明有限且严格大于零。
    pub(crate) speed_limit_meters_per_second: f64,
    /// 按领域顺序保存的下游边序号区间。
    pub(crate) successors: TableRange<LaneEdgeOrdinal>,
    /// 按路线序号和路线内边下标排序的反向出现项区间。
    pub(crate) static_route_occurrences: TableRange<LirRouteOccurrenceRef>,
}

mod access;
mod control;
mod cross_section;
mod junction;
mod orders;
mod parking;
mod plan;
mod route;
mod signal;
mod spatial;

#[cfg(test)]
mod tests;

pub(crate) use access::{
    LirAccessRegulation, LirAccessRule, LirAccessTarget, LirParticipantClass, LirVehicleProfile,
};
pub(crate) use control::{LirManeuverGate, LirStopLine, LirWaitingZone};
pub(crate) use cross_section::{
    LirAuthoringLane, LirCorridorElement, LirFacilityBand, LirLaneGroup, LirRoadCorridor,
    LirRoadSection,
};
pub(crate) use junction::{LirJunction, LirJunctionInternalEdge, LirManeuverPath, LirMovement};
pub(crate) use orders::{CanonicalOrders, LirEntityOrder, OwnerLocalPermutation};
pub(crate) use parking::{
    LirParkingArea, LirParkingLaneAnchor, LirParkingSpace, LirParkingSpaceGeometry,
};
pub(crate) use plan::LirFreezePlan;
pub(crate) use route::{
    LirGateOccurrence, LirManeuverOccurrence, LirRouteOccurrenceRef, LirStaticRoute,
    LirStaticRouteTransition, LirWaitingZoneOccurrence,
};
pub(crate) use signal::{
    LirSignalControl, LirSignalController, LirSignalGroup, LirSignalPhase, LirSignalPhaseState,
};
pub(crate) use spatial::{
    LirCanonicalFrame, LirCanonicalPoint3F32, LirFacilityBandGeometry, LirLaneEdgeGeometry,
    LirSpatialSegment,
};

/// 当前纵向切片冻结出的连续、目标布局中立 LIR 表。
///
/// 每条边的 `ordinal` 必须等于其切片下标；全部身份字段区间和连接区间均落在本实例的
/// 对应平面表内。`controlled_live_bytes` 只统计成功返回后由本结果持有的请求字节；
/// `peak_controlled_live_bytes` 另保存 MIR、冻结暂存区与新输出共存时的阶段峰值。
pub(crate) struct LirUnit {
    /// `None` 使用两个零 code；道路编辑规范几何使用两个闭合的非零档位 code。
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "consumed by the following artifact emission slice"
        )
    )]
    pub(crate) geometry_profiles: Option<GeometryCompilationProfiles>,
    pub(crate) lane_edges: Box<[LirLaneEdge]>,
    pub(crate) lane_edge_successors: Box<[LaneEdgeOrdinal]>,
    pub(crate) road_corridors: Box<[LirRoadCorridor]>,
    pub(crate) corridor_elements: Box<[LirCorridorElement]>,
    pub(crate) road_sections: Box<[LirRoadSection]>,
    pub(crate) road_section_lanes: Box<[AuthoringLaneOrdinal]>,
    pub(crate) authoring_lanes: Box<[LirAuthoringLane]>,
    pub(crate) authoring_lane_edges: Box<[LaneEdgeOrdinal]>,
    pub(crate) lane_groups: Box<[LirLaneGroup]>,
    pub(crate) lane_group_members: Box<[AuthoringLaneOrdinal]>,
    pub(crate) facility_bands: Box<[LirFacilityBand]>,
    pub(crate) junctions: Box<[LirJunction]>,
    pub(crate) junction_movements: Box<[MovementOrdinal]>,
    pub(crate) movements: Box<[LirMovement]>,
    pub(crate) movement_maneuver_paths: Box<[ManeuverPathOrdinal]>,
    pub(crate) maneuver_paths: Box<[LirManeuverPath]>,
    pub(crate) maneuver_path_edges: Box<[LaneEdgeOrdinal]>,
    pub(crate) junction_internal_edges: Box<[LirJunctionInternalEdge]>,
    pub(crate) stop_lines: Box<[LirStopLine]>,
    pub(crate) maneuver_gates: Box<[LirManeuverGate]>,
    pub(crate) waiting_zones: Box<[LirWaitingZone]>,
    pub(crate) maneuver_path_gates: Box<[ManeuverGateOrdinal]>,
    pub(crate) maneuver_path_waiting_zones: Box<[WaitingZoneOrdinal]>,
    pub(crate) stop_line_maneuver_gates: Box<[ManeuverGateOrdinal]>,
    pub(crate) signal_groups: Box<[LirSignalGroup]>,
    pub(crate) signal_controllers: Box<[LirSignalController]>,
    pub(crate) signal_controller_groups: Box<[SignalGroupOrdinal]>,
    pub(crate) signal_controller_phases: Box<[SignalPhaseOrdinal]>,
    pub(crate) signal_phases: Box<[LirSignalPhase]>,
    pub(crate) signal_phase_states: Box<[LirSignalPhaseState]>,
    pub(crate) signal_group_maneuver_gates: Box<[ManeuverGateOrdinal]>,
    pub(crate) parking_areas: Box<[LirParkingArea]>,
    pub(crate) parking_spaces: Box<[LirParkingSpace]>,
    pub(crate) parking_area_spaces: Box<[ParkingSpaceOrdinal]>,
    pub(crate) participant_classes: Box<[LirParticipantClass]>,
    pub(crate) vehicle_profiles: Box<[LirVehicleProfile]>,
    pub(crate) canonical_frames: Box<[LirCanonicalFrame]>,
    pub(crate) lane_edge_geometries: Box<[LirLaneEdgeGeometry]>,
    pub(crate) facility_band_geometries: Box<[LirFacilityBandGeometry]>,
    pub(crate) canonical_points: Box<[LirCanonicalPoint3F32]>,
    pub(crate) spatial_segments: Box<[LirSpatialSegment]>,
    pub(crate) access_rules: Box<[LirAccessRule]>,
    pub(crate) access_rule_participant_classes: Box<[ParticipantClassOrdinal]>,
    pub(crate) static_routes: Box<[LirStaticRoute]>,
    pub(crate) static_route_edges: Box<[LaneEdgeOrdinal]>,
    pub(crate) static_route_transitions: Box<[LirStaticRouteTransition]>,
    pub(crate) maneuver_occurrences: Box<[LirManeuverOccurrence]>,
    pub(crate) gate_occurrences: Box<[LirGateOccurrence]>,
    pub(crate) waiting_zone_occurrences: Box<[LirWaitingZoneOccurrence]>,
    pub(crate) lane_edge_route_occurrences: Box<[LirRouteOccurrenceRef]>,
    pub(crate) maneuver_path_route_occurrences: Box<[LirRouteOccurrenceRef]>,
    pub(crate) maneuver_gate_route_occurrences: Box<[LirRouteOccurrenceRef]>,
    pub(crate) waiting_zone_route_occurrences: Box<[LirRouteOccurrenceRef]>,
    pub(crate) identity_fields: Box<[LirIdentityField]>,
    pub(crate) identity_field_bytes: Box<[u8]>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) semantic_digest: [u8; 32],
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) lir_record_count: u64,
    pub(crate) output_bytes: u64,
    pub(crate) controlled_live_bytes: u64,
    pub(crate) peak_controlled_live_bytes: u64,
}

/// LIR 与冻结源映射所需的全部临时阶段排列。
pub(crate) struct LirFreezeOutput {
    pub(crate) lir: LirUnit,
    pub(crate) lane_edges: LirEntityOrder<MirLaneEdgeKey, LaneEdgeOrdinal>,
    pub(crate) lane_edge_successors: OwnerLocalPermutation<MirLaneEdgeConnection>,
    pub(crate) road_corridors: LirEntityOrder<MirRoadCorridorKey, RoadCorridorOrdinal>,
    pub(crate) road_sections: LirEntityOrder<MirRoadSectionKey, RoadSectionOrdinal>,
    pub(crate) authoring_lanes: LirEntityOrder<MirAuthoringLaneKey, AuthoringLaneOrdinal>,
    pub(crate) lane_groups: LirEntityOrder<MirLaneGroupKey, LaneGroupOrdinal>,
    pub(crate) facility_bands: LirEntityOrder<MirFacilityBandKey, FacilityBandOrdinal>,
    pub(crate) junctions: LirEntityOrder<MirJunctionKey, JunctionOrdinal>,
    pub(crate) movements: LirEntityOrder<MirMovementKey, MovementOrdinal>,
    pub(crate) maneuver_paths: LirEntityOrder<MirManeuverPathKey, ManeuverPathOrdinal>,
    pub(crate) canonical_mir_internal_edge_order: Box<[u32]>,
    pub(crate) stop_lines: LirEntityOrder<MirStopLineKey, StopLineOrdinal>,
    pub(crate) maneuver_gates: LirEntityOrder<MirManeuverGateKey, ManeuverGateOrdinal>,
    pub(crate) waiting_zones: LirEntityOrder<MirWaitingZoneKey, WaitingZoneOrdinal>,
    pub(crate) signal_groups: LirEntityOrder<MirSignalGroupKey, SignalGroupOrdinal>,
    pub(crate) signal_controllers: LirEntityOrder<MirSignalControllerKey, SignalControllerOrdinal>,
    pub(crate) signal_phases: LirEntityOrder<MirSignalPhaseKey, SignalPhaseOrdinal>,
    pub(crate) signal_controller_groups: OwnerLocalPermutation<MirSignalControllerGroup>,
    pub(crate) signal_phase_states: OwnerLocalPermutation<MirSignalPhaseState>,
    pub(crate) parking_areas: LirEntityOrder<MirParkingAreaKey, ParkingAreaOrdinal>,
    pub(crate) parking_spaces: LirEntityOrder<MirParkingSpaceKey, ParkingSpaceOrdinal>,
    pub(crate) participant_classes: LirEntityOrder<MirParticipantClassKey, ParticipantClassOrdinal>,
    pub(crate) vehicle_profiles: LirEntityOrder<MirVehicleProfileKey, VehicleProfileOrdinal>,
    pub(crate) canonical_frames: LirEntityOrder<MirCanonicalFrameKey, CanonicalFrameOrdinal>,
    pub(crate) access_rules: LirEntityOrder<MirAccessRuleKey, AccessRuleOrdinal>,
    pub(crate) static_routes: LirEntityOrder<MirStaticRouteKey, StaticRouteOrdinal>,
}

impl LirFreezeOutput {
    /// 返回临时有类型实体映射与 owner-local 行排列的真实请求容量字节。
    pub(crate) fn mapping_bytes(&self) -> u64 {
        self.lane_edges
            .mapping_bytes()
            .saturating_add(self.lane_edge_successors.mapping_bytes())
            .saturating_add(self.road_corridors.mapping_bytes())
            .saturating_add(self.road_sections.mapping_bytes())
            .saturating_add(self.authoring_lanes.mapping_bytes())
            .saturating_add(self.lane_groups.mapping_bytes())
            .saturating_add(self.facility_bands.mapping_bytes())
            .saturating_add(self.junctions.mapping_bytes())
            .saturating_add(self.movements.mapping_bytes())
            .saturating_add(self.maneuver_paths.mapping_bytes())
            .saturating_add(requested_bytes::<u32>(
                self.canonical_mir_internal_edge_order
                    .len()
                    .try_into()
                    .unwrap_or(u64::MAX),
            ))
            .saturating_add(self.stop_lines.mapping_bytes())
            .saturating_add(self.maneuver_gates.mapping_bytes())
            .saturating_add(self.waiting_zones.mapping_bytes())
            .saturating_add(self.signal_groups.mapping_bytes())
            .saturating_add(self.signal_controllers.mapping_bytes())
            .saturating_add(self.signal_phases.mapping_bytes())
            .saturating_add(self.signal_controller_groups.mapping_bytes())
            .saturating_add(self.signal_phase_states.mapping_bytes())
            .saturating_add(self.parking_areas.mapping_bytes())
            .saturating_add(self.parking_spaces.mapping_bytes())
            .saturating_add(self.participant_classes.mapping_bytes())
            .saturating_add(self.vehicle_profiles.mapping_bytes())
            .saturating_add(self.canonical_frames.mapping_bytes())
            .saturating_add(self.access_rules.mapping_bytes())
            .saturating_add(self.static_routes.mapping_bytes())
    }
}

/// 将全部 MIR 引用重映射到规范 LIR 序号，并原子冻结连续只读表。
///
/// 排序键是 Identity v1 完整前像的逐字节顺序，而不是 `StableId128` 或普通字符串
/// 顺序。每种实体分别比较其登记字段的编码片段；字段长度采用 `u32_le`，父项稳定标识
/// 使用固定 16 字节原值，与规范编码器保持一致。
///
/// # Errors
///
/// 当 LIR 记录数、阶段暂存字节、输出字节、编译器控制存续字节或有类型 `u32` 边界超过
/// 所选资源配置档时，返回结构化资源诊断且不返回部分 LIR。
pub(crate) fn freeze_lir(
    unit: &CompilationUnit,
    mir: &MirUnit,
) -> Result<LirFreezeOutput, DiagnosticBundle> {
    let plan = LirFreezePlan::analyze(unit, mir);
    plan.check_limits(unit, mir)?;
    let primary_span = mir.modules.first().map(|module| module.source_span.clone());

    let edge_capacity = usize::try_from(plan.lane_edge_count)
        .map_err(|_| ordinal_overflow(&unit.limits, primary_span.clone()))?;
    let successor_capacity = usize::try_from(plan.successor_count)
        .map_err(|_| ordinal_overflow(&unit.limits, primary_span.clone()))?;
    let identity_field_capacity = usize::try_from(plan.identity_field_count)
        .map_err(|_| ordinal_overflow(&unit.limits, primary_span.clone()))?;
    let identity_byte_capacity = usize::try_from(plan.identity_field_byte_count)
        .map_err(|_| output_overflow(&unit.limits, primary_span.clone()))?;

    let orders = CanonicalOrders::build(mir, &unit.limits, primary_span.clone())?;

    let mut lane_edges = Vec::with_capacity(edge_capacity);
    let mut successors = Vec::with_capacity(successor_capacity);
    let mut lane_edge_successor_mir_rows =
        Vec::<ArenaKey<MirLaneEdgeConnection>>::with_capacity(successor_capacity);
    let mut identity_fields = Vec::with_capacity(identity_field_capacity);
    let mut identity_field_bytes = Vec::with_capacity(identity_byte_capacity);
    for mir_key in orders.lane_edges.stage_keys_in_lir_order().iter().copied() {
        let edge = &mir.lane_edges[mir_key.index()];
        let namespace = &mir.modules[edge.module.index()].authoring_namespace_id;
        let identity_start = identity_fields.len();
        push_identity_field(
            &mut identity_fields,
            &mut identity_field_bytes,
            FieldTag::AuthoringNamespaceId,
            namespace.as_bytes(),
            &unit.limits,
            primary_span.clone(),
        )?;
        push_identity_field(
            &mut identity_fields,
            &mut identity_field_bytes,
            FieldTag::LaneEdgeKey,
            edge.stable_key.as_bytes(),
            &unit.limits,
            primary_span.clone(),
        )?;

        let successor_start = successors.len();
        let successor_row_start = lane_edge_successor_mir_rows.len();
        lane_edge_successor_mir_rows.extend(edge.connections.as_usize_range().map(|index| {
            ArenaKey::from_raw(
                u32::try_from(index)
                    .expect("MIR relation range precheck proved row index fits u32"),
            )
        }));
        lane_edge_successor_mir_rows[successor_row_start..].sort_unstable_by_key(|row| {
            orders
                .lane_edges
                .ordinal(mir.lane_edge_connections[row.index()].target)
        });
        successors.extend(
            lane_edge_successor_mir_rows[successor_row_start..]
                .iter()
                .map(|row| {
                    orders
                        .lane_edges
                        .ordinal(mir.lane_edge_connections[row.index()].target)
                }),
        );
        debug_assert!(
            successors[successor_start..]
                .windows(2)
                .all(|pair| pair[0] < pair[1]),
            "LaneEdge successor sets must be strictly ordered by target typed ordinal"
        );
        let ordinal = orders.lane_edges.ordinal(mir_key);
        lane_edges.push(LirLaneEdge {
            ordinal,
            stable_id: edge.stable_id,
            identity_fields: TableRange::try_from_usize(
                identity_start,
                identity_fields.len().saturating_sub(identity_start),
            )
            .map_err(|overflow| table_overflow(overflow, &unit.limits, primary_span.clone()))?,
            length_meters: edge.length_meters,
            speed_limit_meters_per_second: edge.speed_limit_meters_per_second,
            successors: TableRange::try_from_usize(
                successor_start,
                successors.len().saturating_sub(successor_start),
            )
            .map_err(|overflow| table_overflow(overflow, &unit.limits, primary_span.clone()))?,
            static_route_occurrences: TableRange::empty(),
        });
    }

    let mut road_corridors = Vec::with_capacity(mir.road_corridors.len());
    let mut corridor_elements = Vec::with_capacity(mir.corridor_elements.len());
    for mir_key in orders
        .road_corridors
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let corridor = &mir.road_corridors[mir_key.index()];
        let identity_range = push_lir_identity(
            &mut identity_fields,
            &mut identity_field_bytes,
            FieldTag::CorridorKey,
            &mir.modules[corridor.module.index()].authoring_namespace_id,
            &corridor.stable_key,
            None,
            &unit.limits,
            primary_span.clone(),
        )?;
        let relation_start = corridor_elements.len();
        corridor_elements.extend(
            mir.corridor_elements[corridor.elements.as_usize_range()]
                .iter()
                .map(|element| match element {
                    MirCorridorElement::RoadSection { road_section, .. } => {
                        LirCorridorElement::RoadSection(orders.road_sections.ordinal(*road_section))
                    }
                    MirCorridorElement::FacilityBand { facility_band, .. } => {
                        LirCorridorElement::FacilityBand(
                            orders.facility_bands.ordinal(*facility_band),
                        )
                    }
                }),
        );
        road_corridors.push(LirRoadCorridor {
            ordinal: orders.road_corridors.ordinal(mir_key),
            stable_id: corridor.stable_id,
            identity_fields: identity_range,
            reference_section: orders.road_sections.ordinal(corridor.reference_section),
            elements: relation_range(
                relation_start,
                corridor_elements.len(),
                &unit.limits,
                primary_span.clone(),
            )?,
        });
    }

    let mut road_sections = Vec::with_capacity(mir.road_sections.len());
    let mut road_section_lanes = Vec::with_capacity(mir.authoring_lanes.len());
    for mir_key in orders
        .road_sections
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let section = &mir.road_sections[mir_key.index()];
        let parent_id = mir.road_corridors[section.road_corridor.index()]
            .stable_id
            .into_untyped();
        let identity_range = push_lir_identity(
            &mut identity_fields,
            &mut identity_field_bytes,
            FieldTag::SectionKey,
            &mir.modules[section.module.index()].authoring_namespace_id,
            &section.stable_key,
            Some((FieldTag::RoadCorridorStableId, parent_id.as_bytes())),
            &unit.limits,
            primary_span.clone(),
        )?;
        let relation_start = road_section_lanes.len();
        road_section_lanes.extend(section.lanes.as_usize_range().map(|index| {
            orders
                .authoring_lanes
                .ordinal(MirAuthoringLaneKey::from_raw(
                    u32::try_from(index).expect("MIR range prevalidated as u32"),
                ))
        }));
        road_sections.push(LirRoadSection {
            ordinal: orders.road_sections.ordinal(mir_key),
            stable_id: section.stable_id,
            identity_fields: identity_range,
            road_corridor: orders.road_corridors.ordinal(section.road_corridor),
            kind_id: section.kind_id.as_ref().into(),
            lanes: relation_range(
                relation_start,
                road_section_lanes.len(),
                &unit.limits,
                primary_span.clone(),
            )?,
        });
    }

    let mut authoring_lanes = Vec::with_capacity(mir.authoring_lanes.len());
    let mut authoring_lane_edges = Vec::with_capacity(mir.authoring_lane_edges.len());
    for mir_key in orders
        .authoring_lanes
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let lane = &mir.authoring_lanes[mir_key.index()];
        let parent_id = mir.road_sections[lane.road_section.index()]
            .stable_id
            .into_untyped();
        let identity_range = push_lir_identity(
            &mut identity_fields,
            &mut identity_field_bytes,
            FieldTag::LaneKey,
            &mir.modules[lane.module.index()].authoring_namespace_id,
            &lane.stable_key,
            Some((FieldTag::RoadSectionStableId, parent_id.as_bytes())),
            &unit.limits,
            primary_span.clone(),
        )?;
        let relation_start = authoring_lane_edges.len();
        authoring_lane_edges.extend(
            mir.authoring_lane_edges[lane.edge_chain.as_usize_range()]
                .iter()
                .map(|edge| orders.lane_edges.ordinal(edge.target)),
        );
        authoring_lanes.push(LirAuthoringLane {
            ordinal: orders.authoring_lanes.ordinal(mir_key),
            stable_id: lane.stable_id,
            identity_fields: identity_range,
            road_section: orders.road_sections.ordinal(lane.road_section),
            edge_chain: relation_range(
                relation_start,
                authoring_lane_edges.len(),
                &unit.limits,
                primary_span.clone(),
            )?,
            lane_group: lane.lane_group.map(|key| orders.lane_groups.ordinal(key)),
        });
    }

    let mut lane_groups = Vec::with_capacity(mir.lane_groups.len());
    let mut lane_group_members = Vec::with_capacity(mir.lane_group_members.len());
    for mir_key in orders.lane_groups.stage_keys_in_lir_order().iter().copied() {
        let group = &mir.lane_groups[mir_key.index()];
        let parent_id = mir.road_sections[group.road_section.index()]
            .stable_id
            .into_untyped();
        let identity_range = push_lir_identity(
            &mut identity_fields,
            &mut identity_field_bytes,
            FieldTag::LaneGroupKey,
            &mir.modules[group.module.index()].authoring_namespace_id,
            &group.stable_key,
            Some((FieldTag::RoadSectionStableId, parent_id.as_bytes())),
            &unit.limits,
            primary_span.clone(),
        )?;
        let relation_start = lane_group_members.len();
        lane_group_members.extend(
            mir.lane_group_members[group.members.as_usize_range()]
                .iter()
                .map(|member| orders.authoring_lanes.ordinal(member.lane)),
        );
        lane_groups.push(LirLaneGroup {
            ordinal: orders.lane_groups.ordinal(mir_key),
            stable_id: group.stable_id,
            identity_fields: identity_range,
            road_section: orders.road_sections.ordinal(group.road_section),
            members: relation_range(
                relation_start,
                lane_group_members.len(),
                &unit.limits,
                primary_span.clone(),
            )?,
        });
    }

    let mut facility_bands = Vec::with_capacity(mir.facility_bands.len());
    for mir_key in orders
        .facility_bands
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let band = &mir.facility_bands[mir_key.index()];
        let parent_id = mir.road_corridors[band.road_corridor.index()]
            .stable_id
            .into_untyped();
        let identity_range = push_lir_identity(
            &mut identity_fields,
            &mut identity_field_bytes,
            FieldTag::FacilityBandKey,
            &mir.modules[band.module.index()].authoring_namespace_id,
            &band.stable_key,
            Some((FieldTag::RoadCorridorStableId, parent_id.as_bytes())),
            &unit.limits,
            primary_span.clone(),
        )?;
        facility_bands.push(LirFacilityBand {
            ordinal: orders.facility_bands.ordinal(mir_key),
            stable_id: band.stable_id,
            identity_fields: identity_range,
            road_corridor: orders.road_corridors.ordinal(band.road_corridor),
            kind_id: band.kind_id.as_ref().into(),
        });
    }

    let mut junctions = Vec::with_capacity(mir.junctions.len());
    let mut junction_movements = Vec::with_capacity(mir.junction_movements.len());
    for mir_key in orders.junctions.stage_keys_in_lir_order().iter().copied() {
        let junction = &mir.junctions[mir_key.index()];
        let identity_range = push_lir_identity(
            &mut identity_fields,
            &mut identity_field_bytes,
            FieldTag::JunctionKey,
            &mir.modules[junction.module.index()].authoring_namespace_id,
            &junction.stable_key,
            None,
            &unit.limits,
            primary_span.clone(),
        )?;
        let relation_start = junction_movements.len();
        junction_movements.extend(
            mir.junction_movements[junction.movements.as_usize_range()]
                .iter()
                .map(|member| orders.movements.ordinal(member.movement)),
        );
        // 所有者成员关系是集合语义；按子实体规范序号冻结，避免声明先后进入语义摘要。
        junction_movements[relation_start..].sort_unstable();
        junctions.push(LirJunction {
            ordinal: orders.junctions.ordinal(mir_key),
            stable_id: junction.stable_id,
            identity_fields: identity_range,
            movements: relation_range(
                relation_start,
                junction_movements.len(),
                &unit.limits,
                primary_span.clone(),
            )?,
        });
    }

    let mut movements = Vec::with_capacity(mir.movements.len());
    let mut movement_maneuver_paths = Vec::with_capacity(mir.movement_maneuver_paths.len());
    for mir_key in orders.movements.stage_keys_in_lir_order().iter().copied() {
        let movement = &mir.movements[mir_key.index()];
        let identity_start = identity_fields.len();
        for (tag, value) in [
            (
                FieldTag::AuthoringNamespaceId,
                mir.modules[movement.module.index()]
                    .authoring_namespace_id
                    .as_bytes(),
            ),
            (FieldTag::MovementKey, movement.stable_key.as_bytes()),
            (
                FieldTag::DirectedEntryApproachKey,
                movement.directed_entry_approach_key.as_bytes(),
            ),
            (
                FieldTag::DirectedExitApproachKey,
                movement.directed_exit_approach_key.as_bytes(),
            ),
            (
                FieldTag::JunctionStableId,
                mir.junctions[movement.junction.index()]
                    .stable_id
                    .as_untyped()
                    .as_bytes(),
            ),
        ] {
            push_identity_field(
                &mut identity_fields,
                &mut identity_field_bytes,
                tag,
                value,
                &unit.limits,
                primary_span.clone(),
            )?;
        }
        let relation_start = movement_maneuver_paths.len();
        movement_maneuver_paths.extend(
            mir.movement_maneuver_paths[movement.maneuver_paths.as_usize_range()]
                .iter()
                .map(|member| orders.maneuver_paths.ordinal(member.maneuver_path)),
        );
        movement_maneuver_paths[relation_start..].sort_unstable();
        movements.push(LirMovement {
            ordinal: orders.movements.ordinal(mir_key),
            stable_id: movement.stable_id,
            identity_fields: relation_range(
                identity_start,
                identity_fields.len(),
                &unit.limits,
                primary_span.clone(),
            )?,
            junction: orders.junctions.ordinal(movement.junction),
            directed_entry_approach_key: movement.directed_entry_approach_key.as_ref().into(),
            directed_exit_approach_key: movement.directed_exit_approach_key.as_ref().into(),
            maneuver_paths: relation_range(
                relation_start,
                movement_maneuver_paths.len(),
                &unit.limits,
                primary_span.clone(),
            )?,
        });
    }

    let mut maneuver_paths = Vec::with_capacity(mir.maneuver_paths.len());
    let mut maneuver_path_edges = Vec::with_capacity(mir.maneuver_path_edges.len());
    let mut maneuver_path_gates = Vec::with_capacity(mir.maneuver_path_gates.len());
    let mut maneuver_path_waiting_zones = Vec::with_capacity(mir.maneuver_path_waiting_zones.len());
    for mir_key in orders
        .maneuver_paths
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let path = &mir.maneuver_paths[mir_key.index()];
        let edges = &mir.maneuver_path_edges[path.edges.as_usize_range()];
        let identity_start = identity_fields.len();
        for (tag, value) in [
            (
                FieldTag::AuthoringNamespaceId,
                mir.modules[path.module.index()]
                    .authoring_namespace_id
                    .as_bytes(),
            ),
            (FieldTag::PathKey, path.stable_key.as_bytes()),
            (
                FieldTag::MovementStableId,
                mir.movements[path.movement.index()]
                    .stable_id
                    .as_untyped()
                    .as_bytes(),
            ),
            (
                FieldTag::EntryEdgeStableId,
                mir.lane_edges[edges[0].target.index()]
                    .stable_id
                    .as_untyped()
                    .as_bytes(),
            ),
            (
                FieldTag::ExitEdgeStableId,
                mir.lane_edges[edges[edges.len() - 1].target.index()]
                    .stable_id
                    .as_untyped()
                    .as_bytes(),
            ),
        ] {
            push_identity_field(
                &mut identity_fields,
                &mut identity_field_bytes,
                tag,
                value,
                &unit.limits,
                primary_span.clone(),
            )?;
        }
        let edge_start = maneuver_path_edges.len();
        maneuver_path_edges.extend(
            edges
                .iter()
                .map(|edge| orders.lane_edges.ordinal(edge.target)),
        );
        let gate_start = maneuver_path_gates.len();
        maneuver_path_gates.extend(
            mir.maneuver_path_gates[path.maneuver_gates.as_usize_range()]
                .iter()
                .map(|member| orders.maneuver_gates.ordinal(member.maneuver_gate)),
        );
        let waiting_start = maneuver_path_waiting_zones.len();
        maneuver_path_waiting_zones.extend(
            mir.maneuver_path_waiting_zones[path.waiting_zones.as_usize_range()]
                .iter()
                .map(|member| orders.waiting_zones.ordinal(member.waiting_zone)),
        );
        maneuver_paths.push(LirManeuverPath {
            ordinal: orders.maneuver_paths.ordinal(mir_key),
            stable_id: path.stable_id,
            identity_fields: relation_range(
                identity_start,
                identity_fields.len(),
                &unit.limits,
                primary_span.clone(),
            )?,
            movement: orders.movements.ordinal(path.movement),
            edges: relation_range(
                edge_start,
                maneuver_path_edges.len(),
                &unit.limits,
                primary_span.clone(),
            )?,
            maneuver_gates: relation_range(
                gate_start,
                maneuver_path_gates.len(),
                &unit.limits,
                primary_span.clone(),
            )?,
            waiting_zones: relation_range(
                waiting_start,
                maneuver_path_waiting_zones.len(),
                &unit.limits,
                primary_span.clone(),
            )?,
            static_route_occurrences: TableRange::empty(),
        });
    }

    let mut canonical_mir_internal_edge_order: Vec<u32> = (0..mir.junction_internal_edges.len())
        .map(|index| u32::try_from(index).expect("LIR precheck proved relation count fits u32"))
        .collect();
    canonical_mir_internal_edge_order.sort_unstable_by_key(|index| {
        orders
            .lane_edges
            .ordinal(mir.junction_internal_edges[*index as usize].edge)
    });
    let junction_internal_edges = canonical_mir_internal_edge_order
        .iter()
        .map(|index| {
            let relation = &mir.junction_internal_edges[*index as usize];
            LirJunctionInternalEdge {
                edge: orders.lane_edges.ordinal(relation.edge),
                junction: orders.junctions.ordinal(relation.junction),
            }
        })
        .collect::<Vec<_>>();

    let mut stop_lines = Vec::with_capacity(mir.stop_lines.len());
    let mut stop_line_maneuver_gates = Vec::with_capacity(mir.stop_line_maneuver_gates.len());
    for mir_key in orders.stop_lines.stage_keys_in_lir_order().iter().copied() {
        let stop_line = &mir.stop_lines[mir_key.index()];
        let identity_range = push_lir_identity(
            &mut identity_fields,
            &mut identity_field_bytes,
            FieldTag::StopLineKey,
            &mir.modules[stop_line.module.index()].authoring_namespace_id,
            &stop_line.stable_key,
            None,
            &unit.limits,
            primary_span.clone(),
        )?;
        let relation_start = stop_line_maneuver_gates.len();
        stop_line_maneuver_gates.extend(
            mir.stop_line_maneuver_gates[stop_line.maneuver_gates.as_usize_range()]
                .iter()
                .map(|member| orders.maneuver_gates.ordinal(member.maneuver_gate)),
        );
        // 共享静态路网要求同一停止线的门成员按 LIR 序号严格递增；MIR 仍按 stable_id
        // 排列，映射后必须再按序号冻结。
        stop_line_maneuver_gates[relation_start..].sort_unstable();
        stop_lines.push(LirStopLine {
            ordinal: orders.stop_lines.ordinal(mir_key),
            stable_id: stop_line.stable_id,
            identity_fields: identity_range,
            lane_edge: orders.lane_edges.ordinal(stop_line.lane_edge),
            maneuver_gates: relation_range(
                relation_start,
                stop_line_maneuver_gates.len(),
                &unit.limits,
                primary_span.clone(),
            )?,
        });
    }

    let mut maneuver_gates = Vec::with_capacity(mir.maneuver_gates.len());
    for mir_key in orders
        .maneuver_gates
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let gate = &mir.maneuver_gates[mir_key.index()];
        let identity_start = identity_fields.len();
        for (tag, value) in [
            (
                FieldTag::AuthoringNamespaceId,
                mir.modules[gate.module.index()]
                    .authoring_namespace_id
                    .as_bytes(),
            ),
            (
                FieldTag::ManeuverPathStableId,
                mir.maneuver_paths[gate.maneuver_path.index()]
                    .stable_id
                    .as_untyped()
                    .as_bytes(),
            ),
            (FieldTag::GateKey, gate.stable_key.as_bytes()),
        ] {
            push_identity_field(
                &mut identity_fields,
                &mut identity_field_bytes,
                tag,
                value,
                &unit.limits,
                primary_span.clone(),
            )?;
        }
        maneuver_gates.push(LirManeuverGate {
            ordinal: orders.maneuver_gates.ordinal(mir_key),
            stable_id: gate.stable_id,
            identity_fields: relation_range(
                identity_start,
                identity_fields.len(),
                &unit.limits,
                primary_span.clone(),
            )?,
            maneuver_path: orders.maneuver_paths.ordinal(gate.maneuver_path),
            transition_index: gate.transition_index,
            stop_line: orders.stop_lines.ordinal(gate.stop_line),
            signal_control: match gate.signal_control {
                MirSignalControl::Group { signal_group, .. } => {
                    LirSignalControl::Group(orders.signal_groups.ordinal(signal_group))
                }
                MirSignalControl::None => LirSignalControl::None,
            },
            static_route_occurrences: TableRange::empty(),
        });
    }

    let mut waiting_zones = Vec::with_capacity(mir.waiting_zones.len());
    for mir_key in orders
        .waiting_zones
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let waiting = &mir.waiting_zones[mir_key.index()];
        let identity_start = identity_fields.len();
        for (tag, value) in [
            (
                FieldTag::AuthoringNamespaceId,
                mir.modules[waiting.module.index()]
                    .authoring_namespace_id
                    .as_bytes(),
            ),
            (
                FieldTag::ManeuverPathStableId,
                mir.maneuver_paths[waiting.maneuver_path.index()]
                    .stable_id
                    .as_untyped()
                    .as_bytes(),
            ),
            (FieldTag::WaitingZoneKey, waiting.stable_key.as_bytes()),
        ] {
            push_identity_field(
                &mut identity_fields,
                &mut identity_field_bytes,
                tag,
                value,
                &unit.limits,
                primary_span.clone(),
            )?;
        }
        waiting_zones.push(LirWaitingZone {
            ordinal: orders.waiting_zones.ordinal(mir_key),
            stable_id: waiting.stable_id,
            identity_fields: relation_range(
                identity_start,
                identity_fields.len(),
                &unit.limits,
                primary_span.clone(),
            )?,
            maneuver_path: orders.maneuver_paths.ordinal(waiting.maneuver_path),
            entry_gate: orders.maneuver_gates.ordinal(waiting.entry_gate),
            release_gate: orders.maneuver_gates.ordinal(waiting.release_gate),
            max_occupancy: waiting.max_occupancy,
            static_route_occurrences: TableRange::empty(),
        });
    }

    let mut signal_groups = Vec::with_capacity(mir.signal_groups.len());
    let mut signal_group_maneuver_gates = Vec::with_capacity(mir.signal_group_maneuver_gates.len());
    for mir_key in orders
        .signal_groups
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let group = &mir.signal_groups[mir_key.index()];
        let identity_range = push_lir_identity(
            &mut identity_fields,
            &mut identity_field_bytes,
            FieldTag::SignalGroupKey,
            &mir.modules[group.module.index()].authoring_namespace_id,
            &group.stable_key,
            None,
            &unit.limits,
            primary_span.clone(),
        )?;
        let gate_start = signal_group_maneuver_gates.len();
        signal_group_maneuver_gates.extend(
            mir.signal_group_maneuver_gates[group.maneuver_gates.as_usize_range()]
                .iter()
                .map(|member| orders.maneuver_gates.ordinal(member.maneuver_gate)),
        );
        signal_group_maneuver_gates[gate_start..].sort_unstable();
        signal_groups.push(LirSignalGroup {
            ordinal: orders.signal_groups.ordinal(mir_key),
            stable_id: group.stable_id,
            identity_fields: identity_range,
            controller: orders.signal_controllers.ordinal(group.controller),
            maneuver_gates: relation_range(
                gate_start,
                signal_group_maneuver_gates.len(),
                &unit.limits,
                primary_span.clone(),
            )?,
        });
    }

    let mut signal_controllers = Vec::with_capacity(mir.signal_controllers.len());
    let mut signal_controller_groups = Vec::with_capacity(mir.signal_controller_groups.len());
    let mut signal_controller_group_mir_rows: Vec<ArenaKey<MirSignalControllerGroup>> =
        Vec::with_capacity(mir.signal_controller_groups.len());
    let mut signal_controller_phases = Vec::with_capacity(mir.signal_phases.len());
    for mir_key in orders
        .signal_controllers
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let controller = &mir.signal_controllers[mir_key.index()];
        let identity_range = push_lir_identity(
            &mut identity_fields,
            &mut identity_field_bytes,
            FieldTag::SignalControllerKey,
            &mir.modules[controller.module.index()].authoring_namespace_id,
            &controller.stable_key,
            None,
            &unit.limits,
            primary_span.clone(),
        )?;
        let group_start = signal_controller_groups.len();
        let permutation_start = signal_controller_group_mir_rows.len();
        signal_controller_group_mir_rows.extend(controller.signal_groups.as_usize_range().map(
            |index| {
                ArenaKey::from_raw(
                    u32::try_from(index).expect("LIR precheck proved every MIR key fits u32"),
                )
            },
        ));
        signal_controller_group_mir_rows[permutation_start..].sort_unstable_by_key(|mir_row| {
            let member = &mir.signal_controller_groups[mir_row.index()];
            (
                orders.signal_groups.ordinal(member.signal_group),
                mir_row.raw(),
            )
        });
        // 集合语义只排序这一份 MIR 行地址；语义目标和来源随后都借用此排列。
        signal_controller_groups.extend(
            signal_controller_group_mir_rows[permutation_start..]
                .iter()
                .map(|mir_row| {
                    let member = &mir.signal_controller_groups[mir_row.index()];
                    orders.signal_groups.ordinal(member.signal_group)
                }),
        );
        debug_assert_eq!(group_start, permutation_start);
        let phase_start = signal_controller_phases.len();
        for phase_index in controller.phases.as_usize_range() {
            signal_controller_phases.push(orders.signal_phases.ordinal(
                MirSignalPhaseKey::from_raw(
                    u32::try_from(phase_index).expect("MIR range prevalidated as u32"),
                ),
            ));
        }
        signal_controllers.push(LirSignalController {
            ordinal: orders.signal_controllers.ordinal(mir_key),
            stable_id: controller.stable_id,
            identity_fields: identity_range,
            offset_ms: controller.offset_ms,
            cycle_duration_ms: controller.cycle_duration_ms,
            signal_groups: relation_range(
                group_start,
                signal_controller_groups.len(),
                &unit.limits,
                primary_span.clone(),
            )?,
            // 相位顺序就是固定时制程序顺序，不能按 ordinal 再排序。
            phases: relation_range(
                phase_start,
                signal_controller_phases.len(),
                &unit.limits,
                primary_span.clone(),
            )?,
        });
    }

    let mut signal_phases = Vec::with_capacity(mir.signal_phases.len());
    let mut signal_phase_states = Vec::with_capacity(mir.signal_phase_states.len());
    let mut signal_phase_state_mir_rows: Vec<ArenaKey<MirSignalPhaseState>> =
        Vec::with_capacity(mir.signal_phase_states.len());
    for mir_key in orders
        .signal_phases
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let phase = &mir.signal_phases[mir_key.index()];
        let identity_start = identity_fields.len();
        for (tag, value) in [
            (
                FieldTag::AuthoringNamespaceId,
                mir.modules[phase.module.index()]
                    .authoring_namespace_id
                    .as_bytes(),
            ),
            (
                FieldTag::SignalControllerStableId,
                mir.signal_controllers[phase.controller.index()]
                    .stable_id
                    .as_untyped()
                    .as_bytes(),
            ),
            (FieldTag::PhaseKey, phase.stable_key.as_bytes()),
        ] {
            push_identity_field(
                &mut identity_fields,
                &mut identity_field_bytes,
                tag,
                value,
                &unit.limits,
                primary_span.clone(),
            )?;
        }
        let state_start = signal_phase_states.len();
        let permutation_start = signal_phase_state_mir_rows.len();
        signal_phase_state_mir_rows.extend(phase.states.as_usize_range().map(|index| {
            ArenaKey::from_raw(
                u32::try_from(index).expect("LIR precheck proved every MIR key fits u32"),
            )
        }));
        signal_phase_state_mir_rows[permutation_start..].sort_unstable_by_key(|mir_row| {
            let state = &mir.signal_phase_states[mir_row.index()];
            (
                orders.signal_groups.ordinal(state.signal_group),
                mir_row.raw(),
            )
        });
        // 相位状态与控制器组表共享 LIR signal-group ordinal 轴。
        signal_phase_states.extend(signal_phase_state_mir_rows[permutation_start..].iter().map(
            |mir_row| {
                let state = &mir.signal_phase_states[mir_row.index()];
                LirSignalPhaseState {
                    signal_group: orders.signal_groups.ordinal(state.signal_group),
                    aspect: state.aspect,
                }
            },
        ));
        debug_assert_eq!(state_start, permutation_start);
        signal_phases.push(LirSignalPhase {
            ordinal: orders.signal_phases.ordinal(mir_key),
            stable_id: phase.stable_id,
            identity_fields: relation_range(
                identity_start,
                identity_fields.len(),
                &unit.limits,
                primary_span.clone(),
            )?,
            controller: orders.signal_controllers.ordinal(phase.controller),
            duration_ms: phase.duration_ms,
            states: relation_range(
                state_start,
                signal_phase_states.len(),
                &unit.limits,
                primary_span.clone(),
            )?,
        });
    }

    let mut parking_areas = Vec::with_capacity(mir.parking_areas.len());
    let mut parking_area_spaces = Vec::with_capacity(mir.parking_area_spaces.len());
    for mir_key in orders
        .parking_areas
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let area = &mir.parking_areas[mir_key.index()];
        let identity_range = push_lir_identity(
            &mut identity_fields,
            &mut identity_field_bytes,
            FieldTag::ParkingAreaKey,
            &mir.modules[area.module.index()].authoring_namespace_id,
            &area.stable_key,
            None,
            &unit.limits,
            primary_span.clone(),
        )?;
        let member_start = parking_area_spaces.len();
        parking_area_spaces.extend(
            mir.parking_area_spaces[area.parking_spaces.as_usize_range()]
                .iter()
                .map(|member| orders.parking_spaces.ordinal(member.parking_space)),
        );
        parking_area_spaces[member_start..].sort_unstable();
        parking_areas.push(LirParkingArea {
            ordinal: orders.parking_areas.ordinal(mir_key),
            stable_id: area.stable_id,
            identity_fields: identity_range,
            parking_spaces: relation_range(
                member_start,
                parking_area_spaces.len(),
                &unit.limits,
                primary_span.clone(),
            )?,
        });
    }

    let mut parking_spaces = Vec::with_capacity(mir.parking_spaces.len());
    for mir_key in orders
        .parking_spaces
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let space = &mir.parking_spaces[mir_key.index()];
        let identity_range = push_lir_identity(
            &mut identity_fields,
            &mut identity_field_bytes,
            FieldTag::ParkingSpaceKey,
            &mir.modules[space.module.index()].authoring_namespace_id,
            &space.stable_key,
            None,
            &unit.limits,
            primary_span.clone(),
        )?;
        parking_spaces.push(LirParkingSpace {
            ordinal: orders.parking_spaces.ordinal(mir_key),
            stable_id: space.stable_id,
            identity_fields: identity_range,
            parking_area: space
                .parking_area
                .map(|area| orders.parking_areas.ordinal(area)),
            entry: LirParkingLaneAnchor {
                lane_edge: orders.lane_edges.ordinal(space.entry.lane_edge),
                progress_meters: space.entry.progress_meters,
            },
            exit: LirParkingLaneAnchor {
                lane_edge: orders.lane_edges.ordinal(space.exit.lane_edge),
                progress_meters: space.exit.progress_meters,
            },
            geometry: LirParkingSpaceGeometry {
                lateral_offset_meters: space.geometry.lateral_offset_meters,
                heading_offset_radians: space.geometry.heading_offset_radians,
                length_meters: space.geometry.length_meters,
                width_meters: space.geometry.width_meters,
            },
        });
    }

    let mut participant_classes = Vec::with_capacity(mir.participant_classes.len());
    for mir_key in orders
        .participant_classes
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let participant_class = &mir.participant_classes[mir_key.index()];
        let identity_range = push_lir_identity(
            &mut identity_fields,
            &mut identity_field_bytes,
            FieldTag::ParticipantClassKey,
            &mir.modules[participant_class.module.index()].authoring_namespace_id,
            &participant_class.stable_key,
            None,
            &unit.limits,
            primary_span.clone(),
        )?;
        participant_classes.push(LirParticipantClass {
            ordinal: orders.participant_classes.ordinal(mir_key),
            stable_id: participant_class.stable_id,
            identity_fields: identity_range,
            parent: participant_class
                .parent
                .map(|parent| orders.participant_classes.ordinal(parent)),
            depth: participant_class.depth,
            subtree_enter: participant_class.subtree_enter,
            subtree_exit: participant_class.subtree_exit,
        });
    }

    let mut vehicle_profiles = Vec::with_capacity(mir.vehicle_profiles.len());
    for mir_key in orders
        .vehicle_profiles
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let profile = &mir.vehicle_profiles[mir_key.index()];
        let identity_range = push_lir_identity(
            &mut identity_fields,
            &mut identity_field_bytes,
            FieldTag::VehicleProfileKey,
            &mir.modules[profile.module.index()].authoring_namespace_id,
            &profile.stable_key,
            None,
            &unit.limits,
            primary_span.clone(),
        )?;
        vehicle_profiles.push(LirVehicleProfile {
            ordinal: orders.vehicle_profiles.ordinal(mir_key),
            stable_id: profile.stable_id,
            identity_fields: identity_range,
            participant_class: orders
                .participant_classes
                .ordinal(profile.participant_class),
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
        });
    }

    let mut canonical_frames = Vec::with_capacity(mir.canonical_frames.len());
    for mir_key in orders
        .canonical_frames
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let frame = &mir.canonical_frames[mir_key.index()];
        let identity_range = push_lir_identity(
            &mut identity_fields,
            &mut identity_field_bytes,
            FieldTag::CanonicalFrameKey,
            &mir.modules[frame.module.index()].authoring_namespace_id,
            &frame.stable_key,
            None,
            &unit.limits,
            primary_span.clone(),
        )?;
        canonical_frames.push(LirCanonicalFrame {
            ordinal: orders.canonical_frames.ordinal(mir_key),
            stable_id: frame.stable_id,
            identity_fields: identity_range,
        });
    }

    // HIR 已证明“空间存在时每条 LaneEdge 恰好一条几何”。冻结阶段只按最终
    // LaneEdgeOrdinal 重排，并保持每条中心线内部的点/线段顺序。
    let mut mir_edge_to_geometry = vec![None; mir.lane_edges.len()];
    for (index, geometry) in mir.lane_edge_geometries.iter().enumerate() {
        debug_assert!(mir_edge_to_geometry[geometry.lane_edge.index()].is_none());
        mir_edge_to_geometry[geometry.lane_edge.index()] = Some(index);
    }
    let mut lane_edge_geometries = Vec::with_capacity(mir.lane_edge_geometries.len());
    let mut canonical_points = Vec::with_capacity(mir.canonical_points.len());
    let mut spatial_segments = Vec::with_capacity(mir.spatial_segments.len());
    for mir_edge in orders.lane_edges.stage_keys_in_lir_order().iter().copied() {
        let Some(geometry_index) = mir_edge_to_geometry[mir_edge.index()] else {
            debug_assert!(mir.lane_edge_geometries.is_empty());
            continue;
        };
        let geometry = &mir.lane_edge_geometries[geometry_index];
        let point_start = canonical_points.len();
        canonical_points.extend(
            mir.canonical_points[geometry.points.as_usize_range()]
                .iter()
                .map(|point| LirCanonicalPoint3F32 {
                    x: point.x,
                    y: point.y,
                    z: point.z,
                }),
        );
        let segment_start = spatial_segments.len();
        spatial_segments.extend(
            mir.spatial_segments[geometry.segments.as_usize_range()]
                .iter()
                .map(|segment| LirSpatialSegment {
                    length_meters: segment.length_meters,
                    cumulative_end_meters: segment.cumulative_end_meters,
                    tangent: segment.tangent,
                    up: segment.up,
                }),
        );
        lane_edge_geometries.push(LirLaneEdgeGeometry {
            canonical_frame: orders.canonical_frames.ordinal(geometry.canonical_frame),
            points: TableRange::try_from_usize(
                point_start,
                canonical_points.len().saturating_sub(point_start),
            )
            .map_err(|overflow| table_overflow(overflow, &unit.limits, primary_span.clone()))?,
            segments: TableRange::try_from_usize(
                segment_start,
                spatial_segments.len().saturating_sub(segment_start),
            )
            .map_err(|overflow| table_overflow(overflow, &unit.limits, primary_span.clone()))?,
            arc_length_meters: geometry.arc_length_meters,
        });
    }

    // FacilityBand 不进入可通行图，但其可视几何必须和实体使用同一规范顺序。每个
    // 稀疏几何行携带 band ordinal，view 通过有序表查找，避免复制第二份范围索引。
    let mut mir_band_to_geometry = vec![None; mir.facility_bands.len()];
    for (index, geometry) in mir.facility_band_geometries.iter().enumerate() {
        debug_assert!(mir_band_to_geometry[geometry.facility_band.index()].is_none());
        mir_band_to_geometry[geometry.facility_band.index()] = Some(index);
    }
    let mut facility_band_geometries = Vec::with_capacity(mir.facility_band_geometries.len());
    for mir_band in orders
        .facility_bands
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let Some(geometry_index) = mir_band_to_geometry[mir_band.index()] else {
            continue;
        };
        let geometry = &mir.facility_band_geometries[geometry_index];
        let point_start = canonical_points.len();
        canonical_points.extend(
            mir.canonical_points[geometry.points.as_usize_range()]
                .iter()
                .map(|point| LirCanonicalPoint3F32 {
                    x: point.x,
                    y: point.y,
                    z: point.z,
                }),
        );
        let facility_band = orders.facility_bands.ordinal(mir_band);
        facility_band_geometries.push(LirFacilityBandGeometry {
            facility_band,
            canonical_frame: orders.canonical_frames.ordinal(geometry.canonical_frame),
            points: TableRange::try_from_usize(
                point_start,
                canonical_points.len().saturating_sub(point_start),
            )
            .map_err(|overflow| table_overflow(overflow, &unit.limits, primary_span.clone()))?,
        });
    }

    let mut access_rules = Vec::with_capacity(mir.access_rules.len());
    let mut access_rule_participant_classes =
        Vec::with_capacity(mir.access_rule_participant_classes.len());
    for mir_key in orders
        .access_rules
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let rule = &mir.access_rules[mir_key.index()];
        let identity_range = push_lir_identity(
            &mut identity_fields,
            &mut identity_field_bytes,
            FieldTag::AccessRuleKey,
            &mir.modules[rule.module.index()].authoring_namespace_id,
            &rule.stable_key,
            None,
            &unit.limits,
            primary_span.clone(),
        )?;
        let class_start = access_rule_participant_classes.len();
        access_rule_participant_classes.extend(
            mir.access_rule_participant_classes[rule.participant_classes.as_usize_range()]
                .iter()
                .map(|selector| {
                    orders
                        .participant_classes
                        .ordinal(selector.participant_class)
                }),
        );
        access_rule_participant_classes[class_start..].sort_unstable();
        let target = match rule.target {
            MirAccessTarget::LaneEdge(target) => {
                LirAccessTarget::LaneEdge(orders.lane_edges.ordinal(target))
            }
            MirAccessTarget::LaneGroup(target) => {
                LirAccessTarget::LaneGroup(orders.lane_groups.ordinal(target))
            }
            MirAccessTarget::RoadSection(target) => {
                LirAccessTarget::RoadSection(orders.road_sections.ordinal(target))
            }
            MirAccessTarget::ManeuverPath(target) => {
                LirAccessTarget::ManeuverPath(orders.maneuver_paths.ordinal(target))
            }
        };
        access_rules.push(LirAccessRule {
            ordinal: orders.access_rules.ordinal(mir_key),
            stable_id: rule.stable_id,
            identity_fields: identity_range,
            target,
            effect: rule.effect,
            participant_classes: relation_range(
                class_start,
                access_rule_participant_classes.len(),
                &unit.limits,
                primary_span.clone(),
            )?,
            regulation: rule
                .regulation
                .as_ref()
                .map(|regulation| LirAccessRegulation {
                    jurisdiction: regulation.jurisdiction.as_ref().into(),
                    version: regulation.version.as_ref().into(),
                    source: regulation.source.as_deref().map(Into::into),
                }),
            priority: rule.priority,
        });
    }

    let mut static_routes = Vec::with_capacity(mir.static_routes.len());
    let mut static_route_edges = Vec::with_capacity(mir.static_route_edges.len());
    let mut static_route_transitions = Vec::with_capacity(mir.static_route_transitions.len());
    let mut maneuver_occurrences = Vec::with_capacity(mir.maneuver_occurrences.len());
    let mut gate_occurrences = Vec::with_capacity(mir.gate_occurrences.len());
    let mut waiting_zone_occurrences = Vec::with_capacity(mir.waiting_zone_occurrences.len());
    let mut edge_reverse = Vec::with_capacity(mir.static_route_edges.len());
    let mut path_reverse = Vec::with_capacity(mir.maneuver_occurrences.len());
    let mut gate_reverse = Vec::with_capacity(mir.gate_occurrences.len());
    let mut waiting_reverse = Vec::with_capacity(mir.waiting_zone_occurrences.len());

    for mir_key in orders
        .static_routes
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let route = &mir.static_routes[mir_key.index()];
        let route_ordinal = orders.static_routes.ordinal(mir_key);
        let identity_range = push_lir_identity(
            &mut identity_fields,
            &mut identity_field_bytes,
            FieldTag::RouteKey,
            &mir.modules[route.module.index()].authoring_namespace_id,
            &route.stable_key,
            None,
            &unit.limits,
            primary_span.clone(),
        )?;

        let edge_start = static_route_edges.len();
        for (local_index, edge) in mir.static_route_edges[route.edges.as_usize_range()]
            .iter()
            .enumerate()
        {
            let ordinal = orders.lane_edges.ordinal(edge.target);
            static_route_edges.push(ordinal);
            edge_reverse.push((
                ordinal.raw(),
                LirRouteOccurrenceRef {
                    static_route: route_ordinal,
                    occurrence_index: u32::try_from(local_index).unwrap_or(u32::MAX),
                },
            ));
        }
        let transition_start = static_route_transitions.len();
        static_route_transitions.extend(
            mir.static_route_transitions[route.transitions.as_usize_range()]
                .iter()
                .map(|transition| LirStaticRouteTransition {
                    maneuver_gate: transition
                        .maneuver_gate
                        .map(|key| orders.maneuver_gates.ordinal(key)),
                }),
        );

        let gate_start = gate_occurrences.len();
        for (local_index, occurrence) in mir.gate_occurrences
            [route.gate_occurrences.as_usize_range()]
        .iter()
        .enumerate()
        {
            let ordinal = orders.maneuver_gates.ordinal(occurrence.maneuver_gate);
            gate_occurrences.push(LirGateOccurrence {
                maneuver_gate: ordinal,
                maneuver_occurrence_index: occurrence.maneuver_occurrence_index,
                from_route_edge_index: occurrence.from_route_edge_index,
                next_gate_occurrence_index: occurrence.next_gate_occurrence_index,
                next_boundary_route_edge_index: occurrence.next_boundary_route_edge_index,
                waiting_zone_occurrence_index: occurrence.waiting_zone_occurrence_index,
            });
            gate_reverse.push((
                ordinal.raw(),
                LirRouteOccurrenceRef {
                    static_route: route_ordinal,
                    occurrence_index: u32::try_from(local_index).unwrap_or(u32::MAX),
                },
            ));
        }
        let waiting_start = waiting_zone_occurrences.len();
        for (local_index, occurrence) in mir.waiting_zone_occurrences
            [route.waiting_zone_occurrences.as_usize_range()]
        .iter()
        .enumerate()
        {
            let ordinal = orders.waiting_zones.ordinal(occurrence.waiting_zone);
            waiting_zone_occurrences.push(LirWaitingZoneOccurrence {
                waiting_zone: ordinal,
                maneuver_occurrence_index: occurrence.maneuver_occurrence_index,
                entry_gate_occurrence_index: occurrence.entry_gate_occurrence_index,
                release_gate_occurrence_index: occurrence.release_gate_occurrence_index,
                entry_route_edge_index: occurrence.entry_route_edge_index,
                release_route_edge_index: occurrence.release_route_edge_index,
            });
            waiting_reverse.push((
                ordinal.raw(),
                LirRouteOccurrenceRef {
                    static_route: route_ordinal,
                    occurrence_index: u32::try_from(local_index).unwrap_or(u32::MAX),
                },
            ));
        }

        let maneuver_start = maneuver_occurrences.len();
        for (local_index, occurrence) in mir.maneuver_occurrences
            [route.maneuver_occurrences.as_usize_range()]
        .iter()
        .enumerate()
        {
            let ordinal = orders.maneuver_paths.ordinal(occurrence.maneuver_path);
            let gate_local_start = occurrence
                .gate_occurrences
                .start()
                .saturating_sub(route.gate_occurrences.start());
            let waiting_local_start = occurrence
                .waiting_zone_occurrences
                .start()
                .saturating_sub(route.waiting_zone_occurrences.start());
            maneuver_occurrences.push(LirManeuverOccurrence {
                maneuver_path: ordinal,
                entry_route_edge_index: occurrence.entry_route_edge_index,
                exit_route_edge_index: occurrence.exit_route_edge_index,
                gate_occurrences: TableRange::try_from_usize(
                    gate_start + gate_local_start as usize,
                    occurrence.gate_occurrences.len() as usize,
                )
                .map_err(|overflow| table_overflow(overflow, &unit.limits, primary_span.clone()))?,
                waiting_zone_occurrences: TableRange::try_from_usize(
                    waiting_start + waiting_local_start as usize,
                    occurrence.waiting_zone_occurrences.len() as usize,
                )
                .map_err(|overflow| table_overflow(overflow, &unit.limits, primary_span.clone()))?,
            });
            path_reverse.push((
                ordinal.raw(),
                LirRouteOccurrenceRef {
                    static_route: route_ordinal,
                    occurrence_index: u32::try_from(local_index).unwrap_or(u32::MAX),
                },
            ));
        }

        static_routes.push(LirStaticRoute {
            ordinal: route_ordinal,
            stable_id: route.stable_id,
            identity_fields: identity_range,
            edges: relation_range(
                edge_start,
                static_route_edges.len(),
                &unit.limits,
                primary_span.clone(),
            )?,
            transitions: relation_range(
                transition_start,
                static_route_transitions.len(),
                &unit.limits,
                primary_span.clone(),
            )?,
            maneuver_occurrences: relation_range(
                maneuver_start,
                maneuver_occurrences.len(),
                &unit.limits,
                primary_span.clone(),
            )?,
            gate_occurrences: relation_range(
                gate_start,
                gate_occurrences.len(),
                &unit.limits,
                primary_span.clone(),
            )?,
            waiting_zone_occurrences: relation_range(
                waiting_start,
                waiting_zone_occurrences.len(),
                &unit.limits,
                primary_span.clone(),
            )?,
        });
    }

    let lane_edge_route_occurrences = freeze_reverse_occurrences(
        edge_reverse,
        &mut lane_edges,
        |entity, range| entity.static_route_occurrences = range,
        &unit.limits,
        primary_span.clone(),
    )?;
    let maneuver_path_route_occurrences = freeze_reverse_occurrences(
        path_reverse,
        &mut maneuver_paths,
        |entity, range| entity.static_route_occurrences = range,
        &unit.limits,
        primary_span.clone(),
    )?;
    let maneuver_gate_route_occurrences = freeze_reverse_occurrences(
        gate_reverse,
        &mut maneuver_gates,
        |entity, range| entity.static_route_occurrences = range,
        &unit.limits,
        primary_span.clone(),
    )?;
    let waiting_zone_route_occurrences = freeze_reverse_occurrences(
        waiting_reverse,
        &mut waiting_zones,
        |entity, range| entity.static_route_occurrences = range,
        &unit.limits,
        primary_span.clone(),
    )?;

    debug_assert_eq!(lane_edges.len(), edge_capacity);
    debug_assert_eq!(successors.len(), successor_capacity);
    debug_assert_eq!(identity_fields.len(), identity_field_capacity);
    debug_assert_eq!(identity_field_bytes.len(), identity_byte_capacity);
    let geometry_profiles = mir.geometry_profiles;
    let semantic_digest = semantic_digest(
        geometry_profiles,
        &lane_edges,
        &successors,
        &road_corridors,
        &corridor_elements,
        &road_sections,
        &road_section_lanes,
        &authoring_lanes,
        &authoring_lane_edges,
        &lane_groups,
        &lane_group_members,
        &facility_bands,
        &junctions,
        &junction_movements,
        &movements,
        &movement_maneuver_paths,
        &maneuver_paths,
        &maneuver_path_edges,
        &junction_internal_edges,
        &stop_lines,
        &maneuver_gates,
        &waiting_zones,
        &maneuver_path_gates,
        &maneuver_path_waiting_zones,
        &stop_line_maneuver_gates,
        &signal_groups,
        &signal_controllers,
        &signal_controller_groups,
        &signal_controller_phases,
        &signal_phases,
        &signal_phase_states,
        &signal_group_maneuver_gates,
        &parking_areas,
        &parking_spaces,
        &parking_area_spaces,
        &participant_classes,
        &vehicle_profiles,
        &canonical_frames,
        &lane_edge_geometries,
        &facility_band_geometries,
        &canonical_points,
        &spatial_segments,
        &access_rules,
        &access_rule_participant_classes,
        &static_routes,
        &static_route_edges,
        &static_route_transitions,
        &maneuver_occurrences,
        &gate_occurrences,
        &waiting_zone_occurrences,
        &lane_edge_route_occurrences,
        &maneuver_path_route_occurrences,
        &maneuver_gate_route_occurrences,
        &waiting_zone_route_occurrences,
        &identity_fields,
        &identity_field_bytes,
    );
    Ok(LirFreezeOutput {
        lir: LirUnit {
            geometry_profiles,
            lane_edges: lane_edges.into_boxed_slice(),
            lane_edge_successors: successors.into_boxed_slice(),
            road_corridors: road_corridors.into_boxed_slice(),
            corridor_elements: corridor_elements.into_boxed_slice(),
            road_sections: road_sections.into_boxed_slice(),
            road_section_lanes: road_section_lanes.into_boxed_slice(),
            authoring_lanes: authoring_lanes.into_boxed_slice(),
            authoring_lane_edges: authoring_lane_edges.into_boxed_slice(),
            lane_groups: lane_groups.into_boxed_slice(),
            lane_group_members: lane_group_members.into_boxed_slice(),
            facility_bands: facility_bands.into_boxed_slice(),
            junctions: junctions.into_boxed_slice(),
            junction_movements: junction_movements.into_boxed_slice(),
            movements: movements.into_boxed_slice(),
            movement_maneuver_paths: movement_maneuver_paths.into_boxed_slice(),
            maneuver_paths: maneuver_paths.into_boxed_slice(),
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
            signal_controller_phases: signal_controller_phases.into_boxed_slice(),
            signal_phases: signal_phases.into_boxed_slice(),
            signal_phase_states: signal_phase_states.into_boxed_slice(),
            signal_group_maneuver_gates: signal_group_maneuver_gates.into_boxed_slice(),
            parking_areas: parking_areas.into_boxed_slice(),
            parking_spaces: parking_spaces.into_boxed_slice(),
            parking_area_spaces: parking_area_spaces.into_boxed_slice(),
            participant_classes: participant_classes.into_boxed_slice(),
            vehicle_profiles: vehicle_profiles.into_boxed_slice(),
            canonical_frames: canonical_frames.into_boxed_slice(),
            lane_edge_geometries: lane_edge_geometries.into_boxed_slice(),
            facility_band_geometries: facility_band_geometries.into_boxed_slice(),
            canonical_points: canonical_points.into_boxed_slice(),
            spatial_segments: spatial_segments.into_boxed_slice(),
            access_rules: access_rules.into_boxed_slice(),
            access_rule_participant_classes: access_rule_participant_classes.into_boxed_slice(),
            static_routes: static_routes.into_boxed_slice(),
            static_route_edges: static_route_edges.into_boxed_slice(),
            static_route_transitions: static_route_transitions.into_boxed_slice(),
            maneuver_occurrences: maneuver_occurrences.into_boxed_slice(),
            gate_occurrences: gate_occurrences.into_boxed_slice(),
            waiting_zone_occurrences: waiting_zone_occurrences.into_boxed_slice(),
            lane_edge_route_occurrences: lane_edge_route_occurrences.into_boxed_slice(),
            maneuver_path_route_occurrences: maneuver_path_route_occurrences.into_boxed_slice(),
            maneuver_gate_route_occurrences: maneuver_gate_route_occurrences.into_boxed_slice(),
            waiting_zone_route_occurrences: waiting_zone_route_occurrences.into_boxed_slice(),
            identity_fields: identity_fields.into_boxed_slice(),
            identity_field_bytes: identity_field_bytes.into_boxed_slice(),
            semantic_digest,
            lir_record_count: plan.lir_record_count,
            output_bytes: plan.output_bytes,
            controlled_live_bytes: plan.output_owned_bytes,
            peak_controlled_live_bytes: plan.controlled_live_bytes,
        },
        lane_edges: orders.lane_edges,
        lane_edge_successors: OwnerLocalPermutation::from_rows(lane_edge_successor_mir_rows),
        road_corridors: orders.road_corridors,
        road_sections: orders.road_sections,
        authoring_lanes: orders.authoring_lanes,
        lane_groups: orders.lane_groups,
        facility_bands: orders.facility_bands,
        junctions: orders.junctions,
        movements: orders.movements,
        maneuver_paths: orders.maneuver_paths,
        canonical_mir_internal_edge_order: canonical_mir_internal_edge_order.into_boxed_slice(),
        stop_lines: orders.stop_lines,
        maneuver_gates: orders.maneuver_gates,
        waiting_zones: orders.waiting_zones,
        signal_groups: orders.signal_groups,
        signal_controllers: orders.signal_controllers,
        signal_phases: orders.signal_phases,
        signal_controller_groups: OwnerLocalPermutation::from_rows(
            signal_controller_group_mir_rows,
        ),
        signal_phase_states: OwnerLocalPermutation::from_rows(signal_phase_state_mir_rows),
        parking_areas: orders.parking_areas,
        parking_spaces: orders.parking_spaces,
        participant_classes: orders.participant_classes,
        vehicle_profiles: orders.vehicle_profiles,
        canonical_frames: orders.canonical_frames,
        access_rules: orders.access_rules,
        static_routes: orders.static_routes,
    })
}

/// 比较两个 `LaneEdge` 的完整 Identity v1 前像，而不物化拼接缓冲区。
fn compare_identity_v1(mir: &MirUnit, left: MirLaneEdgeKey, right: MirLaneEdgeKey) -> Ordering {
    let left_edge = &mir.lane_edges[left.index()];
    let right_edge = &mir.lane_edges[right.index()];
    let left_namespace = mir.modules[left_edge.module.index()]
        .authoring_namespace_id
        .as_bytes();
    let right_namespace = mir.modules[right_edge.module.index()]
        .authoring_namespace_id
        .as_bytes();

    // magic、encoding version、kind、field count 和字段标签对同种实体完全相同；每个
    // 变长字段在前像中都是 `u32_le(length) || value`，因此只比较这些差异片段即可得到
    // 与完整编码逐字节比较完全相同的顺序。
    compare_lane_edge_identity_fields(
        left_namespace,
        left_edge.stable_key.as_bytes(),
        right_namespace,
        right_edge.stable_key.as_bytes(),
    )
}

fn compare_lane_edge_identity_fields(
    left_namespace: &[u8],
    left_key: &[u8],
    right_namespace: &[u8],
    right_key: &[u8],
) -> Ordering {
    compare_length_prefixed(left_namespace, right_namespace)
        .then_with(|| compare_length_prefixed(left_key, right_key))
}

fn compare_length_prefixed(left: &[u8], right: &[u8]) -> Ordering {
    let left_length = u32::try_from(left.len())
        .expect("source validation proved Identity v1 field length fits u32");
    let right_length = u32::try_from(right.len())
        .expect("source validation proved Identity v1 field length fits u32");
    left_length
        .to_le_bytes()
        .cmp(&right_length.to_le_bytes())
        .then_with(|| left.cmp(right))
}

fn compare_identity_parts(
    left_namespace: &str,
    left_key: &str,
    left_parent: Option<&[u8; 16]>,
    right_namespace: &str,
    right_key: &str,
    right_parent: Option<&[u8; 16]>,
) -> Ordering {
    compare_length_prefixed(left_namespace.as_bytes(), right_namespace.as_bytes())
        .then_with(|| compare_length_prefixed(left_key.as_bytes(), right_key.as_bytes()))
        .then_with(|| left_parent.cmp(&right_parent))
}

fn dense_mir_keys<K>(count: usize) -> Vec<ArenaKey<K>> {
    (0..count)
        .map(|index| {
            ArenaKey::from_raw(
                u32::try_from(index).expect("LIR precheck proved MIR table length fits u32"),
            )
        })
        .collect()
}

fn ordinal_mapping<K, O: Copy, E>(
    count: usize,
    canonical_order: &[ArenaKey<K>],
    make_ordinal: fn(usize) -> Result<O, E>,
    limits: &crate::CompileLimits,
    primary_span: Option<SourceLocation>,
) -> Result<Vec<O>, DiagnosticBundle> {
    let first = make_ordinal(0).map_err(|_| ordinal_overflow(limits, primary_span.clone()))?;
    let mut mapping = vec![first; count];
    for (index, mir_key) in canonical_order.iter().copied().enumerate() {
        mapping[mir_key.index()] =
            make_ordinal(index).map_err(|_| ordinal_overflow(limits, primary_span.clone()))?;
    }
    Ok(mapping)
}

fn relation_range<T>(
    start: usize,
    end: usize,
    limits: &crate::CompileLimits,
    primary_span: Option<SourceLocation>,
) -> Result<TableRange<T>, DiagnosticBundle> {
    TableRange::try_from_usize(start, end.saturating_sub(start))
        .map_err(|overflow| table_overflow(overflow, limits, primary_span))
}

#[allow(clippy::too_many_arguments)]
fn push_lir_identity(
    fields: &mut Vec<LirIdentityField>,
    bytes: &mut Vec<u8>,
    key_tag: FieldTag,
    namespace: &str,
    stable_key: &str,
    parent: Option<(FieldTag, &[u8; 16])>,
    limits: &crate::CompileLimits,
    primary_span: Option<SourceLocation>,
) -> Result<TableRange<LirIdentityField>, DiagnosticBundle> {
    let start = fields.len();
    push_identity_field(
        fields,
        bytes,
        FieldTag::AuthoringNamespaceId,
        namespace.as_bytes(),
        limits,
        primary_span.clone(),
    )?;
    push_identity_field(
        fields,
        bytes,
        key_tag,
        stable_key.as_bytes(),
        limits,
        primary_span.clone(),
    )?;
    if let Some((tag, value)) = parent {
        push_identity_field(fields, bytes, tag, value, limits, primary_span.clone())?;
    }
    relation_range(start, fields.len(), limits, primary_span)
}

fn identity_field_byte_count(mir: &MirUnit) -> u64 {
    let mut total = 0_u64;
    let add = |total: &mut u64, module_index: usize, stable_key: &str, has_parent: bool| {
        *total = total
            .saturating_add(
                u64::try_from(mir.modules[module_index].authoring_namespace_id.len())
                    .unwrap_or(u64::MAX),
            )
            .saturating_add(u64::try_from(stable_key.len()).unwrap_or(u64::MAX))
            .saturating_add(if has_parent { 16 } else { 0 });
    };
    for edge in &mir.lane_edges {
        add(&mut total, edge.module.index(), &edge.stable_key, false);
    }
    for corridor in &mir.road_corridors {
        add(
            &mut total,
            corridor.module.index(),
            &corridor.stable_key,
            false,
        );
    }
    for section in &mir.road_sections {
        add(
            &mut total,
            section.module.index(),
            &section.stable_key,
            true,
        );
    }
    for lane in &mir.authoring_lanes {
        add(&mut total, lane.module.index(), &lane.stable_key, true);
    }
    for group in &mir.lane_groups {
        add(&mut total, group.module.index(), &group.stable_key, true);
    }
    for band in &mir.facility_bands {
        add(&mut total, band.module.index(), &band.stable_key, true);
    }
    for junction in &mir.junctions {
        add(
            &mut total,
            junction.module.index(),
            &junction.stable_key,
            false,
        );
    }
    for movement in &mir.movements {
        add(
            &mut total,
            movement.module.index(),
            &movement.stable_key,
            true,
        );
        total = total
            .saturating_add(
                u64::try_from(movement.directed_entry_approach_key.len()).unwrap_or(u64::MAX),
            )
            .saturating_add(
                u64::try_from(movement.directed_exit_approach_key.len()).unwrap_or(u64::MAX),
            );
    }
    for path in &mir.maneuver_paths {
        // ManeuverPath identity has three fixed StableId128 parents: Movement、entry 与 exit。
        add(&mut total, path.module.index(), &path.stable_key, false);
        total = total.saturating_add(16_u64.saturating_mul(3));
    }
    for stop_line in &mir.stop_lines {
        add(
            &mut total,
            stop_line.module.index(),
            &stop_line.stable_key,
            false,
        );
    }
    for gate in &mir.maneuver_gates {
        add(&mut total, gate.module.index(), &gate.stable_key, true);
    }
    for waiting in &mir.waiting_zones {
        add(
            &mut total,
            waiting.module.index(),
            &waiting.stable_key,
            true,
        );
    }
    for group in &mir.signal_groups {
        add(&mut total, group.module.index(), &group.stable_key, false);
    }
    for controller in &mir.signal_controllers {
        add(
            &mut total,
            controller.module.index(),
            &controller.stable_key,
            false,
        );
    }
    for phase in &mir.signal_phases {
        add(&mut total, phase.module.index(), &phase.stable_key, true);
    }
    for area in &mir.parking_areas {
        add(&mut total, area.module.index(), &area.stable_key, false);
    }
    for space in &mir.parking_spaces {
        add(&mut total, space.module.index(), &space.stable_key, false);
    }
    for participant_class in &mir.participant_classes {
        add(
            &mut total,
            participant_class.module.index(),
            &participant_class.stable_key,
            false,
        );
    }
    for profile in &mir.vehicle_profiles {
        add(
            &mut total,
            profile.module.index(),
            &profile.stable_key,
            false,
        );
    }
    for frame in &mir.canonical_frames {
        add(&mut total, frame.module.index(), &frame.stable_key, false);
    }
    for rule in &mir.access_rules {
        add(&mut total, rule.module.index(), &rule.stable_key, false);
    }
    for route in &mir.static_routes {
        add(&mut total, route.module.index(), &route.stable_key, false);
    }
    total
}

fn mapping_pair_bytes<K, O>(order_len: usize, mapping_len: usize) -> u64 {
    requested_bytes::<K>(u64::try_from(order_len).unwrap_or(u64::MAX)).saturating_add(
        requested_bytes::<O>(u64::try_from(mapping_len).unwrap_or(u64::MAX)),
    )
}

fn freeze_reverse_occurrences<T>(
    mut entries: Vec<(u32, LirRouteOccurrenceRef)>,
    entities: &mut [T],
    mut set_range: impl FnMut(&mut T, TableRange<LirRouteOccurrenceRef>),
    limits: &crate::CompileLimits,
    primary_span: Option<SourceLocation>,
) -> Result<Vec<LirRouteOccurrenceRef>, DiagnosticBundle> {
    entries.sort_unstable_by_key(|(target, occurrence)| {
        (
            *target,
            occurrence.static_route.raw(),
            occurrence.occurrence_index,
        )
    });
    let mut output = Vec::with_capacity(entries.len());
    let mut cursor = 0_usize;
    for (target_index, entity) in entities.iter_mut().enumerate() {
        let start = output.len();
        while cursor < entries.len() && entries[cursor].0 as usize == target_index {
            output.push(entries[cursor].1);
            cursor += 1;
        }
        set_range(
            entity,
            relation_range(start, output.len(), limits, primary_span.clone())?,
        );
    }
    debug_assert_eq!(cursor, entries.len());
    Ok(output)
}

fn push_identity_field(
    fields: &mut Vec<LirIdentityField>,
    bytes: &mut Vec<u8>,
    tag: FieldTag,
    value: &[u8],
    limits: &crate::CompileLimits,
    primary_span: Option<SourceLocation>,
) -> Result<(), DiagnosticBundle> {
    let start = bytes.len();
    bytes.extend_from_slice(value);
    let value_bytes = TableRange::try_from_usize(start, value.len())
        .map_err(|_| output_overflow(limits, primary_span))?;
    fields.push(LirIdentityField { tag, value_bytes });
    Ok(())
}

// 摘要输入显式列出所有规范表，避免把尚未构造完成的 LirUnit 借给哈希器；参数数量随
// 已支持的封闭实体表增长，但保持每张表是否进入确定性摘要一目了然。
#[allow(clippy::too_many_arguments)]
fn semantic_digest(
    geometry_profiles: Option<GeometryCompilationProfiles>,
    edges: &[LirLaneEdge],
    successors: &[LaneEdgeOrdinal],
    corridors: &[LirRoadCorridor],
    corridor_elements: &[LirCorridorElement],
    sections: &[LirRoadSection],
    section_lanes: &[AuthoringLaneOrdinal],
    lanes: &[LirAuthoringLane],
    lane_edges: &[LaneEdgeOrdinal],
    groups: &[LirLaneGroup],
    group_members: &[AuthoringLaneOrdinal],
    bands: &[LirFacilityBand],
    junctions: &[LirJunction],
    junction_movements: &[MovementOrdinal],
    movements: &[LirMovement],
    movement_maneuver_paths: &[ManeuverPathOrdinal],
    maneuver_paths: &[LirManeuverPath],
    maneuver_path_edges: &[LaneEdgeOrdinal],
    junction_internal_edges: &[LirJunctionInternalEdge],
    stop_lines: &[LirStopLine],
    maneuver_gates: &[LirManeuverGate],
    waiting_zones: &[LirWaitingZone],
    maneuver_path_gates: &[ManeuverGateOrdinal],
    maneuver_path_waiting_zones: &[WaitingZoneOrdinal],
    stop_line_maneuver_gates: &[ManeuverGateOrdinal],
    signal_groups: &[LirSignalGroup],
    signal_controllers: &[LirSignalController],
    signal_controller_groups: &[SignalGroupOrdinal],
    signal_controller_phases: &[SignalPhaseOrdinal],
    signal_phases: &[LirSignalPhase],
    signal_phase_states: &[LirSignalPhaseState],
    signal_group_maneuver_gates: &[ManeuverGateOrdinal],
    parking_areas: &[LirParkingArea],
    parking_spaces: &[LirParkingSpace],
    parking_area_spaces: &[ParkingSpaceOrdinal],
    participant_classes: &[LirParticipantClass],
    vehicle_profiles: &[LirVehicleProfile],
    canonical_frames: &[LirCanonicalFrame],
    lane_edge_geometries: &[LirLaneEdgeGeometry],
    facility_band_geometries: &[LirFacilityBandGeometry],
    canonical_points: &[LirCanonicalPoint3F32],
    spatial_segments: &[LirSpatialSegment],
    access_rules: &[LirAccessRule],
    access_rule_participant_classes: &[ParticipantClassOrdinal],
    static_routes: &[LirStaticRoute],
    static_route_edges: &[LaneEdgeOrdinal],
    static_route_transitions: &[LirStaticRouteTransition],
    maneuver_occurrences: &[LirManeuverOccurrence],
    gate_occurrences: &[LirGateOccurrence],
    waiting_zone_occurrences: &[LirWaitingZoneOccurrence],
    lane_edge_route_occurrences: &[LirRouteOccurrenceRef],
    maneuver_path_route_occurrences: &[LirRouteOccurrenceRef],
    maneuver_gate_route_occurrences: &[LirRouteOccurrenceRef],
    waiting_zone_route_occurrences: &[LirRouteOccurrenceRef],
    identity_fields: &[LirIdentityField],
    identity_field_bytes: &[u8],
) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(LIR_SEMANTIC_DIGEST_DOMAIN);
    let profile_codes = geometry_profiles.map_or([0, 0], |profiles| {
        [profiles.accuracy as u8, profiles.direction as u8]
    });
    hasher.update(&profile_codes);
    hash_u32(&mut hasher, EntityKind::LaneEdge.code().into());
    hash_u32(
        &mut hasher,
        u32::try_from(edges.len()).expect("LIR edge count was validated before allocation"),
    );
    for edge in edges {
        hash_u32(&mut hasher, edge.ordinal.raw());
        hasher.update(edge.stable_id.as_untyped().as_bytes());
        hash_identity(
            &mut hasher,
            edge.identity_fields,
            identity_fields,
            identity_field_bytes,
        );
        hasher.update(&edge.length_meters.to_bits().to_le_bytes());
        hasher.update(&edge.speed_limit_meters_per_second.to_bits().to_le_bytes());
        hash_u32(&mut hasher, edge.successors.len());
        for successor in &successors[edge.successors.as_usize_range()] {
            hash_u32(&mut hasher, successor.raw());
        }
    }
    hash_u32(&mut hasher, EntityKind::RoadCorridor.code().into());
    hash_u32(&mut hasher, corridors.len().try_into().unwrap_or(u32::MAX));
    for corridor in corridors {
        hash_u32(&mut hasher, corridor.ordinal.raw());
        hasher.update(corridor.stable_id.as_untyped().as_bytes());
        hash_identity(
            &mut hasher,
            corridor.identity_fields,
            identity_fields,
            identity_field_bytes,
        );
        hash_u32(&mut hasher, corridor.reference_section.raw());
        hash_u32(&mut hasher, corridor.elements.len());
        for element in &corridor_elements[corridor.elements.as_usize_range()] {
            match element {
                LirCorridorElement::RoadSection(ordinal) => {
                    hasher.update(&EntityKind::RoadSection.code().to_le_bytes());
                    hash_u32(&mut hasher, ordinal.raw());
                }
                LirCorridorElement::FacilityBand(ordinal) => {
                    hasher.update(&EntityKind::FacilityBand.code().to_le_bytes());
                    hash_u32(&mut hasher, ordinal.raw());
                }
            }
        }
    }
    hash_u32(&mut hasher, EntityKind::RoadSection.code().into());
    hash_u32(&mut hasher, sections.len().try_into().unwrap_or(u32::MAX));
    for section in sections {
        hash_u32(&mut hasher, section.ordinal.raw());
        hasher.update(section.stable_id.as_untyped().as_bytes());
        hash_identity(
            &mut hasher,
            section.identity_fields,
            identity_fields,
            identity_field_bytes,
        );
        hash_u32(&mut hasher, section.road_corridor.raw());
        hash_bytes(&mut hasher, section.kind_id.as_bytes());
        hash_u32(&mut hasher, section.lanes.len());
        for lane in &section_lanes[section.lanes.as_usize_range()] {
            hash_u32(&mut hasher, lane.raw());
        }
    }
    hash_u32(&mut hasher, EntityKind::AuthoringLane.code().into());
    hash_u32(&mut hasher, lanes.len().try_into().unwrap_or(u32::MAX));
    for lane in lanes {
        hash_u32(&mut hasher, lane.ordinal.raw());
        hasher.update(lane.stable_id.as_untyped().as_bytes());
        hash_identity(
            &mut hasher,
            lane.identity_fields,
            identity_fields,
            identity_field_bytes,
        );
        hash_u32(&mut hasher, lane.road_section.raw());
        hash_u32(&mut hasher, lane.edge_chain.len());
        for edge in &lane_edges[lane.edge_chain.as_usize_range()] {
            hash_u32(&mut hasher, edge.raw());
        }
        match lane.lane_group {
            Some(group) => {
                hasher.update(&[1]);
                hash_u32(&mut hasher, group.raw());
            }
            None => {
                hasher.update(&[0]);
            }
        }
    }
    hash_u32(&mut hasher, EntityKind::LaneGroup.code().into());
    hash_u32(&mut hasher, groups.len().try_into().unwrap_or(u32::MAX));
    for group in groups {
        hash_u32(&mut hasher, group.ordinal.raw());
        hasher.update(group.stable_id.as_untyped().as_bytes());
        hash_identity(
            &mut hasher,
            group.identity_fields,
            identity_fields,
            identity_field_bytes,
        );
        hash_u32(&mut hasher, group.road_section.raw());
        hash_u32(&mut hasher, group.members.len());
        for lane in &group_members[group.members.as_usize_range()] {
            hash_u32(&mut hasher, lane.raw());
        }
    }
    hash_u32(&mut hasher, EntityKind::FacilityBand.code().into());
    hash_u32(&mut hasher, bands.len().try_into().unwrap_or(u32::MAX));
    for band in bands {
        hash_u32(&mut hasher, band.ordinal.raw());
        hasher.update(band.stable_id.as_untyped().as_bytes());
        hash_identity(
            &mut hasher,
            band.identity_fields,
            identity_fields,
            identity_field_bytes,
        );
        hash_u32(&mut hasher, band.road_corridor.raw());
        hash_bytes(&mut hasher, band.kind_id.as_bytes());
    }
    hash_u32(&mut hasher, EntityKind::Junction.code().into());
    hash_u32(&mut hasher, junctions.len().try_into().unwrap_or(u32::MAX));
    for junction in junctions {
        hash_u32(&mut hasher, junction.ordinal.raw());
        hasher.update(junction.stable_id.as_untyped().as_bytes());
        hash_identity(
            &mut hasher,
            junction.identity_fields,
            identity_fields,
            identity_field_bytes,
        );
        hash_u32(&mut hasher, junction.movements.len());
        for movement in &junction_movements[junction.movements.as_usize_range()] {
            hash_u32(&mut hasher, movement.raw());
        }
    }
    hash_u32(&mut hasher, EntityKind::Movement.code().into());
    hash_u32(&mut hasher, movements.len().try_into().unwrap_or(u32::MAX));
    for movement in movements {
        hash_u32(&mut hasher, movement.ordinal.raw());
        hasher.update(movement.stable_id.as_untyped().as_bytes());
        hash_identity(
            &mut hasher,
            movement.identity_fields,
            identity_fields,
            identity_field_bytes,
        );
        hash_u32(&mut hasher, movement.junction.raw());
        hash_bytes(&mut hasher, movement.directed_entry_approach_key.as_bytes());
        hash_bytes(&mut hasher, movement.directed_exit_approach_key.as_bytes());
        hash_u32(&mut hasher, movement.maneuver_paths.len());
        for path in &movement_maneuver_paths[movement.maneuver_paths.as_usize_range()] {
            hash_u32(&mut hasher, path.raw());
        }
    }
    hash_u32(&mut hasher, EntityKind::ManeuverPath.code().into());
    hash_u32(
        &mut hasher,
        maneuver_paths.len().try_into().unwrap_or(u32::MAX),
    );
    for path in maneuver_paths {
        hash_u32(&mut hasher, path.ordinal.raw());
        hasher.update(path.stable_id.as_untyped().as_bytes());
        hash_identity(
            &mut hasher,
            path.identity_fields,
            identity_fields,
            identity_field_bytes,
        );
        hash_u32(&mut hasher, path.movement.raw());
        hash_u32(&mut hasher, path.edges.len());
        for edge in &maneuver_path_edges[path.edges.as_usize_range()] {
            hash_u32(&mut hasher, edge.raw());
        }
        hash_u32(&mut hasher, path.maneuver_gates.len());
        for gate in &maneuver_path_gates[path.maneuver_gates.as_usize_range()] {
            hash_u32(&mut hasher, gate.raw());
        }
        hash_u32(&mut hasher, path.waiting_zones.len());
        for waiting in &maneuver_path_waiting_zones[path.waiting_zones.as_usize_range()] {
            hash_u32(&mut hasher, waiting.raw());
        }
    }
    // internal-role 表是由全路径闭包验证出的规范关系，必须参与摘要；否则角色冲突或路径
    // 内部结构变化可能被一个只观察实体身份的摘要漏掉。
    hash_u32(
        &mut hasher,
        junction_internal_edges.len().try_into().unwrap_or(u32::MAX),
    );
    for relation in junction_internal_edges {
        hash_u32(&mut hasher, relation.edge.raw());
        hash_u32(&mut hasher, relation.junction.raw());
    }
    hash_u32(&mut hasher, EntityKind::StopLine.code().into());
    hash_u32(&mut hasher, stop_lines.len().try_into().unwrap_or(u32::MAX));
    for stop_line in stop_lines {
        hash_u32(&mut hasher, stop_line.ordinal.raw());
        hasher.update(stop_line.stable_id.as_untyped().as_bytes());
        hash_identity(
            &mut hasher,
            stop_line.identity_fields,
            identity_fields,
            identity_field_bytes,
        );
        hash_u32(&mut hasher, stop_line.lane_edge.raw());
        hash_u32(&mut hasher, stop_line.maneuver_gates.len());
        for gate in &stop_line_maneuver_gates[stop_line.maneuver_gates.as_usize_range()] {
            hash_u32(&mut hasher, gate.raw());
        }
    }
    hash_u32(&mut hasher, EntityKind::ManeuverGate.code().into());
    hash_u32(
        &mut hasher,
        maneuver_gates.len().try_into().unwrap_or(u32::MAX),
    );
    for gate in maneuver_gates {
        hash_u32(&mut hasher, gate.ordinal.raw());
        hasher.update(gate.stable_id.as_untyped().as_bytes());
        hash_identity(
            &mut hasher,
            gate.identity_fields,
            identity_fields,
            identity_field_bytes,
        );
        hash_u32(&mut hasher, gate.maneuver_path.raw());
        hash_u32(&mut hasher, gate.transition_index);
        hash_u32(&mut hasher, gate.stop_line.raw());
        match gate.signal_control {
            LirSignalControl::Group(group) => {
                hasher.update(&[1]);
                hash_u32(&mut hasher, group.raw());
            }
            LirSignalControl::None => {
                hasher.update(&[0]);
            }
        }
    }
    hash_u32(&mut hasher, EntityKind::WaitingZone.code().into());
    hash_u32(
        &mut hasher,
        waiting_zones.len().try_into().unwrap_or(u32::MAX),
    );
    for waiting in waiting_zones {
        hash_u32(&mut hasher, waiting.ordinal.raw());
        hasher.update(waiting.stable_id.as_untyped().as_bytes());
        hash_identity(
            &mut hasher,
            waiting.identity_fields,
            identity_fields,
            identity_field_bytes,
        );
        hash_u32(&mut hasher, waiting.maneuver_path.raw());
        hash_u32(&mut hasher, waiting.entry_gate.raw());
        hash_u32(&mut hasher, waiting.release_gate.raw());
        hash_u32(&mut hasher, waiting.max_occupancy);
    }
    hash_u32(&mut hasher, EntityKind::SignalGroup.code().into());
    hash_u32(
        &mut hasher,
        signal_groups.len().try_into().unwrap_or(u32::MAX),
    );
    for group in signal_groups {
        hash_u32(&mut hasher, group.ordinal.raw());
        hasher.update(group.stable_id.as_untyped().as_bytes());
        hash_identity(
            &mut hasher,
            group.identity_fields,
            identity_fields,
            identity_field_bytes,
        );
        hash_u32(&mut hasher, group.controller.raw());
        hash_u32(&mut hasher, group.maneuver_gates.len());
        for gate in &signal_group_maneuver_gates[group.maneuver_gates.as_usize_range()] {
            hash_u32(&mut hasher, gate.raw());
        }
    }
    hash_u32(&mut hasher, EntityKind::SignalController.code().into());
    hash_u32(
        &mut hasher,
        signal_controllers.len().try_into().unwrap_or(u32::MAX),
    );
    for controller in signal_controllers {
        hash_u32(&mut hasher, controller.ordinal.raw());
        hasher.update(controller.stable_id.as_untyped().as_bytes());
        hash_identity(
            &mut hasher,
            controller.identity_fields,
            identity_fields,
            identity_field_bytes,
        );
        hasher.update(&controller.offset_ms.to_le_bytes());
        hasher.update(&controller.cycle_duration_ms.to_le_bytes());
        hash_u32(&mut hasher, controller.signal_groups.len());
        for group in &signal_controller_groups[controller.signal_groups.as_usize_range()] {
            hash_u32(&mut hasher, group.raw());
        }
        hash_u32(&mut hasher, controller.phases.len());
        for phase in &signal_controller_phases[controller.phases.as_usize_range()] {
            hash_u32(&mut hasher, phase.raw());
        }
    }
    hash_u32(&mut hasher, EntityKind::SignalPhase.code().into());
    hash_u32(
        &mut hasher,
        signal_phases.len().try_into().unwrap_or(u32::MAX),
    );
    for phase in signal_phases {
        hash_u32(&mut hasher, phase.ordinal.raw());
        hasher.update(phase.stable_id.as_untyped().as_bytes());
        hash_identity(
            &mut hasher,
            phase.identity_fields,
            identity_fields,
            identity_field_bytes,
        );
        hash_u32(&mut hasher, phase.controller.raw());
        hasher.update(&phase.duration_ms.to_le_bytes());
        hash_u32(&mut hasher, phase.states.len());
        for state in &signal_phase_states[phase.states.as_usize_range()] {
            hash_u32(&mut hasher, state.signal_group.raw());
            hasher.update(&[signal_aspect_digest_code(state.aspect)]);
        }
    }
    hash_u32(&mut hasher, EntityKind::ParkingArea.code().into());
    hash_u32(
        &mut hasher,
        parking_areas.len().try_into().unwrap_or(u32::MAX),
    );
    for area in parking_areas {
        hash_u32(&mut hasher, area.ordinal.raw());
        hasher.update(area.stable_id.as_untyped().as_bytes());
        hash_identity(
            &mut hasher,
            area.identity_fields,
            identity_fields,
            identity_field_bytes,
        );
        hash_u32(&mut hasher, area.parking_spaces.len());
        for space in &parking_area_spaces[area.parking_spaces.as_usize_range()] {
            hash_u32(&mut hasher, space.raw());
        }
    }
    hash_u32(&mut hasher, EntityKind::ParkingSpace.code().into());
    hash_u32(
        &mut hasher,
        parking_spaces.len().try_into().unwrap_or(u32::MAX),
    );
    for space in parking_spaces {
        hash_u32(&mut hasher, space.ordinal.raw());
        hasher.update(space.stable_id.as_untyped().as_bytes());
        hash_identity(
            &mut hasher,
            space.identity_fields,
            identity_fields,
            identity_field_bytes,
        );
        hash_optional_ordinal(&mut hasher, space.parking_area.map(ParkingAreaOrdinal::raw));
        for anchor in [space.entry, space.exit] {
            hash_u32(&mut hasher, anchor.lane_edge.raw());
            hasher.update(&anchor.progress_meters.to_bits().to_le_bytes());
        }
        for value in [
            space.geometry.lateral_offset_meters,
            space.geometry.heading_offset_radians,
            space.geometry.length_meters,
            space.geometry.width_meters,
        ] {
            hasher.update(&value.to_bits().to_le_bytes());
        }
    }
    hash_u32(&mut hasher, EntityKind::ParticipantClass.code().into());
    hash_u32(
        &mut hasher,
        participant_classes.len().try_into().unwrap_or(u32::MAX),
    );
    for participant_class in participant_classes {
        hash_u32(&mut hasher, participant_class.ordinal.raw());
        hasher.update(participant_class.stable_id.as_untyped().as_bytes());
        hash_identity(
            &mut hasher,
            participant_class.identity_fields,
            identity_fields,
            identity_field_bytes,
        );
        hash_optional_ordinal(
            &mut hasher,
            participant_class.parent.map(ParticipantClassOrdinal::raw),
        );
        hash_u32(&mut hasher, participant_class.depth);
        hash_u32(&mut hasher, participant_class.subtree_enter);
        hash_u32(&mut hasher, participant_class.subtree_exit);
    }
    hash_u32(&mut hasher, EntityKind::VehicleProfile.code().into());
    hash_u32(
        &mut hasher,
        vehicle_profiles.len().try_into().unwrap_or(u32::MAX),
    );
    for profile in vehicle_profiles {
        hash_u32(&mut hasher, profile.ordinal.raw());
        hasher.update(profile.stable_id.as_untyped().as_bytes());
        hash_identity(
            &mut hasher,
            profile.identity_fields,
            identity_fields,
            identity_field_bytes,
        );
        hash_u32(&mut hasher, profile.participant_class.raw());
        for value in [
            profile.length_meters,
            profile.desired_speed_meters_per_second,
            profile.min_gap_meters,
            profile.time_headway_seconds,
            profile.max_acceleration_meters_per_second_squared,
            profile.comfortable_deceleration_meters_per_second_squared,
            profile.emergency_deceleration_meters_per_second_squared,
        ] {
            hasher.update(&value.to_bits().to_le_bytes());
        }
    }
    hash_u32(&mut hasher, EntityKind::CanonicalFrame.code().into());
    hash_u32(
        &mut hasher,
        canonical_frames.len().try_into().unwrap_or(u32::MAX),
    );
    for frame in canonical_frames {
        hash_u32(&mut hasher, frame.ordinal.raw());
        hasher.update(frame.stable_id.as_untyped().as_bytes());
        hash_identity(
            &mut hasher,
            frame.identity_fields,
            identity_fields,
            identity_field_bytes,
        );
    }
    hash_u32(
        &mut hasher,
        lane_edge_geometries.len().try_into().unwrap_or(u32::MAX),
    );
    for geometry in lane_edge_geometries {
        hash_u32(&mut hasher, geometry.canonical_frame.raw());
        hasher.update(&geometry.arc_length_meters.to_bits().to_le_bytes());
        hash_u32(&mut hasher, geometry.points.len());
        for point in &canonical_points[geometry.points.as_usize_range()] {
            for component in [point.x, point.y, point.z] {
                hasher.update(&component.to_bits().to_le_bytes());
            }
        }
        hash_u32(&mut hasher, geometry.segments.len());
        for segment in &spatial_segments[geometry.segments.as_usize_range()] {
            for value in [segment.length_meters, segment.cumulative_end_meters]
                .into_iter()
                .chain(segment.tangent)
                .chain(segment.up)
            {
                hasher.update(&value.to_bits().to_le_bytes());
            }
        }
    }
    hash_u32(
        &mut hasher,
        facility_band_geometries
            .len()
            .try_into()
            .unwrap_or(u32::MAX),
    );
    for geometry in facility_band_geometries {
        hash_u32(&mut hasher, geometry.facility_band.raw());
        hash_u32(&mut hasher, geometry.canonical_frame.raw());
        hash_u32(&mut hasher, geometry.points.len());
        for point in &canonical_points[geometry.points.as_usize_range()] {
            for component in [point.x, point.y, point.z] {
                hasher.update(&component.to_bits().to_le_bytes());
            }
        }
    }
    hash_u32(&mut hasher, EntityKind::AccessRule.code().into());
    hash_u32(
        &mut hasher,
        access_rules.len().try_into().unwrap_or(u32::MAX),
    );
    for rule in access_rules {
        hash_u32(&mut hasher, rule.ordinal.raw());
        hasher.update(rule.stable_id.as_untyped().as_bytes());
        hash_identity(
            &mut hasher,
            rule.identity_fields,
            identity_fields,
            identity_field_bytes,
        );
        match rule.target {
            LirAccessTarget::LaneEdge(target) => {
                hasher.update(&EntityKind::LaneEdge.code().to_le_bytes());
                hash_u32(&mut hasher, target.raw());
            }
            LirAccessTarget::LaneGroup(target) => {
                hasher.update(&EntityKind::LaneGroup.code().to_le_bytes());
                hash_u32(&mut hasher, target.raw());
            }
            LirAccessTarget::RoadSection(target) => {
                hasher.update(&EntityKind::RoadSection.code().to_le_bytes());
                hash_u32(&mut hasher, target.raw());
            }
            LirAccessTarget::ManeuverPath(target) => {
                hasher.update(&EntityKind::ManeuverPath.code().to_le_bytes());
                hash_u32(&mut hasher, target.raw());
            }
        }
        hasher.update(&[access_effect_digest_code(rule.effect)]);
        hash_u32(&mut hasher, rule.participant_classes.len());
        for participant_class in
            &access_rule_participant_classes[rule.participant_classes.as_usize_range()]
        {
            hash_u32(&mut hasher, participant_class.raw());
        }
        match &rule.regulation {
            Some(regulation) => {
                hasher.update(&[1]);
                hash_bytes(&mut hasher, regulation.jurisdiction.as_bytes());
                hash_bytes(&mut hasher, regulation.version.as_bytes());
                match &regulation.source {
                    Some(source) => {
                        hasher.update(&[1]);
                        hash_bytes(&mut hasher, source.as_bytes());
                    }
                    None => {
                        hasher.update(&[0]);
                    }
                }
            }
            None => {
                hasher.update(&[0]);
            }
        }
        hasher.update(&rule.priority.to_le_bytes());
    }
    hash_u32(&mut hasher, EntityKind::StaticRoute.code().into());
    hash_u32(
        &mut hasher,
        static_routes.len().try_into().unwrap_or(u32::MAX),
    );
    for route in static_routes {
        hash_u32(&mut hasher, route.ordinal.raw());
        hasher.update(route.stable_id.as_untyped().as_bytes());
        hash_identity(
            &mut hasher,
            route.identity_fields,
            identity_fields,
            identity_field_bytes,
        );
        hash_u32(&mut hasher, route.edges.len());
        for edge in &static_route_edges[route.edges.as_usize_range()] {
            hash_u32(&mut hasher, edge.raw());
        }
        hash_u32(&mut hasher, route.transitions.len());
        for transition in &static_route_transitions[route.transitions.as_usize_range()] {
            hash_optional_ordinal(
                &mut hasher,
                transition.maneuver_gate.map(ManeuverGateOrdinal::raw),
            );
        }
        hash_u32(&mut hasher, route.maneuver_occurrences.len());
        for occurrence in &maneuver_occurrences[route.maneuver_occurrences.as_usize_range()] {
            hash_u32(&mut hasher, occurrence.maneuver_path.raw());
            hash_u32(&mut hasher, occurrence.entry_route_edge_index);
            hash_u32(&mut hasher, occurrence.exit_route_edge_index);
            hash_u32(&mut hasher, occurrence.gate_occurrences.start());
            hash_u32(&mut hasher, occurrence.gate_occurrences.len());
            hash_u32(&mut hasher, occurrence.waiting_zone_occurrences.start());
            hash_u32(&mut hasher, occurrence.waiting_zone_occurrences.len());
        }
        hash_u32(&mut hasher, route.gate_occurrences.len());
        for occurrence in &gate_occurrences[route.gate_occurrences.as_usize_range()] {
            hash_u32(&mut hasher, occurrence.maneuver_gate.raw());
            hash_u32(&mut hasher, occurrence.maneuver_occurrence_index);
            hash_u32(&mut hasher, occurrence.from_route_edge_index);
            hash_optional_ordinal(&mut hasher, occurrence.next_gate_occurrence_index);
            hash_u32(&mut hasher, occurrence.next_boundary_route_edge_index);
            hash_optional_ordinal(&mut hasher, occurrence.waiting_zone_occurrence_index);
        }
        hash_u32(&mut hasher, route.waiting_zone_occurrences.len());
        for occurrence in &waiting_zone_occurrences[route.waiting_zone_occurrences.as_usize_range()]
        {
            hash_u32(&mut hasher, occurrence.waiting_zone.raw());
            hash_u32(&mut hasher, occurrence.maneuver_occurrence_index);
            hash_u32(&mut hasher, occurrence.entry_gate_occurrence_index);
            hash_u32(&mut hasher, occurrence.release_gate_occurrence_index);
            hash_u32(&mut hasher, occurrence.entry_route_edge_index);
            hash_u32(&mut hasher, occurrence.release_route_edge_index);
        }
    }
    // 反向 occurrence 表是 Canonical LIR 的可观察输出，不能只依赖正向路线表间接覆盖。
    // 同时哈希实体范围和连续表，确保范围切分或冻结顺序的回退也会改变语义摘要。
    hash_reverse_occurrences(
        &mut hasher,
        EntityKind::LaneEdge,
        edges
            .iter()
            .map(|entity| (entity.ordinal.raw(), entity.static_route_occurrences)),
        lane_edge_route_occurrences,
    );
    hash_reverse_occurrences(
        &mut hasher,
        EntityKind::ManeuverPath,
        maneuver_paths
            .iter()
            .map(|entity| (entity.ordinal.raw(), entity.static_route_occurrences)),
        maneuver_path_route_occurrences,
    );
    hash_reverse_occurrences(
        &mut hasher,
        EntityKind::ManeuverGate,
        maneuver_gates
            .iter()
            .map(|entity| (entity.ordinal.raw(), entity.static_route_occurrences)),
        maneuver_gate_route_occurrences,
    );
    hash_reverse_occurrences(
        &mut hasher,
        EntityKind::WaitingZone,
        waiting_zones
            .iter()
            .map(|entity| (entity.ordinal.raw(), entity.static_route_occurrences)),
        waiting_zone_route_occurrences,
    );
    *hasher.finalize().as_bytes()
}

fn hash_reverse_occurrences(
    hasher: &mut blake3::Hasher,
    entity_kind: EntityKind,
    entities: impl ExactSizeIterator<Item = (u32, TableRange<LirRouteOccurrenceRef>)>,
    occurrences: &[LirRouteOccurrenceRef],
) {
    hash_u32(hasher, entity_kind.code().into());
    hash_u32(hasher, entities.len().try_into().unwrap_or(u32::MAX));
    for (ordinal, range) in entities {
        hash_u32(hasher, ordinal);
        hash_u32(hasher, range.start());
        hash_u32(hasher, range.len());
    }
    hash_u32(hasher, occurrences.len().try_into().unwrap_or(u32::MAX));
    for occurrence in occurrences {
        hash_u32(hasher, occurrence.static_route.raw());
        hash_u32(hasher, occurrence.occurrence_index);
    }
}

#[allow(unreachable_patterns)]
fn signal_aspect_digest_code(aspect: SignalAspect) -> u8 {
    match aspect {
        SignalAspect::Red => 1,
        SignalAspect::Yellow => 2,
        SignalAspect::Green => 3,
        _ => unreachable!("compiler received an unsupported SignalAspect variant"),
    }
}

fn access_effect_digest_code(effect: AccessEffect) -> u8 {
    match effect {
        AccessEffect::Allow => 1,
        AccessEffect::Deny => 2,
        _ => unreachable!("compiler received an unsupported AccessEffect variant"),
    }
}

fn hash_optional_ordinal(hasher: &mut blake3::Hasher, value: Option<u32>) {
    match value {
        Some(value) => {
            hasher.update(&[1]);
            hash_u32(hasher, value);
        }
        None => {
            hasher.update(&[0]);
        }
    }
}

fn hash_identity(
    hasher: &mut blake3::Hasher,
    range: TableRange<LirIdentityField>,
    identity_fields: &[LirIdentityField],
    identity_field_bytes: &[u8],
) {
    hash_u32(hasher, range.len());
    for field in &identity_fields[range.as_usize_range()] {
        hasher.update(&field.tag.code().to_le_bytes());
        hash_u32(hasher, field.value_bytes.len());
        hasher.update(&identity_field_bytes[field.value_bytes.as_usize_range()]);
    }
}

fn hash_bytes(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hash_u32(hasher, bytes.len().try_into().unwrap_or(u32::MAX));
    hasher.update(bytes);
}

fn hash_u32(hasher: &mut blake3::Hasher, value: u32) {
    hasher.update(&value.to_le_bytes());
}

fn requested_bytes<T>(count: u64) -> u64 {
    count.saturating_mul(u64::try_from(size_of::<T>()).unwrap_or(u64::MAX))
}

fn table_overflow(
    _: ArenaKeyOverflow,
    limits: &crate::CompileLimits,
    primary_span: Option<SourceLocation>,
) -> DiagnosticBundle {
    ordinal_overflow(limits, primary_span)
}

fn ordinal_overflow(
    limits: &crate::CompileLimits,
    primary_span: Option<SourceLocation>,
) -> DiagnosticBundle {
    DiagnosticBundle::single(Diagnostic::compile_limit_exceeded_at(
        CompileLimitDimension::LirRecordCount,
        limits.value(CompileLimitDimension::LirRecordCount),
        u64::from(u32::MAX) + 1,
        primary_span,
        None,
    ))
}

fn output_overflow(
    limits: &crate::CompileLimits,
    primary_span: Option<SourceLocation>,
) -> DiagnosticBundle {
    DiagnosticBundle::single(Diagnostic::compile_limit_exceeded_at(
        CompileLimitDimension::OutputBytes,
        limits.value(CompileLimitDimension::OutputBytes),
        u64::MAX,
        primary_span,
        None,
    ))
}
