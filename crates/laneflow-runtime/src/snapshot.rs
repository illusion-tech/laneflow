//! 运行时快照的保存路径（#302 快照合同 §5；#512 切片 B）。
//!
//! 保存 = 固定步进安全边界上对已提交状态的**单一时点**捕获：只读
//! 已提交事实、把进程句柄解析为快照局部标识与稳定标识。派生状态
//! （信号灯色、占用索引、profile 派生车长）与禁绑字段（句柄 / 槽位 /
//! generation / 密集序号）不入快照。编码阶段只读不可变捕获，把完整
//! `WorldConfig`（含 edge/conflict 两项路线 occurrence 容量）、绑定集与逻辑状态
//! 映射到 size-prefixed `LFRS`；不回读活动 world，也不推进游标。

use laneflow_runtime_snapshot_wire::generated::lane_flow::runtime_snapshot::v5 as wire;
use laneflow_runtime_snapshot_wire::runtime;
use laneflow_static_contract::StableId128 as ContractStableId128;
use laneflow_static_network::CanonicalNetworkOrigin;
use thiserror::Error;

use crate::{
    CommittedNetworkSource, ParkingBinding, ParkingTarget, TrafficWorld, VehicleStatus, WorldConfig,
};

/// LFRS 容器格式版本（快照合同 §4）。
pub const SNAPSHOT_FORMAT_VERSION: u32 = 5;
/// Runtime 逻辑状态形状轴（快照合同 §2 版本轴分离）。
pub const RUNTIME_STATE_VERSION: u16 = 5;

/// 快照局部标识的起点（1..=N 分配，0 保留为非法）。
const FIRST_SNAPSHOT_ID: u64 = 1;

/// 快照捕获失败（#532 capture 轴错误族；`SnapshotRestoreError` 的保存侧对偶）。
///
/// 捕获只读已提交状态；失败时世界无感知（不推进游标、不改状态）。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum SnapshotCaptureError {
    /// 捕获期容量预留失败（分配压力下失败关闭，旧世界原样继续）。
    #[error("快照捕获容量预留失败")]
    ReservationFailed,
    /// W4 已建立冲突状态，W5 的 wire/digest 尚未接入时拒绝丢失该状态。
    #[error("当前快照版本尚不能编码 Conflict authority")]
    ConflictStateUnsupported,
}

#[cfg(test)]
thread_local! {
    static SNAPSHOT_RESERVATIONS_BEFORE_FAILURE: core::cell::Cell<Option<usize>> =
        const { core::cell::Cell::new(None) };
}

#[cfg(test)]
struct SnapshotAllocationFailpointReset(Option<usize>);

#[cfg(test)]
impl Drop for SnapshotAllocationFailpointReset {
    fn drop(&mut self) {
        SNAPSHOT_RESERVATIONS_BEFORE_FAILURE.with(|remaining| remaining.set(self.0));
    }
}

/// 只供同线程单元测试确定性覆盖第 N 次快照轴预留失败（capture 与摘要共用
/// 一个计数器，注入点在各自模块映射为本轴错误族）。
#[cfg(test)]
pub(crate) fn with_snapshot_allocation_failure_after<T>(
    successful_reservations: usize,
    run: impl FnOnce() -> T,
) -> T {
    SNAPSHOT_RESERVATIONS_BEFORE_FAILURE.with(|remaining| {
        let _reset =
            SnapshotAllocationFailpointReset(remaining.replace(Some(successful_reservations)));
        run()
    })
}

/// 快照轴预留的注入检查：命中注入计数时返回真（不实际预留），
/// 否则递减计数。零增量预留不计入（与 staging 家族同语义）。
#[cfg(test)]
pub(crate) fn snapshot_reservation_injected_failure() -> bool {
    SNAPSHOT_RESERVATIONS_BEFORE_FAILURE.with(|remaining| match remaining.get() {
        Some(0) => true,
        Some(value) => {
            remaining.set(Some(value - 1));
            false
        }
        None => false,
    })
}

/// 快照捕获侧可失败精确预留。
fn capture_try_reserve_exact<T>(
    values: &mut Vec<T>,
    additional: usize,
) -> Result<(), SnapshotCaptureError> {
    if additional == 0 {
        return Ok(());
    }
    #[cfg(test)]
    if snapshot_reservation_injected_failure() {
        return Err(SnapshotCaptureError::ReservationFailed);
    }
    values
        .try_reserve_exact(additional)
        .map_err(|_| SnapshotCaptureError::ReservationFailed)
}

/// 句柄查表 HashMap 的可失败预留（同一注入点；`try_reserve` 按至少容量预留）。
fn capture_map_try_reserve<K, V, S>(
    map: &mut std::collections::HashMap<K, V, S>,
    capacity: usize,
) -> Result<(), SnapshotCaptureError>
where
    K: Eq + std::hash::Hash,
    S: std::hash::BuildHasher,
{
    if capacity == 0 {
        return Ok(());
    }
    #[cfg(test)]
    if snapshot_reservation_injected_failure() {
        return Err(SnapshotCaptureError::ReservationFailed);
    }
    map.try_reserve(capacity)
        .map_err(|_| SnapshotCaptureError::ReservationFailed)
}

/// 快照点已捕获的逻辑状态：编码无关的绑定集 + 全部每世界可变状态。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapturedSnapshot {
    /// 世界身份（快照局部）。
    pub(crate) world_id: u64,
    /// tick 游标。
    pub(crate) tick: u64,
    /// 时钟（`tick × fixed_delta_time_ms`，恢复侧核对）。
    pub(crate) time_ms: u64,
    /// 已应用输入命令计数。
    pub(crate) command_cursor: u64,
    /// 已提交切换事件游标（#513 切片 C 起为真实轴）。
    pub(crate) event_cursor: u64,
    /// 安装时冻结的世界配置。
    pub(crate) config: WorldConfig,
    /// 显式策略选择；规则内容由 LFCA origin 绑定。
    pub(crate) policy_selection: crate::WorldPolicySelection,
    /// 被绑定共享根的 LFCA origin。
    pub(crate) origin: CanonicalNetworkOrigin,
    /// 已提交路网来源。
    pub(crate) source: CommittedNetworkSource,
    /// 路线表（快照局部 ID 按 live 槽位序规范分配）。
    pub(crate) routes: Vec<CapturedRoute>,
    /// 车辆表（快照局部 ID 按 live 槽位序规范分配）。
    pub(crate) vehicles: Vec<CapturedVehicle>,
    /// live 顺序：`snapshot_vehicle_id` 的规范排序序列（实际更新顺序，
    /// 不是局部 ID 的自然序；恢复侧核对其为活跃车辆的精确排列）。
    pub(crate) live_order: Vec<u64>,
    /// 有 member 或历史 counter 的 WaitingZone 语义状态。
    pub(crate) waiting_zones: Vec<CapturedWaitingZoneState>,
}

