//! Core runtime 的错误类型。

use crate::{MovementGateKey, RouteHandle, VehicleHandle, VehicleProfileHandle, VehicleStatus};

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
    /// 每个 SignalGroup 至少必须被一个 MovementGate 使用。
    #[error("SignalGroup `{group_id}` 没有被任何 MovementGate 使用")]
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
    /// MovementGate 的 from edge 必须存在。
    #[error("MovementGate 引用了不存在的 fromEdgeId：{edge_id}")]
    UnknownMovementGateFromEdge { edge_id: String },
    /// MovementGate 的 to edge 必须存在。
    #[error("MovementGate 引用了不存在的 toEdgeId：{edge_id}")]
    UnknownMovementGateToEdge { edge_id: String },
    /// MovementGate pair 必须是 lane graph 中的合法 connection。
    #[error("MovementGate `{from_edge_id}` -> `{to_edge_id}` 不是合法 connection")]
    DisconnectedMovementGate {
        from_edge_id: String,
        to_edge_id: String,
    },
    /// MovementGate pair 在 registry 内必须唯一。
    #[error("MovementGate pair 重复：`{from_edge_id}` -> `{to_edge_id}`")]
    DuplicateMovementGate {
        from_edge_id: String,
        to_edge_id: String,
    },
    /// MovementGate 引用的 StopLine 必须存在。
    #[error("MovementGate 引用了不存在的 StopLine `{stop_line_id}`")]
    UnknownMovementGateStopLine { stop_line_id: String },
    /// MovementGate 的 StopLine 必须属于 from edge。
    #[error(
        "MovementGate fromEdgeId `{from_edge_id}` 与 StopLine `{stop_line_id}` 所属 edge `{stop_line_edge_id}` 不一致"
    )]
    MovementGateStopLineMismatch {
        stop_line_id: String,
        stop_line_edge_id: String,
        from_edge_id: String,
    },
    /// MovementGate 引用的 SignalGroup 必须存在。
    #[error("MovementGate 引用了不存在的 SignalGroup `{group_id}`")]
    UnknownMovementGateSignalGroup { group_id: String },
    /// 声明 StopLine 的 edge 必须为每个 outgoing connection 定义 Gate。
    #[error(
        "StopLine `{stop_line_id}` 缺少 MovementGate coverage：`{from_edge_id}` -> `{to_edge_id}`"
    )]
    MissingMovementGateCoverage {
        stop_line_id: String,
        from_edge_id: String,
        to_edge_id: String,
    },
    /// StopLine 必须位于至少有一个 outgoing connection 的 edge 并被 Gate 使用。
    #[error("StopLine `{stop_line_id}` 位于 terminal edge `{edge_id}`，无法形成 MovementGate")]
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
    #[error("legacy v0.4 Signals capability guard error：#96 完整合规后不再返回")]
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
        gate: MovementGateKey,
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
