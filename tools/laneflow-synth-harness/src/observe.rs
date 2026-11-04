use laneflow_runtime::{
    ManeuverTraversalPhase, ParkingBinding, ParkingTarget, TrafficTransitionKind, VehicleState,
    VehicleStatus,
};
use laneflow_static_contract::SignalAspect;

use crate::install::Sample;
use crate::{Error, cycle_ms, durations, require};

const EDGE_MM: u64 = 5_130_000;

pub(crate) fn aspect(cell: u32, route: u32, tick: u64, dt: u32) -> SignalAspect {
    let cycle = cycle_ms(dt);
    let mut position = (tick * u64::from(dt) + u64::from(cell) * 997 % cycle) % cycle;
    let phase = durations(dt)
        .into_iter()
        .position(|duration| {
            if position < duration {
                true
            } else {
                position -= duration;
                false
            }
        })
        .expect("positive cycle");
    match (phase, route % 2) {
        (0, 0) | (3, 1) => SignalAspect::Green,
        (1, 0) | (4, 1) => SignalAspect::Yellow,
        _ => SignalAspect::Red,
    }
}

pub(crate) fn position(state: VehicleState) -> u64 {
    (u64::from(state.route_edge_index()) * EDGE_MM + u64::from(state.progress_mm())) * 1_000
        + u64::from(state.carry_um())
}

/// 跨拍生命周期序（#1227）：`last_kind` 跨观测持续，接受并推进返回 true；
/// 乱序（重复 acquire、release 后 clear 等）返回 false。
fn lifecycle_order_valid(last_kind: &mut [u32], slot: usize, kind: u32) -> bool {
    // 单生命周期状态机：acquire(4) → enter(5) → clear(6) → release(7)；
    // 释放后可开启新周期（下游冲突区新预约），同周期内不得回退/重复。
    let allowed = match kind {
        4 => matches!(last_kind[slot], 0 | 7),
        5 => last_kind[slot] == 4,
        6 => last_kind[slot] == 5,
        7 => last_kind[slot] == 6,
        _ => true,
    };
    if allowed {
        last_kind[slot] = kind;
        true
    } else {
        false
    }
}

pub(crate) fn motion_valid(
    old_position: u64,
    new_position: u64,
    old_speed: u32,
    new_speed: u32,
    max_accel_mm_s2: u32,
    dt: u32,
) -> bool {
    new_position >= old_position
        && new_position - old_position <= 13_900 * u64::from(dt) + 1
        && new_speed <= 13_900
        && u64::from(new_speed.saturating_sub(old_speed))
            <= (u64::from(max_accel_mm_s2) * u64::from(dt)).div_ceil(1_000) + 1
}