/// 快照路线：局部 ID + 有序边稳定标识序列（允许重复边，ADR 0029 §6）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapturedRoute {
    /// 快照局部路线 ID（本快照内唯一）。
    pub(crate) snapshot_route_id: u64,
    /// 有序边稳定标识（替代 `LaneEdgeOrdinal`）。
    pub(crate) edges: Vec<ContractStableId128>,
}

/// 快照车辆：局部 ID + 局部路线引用 + 路线序列下标与一维运动状态。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapturedVehicle {
    /// 快照局部车辆 ID（本快照内唯一）。
    pub(crate) snapshot_vehicle_id: u64,
    /// 所属快照局部路线 ID。
    pub(crate) snapshot_route_id: u64,
    /// 路线序列下标（不是路网序号）。
    pub(crate) route_edge_index: u32,
    /// 当前边进度（毫米）。
    pub(crate) progress_mm: u32,
    /// 亚毫米余数（微米）。
    pub(crate) carry_um: u16,
    /// 速度（毫米每秒）。
    pub(crate) speed_mm_s: u32,
    /// 生命周期状态。
    pub(crate) status: VehicleStatus,
    /// profile 稳定标识。
    pub(crate) profile: ContractStableId128,
    /// 参与者类别稳定标识。
    pub(crate) class: ContractStableId128,
    /// tagged parking binding；`None` 表示未绑定。
    pub(crate) parking: Option<CapturedParkingBinding>,
    /// stateful maneuver traversal。
    pub(crate) maneuver_traversal: Option<CapturedManeuverTraversal>,
    /// WaitingZone semantic membership。
    pub(crate) waiting_membership: Option<CapturedWaitingMembership>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapturedManeuverTraversalPhase {
    PreGate,
    Committed,
    Waiting,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapturedManeuverTraversal {
    pub(crate) maneuver_occurrence_index: u32,
    pub(crate) maneuver_path: ContractStableId128,
    pub(crate) phase: CapturedManeuverTraversalPhase,
    pub(crate) phase_gate: ContractStableId128,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapturedWaitingMembership {
    pub(crate) waiting_zone: ContractStableId128,
    pub(crate) maneuver_occurrence_index: u32,
    pub(crate) entry_gate: ContractStableId128,
    pub(crate) release_gate: ContractStableId128,
    pub(crate) admission_sequence: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapturedWaitingZoneState {
    pub(crate) waiting_zone: ContractStableId128,
    pub(crate) occupancy: u32,
    pub(crate) next_admission_sequence: u64,
}

/// 快照中的 tagged parking target stable identity。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapturedParkingTarget {
    ExplicitSpace(ContractStableId128),
    VirtualPool(ContractStableId128),
}

/// virtual Reserved selector 的跨修订 semantic 形态。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapturedVirtualParkingEntry {
    pub(crate) lane_edge: ContractStableId128,
    pub(crate) progress_mm: u32,
}

/// 快照中的完整 parking binding。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapturedParkingBinding {
    Reserved {
        target: CapturedParkingTarget,
        entry_route_occurrence: u32,
        virtual_entry: Option<CapturedVirtualParkingEntry>,
    },
    Occupied {
        target: CapturedParkingTarget,
    },
}

impl CapturedRoute {
    /// 本快照内唯一的路线局部 ID。
    #[must_use]
    pub const fn snapshot_route_id(&self) -> u64 {
        self.snapshot_route_id
    }

    /// 路线的有序 LaneEdge 稳定标识序列。
    #[must_use]
    pub fn edges(&self) -> &[ContractStableId128] {
        &self.edges
    }
}

impl CapturedVehicle {
    /// 本快照内唯一的车辆局部 ID。
    #[must_use]
    pub const fn snapshot_vehicle_id(&self) -> u64 {
        self.snapshot_vehicle_id
    }

    /// 所属路线的快照局部 ID。
    #[must_use]
    pub const fn snapshot_route_id(&self) -> u64 {
        self.snapshot_route_id
    }

    /// 当前路线边下标。
    #[must_use]
    pub const fn route_edge_index(&self) -> u32 {
        self.route_edge_index
    }

    /// 当前边进度（毫米）。
    #[must_use]
    pub const fn progress_mm(&self) -> u32 {
        self.progress_mm
    }

    /// 亚毫米余数（微米）。
    #[must_use]
    pub const fn carry_um(&self) -> u16 {
        self.carry_um
    }

    /// 当前速度（毫米每秒）。
    #[must_use]
    pub const fn speed_mm_s(&self) -> u32 {
        self.speed_mm_s
    }

    /// 生命周期状态。
    #[must_use]
    pub const fn status(&self) -> VehicleStatus {
        self.status
    }

    /// 车辆 profile 稳定标识。
    #[must_use]
    pub const fn profile(&self) -> ContractStableId128 {
        self.profile
    }

    /// 参与者类别稳定标识。
    #[must_use]
    pub const fn class(&self) -> ContractStableId128 {
        self.class
    }

    /// tagged parking binding；未绑定时为 `None`。
    #[must_use]
    pub const fn parking_binding(&self) -> Option<CapturedParkingBinding> {
        self.parking
    }

    #[must_use]
    pub const fn maneuver_traversal(&self) -> Option<CapturedManeuverTraversal> {
        self.maneuver_traversal
    }

    #[must_use]
    pub const fn waiting_membership(&self) -> Option<CapturedWaitingMembership> {
        self.waiting_membership
    }
}

impl CapturedManeuverTraversal {
    #[must_use]
    pub const fn maneuver_occurrence_index(self) -> u32 {
        self.maneuver_occurrence_index
    }

    #[must_use]
    pub const fn maneuver_path(self) -> ContractStableId128 {
        self.maneuver_path
    }

    #[must_use]
    pub const fn phase(self) -> CapturedManeuverTraversalPhase {
        self.phase
    }

    #[must_use]
    pub const fn phase_gate(self) -> ContractStableId128 {
        self.phase_gate
    }
}

impl CapturedWaitingMembership {
    #[must_use]
    pub const fn waiting_zone(self) -> ContractStableId128 {
        self.waiting_zone
    }

    #[must_use]
    pub const fn maneuver_occurrence_index(self) -> u32 {
        self.maneuver_occurrence_index
    }

