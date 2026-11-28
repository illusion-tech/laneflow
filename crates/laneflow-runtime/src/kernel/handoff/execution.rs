//! 固定步进接收的交接申请和已提交的车辆组合预约。

use laneflow_static_contract::{CanonicalFrameOrdinal, LaneEdgeOrdinal, NetworkRevisionId};
use laneflow_static_network::{
    SharedNetworkRevision, StationAddress, StationCandidates,
    vehicle::{
        BodyEnvelope, GeometryWorkLimits, HandoffFootprint, HandoffLaneSide, HandoffQuery,
        HandoffStationRange, HandoffTravelTarget, HandoffWindow, PoseBounds,
    },
};
use std::sync::Arc;

use super::{
    CursorRun, HandoffRouteRef, LocatedTrajectory, RouteHandoffSpeedPlan, RouteHandoffTrial,
    sample::LocatedSample,
};
use crate::{
    DownstreamInterval, RouteHandle, StepError, StepOutcome, TickInput, TrafficWorld,
    VehicleHandle, VehicleState, VehicleStatus,
    kernel::{
        motion_updates::MotionUpdates,
        phase::{StepReadView, StepWorkspace},
        tables::CompiledRoute,
    },
};

/// 本拍申请在已登记的路线出现上进行横移。身份和起始前杠必须匹配拍初状态。
#[derive(Clone, Copy, Debug)]
pub struct RouteHandoffRequest {
    /// 车辆完整句柄。
    pub vehicle: VehicleHandle,
    /// 车辆本次行驶的已登记路线。
    pub route: RouteHandle,
    /// 窗口所属修订。
    pub revision: NetworkRevisionId,
    /// 已登记窗口的源出现下标。
    pub source_index: u32,
    /// 用于整车投影的数值递增站位窗口，单位毫米。
    pub station_window_mm: [u64; 2],
    /// 沿行驶方向的后轴起止站位，单位毫米。
    pub rear_stations_mm: [u64; 2],
    /// 转向速率求证采用的最高后轴速度，单位米每秒。
    pub speed_m_s: f64,
    /// 轮迹和扫过的逻辑预算。
    pub limits: GeometryWorkLimits,
}

/// 已提交候选的行程与前杠交接阶段。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouteHandoffProgress {
    /// 此车路线中的源出现。
    pub source_index: u32,
    /// 从候选起点累计走完的整数毫米，拍间积分保留余数。
    pub rear_travel_mm: u64,
    /// 前杠中点是否已跨入目标路线。
    pub front_handed_off: bool,
    /// 组合预约首次实际进入的刻度。
    pub acquired_tick: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActiveRouteHandoff {
    pub(crate) vehicle: VehicleHandle,
    pub(crate) source_index: u32,
    pub(crate) trajectory_version: u16,
    pub(crate) station_window_mm: [u64; 2],
    pub(crate) rear_stations_mm: [u64; 2],
    pub(crate) admitted_speed_bits: u64,
    pub(crate) tracks: u32,
    pub(crate) sweep: u32,
    pub(crate) rear_travel_um: u64,
    pub(crate) front_handed_off: bool,
    pub(crate) acquired_tick: u64,
    pub(crate) protections: Vec<HandoffProtection>,
}

/// 每个角色按物理边的稳定身份顺序保存规范半开区间。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HandoffProtection {
    pub(crate) role: u8,
    pub(crate) edge: LaneEdgeOrdinal,
    pub(crate) start_mm: u32,
    pub(crate) end_mm: u32,
}

/// 只保留已获准候选在其路线内的出现下标。修订与路线由世界持有，限速表按相同配置重建。
pub(crate) struct PreparedRouteHandoff {
    vehicle: VehicleHandle,
    route: RouteHandle,
    acquired_tick: u64,
    query: HandoffQuery,
    window: HandoffWindow,
    body: BodyEnvelope,
    runs: [std::ops::Range<usize>; 2],
    plan: RouteHandoffSpeedPlan,
}

impl PreparedRouteHandoff {
    #[cfg(any(test, feature = "execution-evidence"))]
    pub(crate) fn retained_logical_bytes(&self) -> u64 {
        std::mem::size_of::<Self>() as u64 + crate::kernel::state::vec_bytes(&self.plan.windows)
    }
    fn prepare(
        record: &ActiveRouteHandoff,
        route: HandoffRouteRef<'_>,
        trial: &RouteHandoffTrial,
        vehicle: VehicleState,
        config: &super::HandoffDrivingConfig,
        dt: u32,
    ) -> Option<Self> {
        let preview = route.preview(record.source_index, trial).ok()?;
        let plan = preview.speed_plan(trial, config, dt).ok()?;
        let p = preview.trajectory?;
        Some(Self {
            vehicle: vehicle.handle,
            route: vehicle.route,
            acquired_tick: record.acquired_tick,
            query: p.query,
            window: p.window,
            body: p.body,
            runs: p.cursor_runs.map(CursorRun::indices),
            plan,
        })
    }
    fn located<'a>(&self, binding: HandoffRouteRef<'a>) -> Option<LocatedTrajectory<'a>> {
        if self.query.revision != binding.revision.network_revision() {
            return None;
        }
        Some(LocatedTrajectory {
            revision: binding.revision,
            query: self.query,
            window: self.window,
            body: self.body,
            comfort_decel: self.plan.comfort_decel,
            cursor_runs: [
                CursorRun::in_route(binding.edges, self.runs[0].clone())?,
                CursorRun::in_route(binding.edges, self.runs[1].clone())?,
            ],
        })
    }
    /// 同拍授权和提交后的稀疏计划均按槽位升序。恢复后的空缓存按原轨迹重建。
    pub(in crate::kernel) fn cached<'a>(
        plans: &'a [Arc<Self>],
        record: &ActiveRouteHandoff,
        route: RouteHandle,
    ) -> Option<&'a Arc<Self>> {
        let index = plans
            .binary_search_by_key(&record.vehicle.index(), |plan| plan.vehicle.index())
            .ok()?;
        let plan = &plans[index];
        (plan.vehicle == record.vehicle
            && plan.route == route
            && plan.acquired_tick == record.acquired_tick)
            .then_some(plan)
    }
}

