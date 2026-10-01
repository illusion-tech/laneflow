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
        let mut batch = None;
        let prepared: [Option<Result<PreparedVehicleMotion<'a>, StepError>>; 4] =
            std::array::from_fn(|i| {
                if i >= slots.len() {
                    return None;
                }
                let handle = self.read.derived.active_order[base + i];
                self.read.vehicle_state(handle).map(|state| {
                    self.prepare_vehicle_motion(state, base + i, delta_s, &mut batch, i)
                })
            });
        let results = batch.as_ref().map(|batch| batch.BATCH_SOLVE());
        for (i, slot) in slots.iter_mut().enumerate() {
            let result = match prepared[i] {
                Some(Ok(prepared)) => self
                    .finish_vehicle_motion(prepared, results.as_ref().and_then(|values| values[i]))
                    .map(Some),
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