    #[must_use]
    pub const fn entry_gate(self) -> ContractStableId128 {
        self.entry_gate
    }

    #[must_use]
    pub const fn release_gate(self) -> ContractStableId128 {
        self.release_gate
    }

    #[must_use]
    pub const fn admission_sequence(self) -> u64 {
        self.admission_sequence
    }
}

impl CapturedWaitingZoneState {
    #[must_use]
    pub const fn waiting_zone(self) -> ContractStableId128 {
        self.waiting_zone
    }

    #[must_use]
    pub const fn occupancy(self) -> u32 {
        self.occupancy
    }

    #[must_use]
    pub const fn next_admission_sequence(self) -> u64 {
        self.next_admission_sequence
    }
}

/// 把不可变快照点编码为 size-prefixed `LFRS` v5。
///
/// 捕获与编码分离：调用方可先在固定步进安全边界调用
/// [`TrafficWorld::capture_snapshot`]，再把本函数放到后台线程。编码只映射已捕获
/// 事实，不重新读取活动 world；输出始终携带 `LFRS` file identifier。
#[must_use]
pub fn encode_lfrs(snapshot: &CapturedSnapshot) -> Vec<u8> {
    let mut fbb = runtime::FlatBufferBuilder::new();

    let world_config = wire::WorldConfigBinding::create(
        &mut fbb,
        &wire::WorldConfigBindingArgs {
            vehicle_capacity: snapshot.config.vehicle_capacity(),
            route_capacity: snapshot.config.route_capacity(),
            route_edge_occurrence_capacity: snapshot.config.route_edge_occurrence_capacity(),
            route_conflict_occurrence_capacity: snapshot
                .config
                .route_conflict_occurrence_capacity(),
            worker_count: snapshot.config.worker_count(),
            fixed_delta_time_ms: snapshot.config.fixed_delta_time_ms(),
        },
    );

    let route_offsets = snapshot
        .routes
        .iter()
        .map(|route| {
            let edges = route
                .edges
                .iter()
                .map(|stable_id| wire::StableId128::new(stable_id.as_bytes()))
                .collect::<Vec<_>>();
            let edges = fbb.create_vector(&edges);
            wire::SnapshotRoute::create(
                &mut fbb,
                &wire::SnapshotRouteArgs {
                    snapshot_route_id: route.snapshot_route_id,
                    edges: Some(edges),
                },
            )
        })
        .collect::<Vec<_>>();
    let routes = fbb.create_vector(&route_offsets);

    let vehicle_offsets = snapshot
        .vehicles
        .iter()
        .map(|vehicle| {
            let profile = wire::StableId128::new(vehicle.profile.as_bytes());
            let class = wire::StableId128::new(vehicle.class.as_bytes());
            let parking = vehicle.parking.map(|binding| {
                let (state, target, target_kind, entry_occurrence, virtual_entry) = match binding {
                    CapturedParkingBinding::Reserved {
                        target,
                        entry_route_occurrence,
                        virtual_entry,
                    } => (
                        wire::ParkingBindingStateKind::Reserved,
                        target,
                        match target {
                            CapturedParkingTarget::ExplicitSpace(_) => {
                                wire::ParkingTargetKind::ExplicitSpace
                            }
                            CapturedParkingTarget::VirtualPool(_) => {
                                wire::ParkingTargetKind::VirtualPool
                            }
                        },
                        entry_route_occurrence,
                        virtual_entry,
                    ),
                    CapturedParkingBinding::Occupied { target } => (
                        wire::ParkingBindingStateKind::Occupied,
                        target,
                        match target {
                            CapturedParkingTarget::ExplicitSpace(_) => {
                                wire::ParkingTargetKind::ExplicitSpace
                            }
                            CapturedParkingTarget::VirtualPool(_) => {
                                wire::ParkingTargetKind::VirtualPool
                            }
                        },
                        0,
                        None,
                    ),
                };
                let target = match target {
                    CapturedParkingTarget::ExplicitSpace(stable)
                    | CapturedParkingTarget::VirtualPool(stable) => {
                        wire::StableId128::new(stable.as_bytes())
                    }
                };
                let virtual_entry_edge =
                    virtual_entry.map(|entry| wire::StableId128::new(entry.lane_edge.as_bytes()));
                wire::ParkingBinding::create(
                    &mut fbb,
                    &wire::ParkingBindingArgs {
                        state,
                        target_kind,
                        target: Some(&target),
                        entry_route_occurrence: entry_occurrence,
                        virtual_entry_edge: virtual_entry_edge.as_ref(),
                        virtual_entry_progress_mm: virtual_entry
                            .map_or(0, |entry| entry.progress_mm),
                    },
                )
            });
            let maneuver_traversal = vehicle.maneuver_traversal.map(|traversal| {
                let maneuver_path = wire::StableId128::new(traversal.maneuver_path.as_bytes());
                let phase_gate = wire::StableId128::new(traversal.phase_gate.as_bytes());
                wire::ManeuverTraversalBinding::create(
                    &mut fbb,
                    &wire::ManeuverTraversalBindingArgs {
                        maneuver_occurrence_index: traversal.maneuver_occurrence_index,
                        maneuver_path: Some(&maneuver_path),
                        phase: match traversal.phase {
                            CapturedManeuverTraversalPhase::PreGate => {
                                wire::ManeuverTraversalPhaseKind::PreGate
                            }
                            CapturedManeuverTraversalPhase::Committed => {
                                wire::ManeuverTraversalPhaseKind::Committed
                            }
                            CapturedManeuverTraversalPhase::Waiting => {
                                wire::ManeuverTraversalPhaseKind::Waiting
                            }
                        },
                        phase_gate: Some(&phase_gate),
                    },
                )
            });
            let waiting_membership = vehicle.waiting_membership.map(|membership| {
                let waiting_zone = wire::StableId128::new(membership.waiting_zone.as_bytes());
                let entry_gate = wire::StableId128::new(membership.entry_gate.as_bytes());
                let release_gate = wire::StableId128::new(membership.release_gate.as_bytes());
                wire::WaitingMembershipBinding::create(
                    &mut fbb,
                    &wire::WaitingMembershipBindingArgs {
                        waiting_zone: Some(&waiting_zone),
                        maneuver_occurrence_index: membership.maneuver_occurrence_index,
                        entry_gate: Some(&entry_gate),
                        release_gate: Some(&release_gate),
                        admission_sequence: membership.admission_sequence,
                    },
                )
            });
            wire::SnapshotVehicle::create(
                &mut fbb,
                &wire::SnapshotVehicleArgs {
                    snapshot_vehicle_id: vehicle.snapshot_vehicle_id,
                    snapshot_route_id: vehicle.snapshot_route_id,
                    route_edge_index: vehicle.route_edge_index,
                    progress_mm: vehicle.progress_mm,
                    carry_um: vehicle.carry_um,
                    speed_mm_s: vehicle.speed_mm_s,
                    status: encode_vehicle_status(vehicle.status),
                    profile: Some(&profile),
                    class: Some(&class),
                    parking,
                    maneuver_traversal,
                    waiting_membership,
                },
            )
        })
        .collect::<Vec<_>>();
    let vehicles = fbb.create_vector(&vehicle_offsets);
    let live_order = fbb.create_vector(&snapshot.live_order);
    let waiting_zone_offsets = snapshot
        .waiting_zones
        .iter()
        .map(|state| {
            let waiting_zone = wire::StableId128::new(state.waiting_zone.as_bytes());
            wire::WaitingZoneState::create(
                &mut fbb,
                &wire::WaitingZoneStateArgs {
                    waiting_zone: Some(&waiting_zone),
                    occupancy: state.occupancy,
                    next_admission_sequence: state.next_admission_sequence,
                },
            )
        })
        .collect::<Vec<_>>();
    let waiting_zones = fbb.create_vector(&waiting_zone_offsets);

    let (source_kind, source_published) = match &snapshot.source {
        CommittedNetworkSource::Published { reference } => {
            let asset_key = fbb.create_string(reference.asset_key());
            let artifact_digest =
                wire::Digest256::new(reference.canonical_artifact_digest().as_bytes());
            let network_revision =
                wire::Digest256::new(reference.network_revision().as_digest().as_bytes());
            let published = wire::PublishedSourceBinding::create(
                &mut fbb,
                &wire::PublishedSourceBindingArgs {
                    asset_key: Some(asset_key),
                    artifact_digest: Some(&artifact_digest),
                    artifact_byte_length: reference.canonical_artifact_byte_length().get(),
                    network_revision: Some(&network_revision),
                },
            );
            (wire::SourceKind::Published, Some(published))
        }
    };

    let origin = snapshot.origin;
    let network_revision = wire::Digest256::new(origin.network_revision().as_digest().as_bytes());
    let artifact_digest = wire::Digest256::new(origin.canonical_artifact_digest().as_bytes());
    let contracts = origin.static_contract_versions();
    let contract_versions = wire::StaticContractVersionSet::new(
        contracts.canonical_format_version(),
        contracts.identity_encoding_version(),
        contracts.identity_registry_revision(),
        contracts.network_revision_derivation_version(),
        contracts.constraint_contract_version(),
        contracts.static_execution_contract_version(),
    );
    let (selection, policy) = match snapshot.policy_selection {
        crate::WorldPolicySelection::NotRequired => {
            (wire::WorldPolicySelectionKind::NotRequired, None)
        }
        crate::WorldPolicySelection::Pinned(pin) => (
            wire::WorldPolicySelectionKind::Pinned,
            Some(wire::StableId128::new(pin.policy.as_untyped().as_bytes())),
        ),
    };
    let world_policy = wire::WorldPolicyBinding::create(
        &mut fbb,
        &wire::WorldPolicyBindingArgs {
            selection,
            policy: policy.as_ref(),
        },
    );
    let root = wire::RuntimeSnapshot::create(
        &mut fbb,
        &wire::RuntimeSnapshotArgs {
            format_version: SNAPSHOT_FORMAT_VERSION,
            runtime_state_version: RUNTIME_STATE_VERSION,
            world_id: snapshot.world_id,
            tick: snapshot.tick,
            time_ms: snapshot.time_ms,
            command_cursor: snapshot.command_cursor,
            event_cursor: snapshot.event_cursor,
            world_config: Some(world_config),
            network_revision: Some(&network_revision),
            lfca_artifact_digest: Some(&artifact_digest),
            lfca_artifact_byte_length: origin.canonical_artifact_byte_length().get(),
            static_contract_versions: Some(&contract_versions),
            source_kind,
            source_published,
            routes: Some(routes),
            vehicles: Some(vehicles),
            live_order: Some(live_order),
            waiting_zones: Some(waiting_zones),
            world_policy: Some(world_policy),
        },
    );
    wire::finish_size_prefixed_runtime_snapshot_buffer(&mut fbb, root);
    fbb.finished_data().to_vec()
}