/// 提交态的前杠定位和退出候选时的余程按一毫米求界。
const COMMITTED_TRAVEL_TOLERANCE_MM: u64 = 1;

struct HandoffMovement {
    vehicle: VehicleState,
    record: ActiveRouteHandoff,
    prepared: Arc<PreparedRouteHandoff>,
    cleared: bool,
}

#[derive(Default)]
pub(crate) struct HandoffGrants {
    pub(crate) records: Vec<ActiveRouteHandoff>,
    plans: Vec<Arc<PreparedRouteHandoff>>,
}

impl TrafficWorld {
    /// 读取这辆车在当前成功提交拍仍有效的横移记录。
    #[must_use = "迭代器只有消费时才读取已提交状态"]
    pub fn vehicle_route_handoffs(
        &self,
        handle: VehicleHandle,
    ) -> impl Iterator<Item = RouteHandoffProgress> + '_ {
        self.execution.assert_usable();
        self.state
            .committed
            .handoffs
            .iter()
            .filter(move |h| h.vehicle == handle)
            .map(|h| RouteHandoffProgress {
                source_index: h.source_index,
                rear_travel_mm: h.rear_travel_um / 1000,
                front_handed_off: h.front_handed_off,
                acquired_tick: h.acquired_tick,
            })
    }
    /// 用同一拍初状态裁决一次固定步进及其横移申请。未获授予的申请仅在本拍有效。
    /// 已进入的车辆继续沿原候选前进。申请按世界车辆更新次序受理。
    ///
    /// # Errors
    ///
    /// 句柄或修订不匹配、重复申请或无法证明已提交预约仍安全时，整拍失败。
    ///
    /// # Panics
    ///
    /// 世界的执行作用域已经失效时产生 panic。
    pub fn step_with_route_handoffs(
        &mut self,
        input: TickInput,
        requests: &[RouteHandoffRequest],
    ) -> Result<StepOutcome, StepError> {
        self.execution.assert_usable();
        self.execution.run(&mut self.state, |state, resources| {
            #[cfg(feature = "execution-evidence")]
            let _step = crate::kernel::execution_evidence::step();
            state.step_vehicles_with_handoffs(input, requests, Some(resources))
        })
    }
}

impl ActiveRouteHandoff {
    /// 停车命令须从本次横移可恢复普通行驶的前杠位置抵达入口。
    pub(in crate::kernel) fn parking_entry_reachable(
        &self,
        world: StepReadView<'_>,
        vehicle: VehicleState,
        entry: (u32, u32),
    ) -> bool {
        let prove = || {
            if entry.0 <= self.source_index {
                return Some(false);
            }
            let binding = world
                .compiled_route(vehicle.route)?
                .handoff_binding(&world.binding.revision)?;
            let trial = self.trial(&world.binding.revision, vehicle)?;
            let path = match PreparedRouteHandoff::cached(
                &world.derived.handoff_plans,
                self,
                vehicle.route,
            ) {
                Some(plan) => plan.located(binding)?,
                None => {
                    binding
                        .preview(self.source_index, &trial)
                        .ok()?
                        .trajectory?
                }
            };
            Some(path.parking_entry_reachable(trial.limits.sweep, entry))
        };
        prove() == Some(true)
    }

    pub(crate) fn sample(
        &self,
        binding: &crate::kernel::state::WorldBindingState,
        route: &CompiledRoute,
        vehicle: VehicleState,
        plans: &[Arc<PreparedRouteHandoff>],
    ) -> Option<LocatedSample> {
        self.geometry(binding, route, vehicle, plans)
            .map(|(sample, _, _)| sample)
    }

    pub(super) fn geometry(
        &self,
        binding: &crate::kernel::state::WorldBindingState,
        route: &CompiledRoute,
        vehicle: VehicleState,
        plans: &[Arc<PreparedRouteHandoff>],
    ) -> Option<(LocatedSample, PoseBounds, CanonicalFrameOrdinal)> {
        binding.config.handoff_driving()?;
        let trial = self.trial(&binding.revision, vehicle)?;
        let binding = route.handoff_binding(&binding.revision)?;
        let path = match PreparedRouteHandoff::cached(plans, self, vehicle.route) {
            Some(plan) => plan.located(binding)?,
            None => {
                binding
                    .preview(self.source_index, &trial)
                    .ok()?
                    .trajectory?
            }
        };
        let (sample, pose, progress) = position_on(&path, self.rear_travel_um, trial.limits, true)?;
        committed_position(vehicle, &sample, progress).then_some((sample, pose, path.query.frame))
    }

    fn requested(request: &RouteHandoffRequest, tick: u64) -> Self {
        Self {
            vehicle: request.vehicle,
            source_index: request.source_index,
            trajectory_version: 1,
            station_window_mm: request.station_window_mm,
            rear_stations_mm: request.rear_stations_mm,
            admitted_speed_bits: if request.speed_m_s == 0. {
                0
            } else {
                request.speed_m_s.to_bits()
            },
            tracks: request
                .limits
                .tracks
                .min(GeometryWorkLimits::STANDARD.tracks),
            sweep: request.limits.sweep.min(GeometryWorkLimits::STANDARD.sweep),
            rear_travel_um: 0,
            front_handed_off: false,
            acquired_tick: tick,
            protections: Vec::new(),
        }
    }

