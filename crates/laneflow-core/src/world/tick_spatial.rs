use super::*;

impl CoreWorld {

    pub(super) fn rebuild_command_spatial_index(&mut self) {
        let mut spatial = std::mem::take(&mut self.command_spatial_index);
        spatial.begin_rebuild(self.vehicles.len());
        for vehicle in self.vehicles() {
            if !matches!(
                vehicle.status,
                VehicleStatus::Active | VehicleStatus::Stopped
            ) {
                continue;
            }
            spatial.stage(
                self.vehicle_edge(vehicle),
                CommandOccupant {
                    vehicle: vehicle.handle,
                    front_progress: vehicle.edge_progress.value(),
                },
            );
        }
        spatial.finish_rebuild();
        self.command_spatial_index = spatial;
    }

    pub(super) fn sync_changed_command_spatial_memberships(&mut self, candidate: &CandidateStateScratch) {
        let routes = &self.routes;
        let vehicles = &self.vehicles;
        let spatial = &mut self.command_spatial_index;
        let membership = |state: &VehicleState| {
            matches!(state.status, VehicleStatus::Active | VehicleStatus::Stopped).then(|| {
                (
                    routes[state.route.index()].edge_handles[state.route_edge_index],
                    CommandOccupant {
                        vehicle: state.handle,
                        front_progress: state.edge_progress.value(),
                    },
                )
            })
        };
        for vehicle in candidate.spatial_changes.iter().copied() {
            let old_membership = vehicles
                .get(vehicle.index())
                .filter(|slot| slot.generation == vehicle.generation())
                .and_then(|slot| slot.state.as_ref())
                .and_then(membership);
            let new_membership = candidate.state(vehicle).and_then(membership);
            spatial.sync_vehicle(old_membership, new_membership);
        }
    }
}
