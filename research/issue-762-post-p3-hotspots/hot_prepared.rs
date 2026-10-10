// #808：复用结果与计算冷字段分开；四车暂存只在 P5 工作块栈上存活。
#[derive(Clone, Copy)]
struct PreparedVehicleMotion {
    reused: Option<VehicleState>,
    reservation: Option<ParkingReservation>,
    arrived_before: bool,
    handle: crate::VehicleHandle,
}

impl<'a> MotionTaskView<'a> {
    fn finish_vehicle_motion(
        self,
        prepared: PreparedVehicleMotion,
        cold: Option<&PreparedActiveMotion<'a>>,
        iidm: Option<(f32, f32)>,
    ) -> Result<VehicleMotionOutcome, StepError> {
        #[cfg(test)]
        let cache_served = prepared.reused.is_some();
        let next = match prepared.reused {
            Some(next) => next,
            None => self
                .read
                .finish_active_motion(*cold.ok_or(StepError::NonFiniteMotion)?, None, false, iidm)
                .ok_or(StepError::NonFiniteMotion)?,
        };
        #[cfg(test)]
        note_motion_cache_use(cache_served);
        let arrival = if let Some(reservation) = prepared.reservation {
            if next.status != VehicleStatus::Active {
                return Err(StepError::ParkingInvariantViolation);
            }
            (!prepared.arrived_before && self.read.parking_arrived_for(next, reservation)).then(
                || ParkingArrivalObservation {
                    vehicle: prepared.handle,
                    target: reservation.target(),
                },
            )
        } else {
            None
        };
        Ok(VehicleMotionOutcome { next, arrival })
    }
}