    pub(super) fn trial(
        &self,
        revision: &SharedNetworkRevision,
        vehicle: VehicleState,
    ) -> Option<RouteHandoffTrial> {
        let speed = f64::from_bits(self.admitted_speed_bits);
        if self.trajectory_version != 1
            || !speed.is_finite()
            || speed.is_sign_negative()
            || self.tracks > GeometryWorkLimits::STANDARD.tracks
            || self.sweep > GeometryWorkLimits::STANDARD.sweep
        {
            return None;
        }
        Some(RouteHandoffTrial {
            profile: revision.identity().stable_id(vehicle.profile)?,
            station_window_mm: self.station_window_mm,
            rear_stations_mm: self.rear_stations_mm,
            speed_m_s: speed,
            limits: GeometryWorkLimits {
                tracks: self.tracks,
                sweep: self.sweep,
            },
        })
    }
}

impl LocatedTrajectory<'_> {
    /// 限速、余程及停车入口共用车尾清空时的唯一目标前杠游标。
    fn terminal_front(
        &self,
        sweep: u32,
    ) -> Option<(HandoffStationRange, HandoffFootprint, (u32, u32))> {
        let rear = HandoffStationRange::from_millimetres([self.query.rear_stations_mm[1]; 2])?;
        let foot = self
            .revision
            .handoff_footprint(&self.query, self.window, self.body, rear, sweep)
            .footprint
            .ok()?;
        let cursor = {
            let mut addresses = foot.front.addresses();
            let (Some(target), None) = (addresses.next(), addresses.next()) else {
                return None;
            };
            (
                self.cursor_runs[1].resolve(target.edge).ok()?,
                target.progress_mm[0],
            )
        };
        foot.sides[0].is_none().then_some((rear, foot, cursor))
    }

    fn parking_entry_reachable(&self, sweep: u32, entry: (u32, u32)) -> bool {
        self.terminal_front(sweep)
            .is_some_and(|(_, _, endpoint)| entry >= endpoint)
    }

    fn preserves_parking(
        &self,
        world: StepReadView<'_>,
        vehicle: VehicleHandle,
        sweep: u32,
    ) -> bool {
        match world.committed.parking.binding(vehicle) {
            None => true,
            Some(crate::ParkingBinding::Reserved(r)) => {
                world.reservation_anchor(r).is_some_and(|(_, mm)| {
                    self.parking_entry_reachable(sweep, (r.entry_route_occurrence(), mm))
                })
            }
            Some(crate::ParkingBinding::Occupied(_)) => false,
        }
    }
}

/// 前杠列按已提交侧别取有向毫米下界。前杠可能横跨共边时保留源游标。
/// 所有占用、停车和释放判断继续读取取样中的完整几何区间。
fn position_on(
    path: &LocatedTrajectory<'_>,
    travel_um: u64,
    limits: GeometryWorkLimits,
    reserved: bool,
) -> Option<(LocatedSample, PoseBounds, u32)> {
    let (sample, pose) = path
        .sample_geometry(
            travel_um / 1000,
            (travel_um % 1000) as u16,
            COMMITTED_TRAVEL_TOLERANCE_MM,
            limits,
            reserved,
        )
        .0
        .ok()?;
    let progress_mm = sample.footprint.front.addresses().next()?.progress_mm[0];
    Some((sample, pose, progress_mm))
}

fn half_open(
    revision: &SharedNetworkRevision,
    address: StationAddress,
    role: u8,
) -> Option<HandoffProtection> {
    let edge = address.edge;
    let length = *revision
        .traffic()
        .lane_lengths_millimetres()
        .get(edge.index())?;
    let [lo, hi] = address.progress_mm;
    if lo > hi || hi > length || length == 0 {
        return None;
    }
    Some(HandoffProtection {
        role,
        edge,
        start_mm: lo.min(length - 1),
        end_mm: hi.saturating_add(1).min(length),
    })
}

fn overlap(left: &HandoffProtection, right: &HandoffProtection) -> bool {
    left.edge == right.edge && left.start_mm < right.end_mm && right.start_mm < left.end_mm
}

impl StepReadView<'_> {
    fn handoff_tick_travel_mm(self, trial: &RouteHandoffTrial) -> u64 {
        let speed = super::clearance_plan::admitted_speed_mm_s(trial.speed_m_s);
        let travel_um = u64::from(speed) * self.binding.config.fixed_delta_time_ms();
        // 配置规定步长至多一秒；首尾速度都不超过已证上限。
        travel_um.div_ceil(1000)
    }
}