const fn encode_vehicle_status(status: VehicleStatus) -> wire::VehicleStatusKind {
    match status {
        VehicleStatus::Active => wire::VehicleStatusKind::Active,
        VehicleStatus::Parked => wire::VehicleStatusKind::Parked,
        VehicleStatus::Completed => wire::VehicleStatusKind::Completed,
    }
}

impl CapturedSnapshot {
    /// 保存时绑定的策略身份。
    #[must_use]
    pub const fn policy_selection(&self) -> crate::WorldPolicySelection {
        self.policy_selection
    }

    /// 世界身份。
    #[must_use]
    pub const fn world_id(&self) -> u64 {
        self.world_id
    }

    /// tick 游标。
    #[must_use]
    pub const fn tick(&self) -> u64 {
        self.tick
    }

    /// 时钟（毫秒）。
    #[must_use]
    pub const fn time_ms(&self) -> u64 {
        self.time_ms
    }

    /// 已应用输入命令计数。
    #[must_use]
    pub const fn command_cursor(&self) -> u64 {
        self.command_cursor
    }

    /// 已提交切换事件游标（#513 切片 C 起为真实轴）。
    #[must_use]
    pub const fn event_cursor(&self) -> u64 {
        self.event_cursor
    }

    /// 安装时冻结的世界配置。
    #[must_use]
    pub const fn config(&self) -> WorldConfig {
        self.config
    }

    /// 被绑定共享根的 LFCA origin。
    #[must_use]
    pub const fn origin(&self) -> CanonicalNetworkOrigin {
        self.origin
    }

    /// 已提交路网来源。
    #[must_use]
    pub const fn source(&self) -> &CommittedNetworkSource {
        &self.source
    }

    /// 快照路线表（按局部 ID 升序）。
    #[must_use]
    pub fn routes(&self) -> &[CapturedRoute] {
        &self.routes
    }

    /// 快照车辆表（按局部 ID 升序）。
    #[must_use]
    pub fn vehicles(&self) -> &[CapturedVehicle] {
        &self.vehicles
    }

    /// live 顺序（`snapshot_vehicle_id` 序列，实际更新顺序）。
    #[must_use]
    pub fn live_order(&self) -> &[u64] {
        &self.live_order
    }

