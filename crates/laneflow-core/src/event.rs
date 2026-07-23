//! Core step 输出事件。

use crate::{
    EdgeHandle, MovementGateKey, ParkingReleaseReason, ParkingSpaceHandle, RouteHandle,
    SignalAspect, SignalControllerHandle, SignalGroupHandle, SignalPhaseRef, StopLineHandle,
    VehicleHandle,
};

/// Core step 产生的可观察事件。
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CoreEvent {
    /// 道路限速边界将车辆 motion 压到 emergency envelope 以下。
    VehicleSpeedLimitProjectionApplied(VehicleSpeedLimitProjectionAppliedEvent),
    /// SignalStop hard boundary 将车辆 motion 压到 emergency envelope 以下。
    VehicleSignalStopProjectionApplied(VehicleSignalStopProjectionAppliedEvent),
    /// ParkingStop hard boundary 将车辆 motion 压到 emergency envelope 以下。
    VehicleParkingStopProjectionApplied(VehicleParkingStopProjectionAppliedEvent),
    /// 车辆为维持最终 minimum-gap 不变量而应用了超出 emergency envelope 的几何投影。
    VehicleFollowingSafetyProjectionApplied(VehicleFollowingSafetyProjectionAppliedEvent),
    /// 车辆从 route 中的一个 edge 切换到下一个 edge。
    VehicleChangedEdge(VehicleChangedEdgeEvent),
    /// Reserved vehicle 在 successful step 中首次满足 arrival predicate。
    VehicleParkingArrivalReached(VehicleParkingArrivalReachedEvent),
    /// successful step 由于 lifecycle transition 释放了 Parking reservation。
    ParkingReservationReleased(ParkingReservationReleasedEvent),
    /// 车辆到达 route 末端。
    VehicleCompletedRoute(VehicleCompletedRouteEvent),
    /// fixed-time controller 的当前 phase identity 已改变。
    SignalPhaseChanged(SignalPhaseChangedEvent),
    /// SignalGroup 的当前 indication 已改变。
    SignalGroupAspectChanged(SignalGroupAspectChangedEvent),
}

/// 道路限速 hard projection 的完整 route-occurrence attribution。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VehicleSpeedLimitProjectionAppliedEvent {
    /// 事件所属的 post-step tick index。
    pub tick_index: u64,
    /// 被投影的 vehicle。
    pub vehicle: VehicleHandle,
    /// vehicle 当前使用的 route。
    pub route: RouteHandle,
    /// 降限速边界的 from-edge occurrence index。
    pub from_route_edge_index: usize,
    /// 降限速边界的 to-edge occurrence index。
    pub to_route_edge_index: usize,
    /// 降限速边界的 from edge。
    pub from_edge: EdgeHandle,
    /// 降限速边界的 to edge。
    pub to_edge: EdgeHandle,
}

/// ParkingStop hard projection 的完整 route-occurrence attribution。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VehicleParkingStopProjectionAppliedEvent {
    /// 事件所属的 post-step tick index。
    pub tick_index: u64,
    /// 被投影的 vehicle。
    pub vehicle: VehicleHandle,
    /// 被该 vehicle 预订的 ParkingSpace。
    pub space: ParkingSpaceHandle,
    /// vehicle 当前使用的 route。
    pub route: RouteHandle,
    /// selected entry 在 route 中的 occurrence index。
    pub route_edge_index: usize,
}

/// Reserved vehicle 首次满足 arrival predicate 的 step event。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VehicleParkingArrivalReachedEvent {
    /// 事件所属的 post-step tick index。
    pub tick_index: u64,
    /// 到达 entry 的 vehicle。
    pub vehicle: VehicleHandle,
    /// 被该 vehicle 预订的 ParkingSpace。
    pub space: ParkingSpaceHandle,
    /// vehicle 当前使用的 route。
    pub route: RouteHandle,
    /// selected entry 在 route 中的 occurrence index。
    pub route_edge_index: usize,
}

/// fixed step 释放 Parking reservation 的事件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParkingReservationReleasedEvent {
    /// 事件所属的 post-step tick index。
    pub tick_index: u64,
    /// reservation owner。
    pub vehicle: VehicleHandle,
    /// 被释放的 ParkingSpace。
    pub space: ParkingSpaceHandle,
    /// 释放原因。
    pub reason: ParkingReleaseReason,
}

/// SignalStop hard projection 的完整 route-occurrence attribution。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VehicleSignalStopProjectionAppliedEvent {
    /// 事件所属的 post-step tick index。
    pub tick_index: u64,
    /// 被投影的 vehicle。
    pub vehicle: VehicleHandle,
    /// vehicle 当前使用的 route。
    pub route: RouteHandle,
    /// denied Gate 的 from-edge occurrence index。
    pub from_route_edge_index: usize,
    /// denied Gate 的 to-edge occurrence index。
    pub to_route_edge_index: usize,
    /// 被拒绝的 MovementGate value identity。
    pub gate: MovementGateKey,
    /// Gate 使用的 StopLine。
    pub stop_line: StopLineHandle,
    /// 控制 Gate 的 SignalGroup。
    pub group: SignalGroupHandle,
    /// 触发 denial 的 tick-start aspect。
    pub aspect: SignalAspect,
}

/// Vehicle Following minimum-gap-preserving 最终几何投影事件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VehicleFollowingSafetyProjectionAppliedEvent {
    /// 事件所属的 post-step tick index。
    pub tick_index: u64,
    /// 被投影的 follower vehicle handle。
    pub vehicle: VehicleHandle,
    /// 约束该 follower 的 leader vehicle handle。
    pub leader: VehicleHandle,
}

/// 车辆跨 edge 事件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VehicleChangedEdgeEvent {
    /// 事件所属的 post-step tick index。
    pub tick_index: u64,
    /// 车辆 handle。
    pub vehicle: VehicleHandle,
    /// route handle。
    pub route: RouteHandle,
    /// 切换前的 lane edge handle。
    pub from_edge: EdgeHandle,
    /// 切换后的 lane edge handle。
    pub to_edge: EdgeHandle,
    /// 切换前的 route edge index。
    pub from_route_edge_index: usize,
    /// 切换后的 route edge index。
    pub to_route_edge_index: usize,
}

/// 车辆完成 route 事件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VehicleCompletedRouteEvent {
    /// 事件所属的 post-step tick index。
    pub tick_index: u64,
    /// 车辆 handle。
    pub vehicle: VehicleHandle,
    /// route handle。
    pub route: RouteHandle,
    /// 完成时所在的 lane edge handle。
    pub edge: EdgeHandle,
    /// 完成时所在的 route edge index。
    pub route_edge_index: usize,
}

/// fixed-time controller phase 变化事件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignalPhaseChangedEvent {
    /// 事件所属的 post-step tick index。
    pub tick_index: u64,
    /// 发生变化的 controller。
    pub controller: SignalControllerHandle,
    /// step 前已提交的 phase。
    pub from_phase: SignalPhaseRef,
    /// step 后提交的 phase。
    pub to_phase: SignalPhaseRef,
}

/// SignalGroup aspect 变化事件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignalGroupAspectChangedEvent {
    /// 事件所属的 post-step tick index。
    pub tick_index: u64,
    /// 发生变化的 group。
    pub group: SignalGroupHandle,
    /// step 前已提交的 indication。
    pub from_aspect: SignalAspect,
    /// step 后提交的 indication。
    pub to_aspect: SignalAspect,
}
