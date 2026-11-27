use crate::DownstreamRoutePoint;
pub(crate) use crate::StableRouteHandoffWindow as CapturedHandoffWindow;
use laneflow_static_contract::StableId128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CapturedDownstreamEnd {
    pub(crate) relative_hop_offset: u32,
    pub(crate) progress_mm: u32,
}

impl CapturedDownstreamEnd {
    /// 本次机动内的后续门也以机动入口为相对原点，快照与迁移日志共用此转换。
    pub(crate) fn from_range(
        compiled: &crate::kernel::tables::CompiledRoute,
        range: crate::ConflictPassageRange,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CapturedHandoffProtection {
    pub(crate) role: u8,
    pub(crate) edge: StableId128,
    pub(crate) start_mm: u32,
    pub(crate) end_mm: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CapturedHandoff {
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
    pub(crate) protections: Vec<CapturedHandoffProtection>,
}

impl crate::kernel::state::WorldState {
    pub(super) fn capture_handoffs_for(
        &self,
        vehicle: crate::VehicleHandle,
        budget: &mut crate::admin::snapshot_digest::NormalizationBudget,
    ) -> Result<Vec<CapturedHandoff>, super::SnapshotCaptureError> {
        let all = &self.committed.handoffs;
        let start = all.partition_point(|h| h.vehicle.index() < vehicle.index());
        let end = all.partition_point(|h| h.vehicle.index() <= vehicle.index());
        let mut records = capture_rows(end - start, budget)?;
        for h in &all[start..end] {
            if h.vehicle != vehicle {
                return Err(super::SnapshotCaptureError::HandoffInvariantViolation);
            }
            let mut protections = capture_rows(h.protections.len(), budget)?;
            for p in &h.protections {
                let edge = self
                    .binding
                    .revision
                    .identity()
                    .stable_id(p.edge)
                    .ok_or(super::SnapshotCaptureError::HandoffInvariantViolation)?;
                protections.push(CapturedHandoffProtection {
                    role: p.role,
                    edge: *edge.as_untyped(),
                    start_mm: p.start_mm,
                    end_mm: p.end_mm,
                });
            }
            records.push(CapturedHandoff {
                source_index: h.source_index,
                trajectory_version: h.trajectory_version,
                station_window_mm: h.station_window_mm,
                rear_stations_mm: h.rear_stations_mm,
                admitted_speed_bits: h.admitted_speed_bits,
                tracks: h.tracks,
                sweep: h.sweep,
                rear_travel_um: h.rear_travel_um,
                front_handed_off: h.front_handed_off,
                acquired_tick: h.acquired_tick,
                protections,
            });
        }
        Ok(records)
    }
}

pub(super) fn capture_rows<T>(
    count: usize,
    budget: &mut crate::admin::snapshot_digest::NormalizationBudget,
) -> Result<Vec<T>, super::SnapshotCaptureError> {
    budget
        .charge_bytes_for_reserve(size_of::<T>(), count)
        .map_err(|_| super::SnapshotCaptureError::LimitExceeded)?;
    let mut rows = Vec::new();
    super::capture_try_reserve_exact(&mut rows, count)?;
    Ok(rows)
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
        let encoded = CapturedDownstreamEnd::from_range(route, range, point).unwrap();
        assert_eq!(encoded.relative_hop_offset, 2);
        assert_eq!(encoded.point(entry), Some(point));
    }

    #[test]
    fn downstream_end_requires_forward_occurrence_and_zero_carry() {
        let point = |index, mm, carry| DownstreamRoutePoint::new(index, mm, carry).unwrap();
        let end = CapturedDownstreamEnd::from_point(7, point(10, 25, 0)).unwrap();
        assert_eq!(end.relative_hop_offset, 3);
        assert_eq!(end.point(7), Some(point(10, 25, 0)));
        assert_eq!(end.point(u32::MAX - 3), Some(point(u32::MAX, 25, 0)));
        assert_eq!(end.point(u32::MAX - 2), None);
        assert_eq!(CapturedDownstreamEnd::from_point(7, point(6, 25, 0)), None);
        assert_eq!(CapturedDownstreamEnd::from_point(7, point(7, 25, 1)), None);
    }
}
