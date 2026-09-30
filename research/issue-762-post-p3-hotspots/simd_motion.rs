// #805：暂存仅在 P5 工作块栈上，完整句柄和规范结果槽仍由 Active 投影定位。
#[derive(Clone, Copy)]
struct PreparedActiveMotion<'a> {
    state: VehicleState,
    compiled: &'a CompiledRoute,
    lengths: &'a [u32],
    speed_limits: &'a [u32],
    cursor: usize,
    edge: LaneEdgeOrdinal,
    profile: VehicleProfileView,
    leader_gap: Option<i64>,
    route_end: BoundedDistance,
    reach: Option<MotionReach>,
    parking: Option<(ParkingReservation, BoundedDistance)>,
    movement_stop: Option<BoundedDistance>,
    waiting_stop: Option<crate::kernel::waiting::WaitingStopConstraint>,
    conflict_stop: Option<crate::kernel::waiting::WaitingStopConstraint>,
    delta_s: f32,
    desired_mm_s: u32,
    iidm: IidmInput,
}

#[derive(Clone, Copy)]
enum PreparedMotionState<'a> {
    Reused(VehicleState),
    Compute(PreparedActiveMotion<'a>),
}

#[derive(Clone, Copy)]
struct PreparedVehicleMotion<'a> {
    motion: PreparedMotionState<'a>,
    reservation: Option<ParkingReservation>,
    arrived_before: bool,
    handle: crate::VehicleHandle,
}

impl<'a> MotionTaskView<'a> {
    fn finish_vehicle_motion(
        self,
        prepared: PreparedVehicleMotion<'a>,
        iidm: Option<(f32, f32)>,
    ) -> Result<VehicleMotionOutcome, StepError> {
        #[cfg(test)]
        let cache_served = matches!(prepared.motion, PreparedMotionState::Reused(_));
        let next = match prepared.motion {
            PreparedMotionState::Reused(next) => next,
            PreparedMotionState::Compute(motion) => self.read
                .finish_active_motion(motion, None, false, iidm)
                .ok_or(StepError::NonFiniteMotion)?,
        };
        #[cfg(test)]
        note_motion_cache_use(cache_served);
        let arrival = if let Some(reservation) = prepared.reservation {
            if next.status != VehicleStatus::Active {
                return Err(StepError::ParkingInvariantViolation);
            }
            (!prepared.arrived_before && self.read.parking_arrived_for(next, reservation))
                .then(|| ParkingArrivalObservation {
                    vehicle: prepared.handle, target: reservation.target(),
                })
        } else { None };
        Ok(VehicleMotionOutcome { next, arrival })
    }

    fn compute_motion_batch(
        self,
        start: usize,
        chunk: &mut [crate::kernel::execution::DispatchSlot<Option<VehicleMotionOutcome>>],
        delta_s: f32,
        first_error: &std::sync::atomic::AtomicUsize,
    ) {
        use crate::kernel::execution::DispatchSlot;
        use std::sync::atomic::Ordering;
        for (batch_index, slots) in chunk.chunks_mut(4).enumerate() {
            let base = start + batch_index * 4;
            let prepared: [Option<Result<PreparedVehicleMotion<'a>, StepError>>; 4] =
                std::array::from_fn(|i| {
                    if i >= slots.len() { return None; }
                    let handle = self.read.derived.active_order[base + i];
                    self.read.vehicle_state(handle).map(|state| {
                        self.prepare_vehicle_motion(state, base + i, delta_s)
                    })
                });
            let inputs = std::array::from_fn(|i| match &prepared[i] {
                Some(Ok(PreparedVehicleMotion { motion: PreparedMotionState::Compute(p), .. }))
                    => p.iidm,
                _ => IidmInput::EMPTY,
            });
            let results = IidmBatch::gather(inputs).BATCH_SOLVE();
            for (i, slot) in slots.iter_mut().enumerate() {
                let result = match prepared[i] {
                    Some(Ok(prepared)) => self.finish_vehicle_motion(prepared, results[i]).map(Some),
                    Some(Err(error)) => Err(error),
                    None => Ok(None),
                };
                let failed = result.is_err();
                *slot = DispatchSlot::Done(result);
                if failed {
                    first_error.fetch_min(base + i, Ordering::Relaxed);
                    return;
                }
            }
        }
    }
}
