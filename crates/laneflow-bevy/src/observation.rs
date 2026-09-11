//! 复杂路口领域只读观测视图（#285 复杂路口观测 G1 §3）。

use std::sync::Arc;

use laneflow_runtime::{
    ConflictDecision, ConflictReservation, EntityKind, NetworkRevisionId, RouteGateObservation,
    RouteHandle, TrafficTransitionEvent, TrafficWorld, VehicleHandle, VehicleState,
    WaitingDecision, WaitingZoneMember, WaitingZoneOrdinal, WaitingZoneSnapshot, WorldGeneration,
};
use laneflow_static_network::{SharedNetworkRevision, SharedTrafficNetwork};

/// 观测视图的读取边界上下文。
///
/// 世界身份、世代与修订共同标记本次读取的归属；`tick_index`、
/// `command_cursor` 与 `event_cursor` 是读取时的已提交游标。车辆、member、
/// occupancy 与 reservation 表达读取时的当前已提交状态；latest decision 与
/// transition 表达 Runtime 保留的最近 successful tick 结果。需要持有历史的
/// 调用方自行复制有限记录并附上本上下文，展示前先核对世界、世代、修订与
/// 目标句柄；历史副本不授予更新 Transform 或反向提交命令的能力。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LaneFlowJunctionObservationContext {
    world_id: u64,
    world_generation: WorldGeneration,
    network_revision: NetworkRevisionId,
    tick_index: u64,
    command_cursor: u64,
    event_cursor: u64,
}

impl LaneFlowJunctionObservationContext {
    /// 宿主指定的世界身份。
    #[must_use]
    pub const fn world_id(self) -> u64 {
        self.world_id
    }

    /// 读取时的活动世界世代。
    #[must_use]
    pub const fn world_generation(self) -> WorldGeneration {
        self.world_generation
    }

    /// 读取时的共享路网修订标识。
    #[must_use]
    pub const fn network_revision(self) -> NetworkRevisionId {
        self.network_revision
    }

    /// 读取时的已提交 `tick_index`。
    #[must_use]
    pub const fn tick_index(self) -> u64 {
        self.tick_index
    }

    /// 读取时的已应用输入命令游标。
    #[must_use]
    pub const fn command_cursor(self) -> u64 {
        self.command_cursor
    }

    /// 读取时的已提交切换事件游标。
    #[must_use]
    pub const fn event_cursor(self) -> u64 {
        self.event_cursor
    }
}

/// 观测视图中的一行 live 车辆：句柄、当前已提交状态与当前持有的预约。
///
/// `conflict_reservation` 只表达读取时由 Conflict arbiter 单独持有的当前
/// 预约；最近决策批次中的 `Granted` 不冒充当前持有状态。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LaneFlowJunctionVehicleRow {
    vehicle: VehicleHandle,
    state: VehicleState,
    conflict_reservation: Option<ConflictReservation>,
}

impl LaneFlowJunctionVehicleRow {
    /// 车辆句柄（仅本世界当前世代有效）。
    #[must_use]
    pub const fn vehicle(self) -> VehicleHandle {
        self.vehicle
    }

    /// 读取时的已提交车辆状态。
    #[must_use]
    pub const fn state(self) -> VehicleState {
        self.state
    }

    /// 读取时当前持有的 Conflict 预约；无预约为 `None`。
    #[must_use]
    pub const fn conflict_reservation(self) -> Option<ConflictReservation> {
        self.conflict_reservation
    }
}

/// 借用活动 Session 的复杂路口领域只读观测视图。
///
/// 视图不拥有可变世界、输出队列或第二份静态路网；创建为 O(1) 且无堆分配。
/// Rust 借用期间禁止对同一 Session 推进或提交生命周期命令，因此视图不能
/// 跨 mutable Session 操作保留。headless Session 也能读取领域观察；几何
/// 绘制另外要求有效 Spatial 配对。
///
/// 每 tick 证据必须在 `LaneFlowFixedSet::Observe`、下一次 lifecycle 前消费；
/// catch-up 有多次 successful tick 时分别消费，不只读取 outer frame 最后的
/// 批次。信号组继续经 `LaneFlowSession::world()` 的
/// `committed_signal_groups()` 取得，其拥有式 materialization 的分配与耗时
/// 单独计账，不因视图本身零分配而声称整个观测链零分配。
pub struct LaneFlowJunctionObservation<'a> {
    world: &'a TrafficWorld,
}