impl Sample {
    pub fn observe(&mut self, tick: u64, arrivals_empty: bool) -> Result<blake3::Hash, Error> {
        if self.lane_change_route.is_some() {
            return self.observe_lane_change(tick, arrivals_empty);
        }
        let dt = self.state.identity.fixed_delta_ms;
        require(
            self.world.tick_index() == tick
                && self.world.time_ms() == tick * u64::from(dt)
                && self.world.command_cursor() == self.cursor
                && self.world.live_vehicles() == self.handles
                && arrivals_empty,
            || format!("人口/命令游标/时间/停车事件不符 tick={tick}"),
        )?;
        let signals = self.world.committed_signal_groups();
        let signals = signals.as_slice();
        let signalized = self.groups.iter().filter(|group| group.is_some()).count();
        require(signals.len() == signalized, || {
            format!("信号数量 tick={tick}")
        })?;
        for (rank, group) in self.groups.iter().enumerate() {
            let Some(group) = group else {
                continue;
            };
            let index = signals
                .binary_search_by_key(&group.raw(), |(g, _)| g.raw())
                .map_err(|_| Error::Invalid(format!("缺少信号组 {rank}")))?;
            require(
                signals[index].1 == aspect(rank as u32 / 4, rank as u32 % 4, tick, dt),
                || format!("信号灯态 tick={tick} route={rank}"),
            )?;
        }
        let bytes = &mut self.trace_bytes;
        bytes.clear();
        bytes.extend_from_slice(b"lf-synth-public-boundary-1\0");
        bytes.extend_from_slice(&tick.to_le_bytes());
        bytes.extend_from_slice(&self.world.time_ms().to_le_bytes());
        bytes.extend_from_slice(&self.cursor.to_le_bytes());
        bytes.extend_from_slice(&(self.handles.len() as u32).to_le_bytes());
        let mut event_index = 0;
        let events = self.world.latest_transition_events();
        let mut follower_front = None;
        for (rank, &handle) in self.handles.iter().enumerate() {
            let state = self
                .world
                .vehicle(handle)
                .ok_or_else(|| Error::Invalid(format!("句柄失效 rank={rank} tick={tick}")))?;
            let initial = self.initial[rank];
            let old = self.previous[rank];
            let route_rank = self.state.vehicles[rank].route_rank as usize;
            if rank % 25 == 0 {
                follower_front = None;
            }
            require(
                state.handle() == initial.handle()
                    && state.profile() == initial.profile()
                    && state.class() == initial.class()
                    && state.route() == initial.route()
                    && state.length_mm() == 4_500
                    && state.status() == initial.status()
                    && state.route_edge_index() <= 1
                    && u64::from(state.progress_mm()) <= EDGE_MM
                    && state.carry_um() < 1_000
                    && state.waiting_membership().is_none()
                    && state.maneuver_traversal().is_none()
                    && self.world.parking_binding(handle) == self.bindings[rank],
                || format!("身份/路线/状态/停车绑定 tick={tick} rank={rank}"),
            )?;
            if state.status() == VehicleStatus::Parked {
                require(state == initial, || {
                    format!("停车状态漂移 tick={tick} rank={rank}")
                })?;
                let Some(ParkingBinding::Occupied(ParkingTarget::ExplicitSpace(space))) =
                    self.bindings[rank]
                else {
                    return Err(Error::Invalid(format!("停车绑定类型 rank={rank}")));
                };
                require(
                    self.world.committed_parking_occupant(space) == Some(handle),
                    || format!("停车占用者 tick={tick} rank={rank}"),
                )?;
            } else {
                let next_position = position(state);
                let drop = old.speed_mm_s().saturating_sub(state.speed_mm_s());
                self.max_deceleration_mm_s2 = self
                    .max_deceleration_mm_s2
                    .max((u64::from(drop) * 1_000).div_ceil(u64::from(dt)));
                self.deceleration_above_profile_count += u64::from(drop > 6 * dt + 1);
                require(
                    motion_valid(
                        position(old),
                        next_position,
                        old.speed_mm_s(),
                        state.speed_mm_s(),
                        self.max_accel_mm_s2,
                        dt,
                    ),
                    || format!("进度/速度/加减速 tick={tick} rank={rank}: {old:?} -> {state:?}"),
                )?;
                if let Some(front) = follower_front {
                    require(
                        next_position >= front + u64::from(state.length_mm()) * 1_000,
                        || format!("车身重叠或超车 tick={tick} rank={rank}"),
                    )?;
                }
                follower_front = Some(next_position);
            }
            let changed = state.route_edge_index() != old.route_edge_index();
            if changed {
                require(
                    old.route_edge_index() == 0
                        && state.route_edge_index() == 1
                        && !self.crossed[rank]
                        && aspect(route_rank as u32 / 4, route_rank as u32 % 4, tick - 1, dt)
                            == SignalAspect::Green,
                    || format!("路线转换或闯灯 tick={tick} rank={rank}"),
                )?;
                self.crossed[rank] = true;
                self.transitions += 1;
                for kind_index in 0..2 {
                    let event = events.get(event_index).ok_or_else(|| {
                        Error::Invalid(format!("缺少转换事件 tick={tick} rank={rank}"))
                    })?;
                    event_index += 1;
                    let anchor = event.anchor();
                    let expected_kind = if kind_index == 0 {
                        TrafficTransitionKind::GateCrossed {
                            gate: self.gates[route_rank].expect("W 族唯一信号 Gate"),
                        }
                    } else {
                        TrafficTransitionKind::ManeuverTraversalCompleted {
                            maneuver_occurrence_index: 0,
                        }
                    };
                    require(
                        event.tick() == tick
                            && event.vehicle() == handle
                            && event.vehicle_update_sequence() == rank as u32
                            && event.kind() == expected_kind
                            && anchor.route() == self.routes[route_rank]
                            && anchor.maneuver_occurrence_index() == 0
                            && anchor.hop() == 0
                            && anchor.position().route_edge_index() == 1
                            && anchor.position().progress_mm() == 0
                            && anchor.position().carry_um() == 0,
                        || format!("事件顺序/载荷/因果 tick={tick} rank={rank}: {event:?}"),
                    )?;
                }
            }
            for field in [
                rank as u32,
                route_rank as u32,
                state.route_edge_index(),
                state.progress_mm(),
                u32::from(state.carry_um()),
                state.speed_mm_s(),
                if state.status() == VehicleStatus::Active {
                    0
                } else {
                    1
                },
                u32::from(changed),
            ] {
                bytes.extend_from_slice(&field.to_le_bytes());
            }
            self.previous[rank] = state;
        }
        require(event_index == events.len(), || {
            format!(
                "多余事件 tick={tick}: expected={event_index} actual={}",
                events.len()
            )
        })?;
        // 每个 changed 行对应已逐字段验证的固定 GateCrossed/Completed 事件对，顺序为 rank 升序。
        bytes.extend_from_slice(&(events.len() as u32).to_le_bytes());
        Ok(blake3::hash(bytes))
    }

