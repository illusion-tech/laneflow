use laneflow_core::{EdgeHandle, RouteHandle, VehicleHandle};
use thiserror::Error;

/// v0.8 signalized-corridor population startup 或 lifecycle 失败。
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CorridorPopulationError {
    /// startup target 不在 v0.8 corridor 允许范围内。
    #[error("target vehicle count {actual} 不在 {min}..={max} 范围内")]
    InvalidTargetVehicleCount {
        /// 最小允许值。
        min: usize,
        /// 最大允许值。
        max: usize,
        /// 实际值。
        actual: usize,
    },
    /// catalog TOML 无法解析。
    #[error("无法解析 signalized-corridor catalog TOML：{0}")]
    CatalogToml(#[from] toml::de::Error),
    /// catalog version 不是当前内部契约。
    #[error("不支持 catalog version {actual:?}，当前要求 {expected:?}")]
    UnsupportedCatalogVersion {
        /// 当前要求。
        expected: &'static str,
        /// 输入值。
        actual: String,
    },
    /// catalog portal ID 未知。
    #[error("未知 corridor portal {portal_id:?}")]
    UnknownPortal {
        /// 输入 portal ID。
        portal_id: String,
    },
    /// catalog 缺少冻结 portal。
    #[error("catalog 缺少 portal {portal_id:?}")]
    MissingPortal {
        /// 缺少的 portal ID。
        portal_id: &'static str,
    },
    /// catalog portal ID 重复。
    #[error("catalog portal {portal_id:?} 重复")]
    DuplicatePortal {
        /// 重复的 portal ID。
        portal_id: String,
    },
    /// portal 的 entry route 数量不符合固定 6/4/4 topology。
    #[error("portal {portal_id:?} 需要 {expected} 条 entry routes，实际为 {actual}")]
    InvalidPortalRouteCount {
        /// portal ID。
        portal_id: String,
        /// 冻结数量。
        expected: usize,
        /// 实际数量。
        actual: usize,
    },
    /// portal entry route 引用重复。
    #[error("portal {portal_id:?} 重复引用 route {route_id:?}")]
    DuplicatePortalRoute {
        /// portal ID。
        portal_id: String,
        /// route ID。
        route_id: String,
    },
    /// route ID 重复。
    #[error("catalog route {route_id:?} 重复")]
    DuplicateRoute {
        /// route ID。
        route_id: String,
    },
    /// catalog route 不存在于 production Traffic。
    #[error("catalog route {route_id:?} 不存在于 production Traffic")]
    UnknownTrafficRoute {
        /// route ID。
        route_id: String,
    },
    /// route entry/exit portal 关系非法。
    #[error(
        "route {route_id:?} 的 entry/exit portal 关系非法：{entry_portal_id:?} -> {exit_portal_id:?}"
    )]
    InvalidRoutePortals {
        /// route ID。
        route_id: String,
        /// entry portal。
        entry_portal_id: String,
        /// exit portal。
        exit_portal_id: String,
    },
    /// lane index 超出 portal 固定 lane 数。
    #[error(
        "route {route_id:?} lane index {lane_index} 超出 portal {portal_id:?} 的 {lane_count} 条 lanes"
    )]
    InvalidLaneIndex {
        /// route ID。
        route_id: String,
        /// portal ID。
        portal_id: String,
        /// lane index。
        lane_index: usize,
        /// lane count。
        lane_count: usize,
    },
    /// 同一 portal/lane 出现多个 route。
    #[error("portal {portal_id:?} lane {lane_index} 存在重复 route")]
    DuplicatePortalLane {
        /// portal ID。
        portal_id: String,
        /// lane index。
        lane_index: usize,
    },
    /// portal entry route set 与 route entries 不一致。
    #[error("portal {portal_id:?} 的 entry_route_ids 与 route catalog 不一致")]
    PortalRouteSetMismatch {
        /// portal ID。
        portal_id: String,
    },
    /// spawn slot ID 重复。
    #[error("spawn slot {slot_id:?} 重复")]
    DuplicateSpawnSlot {
        /// slot ID。
        slot_id: String,
    },
    /// spawn slot 引用未知 route。
    #[error("spawn slot {slot_id:?} 引用未知 route {route_id:?}")]
    UnknownSlotRoute {
        /// slot ID。
        slot_id: String,
        /// route ID。
        route_id: String,
    },
    /// spawn slot 的 portal 与 route entry portal 不一致。
    #[error(
        "spawn slot {slot_id:?} portal {portal_id:?} 与 route {route_id:?} entry portal 不一致"
    )]
    SlotPortalMismatch {
        /// slot ID。
        slot_id: String,
        /// portal ID。
        portal_id: String,
        /// route ID。
        route_id: String,
    },
    /// spawn slot 的 route occurrence 不存在。
    #[error("spawn slot {slot_id:?} route edge index {route_edge_index} 越界")]
    SlotRouteEdgeIndexOutOfRange {
        /// slot ID。
        slot_id: String,
        /// route edge index。
        route_edge_index: usize,
    },
    /// spawn slot edge ID 与 route occurrence 不一致。
    #[error(
        "spawn slot {slot_id:?} edge {actual_edge_id:?} 与 route occurrence {expected_edge_id:?} 不一致"
    )]
    SlotEdgeMismatch {
        /// slot ID。
        slot_id: String,
        /// route occurrence 要求的 edge ID。
        expected_edge_id: String,
        /// catalog 实际 edge ID。
        actual_edge_id: String,
    },
    /// spawn slot progress 非有限、为负或超出 edge。
    #[error("spawn slot {slot_id:?} progress {progress} 不在 0..={edge_length} 范围内")]
    InvalidSlotProgress {
        /// slot ID。
        slot_id: String,
        /// progress。
        progress: f64,
        /// edge length。
        edge_length: f64,
    },
    /// 两个 slot 指向相同物理 edge/progress。
    #[error("spawn slot {slot_id:?} 与 {existing_slot_id:?} 占用相同 edge/progress")]
    DuplicateSpawnLocation {
        /// 后出现的 slot。
        slot_id: String,
        /// 先出现的 slot。
        existing_slot_id: String,
    },
    /// route entry slot 不存在或不匹配。
    #[error("route {route_id:?} 的 entry spawn slot {slot_id:?} 不存在或不位于 route edge 0")]
    InvalidEntrySpawnSlot {
        /// route ID。
        route_id: String,
        /// entry slot ID。
        slot_id: String,
    },
    /// catalog 没有达到 v0.8 或 target 容量。
    #[error("spawn slots 不足：至少需要 {required}，实际为 {actual}")]
    InsufficientSpawnSlots {
        /// 要求数量。
        required: usize,
        /// 实际数量。
        actual: usize,
    },
    /// profile handle 不属于准备输入或绑定后的 world。
    #[error("未知 Vehicle Profile handle")]
    UnknownVehicleProfile,
    /// bind 发现 world 与 normalized catalog 不是同一 Traffic authority。
    #[error("绑定 world 与 normalized catalog 不一致：{detail}")]
    BoundWorldCatalogMismatch {
        /// 不一致详情。
        detail: String,
    },
    /// bind 必须发生在首个 Core step 前。
    #[error("population controller 只能绑定 tick 0 world，实际 tick 为 {tick_index}")]
    WorldAlreadyStepped {
        /// world 当前 tick。
        tick_index: u64,
    },
    /// bind 找不到 logical vehicle。
    #[error("绑定时找不到 logical vehicle {vehicle_id:?}")]
    MissingInitialVehicle {
        /// logical external ID。
        vehicle_id: String,
    },
    /// bind 发现 initial vehicle route/profile/status 与 prepared plan 不一致。
    #[error("logical vehicle {vehicle_id:?} 与 prepared population plan 不一致")]
    InitialVehicleMismatch {
        /// logical external ID。
        vehicle_id: String,
    },
    /// 两个 logical slots 解析到同一 initial handle。
    #[error("initial vehicle handle {vehicle:?} 被多个 logical slots 解析")]
    DuplicateInitialVehicleHandle {
        /// 重复 handle。
        vehicle: VehicleHandle,
    },
    /// StepResult tick 没有严格前进。
    #[error("StepResult tick {actual} 必须大于已消费 tick {previous}")]
    NonMonotonicStep {
        /// 已消费 tick。
        previous: u64,
        /// 当前输入 tick。
        actual: u64,
    },
    /// completion event tick 与 StepResult 不一致。
    #[error("completion event tick {event_tick} 与 StepResult tick {step_tick} 不一致")]
    CompletionTickMismatch {
        /// StepResult tick。
        step_tick: u64,
        /// event tick。
        event_tick: u64,
    },
    /// completion handle 不属于 Running logical slot。
    #[error("completion vehicle {vehicle:?} 未知或不处于 Running")]
    UnknownCompletionVehicle {
        /// event vehicle。
        vehicle: VehicleHandle,
    },
    /// 同一 StepResult 重复报告同一 logical slot。
    #[error("completion vehicle {vehicle:?} 在同一 StepResult 中重复")]
    DuplicateCompletionVehicle {
        /// event vehicle。
        vehicle: VehicleHandle,
    },
    /// completion route 与 Running slot 当前 route 不一致。
    #[error("completion vehicle {vehicle:?} route {actual:?} 与 expected {expected:?} 不一致")]
    CompletionRouteMismatch {
        /// event vehicle。
        vehicle: VehicleHandle,
        /// expected route。
        expected: RouteHandle,
        /// actual route。
        actual: RouteHandle,
    },
    /// completion edge occurrence 与 route 末端不一致。
    #[error(
        "completion vehicle {vehicle:?} edge occurrence {actual_edge:?}@{actual_route_edge_index} \
         与 expected {expected_edge:?}@{expected_route_edge_index} 不一致"
    )]
    CompletionEdgeOccurrenceMismatch {
        /// event vehicle。
        vehicle: VehicleHandle,
        /// expected route 末端 edge。
        expected_edge: EdgeHandle,
        /// expected route 末端 occurrence index。
        expected_route_edge_index: usize,
        /// event edge。
        actual_edge: EdgeHandle,
        /// event occurrence index。
        actual_route_edge_index: usize,
    },
    /// lifecycle outcome 不属于本次 old handle。
    #[error("replace outcome old {actual:?} 与 pending old {expected:?} 不一致")]
    ReplaceOutcomeOldMismatch {
        /// pending old。
        expected: VehicleHandle,
        /// outcome old。
        actual: VehicleHandle,
    },
    /// replacement 返回的 new handle 已被另一个 logical slot 跟踪。
    #[error("replacement new handle {vehicle:?} 已被 logical population 跟踪")]
    ReplacementHandleAlreadyTracked {
        /// 重复 new handle。
        vehicle: VehicleHandle,
    },
}