impl<'a> LaneFlowJunctionObservation<'a> {
    pub(crate) const fn new(world: &'a TrafficWorld) -> Self {
        Self { world }
    }

    /// 本次读取的边界上下文；从同一个 world 取得。
    #[must_use]
    pub fn context(&self) -> LaneFlowJunctionObservationContext {
        LaneFlowJunctionObservationContext {
            world_id: self.world.world_id(),
            world_generation: self.world.world_generation(),
            network_revision: self.world.committed_source().network_revision(),
            tick_index: self.world.tick_index(),
            command_cursor: self.world.command_cursor(),
            event_cursor: self.world.event_cursor(),
        }
    }

    /// 活动 Session 的同一共享根；只克隆根 `Arc`，不复制静态表。
    #[must_use]
    pub fn revision(&self) -> Arc<SharedNetworkRevision> {
        self.world.revision()
    }

    /// 活动 Session 同一共享根的 Traffic component 借用。
    #[must_use]
    pub fn traffic(&self) -> &'a SharedTrafficNetwork {
        self.world.traffic()
    }

    /// 沿 `live_vehicles()` 稳定顺序的 live 车辆行。
    ///
    /// 全量遍历为 O(N_live)；每车预约查询为 O(1)，不按车扫描全部决策、
    /// 全部区域或全部路线。按车辆聚合决策时，消费方对整个批次建立一次
    /// 可复用索引或顺序合并，并独立计入观测成本。
    pub fn vehicles(&self) -> impl Iterator<Item = LaneFlowJunctionVehicleRow> + 'a {
        let world = self.world;
        world.live_vehicles().iter().filter_map(move |&vehicle| {
            let state = world.vehicle(vehicle)?;
            Some(LaneFlowJunctionVehicleRow {
                vehicle,
                state,
                conflict_reservation: world.conflict_reservation(vehicle),
            })
        })
    }

    /// 按当前根 WaitingZone ordinal 升序的已提交计数；遍历为 O(N_zone)。
    pub fn waiting_zones(&self) -> impl Iterator<Item = WaitingZoneSnapshot> + 'a {
        let world = self.world;
        let zone_count = world
            .traffic()
            .entity_counts()
            .count(EntityKind::WaitingZone);
        (0..zone_count)
            .map(WaitingZoneOrdinal::from_raw)
            .filter_map(move |zone| world.waiting_zone(zone))
    }

    /// 原样借用全部 Waiting member，保持 zone／admission sequence 顺序。
    #[must_use]
    pub fn waiting_zone_members(&self) -> &'a [WaitingZoneMember] {
        self.world.waiting_zone_members()
    }

    /// 原样借用最近 successful tick 的 Waiting 决策批次。
    ///
    /// 保留完整 anchor、outcome 与原顺序；批次是 Runtime 保留的最近成功
    /// 结果，其中的 `Granted` 不表示当前仍持有对应预约，已 stale 的车辆或
    /// 路线不映射到复用该 slot 的新车辆。
    #[must_use]
    pub fn latest_waiting_decisions(&self) -> &'a [WaitingDecision] {
        self.world.latest_waiting_decisions()
    }

    /// 原样借用最近 successful tick 的 Conflict 决策批次，语义边界同上。
    #[must_use]
    pub fn latest_conflict_decisions(&self) -> &'a [ConflictDecision] {
        self.world.latest_conflict_decisions()
    }

    /// 原样借用最近 successful tick 的 transition event 批次。
    ///
    /// 不重排、不丢失同 tick 多事件；failed step 不形成新证据行，错误与
    /// 上一成功 tick 的批次分开报告。
    #[must_use]
    pub fn latest_transition_events(&self) -> &'a [TrafficTransitionEvent] {
        self.world.latest_transition_events()
    }

    /// 转调 [`TrafficWorld::route_gate`] 的机动门只读定位。
    ///
    /// 保留 route/hop，不按静态 Gate 去重；历史 route 已移除时返回 `None`，
    /// 不从新路线或同 ordinal 猜测位置。
    #[must_use]
    pub fn route_gate(&self, route: RouteHandle, hop: u32) -> Option<RouteGateObservation> {
        self.world.route_gate(route, hop)
    }
}