    #[must_use]
    pub fn waiting_zones(&self) -> &[CapturedWaitingZoneState] {
        &self.waiting_zones
    }
}

impl TrafficWorld {
    /// 在固定步进安全边界捕获快照点（快照合同 §5 单一时点）。
    ///
    /// 只读已提交状态，不改变世界、不推进游标；全部容量按计数可失败预留，
    /// 预留失败即 [`SnapshotCaptureError`]，世界无感知、宿主可直接重试。
    /// 局部标识分配规范：路线按 live 槽位序取 `1..=N`，车辆按 live 槽位序
    /// 取 `1..=M`；`live_order` 保存实际更新顺序，与局部 ID 的自然序解耦。
    pub fn capture_snapshot(&self) -> Result<CapturedSnapshot, SnapshotCaptureError> {
        if !self.conflict_arbiter.is_empty()
            || self.conflict_eligibility.iter().any(Option::is_some)
            || self.vehicles.iter().any(|slot| {
                slot.state
                    .is_some_and(|state| state.conflict_reservation().is_some())
            })
        {
            return Err(SnapshotCaptureError::ConflictStateUnsupported);
        }
        let identity = self.revision.identity();

        // 路线：live 槽位序枚举，序号→稳定标识经 SharedIdentityIndex。
        let route_capacity = usize::try_from(self.live_route_count).unwrap_or(0);
        let mut routes = Vec::new();
        let mut route_ids: Vec<(u32, u32, u64)> = Vec::new();
        capture_try_reserve_exact(&mut routes, route_capacity)?;
        capture_try_reserve_exact(&mut route_ids, route_capacity)?;
        for (slot_index, slot) in self.routes.iter().enumerate() {
            if slot.compiled.is_none() {
                continue;
            }
            let handle = crate::RouteHandle::new(
                u32::try_from(slot_index).expect("route index fits u32"),
                slot.generation,
            );
            let snapshot_route_id = FIRST_SNAPSHOT_ID + routes.len() as u64;
            route_ids.push((slot.generation, handle.index(), snapshot_route_id));
            let edge_ordinals = self
                .route_edges(handle)
                .expect("live route slot resolves edges");
            let mut edges: Vec<ContractStableId128> = Vec::new();
            capture_try_reserve_exact(&mut edges, edge_ordinals.len())?;
            for ordinal in edge_ordinals {
                edges.push(
                    identity
                        .stable_id(*ordinal)
                        .map(|stable| *stable.as_untyped())
                        .expect("live edge ordinal resolves to stable id"),
                );
            }
            routes.push(CapturedRoute {
                snapshot_route_id,
                edges,
            });
        }
        let mut route_id_by_handle: std::collections::HashMap<(u32, u32), u64> =
            std::collections::HashMap::new();
        capture_map_try_reserve(&mut route_id_by_handle, route_ids.len())?;
        for &(generation, index, snapshot_route_id) in &route_ids {
            route_id_by_handle.insert((generation, index), snapshot_route_id);
        }
        let route_id_for = |generation: u32, index: u32| -> u64 {
            *route_id_by_handle
                .get(&(generation, index))
                .expect("live vehicle route resolves to snapshot route id")
        };

        // 车辆：live 槽位序枚举，profile/class/parking target 解析为稳定标识。
        let mut vehicles = Vec::new();
        let mut vehicle_ids: Vec<(u32, u32, u64)> = Vec::new();
        capture_try_reserve_exact(&mut vehicles, self.live_order.len())?;
        capture_try_reserve_exact(&mut vehicle_ids, self.live_order.len())?;
        for (slot_index, slot) in self.vehicles.iter().enumerate() {
            let Some(state) = slot.state.as_ref() else {
                continue;
            };
            let snapshot_vehicle_id = FIRST_SNAPSHOT_ID + vehicles.len() as u64;
            vehicle_ids.push((
                slot.generation,
                u32::try_from(slot_index).expect("vehicle index fits u32"),
                snapshot_vehicle_id,
            ));
            let parking = self.parking.binding(state.handle).map(|binding| {
                let captured_target = |target: ParkingTarget| match target {
                    ParkingTarget::ExplicitSpace(space) => CapturedParkingTarget::ExplicitSpace(
                        *identity
                            .stable_id(space)
                            .expect("parking space ordinal resolves to stable id")
                            .as_untyped(),
                    ),
                    ParkingTarget::VirtualPool(facility) => CapturedParkingTarget::VirtualPool(
                        *identity
                            .stable_id(facility)
                            .expect("parking facility ordinal resolves to stable id")
                            .as_untyped(),
                    ),
                };
                match binding {
                    ParkingBinding::Reserved(reservation) => {
                        let virtual_entry = match reservation.target() {
                            ParkingTarget::ExplicitSpace(_) => None,
                            ParkingTarget::VirtualPool(facility) => {
                                let selector = reservation
                                    .virtual_entry_selector()
                                    .expect("virtual reservation has selector");
                                let facility_view = self
                                    .revision
                                    .traffic()
                                    .relations()
                                    .parking_facility(facility)
                                    .expect("bound parking facility exists");
                                let anchor = facility_view
                                    .virtual_entries()
                                    .get(selector.index())
                                    .expect("bound virtual selector exists");
                                Some(CapturedVirtualParkingEntry {
                                    lane_edge: *identity
                                        .stable_id(anchor.lane_edge())
                                        .expect("parking anchor edge resolves to stable id")
                                        .as_untyped(),
                                    progress_mm: anchor.progress_mm(),
                                })
                            }
                        };
                        CapturedParkingBinding::Reserved {
                            target: captured_target(reservation.target()),
                            entry_route_occurrence: reservation.entry_route_occurrence(),
                            virtual_entry,
                        }
                    }
                    ParkingBinding::Occupied(target) => CapturedParkingBinding::Occupied {
                        target: captured_target(target),
                    },
                }
            });
            let maneuver_traversal = state.maneuver_traversal.map(|traversal| {
                let compiled = self
                    .compiled_route(state.route)
                    .expect("live traversal route exists");
                let maneuver = compiled
                    .maneuvers
                    .get(traversal.maneuver_occurrence_index as usize)
                    .expect("live traversal occurrence exists");
                let (phase, phase_hop) = match traversal.phase {
                    crate::ManeuverTraversalPhase::PreGate { next_gate_hop } => {
                        (CapturedManeuverTraversalPhase::PreGate, next_gate_hop)
                    }
                    crate::ManeuverTraversalPhase::Committed {
                        last_crossed_gate_hop,
                    } => (
                        CapturedManeuverTraversalPhase::Committed,
                        last_crossed_gate_hop,
                    ),
                    crate::ManeuverTraversalPhase::Waiting { release_gate_hop } => {
                        (CapturedManeuverTraversalPhase::Waiting, release_gate_hop)
                    }
                    crate::ManeuverTraversalPhase::Clearing { .. } => {
                        unreachable!("Conflict capture is rejected before row construction")
                    }
                };
                let gate = compiled
                    .hop_gate
                    .get(phase_hop as usize)
                    .copied()
                    .flatten()
                    .expect("traversal phase hop resolves Gate");
                CapturedManeuverTraversal {
                    maneuver_occurrence_index: traversal.maneuver_occurrence_index,
                    maneuver_path: *identity
                        .stable_id(maneuver.path)
                        .expect("maneuver path resolves stable id")
                        .as_untyped(),
                    phase,
                    phase_gate: *identity
                        .stable_id(gate)
                        .expect("maneuver Gate resolves stable id")
                        .as_untyped(),
                }
            });
            let waiting_membership = state.waiting_membership.map(|membership| {
                let traversal = state
                    .maneuver_traversal
                    .expect("Waiting membership has traversal");
                let compiled = self
                    .compiled_route(state.route)
                    .expect("live membership route exists");
                let occurrence = compiled
                    .waiting
                    .iter()
                    .find(|occurrence| {
                        occurrence.maneuver_index == traversal.maneuver_occurrence_index
                            && occurrence.zone == membership.waiting_zone
                            && occurrence.release_hop == membership.release_hop
                    })
                    .expect("Waiting membership resolves occurrence");
                let entry_gate = compiled.hop_gate[occurrence.entry_hop as usize]
                    .expect("Waiting entry hop resolves Gate");
                let release_gate = compiled.hop_gate[occurrence.release_hop as usize]
                    .expect("Waiting release hop resolves Gate");
                CapturedWaitingMembership {
                    waiting_zone: *identity
                        .stable_id(membership.waiting_zone)
                        .expect("WaitingZone resolves stable id")
                        .as_untyped(),
                    maneuver_occurrence_index: traversal.maneuver_occurrence_index,
                    entry_gate: *identity
                        .stable_id(entry_gate)
                        .expect("Waiting entry Gate resolves stable id")
                        .as_untyped(),
                    release_gate: *identity
                        .stable_id(release_gate)
                        .expect("Waiting release Gate resolves stable id")
                        .as_untyped(),
                    admission_sequence: membership.admission_sequence,
                }
            });
            vehicles.push(CapturedVehicle {
                snapshot_vehicle_id,
                snapshot_route_id: route_id_for(state.route.generation(), state.route.index()),
                route_edge_index: state.route_edge_index,
                progress_mm: state.progress_mm,
                carry_um: state.carry_um,
                speed_mm_s: state.speed_mm_s,
                status: state.status,
                profile: *identity
                    .stable_id(state.profile)
                    .expect("live profile ordinal resolves to stable id")
                    .as_untyped(),
                class: *identity
                    .stable_id(state.class)
                    .expect("live class ordinal resolves to stable id")
                    .as_untyped(),
                parking,
                maneuver_traversal,
                waiting_membership,
            });
        }
        let mut vehicle_id_by_handle: std::collections::HashMap<(u32, u32), u64> =
            std::collections::HashMap::new();
        capture_map_try_reserve(&mut vehicle_id_by_handle, vehicle_ids.len())?;
        for &(generation, index, snapshot_vehicle_id) in &vehicle_ids {
            vehicle_id_by_handle.insert((generation, index), snapshot_vehicle_id);
        }
        let mut live_order = Vec::new();
        capture_try_reserve_exact(&mut live_order, self.live_order.len())?;
        for handle in &self.live_order {
            live_order.push(
                *vehicle_id_by_handle
                    .get(&(handle.generation(), handle.index()))
                    .expect("live order handle resolves to snapshot vehicle id"),
            );
        }

        let waiting_state_count = self
            .waiting_zones
            .iter()
            .filter(|state| state.occupancy != 0 || state.next_admission_sequence != 0)
            .count();
        let mut waiting_zones = Vec::new();
        capture_try_reserve_exact(&mut waiting_zones, waiting_state_count)?;
        for (index, state) in self.waiting_zones.iter().copied().enumerate() {
            if state.occupancy == 0 && state.next_admission_sequence == 0 {
                continue;
            }
            let zone = laneflow_static_contract::WaitingZoneOrdinal::from_raw(
                u32::try_from(index).expect("WaitingZone index fits u32"),
            );
            waiting_zones.push(CapturedWaitingZoneState {
                waiting_zone: *identity
                    .stable_id(zone)
                    .expect("WaitingZone resolves stable id")
                    .as_untyped(),
                occupancy: state.occupancy,
                next_admission_sequence: state.next_admission_sequence,
            });
        }

        Ok(CapturedSnapshot {
            world_id: self.world_id,
            tick: self.tick_index,
            time_ms: self.time_ms,
            command_cursor: self.command_cursor,
            event_cursor: self.event_cursor,
            config: self.config,
            policy_selection: self.policy_selection(),
            origin: *self.revision.canonical_origin(),
            source: self
                .source
                .try_clone()
                .map_err(|_| SnapshotCaptureError::ReservationFailed)?,
            routes,
            vehicles,
            live_order,
            waiting_zones,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cutover::tests::transaction_tests::world_with_vehicle;
    use crate::{ParkedVehicleSpawnInput, ParkingTarget, RouteRegisterInput, TickInput};

    #[test]
    fn capture_binds_cursors_config_and_origin() {
        let (world, _, _) = world_with_vehicle(true);
        let snapshot = world.capture_snapshot().expect("capture");
        assert_eq!(snapshot.world_id(), 1);
        assert_eq!(snapshot.tick(), 0);
        assert_eq!(snapshot.time_ms(), 0);
        assert_eq!(snapshot.command_cursor(), 2);
        assert_eq!(snapshot.event_cursor(), 0);
        assert_eq!(snapshot.config(), world.config());
        assert_eq!(snapshot.origin(), *world.revision().canonical_origin());
        assert_eq!(snapshot.source(), world.committed_source());
    }

    #[test]
    fn capture_resolves_routes_vehicles_and_live_order() {
        let (mut world, route, vehicle) = world_with_vehicle(true);
        world.step(TickInput::new(100)).expect("step");
        // parked spawn 不占车道，因此可与现有 Active 的 route cursor 相同。
        let space = laneflow_static_contract::ParkingSpaceOrdinal::from_raw(0);
        let vehicle_2 = world
            .spawn_parked_vehicle(
                ParkedVehicleSpawnInput::new(
                    laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    1_000,
                ),
                ParkingTarget::ExplicitSpace(space),
            )
            .expect("spawn parked")
            .vehicle;

        let snapshot = world.capture_snapshot().expect("capture");
        // 路线：单条，局部 ID 1，边序解析为稳定标识。
        assert_eq!(snapshot.routes().len(), 1);
        assert_eq!(snapshot.routes()[0].snapshot_route_id, 1);
        let revision = world.revision();
        let identity = revision.identity();
        let expected_edges: Vec<ContractStableId128> = world
            .route_edges(route)
            .expect("route")
            .iter()
            .map(|ordinal| *identity.stable_id(*ordinal).expect("edge").as_untyped())
            .collect();
        assert_eq!(snapshot.routes()[0].edges, expected_edges);

        // 车辆：两辆，局部 ID 按槽位序 1、2；停车绑定在 parked spawn 上。
        assert_eq!(snapshot.vehicles().len(), 2);
        let [first, second] = snapshot.vehicles() else {
            unreachable!("两辆车");
        };
        assert_eq!(first.snapshot_vehicle_id, 1);
        assert_eq!(second.snapshot_vehicle_id, 2);
        assert_eq!(first.snapshot_route_id, 1);
        assert_eq!(second.snapshot_route_id, 1);
        assert!(first.parking.is_none());
        assert!(second.parking.is_some());
        assert_eq!(first.status, VehicleStatus::Active);
        assert_eq!(second.status, VehicleStatus::Parked);
        assert_eq!(
            second.parking,
            Some(CapturedParkingBinding::Occupied {
                target: CapturedParkingTarget::ExplicitSpace(
                    *identity.stable_id(space).expect("space").as_untyped(),
                ),
            })
        );
        let state = world.vehicle(vehicle).expect("vehicle");
        assert_eq!(first.route_edge_index, state.route_edge_index());
        assert_eq!(first.progress_mm, state.progress_mm());
        assert_eq!(first.speed_mm_s, state.speed_mm_s());
        assert_eq!(world.vehicle(vehicle_2).expect("parked").speed_mm_s(), 0);

        // live 顺序 = 实际更新顺序（先 1 后 2），不是槽位自然序的重复声明。
        assert_eq!(snapshot.live_order(), &[1, 2]);
    }

    #[test]
    fn capture_is_deterministic_and_side_effect_free() {
        let (mut world, _, _) = world_with_vehicle(true);
        world.step(TickInput::new(100)).expect("step");
        let before_cursor = world.command_cursor();
        let first = world.capture_snapshot().expect("capture");
        let second = world.capture_snapshot().expect("capture");
        assert_eq!(first, second);
        assert_eq!(world.command_cursor(), before_cursor);
        // 捕获后世界照常步进：单一时点捕获不持借用、不改状态。
        world.step(TickInput::new(100)).expect("step after capture");
        assert_eq!(world.command_cursor(), before_cursor);
    }

    #[test]
    fn capture_reservation_failure_fails_closed_and_is_retryable() {
        // #532：捕获的每个预留注入点都失败关闭——世界无感知，清点后
        // 重试得到同一快照（save 路径的失败关闭由 capture 侧承载）。
        let (mut world, _, _) = world_with_vehicle(true);
        world.step(TickInput::new(100)).expect("step");
        let baseline = world.capture_snapshot().expect("baseline capture");
        let baseline_bytes = encode_lfrs(&baseline);
        let before_cursor = world.command_cursor();
        for fail_after in 0..9 {
            let failed =
                with_snapshot_allocation_failure_after(fail_after, || world.capture_snapshot());
            assert_eq!(
                failed,
                Err(SnapshotCaptureError::ReservationFailed),
                "fail_after={fail_after}"
            );
            assert_eq!(world.command_cursor(), before_cursor);
        }
        let retried = world.capture_snapshot().expect("retry after clearing");
        assert_eq!(retried, baseline);
        assert_eq!(encode_lfrs(&retried), baseline_bytes);
    }

    #[test]
    fn encode_lfrs_maps_the_complete_captured_binding_and_state() {
        let (mut world, route, vehicle) = world_with_vehicle(true);
        world.step(TickInput::new(100)).expect("step");
        let space = laneflow_static_contract::ParkingSpaceOrdinal::from_raw(0);
        world
            .spawn_parked_vehicle(
                ParkedVehicleSpawnInput::new(
                    laneflow_static_contract::VehicleProfileOrdinal::from_raw(0),
                    route,
                    0,
                    1_000,
                ),
                ParkingTarget::ExplicitSpace(space),
            )
            .expect("spawn parked");
        let snapshot = world.capture_snapshot().expect("capture");

        let bytes = encode_lfrs(&snapshot);
        assert_eq!(bytes, encode_lfrs(&snapshot));
        assert!(wire::runtime_snapshot_size_prefixed_buffer_has_identifier(
            &bytes
        ));
        let prefixed_len = u32::from_le_bytes(bytes[..4].try_into().expect("size prefix"));
        assert_eq!(
            usize::try_from(prefixed_len).expect("usize"),
            bytes.len() - 4
        );
        let root = wire::size_prefixed_root_as_runtime_snapshot(&bytes).expect("verified LFRS");

        assert_eq!(root.format_version(), SNAPSHOT_FORMAT_VERSION);
        assert_eq!(root.runtime_state_version(), RUNTIME_STATE_VERSION);
        assert_eq!(root.world_id(), snapshot.world_id());
        assert_eq!(root.tick(), snapshot.tick());
        assert_eq!(root.time_ms(), snapshot.time_ms());
        assert_eq!(root.command_cursor(), snapshot.command_cursor());
        assert_eq!(root.event_cursor(), snapshot.event_cursor());

        let config = root.world_config();
        assert_eq!(
            config.vehicle_capacity(),
            snapshot.config().vehicle_capacity()
        );
        assert_eq!(config.route_capacity(), snapshot.config().route_capacity());
        assert_eq!(
            config.route_edge_occurrence_capacity(),
            snapshot.config().route_edge_occurrence_capacity()
        );
        assert_eq!(
            config.route_conflict_occurrence_capacity(),
            snapshot.config().route_conflict_occurrence_capacity()
        );
        assert_eq!(config.worker_count(), snapshot.config().worker_count());
        assert_eq!(
            config.fixed_delta_time_ms(),
            snapshot.config().fixed_delta_time_ms()
        );

        let origin = snapshot.origin();
        assert_eq!(
            root.network_revision().expect("network revision").0,
            *origin.network_revision().as_digest().as_bytes()
        );
        assert_eq!(
            root.lfca_artifact_digest().expect("artifact digest").0,
            *origin.canonical_artifact_digest().as_bytes()
        );
        assert_eq!(
            root.lfca_artifact_byte_length(),
            origin.canonical_artifact_byte_length().get()
        );
        let expected_contracts = origin.static_contract_versions();
        let contracts = root.static_contract_versions().expect("contract versions");
        assert_eq!(
            contracts.canonical_format_version(),
            expected_contracts.canonical_format_version()
        );
        assert_eq!(
            contracts.identity_encoding_version(),
            expected_contracts.identity_encoding_version()
        );
        assert_eq!(
            contracts.identity_registry_revision(),
            expected_contracts.identity_registry_revision()
        );
        assert_eq!(
            contracts.network_revision_derivation_version(),
            expected_contracts.network_revision_derivation_version()
        );
        assert_eq!(
            contracts.constraint_contract_version(),
            expected_contracts.constraint_contract_version()
        );
        assert_eq!(
            contracts.static_execution_contract_version(),
            expected_contracts.static_execution_contract_version()
        );

        let CommittedNetworkSource::Published { reference } = snapshot.source();
        assert_eq!(root.source_kind(), wire::SourceKind::Published);
        let published = root.source_published().expect("published source");
        assert_eq!(published.asset_key(), reference.asset_key());
        assert_eq!(
            published.artifact_digest().expect("source digest").0,
            *reference.canonical_artifact_digest().as_bytes()
        );
        assert_eq!(
            published.artifact_byte_length(),
            reference.canonical_artifact_byte_length().get()
        );
        assert_eq!(
            published.network_revision().expect("source revision").0,
            *reference.network_revision().as_digest().as_bytes()
        );

        let routes = root.routes();
        assert_eq!(routes.len(), snapshot.routes().len());
        for (index, captured_route) in snapshot.routes().iter().enumerate() {
            let route = routes.get(index);
            assert_eq!(route.snapshot_route_id(), captured_route.snapshot_route_id);
            assert_eq!(route.edges().len(), captured_route.edges.len());
            for (wire_edge, captured_edge) in route.edges().iter().zip(captured_route.edges.iter())
            {
                assert_eq!(wire_edge.0, *captured_edge.as_bytes());
            }
        }

        let vehicles = root.vehicles();
        assert_eq!(vehicles.len(), snapshot.vehicles().len());
        for (index, captured) in snapshot.vehicles().iter().enumerate() {
            let vehicle = vehicles.get(index);
            assert_eq!(vehicle.snapshot_vehicle_id(), captured.snapshot_vehicle_id);
            assert_eq!(vehicle.snapshot_route_id(), captured.snapshot_route_id);
            assert_eq!(vehicle.route_edge_index(), captured.route_edge_index);
            assert_eq!(vehicle.progress_mm(), captured.progress_mm);
            assert_eq!(vehicle.carry_um(), captured.carry_um);
            assert_eq!(vehicle.speed_mm_s(), captured.speed_mm_s);
            assert_eq!(vehicle.status(), encode_vehicle_status(captured.status));
            assert_eq!(
                vehicle.profile().expect("profile stable id").0,
                *captured.profile.as_bytes()
            );
            assert_eq!(
                vehicle.class().expect("class stable id").0,
                *captured.class.as_bytes()
            );
            match captured.parking {
                None => assert!(vehicle.parking().is_none()),
                Some(CapturedParkingBinding::Occupied { target }) => {
                    let parking = vehicle.parking().expect("occupied parking binding");
                    assert_eq!(parking.state(), wire::ParkingBindingStateKind::Occupied);
                    let (expected_kind, expected_target) = match target {
                        CapturedParkingTarget::ExplicitSpace(stable) => {
                            (wire::ParkingTargetKind::ExplicitSpace, stable)
                        }
                        CapturedParkingTarget::VirtualPool(stable) => {
                            (wire::ParkingTargetKind::VirtualPool, stable)
                        }
                    };
                    assert_eq!(parking.target_kind(), expected_kind);
                    assert_eq!(
                        parking.target().expect("parking target").0,
                        *expected_target.as_bytes()
                    );
                }
                Some(CapturedParkingBinding::Reserved { .. }) => {
                    panic!("fixture vehicle is occupied, not reserved")
                }
            }
        }
        assert_eq!(vehicles.get(0).status(), wire::VehicleStatusKind::Active);
        assert_eq!(vehicles.get(1).status(), wire::VehicleStatusKind::Parked);
        assert_eq!(
            snapshot.live_order(),
            root.live_order().iter().collect::<Vec<_>>().as_slice()
        );
        assert_eq!(
            world.vehicle(vehicle).expect("active").status(),
            VehicleStatus::Active
        );
    }

    #[test]
    fn encode_lfrs_keeps_required_empty_state_vectors() {
        let root_revision = crate::cutover::tests::transaction_tests::revision(true);
        let origin = *root_revision.canonical_origin();
        let world = TrafficWorld::install(
            std::sync::Arc::clone(&root_revision),
            WorldConfig::new(8, 4, 1_024, 1_024, 1, 100),
            crate::cutover::tests::transaction_tests::source_for(
                origin,
                "fixture://empty-snapshot",
            ),
            9,
            crate::test_policy::selection(&root_revision),
        )
        .expect("install");
        let bytes = encode_lfrs(&world.capture_snapshot().expect("capture"));
        let root = wire::size_prefixed_root_as_runtime_snapshot(&bytes).expect("verified LFRS");
        assert!(root.routes().is_empty());
        assert!(root.vehicles().is_empty());
        assert!(root.live_order().is_empty());
    }

    #[test]
    fn capture_survives_route_slot_reuse() {
        let (mut world, route, _) = world_with_vehicle(true);
        // 追加一条无车路线再移除，槽位回收后重新注册：局部 ID 仍按
        // live 槽位序稠密分配，不受槽位复用影响。
        let second = world
            .register_route(RouteRegisterInput::new(
                world.route_edges(route).expect("route").to_vec(),
            ))
            .expect("register second");
        world.remove_route(second).expect("remove unused route");
        let third = world
            .register_route(RouteRegisterInput::new(
                world.route_edges(route).expect("route").to_vec(),
            ))
            .expect("register third");
        let _ = third;
        let snapshot = world.capture_snapshot().expect("capture");
        assert_eq!(snapshot.routes().len(), 2);
        assert_eq!(snapshot.routes()[0].snapshot_route_id, 1);
        assert_eq!(snapshot.routes()[1].snapshot_route_id, 2);
    }
}
