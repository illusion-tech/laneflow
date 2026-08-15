//! WaitingZone 领域错误（`CoreError::WaitingZone` 的嵌套子枚举）。

/// WaitingZone 静态模型、route binding 与 capability guard 的领域错误。
#[derive(Clone, Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WaitingZoneError {
    /// WaitingZone external ID 在 registry 内必须唯一。
    #[error("WaitingZone id 重复：{waiting_zone_id}")]
    DuplicateId { waiting_zone_id: String },
    /// WaitingZone 必须引用已声明的 ManeuverPath。
    #[error("WaitingZone `{waiting_zone_id}` 引用了不存在的 ManeuverPath `{maneuver_path_id}`")]
    UnknownPath {
        waiting_zone_id: String,
        maneuver_path_id: String,
    },
    /// WaitingZone entry/release gate 必须存在。
    #[error(
        "WaitingZone `{waiting_zone_id}` 的 {gate_role}GateId 引用了不存在的 ManeuverGate `{maneuver_gate_id}`"
    )]
    UnknownGate {
        waiting_zone_id: String,
        gate_role: &'static str,
        maneuver_gate_id: String,
    },
    /// WaitingZone gate 必须属于声明的 ManeuverPath。
    #[error(
        "WaitingZone `{waiting_zone_id}` 的 {gate_role} gate `{maneuver_gate_id}` 不属于 ManeuverPath `{maneuver_path_id}`"
    )]
    GatePathMismatch {
        waiting_zone_id: String,
        gate_role: &'static str,
        maneuver_gate_id: String,
        maneuver_path_id: String,
    },
    /// WaitingZone entry transition 必须严格早于 release transition。
    #[error(
        "WaitingZone `{waiting_zone_id}` gate 顺序无效：entry transition {entry_transition_index} 必须早于 release transition {release_transition_index}"
    )]
    InvalidGateOrder {
        waiting_zone_id: String,
        entry_transition_index: u32,
        release_transition_index: u32,
    },
    /// WaitingZone maxOccupancy 必须大于 0。
    #[error("WaitingZone `{waiting_zone_id}` maxOccupancy 必须大于 0")]
    InvalidMaxOccupancy { waiting_zone_id: String },
    /// 同一 ManeuverPath 上 WaitingZone interior 不得重叠或嵌套。
    #[error(
        "ManeuverPath `{maneuver_path_id}` 上 WaitingZone interior 重叠：first=`{first_waiting_zone_id}`, second=`{second_waiting_zone_id}`"
    )]
    Overlap {
        maneuver_path_id: String,
        first_waiting_zone_id: String,
        second_waiting_zone_id: String,
    },
    /// WaitingZone 的 boundary-to-boundary route distance 必须可证明为 finite。
    #[error(
        "Vehicle Profile `{profile_id}` 无法绑定 route `{route_id}`：WaitingZone `{waiting_zone_id}` 空容量在 route edge {entry_route_edge_index}..={release_route_edge_index} 上无法证明为 finite"
    )]
    StorageDistanceUnprovable {
        profile_id: String,
        route_id: String,
        waiting_zone_id: String,
        entry_route_edge_index: usize,
        release_route_edge_index: usize,
    },
    /// profile-route binding 必须满足空 WaitingZone 的整车长度容量。
    #[error(
        "Vehicle Profile `{profile_id}` 无法绑定 route `{route_id}`：WaitingZone `{waiting_zone_id}` 空容量 {available_meters} m 小于车长 {required_meters} m"
    )]
    InsufficientStorage {
        profile_id: String,
        route_id: String,
        waiting_zone_id: String,
        available_meters: f64,
        required_meters: f64,
    },
    /// WaitingZone runtime authority 尚未实现，禁止静默穿越。
    #[error(
        "route `{route_id}` 包含 pending WaitingZone `{waiting_zone_id}`；WaitingZone runtime authority 尚未实现"
    )]
    RuntimeUnavailable {
        route_id: String,
        waiting_zone_id: String,
    },
}
