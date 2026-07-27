//! Core runtime 的错误类型。

use crate::{
    EdgeHandle, ManeuverGateHandle, ParkingAnchorKind, ParkingBindingKind, ParkingCommandKind,
    ParkingSpaceHandle, RouteHandle, VehicleHandle, VehicleProfileHandle, VehicleStatus,
};

/// Core runtime 暴露给调用方的错误。
#[derive(Clone, Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CoreError {
    /// `CoreWorld` 的固定步长必须大于 0。
    #[error("`fixed_delta_time_ms` 必须大于 0，实际值为 {fixed_delta_time_ms}")]
    InvalidFixedDeltaTime { fixed_delta_time_ms: u64 },
    /// tick 输入的 delta 必须等于当前 world 的固定步长。
    #[error("tick delta 不匹配：期望 {expected_delta_time_ms} ms，实际 {actual_delta_time_ms} ms")]
    TickDeltaMismatch {
        expected_delta_time_ms: u64,
        actual_delta_time_ms: u64,
    },
    /// tick/time 累计发生整数溢出。
    #[error("tick/time 累计发生整数溢出")]
    TimeOverflow,
    /// speed 必须是 finite 且大于或等于 0。
    #[error("speed 无效：{speed}")]
    InvalidSpeed { speed: f64 },
    /// acceleration 必须是 finite 有符号数值。
    #[error("acceleration 无效：{acceleration}")]
    InvalidAcceleration { acceleration: f64 },
    /// edge progress 必须是 finite 且大于或等于 0。
    #[error("edge progress 无效：{edge_progress}")]
    InvalidEdgeProgress { edge_progress: f64 },
    /// lane edge length 必须是 finite 且大于 epsilon。
    #[error("lane edge length 无效：{edge_length}，必须是 finite 且大于 {min_exclusive}")]
    InvalidLaneEdgeLength {
        edge_length: f64,
        min_exclusive: f64,
    },
    /// 基础道路限速必须是 finite 且严格大于 0。
    #[error("speed limit 无效：{speed_limit}，必须是 finite 且严格大于 0 m/s")]
    InvalidSpeedLimit { speed_limit: f64 },
    /// external ID 必须满足当前 data format 的 ASCII token 规则。
    #[error("external ID 无效：field={field}, value=`{external_id}`，必须匹配 {pattern}")]
    InvalidExternalId {
        field: &'static str,
        external_id: String,
        pattern: &'static str,
    },
    /// Vehicle Profile 数值必须满足对应字段约束。
    #[error("Vehicle Profile `{profile_id}` 的 `{field}` 无效：{value}，{requirement}")]
    InvalidVehicleProfileValue {
        profile_id: String,
        field: &'static str,
        value: f64,
        requirement: &'static str,
    },
    /// emergency deceleration 必须大于或等于 comfortable deceleration。
    #[error(
        "Vehicle Profile `{profile_id}` 的制动参数无效：emergencyDeceleration={emergency_deceleration} 必须大于或等于 comfortableDeceleration={comfortable_deceleration}"
    )]
    InvalidVehicleProfileDecelerationOrder {
        profile_id: String,
        comfortable_deceleration: f64,
        emergency_deceleration: f64,
    },
    /// Vehicle Profile external ID 在 registry 内必须唯一。
    #[error("Vehicle Profile id 重复：{profile_id}")]
    DuplicateVehicleProfileId { profile_id: String },
    /// FacilityKind 必须是 SSOT seed 值或以 `x-` 前缀声明的自定义 kind。
    #[error("未知 FacilityKind：`{kind}`，必须是 seed kind 或以 `x-` 前缀声明自定义 kind")]
    UnknownFacilityKind { kind: String },
    /// ParticipantClass external ID 在 registry 内必须唯一。
    #[error("ParticipantClass id 重复：{class_id}")]
    DuplicateParticipantClassId { class_id: String },
    /// ParticipantClass 的 extendsId 必须引用已声明的 class。
    #[error("ParticipantClass `{class_id}` 引用了不存在的父类 `{extends_id}`")]
    UnknownParticipantClassExtends {
        class_id: String,
        extends_id: String,
    },
    /// ParticipantClass 继承链必须无环。
    #[error("ParticipantClass `{class_id}` 的继承链存在环")]
    ParticipantClassInheritanceCycle { class_id: String },
    /// Vehicle Profile 的 ParticipantClass handle 必须属于注册时使用的 ParticipantClassRegistry。
    #[error(
        "Vehicle Profile `{profile_id}` 的 ParticipantClass handle 越界：index={class_index}, class count={class_count}"
    )]
    VehicleProfileParticipantClassOutOfRange {
        profile_id: String,
        class_index: usize,
        class_count: usize,
    },
    /// rebind 时 Vehicle Profile 保留的 ParticipantClass external ID 必须解析到目标 registry。
    #[error(
        "Vehicle Profile `{profile_id}` 的 ParticipantClass `{class_id}` 在目标 ParticipantClassRegistry 中不存在"
    )]
    UnknownVehicleProfileParticipantClass {
        profile_id: String,
        class_id: String,
    },
    /// AccessRule 的 regulation provenance 字符串必须为 1 到 128 字符（与 schema 契约一致）。
    #[error("regulation.{field} 必须为 1 到 128 字符，实际为 {len} 字符")]
    InvalidAccessRegulationString { field: &'static str, len: usize },
    /// AccessRule 的 priority 必须是 i32 范围内的整数（与 schema 契约一致）。
    #[error("AccessRule 的 priority 必须是 i32 范围内的整数，实际为 {priority}")]
    InvalidAccessRulePriority { priority: String },
    /// FacilityKind token 长度必须为 1 到 128 字符（与 schema 的 `facilityKindId` 契约一致）。
    #[error("FacilityKind token `{kind}` 长度必须为 1 到 128 字符，实际为 {len} 字符")]
    FacilityKindTokenTooLong { kind: String, len: usize },
    /// FacilityBand external ID 在 registry 内必须唯一。
    #[error("FacilityBand id 重复：{band_id}")]
    DuplicateFacilityBandId { band_id: String },
    /// FacilityBand 的 kindId 必须是 non-traversable 类别。
    #[error("FacilityBand `{band_id}` 的 kindId `{kind_id}` 不是 non-traversable 类别")]
    FacilityBandKindNotNonTraversable { band_id: String, kind_id: String },
    /// RoadSection external ID 在 registry 内必须唯一。
    #[error("RoadSection id 重复：{section_id}")]
    DuplicateRoadSectionId { section_id: String },
    /// RoadSection 的 kindId 必须是 lane-bearing 类别。
    #[error("RoadSection `{section_id}` 的 kindId `{kind_id}` 不是 lane-bearing 类别")]
    RoadSectionKindNotLaneBearing { section_id: String, kind_id: String },
    /// LaneGroup external ID 在 registry 内必须唯一。
    #[error("LaneGroup id 重复：{group_id}")]
    DuplicateLaneGroupId { group_id: String },
    /// LaneGroup 必须引用已声明的 RoadSection。
    #[error("LaneGroup `{group_id}` 引用了不存在的 RoadSection `{section_id}`")]
    UnknownLaneGroupRoadSection {
        group_id: String,
        section_id: String,
    },
    /// 每个 RoadSection 至少拥有一条 lane。
    #[error("RoadSection `{section_id}` 没有 lane")]
    EmptyRoadSectionLanes { section_id: String },
    /// 每条 lane 的 edge 链必须非空。
    #[error("RoadSection `{section_id}` 的 lane {lane_index} 没有 edge")]
    EmptySectionLaneChain {
        section_id: String,
        lane_index: usize,
    },
    /// lane 引用的 edge 必须存在。
    #[error("RoadSection `{section_id}` 的 lane {lane_index} 引用了不存在的 LaneEdge `{edge_id}`")]
    UnknownSectionLaneEdge {
        section_id: String,
        lane_index: usize,
        edge_id: String,
    },
    /// lane 链内相邻 edge 必须存在 LaneGraph directed connection。
    #[error(
        "RoadSection `{section_id}` 的 lane {lane_index} transition {transition_index} 不连通：`{from_edge_id}` -> `{to_edge_id}`"
    )]
    DisconnectedSectionLane {
        section_id: String,
        lane_index: usize,
        transition_index: usize,
        from_edge_id: String,
        to_edge_id: String,
    },
    /// 同一 lane 链内不得重复引用同一 edge。
    #[error("RoadSection `{section_id}` 的 lane {lane_index} 重复引用 LaneEdge `{edge_id}`")]
    DuplicateSectionLaneEdge {
        section_id: String,
        lane_index: usize,
        edge_id: String,
    },
    /// 一条 LaneEdge 至多属于一条 lane、至多一个 RoadSection。
    #[error(
        "LaneEdge `{edge_id}` 被多条 lane 占用：first=`{first_section_id}` lane {first_lane_index}, duplicate=`{duplicate_section_id}` lane {duplicate_lane_index}"
    )]
    SectionLaneEdgeClaimConflict {
        edge_id: String,
        first_section_id: String,
        first_lane_index: usize,
        duplicate_section_id: String,
        duplicate_lane_index: usize,
    },
    /// lane 的 laneGroupId 必须引用已声明的 LaneGroup。
    #[error(
        "RoadSection `{section_id}` 的 lane {lane_index} 引用了不存在的 LaneGroup `{group_id}`"
    )]
    UnknownSectionLaneGroup {
        section_id: String,
        lane_index: usize,
        group_id: String,
    },
    /// lane 引用的 LaneGroup 必须属于该 lane 所在的 RoadSection。
    #[error(
        "RoadSection `{section_id}` 的 lane {lane_index} 引用的 LaneGroup `{group_id}` 属于 RoadSection `{group_section_id}`"
    )]
    SectionLaneGroupSectionMismatch {
        section_id: String,
        lane_index: usize,
        group_id: String,
        group_section_id: String,
    },
    /// 每个 LaneGroup 至少聚合一条 lane。
    #[error("LaneGroup `{group_id}` 没有成员 lane")]
    EmptyLaneGroup { group_id: String },
    /// RoadCorridor external ID 在 registry 内必须唯一。
    #[error("RoadCorridor id 重复：{corridor_id}")]
    DuplicateRoadCorridorId { corridor_id: String },
    /// RoadCorridor 的 elements 必须非空。
    #[error("RoadCorridor `{corridor_id}` 的 elements 不能为空")]
    EmptyRoadCorridorElements { corridor_id: String },
    /// RoadCorridor 的 element 必须引用已声明的 RoadSection 或 FacilityBand。
    #[error("RoadCorridor `{corridor_id}` 引用了不存在的 {element_kind} `{element_id}`")]
    UnknownCorridorElement {
        corridor_id: String,
        element_kind: &'static str,
        element_id: String,
    },
    /// 同一 corridor 的 elements 内不得重复引用同一 section/band。
    #[error("RoadCorridor `{corridor_id}` 重复引用 {element_kind} `{element_id}`")]
    DuplicateCorridorElement {
        corridor_id: String,
        element_kind: &'static str,
        element_id: String,
    },
    /// 同一 section/band 只能出现在一个 corridor。
    #[error(
        "{element_kind} `{element_id}` 被多个 RoadCorridor 持有：first=`{first_corridor_id}`, duplicate=`{duplicate_corridor_id}`"
    )]
    CorridorElementMultipleOwners {
        element_kind: &'static str,
        element_id: String,
        first_corridor_id: String,
        duplicate_corridor_id: String,
    },
    /// 每个 section/band 恰好属于一个 corridor（完备 owner 树）。
    #[error("{element_kind} `{element_id}` 没有 RoadCorridor owner")]
    UnownedCorridorElement {
        element_kind: &'static str,
        element_id: String,
    },
    /// RoadCorridor 的 referenceSectionId 必须是其成员 RoadSection。
    #[error(
        "RoadCorridor `{corridor_id}` 的 referenceSectionId `{reference_section_id}` 不是其成员 RoadSection"
    )]
    CorridorReferenceSectionNotMember {
        corridor_id: String,
        reference_section_id: String,
    },
    /// AccessRule external ID 在 registry 内必须唯一。
    #[error("AccessRule id 重复：{rule_id}")]
    DuplicateAccessRuleId { rule_id: String },
    /// AccessRule 的 target 必须引用已声明的实体。
    #[error("AccessRule `{rule_id}` 的 target 引用了不存在的 {target_kind} `{target_id}`")]
    UnknownAccessRuleTarget {
        rule_id: String,
        target_kind: &'static str,
        target_id: String,
    },
    /// AccessRule 的 participantClassIds 必须非空。
    #[error("AccessRule `{rule_id}` 的 participantClassIds 不能为空")]
    EmptyAccessRuleParticipantClasses { rule_id: String },
    /// AccessRule 引用的 ParticipantClass 必须存在。
    #[error("AccessRule `{rule_id}` 引用了不存在的 ParticipantClass `{class_id}`")]
    UnknownAccessRuleParticipantClass { rule_id: String, class_id: String },
    /// AccessRule 声明了 v1 不支持的能力，必须显式拒绝载入。
    #[error("AccessRule `{rule_id}` 声明了 v1 不支持的能力：{capability}")]
    AccessCapabilityUnavailable {
        rule_id: String,
        capability: &'static str,
    },
    /// 同一 package 内声明了 regulation 的规则必须共享同一 (jurisdiction, version)。
    #[error(
        "AccessRule regulation provenance 不一致：first=`{first_rule_id}` ({jurisdiction}, {version}), duplicate=`{duplicate_rule_id}` ({duplicate_jurisdiction}, {duplicate_version})"
    )]
    AccessRegulationMismatch {
        first_rule_id: String,
        jurisdiction: String,
        version: String,
        duplicate_rule_id: String,
        duplicate_jurisdiction: String,
        duplicate_version: String,
    },
    /// 经参与者/target/priority 裁决仍在 allow/deny 间并列的规则属于 authoring 歧义。
    #[error(
        "AccessRule 组合歧义：{plane} 平面 target `{target_id}` 对 ParticipantClass `{class_id}`，规则 `{first_rule_id}` 与 `{second_rule_id}` 在全部裁决轴上并列但 effect 相反"
    )]
    AccessRuleAmbiguity {
        plane: &'static str,
        target_id: String,
        class_id: String,
        first_rule_id: String,
        second_rule_id: String,
    },
    /// (ParticipantClass, Route) 绑定期静态准入校验命中 deny，绑定被原子拒绝。
    ///
    /// edge 平面时 `route_edge_index` 是命中 edge 在 route 中的 index、`target_id` 是
    /// LaneEdge external ID；path 平面时 `route_edge_index` 是 pending occurrence 的
    /// entry route edge index、`target_id` 是 ManeuverPath external ID。
    #[error(
        "(class, Route) 绑定期准入校验失败：profile `{profile_id}` 在 route `{route_id}` 的 {plane} 平面命中 deny（routeEdgeIndex={route_edge_index}, target=`{target_id}`, rule=`{rule_id}`）"
    )]
    RouteAccessDenied {
        profile_id: String,
        route_id: String,
        plane: &'static str,
        route_edge_index: usize,
        target_id: String,
        rule_id: String,
    },
    /// lane edge id 在 graph 内必须唯一。
    #[error("lane edge id 重复：{edge_id}")]
    DuplicateLaneEdgeId { edge_id: String },
    /// 同一个 source edge 内不能重复声明同一个 connection target。
    #[error("lane edge `{edge_id}` 重复声明 connection target：{next_edge_id}")]
    DuplicateLaneEdgeConnection {
        edge_id: String,
        next_edge_id: String,
    },
    /// lane edge 的 next edge 引用必须存在。
    #[error("lane edge `{edge_id}` 引用了不存在的 next edge：{next_edge_id}")]
    UnknownNextLaneEdge {
        edge_id: String,
        next_edge_id: String,
    },
    /// route id 在 world 内必须唯一。
    #[error("route id 重复：{route_id}")]
    DuplicateRouteId { route_id: String },
    /// route 至少需要一个 edge。
    #[error("route `{route_id}` 不能为空")]
    EmptyRoute { route_id: String },
    /// route 引用的 edge 必须存在。
    #[error("route `{route_id}` 引用了不存在的 lane edge：{edge_id}")]
    UnknownRouteEdge { route_id: String, edge_id: String },
    /// route 相邻 edge 必须连通。
    #[error("route `{route_id}` 中 edge `{from_edge_id}` 不能连接到 `{to_edge_id}`")]
    DisconnectedRouteEdge {
        route_id: String,
        from_edge_id: String,
        to_edge_id: String,
    },
    /// 新 static domain 的 normalization-order handle count 必须可由 u32 表示。
    #[error(
        "static domain `{domain}` entity count 超出 handle capacity：count={count}, max={max_inclusive}"
    )]
    StaticDomainCapacityExceeded {
        domain: &'static str,
        count: usize,
        max_inclusive: u32,
    },
    /// Junction external ID 在 registry 内必须唯一。
    #[error("Junction id 重复：{junction_id}")]
    DuplicateJunctionId { junction_id: String },
    /// Movement external ID 在 registry 内必须唯一。
    #[error("Movement id 重复：{movement_id}")]
    DuplicateMovementId { movement_id: String },
    /// Movement 必须引用已声明的 Junction。
    #[error("Movement `{movement_id}` 引用了不存在的 Junction `{junction_id}`")]
    UnknownMovementJunction {
        movement_id: String,
        junction_id: String,
    },
    /// ManeuverPath external ID 在 registry 内必须唯一。
    #[error("ManeuverPath id 重复：{maneuver_path_id}")]
    DuplicateManeuverPathId { maneuver_path_id: String },
    /// ManeuverPath 必须引用已声明的 Movement。
    #[error("ManeuverPath `{maneuver_path_id}` 引用了不存在的 Movement `{movement_id}`")]
    UnknownManeuverPathMovement {
        maneuver_path_id: String,
        movement_id: String,
    },
    /// ManeuverPath 的 entry/internal/exit edge 必须存在。
    #[error("ManeuverPath `{maneuver_path_id}` 的 {role} edge 引用了不存在的 LaneEdge `{edge_id}`")]
    UnknownManeuverPathEdge {
        maneuver_path_id: String,
        role: &'static str,
        edge_id: String,
    },
    /// ManeuverPath 的每个相邻 edge pair 必须可遍历。
    #[error(
        "ManeuverPath `{maneuver_path_id}` transition {transition_index} 不连通：`{from_edge_id}` -> `{to_edge_id}`"
    )]
    DisconnectedManeuverPath {
        maneuver_path_id: String,
        transition_index: usize,
        from_edge_id: String,
        to_edge_id: String,
    },
    /// 一个 resolved traversal signature 只能对应一个 ManeuverPath。
    #[error(
        "ManeuverPath 完整 edge sequence 重复：first=`{first_maneuver_path_id}` (Junction `{first_junction_id}`), duplicate=`{duplicate_maneuver_path_id}` (Junction `{duplicate_junction_id}`)"
    )]
    DuplicateManeuverPathSequence {
        first_maneuver_path_id: String,
        first_junction_id: String,
        duplicate_maneuver_path_id: String,
        duplicate_junction_id: String,
    },
    /// internal edge 的 semantic owner 在 Junction 间必须唯一。
    #[error(
        "LaneEdge `{edge_id}` 被多个 Junction 声明为 internal：first=`{first_junction_id}`, duplicate=`{duplicate_junction_id}`"
    )]
    ManeuverInternalEdgeJunctionConflict {
        edge_id: String,
        first_junction_id: String,
        duplicate_junction_id: String,
    },
    /// 同一 edge 不能同时充当 ManeuverPath internal 与 entry/exit boundary。
    #[error(
        "LaneEdge `{edge_id}` 同时声明为 internal 与 boundary：internal path=`{internal_maneuver_path_id}`, boundary path=`{boundary_maneuver_path_id}`"
    )]
    ManeuverPathEdgeRoleConflict {
        edge_id: String,
        internal_maneuver_path_id: String,
        boundary_maneuver_path_id: String,
    },
    /// 每个 Junction 至少拥有一个 Movement。
    #[error("Junction `{junction_id}` 没有 Movement")]
    EmptyJunction { junction_id: String },
    /// 每个 Movement 至少拥有一个 ManeuverPath。
    #[error("Movement `{movement_id}` 没有 ManeuverPath")]
    EmptyMovement { movement_id: String },
    /// Route 不能从 Junction internal edge 开始。
    #[error("route `{route_id}` 不能从 Junction internal edge `{edge_id}` 开始")]
    RouteStartsInsideJunction { route_id: String, edge_id: String },
    /// Route 不能在 Junction internal edge 结束。
    #[error("route `{route_id}` 不能在 Junction internal edge `{edge_id}` 结束")]
    RouteEndsInsideJunction { route_id: String, edge_id: String },
    /// entry transition 存在候选 path 时 Route 必须完整匹配一个候选。
    #[error(
        "route `{route_id}` 在 edge index {entry_route_edge_index} 未完整匹配 ManeuverPath：`{from_edge_id}` -> `{to_edge_id}`, candidates={candidate_count}"
    )]
    RouteManeuverNoFullMatch {
        route_id: String,
        entry_route_edge_index: usize,
        from_edge_id: String,
        to_edge_id: String,
        candidate_count: usize,
    },
    /// 同一 Route position 只能完整匹配一个 ManeuverPath。
    #[error(
        "route `{route_id}` 在 edge index {entry_route_edge_index} 同时匹配多个 ManeuverPath：first=`{first_maneuver_path_id}`, second=`{second_maneuver_path_id}`"
    )]
    RouteManeuverMultipleFullMatches {
        route_id: String,
        entry_route_edge_index: usize,
        first_maneuver_path_id: String,
        second_maneuver_path_id: String,
    },
    /// 两个 compiled occurrence 的 internal spans 不得重叠。
    #[error(
        "route `{route_id}` 的 internal edge occurrence {route_edge_index} 被多个 ManeuverPath 覆盖：first=`{first_maneuver_path_id}`, second=`{second_maneuver_path_id}`"
    )]
    RouteManeuverInternalOverlap {
        route_id: String,
        route_edge_index: usize,
        first_maneuver_path_id: String,
        second_maneuver_path_id: String,
    },
    /// Route 中的每个 Junction internal edge occurrence 必须由一个 path occurrence 覆盖。
    #[error(
        "route `{route_id}` 的 Junction internal edge occurrence 未被 ManeuverPath 覆盖：index={route_edge_index}, edge=`{edge_id}`"
    )]
    RouteInternalEdgeUncovered {
        route_id: String,
        route_edge_index: usize,
        edge_id: String,
    },
    /// StopLine external ID 在 registry 内必须唯一。
    #[error("StopLine id 重复：{stop_line_id}")]
    DuplicateStopLineId { stop_line_id: String },
    /// StopLine 引用的 edge 必须存在。
    #[error("StopLine `{stop_line_id}` 引用了不存在的 edge：{edge_id}")]
    UnknownStopLineEdge {
        stop_line_id: String,
        edge_id: String,
    },
    /// 每个 edge 最多声明一个 StopLine。
    #[error(
        "edge `{edge_id}` 重复声明 StopLine：first=`{first_stop_line_id}`, duplicate=`{duplicate_stop_line_id}`"
    )]
    DuplicateStopLineEdge {
        edge_id: String,
        first_stop_line_id: String,
        duplicate_stop_line_id: String,
    },
    /// SignalGroup external ID 在 registry 内必须唯一。
    #[error("SignalGroup id 重复：{group_id}")]
    DuplicateSignalGroupId { group_id: String },
    /// SignalController external ID 在 registry 内必须唯一。
    #[error("SignalController id 重复：{controller_id}")]
    DuplicateSignalControllerId { controller_id: String },
    /// SignalController 至少拥有一个 group。
    #[error("SignalController `{controller_id}` 的 groupIds 不能为空")]
    EmptySignalControllerGroups { controller_id: String },
    /// SignalController 至少拥有一个 phase。
    #[error("SignalController `{controller_id}` 的 phases 不能为空")]
    EmptySignalControllerPhases { controller_id: String },
    /// SignalController 不能重复引用同一个 group。
    #[error("SignalController `{controller_id}` 重复引用 SignalGroup `{group_id}`")]
    DuplicateSignalControllerGroup {
        controller_id: String,
        group_id: String,
    },
    /// SignalController 引用的 group 必须存在。
    #[error("SignalController `{controller_id}` 引用了不存在的 SignalGroup `{group_id}`")]
    UnknownSignalControllerGroup {
        controller_id: String,
        group_id: String,
    },
    /// 每个 SignalGroup 必须且只能属于一个 controller。
    #[error(
        "SignalGroup `{group_id}` 被多个 controller 持有：first=`{first_controller_id}`, duplicate=`{duplicate_controller_id}`"
    )]
    SignalGroupMultipleControllers {
        group_id: String,
        first_controller_id: String,
        duplicate_controller_id: String,
    },
    /// 每个 SignalGroup 必须属于一个 controller。
    #[error("SignalGroup `{group_id}` 没有 SignalController owner")]
    UnownedSignalGroup { group_id: String },
    /// 每个 SignalGroup 至少必须被一个 ManeuverGate 使用。
    #[error("SignalGroup `{group_id}` 没有被任何 ManeuverGate 使用")]
    UnusedSignalGroup { group_id: String },
    /// Phase duration 必须为 portable positive integer。
    #[error(
        "SignalController `{controller_id}` 的 Phase `{phase_id}` durationMs 无效：{duration_ms}，必须在 1..={max_inclusive}"
    )]
    InvalidSignalPhaseDuration {
        controller_id: String,
        phase_id: String,
        duration_ms: u64,
        max_inclusive: u64,
    },
    /// Controller offset 必须落在 portable safe-integer 范围内。
    #[error(
        "SignalController `{controller_id}` offsetMs 无效：{offset_ms}，必须小于或等于 {max_inclusive}"
    )]
    InvalidSignalControllerOffset {
        controller_id: String,
        offset_ms: u64,
        max_inclusive: u64,
    },
    /// 同一 controller 内的 phase ID 必须唯一。
    #[error("SignalController `{controller_id}` 的 Phase id 重复：{phase_id}")]
    DuplicateSignalPhaseId {
        controller_id: String,
        phase_id: String,
    },
    /// Phase state 只能引用 controller 拥有的 group。
    #[error(
        "SignalController `{controller_id}` 的 Phase `{phase_id}` 引用了未知 group `{group_id}`"
    )]
    UnknownSignalPhaseGroup {
        controller_id: String,
        phase_id: String,
        group_id: String,
    },
    /// Phase state 不能重复定义同一个 group。
    #[error("SignalController `{controller_id}` 的 Phase `{phase_id}` 重复定义 group `{group_id}`")]
    DuplicateSignalPhaseGroup {
        controller_id: String,
        phase_id: String,
        group_id: String,
    },
    /// Phase state 必须完整覆盖 controller 的全部 groups。
    #[error(
        "SignalController `{controller_id}` 的 Phase `{phase_id}` 缺少 group `{group_id}` state"
    )]
    MissingSignalPhaseGroup {
        controller_id: String,
        phase_id: String,
        group_id: String,
    },
    /// Controller cycle sum 必须落在 portable safe-integer 范围内。
    #[error("SignalController `{controller_id}` cycle duration 超过 {max_inclusive}")]
    SignalCycleDurationOverflow {
        controller_id: String,
        max_inclusive: u64,
    },
    /// Controller offset 必须是小于 cycle duration 的 canonical value。
    #[error(
        "SignalController `{controller_id}` offsetMs={offset_ms} 必须小于 cycleDurationMs={cycle_duration_ms}"
    )]
    SignalControllerOffsetOutOfRange {
        controller_id: String,
        offset_ms: u64,
        cycle_duration_ms: u64,
    },
    /// ManeuverGate external ID 在 registry 内必须唯一。
    #[error("ManeuverGate id 重复：{maneuver_gate_id}")]
    DuplicateManeuverGateId { maneuver_gate_id: String },
    /// ManeuverGate 必须引用已声明的 ManeuverPath。
    #[error("ManeuverGate `{maneuver_gate_id}` 引用了不存在的 ManeuverPath `{maneuver_path_id}`")]
    UnknownManeuverGatePath {
        maneuver_gate_id: String,
        maneuver_path_id: String,
    },
    /// ManeuverGate transition index 必须落在 path transition 范围内。
    #[error(
        "ManeuverGate `{maneuver_gate_id}` transitionIndex={transition_index} 越界：ManeuverPath `{maneuver_path_id}` 只有 {transition_count} 个 transitions"
    )]
    ManeuverGateTransitionOutOfRange {
        maneuver_gate_id: String,
        maneuver_path_id: String,
        transition_index: u32,
        transition_count: usize,
    },
    /// v0.9 protected profile 只支持 entry transition。
    #[error(
        "ManeuverGate `{maneuver_gate_id}` 使用不支持的 transitionIndex={transition_index}；v0.9 protected profile 只接受 0"
    )]
    UnsupportedManeuverGateTransition {
        maneuver_gate_id: String,
        transition_index: u32,
    },
    /// 同一 path transition 最多声明一个 ManeuverGate。
    #[error(
        "ManeuverPath `{maneuver_path_id}` transition {transition_index} 重复声明 ManeuverGate：first=`{first_maneuver_gate_id}`, duplicate=`{duplicate_maneuver_gate_id}`"
    )]
    DuplicateManeuverGatePathTransition {
        maneuver_path_id: String,
        transition_index: u32,
        first_maneuver_gate_id: String,
        duplicate_maneuver_gate_id: String,
    },
    /// ManeuverGate 引用的 StopLine 必须存在。
    #[error("ManeuverGate `{maneuver_gate_id}` 引用了不存在的 StopLine `{stop_line_id}`")]
    UnknownManeuverGateStopLine {
        maneuver_gate_id: String,
        stop_line_id: String,
    },
    /// ManeuverGate 的 StopLine 必须属于 path transition 的 from edge。
    #[error(
        "ManeuverGate `{maneuver_gate_id}` transition from-edge `{path_from_edge_id}` 与 StopLine `{stop_line_id}` 所属 edge `{stop_line_edge_id}` 不一致"
    )]
    ManeuverGateStopLineMismatch {
        maneuver_gate_id: String,
        stop_line_id: String,
        stop_line_edge_id: String,
        path_from_edge_id: String,
    },
    /// ManeuverGate 引用的 SignalGroup 必须存在。
    #[error("ManeuverGate `{maneuver_gate_id}` 引用了不存在的 SignalGroup `{group_id}`")]
    UnknownManeuverGateSignalGroup {
        maneuver_gate_id: String,
        group_id: String,
    },
    /// 声明 StopLine 的 outgoing connection 必须对应至少一条 ManeuverPath。
    #[error(
        "StopLine `{stop_line_id}` 缺少 ManeuverPath coverage：`{from_edge_id}` -> `{to_edge_id}`"
    )]
    MissingManeuverPathCoverage {
        stop_line_id: String,
        from_edge_id: String,
        to_edge_id: String,
    },
    /// StopLine entry 的每条 ManeuverPath 必须拥有唯一 entry Gate。
    #[error(
        "StopLine `{stop_line_id}` 的 ManeuverPath `{maneuver_path_id}` 缺少 entry ManeuverGate"
    )]
    MissingManeuverGateCoverage {
        stop_line_id: String,
        maneuver_path_id: String,
    },
    /// StopLine 必须位于至少有一个 outgoing connection 的 edge 并被 path/Gate 使用。
    #[error("StopLine `{stop_line_id}` 位于 terminal edge `{edge_id}`，无法形成 ManeuverPath")]
    OrphanStopLine {
        stop_line_id: String,
        edge_id: String,
    },
    /// Route 不得终止在声明 StopLine 的 edge 上。
    #[error("route `{route_id}` 不能终止在声明 StopLine `{stop_line_id}` 的 edge `{edge_id}` 上")]
    RouteTerminatesAtStopLine {
        route_id: String,
        edge_id: String,
        stop_line_id: String,
    },
    /// ParkingArea external ID 在 registry 内必须唯一。
    #[error("ParkingArea id 重复：{area_id}")]
    DuplicateParkingAreaId { area_id: String },
    /// ParkingSpace external ID 在 registry 内必须唯一。
    #[error("ParkingSpace id 重复：{space_id}")]
    DuplicateParkingSpaceId { space_id: String },
    /// ParkingSpace 的 optional area 引用必须存在。
    #[error("ParkingSpace `{space_id}` 引用了不存在的 ParkingArea `{area_id}`")]
    UnknownParkingSpaceArea { space_id: String, area_id: String },
    /// Parking entry/exit anchor 引用的 edge 必须存在。
    #[error("ParkingSpace `{space_id}` 的 {anchor:?} anchor 引用了不存在的 edge `{edge_id}`")]
    UnknownParkingAnchorEdge {
        space_id: String,
        anchor: ParkingAnchorKind,
        edge_id: String,
    },
    /// Parking anchor 必须严格位于 edge 两端 epsilon 之间。
    #[error(
        "ParkingSpace `{space_id}` 的 {anchor:?} anchor progress 超出范围：edge=`{edge_id}`, progress={edge_progress}, edge length={edge_length}"
    )]
    ParkingAnchorProgressOutOfRange {
        space_id: String,
        anchor: ParkingAnchorKind,
        edge_id: String,
        edge_progress: f64,
        edge_length: f64,
    },
    /// ParkingSpace geometry 必须满足 canonical numeric constraints。
    #[error("ParkingSpace `{space_id}` 的 geometry `{field}` 无效：{value}，{requirement}")]
    InvalidParkingGeometryValue {
        space_id: String,
        field: &'static str,
        value: f64,
        requirement: &'static str,
    },
    /// 每个声明的 ParkingArea 至少包含一个 member space。
    #[error("ParkingArea `{area_id}` 没有 member ParkingSpace")]
    OrphanParkingArea { area_id: String },
    /// Runtime ParkingSpace handle 必须解析到当前 world。
    #[error("未知 ParkingSpace handle：{space:?}")]
    UnknownParkingSpaceHandle { space: ParkingSpaceHandle },
    /// 普通 vehicle spawn 不得制造缺失 Occupied binding 的 Parked vehicle。
    #[error("parked vehicle `{vehicle_id}` 必须通过 spawn_parked_vehicle 创建")]
    ParkedVehicleRequiresParkingCommand { vehicle_id: String },
    /// Parking command 要求精确 lifecycle status。
    #[error(
        "Parking command {command:?} 的 vehicle {vehicle:?} 状态不匹配：expected={expected:?}, actual={actual:?}"
    )]
    ParkingVehicleStatusMismatch {
        command: ParkingCommandKind,
        vehicle: VehicleHandle,
        expected: VehicleStatus,
        actual: VehicleStatus,
    },
    /// Vehicle 已绑定另一 ParkingSpace 或 binding kind。
    #[error(
        "Parking command {command:?} 的 vehicle {vehicle:?} 已绑定 space {current_space:?} ({binding:?})，不能请求 {requested_space:?}"
    )]
    ParkingVehicleAlreadyBound {
        command: ParkingCommandKind,
        vehicle: VehicleHandle,
        requested_space: ParkingSpaceHandle,
        current_space: ParkingSpaceHandle,
        binding: ParkingBindingKind,
    },
    /// ParkingSpace 当前不可供请求 vehicle 使用。
    #[error(
        "Parking command {command:?} 的 space {space:?} 当前由 vehicle {current_vehicle:?} 以 {binding:?} 占用，请求 vehicle={requested_vehicle:?}"
    )]
    ParkingSpaceUnavailable {
        command: ParkingCommandKind,
        space: ParkingSpaceHandle,
        requested_vehicle: VehicleHandle,
        current_vehicle: VehicleHandle,
        binding: ParkingBindingKind,
    },
    /// Command 要求 exact Reserved pair。
    #[error(
        "Parking command {command:?} 要求 exact reservation：vehicle={vehicle:?}, space={space:?}"
    )]
    ParkingReservationMismatch {
        command: ParkingCommandKind,
        vehicle: VehicleHandle,
        space: ParkingSpaceHandle,
    },
    /// Command 要求 exact Occupied pair。
    #[error(
        "Parking command {command:?} 要求 exact occupancy：vehicle={vehicle:?}, space={space:?}"
    )]
    ParkingOccupancyMismatch {
        command: ParkingCommandKind,
        vehicle: VehicleHandle,
        space: ParkingSpaceHandle,
    },
    /// Explicit commit 前 Reserved vehicle 必须满足 arrival predicate。
    #[error("vehicle {vehicle:?} 尚未到达 ParkingSpace {space:?} 的 entry")]
    ParkingVehicleNotArrived {
        vehicle: VehicleHandle,
        space: ParkingSpaceHandle,
    },
    /// Parking route occurrence 必须落在 active route 范围内。
    #[error(
        "Parking command {command:?} 的 route occurrence 越界：vehicle={vehicle:?}, route={route:?}, index={route_edge_index}, count={route_edge_count}"
    )]
    InvalidParkingRouteOccurrence {
        command: ParkingCommandKind,
        vehicle: VehicleHandle,
        route: RouteHandle,
        route_edge_index: usize,
        route_edge_count: usize,
    },
    /// Parking entry/exit anchor 必须与 caller occurrence physical edge 相同。
    #[error(
        "Parking command {command:?} 的 {anchor:?} occurrence edge 不匹配：space={space:?}, route={route:?}, index={route_edge_index}, expected={expected_edge:?}, actual={actual_edge:?}"
    )]
    ParkingRouteOccurrenceEdgeMismatch {
        command: ParkingCommandKind,
        space: ParkingSpaceHandle,
        anchor: ParkingAnchorKind,
        route: RouteHandle,
        route_edge_index: usize,
        expected_edge: EdgeHandle,
        actual_edge: EdgeHandle,
    },
    /// Reserved rebind 不能改变当前 physical edge。
    #[error(
        "reserved vehicle {vehicle:?} rebind 会改变 physical edge：space={space:?}, route={route:?}, index={route_edge_index}, current={current_edge:?}, target={target_edge:?}"
    )]
    ParkingRouteRebindEdgeMismatch {
        vehicle: VehicleHandle,
        space: ParkingSpaceHandle,
        route: RouteHandle,
        route_edge_index: usize,
        current_edge: EdgeHandle,
        target_edge: EdgeHandle,
    },
    /// Reserved target route 从 caller occurrence 起必须包含可达 entry。
    #[error(
        "vehicle {vehicle:?} 从 route {route:?} occurrence {from_route_edge_index} 无法到达 ParkingSpace {space:?} entry"
    )]
    ParkingEntryUnreachable {
        vehicle: VehicleHandle,
        space: ParkingSpaceHandle,
        route: RouteHandle,
        from_route_edge_index: usize,
    },
    /// Leave 插入会迫使 direct follower 依赖 geometry hard projection。
    #[error(
        "vehicle {vehicle:?} 离开 ParkingSpace {space:?} 对 direct follower {follower:?} 不满足 emergency envelope"
    )]
    ParkingLeaveUnsafeFollower {
        vehicle: VehicleHandle,
        space: ParkingSpaceHandle,
        follower: VehicleHandle,
    },
    /// Public API 不应制造的 Parking aggregate 矛盾。
    #[error(
        "Parking binding invariant violation：stage={stage}, vehicle={vehicle:?}, space={space:?}"
    )]
    ParkingBindingInvariantViolation {
        stage: &'static str,
        vehicle: Option<VehicleHandle>,
        space: Option<ParkingSpaceHandle>,
    },
    /// Parking command 的有限数值计算失败。
    #[error(
        "Parking computation 不是 finite：stage={stage}, vehicle={vehicle:?}, space={space:?}, value={value}"
    )]
    NonFiniteParkingComputation {
        stage: &'static str,
        vehicle: VehicleHandle,
        space: ParkingSpaceHandle,
        value: f64,
    },
    /// #108 过渡期的 legacy capability guard；#109 激活后合法 world 不再返回。
    #[error(
        "旧版 v0.5 Parking 车辆能力防护错误：#109 激活后不应再触发；若再次出现，请检查 ParkingStop、arrival、traversal 与 completion release 是否完整接入"
    )]
    ParkingVehicleCapabilityUnavailable,
    /// World fixed delta 不得超过任一 static SignalPhase duration。
    #[error(
        "SignalController `{controller_id}` 的 Phase `{phase_id}` durationMs={duration_ms} 小于 fixedDeltaTimeMs={fixed_delta_time_ms}"
    )]
    SignalPhaseShorterThanFixedDelta {
        controller_id: String,
        phase_id: String,
        duration_ms: u64,
        fixed_delta_time_ms: u64,
    },
    /// #94/#95 的 legacy capability guard error；#96 完整合规闭环后不再返回。
    #[error(
        "旧版 v0.4 Signals 车辆能力防护错误：#96 完整合规后不应再触发；若再次出现，请检查 SignalStop、hard projection 与 permission-aware traversal 是否完整接入"
    )]
    SignalsVehicleCapabilityUnavailable,
    /// vehicle id 在 world 内必须唯一。
    #[error("vehicle id 重复：{vehicle_id}")]
    DuplicateVehicleId { vehicle_id: String },
    /// vehicle 引用的 Vehicle Profile handle 必须属于当前 world registry。
    #[error("vehicle `{vehicle_id}` 引用了未知的 Vehicle Profile handle：{profile:?}")]
    UnknownVehicleProfileHandle {
        vehicle_id: String,
        profile: VehicleProfileHandle,
    },
    /// inactive vehicle 的初始运动状态必须为零。
    #[error(
        "inactive vehicle `{vehicle_id}` 的初始速度必须为 0：status={status:?}, initial_speed={initial_speed}"
    )]
    InvalidInactiveVehicleMotion {
        vehicle_id: String,
        status: VehicleStatus,
        initial_speed: f64,
    },
    /// vehicle 初始速度不得超过当前 edge 的基础道路限速。
    #[error(
        "vehicle `{vehicle_id}` 的初始速度超过 edge `{edge_id}` 基础限速：initial_speed={initial_speed}, speed_limit={speed_limit}"
    )]
    VehicleInitialSpeedExceedsLimit {
        vehicle_id: String,
        edge_id: String,
        initial_speed: f64,
        speed_limit: f64,
    },
    /// candidate vehicle 与现有 vehicle 的物理车身不得重叠。
    #[error(
        "vehicle `{follower_id}` 与 leader `{leader_id}` 发生物理重叠：bumper_gap={bumper_gap}"
    )]
    VehiclePhysicalOverlap {
        follower_id: String,
        leader_id: String,
        bumper_gap: f64,
    },
    /// vehicle 引用的 route 必须存在。
    #[error("vehicle `{vehicle_id}` 引用了不存在的 route：{route_id}")]
    UnknownVehicleRoute {
        vehicle_id: String,
        route_id: String,
    },
    /// vehicle route edge index 必须落在 route edge sequence 范围内。
    #[error(
        "vehicle `{vehicle_id}` 的 route edge index 无效：route `{route_id}` 长度为 {route_edge_count}，实际 index 为 {route_edge_index}"
    )]
    InvalidVehicleRouteEdgeIndex {
        vehicle_id: String,
        route_id: String,
        route_edge_index: usize,
        route_edge_count: usize,
    },
    /// vehicle edge progress 必须小于或等于当前 edge length。
    #[error(
        "vehicle `{vehicle_id}` 在 edge `{edge_id}` 上的 progress 超出范围：progress={edge_progress}，edge length={edge_length}"
    )]
    VehicleEdgeProgressOutOfRange {
        vehicle_id: String,
        edge_id: String,
        edge_progress: f64,
        edge_length: f64,
    },
    /// completed vehicle 的初始位置必须位于 route 终点。
    #[error(
        "completed vehicle `{vehicle_id}` 的初始状态无效：route `{route_id}` 期望最后 edge index={expected_route_edge_index} 且 progress 在终点 epsilon 内，实际 index={route_edge_index}, progress={edge_progress}, edge length={edge_length}"
    )]
    InvalidCompletedVehicleState {
        vehicle_id: String,
        route_id: String,
        route_edge_index: usize,
        expected_route_edge_index: usize,
        edge_progress: f64,
        edge_length: f64,
    },
    /// vehicle handle 必须指向当前 active vehicle slot。
    #[error("vehicle handle 无效或已过期：{vehicle:?}；active resolver 将返回 None")]
    UnknownVehicleHandle { vehicle: VehicleHandle },
    /// atomic replace 只接受 Completed old vehicle。
    #[error("atomic replace 只接受 Completed vehicle：vehicle={vehicle:?}, actual={actual:?}")]
    VehicleReplaceStatusMismatch {
        vehicle: VehicleHandle,
        actual: VehicleStatus,
    },
    /// atomic replace 的 `route_edge_index` 必须落在 active route edge sequence 范围内。
    #[error(
        "atomic replace 的 route edge index 无效：vehicle={vehicle:?}, route={route:?}, route_edge_count={route_edge_count}, actual={route_edge_index}"
    )]
    InvalidVehicleReplaceRouteEdgeIndex {
        vehicle: VehicleHandle,
        route: RouteHandle,
        route_edge_index: usize,
        route_edge_count: usize,
    },
    /// atomic replace 的 edge progress 必须落在目标 edge 范围内。
    #[error(
        "atomic replace 的 edge progress 超出范围：vehicle={vehicle:?}, edge={edge:?}, progress={edge_progress}, edge_length={edge_length}"
    )]
    VehicleReplaceEdgeProgressOutOfRange {
        vehicle: VehicleHandle,
        edge: EdgeHandle,
        edge_progress: f64,
        edge_length: f64,
    },
    /// atomic replace 的初始速度不得超过目标 edge 的基础道路限速。
    #[error(
        "atomic replace 的初始速度超过基础道路限速：vehicle={vehicle:?}, edge={edge:?}, initial_speed={initial_speed}, speed_limit={speed_limit}"
    )]
    VehicleReplaceInitialSpeedExceedsLimit {
        vehicle: VehicleHandle,
        edge: EdgeHandle,
        initial_speed: f64,
        speed_limit: f64,
    },

    /// route handle 必须指向当前 active route slot。
    #[error("route handle 无效或已过期：{route:?}；active resolver 将返回 None")]
    UnknownRouteHandle { route: RouteHandle },
    /// 正被 live vehicle 引用的 route 不能被移除。
    #[error("route `{route:?}` 仍被 vehicle `{vehicle:?}` 引用，不能移除")]
    RouteInUse {
        route: RouteHandle,
        vehicle: VehicleHandle,
    },
    /// leader detection 的 horizon 或 route distance 计算必须保持 finite。
    #[error(
        "vehicle `{vehicle:?}` 的 leader detection 计算不是 finite：stage={stage}, value={value}"
    )]
    NonFiniteLeaderComputation {
        vehicle: VehicleHandle,
        stage: &'static str,
        value: f64,
    },
    /// IIDM、safe-speed、ballistic integration 或 geometry projection 必须保持 finite。
    #[error("vehicle `{vehicle:?}` 的纵向计算不是 finite：stage={stage}, value={value}")]
    NonFiniteLongitudinalComputation {
        vehicle: VehicleHandle,
        stage: &'static str,
        value: f64,
    },
    /// speed-limit route-distance、braking envelope 或 crossing guard 必须保持 finite。
    #[error("vehicle `{vehicle:?}` 的 speed-limit 计算不是 finite：stage={stage}, value={value}")]
    NonFiniteSpeedLimitComputation {
        vehicle: VehicleHandle,
        stage: &'static str,
        value: f64,
    },
    /// 最终 traversal 不得以超过目标 edge 基础限速的速度跨越降限速边界。
    #[error(
        "vehicle `{vehicle:?}` 的 speed-limit motion 与 edge traversal 矛盾：route={route:?}, occurrence={from_route_edge_index}->{to_route_edge_index}, edge={from_edge:?}->{to_edge:?}, final_speed={final_speed}, target_limit={target_limit}"
    )]
    SpeedLimitTraversalInvariant {
        vehicle: VehicleHandle,
        route: RouteHandle,
        from_route_edge_index: usize,
        to_route_edge_index: usize,
        from_edge: EdgeHandle,
        to_edge: EdgeHandle,
        final_speed: f64,
        target_limit: f64,
    },
    /// SignalStop route-distance、reducer 或 hard projection 必须保持 finite。
    #[error("vehicle `{vehicle:?}` 的 SignalStop 计算不是 finite：stage={stage}, value={value}")]
    NonFiniteSignalStopComputation {
        vehicle: VehicleHandle,
        stage: &'static str,
        value: f64,
    },
    /// D6 motion 与 D7 denied traversal 不得产生可越过 Gate 的剩余位移。
    #[error(
        "vehicle `{vehicle:?}` 的 SignalStop motion 与 denied Gate traversal 矛盾：route={route:?}, occurrence={from_route_edge_index}->{to_route_edge_index}, gate={gate:?}, remaining={remaining_travel}, final_speed={final_speed}"
    )]
    SignalTraversalDeniedInvariant {
        vehicle: VehicleHandle,
        route: RouteHandle,
        from_route_edge_index: usize,
        to_route_edge_index: usize,
        gate: ManeuverGateHandle,
        remaining_travel: f64,
        final_speed: f64,
    },
    /// ParkingStop motion 与 traversal 不得产生越过 selected entry 的剩余位移。
    #[error(
        "vehicle `{vehicle:?}` 的 ParkingStop motion 与 entry traversal 矛盾：space={space:?}, route={route:?}, occurrence={route_edge_index}, remaining={remaining_travel}, final_speed={final_speed}"
    )]
    ParkingTraversalBoundaryInvariant {
        vehicle: VehicleHandle,
        space: ParkingSpaceHandle,
        route: RouteHandle,
        route_edge_index: usize,
        remaining_travel: f64,
        final_speed: f64,
    },
    /// route following 计算出的 travel distance 必须保持 finite。
    #[error(
        "vehicle `{vehicle:?}` 的 route travel distance 不是 finite：speed={speed}, delta={delta_time_ms} ms；可通过同一 CoreWorld resolver 查询 external ID"
    )]
    NonFiniteRouteTravel {
        vehicle: VehicleHandle,
        speed: f64,
        delta_time_ms: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_messages_use_chinese_runtime_text() {
        assert_eq!(
            CoreError::InvalidFixedDeltaTime {
                fixed_delta_time_ms: 0
            }
            .to_string(),
            "`fixed_delta_time_ms` 必须大于 0，实际值为 0"
        );
        assert_eq!(
            CoreError::TickDeltaMismatch {
                expected_delta_time_ms: 20,
                actual_delta_time_ms: 16
            }
            .to_string(),
            "tick delta 不匹配：期望 20 ms，实际 16 ms"
        );
        assert_eq!(
            CoreError::TimeOverflow.to_string(),
            "tick/time 累计发生整数溢出"
        );
        assert_eq!(
            CoreError::SignalsVehicleCapabilityUnavailable.to_string(),
            "旧版 v0.4 Signals 车辆能力防护错误：#96 完整合规后不应再触发；若再次出现，请检查 SignalStop、hard projection 与 permission-aware traversal 是否完整接入"
        );
        assert_eq!(
            CoreError::InvalidSpeed { speed: -1.0 }.to_string(),
            "speed 无效：-1"
        );
        assert_eq!(
            CoreError::InvalidAcceleration { acceleration: -2.5 }.to_string(),
            "acceleration 无效：-2.5"
        );
        assert_eq!(
            CoreError::InvalidEdgeProgress {
                edge_progress: f64::NAN
            }
            .to_string(),
            "edge progress 无效：NaN"
        );
        assert_eq!(
            CoreError::InvalidExternalId {
                field: "laneGraph.edges[].id",
                external_id: "edge 1".to_owned(),
                pattern: "^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$",
            }
            .to_string(),
            "external ID 无效：field=laneGraph.edges[].id, value=`edge 1`，必须匹配 ^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$"
        );
        assert_eq!(
            CoreError::DuplicateLaneEdgeConnection {
                edge_id: "A".to_owned(),
                next_edge_id: "B".to_owned(),
            }
            .to_string(),
            "lane edge `A` 重复声明 connection target：B"
        );
        assert_eq!(
            CoreError::UnknownVehicleProfileHandle {
                vehicle_id: "V1".to_owned(),
                profile: VehicleProfileHandle::new(1),
            }
            .to_string(),
            format!(
                "vehicle `V1` 引用了未知的 Vehicle Profile handle：{:?}",
                VehicleProfileHandle::new(1)
            )
        );
        assert_eq!(
            CoreError::InvalidInactiveVehicleMotion {
                vehicle_id: "V1".to_owned(),
                status: VehicleStatus::Stopped,
                initial_speed: 1.0,
            }
            .to_string(),
            "inactive vehicle `V1` 的初始速度必须为 0：status=Stopped, initial_speed=1"
        );
        assert_eq!(
            CoreError::InvalidCompletedVehicleState {
                vehicle_id: "V1".to_owned(),
                route_id: "R1".to_owned(),
                route_edge_index: 0,
                expected_route_edge_index: 1,
                edge_progress: 1.0,
                edge_length: 5.0,
            }
            .to_string(),
            "completed vehicle `V1` 的初始状态无效：route `R1` 期望最后 edge index=1 且 progress 在终点 epsilon 内，实际 index=0, progress=1, edge length=5"
        );
        assert_eq!(
            CoreError::VehiclePhysicalOverlap {
                follower_id: "V1".to_owned(),
                leader_id: "V2".to_owned(),
                bumper_gap: -0.5,
            }
            .to_string(),
            "vehicle `V1` 与 leader `V2` 发生物理重叠：bumper_gap=-0.5"
        );
        assert_eq!(
            CoreError::NonFiniteLeaderComputation {
                vehicle: VehicleHandle::new(0, 0),
                stage: "hard_horizon",
                value: f64::INFINITY,
            }
            .to_string(),
            format!(
                "vehicle `{:?}` 的 leader detection 计算不是 finite：stage=hard_horizon, value=inf",
                VehicleHandle::new(0, 0)
            )
        );
        assert_eq!(
            CoreError::NonFiniteLongitudinalComputation {
                vehicle: VehicleHandle::new(0, 0),
                stage: "ballistic_travel",
                value: f64::INFINITY,
            }
            .to_string(),
            format!(
                "vehicle `{:?}` 的纵向计算不是 finite：stage=ballistic_travel, value=inf",
                VehicleHandle::new(0, 0)
            )
        );
        assert_eq!(
            CoreError::NonFiniteRouteTravel {
                vehicle: VehicleHandle::new(0, 0),
                speed: f64::MAX,
                delta_time_ms: 1_000,
            }
            .to_string(),
            format!(
                "vehicle `{:?}` 的 route travel distance 不是 finite：speed={}, delta=1000 ms；可通过同一 CoreWorld resolver 查询 external ID",
                VehicleHandle::new(0, 0),
                f64::MAX
            )
        );
    }
}