fn protections(
    route: &CompiledRoute,
    path: &LocatedTrajectory<'_>,
    sides: &[Option<StationCandidates>; 2],
    vehicle: VehicleState,
    tick_travel_mm: u64,
) -> Option<Vec<HandoffProtection>> {
    let revision = path.revision;
    let traffic = revision.traffic();
    let mut pieces = Vec::new();
    for (i, side) in sides.iter().enumerate() {
        if let Some(side) = side {
            for address in side.addresses() {
                let physical = half_open(revision, address, [1, 2][i])?;
                pieces.push(physical);
                pieces.push(HandoffProtection {
                    role: 3,
                    ..physical
                });
            }
        }
    }
    // 容纳整车，并为可能几乎整拍都落在目标普通路线上的余程保留间距。
    let profile = traffic.relations().vehicle_profile(vehicle.profile)?;
    let target = sides[1].as_ref()?.addresses().last()?;
    let mut index = usize::try_from(path.cursor_runs[1].resolve(target.edge).ok()?).ok()?;
    let mut start = target.progress_mm[1];
    let minimum = vehicle.length_mm.checked_add(profile.min_gap_mm())?;
    let reach = u32::try_from(tick_travel_mm)
        .ok()?
        .max(vehicle.length_mm)
        .checked_add(profile.min_gap_mm())?;
    let mut left = reach;
    while left > 0 {
        let edge = *route.edges.get(index)?;
        let length = traffic.lane_lengths_millimetres()[edge.index()];
        let take = left.min(length.checked_sub(start)?);
        if take != 0 {
            pieces.push(HandoffProtection {
                role: 4,
                edge,
                start_mm: start,
                end_mm: start + take,
            });
            left -= take;
        }
        if left > 0
            && (index + 1 == route.edges.len()
                || route
                    .handoff_windows()
                    .binary_search_by_key(&index, |(i, _)| *i as usize)
                    .is_ok())
        {
            // 路线终点和下一横移源边均为普通游标的硬边界，余程到此即止。
            if reach - left < minimum {
                return None;
            }
            break;
        }
        index = index.checked_add(1)?;
        start = 0;
    }
    pieces.sort_unstable_by_key(|p| {
        (
            p.role,
            revision.identity().stable_id(p.edge),
            p.start_mm,
            p.end_mm,
        )
    });
    let mut used = 0usize;
    for i in 0..pieces.len() {
        let next = pieces[i];
        if used != 0 {
            let prior = &mut pieces[used - 1];
            if prior.role == next.role && prior.edge == next.edge && prior.end_mm >= next.start_mm {
                prior.end_mm = prior.end_mm.max(next.end_mm);
                continue;
            }
        }
        pieces[used] = next;
        used += 1;
    }
    pieces.truncate(used);
    Some(pieces)
}

fn deferred_front_is_reserved(
    revision: &SharedNetworkRevision,
    foot: &HandoffFootprint,
    protections: &[HandoffProtection],
) -> bool {
    let Some(target) = foot.possible_target_front else {
        return true;
    };
    let Some(source) = foot.front.addresses().next() else {
        return false;
    };
    [source, target]
        .into_iter()
        .zip([1, 2])
        .all(|(address, role)| {
            half_open(revision, address, role).is_some_and(|p| {
                protections.iter().any(|held| {
                    held.role == role
                        && held.edge == p.edge
                        && held.start_mm <= p.start_mm
                        && held.end_mm >= p.end_mm
                })
            })
        })
}

fn committed_position(vehicle: VehicleState, sample: &LocatedSample, progress_mm: u32) -> bool {
    sample.front_occurrence == vehicle.route_edge_index
        && vehicle.progress_mm == progress_mm
        && vehicle.carry_um == 0
}

fn matches_entry(vehicle: VehicleState, sample: &LocatedSample) -> bool {
    let Some(front) = sample.footprint.front.addresses().next() else {
        return false;
    };
    let observed = vehicle.rounded_front_progress_mm();
    sample.front_occurrence == vehicle.route_edge_index
        && sample.footprint.front_side == HandoffLaneSide::Source
        && (front.progress_mm[0]..=front.progress_mm[1]).contains(&observed)
}

fn swept(
    path: &LocatedTrajectory<'_>,
    rear: HandoffStationRange,
    limit: u32,
) -> Option<[Option<StationCandidates>; 2]> {
    path.revision
        .handoff_swept_occupancy(&path.query, path.window, path.body, rear, limit)
        .sides
        .ok()
}

fn remaining_sweep(
    path: &LocatedTrajectory<'_>,
    trial: &RouteHandoffTrial,
    rear: HandoffStationRange,
) -> Option<[Option<StationCandidates>; 2]> {
    let end = HandoffStationRange::from_millimetres([path.query.rear_stations_mm[1]; 2])?;
    swept(path, rear.hull(end), trial.limits.sweep)
}

fn stop_lines_allow(
    world: StepReadView<'_>,
    route: HandoffRouteRef<'_>,
    path: &LocatedTrajectory<'_>,
    vehicle: VehicleState,
    sides: &[Option<StationCandidates>; 2],
    current: &HandoffFootprint,
    reached: Option<(&HandoffFootprint, u64)>,
) -> bool {
    let mapping = path.revision.station_mapping();
    let relations = path.revision.traffic().relations();
    for (index, side) in sides.iter().enumerate() {
        let Some(side) = side else {
            continue;
        };
        let prior = side
            .addresses()
            .next()
            .and_then(|h| mapping.preceding_lane_edge(h.edge));
        let mut edges = prior
            .into_iter()
            .chain(side.addresses().map(|h| h.edge))
            .peekable();
        while let Some(edge) = edges.next() {
            if relations.stop_line_for_edge(edge).is_none() {
                continue;
            }
            let Some(station) = mapping.edge_end_station_mm(edge) else {
                return false;
            };
            if !side.station_relation(station).is_eq() {
                continue;
            }
            let Some(gate) = edges
                .peek()
                .and_then(|&next| route.selected_gate(path.cursor_runs[index], edge, next))
            else {
                return false;
            };
            if world.gate_policy_decision(gate, vehicle.profile)
                == crate::GatePolicyDecision::DenyAndStop
            {
                let already_entered = |foot: &HandoffFootprint| {
                    foot.sides[index]
                        .as_ref()
                        .is_some_and(|side| side.has_passed_station(station))
                };
                // 先前已提交的这一侧实体确已越线，允许尾部清空。黄灯仅在有实体见证时
                // 才能以上界距离套用原有紧急停车困境裁决。
                if !already_entered(current)
                    && !reached.is_some_and(|(foot, travel_um)| {
                        already_entered(foot)
                            && u32::try_from(travel_um.div_ceil(1000))
                                .ok()
                                .is_some_and(|mm| {
                                    world.gate_yellow_dilemma_exempt(
                                        gate,
                                        vehicle.profile,
                                        vehicle.speed_mm_s,
                                        mm,
                                    )
                                })
                    })
                {
                    return false;
                }
            }
        }
    }
    true
}

