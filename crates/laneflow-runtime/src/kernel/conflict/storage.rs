use super::{
    ConflictAcquireError, ConflictPassageRange, ConflictReservation, DownstreamRoutePoint,
    downstream_claim_target,
};
use crate::kernel::state::WorldState;
use crate::kernel::tables::CompiledRoute;
use crate::{ParkingBinding, VehicleState, VehicleStatus};

type Proof<T> = Result<T, ConflictAcquireError>;
const INVALID: ConflictAcquireError = ConflictAcquireError::InvalidBundle;

/// 以本次机动入口为原点的下游授权终点。机动内各道准入门共用这一原点。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RelativeDownstreamEnd {
    pub(crate) relative_hop_offset: u32,
    pub(crate) progress_mm: u32,
}

impl RelativeDownstreamEnd {
    pub(crate) fn from_range(
        compiled: &CompiledRoute,
        range: ConflictPassageRange,
        point: DownstreamRoutePoint,
    ) -> Option<Self> {
        let maneuver = compiled
            .maneuvers
            .get(range.maneuver_occurrence_index() as usize)?;
        Self::from_point(maneuver.entry_route_edge_index, point)
    }

    pub(crate) fn from_point(entry: u32, point: DownstreamRoutePoint) -> Option<Self> {
        if point.carry_um() != 0 {
            return None;
        }
        Some(Self {
            relative_hop_offset: point.route_edge_index().checked_sub(entry)?,
            progress_mm: point.progress_mm(),
        })
    }

    pub(crate) fn point(self, entry: u32) -> Option<DownstreamRoutePoint> {
        DownstreamRoutePoint::new(
            entry.checked_add(self.relative_hop_offset)?,
            self.progress_mm,
            0,
        )
    }
}

fn position(state: VehicleState) -> Proof<DownstreamRoutePoint> {
    DownstreamRoutePoint::new(state.route_edge_index, state.progress_mm, state.carry_um)
        .ok_or(INVALID)
}

/// 两个车头出现是同一物理位置；只要求到义务点的前向区段一致。
pub(crate) fn remaining_prefix_matches(
    source: &CompiledRoute,
    source_front: u32,
    target: &CompiledRoute,
    target_front: u32,
    required: DownstreamRoutePoint,
) -> bool {
    let Some(end) = required.rebase(source_front, target_front) else {
        return false;
    };
    let prefix = |front: u32, end: DownstreamRoutePoint| {
        front as usize
            ..end.route_edge_index() as usize
                + usize::from(end.progress_mm() != 0 || end.carry_um() != 0)
    };
    source
        .edges
        .get(prefix(source_front, required))
        .is_some_and(|edges| target.edges.get(prefix(target_front, end)) == Some(edges))
}

