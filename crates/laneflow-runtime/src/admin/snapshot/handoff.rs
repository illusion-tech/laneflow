pub(crate) use crate::StableRouteHandoffWindow as CapturedHandoffWindow;
use laneflow_static_contract::StableId128;

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