impl StepWorkspace<'_> {
    pub(crate) fn grant_handoffs(
        &self,
        requests: &[RouteHandoffRequest],
        tick: u64,
    ) -> Result<HandoffGrants, StepError> {
        let mut grants = HandoffGrants::default();
        if requests.is_empty() {
            return Ok(grants);
        }
        let world = self.read_view();
        let config = world
            .binding
            .config
            .handoff_driving()
            .ok_or(StepError::InvalidHandoffRequest)?;
        for (i, r) in requests.iter().enumerate() {
            if !r.speed_m_s.is_finite()
                || r.speed_m_s < 0.
                || r.revision != world.binding.revision.network_revision()
                || !world
                    .vehicle_state(r.vehicle)
                    .is_some_and(|v| v.status == VehicleStatus::Active && v.route == r.route)
                || !world.compiled_route(r.route).is_some_and(|route| {
                    route
                        .handoff_windows()
                        .binary_search_by_key(&r.source_index, |w| w.0)
                        .is_ok()
                })
                || requests[..i].iter().any(|other| other.vehicle == r.vehicle)
                || self
                    .committed
                    .handoffs
                    .iter()
                    .any(|h| h.vehicle == r.vehicle)
            {
                return Err(StepError::InvalidHandoffRequest);
            }
        }
        for vehicle in &self.derived.active_order {
            let Some(request) = requests.iter().find(|r| r.vehicle == *vehicle) else {
                continue;
            };
            let mut record = ActiveRouteHandoff::requested(request, tick);
            let vehicle = world
                .vehicle_state(*vehicle)
                .ok_or(StepError::InvalidHandoffRequest)?;
            let route = world
                .compiled_route(vehicle.route)
                .ok_or(StepError::InvalidHandoffRequest)?;
            let binding = route
                .handoff_binding(&world.binding.revision)
                .ok_or(StepError::InvalidHandoffRequest)?;
            let Some(trial) = record.trial(&world.binding.revision, vehicle) else {
                continue;
            };
            let Some(prepared) = PreparedRouteHandoff::prepare(
                &record,
                binding,
                &trial,
                vehicle,
                config,
                u32::try_from(world.binding.config.fixed_delta_time_ms())
                    .map_err(|_| StepError::HandoffUnproven)?,
            ) else {
                continue;
            };
            let path = prepared
                .located(binding)
                .ok_or(StepError::HandoffUnproven)?;
            if !path.preserves_parking(world, vehicle.handle, trial.limits.sweep) {
                continue;
            }
            let Some((sample, _, _)) = position_on(&path, 0, trial.limits, false) else {
                continue;
            };
            if !matches_entry(vehicle, &sample) {
                continue;
            }
            let Some(sides) = path
                .query
                .rear_range()
                .and_then(|rear| swept(&path, rear, trial.limits.sweep))
            else {
                continue;
            };
            let Some(physical) = protections(
                route,
                &path,
                &sides,
                vehicle,
                world.handoff_tick_travel_mm(&trial),
            ) else {
                continue;
            };
            let profile = world
                .binding
                .revision
                .traffic()
                .relations()
                .vehicle_profile(vehicle.profile)
                .ok_or(StepError::HandoffUnproven)?;
            let prior = world
                .committed
                .handoffs
                .iter()
                .chain(&grants.records)
                .flat_map(|h| &h.protections);
            if physical.iter().any(|p| {
                prior.clone().any(|other| overlap(p, other))
                    || DownstreamInterval::new(p.edge, p.start_mm, p.end_mm).is_none_or(
                        |interval| {
                            self.conflict_read().handoff_downstream_conflicts(
                                interval,
                                vehicle.handle,
                                profile.min_gap_mm(),
                            )
                        },
                    )
            }) {
                continue;
            }
            let entrants = super::entrants::check_other_entrants(
                world,
                sides.each_ref().map(Option::as_ref),
                u32::MAX,
                Some(vehicle.handle),
                &physical,
            )
            .map_err(|_| StepError::HandoffUnproven)?;
            if entrants.side_blockers.iter().any(Option::is_some) {
                continue;
            }
            record.protections = physical;
            grants
                .records
                .try_reserve(1)
                .map_err(|_| StepError::VehicleStorageAllocFailed)?;
            grants
                .plans
                .try_reserve(1)
                .map_err(|_| StepError::VehicleStorageAllocFailed)?;
            grants.records.push(record);
            grants.plans.push(Arc::new(prepared));
        }
        grants.plans.sort_unstable_by_key(|p| p.vehicle.index());
        Ok(grants)
    }

    pub(crate) fn stage_handoffs(
        &self,
        grants: &HandoffGrants,
        updates: &mut MotionUpdates,
        result: &mut Vec<ActiveRouteHandoff>,
        plans: &mut Vec<Arc<PreparedRouteHandoff>>,
        parking_arrivals: &mut Vec<crate::ParkingArrivalObservation>,
    ) -> Result<bool, StepError> {
        if grants.records.is_empty() && self.committed.handoffs.is_empty() {
            return Ok(false);
        }
        let world = self.read_view();
        result
            .try_reserve(
                self.committed
                    .handoffs
                    .len()
                    .saturating_add(grants.records.len()),
            )
            .map_err(|_| StepError::VehicleStorageAllocFailed)?;
        plans
            .try_reserve(
                self.committed
                    .handoffs
                    .len()
                    .saturating_add(grants.records.len()),
            )
            .map_err(|_| StepError::VehicleStorageAllocFailed)?;
        // 既有授权优先。新申请按已提交 live 顺序，后来的申请可见同拍先获准的保护。
        let mut finished = false;
        let mut parking_arrival_changed = false;
        for existing in [true, false] {
            for index in 0..updates.active_len() {
                let (_, ordinary) = updates.get(index, &self.committed.vehicles);
                let handle = ordinary.handle;
                let candidate = if existing {
                    self.committed
                        .handoffs
                        .iter()
                        .find(|h| h.vehicle == handle)
                        .cloned()
                } else {
                    grants.records.iter().find(|r| r.vehicle == handle).cloned()
                };
                let Some(candidate) = candidate else {
                    continue;
                };
                let vehicle = world
                    .vehicle_state(handle)
                    .ok_or(StepError::HandoffUnproven)?;
                match self.advance_handoff(vehicle, ordinary, candidate, existing, &grants.plans)? {
                    Some(movement) => {
                        if let Some(crate::ParkingBinding::Reserved(r)) =
                            world.committed.parking.binding(handle)
                        {
                            parking_arrival_changed |= world.parking_arrived_for(ordinary, r)
                                != world.parking_arrived_for(movement.vehicle, r);
                        }
                        updates.set_handoff_motion(
                            index,
                            movement.vehicle,
                            &self.committed.vehicles,
                            world.compiled_route(vehicle.route),
                            world.binding.revision.traffic().lane_lengths_millimetres(),
                        )?;
                        if movement.cleared {
                            finished = true;
                        } else {
                            result.push(movement.record);
                            plans.push(movement.prepared);
                        }
                    }
                    None if existing => return Err(StepError::HandoffUnproven),
                    None => {}
                }
            }
        }
        if parking_arrival_changed {
            world.rebuild_parking_arrivals(updates, parking_arrivals)?;
        }
        result.sort_unstable_by_key(|h| (h.vehicle.index(), h.source_index));
        plans.sort_unstable_by_key(|p| p.vehicle.index());
        Ok(!result.is_empty() || finished)
    }

    fn advance_handoff(
        &self,
        vehicle: VehicleState,
        ordinary: VehicleState,
        mut candidate: ActiveRouteHandoff,
        existing: bool,
        new_plans: &[Arc<PreparedRouteHandoff>],
    ) -> Result<Option<HandoffMovement>, StepError> {
        let world = self.read_view();
        let revision = &world.binding.revision;
        let config = world
            .binding
            .config
            .handoff_driving()
            .ok_or(StepError::InvalidHandoffRequest)?;
        let dt = u32::try_from(world.binding.config.fixed_delta_time_ms())
            .map_err(|_| StepError::HandoffUnproven)?;
        let route = world
            .compiled_route(vehicle.route)
            .ok_or(StepError::HandoffUnproven)?;
        let binding = route
            .handoff_binding(revision)
            .ok_or(StepError::HandoffUnproven)?;
        let Some(trial) = candidate.trial(revision, vehicle) else {
            return Ok(None);
        };
        let plans = if existing {
            &self.derived.handoff_plans[..]
        } else {
            new_plans
        };
        let prepared = match PreparedRouteHandoff::cached(plans, &candidate, vehicle.route) {
            Some(cached) => Arc::clone(cached),
            None => {
                let Some(p) =
                    PreparedRouteHandoff::prepare(&candidate, binding, &trial, vehicle, config, dt)
                else {
                    return Ok(None);
                };
                Arc::new(p)
            }
        };
        let path = prepared
            .located(binding)
            .ok_or(StepError::HandoffUnproven)?;
        let tolerance = u64::from(config.travel_tolerance_mm.get());
        let Some((current, _, progress_mm)) =
            position_on(&path, candidate.rear_travel_um, trial.limits, existing)
        else {
            return Ok(None);
        };
        if (existing && !committed_position(vehicle, &current, progress_mm))
            || (!existing && !matches_entry(vehicle, &current))
            || candidate.front_handed_off
                != (current.footprint.front_side == HandoffLaneSide::Target)
        {
            return Ok(None);
        }
        let Some(full) = remaining_sweep(&path, &trial, current.rear) else {
            return Ok(None);
        };
        let tick_travel_mm = world.handoff_tick_travel_mm(&trial);
        let Some(physical) = protections(route, &path, &full, vehicle, tick_travel_mm) else {
            return Ok(None);
        };
        let profile = revision
            .traffic()
            .relations()
            .vehicle_profile(vehicle.profile)
            .ok_or(StepError::HandoffUnproven)?;
        if physical.iter().any(|p| {
            DownstreamInterval::new(p.edge, p.start_mm, p.end_mm).is_none_or(|interval| {
                self.conflict_read().handoff_downstream_conflicts(
                    interval,
                    vehicle.handle,
                    profile.min_gap_mm(),
                )
            })
        }) {
            return Ok(None);
        }
        let plan = &prepared.plan;
        let mut cap = profile.desired_speed_mm_s();
        for p in &physical {
            cap = cap.min(
                revision
                    .traffic()
                    .lane_speed_limits_millimetres_per_second()[p.edge.index()],
            );
        }
        let mut proposed = ordinary.speed_mm_s.min(cap);
        if ordinary.status != VehicleStatus::Active
            || vehicle.maneuver_traversal != ordinary.maneuver_traversal
            || vehicle.waiting_membership != ordinary.waiting_membership
        {
            return Ok(None);
        }
        if !stop_lines_allow(
            world,
            binding,
            &path,
            vehicle,
            &full,
            &current.footprint,
            None,
        ) {
            let horizon = plan
                .stop_horizon_um(vehicle.speed_mm_s, proposed)
                .ok_or(StepError::HandoffUnproven)?;
            let ahead = candidate
                .rear_travel_um
                .checked_add(horizon)
                .ok_or(StepError::HandoffUnproven)?;
            let range = if ahead <= plan.length_lower_um {
                let Ok(bounds) = revision
                    .handoff_travel_after_motion(
                        &path.query,
                        path.window,
                        ahead / 1000,
                        (ahead % 1000) as u16,
                        tolerance,
                        trial.limits.tracks,
                    )
                    .bounds
                else {
                    return Ok(None);
                };
                bounds.rear
            } else {
                HandoffStationRange::from_millimetres([path.query.rear_stations_mm[1]; 2])
                    .ok_or(StepError::HandoffUnproven)?
            };
            let span = current.rear.hull(range);
            let Some(lookahead) = swept(&path, span, trial.limits.sweep) else {
                return Ok(None);
            };
            let far = revision
                .handoff_footprint(
                    &path.query,
                    path.window,
                    path.body,
                    range,
                    trial.limits.sweep,
                )
                .footprint
                .ok();
            if !stop_lines_allow(
                world,
                binding,
                &path,
                vehicle,
                &lookahead,
                &current.footprint,
                far.as_ref().map(|foot| (foot, horizon)),
            ) {
                let drop = (f64::from(profile.comfort_decel()) * f64::from(dt)).floor() as u32;
                proposed = proposed.min(vehicle.speed_mm_s.saturating_sub(drop));
            }
        }
        let ordinary_mm = crate::kernel::tables::distance_to_occurrence_progress(
            &route.occurrence_segments,
            &route.occurrence_offsets,
            &route.segment_totals,
            vehicle.route_edge_index as usize,
            vehicle.progress_mm,
            ordinary.route_edge_index as usize,
            ordinary.progress_mm,
        );
        let Some(laneflow_static_network::BoundedDistance::Finite(ordinary_mm)) = ordinary_mm
        else {
            return Ok(None);
        };
        let ordinary_um = (u64::from(ordinary_mm) * 1000 + u64::from(ordinary.carry_um))
            .saturating_sub(u64::from(vehicle.carry_um));
        let Ok((next_speed, travel, may_finish)) = plan.motion_from_proven_position(
            candidate.rear_travel_um,
            vehicle.speed_mm_s,
            proposed,
            ordinary_um,
            existing && proposed == ordinary.speed_mm_s,
        ) else {
            return Ok(None);
        };
        if travel == 0 && !existing {
            return Ok(None);
        }
        let total = candidate
            .rear_travel_um
            .checked_add(travel)
            .ok_or(StepError::HandoffUnproven)?;
        let mut moved = vehicle;
        moved.speed_mm_s = next_speed;
        let terminal = if may_finish {
            terminal_remainder(&path, total, trial.limits.tracks)
        } else {
            None
        };
        let (front_index, front_mm, carry_um, next_foot, next_rear) = if let Some(min_remainder) =
            terminal
        {
            let Some((rear, foot, (index, progress_mm))) = path.terminal_front(trial.limits.sweep)
            else {
                return Ok(None);
            };
            // 提交较小的已证余程，它不超过本拍行程；普通游标还会在硬边界截停。
            if travel > tick_travel_mm * 1000 {
                return Ok(None);
            }
            let endpoint = VehicleState {
                route_edge_index: index,
                progress_mm,
                carry_um: 0,
                ..vehicle
            };
            moved = self.continue_after_handoff(
                endpoint,
                route,
                min_remainder,
                travel,
                next_speed,
                self.waiting_stop_for(&vehicle)?,
            )?;
            (
                moved.route_edge_index,
                moved.progress_mm,
                moved.carry_um,
                foot,
                rear,
            )
        } else {
            let Some((sample, _, mm)) = position_on(&path, total, trial.limits, existing) else {
                return Ok(None);
            };
            (
                sample.front_occurrence,
                mm,
                0,
                sample.footprint,
                sample.rear,
            )
        };
        let rear = current.rear.hull(next_rear);
        let Some(this_tick) = swept(&path, rear, trial.limits.sweep) else {
            return Ok(None);
        };
        let entrants = super::entrants::check_other_entrants(
            world,
            this_tick.each_ref().map(Option::as_ref),
            u32::MAX,
            Some(vehicle.handle),
            &physical,
        )
        .map_err(|_| StepError::HandoffUnproven)?;
        if entrants.side_blockers.iter().any(Option::is_some)
            || !stop_lines_allow(
                world,
                binding,
                &path,
                vehicle,
                &this_tick,
                &current.footprint,
                Some((&next_foot, travel)),
            )
        {
            return Ok(None);
        }
        if (candidate.front_handed_off && next_foot.front_side == HandoffLaneSide::Source)
            || !deferred_front_is_reserved(revision, &next_foot, &candidate.protections)
        {
            return Ok(None);
        }
        moved.route_edge_index = front_index;
        moved.progress_mm = front_mm;
        moved.carry_um = carry_um;
        candidate.front_handed_off = next_foot.front_side == HandoffLaneSide::Target;
        candidate.rear_travel_um = total;
        candidate.protections = if terminal.is_some() {
            physical
        } else {
            let Some(remaining) = remaining_sweep(&path, &trial, next_rear) else {
                return Ok(None);
            };
            let Some(next) = protections(route, &path, &remaining, vehicle, tick_travel_mm) else {
                return Ok(None);
            };
            if !deferred_front_is_reserved(revision, &next_foot, &next) {
                return Ok(None);
            }
            next
        };
        Ok(Some(HandoffMovement {
            vehicle: moved,
            record: candidate,
            prepared,
            cleared: terminal.is_some(),
        }))
    }
}

