//! 新鲜摆放的运动安全准入。只在 `spawn_vehicle` 与 `replace_completed_vehicle`
//! 提交前使用；快照恢复和修订切换不调用。

use laneflow_static_contract::VehicleProfileOrdinal;
use laneflow_static_network::BoundedDistance;

use super::occupancy::LeaderQueryHorizon;
use super::tables::occupancy_front_gap;
use crate::{SpawnError, VehicleHandle, VehicleSpawnInput, VehicleStatus};

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

/// 后车能否在不依赖硬投影的前提下接纳这名前车。
///
/// 前车速度为 0 时额外房间为 0，与驶离停车的现行判断相同。
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
        if let Some((room_mm, target_mm_s)) =
            self.downstream_speed_room_mm(input.route(), cursor, input.progress_mm())
            && !can_slow_to_before(input.initial_speed_mm_s(), target_mm_s, emergency, room_mm)
        {
            return Err(FreshAdmissionFailure::DownstreamSpeed);
        }
        self.ensure_current_occupancy()
            .map_err(|_| FreshAdmissionFailure::OccupancyAlloc)?;
        let delta_s = self.binding.config.fixed_delta_time_ms() as f32 / 1_000.0;
        self.admit_nearest_leader(input, profile.min_gap_mm(), emergency, delta_s)?;
        self.admit_direct_followers(input, vehicle_length_mm, emergency, delta_s)
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
        let rolled = progress_mm == 0 && route_edge_index > 0;
        let mut hop = usize::try_from(if rolled {
            route_edge_index - 1
        } else {
            route_edge_index
        })
        .ok()?;
        let progress_base = if rolled {
            let edge = *compiled.edges.get(hop)?;
            *self
                .binding
                .revision
                .traffic()
                .lane_lengths_millimetres()
                .get(edge.index())?
        } else {
            progress_mm
        };
        let mut from_cursor_start = BoundedDistance::Finite(0);
        let mut accumulated = false;
        while hop < compiled.next_controlled.len() {
            let next = compiled.next_controlled[hop]?;
            from_cursor_start = if accumulated {
                from_cursor_start.add_bounded(next.distance_from_hop_start)
            } else {
                next.distance_from_hop_start
            };
            accumulated = true;
            if read.gate_is_restrictive(next.gate, profile) {
                let BoundedDistance::Finite(mm) = from_cursor_start.saturating_sub(progress_base)
                else {
                    return None;
                };
                return Some(mm);
            }
            let next_hop = usize::try_from(next.hop).ok()?.checked_add(1)?;
            if next_hop <= hop {
                return None;
            }
            hop = next_hop;
        }
        None
    }

    fn downstream_speed_room_mm(
        &self,
        route: crate::RouteHandle,
        cursor: usize,
        progress_mm: u32,
    ) -> Option<(u32, u32)> {
        let compiled = self.compiled_route(route)?;
        let mut nearest: Option<(usize, u32)> = None;
        for drop in &compiled.speed_limit_drop {
            let from = usize::try_from(drop.from_route_edge_index).ok()?;
            if from < cursor {
                continue;
            }
            if nearest.is_none_or(|(index, _)| from < index) {
                nearest = Some((from, drop.target_mm_s));
            }
        }
        let (from, target_mm_s) = nearest?;
        let lengths = self.binding.revision.traffic().lane_lengths_millimetres();
        let current = *compiled.edges.get(cursor)?;
        let mut room = lengths.get(current.index())?.saturating_sub(progress_mm);
        for index in (cursor + 1)..=from {
            let edge = *compiled.edges.get(index)?;
            room = room.saturating_add(*lengths.get(edge.index())?);
        }
        Some((room, target_mm_s))
    }

    fn admit_nearest_leader(
        &self,
        input: VehicleSpawnInput,
        follower_min_gap_mm: u32,
        follower_emergency: f32,
        delta_s: f32,
    ) -> Result<(), FreshAdmissionFailure> {
        let lengths = self.binding.revision.traffic().lane_lengths_millimetres();
        let follower_edges = self.route_edges(input.route()).expect("已校验的路线仍在");
        let follower_index =
            usize::try_from(input.route_edge_index()).expect("route index fits usize");
        let mut nearest: Option<(VehicleHandle, u32)> = None;
        for handle in self.derived.active_order.iter().copied() {
            let Some(leader) = self.vehicle_state(handle).copied() else {
                continue;
            };
            if leader.status != VehicleStatus::Active {
                continue;
            }
            let Some(leader_edges) = self.route_edges(leader.route) else {
                return Err(FreshAdmissionFailure::UnsafeLeader(handle));
            };
            let Ok(leader_index) = usize::try_from(leader.route_edge_index) else {
                return Err(FreshAdmissionFailure::UnsafeLeader(handle));
            };
            let Some(gap) = occupancy_front_gap(
                lengths,
                follower_edges,
                follower_index,
                input.progress_mm(),
                leader_edges,
                leader_index,
                leader.progress_mm,
                leader.length_mm,
            ) else {
                continue;
            };
            if gap < 0 {
                continue;
            }
            let gap_mm = u32::try_from(gap).unwrap_or(u32::MAX);
            if nearest.is_none_or(|(_, current)| gap_mm < current) {
                nearest = Some((handle, gap_mm));
            }
        }
        let Some((leader_handle, gap_mm)) = nearest else {
            return Ok(());
        };
        let leader = self
            .vehicle_state(leader_handle)
            .copied()
            .ok_or(FreshAdmissionFailure::UnsafeLeader(leader_handle))?;
        let leader_profile = self
            .binding
            .revision
            .traffic()
            .relations()
            .vehicle_profile(leader.profile)
            .ok_or(FreshAdmissionFailure::UnsafeLeader(leader_handle))?;
        if !moving_follower_can_admit(
            input.initial_speed_mm_s(),
            follower_emergency,
            follower_min_gap_mm,
            gap_mm,
            leader.speed_mm_s,
            leader_profile.emergency_decel(),
            delta_s,
        ) {
            return Err(FreshAdmissionFailure::UnsafeLeader(leader_handle));
        }
        Ok(())
    }

    fn admit_direct_followers(
        &self,
        input: VehicleSpawnInput,
        vehicle_length_mm: u32,
        leader_emergency: f32,
        delta_s: f32,
    ) -> Result<(), FreshAdmissionFailure> {
        let lengths = self.binding.revision.traffic().lane_lengths_millimetres();
        let candidate_edges = self.route_edges(input.route()).expect("已校验的路线仍在");
        let candidate_index =
            usize::try_from(input.route_edge_index()).expect("route index fits usize");
        let candidate_identity = VehicleHandle::new(u32::MAX, u32::MAX);
        for handle in self.derived.active_order.iter().copied() {
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
            let candidate_gap_limit = u32::try_from(candidate_gap.max(0)).unwrap_or(u32::MAX);
            if self
                .derived
                .occupancy
                .leader_gap(
                    candidate_identity,
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
            let gap_mm = u32::try_from(candidate_gap.max(0)).unwrap_or(u32::MAX);
            if !moving_follower_can_admit(
                follower.speed_mm_s,
                profile.emergency_decel(),
                profile.min_gap_mm(),
                gap_mm,
                input.initial_speed_mm_s(),
                leader_emergency,
                delta_s,
            ) {
                return Err(FreshAdmissionFailure::UnsafeFollower(handle));
            }
        }
        Ok(())
    }
}
