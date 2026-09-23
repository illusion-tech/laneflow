//! 新鲜摆放的运动安全准入。只在 `spawn_vehicle` 与 `replace_completed_vehicle`
//! 提交前使用；快照恢复和修订切换不调用。

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use laneflow_static_contract::VehicleProfileOrdinal;

use super::occupancy::LeaderQueryHorizon;
use super::tables::{for_each_admission_interval, occupancy_front_gap};
use super::tick::{PlacementMotion, leader_query_horizon};
use crate::kernel::units::ceil_mm;
use crate::{SpawnError, VehicleHandle, VehicleSpawnInput, VehicleState, VehicleStatus};

#[cfg(test)]
use std::cell::Cell;

#[cfg(test)]
thread_local! {
    static FOLLOWER_CANDIDATES: Cell<u64> = const { Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_follower_candidates() {
    FOLLOWER_CANDIDATES.set(0);
}

#[cfg(test)]
pub(crate) fn follower_candidates() -> u64 {
    FOLLOWER_CANDIDATES.with(Cell::get)
}

/// 新鲜摆放在重叠和权威检查之后仍可能失败的原因。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FreshAdmissionFailure {
    /// 当前必须停车的门按紧急制动停不住。
    StopConstraint,
    /// 前方更低限速按紧急制动降不到。
    DownstreamSpeed,
    /// 已有移动后车会把候选当成直接前车，且无法安全制动。
    UnsafeFollower(VehicleHandle),
    /// 候选相对最近前车无法安全制动。
    UnsafeLeader(VehicleHandle),
    /// 派生占用索引重建时分配失败。
    OccupancyAlloc,
}

impl FreshAdmissionFailure {
    pub(crate) fn into_spawn(self) -> SpawnError {
        match self {
            Self::StopConstraint => SpawnError::StopConstraintUnsatisfiable,
            Self::DownstreamSpeed => SpawnError::DownstreamSpeedUnsatisfiable,
            Self::UnsafeFollower(follower) => SpawnError::UnsafeFollower { follower },
            Self::UnsafeLeader(leader) => SpawnError::UnsafeLeader { leader },
            Self::OccupancyAlloc => SpawnError::OccupancyAllocFailed,
        }
    }

    pub(crate) fn into_replace(self) -> crate::ReplaceError {
        match self {
            Self::StopConstraint => crate::ReplaceError::StopConstraintUnsatisfiable,
            Self::DownstreamSpeed => crate::ReplaceError::DownstreamSpeedUnsatisfiable,
            Self::UnsafeFollower(follower) => crate::ReplaceError::UnsafeFollower { follower },
            Self::UnsafeLeader(leader) => crate::ReplaceError::UnsafeLeader { leader },
            Self::OccupancyAlloc => crate::ReplaceError::OccupancyAllocFailed,
        }
    }
}

/// 速度为 0 视为已经停住。紧急减速度不是有限正数时，无法证明能在给定距离内停住。
/// 红灯到达下界与新鲜摆放共用这个判断。
pub(crate) fn can_stop_before(
    speed_mm_s: u32,
    emergency_decel_m_s2: f32,
    distance_mm: u32,
) -> bool {
    can_slow_to_before(speed_mm_s, 0, emergency_decel_m_s2, distance_mm)
}

/// 当前速度已经不高于目标速度时通过。否则要求紧急制动距离不超过剩余房间。
pub(crate) fn can_slow_to_before(
    speed_mm_s: u32,
    target_mm_s: u32,
    emergency_decel_m_s2: f32,
    distance_mm: u32,
) -> bool {
    if speed_mm_s <= target_mm_s {
        return true;
    }
    if !emergency_decel_m_s2.is_finite() || emergency_decel_m_s2 <= 0.0 {
        return false;
    }
    let decel_mm_s2 = f64::from(emergency_decel_m_s2) * 1_000.0;
    let speed = f64::from(speed_mm_s);
    let target = f64::from(target_mm_s);
    let needed_mm = (speed * speed - target * target) / (2.0 * decel_mm_s2);
    needed_mm.is_finite() && needed_mm <= f64::from(distance_mm)
}

/// 驶离停车使用的静止前车算式。前车速度不为 0 时仍按调用方传入的速度计算，
/// 新鲜摆放不再走这里。
pub(crate) fn moving_follower_can_admit(
    follower_speed_mm_s: u32,
    follower_emergency_m_s2: f32,
    follower_min_gap_mm: u32,
    gap_mm: u32,
    leader_speed_mm_s: u32,
    leader_emergency_m_s2: f32,
    delta_s: f32,
) -> bool {
    let v = follower_speed_mm_s as f32 / 1_000.0;
    let emergency = follower_emergency_m_s2;
    let gap_m = gap_mm as f32 / 1_000.0;
    let preserved_gap_mm = gap_mm.min(follower_min_gap_mm);
    let raw_available_gap_mm = gap_mm.saturating_sub(preserved_gap_mm);
    let available_gap_mm = if raw_available_gap_mm <= 1 {
        0
    } else {
        raw_available_gap_mm
    };
    if ![v, emergency, delta_s, gap_m]
        .into_iter()
        .all(f32::is_finite)
        || emergency <= 0.0
        || delta_s <= 0.0
    {
        return false;
    }
    let u_min = (v - emergency * delta_s).max(0.0);
    let safe_envelope = 0.5 * (v + u_min) * delta_s + u_min * u_min / (2.0 * emergency);
    let emergency_min_travel = if v <= emergency * delta_s {
        v * v / (2.0 * emergency)
    } else {
        v * delta_s - 0.5 * emergency * delta_s * delta_s
    };
    let (leader_stop_m, leader_min_travel_m) = if leader_speed_mm_s == 0 {
        (0.0, 0.0)
    } else {
        let leader_v = leader_speed_mm_s as f32 / 1_000.0;
        if !leader_v.is_finite()
            || !leader_emergency_m_s2.is_finite()
            || leader_emergency_m_s2 <= 0.0
        {
            return false;
        }
        let leader_stop = leader_v * leader_v / (2.0 * leader_emergency_m_s2);
        let leader_min_travel = if leader_v <= leader_emergency_m_s2 * delta_s {
            leader_v * leader_v / (2.0 * leader_emergency_m_s2)
        } else {
            leader_v * delta_s - 0.5 * leader_emergency_m_s2 * delta_s * delta_s
        };
        if !leader_stop.is_finite() || !leader_min_travel.is_finite() {
            return false;
        }
        (leader_stop, leader_min_travel)
    };
    let available_m = available_gap_mm as f32 / 1_000.0;
    safe_envelope.is_finite()
        && emergency_min_travel.is_finite()
        && safe_envelope <= gap_m + leader_stop_m
        && emergency_min_travel <= available_m + leader_min_travel_m
}

fn room_to_edge_end(
    compiled: &super::tables::CompiledRoute,
    lengths: &[u32],
    cursor: usize,
    progress_mm: u32,
    end_hop: usize,
) -> Option<u32> {
    let current = *compiled.edges.get(cursor)?;
    let mut room = lengths.get(current.index())?.saturating_sub(progress_mm);
    for index in (cursor + 1)..=end_hop {
        let edge = *compiled.edges.get(index)?;
        room = room.saturating_add(*lengths.get(edge.index())?);
    }
    Some(room)
}

impl crate::kernel::state::WorldState {
    /// 重叠与权威已经通过之后，检查当前约束和前后车。失败不提交车辆。
    pub(crate) fn fresh_motion_admission(
        &mut self,
        input: VehicleSpawnInput,
        vehicle_length_mm: u32,
    ) -> Result<(), FreshAdmissionFailure> {
        let profile = self
            .binding
            .revision
            .traffic()
            .relations()
            .vehicle_profile(input.profile())
            .expect("未停车校验已经解析过车辆 profile");
        let emergency = profile.emergency_decel();
        let cursor = usize::try_from(input.route_edge_index()).expect("route index fits usize");
        if let Some(distance_mm) = self.restrictive_stop_mm(
            input.route(),
            input.route_edge_index(),
            input.progress_mm(),
            input.profile(),
        ) && !can_stop_before(input.initial_speed_mm_s(), emergency, distance_mm)
        {
            return Err(FreshAdmissionFailure::StopConstraint);
        }
        if self.downstream_speed_infeasible(
            input.route(),
            cursor,
            input.progress_mm(),
            input.initial_speed_mm_s(),
            emergency,
        ) {
            return Err(FreshAdmissionFailure::DownstreamSpeed);
        }
        self.ensure_current_occupancy()
            .map_err(|_| FreshAdmissionFailure::OccupancyAlloc)?;
        let delta_s = self.binding.config.fixed_delta_time_ms() as f32 / 1_000.0;
        self.admit_own_motion(input, profile, vehicle_length_mm, delta_s)?;
        self.admit_nearest_leader(input, profile, vehicle_length_mm, delta_s)?;
        self.admit_direct_followers(input, vehicle_length_mm, delta_s)
    }

    fn restrictive_stop_mm(
        &self,
        route: crate::RouteHandle,
        route_edge_index: u32,
        progress_mm: u32,
        profile: VehicleProfileOrdinal,
    ) -> Option<u32> {
        let compiled = self.compiled_route(route)?;
        let read = self.read_view();
        let lengths = self.binding.revision.traffic().lane_lengths_millimetres();
        let rolled = progress_mm == 0 && route_edge_index > 0;
        let mut hop = usize::try_from(if rolled {
            route_edge_index - 1
        } else {
            route_edge_index
        })
        .ok()?;
        let progress_base = if rolled {
            let edge = *compiled.edges.get(hop)?;
            *lengths.get(edge.index())?
        } else {
            progress_mm
        };
        let edge = *compiled.edges.get(hop)?;
        let mut room = lengths.get(edge.index())?.saturating_sub(progress_base);
        loop {
            if let Some(gate) = compiled.hop_gate.get(hop).copied().flatten()
                && read.gate_is_restrictive(gate, profile)
            {
                return Some(room);
            }
            hop = hop.checked_add(1)?;
            let edge = compiled.edges.get(hop).copied()?;
            room = room.saturating_add(*lengths.get(edge.index())?);
        }
    }

    fn downstream_speed_infeasible(
        &self,
        route: crate::RouteHandle,
        cursor: usize,
        progress_mm: u32,
        speed_mm_s: u32,
        emergency_decel_m_s2: f32,
    ) -> bool {
        let Some(compiled) = self.compiled_route(route) else {
            return false;
        };
        let lengths = self.binding.revision.traffic().lane_lengths_millimetres();
        let Some(current) = compiled.edges.get(cursor).copied() else {
            return false;
        };
        let Some(current_length) = lengths.get(current.index()).copied() else {
            return false;
        };
        // 降速点按路线顺序写出。从当前位置向前累加一次，不再为每个点从头重走。
        let mut room = current_length.saturating_sub(progress_mm);
        let mut walked = cursor;
        for drop in &compiled.speed_limit_drop {
            let Ok(from) = usize::try_from(drop.from_route_edge_index) else {
                continue;
            };
            if from < cursor || speed_mm_s <= drop.target_mm_s {
                continue;
            }
            let room_mm = if from < walked {
                let Some(restarted) =
                    room_to_edge_end(compiled, lengths, cursor, progress_mm, from)
                else {
                    continue;
                };
                restarted
            } else {
                while walked < from {
                    walked = walked.saturating_add(1);
                    let Some(edge) = compiled.edges.get(walked).copied() else {
                        return false;
                    };
                    let Some(length) = lengths.get(edge.index()).copied() else {
                        return false;
                    };
                    room = room.saturating_add(length);
                }
                room
            };
            if !can_slow_to_before(speed_mm_s, drop.target_mm_s, emergency_decel_m_s2, room_mm) {
                return true;
            }
        }
        false
    }

    /// 没有前车时也预览新车自己的第一拍。红灯、未放行的边末、路的尽头，
    /// 以及已经被其他车占用的冲突区，都在这次预览里。
    fn admit_own_motion(
        &self,
        input: VehicleSpawnInput,
        profile: laneflow_static_network::VehicleProfileView,
        vehicle_length_mm: u32,
        delta_s: f32,
    ) -> Result<(), FreshAdmissionFailure> {
        let state = preview_vehicle(input, profile, vehicle_length_mm);
        let motion = self
            .read_view()
            .placement_motion(state, None, false)
            .ok_or(FreshAdmissionFailure::StopConstraint)?;
        if projection_exceeds_emergency(
            input.initial_speed_mm_s(),
            motion,
            profile.emergency_decel(),
            delta_s,
        ) {
            return Err(FreshAdmissionFailure::StopConstraint);
        }
        Ok(())
    }

    fn admit_nearest_leader(
        &self,
        input: VehicleSpawnInput,
        profile: laneflow_static_network::VehicleProfileView,
        vehicle_length_mm: u32,
        delta_s: f32,
    ) -> Result<(), FreshAdmissionFailure> {
        let lengths = self.binding.revision.traffic().lane_lengths_millimetres();
        let follower_edges = self.route_edges(input.route()).expect("已校验的路线仍在");
        let follower_index =
            usize::try_from(input.route_edge_index()).expect("route index fits usize");
        let Some(horizon) = leader_query_horizon(input.initial_speed_mm_s(), profile, delta_s)
        else {
            return Err(FreshAdmissionFailure::UnsafeLeader(VehicleHandle::new(
                0, 0,
            )));
        };
        let Some(contact) = self.derived.occupancy.nearest_leader(
            VehicleHandle::new(u32::MAX, 0),
            follower_edges,
            follower_index,
            input.progress_mm(),
            lengths,
            horizon,
            true,
        ) else {
            return Ok(());
        };
        if self.vehicle_state(contact.vehicle).is_none() {
            return Err(FreshAdmissionFailure::UnsafeLeader(contact.vehicle));
        }
        let state = preview_vehicle(input, profile, vehicle_length_mm);
        let motion = self
            .read_view()
            .placement_motion(state, Some(contact.gap_mm), false)
            .ok_or(FreshAdmissionFailure::UnsafeLeader(contact.vehicle))?;
        if projection_exceeds_emergency(
            input.initial_speed_mm_s(),
            motion,
            profile.emergency_decel(),
            delta_s,
        ) {
            return Err(FreshAdmissionFailure::UnsafeLeader(contact.vehicle));
        }
        Ok(())
    }

    fn admit_direct_followers(
        &mut self,
        input: VehicleSpawnInput,
        vehicle_length_mm: u32,
        delta_s: f32,
    ) -> Result<(), FreshAdmissionFailure> {
        let candidates = self.upstream_follower_candidates(input, vehicle_length_mm)?;
        let lengths = self.binding.revision.traffic().lane_lengths_millimetres();
        let candidate_edges = self.route_edges(input.route()).expect("已校验的路线仍在");
        let candidate_index =
            usize::try_from(input.route_edge_index()).expect("route index fits usize");
        #[cfg(test)]
        FOLLOWER_CANDIDATES.with(|count| {
            count.set(count.get().saturating_add(candidates.len() as u64));
        });
        for handle in candidates {
            let Some(follower) = self.vehicle_state(handle).copied() else {
                continue;
            };
            if follower.status != VehicleStatus::Active || follower.speed_mm_s == 0 {
                continue;
            }
            let Some(follower_edges) = self.route_edges(follower.route) else {
                return Err(FreshAdmissionFailure::UnsafeFollower(handle));
            };
            let Ok(follower_index) = usize::try_from(follower.route_edge_index) else {
                return Err(FreshAdmissionFailure::UnsafeFollower(handle));
            };
            let Some(candidate_gap) = occupancy_front_gap(
                lengths,
                follower_edges,
                follower_index,
                follower.progress_mm,
                candidate_edges,
                candidate_index,
                input.progress_mm(),
                vehicle_length_mm,
            ) else {
                continue;
            };
            if candidate_gap < 0 {
                continue;
            }
            let candidate_gap_limit = u32::try_from(candidate_gap).unwrap_or(u32::MAX);
            if self
                .derived
                .occupancy
                .leader_gap(
                    handle,
                    follower_edges,
                    follower_index,
                    follower.progress_mm,
                    lengths,
                    LeaderQueryHorizon::new(candidate_gap_limit, u32::MAX),
                )
                .is_some()
            {
                continue;
            }
            let profile = self
                .binding
                .revision
                .traffic()
                .relations()
                .vehicle_profile(follower.profile)
                .ok_or(FreshAdmissionFailure::UnsafeFollower(handle))?;
            let motion = self
                .read_view()
                .placement_motion(follower, Some(candidate_gap), true)
                .ok_or(FreshAdmissionFailure::UnsafeFollower(handle))?;
            if projection_exceeds_emergency(
                follower.speed_mm_s,
                motion,
                profile.emergency_decel(),
                delta_s,
            ) {
                return Err(FreshAdmissionFailure::UnsafeFollower(handle));
            }
        }
        Ok(())
    }

    /// 车身所在边，以及制动窗内能开到候选车的上游边。精确路线过滤仍在调用方。
    fn upstream_follower_candidates(
        &mut self,
        input: VehicleSpawnInput,
        vehicle_length_mm: u32,
    ) -> Result<Vec<VehicleHandle>, FreshAdmissionFailure> {
        let reach = self.max_follower_bumper_mm();
        let traffic = self.binding.revision.traffic();
        self.workspace
            .occupancy_scratch
            .ensure_maneuver_upstream(traffic)
            .map_err(|_| FreshAdmissionFailure::OccupancyAlloc)?;
        let lengths = traffic.lane_lengths_millimetres();
        let Some(edges) = self.route_edges(input.route()) else {
            return Ok(Vec::new());
        };
        let Ok(cursor) = usize::try_from(input.route_edge_index()) else {
            return Ok(Vec::new());
        };
        let mut body: Vec<(laneflow_static_contract::LaneEdgeOrdinal, u32)> = Vec::new();
        let mut alloc_failed = false;
        let _ = for_each_admission_interval(
            lengths,
            edges,
            cursor,
            input.progress_mm(),
            vehicle_length_mm,
            |edge, lo, _| {
                if alloc_failed {
                    return;
                }
                // 环线上同一条物理边可以有两段车身。两段后杠都要留，不能只留更小的那个。
                if body.try_reserve(1).is_err() {
                    alloc_failed = true;
                } else {
                    body.push((edge, lo));
                }
            },
        );
        if alloc_failed {
            return Err(FreshAdmissionFailure::OccupancyAlloc);
        }
        if body.is_empty()
            && let Some(edge) = edges.get(cursor).copied()
        {
            push_fallible(&mut body, (edge, input.progress_mm()))?;
        }
        let mut found = Vec::new();
        let mut search = UpstreamSearch {
            reach,
            body_edges: Vec::new(),
            best_behind: Vec::new(),
            pending: BinaryHeap::new(),
        };
        // 车身跨边时，后杠所在边也是更前方车身的车道前驱。先按后杠开窗。
        // 上游边保留目前更短的到达距离；更长的路径不能把这条边占死。
        for (edge, rear_lo) in body {
            push_fallible(&mut search.body_edges, edge.raw())?;
            collect_follower_window(
                &self.derived.occupancy,
                edge,
                rear_lo.saturating_sub(reach),
                rear_lo,
                &mut found,
            )?;
            if rear_lo <= reach {
                relax_follower_upstream(
                    traffic,
                    &self.workspace.occupancy_scratch,
                    edge,
                    rear_lo,
                    &mut search,
                )?;
            }
        }
        while let Some(Reverse((behind_end, raw))) = search.pending.pop() {
            if search
                .best_behind
                .iter()
                .find(|(edge, _)| *edge == raw)
                .is_none_or(|(_, best)| *best != behind_end)
                || behind_end > search.reach
                || search.body_edges.contains(&raw)
            {
                continue;
            }
            let edge = laneflow_static_contract::LaneEdgeOrdinal::from_raw(raw);
            let length = lengths.get(edge.index()).copied().unwrap_or(0);
            let budget = reach - behind_end;
            collect_follower_window(
                &self.derived.occupancy,
                edge,
                length.saturating_sub(budget),
                length,
                &mut found,
            )?;
            let next = behind_end.saturating_add(length);
            if next <= reach {
                relax_follower_upstream(
                    traffic,
                    &self.workspace.occupancy_scratch,
                    edge,
                    next,
                    &mut search,
                )?;
            }
        }
        found.sort_by_key(|(sequence, handle)| (handle.index(), *sequence));
        found.dedup_by_key(|(_, handle)| handle.index());
        found.sort_by_key(|(sequence, handle)| (*sequence, handle.index()));
        let mut handles = Vec::new();
        handles
            .try_reserve(found.len())
            .map_err(|_| FreshAdmissionFailure::OccupancyAlloc)?;
        handles.extend(found.into_iter().map(|(_, handle)| handle));
        Ok(handles)
    }

    fn max_follower_bumper_mm(&mut self) -> u32 {
        if let Some(reach) = self.workspace.occupancy_scratch.follower_bumper_mm() {
            return reach;
        }
        let traffic = self.binding.revision.traffic();
        let delta_s = self.binding.config.fixed_delta_time_ms() as f32 / 1_000.0;
        let speed = traffic
            .lane_speed_limits_millimetres_per_second()
            .iter()
            .copied()
            .max()
            .unwrap_or(0);
        let profiles = traffic
            .entity_counts()
            .count(laneflow_static_contract::EntityKind::VehicleProfile);
        let mut reach = 0u32;
        for raw in 0..profiles {
            let Some(profile) = traffic
                .relations()
                .vehicle_profile(VehicleProfileOrdinal::from_raw(raw))
            else {
                continue;
            };
            if let Some(horizon) = leader_query_horizon(speed, profile, delta_s) {
                reach = reach.max(horizon.bumper_gap_mm);
            }
        }
        self.workspace
            .occupancy_scratch
            .remember_follower_bumper_mm(reach);
        reach
    }
}

fn collect_follower_window(
    occupancy: &super::occupancy::OccupancyIndex,
    edge: laneflow_static_contract::LaneEdgeOrdinal,
    min_hi: u32,
    max_hi: u32,
    found: &mut Vec<(u32, VehicleHandle)>,
) -> Result<(), FreshAdmissionFailure> {
    let mut alloc_failed = false;
    occupancy.for_each_record_in_hi_window(edge, min_hi, max_hi, |vehicle, sequence| {
        if alloc_failed {
            return;
        }
        if found.try_reserve(1).is_err() {
            alloc_failed = true;
        } else {
            found.push((sequence, vehicle));
        }
    });
    if alloc_failed {
        Err(FreshAdmissionFailure::OccupancyAlloc)
    } else {
        Ok(())
    }
}

struct UpstreamSearch {
    reach: u32,
    body_edges: Vec<u32>,
    best_behind: Vec<(u32, u32)>,
    pending: BinaryHeap<Reverse<(u32, u32)>>,
}

fn relax_follower_upstream(
    traffic: &laneflow_static_network::SharedTrafficNetwork,
    scratch: &super::occupancy::OccupancyScratch,
    edge: laneflow_static_contract::LaneEdgeOrdinal,
    behind_end: u32,
    search: &mut UpstreamSearch,
) -> Result<(), FreshAdmissionFailure> {
    if behind_end > search.reach {
        return Ok(());
    }
    if let Some(predecessors) = traffic.predecessors(edge) {
        for predecessor in predecessors {
            note_shorter_upstream(*predecessor, behind_end, search)?;
        }
    }
    for raw in scratch.maneuver_upstream(edge) {
        note_shorter_upstream(
            laneflow_static_contract::LaneEdgeOrdinal::from_raw(*raw),
            behind_end,
            search,
        )?;
    }
    Ok(())
}

fn preview_vehicle(
    input: VehicleSpawnInput,
    profile: laneflow_static_network::VehicleProfileView,
    vehicle_length_mm: u32,
) -> VehicleState {
    VehicleState {
        handle: VehicleHandle::new(u32::MAX, 0),
        profile: input.profile(),
        class: profile.class(),
        route: input.route(),
        route_edge_index: input.route_edge_index(),
        progress_mm: input.progress_mm(),
        carry_um: 0,
        speed_mm_s: input.initial_speed_mm_s(),
        length_mm: vehicle_length_mm,
        status: VehicleStatus::Active,
        maneuver_traversal: None,
        waiting_membership: None,
    }
}

fn note_shorter_upstream(
    edge: laneflow_static_contract::LaneEdgeOrdinal,
    behind_end: u32,
    search: &mut UpstreamSearch,
) -> Result<(), FreshAdmissionFailure> {
    let raw = edge.raw();
    if search.body_edges.contains(&raw) {
        return Ok(());
    }
    if let Some((_, best)) = search.best_behind.iter_mut().find(|(item, _)| *item == raw) {
        if *best <= behind_end {
            return Ok(());
        }
        *best = behind_end;
    } else {
        push_fallible(&mut search.best_behind, (raw, behind_end))?;
    }
    search
        .pending
        .try_reserve(1)
        .map_err(|_| FreshAdmissionFailure::OccupancyAlloc)?;
    search.pending.push(Reverse((behind_end, raw)));
    Ok(())
}

fn push_fallible<T>(items: &mut Vec<T>, value: T) -> Result<(), FreshAdmissionFailure> {
    items
        .try_reserve(1)
        .map_err(|_| FreshAdmissionFailure::OccupancyAlloc)?;
    items.push(value);
    Ok(())
}

fn projection_exceeds_emergency(
    speed_mm_s: u32,
    motion: PlacementMotion,
    emergency_m_s2: f32,
    delta_s: f32,
) -> bool {
    if !motion.hard_clamped {
        return false;
    }
    let Some(min_travel_mm) = emergency_min_travel_mm(speed_mm_s, emergency_m_s2, delta_s) else {
        return true;
    };
    if motion.committed_travel_mm < min_travel_mm {
        return true;
    }
    let Some(floor_mm_s) = emergency_floor_mm_s(speed_mm_s, emergency_m_s2, delta_s) else {
        return true;
    };
    motion.next_speed_mm_s < floor_mm_s
}

fn emergency_min_travel_mm(speed_mm_s: u32, emergency_m_s2: f32, delta_s: f32) -> Option<u32> {
    let speed_m_s = speed_mm_s as f32 / 1_000.0;
    if ![speed_m_s, emergency_m_s2, delta_s]
        .into_iter()
        .all(f32::is_finite)
        || emergency_m_s2 <= 0.0
        || delta_s <= 0.0
    {
        return None;
    }
    let travel_m = if speed_m_s <= emergency_m_s2 * delta_s {
        speed_m_s * speed_m_s / (2.0 * emergency_m_s2)
    } else {
        speed_m_s * delta_s - 0.5 * emergency_m_s2 * delta_s * delta_s
    };
    if !travel_m.is_finite() || travel_m < 0.0 {
        return None;
    }
    ceil_mm(f64::from(travel_m))
}

fn emergency_floor_mm_s(speed_mm_s: u32, emergency_m_s2: f32, delta_s: f32) -> Option<u32> {
    let drop_mm_s = f64::from(emergency_m_s2) * 1_000.0 * f64::from(delta_s);
    if !drop_mm_s.is_finite() || drop_mm_s < 0.0 {
        return None;
    }
    let speed = f64::from(speed_mm_s);
    if drop_mm_s >= speed {
        return Some(0);
    }
    let floor = (speed - drop_mm_s).ceil();
    if floor > f64::from(u32::MAX) {
        return None;
    }
    Some(floor as u32)
}