fn terminal_remainder(path: &LocatedTrajectory<'_>, total_um: u64, work: u32) -> Option<u64> {
    let end = HandoffTravelTarget::RearStationMm(path.query.rear_stations_mm[1]);
    let distance = path
        .revision
        .handoff_travel(
            &path.query,
            path.window,
            end,
            COMMITTED_TRAVEL_TOLERANCE_MM,
            work,
        )
        .bounds
        .ok()?
        .distance;
    // 先证明已跨完整段，再保守分配续行；整毫米及积分余数一起进入下一拍。
    let [(mm, carry), _] =
        distance.remaining_after_motion(total_um / 1000, (total_um % 1000) as u16)?;
    Some(mm * 1000 + u64::from(carry))
}

impl crate::kernel::state::WorldState {
    /// 冷恢复在候选世界完成登记后重建每项几何、游标和组合保护。
    pub(crate) fn validate_restored_handoffs(&self) -> bool {
        let read = self.read_view();
        self.committed
            .handoffs
            .iter()
            .enumerate()
            .all(|(i, record)| {
                let verify = || {
                    let vehicle = read.vehicle_state(record.vehicle)?;
                    let revision = &self.binding.revision;
                    self.binding.config.handoff_driving()?;
                    let route = read.compiled_route(vehicle.route)?;
                    let binding = route.handoff_binding(revision)?;
                    let trial = record.trial(revision, vehicle)?;
                    let preview = binding.preview(record.source_index, &trial).ok()?;
                    let path = preview.trajectory.as_ref()?;
                    if !path.preserves_parking(read, vehicle.handle, trial.limits.sweep) {
                        return None;
                    }
                    let (sample, _, position) =
                        position_on(path, record.rear_travel_um, trial.limits, true)?;
                    let sides = remaining_sweep(path, &trial, sample.rear)?;
                    let required = protections(
                        route,
                        path,
                        &sides,
                        vehicle,
                        read.handoff_tick_travel_mm(&trial),
                    )?;
                    let profile = revision
                        .traffic()
                        .relations()
                        .vehicle_profile(vehicle.profile)?;
                    if required.iter().any(|p| {
                        DownstreamInterval::new(p.edge, p.start_mm, p.end_mm).is_none_or(
                            |interval| {
                                read.conflict_read().handoff_downstream_conflicts(
                                    interval,
                                    record.vehicle,
                                    profile.min_gap_mm(),
                                )
                            },
                        )
                    }) {
                        return None;
                    }
                    let entrants = super::entrants::check_other_entrants(
                        read,
                        sides.each_ref().map(Option::as_ref),
                        u32::MAX,
                        Some(vehicle.handle),
                        &required,
                    )
                    .ok()?;
                    if entrants
                        .side_blockers
                        .iter()
                        .flatten()
                        .any(|b| b.reason == super::entrants::RouteHandoffEntrantReason::Occupied)
                    {
                        return None;
                    }
                    let disjoint = self.committed.handoffs[..i].iter().all(|other| {
                        other.vehicle != record.vehicle
                            && !other
                                .protections
                                .iter()
                                .any(|a| required.iter().any(|b| overlap(a, b)))
                    });
                    Some(
                        vehicle.status == VehicleStatus::Active
                            && disjoint
                            && record.acquired_tick > 0
                            && record.acquired_tick <= self.committed.tick_index
                            && record.rear_travel_um > 0
                            && record.front_handed_off
                                == (sample.footprint.front_side == HandoffLaneSide::Target)
                            && committed_position(vehicle, &sample, position)
                            && deferred_front_is_reserved(revision, &sample.footprint, &required)
                            && required == record.protections,
                    )
                };
                verify() == Some(true)
            })
    }
}

