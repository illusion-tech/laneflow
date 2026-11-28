use crate::kernel::{
    phase::StepWorkspace, tables::CompiledRoute, tick::barrier_speed_cap,
    waiting::WaitingStopConstraint,
};
use crate::{StepError, VehicleState, VehicleStatus};

impl StepWorkspace<'_> {
    /// 车尾已证清空后，已预留下游空间内的余程按普通有向后继、灯色及本拍资源裁决推进。
    pub(super) fn continue_after_handoff(
        &self,
        mut endpoint: VehicleState,
        route: &CompiledRoute,
        remaining_um: u64,
        travel_um: u64,
        next_speed_mm_s: u32,
        waiting_stop: Option<WaitingStopConstraint>,
    ) -> Result<VehicleState, StepError> {
        let world = self.read_view();
        let delta_s = self.binding.config.fixed_delta_time_ms() as f32 / 1000.;
        let resource_stop = self.motion_conflict_stop_for(&endpoint, delta_s)?;
        let prior_um = travel_um
            .checked_sub(remaining_um)
            .ok_or(StepError::HandoffUnproven)?;
        let prior_mm =
            u32::try_from(prior_um.div_ceil(1000)).map_err(|_| StepError::HandoffUnproven)?;
        let mut travel_mm =
            u32::try_from(remaining_um / 1000).map_err(|_| StepError::HandoffUnproven)?;
        let mut parking_entry = None;
        if let Some((reservation, laneflow_static_network::BoundedDistance::Finite(distance))) =
            world
                .parking_stop_distance(
                    route,
                    &endpoint,
                    endpoint.route_edge_index as usize,
                    self.committed.parking.binding(endpoint.handle),
                )
                .ok_or(StepError::ParkingInvariantViolation)?
            && distance <= travel_mm
        {
            let (_, mm) = world
                .reservation_anchor(reservation)
                .ok_or(StepError::ParkingInvariantViolation)?;
            parking_entry = Some((reservation.entry_route_occurrence(), mm));
            travel_mm = distance;
        }
        let initial_speed = endpoint.speed_mm_s;
        endpoint.carry_um = (remaining_um % 1000) as u16;
        world
            .apply_route_travel_mm(
                &mut endpoint,
                route,
                travel_mm,
                prior_mm,
                waiting_stop,
                resource_stop,
            )
            .ok_or(StepError::HandoffUnproven)?;
        if let Some(cursor) = parking_entry
            && (endpoint.route_edge_index, endpoint.progress_mm) >= cursor
        {
            (endpoint.route_edge_index, endpoint.progress_mm) = cursor;
            endpoint.carry_um = 0;
            endpoint.speed_mm_s = 0;
            return Ok(endpoint);
        }
        let traffic = self.binding.revision.traffic();
        let index = endpoint.route_edge_index as usize;
        let edge = route.edges[index].index();
        endpoint.speed_mm_s =
            next_speed_mm_s.min(traffic.lane_speed_limits_millimetres_per_second()[edge]);
        if endpoint.progress_mm == traffic.lane_lengths_millimetres()[edge] {
            endpoint.carry_um = 0;
            if index + 1 == route.edges.len() {
                endpoint.speed_mm_s = 0;
                endpoint.status = VehicleStatus::Completed;
            } else {
                let profile = traffic
                    .relations()
                    .vehicle_profile(endpoint.profile)
                    .ok_or(StepError::HandoffUnproven)?;
                endpoint.speed_mm_s = endpoint.speed_mm_s.min(barrier_speed_cap(
                    initial_speed,
                    profile.emergency_decel(),
                    delta_s,
                ));
            }
        }
        Ok(endpoint)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_remainder_at_a_cut_uses_the_successor_only_if_permitted() {
        let (mut world, follower, _) =
            crate::kernel::occupancy::tests::zero_progress_merge_fixture();
        let mut endpoint = world.vehicle(follower).unwrap();
        let phase = world.state.step_workspace();
        let route = phase.compiled_route(endpoint.route).unwrap();
        let length =
            phase.binding.revision.traffic().lane_lengths_millimetres()[route.edges[0].index()];
        endpoint.progress_mm = length - 100;
        let finish = |stop| {
            phase
                .continue_after_handoff(endpoint, route, 100_000, 101_000, 1000, stop)
                .unwrap()
        };
        let passed = finish(None);
        assert_eq!(
            (passed.route_edge_index, passed.progress_mm, passed.carry_um),
            (1, 0, 0)
        );
        let held = finish(Some(WaitingStopConstraint {
            distance: laneflow_static_network::BoundedDistance::Finite(100),
            hop: 0,
        }));
        assert_eq!(
            (held.route_edge_index, held.progress_mm, held.carry_um),
            (0, length, 0)
        );
    }
}
