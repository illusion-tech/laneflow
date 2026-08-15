use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RouteTransition {
    pub(super) to_edge: EdgeHandle,
    pub(super) gate: Option<ManeuverGateHandle>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct NextControlledRouteTransition {
    pub(super) from_route_edge_index: usize,
    pub(super) gate: ManeuverGateHandle,
    pub(super) distance_from_edge_start: BoundedDistance,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SpeedLimitRouteTransition {
    pub(super) from_route_edge_index: usize,
    pub(super) to_edge: EdgeHandle,
    pub(super) target_speed: f64,
}

#[derive(Clone, Debug, Default)]
pub(super) struct RouteReferenceIndex {
    pub(super) by_update_position: IndexMap<usize, VehicleHandle>,
}

impl PartialEq for RouteReferenceIndex {
    fn eq(&self, other: &Self) -> bool {
        // 精确派生索引的 container order/capacity 不属于 Core authority state。
        self.by_update_position.len() == other.by_update_position.len()
    }
}

impl RouteReferenceIndex {
    pub(super) fn live_count(&self) -> usize {
        self.by_update_position.len()
    }

    pub(super) fn reserve_for_attach(&mut self) {
        self.by_update_position.reserve(1);
    }

    pub(super) fn attach(&mut self, vehicle: VehicleHandle, position: usize) {
        assert_eq!(
            self.by_update_position.insert(position, vehicle),
            None,
            "update-order position must have one route reference"
        );
    }

    pub(super) fn detach(&mut self, vehicle: VehicleHandle, position: usize) {
        assert_eq!(
            self.by_update_position.swap_remove(&position),
            Some(vehicle),
            "route reference must identify detached vehicle"
        );
    }

    pub(super) fn replace(
        &mut self,
        old: VehicleHandle,
        new: VehicleHandle,
        update_order_position: usize,
    ) {
        let vehicle = self
            .by_update_position
            .get_mut(&update_order_position)
            .expect("replacement must preserve an existing route reference");
        assert_eq!(*vehicle, old, "route reference must identify old vehicle");
        *vehicle = new;
    }

    pub(super) fn clear(&mut self) {
        self.by_update_position.clear();
    }

    pub(super) fn first(&self) -> Option<VehicleHandle> {
        self.by_update_position
            .iter()
            .min_by_key(|(position, _)| *position)
            .map(|(_, vehicle)| *vehicle)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct RouteSlot {
    pub(super) generation: u32,
    pub(super) external_id: String,
    pub(super) edge_handles: Vec<EdgeHandle>,
    pub(super) transitions: Vec<RouteTransition>,
    pub(super) maneuver_occurrences: Vec<ManeuverOccurrence>,
    pub(super) gate_occurrences: Vec<GateOccurrence>,
    pub(super) waiting_zone_occurrences: Vec<WaitingZoneOccurrence>,
    pub(super) next_controlled_transition: Vec<Option<NextControlledRouteTransition>>,
    pub(super) speed_limit_transitions: Vec<SpeedLimitRouteTransition>,
    pub(super) active: bool,
}

#[derive(Clone, Copy)]
pub(super) struct VehicleAdvanceContext<'a> {
    pub(super) lane_graph: &'a LaneGraph,
    pub(super) signals: &'a SignalRegistry,
    pub(super) signal_state: &'a SignalRuntimeState,
    pub(super) routes: &'a [RouteSlot],
    pub(super) fixed_delta_time_ms: u64,
    pub(super) tick_index: u64,
}

#[derive(Clone, Debug)]
pub(super) struct VehicleSlot {
    pub(super) generation: u32,
    pub(super) external_id: String,
    pub(super) state: Option<VehicleState>,
    pub(super) update_order_position: Option<usize>,
}

impl PartialEq for VehicleSlot {
    fn eq(&self, other: &Self) -> bool {
        self.generation == other.generation
            && self.external_id == other.external_id
            && self.state == other.state
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct StableVehicleOrder {
    pub(super) entries: Vec<Option<VehicleHandle>>,
    pub(super) tombstones: usize,
}

impl PartialEq for StableVehicleOrder {
    fn eq(&self, other: &Self) -> bool {
        self.iter().eq(other.iter())
    }
}

impl StableVehicleOrder {
    pub(super) fn iter(&self) -> impl Iterator<Item = VehicleHandle> + '_ {
        self.entries.iter().filter_map(|entry| *entry)
    }

    pub(super) fn reserve_for_append(&mut self) {
        self.entries.reserve(1);
    }

    pub(super) fn append(&mut self, handle: VehicleHandle) -> usize {
        let position = self.entries.len();
        self.entries.push(Some(handle));
        position
    }

    pub(super) fn replace(&mut self, position: usize, old: VehicleHandle, new: VehicleHandle) {
        let entry = self
            .entries
            .get_mut(position)
            .expect("replacement update-order position must exist");
        assert_eq!(*entry, Some(old), "replacement must identify old vehicle");
        *entry = Some(new);
    }

    pub(super) fn tombstone(&mut self, position: usize, handle: VehicleHandle) {
        let entry = self
            .entries
            .get_mut(position)
            .expect("live vehicle reverse position must exist");
        assert_eq!(
            *entry,
            Some(handle),
            "reverse position must identify vehicle"
        );
        *entry = None;
        self.tombstones += 1;
    }

    pub(super) fn should_compact(&self) -> bool {
        let live = self.entries.len() - self.tombstones;
        live == 0 || self.tombstones >= live.max(64)
    }

    pub(super) fn compact(&mut self, vehicles: &mut [VehicleSlot]) -> bool {
        if !self.should_compact() {
            return false;
        }
        self.entries.retain(Option::is_some);
        for (position, handle) in self.iter().enumerate() {
            let slot = vehicles
                .get_mut(handle.index())
                .filter(|slot| slot.generation == handle.generation())
                .expect("compacted update order must contain only live vehicles");
            slot.update_order_position = Some(position);
        }
        self.tombstones = 0;
        true
    }
}

/// 可跨 tick 复用、但不属于 Core authority state 的候选车辆状态。
#[derive(Debug, Default)]
pub(super) struct CandidateStateScratch {
    pub(super) states: Vec<Option<VehicleState>>,
    pub(super) spatial_changes: Vec<VehicleHandle>,
    pub(super) parking_releases: Vec<ParkingStepRelease>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ParkingStepRelease {
    pub(super) vehicle: VehicleHandle,
    pub(super) space: crate::ParkingSpaceHandle,
}

pub(super) fn parking_arrived_state(
    vehicle: &VehicleState,
    target: Option<ParkingApproachTarget>,
    entry_progress: Option<f64>,
) -> bool {
    let (Some(target), Some(entry_progress)) = (target, entry_progress) else {
        return false;
    };
    vehicle.status == VehicleStatus::Active
        && vehicle.route == target.route
        && vehicle.route_edge_index == target.route_edge_index
        && longitudinal_positions_match(vehicle.edge_progress.value(), entry_progress)
        && vehicle.current_speed == Speed::ZERO
}

impl Clone for CandidateStateScratch {
    fn clone(&self) -> Self {
        let mut states = Vec::with_capacity(self.states.capacity());
        states.extend(self.states.iter().cloned());
        let mut spatial_changes = Vec::with_capacity(self.spatial_changes.capacity());
        spatial_changes.extend(self.spatial_changes.iter().copied());
        let mut parking_releases = Vec::with_capacity(self.parking_releases.capacity());
        parking_releases.extend(self.parking_releases.iter().copied());
        Self {
            states,
            spatial_changes,
            parking_releases,
        }
    }
}

impl PartialEq for CandidateStateScratch {
    fn eq(&self, _other: &Self) -> bool {
        // Scratch 的内容和 capacity 取决于运行历史，不参与 CoreWorld 语义相等。
        true
    }
}

impl CandidateStateScratch {
    pub(super) fn reserve_for_slots(&mut self, vehicle_slot_count: usize) {
        let additional = vehicle_slot_count.saturating_sub(self.states.len());
        self.states.reserve(additional);
        self.spatial_changes.reserve(additional);
    }

    pub(super) fn begin(&mut self, vehicles: &[VehicleSlot]) {
        self.states.clear();
        self.spatial_changes.clear();
        self.parking_releases.clear();
        self.states
            .extend(vehicles.iter().map(|slot| slot.state.clone()));
    }

    pub(super) fn state(&self, handle: VehicleHandle) -> Option<&VehicleState> {
        self.states.get(handle.index()).and_then(Option::as_ref)
    }

    pub(super) fn commit_into(&mut self, vehicles: &mut [VehicleSlot]) {
        assert_eq!(
            self.states.len(),
            vehicles.len(),
            "candidate state 数量必须与 vehicle slot 数量一致"
        );
        for (slot, next_state) in vehicles.iter_mut().zip(self.states.drain(..)) {
            slot.state = next_state;
        }
    }

    pub(super) fn clear(&mut self) {
        self.states.clear();
        self.spatial_changes.clear();
        self.parking_releases.clear();
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct NormalizedVehicleInput {
    pub(super) profile: VehicleProfileHandle,
    pub(super) route_edge_index: usize,
    pub(super) edge_progress: EdgeProgress,
    pub(super) current_speed: Speed,
    pub(super) status: VehicleStatus,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CandidateVehicleOverlap {
    pub(super) blocker: VehicleHandle,
    pub(super) blocker_position: VehicleReplaceBlockerPosition,
    pub(super) bumper_gap: f64,
}

pub(super) enum PreparedVehicleReplaceIds {
    Preserve,
    Replace { slot: String, resolver: String },
}

pub(super) fn parking_emergency_travel(
    stage: &'static str,
    vehicle: VehicleHandle,
    space: crate::ParkingSpaceHandle,
    speed: f64,
    emergency_deceleration: f64,
    delta_time: f64,
) -> Result<f64, CoreError> {
    emergency_min_travel(vehicle, speed, emergency_deceleration, delta_time).map_err(|error| {
        match error {
            CoreError::NonFiniteLongitudinalComputation { value, .. } => {
                CoreError::NonFiniteParkingComputation {
                    stage,
                    vehicle,
                    space,
                    value,
                }
            }
            error => error,
        }
    })
}