pub(crate) fn reservation_distance<'a>(
    reservations: impl Iterator<Item = &'a ActiveRouteHandoff>,
    route: &CompiledRoute,
    owner: Option<VehicleHandle>,
    index: usize,
    front: u32,
) -> Option<u32> {
    let mut nearest = None;
    for protected in reservations
        .filter(|h| Some(h.vehicle) != owner)
        .flat_map(|h| &h.protections)
    {
        let first = route
            .edge_occurrence_index
            .partition_point(|&(edge, i)| (edge.raw(), i) < (protected.edge.raw(), index as u32));
        for &(edge, i) in &route.edge_occurrence_index[first..] {
            if edge != protected.edge {
                break;
            }
            let i = i as usize;
            if i == index && protected.end_mm <= front {
                continue;
            }
            let target = if i == index {
                protected.start_mm.max(front)
            } else {
                protected.start_mm
            };
            if let Some(laneflow_static_network::BoundedDistance::Finite(mm)) =
                crate::kernel::tables::distance_to_occurrence_progress(
                    &route.occurrence_segments,
                    &route.occurrence_offsets,
                    &route.segment_totals,
                    index,
                    front,
                    i,
                    target,
                )
            {
                nearest = Some(nearest.map_or(mm, |previous: u32| previous.min(mm)));
            }
            break;
        }
    }
    nearest
}