    /// L 族逐拍公开观测（#1218，`lf-synth-public-boundary-lc-1`）：W 族 8 字段 +
    /// 逐车 T 字段（遍历相位、所持预约真实资源身份、acquired 拍）。语义检查含
    /// T 独占恰一、败者在前车预约释放后进入、换道预约生命周期事件序；比较面
    /// 按 #1217 口径携带真实资源身份，不以错误编码摘要互相印证。
    pub fn observe_lane_change(
        &mut self,
        tick: u64,
        arrivals_empty: bool,
    ) -> Result<blake3::Hash, Error> {
        let dt = self.state.identity.fixed_delta_ms;
        require(
            self.world.tick_index() == tick
                && self.world.time_ms() == tick * u64::from(dt)
                && self.world.command_cursor() == self.cursor
                && self.world.live_vehicles() == self.handles
                && arrivals_empty,
            || format!("人口/命令游标/时间/停车事件不符 tick={tick}"),
        )?;
        let cluster_routes = self.lane_change_routes.clone();
        require(!cluster_routes.is_empty(), || "L 族缺少簇路线".into())?;
        let changers_per_cluster = self
            .state
            .identity
            .lane_change()
            .map_or(0, |params| params.changers_per_cluster);
        require(changers_per_cluster >= 2, || "每簇至少两名换道者".into())?;
        let mut cluster_ranks: Vec<Vec<usize>> = Vec::with_capacity(cluster_routes.len());
        for &route in &cluster_routes {
            let ranks: Vec<usize> = (0..self.handles.len())
                .filter(|&rank| self.state.vehicles[rank].route_rank as usize == route)
                .collect();
            require(ranks.len() == changers_per_cluster as usize, || {
                format!("每簇换道者数与参数一致 route={route}")
            })?;
            cluster_ranks.push(ranks);
        }
        let bytes = &mut self.trace_bytes;
        bytes.clear();
        bytes.extend_from_slice(b"lf-synth-public-boundary-lc-1\0");
        bytes.extend_from_slice(&tick.to_le_bytes());
        bytes.extend_from_slice(&self.world.time_ms().to_le_bytes());
        bytes.extend_from_slice(&self.cursor.to_le_bytes());
        bytes.extend_from_slice(&(self.handles.len() as u32).to_le_bytes());
        let mut in_transition = vec![0_u32; cluster_routes.len()];
        for (rank, &handle) in self.handles.iter().enumerate() {
            let state = self
                .world
                .vehicle(handle)
                .ok_or_else(|| Error::Invalid(format!("句柄失效 rank={rank} tick={tick}")))?;
            let initial = self.initial[rank];
            let old = self.previous[rank];
            let on_cluster = self
                .lane_change_routes
                .contains(&(self.state.vehicles[rank].route_rank as usize));
            require(
                state.handle() == initial.handle()
                    && state.profile() == initial.profile()
                    && state.class() == initial.class()
                    && state.route() == initial.route()
                    && state.length_mm() == 4_500
                    && state.status() == initial.status()
                    && state.route_edge_index() <= if on_cluster { 4 } else { 1 }
                    && state.carry_um() < 1_000
                    && self.world.parking_binding(handle) == self.bindings[rank],
                || format!("身份/路线/状态/停车绑定 tick={tick} rank={rank}"),
            )?;
            if state.status() == VehicleStatus::Parked {
                require(state == initial, || {
                    format!("停车状态漂移 tick={tick} rank={rank}")
                })?;
            } else {
                // L 族边长非均匀：位置编码用真实边长前缀和（毫米）。
                let offsets = &self.route_offsets[self.state.vehicles[rank].route_rank as usize];
                let pos = |s: VehicleState| {
                    (offsets[s.route_edge_index() as usize] + u64::from(s.progress_mm())) * 1_000
                        + u64::from(s.carry_um())
                };
                require(
                    motion_valid(
                        pos(old),
                        pos(state),
                        old.speed_mm_s(),
                        state.speed_mm_s(),
                        self.max_accel_mm_s2,
                        dt,
                    ),
                    || {
                        format!(
                            "进度/速度/加减速 tick={tick} rank={rank}: pos {} -> {} speed {} -> {} dt={}",
                            pos(old),
                            pos(state),
                            old.speed_mm_s(),
                            state.speed_mm_s(),
                            dt
                        )
                    },
                )?;
                // 减速诊断（#1228）：L1 分支同样累加实测值（阈值口径与 W 族一致）。
                let drop = old.speed_mm_s().saturating_sub(state.speed_mm_s());
                self.max_deceleration_mm_s2 = self
                    .max_deceleration_mm_s2
                    .max((u64::from(drop) * 1_000).div_ceil(u64::from(dt)));
                self.deceleration_above_profile_count += u64::from(drop > 6 * dt + 1);
            }
            let edge_changed = state.route_edge_index() != old.route_edge_index();
            if on_cluster {
                let (cluster_index, slot) = cluster_ranks
                    .iter()
                    .enumerate()
                    .find_map(|(index, ranks)| {
                        ranks
                            .iter()
                            .position(|&r| r == rank)
                            .map(|slot| (index, slot))
                    })
                    .unwrap();
                if state.route_edge_index() == 1 {
                    in_transition[cluster_index] += 1;
                }
                if edge_changed && state.route_edge_index() == 1 {
                    self.crossed[rank] = true;
                    self.transitions += 1;
                    if slot > 0 {
                        // R1b 层②（多簇）：同簇更早槽位持有 T 预约时不得进入。
                        for &earlier in &cluster_ranks[cluster_index][..slot] {
                            let earlier_handle = self.handles[earlier];
                            let earlier_t_held = self
                                .world
                                .conflict_reservation(earlier_handle)
                                .is_some_and(|r| {
                                    let range = r.passage_range();
                                    self.world
                                        .conflict_passage_occurrence_locator(
                                            range.route(),
                                            range.first_conflict_occurrence_index(),
                                        )
                                        .is_some_and(|locator| {
                                            crate::measurement::events::address_identity(
                                                &locator.address(),
                                            ) == crate::measurement::events::T_RESOURCE_IDENTITY
                                        })
                                });
                            require(!earlier_t_held, || {
                                format!("换道者在前车 T 预约存续期进入 T tick={tick}")
                            })?;
                        }
                    }
                }
            }
            let phase = match state.maneuver_traversal().map(|t| t.phase()) {
                None => 0_u32,
                Some(ManeuverTraversalPhase::PreGate { .. }) => 1,
                Some(ManeuverTraversalPhase::Committed { .. }) => 2,
                Some(ManeuverTraversalPhase::Waiting { .. }) => 3,
                Some(ManeuverTraversalPhase::Clearing { .. }) => 4,
            };
            let (held, held_identity, acquired) = match self.world.conflict_reservation(handle) {
                Some(reservation) => {
                    let range = reservation.passage_range();
                    let locator = self
                        .world
                        .conflict_passage_occurrence_locator(
                            range.route(),
                            range.first_conflict_occurrence_index(),
                        )
                        .ok_or_else(|| Error::Invalid(format!("预约身份缺失 rank={rank}")))?;
                    (
                        1_u32,
                        crate::measurement::events::address_identity(&locator.address()),
                        reservation.acquired_tick(),
                    )
                }
                None => (0, 0, 0),
            };
            for field in [
                rank as u32,
                self.state.vehicles[rank].route_rank,
                state.route_edge_index(),
                state.progress_mm(),
                u32::from(state.carry_um()),
                state.speed_mm_s(),
                if state.status() == VehicleStatus::Active {
                    0
                } else {
                    1
                },
                u32::from(edge_changed),
                phase,
                held,
                held_identity,
            ] {
                bytes.extend_from_slice(&field.to_le_bytes());
            }
            bytes.extend_from_slice(&acquired.to_le_bytes());
            self.previous[rank] = state;
        }
        // T 独占恰一（R1b 层② B 裁定，多簇按簇独立）：每簇逐拍至多一换道者
        // 在过渡段内。
        for (cluster, count) in in_transition.iter().enumerate() {
            require(*count <= 1, || {
                format!("簇 {cluster} T 独占恰一被破坏 tick={tick} in_transition={count}")
            })?;
        }
        let events = self.world.latest_transition_events();
        for event in events {
            let vehicle = event.vehicle();
            let slot = cluster_ranks
                .iter()
                .flat_map(|ranks| ranks.iter())
                .position(|&rank| self.handles[rank] == vehicle)
                .ok_or_else(|| Error::Invalid(format!("换道事件归属非换道者 tick={tick}")))?;
            require(slot < self.lane_change_last_kind.len(), || {
                "换道事件槽位超出生命周期表".into()
            })?;
            let (kind, identity) = match event.kind() {
                TrafficTransitionKind::GateCrossed { gate } => (1_u32, gate.raw()),
                TrafficTransitionKind::ReservationAcquired { passage_range } => (
                    4_u32,
                    crate::measurement::events::passage_identity(&self.world, passage_range)?,
                ),
                TrafficTransitionKind::ConflictEntered { passage } => (
                    5,
                    crate::measurement::events::address_identity(&passage.address()),
                ),
                TrafficTransitionKind::ConflictCleared { passage } => (
                    6,
                    crate::measurement::events::address_identity(&passage.address()),
                ),
                TrafficTransitionKind::ReservationReleased { passage_range } => (
                    7,
                    crate::measurement::events::passage_identity(&self.world, passage_range)?,
                ),
                TrafficTransitionKind::ManeuverTraversalCompleted { .. } => (8, 0),
                other => {
                    return Err(Error::Invalid(format!(
                        "L 族未声明的事件类型 tick={tick}: {other:?}"
                    )));
                }
            };
            if (4..=7).contains(&kind) {
                // 预约生命周期序：acquire(4) → enter(5) → clear(6) → release(7)。
                // 跨拍持续（#1227）：last_kind 保存在 Sample，避免每拍重置漏检跨拍乱序。
                require(
                    lifecycle_order_valid(&mut self.lane_change_last_kind, slot, kind),
                    || format!("换道预约生命周期事件乱序 tick={tick} slot={slot} kind={kind}"),
                )?;
            }
            self.lane_change_journal
                .push((tick, slot as u16, kind, identity));
        }
        bytes.extend_from_slice(&(events.len() as u32).to_le_bytes());
        Ok(blake3::hash(bytes))
    }
}