impl WorldState {
    /// 以新旧车辆游标为对应点重绑定原机动、Gate、passages 和授权终点。物理区间及其 owner 不动。
    pub(crate) fn rebound_conflict_reservation(
        &self,
        old: VehicleState,
        new_route: crate::RouteHandle,
        new_cursor: u32,
    ) -> Proof<Option<ConflictReservation>> {
        let Some(reservation) = self.conflict_reservation(old.handle) else {
            return Ok(None);
        };
        let range = reservation.passage_range;
        let source = self.compiled_route(old.route).ok_or(INVALID)?;
        let target = self.compiled_route(new_route).ok_or(INVALID)?;
        let shift = |index: u32| {
            u32::try_from(
                i64::from(index) + i64::from(new_cursor) - i64::from(old.route_edge_index),
            )
            .map_err(|_| INVALID)
        };
        let maneuver = source
            .maneuvers
            .get(range.maneuver_occurrence_index as usize)
            .ok_or(INVALID)?;
        let entry = shift(maneuver.entry_route_edge_index)?;
        let index = target
            .maneuvers
            .partition_point(|m| m.entry_route_edge_index < entry);
        let mapped = target.maneuvers.get(index).ok_or(INVALID)?;
        if mapped.entry_route_edge_index != entry
            || mapped.exit_route_edge_index != shift(maneuver.exit_route_edge_index)?
            || mapped.path != maneuver.path
        {
            return Err(INVALID);
        }
        let gate = shift(range.admission_gate_hop)?;
        let old_gate = source
            .hop_gate
            .get(range.admission_gate_hop as usize)
            .copied()
            .flatten()
            .ok_or(INVALID)?;
        if target.hop_gate.get(gate as usize).copied().flatten() != Some(old_gate) {
            return Err(INVALID);
        }
        let bounds = target
            .conflict_gate_ranges
            .get(gate as usize)
            .ok_or(INVALID)?;
        if range.passage_count() != bounds.len {
            return Err(INVALID);
        }
        let target_range = ConflictPassageRange::new(
            new_route,
            u32::try_from(index).map_err(|_| INVALID)?,
            gate,
            bounds.start,
            bounds.len,
        )
        .ok_or(INVALID)?;
        for offset in 0..bounds.len {
            let old = source
                .conflicts
                .get((range.first_conflict_occurrence_index + offset) as usize)
                .ok_or(INVALID)?;
            let new = target
                .conflicts
                .get((bounds.start + offset) as usize)
                .ok_or(INVALID)?;
            if old.static_zone().is_none()
                || old.address() != new.address()
                || new.maneuver_index != target_range.maneuver_occurrence_index()
                || new.admission_hop != gate
                || shift(old.entry.route_edge_index)? != new.entry.route_edge_index
                || old.entry.progress_mm != new.entry.progress_mm
                || shift(old.clearance.route_edge_index)? != new.clearance.route_edge_index
                || old.clearance.progress_mm != new.clearance.progress_mm
            {
                return Err(INVALID);
            }
        }
        let endpoint = reservation
            .downstream_target
            .rebase(range.admission_gate_hop, gate)
            .ok_or(INVALID)?;
        if source
            .edges
            .get(
                maneuver.entry_route_edge_index as usize
                    ..=reservation.downstream_target.route_edge_index() as usize,
            )
            .is_none_or(|edges| {
                target
                    .edges
                    .get(entry as usize..=endpoint.route_edge_index() as usize)
                    != Some(edges)
            })
        {
            return Err(INVALID);
        }
        self.reservation_downstream_claim_plan_to(target_range, old.length_mm, endpoint)?;
        Ok(Some(ConflictReservation {
            passage_range: target_range,
            downstream_target: endpoint,
            ..reservation
        }))
    }

    /// 前置检查已证明车辆、资源和区间不变，提交时只更换原预约的路线出现坐标。
    pub(crate) fn install_rebound_conflict(&mut self, reservation: ConflictReservation) {
        let index = self
            .conflict_read()
            .committed_owner_index(reservation.owner.index() as usize)
            .expect("validated reservation owner");
        self.committed.conflict.committed_owners[index] = reservation;
    }

    /// 验证已提交的每车授权及其尚未兑现的腾让义务。空账本不访问车辆表。
    pub(crate) fn conflict_platoons_valid(&self) -> Proof<bool> {
        self.conflict_storage_requires(|_, _| Ok(true))
    }

    /// 生命周期命令在发布前按旧状态导出义务，用新路线和拟增停车位置核验。
    pub(crate) fn parking_preserves_conflict_storage(
        &self,
        old: VehicleState,
        new: VehicleState,
        stop_hop: u32,
        stop_progress: u32,
    ) -> Proof<bool> {
        let stop = DownstreamRoutePoint::new(stop_hop, stop_progress, 0).ok_or(INVALID)?;
        self.conflict_storage_requires(|vehicle, required| {
            if vehicle.handle != old.handle {
                return Ok(true);
            }
            let Some(local) = required.rebase(old.route_edge_index, new.route_edge_index) else {
                return Ok(false);
            };
            Ok(stop >= local
                && remaining_prefix_matches(
                    self.compiled_route(old.route).ok_or(INVALID)?,
                    old.route_edge_index,
                    self.compiled_route(new.route).ok_or(INVALID)?,
                    new.route_edge_index,
                    required,
                ))
        })
    }