#[cfg(test)]
mod lifecycle_order_tests {
    use super::lifecycle_order_valid;

    #[test]
    fn lifecycle_order_is_tracked_across_observations() {
        // #1227：last_kind 跨观测持续，单拍重置会漏检跨拍乱序；释放后可开启
        // 新预约周期（下游冲突区），同周期内不得回退或重复。
        let mut last = [0_u32; 2];
        for kind in [4, 5, 6, 7] {
            assert!(lifecycle_order_valid(&mut last, 0, kind));
        }
        assert!(
            lifecycle_order_valid(&mut last, 0, 4),
            "release 后可开启新预约周期"
        );
        assert!(
            !lifecycle_order_valid(&mut last, 0, 4),
            "同周期重复 acquire 必须拒绝"
        );
        assert!(
            !lifecycle_order_valid(&mut last, 0, 6),
            "acquire 后 clear 必须拒绝"
        );
        assert!(lifecycle_order_valid(&mut last, 1, 4), "槽位独立");
    }
}

#[cfg(test)]
mod tests {
    use super::motion_valid;

    #[test]
    fn motion_valid_uses_profile_max_acceleration_bound() {
        // #1218：加速度上界按车型 profile 派生，不再全局写死 1.4 m/s²。
        // dt=33 ms：1.5 m/s² 允许 ceil(49.5)+1=51 mm/s；1.4 m/s² 允许 48 mm/s。
        assert!(motion_valid(0, 49_000, 0, 49, 1_500, 33));
        assert!(!motion_valid(0, 49_000, 0, 49, 1_400, 33));
        assert!(motion_valid(0, 48_000, 0, 48, 1_400, 33));
        // dt=16 ms：两种车型均允许 24 mm/s；1.4 m/s² 下 25 mm/s 仍拒。
        assert!(motion_valid(0, 24_000, 0, 24, 1_400, 16));
        assert!(motion_valid(0, 24_000, 0, 24, 1_500, 16));
        assert!(!motion_valid(0, 25_000, 0, 25, 1_400, 16));
        // 位置与速度硬上界不随车型变化。
        assert!(!motion_valid(0, 13_900 * 33 + 2, 0, 0, 1_500, 33));
        assert!(!motion_valid(0, 1, 0, 13_901, 1_500, 33));
    }
}