    fn conflict_storage_requires(
        &self,
        mut permits: impl FnMut(VehicleState, DownstreamRoutePoint) -> Proof<bool>,
    ) -> Proof<bool> {
        let ledger = self.conflict_read();
        let mut ahead = Vec::new();
        let lengths = self.binding.revision.traffic().lane_lengths_millimetres();
        for &reservation in ledger.committed_reservations() {
            let state = self.vehicle_state(reservation.owner()).ok_or(INVALID)?;
            let range = reservation.passage_range();
            let single = self
                .reservation_downstream_claim_plan(range, state.length_mm)?
                .target();
            let endpoint = reservation.downstream_target();
            if self
                .reservation_downstream_claim_plan_to(range, state.length_mm, endpoint)
                .is_err()
            {
                return Ok(false);
            }
            let front = position(state)?;
            if front <= single
                && (!self.conflict_member_can_reach(state, single)? || !permits(state, single)?)
            {
                return Ok(false);
            }
            if single == endpoint && !ledger.reservation_shares_cell(state.handle) {
                continue;
            }
            let route = self.compiled_route(state.route).ok_or(INVALID)?;
            let first = route
                .conflicts
                .get(range.first_conflict_occurrence_index() as usize)
                .ok_or(INVALID)?
                .address();
            ahead.clear();
            for &vehicle in &self.committed.live_order {
                if vehicle == state.handle {
                    continue;
                }
                let other = self.vehicle_state(vehicle).ok_or(INVALID)?;
                if other.status != VehicleStatus::Active {
                    continue;
                }
                let other_route = self.compiled_route(other.route).ok_or(INVALID)?;
                let other_position = if ledger.reservation_has_cell(vehicle, first) {
                    // 同一物理门在两条循环路线上的绝对 occurrence 可以不同。
                    let other_held = ledger.reservation(vehicle).ok_or(INVALID)?;
                    let projected = position(other)?
                        .rebase(
                            other_held.admission_gate_hop(),
                            reservation.admission_gate_hop(),
                        )
                        .ok_or(INVALID)?;
                    if route.edges.get(projected.route_edge_index() as usize)
                        != other_route.edges.get(other.route_edge_index as usize)
                        || projected == front
                    {
                        return Ok(false);
                    }
                    if projected < front {
                        continue;
                    }
                    if other_held.acquired_tick() > reservation.acquired_tick() {
                        return Ok(false);
                    }
                    projected
                } else {
                    let Some(projected) = project_front(route, state, front, other_route, other)?
                    else {
                        continue;
                    };
                    projected
                };
                if other_position > endpoint {
                    // 单车不能依赖前车腾让，即使前车车头已经越过本车终点。
                    if single == endpoint {
                        let gap = self.conflict_minimum_gap(state)?;
                        let room = crate::kernel::tables::remaining_along_route_i64(
                            lengths,
                            &route.edges,
                            endpoint.route_edge_index() as usize,
                            endpoint.progress_mm(),
                            other_position.route_edge_index() as usize,
                            other_position.progress_mm(),
                        )
                        .ok_or(INVALID)?;
                        if room < i64::from(gap) + i64::from(other.length_mm) {
                            return Ok(false);
                        }
                    }
                    continue;
                }
                if single == endpoint {
                    return Ok(false);
                }
                ahead
                    .try_reserve(1)
                    .map_err(|_| ConflictAcquireError::ScratchAllocFailed)?;
                ahead.push((other_position, other));
            }
            ahead.sort_unstable_by_key(|(point, vehicle)| (*point, vehicle.handle.index()));
            let mut required = single;
            let mut previous = state;
            for &(actual, other) in &ahead {
                let extension = self
                    .conflict_minimum_gap(previous)?
                    .checked_add(other.length_mm)
                    .ok_or(INVALID)?;
                required = downstream_claim_target(&route.edges, lengths, required, extension)?;
                previous = other;
                if actual > required {
                    continue;
                }
                if required > endpoint {
                    return Ok(false);
                }
                let other_route = self.compiled_route(other.route).ok_or(INVALID)?;
                let local = required
                    .rebase(actual.route_edge_index(), other.route_edge_index)
                    .ok_or(INVALID)?;
                if !remaining_prefix_matches(
                    route,
                    actual.route_edge_index(),
                    other_route,
                    other.route_edge_index,
                    required,
                ) || !self.conflict_member_can_reach(other, local)?
                    || !permits(other, local)?
                {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    fn conflict_minimum_gap(&self, state: VehicleState) -> Proof<u32> {
        self.binding
            .revision
            .traffic()
            .relations()
            .vehicle_profile(state.profile)
            .map(|profile| profile.min_gap_mm())
            .ok_or(INVALID)
    }

    fn conflict_member_can_reach(
        &self,
        state: VehicleState,
        required: DownstreamRoutePoint,
    ) -> Proof<bool> {
        let route = self.compiled_route(state.route).ok_or(INVALID)?;
        if route
            .edges
            .get(required.route_edge_index() as usize)
            .is_none()
        {
            return Ok(false);
        }
        let first = route
            .gate_hops
            .partition_point(|hop| *hop < state.route_edge_index);
        let owned = self
            .conflict_reservation(state.handle)
            .map(|r| r.admission_gate_hop());
        if let Some(hop) = route.gate_hops[first..]
            .iter()
            .copied()
            .find(|hop| Some(*hop) != owned)
        {
            let boundary = DownstreamRoutePoint::new(hop.checked_add(1).ok_or(INVALID)?, 0, 0)
                .ok_or(INVALID)?;
            if required > boundary {
                return Ok(false);
            }
        }
        if let Some(ParkingBinding::Reserved(stop)) = self.committed.parking.binding(state.handle) {
            let (_, progress) = self.reservation_anchor(stop).ok_or(INVALID)?;
            if required
                > DownstreamRoutePoint::new(stop.entry_route_occurrence(), progress, 0)
                    .ok_or(INVALID)?
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn project_front(
    route: &CompiledRoute,
    follower: VehicleState,
    front: DownstreamRoutePoint,
    other_route: &CompiledRoute,
    other: VehicleState,
) -> Proof<Option<DownstreamRoutePoint>> {
    let current = position(other)?;
    if follower.route == other.route {
        return Ok((current > front).then_some(current));
    }
    let edge = *other_route
        .edges
        .get(other.route_edge_index as usize)
        .ok_or(INVALID)?;
    let mut cursor = follower.route_edge_index;
    while let Some(index) = route.first_edge_occurrence_at_or_after(edge, cursor) {
        let p =
            DownstreamRoutePoint::new(index, other.progress_mm, other.carry_um).ok_or(INVALID)?;
        if p > front {
            return Ok(Some(p));
        }
        cursor = index.checked_add(1).ok_or(INVALID)?;
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn later_admission_gate_uses_maneuver_entry_as_persisted_origin() {
        let (world, owner) =
            crate::admin::format_admission::tests::world_with_conflict_reservation();
        let reservation = world.conflict_reservation(owner).unwrap();
        let held = reservation.passage_range();
        let route = world.state.compiled_route(held.route()).unwrap();
        let entry =
            route.maneuvers[held.maneuver_occurrence_index() as usize].entry_route_edge_index;
        let later_gate = entry + 1;
        let range = crate::ConflictPassageRange::new(
            held.route(),
            held.maneuver_occurrence_index(),
            later_gate,
            held.first_conflict_occurrence_index(),
            held.passage_count(),
        )
        .unwrap();
        let point = DownstreamRoutePoint::new(later_gate + 1, 17, 0).unwrap();
        let encoded = RelativeDownstreamEnd::from_range(route, range, point).unwrap();
        assert_eq!(encoded.relative_hop_offset, 2);
        assert_eq!(encoded.point(entry), Some(point));
    }

    #[test]
    fn downstream_end_requires_forward_occurrence_and_zero_carry() {
        let point = |index, mm, carry| DownstreamRoutePoint::new(index, mm, carry).unwrap();
        let end = RelativeDownstreamEnd::from_point(7, point(10, 25, 0)).unwrap();
        assert_eq!(end.relative_hop_offset, 3);
        assert_eq!(end.point(7), Some(point(10, 25, 0)));
        assert_eq!(end.point(u32::MAX - 3), Some(point(u32::MAX, 25, 0)));
        assert_eq!(end.point(u32::MAX - 2), None);
        assert_eq!(RelativeDownstreamEnd::from_point(7, point(6, 25, 0)), None);
        assert_eq!(RelativeDownstreamEnd::from_point(7, point(7, 25, 1)), None);
    }
}
