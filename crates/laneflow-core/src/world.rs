//! Core world 与 fixed-step orchestration。

use std::{
    cmp::{Ordering, Reverse},
    collections::BinaryHeap,
};

use indexmap::IndexMap;

use crate::{
    command_spatial::{CommandOccupant, CommandSpatialIndex},
    error::CoreError,
    event::{
        CoreEvent, ParkingReservationReleasedEvent, SignalGroupAspectChangedEvent,
        SignalPhaseChangedEvent, VehicleChangedEdgeEvent, VehicleCompletedRouteEvent,
        VehicleFollowingSafetyProjectionAppliedEvent, VehicleParkingArrivalReachedEvent,
        VehicleParkingStopProjectionAppliedEvent, VehicleSignalStopProjectionAppliedEvent,
    },
    graph::LaneGraph,
    handle::{
        EdgeHandle, RouteHandle, SignalControllerHandle, SignalGroupHandle, VehicleHandle,
        VehicleProfileHandle,
    },
    id::validate_external_id,
    longitudinal::{
        LeaderKinematics, LongitudinalMotion, LongitudinalScratch, compute_motion,
        emergency_min_travel,
    },
    numeric_policy::{
        EDGE_BOUNDARY_TOLERANCE_METERS, LONGITUDINAL_CONSTRAINT_TOLERANCE_METERS,
        PHYSICAL_GAP_TOLERANCE_METERS, computed_speed_is_above_near_zero,
        is_edge_boundary_remainder_zero, longitudinal_constraint_reached,
        longitudinal_positions_match, normalize_physical_gap, physical_gap_is_overlap,
    },
    occupancy::{LeaderObservation, OccupancyScratch, Occupant},
    parking::{
        LeaveParkingInput, ParkedVehicleSpawnInput, ParkedVehicleSpawnRecord,
        ParkingApproachTarget, ParkingBindingKind, ParkingCommandEffect, ParkingCommandKind,
        ParkingCommitRecord, ParkingLeaveRecord, ParkingRegistry, ParkingReleaseReason,
        ParkingReleaseRecord, ParkingReservationCancellationRecord, ParkingReservationRecord,
        ParkingRuntimeState, ParkingSnapshot, ParkingSpaceState, ParkingStopConstraint,
        RebindReservedVehicleRouteInput, ReservedVehicleRouteRebindRecord,
        RuntimeVehicleParkingBinding,
    },
    profile::{VehicleProfile, VehicleProfileRegistry},
    route::{Route, RouteRemoveRecord},
    route_distance::{BoundedDistance, RouteDistanceIndex, RouteDistanceQuery},
    signal::{
        MovementGateIndex, MovementGateKey, MovementGateSignalState, MovementGateState,
        SignalControllerState, SignalGroupSnapshot, SignalLayerPermission, SignalRegistry,
        SignalRuntimeScratch, SignalRuntimeState, SignalStopConstraint,
    },
    time::{StepResult, TickInput},
    traffic::{InitialTrafficData, resolve_route_edges},
    vehicle::{
        Acceleration, EdgeProgress, Speed, VehicleDespawnRecord, VehicleSpawnInput, VehicleState,
        VehicleStatus,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RouteTransition {
    to_edge: EdgeHandle,
    gate: Option<MovementGateIndex>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct NextControlledRouteTransition {
    from_route_edge_index: usize,
    gate: MovementGateIndex,
    distance_from_edge_start: BoundedDistance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RouteVehicleReference {
    update_order_position: usize,
    vehicle: VehicleHandle,
    route_generation: u32,
}

impl Ord for RouteVehicleReference {
    fn cmp(&self, other: &Self) -> Ordering {
        self.update_order_position
            .cmp(&other.update_order_position)
            .then_with(|| self.vehicle.index().cmp(&other.vehicle.index()))
            .then_with(|| self.vehicle.generation().cmp(&other.vehicle.generation()))
            .then_with(|| self.route_generation.cmp(&other.route_generation))
    }
}

impl PartialOrd for RouteVehicleReference {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug, Default)]
struct RouteReferenceIndex {
    live_count: usize,
    candidates: BinaryHeap<Reverse<RouteVehicleReference>>,
}

impl PartialEq for RouteReferenceIndex {
    fn eq(&self, other: &Self) -> bool {
        // 精确派生索引的 heap history/capacity 不属于 Core authority state。
        self.live_count == other.live_count
    }
}

impl RouteReferenceIndex {
    fn reserve_for_attach(&mut self) {
        self.candidates.reserve(1);
    }

    fn attach(&mut self, route: RouteHandle, vehicle: VehicleHandle, position: usize) {
        self.live_count += 1;
        self.candidates.push(Reverse(RouteVehicleReference {
            update_order_position: position,
            vehicle,
            route_generation: route.generation(),
        }));
    }

    fn detach(&mut self) {
        self.live_count = self
            .live_count
            .checked_sub(1)
            .expect("route live reference count must not underflow");
    }

    fn clear(&mut self) {
        self.live_count = 0;
        self.candidates.clear();
    }

    fn first_valid(
        &mut self,
        route: RouteHandle,
        vehicles: &[VehicleSlot],
    ) -> Option<VehicleHandle> {
        while let Some(candidate) = self.candidates.peek().copied().map(|entry| entry.0) {
            let valid = candidate.route_generation == route.generation()
                && vehicles
                    .get(candidate.vehicle.index())
                    .filter(|slot| slot.generation == candidate.vehicle.generation())
                    .is_some_and(|slot| {
                        slot.update_order_position == Some(candidate.update_order_position)
                            && slot
                                .state
                                .as_ref()
                                .is_some_and(|state| state.route == route)
                    });
            if valid {
                return Some(candidate.vehicle);
            }
            self.candidates.pop();
        }
        None
    }
}

#[derive(Clone, Debug, PartialEq)]
struct RouteSlot {
    generation: u32,
    external_id: String,
    edge_handles: Vec<EdgeHandle>,
    transitions: Vec<RouteTransition>,
    next_controlled_transition: Vec<Option<NextControlledRouteTransition>>,
    active: bool,
}

#[derive(Clone, Copy)]
struct VehicleAdvanceContext<'a> {
    lane_graph: &'a LaneGraph,
    signals: &'a SignalRegistry,
    signal_state: &'a SignalRuntimeState,
    routes: &'a [RouteSlot],
    fixed_delta_time_ms: u64,
    tick_index: u64,
}

#[derive(Clone, Debug)]
struct VehicleSlot {
    generation: u32,
    external_id: String,
    state: Option<VehicleState>,
    update_order_position: Option<usize>,
}

impl PartialEq for VehicleSlot {
    fn eq(&self, other: &Self) -> bool {
        self.generation == other.generation
            && self.external_id == other.external_id
            && self.state == other.state
    }
}

#[derive(Clone, Debug, Default)]
struct StableVehicleOrder {
    entries: Vec<Option<VehicleHandle>>,
    tombstones: usize,
}

impl PartialEq for StableVehicleOrder {
    fn eq(&self, other: &Self) -> bool {
        self.iter().eq(other.iter())
    }
}

impl StableVehicleOrder {
    fn iter(&self) -> impl Iterator<Item = VehicleHandle> + '_ {
        self.entries.iter().filter_map(|entry| *entry)
    }

    fn reserve_for_append(&mut self) {
        self.entries.reserve(1);
    }

    fn append(&mut self, handle: VehicleHandle) -> usize {
        let position = self.entries.len();
        self.entries.push(Some(handle));
        position
    }

    fn tombstone(&mut self, position: usize, handle: VehicleHandle) {
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

    fn should_compact(&self) -> bool {
        let live = self.entries.len() - self.tombstones;
        live == 0 || self.tombstones >= live.max(64)
    }

    fn compact(&mut self, vehicles: &mut [VehicleSlot]) -> bool {
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
struct CandidateStateScratch {
    states: Vec<Option<VehicleState>>,
    spatial_changes: Vec<VehicleHandle>,
    parking_releases: Vec<ParkingStepRelease>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ParkingStepRelease {
    vehicle: VehicleHandle,
    space: crate::ParkingSpaceHandle,
}

fn parking_arrived_state(
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

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
struct LifecycleRetainedStats {
    accounted_bytes: usize,
    expanded_accounted_bytes: usize,
    complete_accounted_bytes: usize,
    owned_heap_bytes: usize,
    world_inline_bytes: usize,
    lane_graph_bytes: usize,
    vehicle_profile_registry_bytes: usize,
    signal_registry_bytes: usize,
    signal_runtime_state_bytes: usize,
    signal_runtime_scratch_bytes: usize,
    route_bytes: usize,
    route_distance_bytes: usize,
    route_reference_bytes: usize,
    vehicle_bytes: usize,
    resolver_bytes: usize,
    free_list_bytes: usize,
    vehicle_order_bytes: usize,
    candidate_state_bytes: usize,
    parking_bytes: usize,
    parking_registry_runtime_bytes: usize,
    occupancy_scratch_bytes: usize,
    longitudinal_scratch_bytes: usize,
    command_spatial_bytes: usize,
    lane_graph_inline_size: usize,
    vehicle_profile_registry_inline_size: usize,
    signal_registry_inline_size: usize,
    signal_runtime_state_inline_size: usize,
    signal_runtime_scratch_inline_size: usize,
    vehicle_state_size: usize,
    vehicle_slot_size: usize,
    live_vehicles: usize,
    route_occurrences: usize,
    tombstones: usize,
    route_candidate_nodes: usize,
    stale_route_candidate_nodes: usize,
    spatial_occupants: usize,
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
    fn reserve_for_slots(&mut self, vehicle_slot_count: usize) {
        let additional = vehicle_slot_count.saturating_sub(self.states.len());
        self.states.reserve(additional);
        self.spatial_changes.reserve(additional);
    }

    fn begin(&mut self, vehicles: &[VehicleSlot]) {
        self.states.clear();
        self.spatial_changes.clear();
        self.parking_releases.clear();
        self.states
            .extend(vehicles.iter().map(|slot| slot.state.clone()));
    }

    fn state(&self, handle: VehicleHandle) -> Option<&VehicleState> {
        self.states.get(handle.index()).and_then(Option::as_ref)
    }

    fn commit_into(&mut self, vehicles: &mut [VehicleSlot]) {
        assert_eq!(
            self.states.len(),
            vehicles.len(),
            "candidate state 数量必须与 vehicle slot 数量一致"
        );
        for (slot, next_state) in vehicles.iter_mut().zip(self.states.drain(..)) {
            slot.state = next_state;
        }
    }

    fn clear(&mut self) {
        self.states.clear();
        self.spatial_changes.clear();
        self.parking_releases.clear();
    }
}

/// LaneFlow Core 的最小 runtime state。
#[derive(Clone, Debug, PartialEq)]
pub struct CoreWorld {
    fixed_delta_time_ms: u64,
    tick_index: u64,
    time_ms: u64,
    lane_graph: LaneGraph,
    vehicle_profiles: VehicleProfileRegistry,
    signals: SignalRegistry,
    parking: ParkingRegistry,
    pub(crate) parking_runtime: ParkingRuntimeState,
    signal_state: SignalRuntimeState,
    signal_candidate_scratch: SignalRuntimeScratch,
    routes: Vec<RouteSlot>,
    route_distance_indices: Vec<RouteDistanceIndex>,
    route_reference_indices: Vec<RouteReferenceIndex>,
    route_handles: IndexMap<String, RouteHandle>,
    free_route_indices: Vec<usize>,
    vehicles: Vec<VehicleSlot>,
    vehicle_handles: IndexMap<String, VehicleHandle>,
    free_vehicle_indices: Vec<usize>,
    vehicle_update_order: StableVehicleOrder,
    candidate_state_scratch: CandidateStateScratch,
    occupancy_scratch: OccupancyScratch,
    longitudinal_scratch: LongitudinalScratch,
    command_spatial_index: CommandSpatialIndex,
    #[cfg(test)]
    step_failure_after_vehicle: Option<VehicleHandle>,
}

impl CoreWorld {
    /// 创建不含 traffic data 和车辆的 Core world。
    pub fn new(fixed_delta_time_ms: u64) -> Result<Self, CoreError> {
        Self::with_traffic_data(fixed_delta_time_ms, InitialTrafficData::empty(), Vec::new())
    }

    /// 创建包含已验证 traffic data 和初始车辆的 Core world。
    pub fn with_traffic_data(
        fixed_delta_time_ms: u64,
        traffic_data: InitialTrafficData,
        mut vehicles: Vec<VehicleSpawnInput>,
    ) -> Result<Self, CoreError> {
        if fixed_delta_time_ms == 0 {
            return Err(CoreError::InvalidFixedDeltaTime {
                fixed_delta_time_ms,
            });
        }

        let (lane_graph, routes, vehicle_profiles, signals, parking) = traffic_data.into_parts();
        signals.validate_fixed_delta_time(fixed_delta_time_ms)?;
        let mut signal_state = SignalRuntimeState::default();
        signals.populate_runtime_state(0, &mut signal_state);
        let command_spatial_index = CommandSpatialIndex::new(&lane_graph, &vehicle_profiles);
        let parking_runtime = ParkingRuntimeState::new(&parking);
        let mut world = Self {
            fixed_delta_time_ms,
            tick_index: 0,
            time_ms: 0,
            lane_graph,
            vehicle_profiles,
            signals,
            parking,
            parking_runtime,
            signal_state,
            signal_candidate_scratch: SignalRuntimeScratch::default(),
            routes: Vec::new(),
            route_distance_indices: Vec::new(),
            route_reference_indices: Vec::new(),
            route_handles: IndexMap::new(),
            free_route_indices: Vec::new(),
            vehicles: Vec::new(),
            vehicle_handles: IndexMap::new(),
            free_vehicle_indices: Vec::new(),
            vehicle_update_order: StableVehicleOrder::default(),
            candidate_state_scratch: CandidateStateScratch::default(),
            occupancy_scratch: OccupancyScratch::default(),
            longitudinal_scratch: LongitudinalScratch::default(),
            command_spatial_index,
            #[cfg(test)]
            step_failure_after_vehicle: None,
        };

        for route in routes {
            world.register_route(route)?;
        }

        vehicles.sort_by(|left, right| left.id.cmp(&right.id));
        for vehicle in vehicles {
            world.spawn_vehicle_without_overlap_validation(vehicle)?;
        }
        world.validate_initial_vehicle_overlaps()?;
        world.rebuild_command_spatial_index();

        Ok(world)
    }

    /// 返回当前 world 的固定 tick 步长。
    pub const fn fixed_delta_time_ms(&self) -> u64 {
        self.fixed_delta_time_ms
    }

    /// 返回当前 tick index。
    pub const fn tick_index(&self) -> u64 {
        self.tick_index
    }

    /// 返回当前累计 simulation time。
    pub const fn time_ms(&self) -> u64 {
        self.time_ms
    }

    /// 返回当前 live vehicle 状态，按稳定更新顺序输出。
    pub fn vehicles(&self) -> impl Iterator<Item = &VehicleState> {
        self.vehicle_update_order
            .iter()
            .filter_map(|handle| self.vehicle(handle))
    }

    /// 返回指定 vehicle handle 的状态。
    pub fn vehicle(&self, handle: VehicleHandle) -> Option<&VehicleState> {
        self.vehicle_slot(handle)
            .and_then(|vehicle| vehicle.state.as_ref())
    }

    /// 返回 vehicle external ID 对应的 handle。
    pub fn vehicle_handle(&self, id: &str) -> Option<VehicleHandle> {
        self.vehicle_handles.get(id).copied()
    }

    /// 返回 vehicle handle 对应的 external ID。
    pub fn vehicle_external_id(&self, handle: VehicleHandle) -> Option<&str> {
        self.vehicle_slot(handle)
            .map(|vehicle| vehicle.external_id.as_str())
    }

    /// 返回 immutable Vehicle Profile registry。
    pub const fn vehicle_profiles(&self) -> &VehicleProfileRegistry {
        &self.vehicle_profiles
    }

    /// 返回 immutable Signals registry。
    pub const fn signals(&self) -> &SignalRegistry {
        &self.signals
    }

    /// 返回 immutable Parking registry。
    pub const fn parking(&self) -> &ParkingRegistry {
        &self.parking
    }

    /// 返回借用当前 committed world 的 immutable Parking snapshot。
    pub const fn parking_snapshot(&self) -> ParkingSnapshot<'_> {
        ParkingSnapshot::new(self)
    }

    /// 为 live Active vehicle 预订 caller-selected ParkingSpace。
    pub fn reserve_parking_space(
        &mut self,
        vehicle: VehicleHandle,
        space: crate::ParkingSpaceHandle,
    ) -> Result<ParkingReservationRecord, CoreError> {
        let vehicle_state = self
            .vehicle(vehicle)
            .ok_or(CoreError::UnknownVehicleHandle { vehicle })?;
        let status = vehicle_state.status;
        if self.parking.space(space).is_none() {
            return Err(CoreError::UnknownParkingSpaceHandle { space });
        }

        let binding = self.parking_runtime.vehicle_binding(vehicle);
        let space_state = self
            .parking_runtime
            .space_state(space)
            .expect("resolved ParkingSpace must have runtime state");
        if matches!(
            binding,
            Some(RuntimeVehicleParkingBinding::Reserved {
                space: current,
                ..
            }) if current == space
        ) && space_state == (ParkingSpaceState::Reserved { vehicle })
        {
            return Ok(ParkingReservationRecord {
                vehicle,
                space,
                effect: ParkingCommandEffect::AlreadySatisfied,
            });
        }
        if status != VehicleStatus::Active {
            return Err(CoreError::ParkingVehicleStatusMismatch {
                command: ParkingCommandKind::Reserve,
                vehicle,
                expected: VehicleStatus::Active,
                actual: status,
            });
        }
        if let Some(binding) = binding {
            return Err(CoreError::ParkingVehicleAlreadyBound {
                command: ParkingCommandKind::Reserve,
                vehicle,
                requested_space: space,
                current_space: binding.space(),
                binding: binding.kind(),
            });
        }
        match space_state {
            ParkingSpaceState::Vacant => {}
            ParkingSpaceState::Reserved {
                vehicle: current_vehicle,
            } => {
                return Err(CoreError::ParkingSpaceUnavailable {
                    command: ParkingCommandKind::Reserve,
                    space,
                    requested_vehicle: vehicle,
                    current_vehicle,
                    binding: ParkingBindingKind::Reserved,
                });
            }
            ParkingSpaceState::Occupied {
                vehicle: current_vehicle,
            } => {
                return Err(CoreError::ParkingSpaceUnavailable {
                    command: ParkingCommandKind::Reserve,
                    space,
                    requested_vehicle: vehicle,
                    current_vehicle,
                    binding: ParkingBindingKind::Occupied,
                });
            }
        }

        let target = self.first_reachable_parking_entry(
            vehicle_state.route,
            vehicle_state.route_edge_index,
            vehicle_state.edge_progress.value(),
            space,
        );
        self.parking_runtime
            .reserve(&self.parking, vehicle, space, target);
        Ok(ParkingReservationRecord {
            vehicle,
            space,
            effect: ParkingCommandEffect::Applied,
        })
    }

    /// 只取消 exact Reserved pair；不会强制释放其他 owner。
    pub fn cancel_parking_reservation(
        &mut self,
        vehicle: VehicleHandle,
        space: crate::ParkingSpaceHandle,
    ) -> Result<ParkingReservationCancellationRecord, CoreError> {
        self.vehicle(vehicle)
            .ok_or(CoreError::UnknownVehicleHandle { vehicle })?;
        if self.parking.space(space).is_none() {
            return Err(CoreError::UnknownParkingSpaceHandle { space });
        }
        let binding = self.parking_runtime.vehicle_binding(vehicle);
        let space_state = self
            .parking_runtime
            .space_state(space)
            .expect("resolved ParkingSpace must have runtime state");
        if matches!(
            binding,
            Some(RuntimeVehicleParkingBinding::Reserved {
                space: current,
                ..
            }) if current == space
        ) && space_state == (ParkingSpaceState::Reserved { vehicle })
        {
            self.parking_runtime.cancel(&self.parking, vehicle, space);
            return Ok(ParkingReservationCancellationRecord {
                vehicle,
                space,
                effect: ParkingCommandEffect::Applied,
            });
        }
        if binding.is_none() && space_state == ParkingSpaceState::Vacant {
            return Ok(ParkingReservationCancellationRecord {
                vehicle,
                space,
                effect: ParkingCommandEffect::AlreadySatisfied,
            });
        }
        Err(CoreError::ParkingReservationMismatch {
            command: ParkingCommandKind::CancelReservation,
            vehicle,
            space,
        })
    }

    /// 把 exact Arrived reservation 原子提交为 Occupied + Parked。
    pub fn commit_parking(
        &mut self,
        vehicle: VehicleHandle,
        space: crate::ParkingSpaceHandle,
    ) -> Result<ParkingCommitRecord, CoreError> {
        let state = self
            .vehicle(vehicle)
            .ok_or(CoreError::UnknownVehicleHandle { vehicle })?;
        let status = state.status;
        if self.parking.space(space).is_none() {
            return Err(CoreError::UnknownParkingSpaceHandle { space });
        }
        let binding = self.parking_runtime.vehicle_binding(vehicle);
        let space_state = self
            .parking_runtime
            .space_state(space)
            .expect("resolved ParkingSpace must have runtime state");
        if status == VehicleStatus::Parked
            && matches!(
                binding,
                Some(RuntimeVehicleParkingBinding::Occupied {
                    space: current,
                    ..
                }) if current == space
            )
            && space_state == (ParkingSpaceState::Occupied { vehicle })
        {
            return Ok(ParkingCommitRecord {
                vehicle,
                space,
                effect: ParkingCommandEffect::AlreadySatisfied,
            });
        }
        if status != VehicleStatus::Active {
            return Err(CoreError::ParkingVehicleStatusMismatch {
                command: ParkingCommandKind::Commit,
                vehicle,
                expected: VehicleStatus::Active,
                actual: status,
            });
        }
        let target = match binding {
            Some(RuntimeVehicleParkingBinding::Reserved {
                space: current,
                target,
                ..
            }) if current == space && space_state == (ParkingSpaceState::Reserved { vehicle }) => {
                target
            }
            _ => {
                return Err(CoreError::ParkingReservationMismatch {
                    command: ParkingCommandKind::Commit,
                    vehicle,
                    space,
                });
            }
        };
        if !self.parking_arrived(vehicle, space, target) {
            return Err(CoreError::ParkingVehicleNotArrived { vehicle, space });
        }

        let edge = self.vehicle_edge(state);
        let removed_speed = state.current_speed.value();
        let occupant = CommandOccupant {
            vehicle,
            front_progress: state.edge_progress,
        };
        let mut spatial = std::mem::take(&mut self.command_spatial_index);
        let vehicles = &self.vehicles;
        spatial.prepare_speed_removal(
            removed_speed,
            vehicles.iter().filter_map(|slot| {
                let state = slot.state.as_ref()?;
                (state.status == VehicleStatus::Active)
                    .then_some((state.handle, state.current_speed.value()))
            }),
        );
        let mut resolve_progress = |candidate: VehicleHandle| {
            vehicles[candidate.index()]
                .state
                .as_ref()
                .expect("command spatial occupant must be live")
                .edge_progress
                .value()
        };
        spatial.remove(edge, occupant, &mut resolve_progress);
        self.command_spatial_index = spatial;

        let entry = self
            .parking
            .space_entry(space)
            .expect("resolved ParkingSpace must have entry");
        let state = self.vehicles[vehicle.index()]
            .state
            .as_mut()
            .expect("resolved vehicle must remain live");
        state.edge_progress = EdgeProgress::try_new(entry.progress()).expect("entry is canonical");
        state.current_speed = Speed::ZERO;
        state.applied_acceleration = Acceleration::ZERO;
        state.status = VehicleStatus::Parked;
        self.parking_runtime.commit(&self.parking, vehicle, space);
        Ok(ParkingCommitRecord {
            vehicle,
            space,
            effect: ParkingCommandEffect::Applied,
        })
    }

    /// 原子创建 off-lane Parked vehicle 与 Occupied binding。
    pub fn spawn_parked_vehicle(
        &mut self,
        input: ParkedVehicleSpawnInput,
    ) -> Result<ParkedVehicleSpawnRecord, CoreError> {
        validate_external_id("vehicles[].id", &input.id)?;
        validate_external_id("vehicles[].routeId", &input.route_id)?;
        if self.vehicle_handles.contains_key(&input.id) {
            return Err(CoreError::DuplicateVehicleId {
                vehicle_id: input.id,
            });
        }
        if self.vehicle_profile(input.profile).is_none() {
            return Err(CoreError::UnknownVehicleProfileHandle {
                vehicle_id: input.id,
                profile: input.profile,
            });
        }
        let route =
            self.route_handle(&input.route_id)
                .ok_or_else(|| CoreError::UnknownVehicleRoute {
                    vehicle_id: input.id.clone(),
                    route_id: input.route_id.clone(),
                })?;
        if self.parking.space(input.space).is_none() {
            return Err(CoreError::UnknownParkingSpaceHandle { space: input.space });
        }

        let planned_slot_index = self
            .free_vehicle_indices
            .last()
            .copied()
            .unwrap_or(self.vehicles.len());
        let planned_generation = self
            .vehicles
            .get(planned_slot_index)
            .map_or(0, |slot| slot.generation);
        let handle = VehicleHandle::new(planned_slot_index, planned_generation);
        match self
            .parking_runtime
            .space_state(input.space)
            .expect("resolved ParkingSpace must have runtime state")
        {
            ParkingSpaceState::Vacant => {}
            ParkingSpaceState::Reserved { vehicle } => {
                return Err(CoreError::ParkingSpaceUnavailable {
                    command: ParkingCommandKind::SpawnParkedVehicle,
                    space: input.space,
                    requested_vehicle: handle,
                    current_vehicle: vehicle,
                    binding: ParkingBindingKind::Reserved,
                });
            }
            ParkingSpaceState::Occupied { vehicle } => {
                return Err(CoreError::ParkingSpaceUnavailable {
                    command: ParkingCommandKind::SpawnParkedVehicle,
                    space: input.space,
                    requested_vehicle: handle,
                    current_vehicle: vehicle,
                    binding: ParkingBindingKind::Occupied,
                });
            }
        }
        let route_slot = self
            .route_slot(route)
            .expect("resolved route handle must remain active");
        let Some(actual_edge) = route_slot.edge_handles.get(input.route_edge_index).copied() else {
            return Err(CoreError::InvalidParkingRouteOccurrence {
                command: ParkingCommandKind::SpawnParkedVehicle,
                vehicle: handle,
                route,
                route_edge_index: input.route_edge_index,
                route_edge_count: route_slot.edge_handles.len(),
            });
        };
        let entry = self
            .parking
            .space_entry(input.space)
            .expect("resolved ParkingSpace must have entry");
        if actual_edge != entry.edge() {
            return Err(CoreError::ParkingRouteOccurrenceEdgeMismatch {
                command: ParkingCommandKind::SpawnParkedVehicle,
                space: input.space,
                anchor: crate::ParkingAnchorKind::Entry,
                route,
                route_edge_index: input.route_edge_index,
                expected_edge: entry.edge(),
                actual_edge,
            });
        }

        self.vehicle_handles.reserve(1);
        self.vehicle_update_order.reserve_for_append();
        if planned_slot_index == self.vehicles.len() {
            self.vehicles.reserve(1);
        }
        self.route_reference_indices[route.index()].reserve_for_attach();
        self.parking_runtime
            .prepare_vehicle_slot(planned_slot_index);
        self.candidate_state_scratch.reserve_for_slots(
            self.vehicles.len() + usize::from(planned_slot_index == self.vehicles.len()),
        );

        let external_id = input.id;
        let resolver_id = external_id.clone();
        let update_order_position = self.vehicle_update_order.append(handle);
        let state = VehicleState::new(
            handle,
            input.profile,
            route,
            input.route_edge_index,
            EdgeProgress::try_new(entry.progress()).expect("entry progress is canonical"),
            Speed::ZERO,
            VehicleStatus::Parked,
        );
        let slot = VehicleSlot {
            generation: planned_generation,
            external_id,
            state: Some(state),
            update_order_position: Some(update_order_position),
        };
        if planned_slot_index < self.vehicles.len() {
            let popped = self
                .free_vehicle_indices
                .pop()
                .expect("planned reusable vehicle slot must remain available");
            assert_eq!(popped, planned_slot_index);
            self.vehicles[planned_slot_index] = slot;
        } else {
            self.vehicles.push(slot);
        }
        self.vehicle_handles.insert(resolver_id, handle);
        self.route_reference_indices[route.index()].attach(route, handle, update_order_position);
        self.parking_runtime
            .occupy_new(&self.parking, handle, input.space);
        self.compact_update_order_if_needed();
        Ok(ParkedVehicleSpawnRecord {
            vehicle: handle,
            space: input.space,
            route,
            route_edge_index: input.route_edge_index,
        })
    }

    /// 在不 teleport 的前提下替换 exact Reserved Active vehicle 的 route occurrence。
    pub fn rebind_reserved_vehicle_route(
        &mut self,
        input: RebindReservedVehicleRouteInput,
    ) -> Result<ReservedVehicleRouteRebindRecord, CoreError> {
        let state = self
            .vehicle(input.vehicle)
            .ok_or(CoreError::UnknownVehicleHandle {
                vehicle: input.vehicle,
            })?;
        let status = state.status;
        let from_route = state.route;
        let from_route_edge_index = state.route_edge_index;
        let current_progress = state.edge_progress;
        let profile = state.profile;
        if self.parking.space(input.space).is_none() {
            return Err(CoreError::UnknownParkingSpaceHandle { space: input.space });
        }
        let route_slot = self
            .route_slot(input.route)
            .ok_or(CoreError::UnknownRouteHandle { route: input.route })?;
        let Some(target_edge) = route_slot.edge_handles.get(input.route_edge_index).copied() else {
            return Err(CoreError::InvalidParkingRouteOccurrence {
                command: ParkingCommandKind::RebindReservedVehicleRoute,
                vehicle: input.vehicle,
                route: input.route,
                route_edge_index: input.route_edge_index,
                route_edge_count: route_slot.edge_handles.len(),
            });
        };
        if status != VehicleStatus::Active {
            return Err(CoreError::ParkingVehicleStatusMismatch {
                command: ParkingCommandKind::RebindReservedVehicleRoute,
                vehicle: input.vehicle,
                expected: VehicleStatus::Active,
                actual: status,
            });
        }
        let exact_reservation = matches!(
            self.parking_runtime.vehicle_binding(input.vehicle),
            Some(RuntimeVehicleParkingBinding::Reserved { space, .. })
                if space == input.space
        ) && self.parking_runtime.space_state(input.space)
            == Some(ParkingSpaceState::Reserved {
                vehicle: input.vehicle,
            });
        if !exact_reservation {
            return Err(CoreError::ParkingReservationMismatch {
                command: ParkingCommandKind::RebindReservedVehicleRoute,
                vehicle: input.vehicle,
                space: input.space,
            });
        }
        if from_route == input.route && from_route_edge_index == input.route_edge_index {
            return Ok(ReservedVehicleRouteRebindRecord {
                vehicle: input.vehicle,
                space: input.space,
                from_route,
                from_route_edge_index,
                to_route: input.route,
                to_route_edge_index: input.route_edge_index,
                effect: ParkingCommandEffect::AlreadySatisfied,
            });
        }

        let current_edge = self
            .routes
            .get(from_route.index())
            .and_then(|route| route.edge_handles.get(from_route_edge_index))
            .copied()
            .expect("live vehicle occurrence must remain valid");
        if target_edge != current_edge {
            return Err(CoreError::ParkingRouteRebindEdgeMismatch {
                vehicle: input.vehicle,
                space: input.space,
                route: input.route,
                route_edge_index: input.route_edge_index,
                current_edge,
                target_edge,
            });
        }
        let target = self
            .first_reachable_parking_entry(
                input.route,
                input.route_edge_index,
                current_progress.value(),
                input.space,
            )
            .ok_or(CoreError::ParkingEntryUnreachable {
                vehicle: input.vehicle,
                space: input.space,
                route: input.route,
                from_route_edge_index: input.route_edge_index,
            })?;
        let candidate = NormalizedVehicleInput {
            profile,
            route_edge_index: input.route_edge_index,
            edge_progress: current_progress,
            current_speed: state.current_speed,
            status: VehicleStatus::Active,
        };
        self.validate_candidate_overlap_excluding(input.vehicle, input.route, &candidate)?;

        let update_order_position = self.vehicles[input.vehicle.index()]
            .update_order_position
            .expect("live vehicle must have update order");
        if from_route != input.route {
            self.route_reference_indices[input.route.index()].reserve_for_attach();
        }
        let state = self.vehicles[input.vehicle.index()]
            .state
            .as_mut()
            .expect("resolved vehicle must remain live");
        state.route = input.route;
        state.route_edge_index = input.route_edge_index;
        if from_route != input.route {
            self.route_reference_indices[from_route.index()].detach();
            self.route_reference_indices[input.route.index()].attach(
                input.route,
                input.vehicle,
                update_order_position,
            );
        }
        self.parking_runtime.rebind_target(input.vehicle, target);
        Ok(ReservedVehicleRouteRebindRecord {
            vehicle: input.vehicle,
            space: input.space,
            from_route,
            from_route_edge_index,
            to_route: input.route,
            to_route_edge_index: input.route_edge_index,
            effect: ParkingCommandEffect::Applied,
        })
    }

    /// 把 exact Occupied/Parked pair 安全插入 caller-selected exit occurrence。
    pub fn leave_parking(
        &mut self,
        input: LeaveParkingInput,
    ) -> Result<ParkingLeaveRecord, CoreError> {
        let state = self
            .vehicle(input.vehicle)
            .ok_or(CoreError::UnknownVehicleHandle {
                vehicle: input.vehicle,
            })?;
        let status = state.status;
        let from_route = state.route;
        let profile = state.profile;
        if self.parking.space(input.space).is_none() {
            return Err(CoreError::UnknownParkingSpaceHandle { space: input.space });
        }
        let route_slot = self
            .route_slot(input.route)
            .ok_or(CoreError::UnknownRouteHandle { route: input.route })?;
        let Some(actual_edge) = route_slot.edge_handles.get(input.route_edge_index).copied() else {
            return Err(CoreError::InvalidParkingRouteOccurrence {
                command: ParkingCommandKind::Leave,
                vehicle: input.vehicle,
                route: input.route,
                route_edge_index: input.route_edge_index,
                route_edge_count: route_slot.edge_handles.len(),
            });
        };
        let exit = self
            .parking
            .space_exit(input.space)
            .expect("resolved ParkingSpace must have exit");
        if actual_edge != exit.edge() {
            return Err(CoreError::ParkingRouteOccurrenceEdgeMismatch {
                command: ParkingCommandKind::Leave,
                space: input.space,
                anchor: crate::ParkingAnchorKind::Exit,
                route: input.route,
                route_edge_index: input.route_edge_index,
                expected_edge: exit.edge(),
                actual_edge,
            });
        }

        let binding = self.parking_runtime.vehicle_binding(input.vehicle);
        let space_state = self
            .parking_runtime
            .space_state(input.space)
            .expect("resolved ParkingSpace must have runtime state");
        let exact_noop = status == VehicleStatus::Active
            && binding.is_none()
            && space_state == ParkingSpaceState::Vacant
            && state.route == input.route
            && state.route_edge_index == input.route_edge_index
            && state.edge_progress.value() == exit.progress()
            && state.current_speed == Speed::ZERO
            && state.applied_acceleration == Acceleration::ZERO;
        if exact_noop {
            return Ok(ParkingLeaveRecord {
                vehicle: input.vehicle,
                space: input.space,
                route: input.route,
                route_edge_index: input.route_edge_index,
                effect: ParkingCommandEffect::AlreadySatisfied,
            });
        }
        if status != VehicleStatus::Parked {
            return Err(CoreError::ParkingVehicleStatusMismatch {
                command: ParkingCommandKind::Leave,
                vehicle: input.vehicle,
                expected: VehicleStatus::Parked,
                actual: status,
            });
        }
        let exact_occupancy = matches!(
            binding,
            Some(RuntimeVehicleParkingBinding::Occupied { space, .. })
                if space == input.space
        ) && space_state
            == (ParkingSpaceState::Occupied {
                vehicle: input.vehicle,
            });
        if !exact_occupancy {
            return Err(CoreError::ParkingOccupancyMismatch {
                command: ParkingCommandKind::Leave,
                vehicle: input.vehicle,
                space: input.space,
            });
        }

        let candidate = NormalizedVehicleInput {
            profile,
            route_edge_index: input.route_edge_index,
            edge_progress: EdgeProgress::try_new(exit.progress()).expect("exit is canonical"),
            current_speed: Speed::ZERO,
            status: VehicleStatus::Active,
        };
        self.validate_candidate_overlap_excluding(input.vehicle, input.route, &candidate)?;
        self.validate_parking_leave_followers(
            input.vehicle,
            input.space,
            input.route,
            input.route_edge_index,
            &candidate,
        )?;

        let update_order_position = self.vehicles[input.vehicle.index()]
            .update_order_position
            .expect("live vehicle must have update order");
        if from_route != input.route {
            self.route_reference_indices[input.route.index()].reserve_for_attach();
        }
        let occupant = CommandOccupant {
            vehicle: input.vehicle,
            front_progress: EdgeProgress::try_new(exit.progress()).expect("exit is canonical"),
        };
        let mut spatial = std::mem::take(&mut self.command_spatial_index);
        let vehicles = &self.vehicles;
        let mut resolve_progress = |handle: VehicleHandle| {
            vehicles[handle.index()]
                .state
                .as_ref()
                .expect("command spatial occupant must be live")
                .edge_progress
                .value()
        };
        spatial.prepare_insert(exit.edge(), occupant, &mut resolve_progress);

        let state = self.vehicles[input.vehicle.index()]
            .state
            .as_mut()
            .expect("resolved vehicle must remain live");
        state.route = input.route;
        state.route_edge_index = input.route_edge_index;
        state.edge_progress = candidate.edge_progress;
        state.current_speed = Speed::ZERO;
        state.applied_acceleration = Acceleration::ZERO;
        state.status = VehicleStatus::Active;
        if from_route != input.route {
            self.route_reference_indices[from_route.index()].detach();
            self.route_reference_indices[input.route.index()].attach(
                input.route,
                input.vehicle,
                update_order_position,
            );
        }
        let released = self.parking_runtime.release(&self.parking, input.vehicle);
        assert_eq!(released, Some((input.space, ParkingBindingKind::Occupied)));
        let vehicles = &self.vehicles;
        let mut resolve_progress = |handle: VehicleHandle| {
            vehicles[handle.index()]
                .state
                .as_ref()
                .expect("command spatial occupant must be live")
                .edge_progress
                .value()
        };
        spatial.insert(exit.edge(), occupant, &mut resolve_progress);
        self.command_spatial_index = spatial;
        Ok(ParkingLeaveRecord {
            vehicle: input.vehicle,
            space: input.space,
            route: input.route,
            route_edge_index: input.route_edge_index,
            effect: ParkingCommandEffect::Applied,
        })
    }

    fn first_reachable_parking_entry(
        &self,
        route: RouteHandle,
        from_route_edge_index: usize,
        from_progress: f64,
        space: crate::ParkingSpaceHandle,
    ) -> Option<ParkingApproachTarget> {
        let entry = self
            .parking
            .space_entry(space)
            .expect("resolved ParkingSpace must have entry");
        let route_slot = self
            .route_slot(route)
            .expect("live vehicle route must remain active");
        let current_matches = route_slot.edge_handles[from_route_edge_index] == entry.edge()
            && longitudinal_constraint_reached(entry.progress(), from_progress);
        let route_edge_index = if current_matches {
            from_route_edge_index
        } else {
            route_slot
                .edge_handles
                .iter()
                .copied()
                .enumerate()
                .skip(from_route_edge_index + 1)
                .find_map(|(index, edge)| (edge == entry.edge()).then_some(index))?
        };
        Some(ParkingApproachTarget {
            route,
            route_edge_index,
        })
    }

    fn parking_arrived(
        &self,
        vehicle: VehicleHandle,
        space: crate::ParkingSpaceHandle,
        target: Option<ParkingApproachTarget>,
    ) -> bool {
        let Some(target) = target else {
            return false;
        };
        let Some(state) = self.vehicle(vehicle) else {
            return false;
        };
        let entry = self
            .parking
            .space_entry(space)
            .expect("resolved ParkingSpace must have entry");
        state.status == VehicleStatus::Active
            && state.route == target.route
            && state.route_edge_index == target.route_edge_index
            && longitudinal_positions_match(state.edge_progress.value(), entry.progress())
            && state.current_speed == Speed::ZERO
    }

    /// 返回当前已提交的 controller snapshot。
    pub fn signal_controller_state(
        &self,
        handle: SignalControllerHandle,
    ) -> Option<SignalControllerState> {
        self.signal_state.controller_state(handle)
    }

    /// 按 controller normalization order 遍历当前 snapshots。
    pub fn signal_controller_states(
        &self,
    ) -> impl ExactSizeIterator<Item = (SignalControllerHandle, SignalControllerState)> + '_ {
        self.signal_state.controller_states()
    }

    /// 返回当前已提交的 SignalGroup snapshot。
    pub fn signal_group_state(&self, handle: SignalGroupHandle) -> Option<SignalGroupSnapshot> {
        self.signal_state.group_state(handle)
    }

    /// 按 SignalGroup normalization order 遍历当前 snapshots。
    pub fn signal_group_states(
        &self,
    ) -> impl ExactSizeIterator<Item = (SignalGroupHandle, SignalGroupSnapshot)> + '_ {
        self.signal_state.group_states()
    }

    /// 返回当前已提交的 MovementGate signal-layer snapshot。
    pub fn movement_gate_state(&self, key: MovementGateKey) -> Option<MovementGateState> {
        self.signals.movement_gate_state(&self.signal_state, key)
    }

    /// 按 MovementGate normalization order 遍历当前 snapshots。
    pub fn movement_gate_states(&self) -> impl ExactSizeIterator<Item = MovementGateState> + '_ {
        self.signals.movement_gates().map(|key| {
            self.movement_gate_state(key)
                .expect("normalized MovementGate must have runtime state")
        })
    }

    /// 返回指定 handle 的 Vehicle Profile。
    pub fn vehicle_profile(&self, handle: VehicleProfileHandle) -> Option<&VehicleProfile> {
        self.vehicle_profiles.profile(handle)
    }

    /// 返回 Vehicle Profile external ID 对应的 handle。
    pub fn vehicle_profile_handle(&self, id: &str) -> Option<VehicleProfileHandle> {
        self.vehicle_profiles.profile_handle(id)
    }

    /// 返回 Vehicle Profile handle 对应的 external ID。
    pub fn vehicle_profile_external_id(&self, handle: VehicleProfileHandle) -> Option<&str> {
        self.vehicle_profiles.profile_external_id(handle)
    }

    /// 返回当前 lane graph。
    pub const fn lane_graph(&self) -> &LaneGraph {
        &self.lane_graph
    }

    /// 返回 edge external ID 对应的 handle。
    pub fn edge_handle(&self, id: &str) -> Option<EdgeHandle> {
        self.lane_graph.edge_handle(id)
    }

    /// 返回 edge handle 对应的 external ID。
    pub fn edge_external_id(&self, handle: EdgeHandle) -> Option<&str> {
        self.lane_graph.edge_external_id(handle)
    }

    /// 返回 route external ID 对应的 handle。
    pub fn route_handle(&self, id: &str) -> Option<RouteHandle> {
        self.route_handles.get(id).copied()
    }

    /// 返回 route handle 对应的 external ID。
    pub fn route_external_id(&self, handle: RouteHandle) -> Option<&str> {
        self.route_slot(handle)
            .map(|route| route.external_id.as_str())
    }

    /// 返回 route 的 edge handle sequence。
    pub fn route_edges(&self, handle: RouteHandle) -> Option<&[EdgeHandle]> {
        self.route_slot(handle)
            .map(|route| route.edge_handles.as_slice())
    }

    /// 返回所有 active route handle，顺序与注册顺序一致。
    pub fn routes(&self) -> impl Iterator<Item = RouteHandle> + '_ {
        self.routes
            .iter()
            .enumerate()
            .filter(|(_, route)| route.active)
            .map(|(index, route)| RouteHandle::new(index, route.generation))
    }

    /// 注册新的 route definition。
    pub fn register_route(&mut self, route: Route) -> Result<RouteHandle, CoreError> {
        if self.route_handles.contains_key(route.id()) {
            return Err(CoreError::DuplicateRouteId {
                route_id: route.id().to_owned(),
            });
        }

        let edge_handles = resolve_route_edges(&self.lane_graph, &self.signals, &route)?;
        let edge_lengths = edge_handles
            .iter()
            .map(|edge| {
                self.lane_graph
                    .edge_length(*edge)
                    .expect("normalized route edge must exist")
                    .value()
            })
            .collect::<Vec<_>>();
        let (transitions, next_controlled_transition) =
            self.build_route_signal_metadata(&edge_handles, &edge_lengths);
        let distance_index = RouteDistanceIndex::build(&edge_lengths);
        let external_id = route.id().to_owned();

        let handle = if let Some(index) = self.free_route_indices.pop() {
            let generation = self.routes[index].generation;
            self.routes[index] = RouteSlot {
                generation,
                external_id: external_id.clone(),
                edge_handles,
                transitions,
                next_controlled_transition,
                active: true,
            };
            self.route_distance_indices[index] = distance_index;
            self.route_reference_indices[index] = RouteReferenceIndex::default();
            RouteHandle::new(index, generation)
        } else {
            let handle = RouteHandle::new(self.routes.len(), 0);
            self.routes.push(RouteSlot {
                generation: 0,
                external_id: external_id.clone(),
                edge_handles,
                transitions,
                next_controlled_transition,
                active: true,
            });
            self.route_distance_indices.push(distance_index);
            self.route_reference_indices
                .push(RouteReferenceIndex::default());
            handle
        };

        self.route_handles.insert(external_id, handle);
        Ok(handle)
    }

    fn build_route_signal_metadata(
        &self,
        edge_handles: &[EdgeHandle],
        edge_lengths: &[f32],
    ) -> (
        Vec<RouteTransition>,
        Vec<Option<NextControlledRouteTransition>>,
    ) {
        let transitions = edge_handles
            .windows(2)
            .map(|pair| {
                let to_edge = pair[1];
                let gate = self
                    .signals
                    .movement_gate_index(MovementGateKey::new(pair[0], to_edge));
                RouteTransition { to_edge, gate }
            })
            .collect::<Vec<_>>();
        let mut next_controlled_transition = vec![None; edge_handles.len()];
        let mut next = None;
        for route_edge_index in (0..edge_handles.len()).rev() {
            let edge_length = f64::from(edge_lengths[route_edge_index]);
            if let Some(gate) = transitions
                .get(route_edge_index)
                .and_then(|transition| transition.gate)
                .filter(|gate| self.signals.movement_gate_is_signal_controlled(*gate))
            {
                next = Some(NextControlledRouteTransition {
                    from_route_edge_index: route_edge_index,
                    gate,
                    distance_from_edge_start: BoundedDistance::Finite(edge_length),
                });
            } else if let Some(candidate) = next.as_mut() {
                candidate.distance_from_edge_start =
                    candidate.distance_from_edge_start.add(edge_length);
            }
            next_controlled_transition[route_edge_index] = next;
        }
        (transitions, next_controlled_transition)
    }

    /// 移除未被 live vehicle 引用的 route definition。
    pub fn remove_route(&mut self, handle: RouteHandle) -> Result<RouteRemoveRecord, CoreError> {
        self.route_slot(handle)
            .ok_or(CoreError::UnknownRouteHandle { route: handle })?;

        if self.route_reference_indices[handle.index()].live_count > 0 {
            let vehicle = self.first_route_reference(handle).or_else(|| {
                self.rebuild_route_reference_index(handle);
                self.first_route_reference(handle)
            });
            return Err(CoreError::RouteInUse {
                route: handle,
                vehicle: vehicle.expect("positive route reference count must have a live vehicle"),
            });
        }

        let reusable = self.routes[handle.index()].generation.checked_add(1);
        if reusable.is_some() {
            self.free_route_indices.reserve(1);
        }
        let route = &mut self.routes[handle.index()];
        let external_id = std::mem::take(&mut route.external_id);
        route.active = false;
        route.edge_handles.clear();
        route.transitions.clear();
        route.next_controlled_transition.clear();
        self.route_distance_indices[handle.index()].clear();
        self.route_reference_indices[handle.index()].clear();
        let removed = self.route_handles.swap_remove(&external_id);
        assert_eq!(
            removed,
            Some(handle),
            "route resolver must identify removed route"
        );
        // generation 耗尽时不复用 slot，避免 stale handle 在回绕后复活。
        if let Some(next_generation) = reusable {
            route.generation = next_generation;
            self.free_route_indices.push(handle.index());
        }

        Ok(RouteRemoveRecord {
            handle,
            external_id,
        })
    }

    /// 创建新的 vehicle runtime entity。
    pub fn spawn_vehicle(&mut self, input: VehicleSpawnInput) -> Result<VehicleHandle, CoreError> {
        self.spawn_vehicle_with_overlap_validation(input, true)
    }

    fn spawn_vehicle_without_overlap_validation(
        &mut self,
        input: VehicleSpawnInput,
    ) -> Result<VehicleHandle, CoreError> {
        self.spawn_vehicle_with_overlap_validation(input, false)
    }

    fn spawn_vehicle_with_overlap_validation(
        &mut self,
        input: VehicleSpawnInput,
        validate_overlap: bool,
    ) -> Result<VehicleHandle, CoreError> {
        validate_external_id("vehicles[].id", &input.id)?;
        validate_external_id("vehicles[].routeId", &input.route_id)?;
        if self.vehicle_handles.contains_key(&input.id) {
            return Err(CoreError::DuplicateVehicleId {
                vehicle_id: input.id,
            });
        }
        if self.vehicle_profile(input.profile).is_none() {
            return Err(CoreError::UnknownVehicleProfileHandle {
                vehicle_id: input.id,
                profile: input.profile,
            });
        }

        let route =
            self.route_handle(&input.route_id)
                .ok_or_else(|| CoreError::UnknownVehicleRoute {
                    vehicle_id: input.id.clone(),
                    route_id: input.route_id.clone(),
                })?;
        if input.status == VehicleStatus::Parked {
            return Err(CoreError::ParkedVehicleRequiresParkingCommand {
                vehicle_id: input.id,
            });
        }
        let normalized = self.normalize_vehicle_input(route, &input)?;
        if validate_overlap {
            self.validate_candidate_overlap(route, &input.id, &normalized)?;
        }

        let planned_slot_index = self
            .free_vehicle_indices
            .last()
            .copied()
            .unwrap_or(self.vehicles.len());
        let planned_generation = self
            .vehicles
            .get(planned_slot_index)
            .map_or(0, |slot| slot.generation);
        let handle = VehicleHandle::new(planned_slot_index, planned_generation);
        self.parking_runtime
            .prepare_vehicle_slot(planned_slot_index);
        let spatial_occupant = matches!(
            normalized.status,
            VehicleStatus::Active | VehicleStatus::Stopped
        )
        .then(|| {
            (
                self.routes[route.index()].edge_handles[normalized.route_edge_index],
                CommandOccupant {
                    vehicle: handle,
                    front_progress: normalized.edge_progress,
                },
            )
        });
        self.vehicle_handles.reserve(1);
        self.vehicle_update_order.reserve_for_append();
        if planned_slot_index == self.vehicles.len() {
            self.vehicles.reserve(1);
        }
        self.route_reference_indices[route.index()].reserve_for_attach();
        if let Some((edge, occupant)) = spatial_occupant {
            let vehicles = &self.vehicles;
            let mut resolve_progress = |handle: VehicleHandle| {
                vehicles[handle.index()]
                    .state
                    .as_ref()
                    .expect("command spatial occupant must be live")
                    .edge_progress
                    .value()
            };
            self.command_spatial_index
                .prepare_insert(edge, occupant, &mut resolve_progress);
        }
        self.candidate_state_scratch.reserve_for_slots(
            self.vehicles.len() + usize::from(planned_slot_index == self.vehicles.len()),
        );

        let external_id = input.id;
        let resolver_id = external_id.clone();
        let update_order_position = self.vehicle_update_order.append(handle);
        let slot = VehicleSlot {
            generation: planned_generation,
            external_id,
            state: Some(VehicleState::new(
                handle,
                normalized.profile,
                route,
                normalized.route_edge_index,
                normalized.edge_progress,
                normalized.current_speed,
                normalized.status,
            )),
            update_order_position: Some(update_order_position),
        };
        if planned_slot_index < self.vehicles.len() {
            let popped = self
                .free_vehicle_indices
                .pop()
                .expect("planned reusable vehicle slot must remain available");
            assert_eq!(popped, planned_slot_index);
            self.vehicles[planned_slot_index] = slot;
        } else {
            self.vehicles.push(slot);
        }

        self.vehicle_handles.insert(resolver_id, handle);
        self.parking_runtime.register_unbound_vehicle(handle);
        self.route_reference_indices[route.index()].attach(route, handle, update_order_position);
        if let Some((edge, occupant)) = spatial_occupant {
            let vehicles = &self.vehicles;
            let mut resolve_progress = |handle: VehicleHandle| {
                vehicles[handle.index()]
                    .state
                    .as_ref()
                    .expect("command spatial occupant must be live")
                    .edge_progress
                    .value()
            };
            self.command_spatial_index
                .insert(edge, occupant, &mut resolve_progress);
        }
        if normalized.status == VehicleStatus::Active {
            self.command_spatial_index
                .note_vehicle_speed(handle, normalized.current_speed.value());
        }
        self.compact_update_order_if_needed();
        Ok(handle)
    }

    /// 移除 live vehicle runtime entity。
    pub fn despawn_vehicle(
        &mut self,
        handle: VehicleHandle,
    ) -> Result<VehicleDespawnRecord, CoreError> {
        self.vehicle_slot(handle)
            .ok_or(CoreError::UnknownVehicleHandle { vehicle: handle })?;

        let slot = &self.vehicles[handle.index()];
        let update_order_position = slot
            .update_order_position
            .expect("live vehicle must have reverse update-order position");
        let state = slot
            .state
            .as_ref()
            .expect("validated vehicle slot must contain state");
        let profile = state.profile;
        let route = state.route;
        let status = state.status;
        let removed_speed = state.current_speed.value();
        let parking_binding = self.parking_runtime.vehicle_binding(handle);
        let parking_binding_valid = match (status, parking_binding) {
            (VehicleStatus::Parked, Some(RuntimeVehicleParkingBinding::Occupied { space, .. })) => {
                self.parking_runtime.space_state(space)
                    == Some(ParkingSpaceState::Occupied { vehicle: handle })
            }
            (VehicleStatus::Parked, _) => false,
            (VehicleStatus::Active, Some(RuntimeVehicleParkingBinding::Reserved { space, .. })) => {
                self.parking_runtime.space_state(space)
                    == Some(ParkingSpaceState::Reserved { vehicle: handle })
            }
            (_, Some(_)) => false,
            (_, None) => true,
        };
        if !parking_binding_valid {
            return Err(CoreError::ParkingBindingInvariantViolation {
                stage: "despawn",
                vehicle: Some(handle),
                space: parking_binding.map(RuntimeVehicleParkingBinding::space),
            });
        }
        let spatial_occupant = matches!(status, VehicleStatus::Active | VehicleStatus::Stopped)
            .then(|| {
                (
                    self.routes[route.index()].edge_handles[state.route_edge_index],
                    CommandOccupant {
                        vehicle: handle,
                        front_progress: state.edge_progress,
                    },
                )
            });
        let reusable = slot.generation.checked_add(1);
        if reusable.is_some() {
            self.free_vehicle_indices.reserve(1);
        }
        if let Some((edge, occupant)) = spatial_occupant {
            let mut spatial = std::mem::take(&mut self.command_spatial_index);
            let vehicles = &self.vehicles;
            if status == VehicleStatus::Active {
                spatial.prepare_speed_removal(
                    removed_speed,
                    vehicles.iter().filter_map(|slot| {
                        let state = slot.state.as_ref()?;
                        (state.status == VehicleStatus::Active)
                            .then_some((state.handle, state.current_speed.value()))
                    }),
                );
            }
            let mut resolve_progress = |candidate: VehicleHandle| {
                vehicles[candidate.index()]
                    .state
                    .as_ref()
                    .expect("command spatial occupant must be live")
                    .edge_progress
                    .value()
            };
            spatial.remove(edge, occupant, &mut resolve_progress);
            self.command_spatial_index = spatial;
        }

        let parking_release =
            self.parking_runtime
                .release(&self.parking, handle)
                .map(|(space, previous_binding)| ParkingReleaseRecord {
                    vehicle: handle,
                    space,
                    previous_binding,
                    reason: ParkingReleaseReason::VehicleDespawn,
                });
        let slot = &mut self.vehicles[handle.index()];
        slot.state
            .take()
            .expect("validated vehicle slot must contain state");
        let external_id = std::mem::take(&mut slot.external_id);
        slot.update_order_position = None;
        let removed = self.vehicle_handles.swap_remove(&external_id);
        assert_eq!(
            removed,
            Some(handle),
            "vehicle resolver must identify removed vehicle"
        );
        // generation 耗尽时不复用 slot，避免 stale handle 在回绕后复活。
        if let Some(next_generation) = reusable {
            slot.generation = next_generation;
            self.free_vehicle_indices.push(handle.index());
        }
        self.vehicle_update_order
            .tombstone(update_order_position, handle);
        self.route_reference_indices[route.index()].detach();
        self.compact_update_order_if_needed();

        Ok(VehicleDespawnRecord {
            handle,
            external_id,
            profile,
            route,
            status,
            parking_release,
        })
    }

    fn first_route_reference(&mut self, route: RouteHandle) -> Option<VehicleHandle> {
        self.route_reference_indices[route.index()].first_valid(route, &self.vehicles)
    }

    fn rebuild_route_reference_index(&mut self, route: RouteHandle) {
        let order = &self.vehicle_update_order;
        let vehicles = &self.vehicles;
        let index = &mut self.route_reference_indices[route.index()];
        index.clear();
        for (position, vehicle) in order
            .entries
            .iter()
            .enumerate()
            .filter_map(|(position, entry)| entry.map(|vehicle| (position, vehicle)))
        {
            let Some(state) = vehicles
                .get(vehicle.index())
                .filter(|slot| slot.generation == vehicle.generation())
                .and_then(|slot| slot.state.as_ref())
            else {
                continue;
            };
            if state.route == route {
                index.attach(route, vehicle, position);
            }
        }
    }

    fn rebuild_all_route_reference_indices(&mut self) {
        for index in &mut self.route_reference_indices {
            index.clear();
        }
        for (position, vehicle) in self
            .vehicle_update_order
            .entries
            .iter()
            .enumerate()
            .filter_map(|(position, entry)| entry.map(|vehicle| (position, vehicle)))
        {
            let state = self.vehicles[vehicle.index()]
                .state
                .as_ref()
                .expect("stable update order must identify live vehicle");
            self.route_reference_indices[state.route.index()].attach(
                state.route,
                vehicle,
                position,
            );
        }
    }

    fn compact_update_order_if_needed(&mut self) {
        if self.vehicle_update_order.compact(&mut self.vehicles) {
            self.rebuild_all_route_reference_indices();
        }
    }

    #[cfg(test)]
    fn assert_lifecycle_indices_consistent(&mut self) {
        let mut seen = vec![false; self.vehicles.len()];
        let mut expected_route_counts = vec![0_usize; self.routes.len()];
        let mut expected_route_first = vec![None; self.routes.len()];

        for (position, vehicle) in self
            .vehicle_update_order
            .entries
            .iter()
            .enumerate()
            .filter_map(|(position, entry)| entry.map(|vehicle| (position, vehicle)))
        {
            let slot = self
                .vehicles
                .get(vehicle.index())
                .filter(|slot| slot.generation == vehicle.generation())
                .expect("stable update order entry must resolve");
            assert!(
                !seen[vehicle.index()],
                "vehicle must occur once in update order"
            );
            seen[vehicle.index()] = true;
            assert_eq!(slot.update_order_position, Some(position));
            let state = slot.state.as_ref().expect("ordered vehicle must be live");
            expected_route_counts[state.route.index()] += 1;
            expected_route_first[state.route.index()].get_or_insert(vehicle);
        }

        for (index, slot) in self.vehicles.iter().enumerate() {
            assert_eq!(slot.state.is_some(), seen[index]);
            assert_eq!(slot.update_order_position.is_some(), seen[index]);
        }

        for route_index in 0..self.routes.len() {
            if !self.routes[route_index].active {
                continue;
            }
            let route = RouteHandle::new(route_index, self.routes[route_index].generation);
            assert_eq!(
                self.route_reference_indices[route_index].live_count,
                expected_route_counts[route_index]
            );
            let actual_first =
                self.route_reference_indices[route_index].first_valid(route, &self.vehicles);
            assert_eq!(actual_first, expected_route_first[route_index]);
        }

        let mut expected_spatial = self
            .vehicles()
            .filter(|vehicle| {
                matches!(
                    vehicle.status,
                    VehicleStatus::Active | VehicleStatus::Stopped
                )
            })
            .map(|vehicle| {
                (
                    self.vehicle_edge(vehicle),
                    CommandOccupant {
                        vehicle: vehicle.handle,
                        front_progress: vehicle.edge_progress,
                    },
                )
            })
            .collect::<Vec<_>>();
        let mut actual_spatial = self.command_spatial_index.occupants().collect::<Vec<_>>();
        let compare = |left: &(EdgeHandle, CommandOccupant),
                       right: &(EdgeHandle, CommandOccupant)| {
            left.0
                .index()
                .cmp(&right.0.index())
                .then_with(|| {
                    left.1
                        .front_progress
                        .value()
                        .total_cmp(&right.1.front_progress.value())
                })
                .then_with(|| left.1.vehicle.index().cmp(&right.1.vehicle.index()))
                .then_with(|| {
                    left.1
                        .vehicle
                        .generation()
                        .cmp(&right.1.vehicle.generation())
                })
        };
        expected_spatial.sort_unstable_by(compare);
        actual_spatial.sort_unstable_by(compare);
        assert_eq!(actual_spatial, expected_spatial);
        self.parking_runtime
            .assert_consistent(&self.parking, |vehicle| {
                self.vehicle(vehicle).map(|state| state.status)
            });
    }

    #[cfg(test)]
    fn lifecycle_retained_stats(&self) -> LifecycleRetainedStats {
        let route_occurrences = self
            .routes
            .iter()
            .filter(|route| route.active)
            .map(|route| route.edge_handles.len())
            .sum();
        let route_candidate_nodes = self
            .route_reference_indices
            .iter()
            .map(|index| index.candidates.len())
            .sum();
        let stale_route_candidate_nodes = self
            .route_reference_indices
            .iter()
            .map(|index| index.candidates.len().saturating_sub(index.live_count))
            .sum();
        let route_distance_bytes = self.route_distance_indices.capacity()
            * std::mem::size_of::<RouteDistanceIndex>()
            + self
                .route_distance_indices
                .iter()
                .map(RouteDistanceIndex::retained_bytes)
                .sum::<usize>();
        let route_reference_bytes = self.route_reference_indices.capacity()
            * std::mem::size_of::<RouteReferenceIndex>()
            + self
                .route_reference_indices
                .iter()
                .map(|index| {
                    index.candidates.capacity()
                        * std::mem::size_of::<Reverse<RouteVehicleReference>>()
                })
                .sum::<usize>();
        let route_bytes = self.routes.capacity() * std::mem::size_of::<RouteSlot>()
            + self
                .routes
                .iter()
                .map(|route| {
                    route.external_id.capacity()
                        + route.edge_handles.capacity() * std::mem::size_of::<EdgeHandle>()
                        + route.transitions.capacity() * std::mem::size_of::<RouteTransition>()
                        + route.next_controlled_transition.capacity()
                            * std::mem::size_of::<Option<NextControlledRouteTransition>>()
                })
                .sum::<usize>()
            + route_distance_bytes
            + route_reference_bytes;
        let vehicle_bytes = self.vehicles.capacity() * std::mem::size_of::<VehicleSlot>()
            + self
                .vehicles
                .iter()
                .map(|vehicle| vehicle.external_id.capacity())
                .sum::<usize>();
        let resolver_bytes = self.route_handles.capacity()
            * std::mem::size_of::<(String, RouteHandle)>()
            + self
                .route_handles
                .keys()
                .map(String::capacity)
                .sum::<usize>()
            + self.vehicle_handles.capacity() * std::mem::size_of::<(String, VehicleHandle)>()
            + self
                .vehicle_handles
                .keys()
                .map(String::capacity)
                .sum::<usize>();
        let free_list_bytes = self.free_route_indices.capacity() * std::mem::size_of::<usize>()
            + self.free_vehicle_indices.capacity() * std::mem::size_of::<usize>();
        let vehicle_order_bytes = self.vehicle_update_order.entries.capacity()
            * std::mem::size_of::<Option<VehicleHandle>>();
        let candidate_state_bytes = self.candidate_state_scratch.states.capacity()
            * std::mem::size_of::<Option<VehicleState>>()
            + self.candidate_state_scratch.spatial_changes.capacity()
                * std::mem::size_of::<VehicleHandle>()
            + self.candidate_state_scratch.parking_releases.capacity()
                * std::mem::size_of::<ParkingStepRelease>();
        let lifecycle_scratch_bytes = free_list_bytes + vehicle_order_bytes + candidate_state_bytes;
        let parking_registry_runtime_bytes =
            self.parking.retained_bytes() + self.parking_runtime.retained_bytes();
        let parking_bytes =
            parking_registry_runtime_bytes + self.longitudinal_scratch.parking_retained_bytes();
        let world_inline_bytes = std::mem::size_of::<Self>();
        let lane_graph_bytes = self.lane_graph.retained_bytes();
        let vehicle_profile_registry_bytes = self.vehicle_profiles.retained_bytes();
        let signal_registry_bytes = self.signals.retained_bytes();
        let signal_runtime_state_bytes = self.signal_state.retained_bytes();
        let signal_runtime_scratch_bytes = self.signal_candidate_scratch.retained_bytes();
        let occupancy_scratch_bytes = self.occupancy_scratch.retained_bytes();
        let longitudinal_scratch_bytes = self.longitudinal_scratch.retained_bytes();
        let command_spatial_bytes = self.command_spatial_index.retained_bytes();
        let accounted_bytes = world_inline_bytes
            + route_bytes
            + vehicle_bytes
            + resolver_bytes
            + lifecycle_scratch_bytes
            + parking_bytes
            + command_spatial_bytes;
        let expanded_accounted_bytes = world_inline_bytes
            + route_bytes
            + vehicle_bytes
            + resolver_bytes
            + lifecycle_scratch_bytes
            + parking_registry_runtime_bytes
            + occupancy_scratch_bytes
            + longitudinal_scratch_bytes
            + command_spatial_bytes;
        let complete_accounted_bytes = expanded_accounted_bytes
            + lane_graph_bytes
            + vehicle_profile_registry_bytes
            + signal_registry_bytes
            + signal_runtime_state_bytes
            + signal_runtime_scratch_bytes;
        let owned_heap_bytes = complete_accounted_bytes - world_inline_bytes;
        LifecycleRetainedStats {
            accounted_bytes,
            expanded_accounted_bytes,
            complete_accounted_bytes,
            owned_heap_bytes,
            world_inline_bytes,
            lane_graph_bytes,
            vehicle_profile_registry_bytes,
            signal_registry_bytes,
            signal_runtime_state_bytes,
            signal_runtime_scratch_bytes,
            route_bytes,
            route_distance_bytes,
            route_reference_bytes,
            vehicle_bytes,
            resolver_bytes,
            free_list_bytes,
            vehicle_order_bytes,
            candidate_state_bytes,
            parking_bytes,
            parking_registry_runtime_bytes,
            occupancy_scratch_bytes,
            longitudinal_scratch_bytes,
            command_spatial_bytes,
            lane_graph_inline_size: std::mem::size_of::<LaneGraph>(),
            vehicle_profile_registry_inline_size: std::mem::size_of::<VehicleProfileRegistry>(),
            signal_registry_inline_size: std::mem::size_of::<SignalRegistry>(),
            signal_runtime_state_inline_size: std::mem::size_of::<SignalRuntimeState>(),
            signal_runtime_scratch_inline_size: std::mem::size_of::<SignalRuntimeScratch>(),
            vehicle_state_size: std::mem::size_of::<VehicleState>(),
            vehicle_slot_size: std::mem::size_of::<VehicleSlot>(),
            live_vehicles: self.vehicles().count(),
            route_occurrences,
            tombstones: self.vehicle_update_order.tombstones,
            route_candidate_nodes,
            stale_route_candidate_nodes,
            spatial_occupants: self.command_spatial_index.occupant_count(),
        }
    }

    /// 推进一个 fixed-step tick。
    ///
    /// 成功时，`StepResult` 使用 post-step tick/time。失败时权威 tick/time、vehicle state
    /// 与 events 保持不变；私有派生 scratch 可以重建，且不参与 `CoreWorld` 语义相等。
    pub fn step(&mut self, input: TickInput) -> Result<StepResult, CoreError> {
        if input.delta_time_ms != self.fixed_delta_time_ms {
            return Err(CoreError::TickDeltaMismatch {
                expected_delta_time_ms: self.fixed_delta_time_ms,
                actual_delta_time_ms: input.delta_time_ms,
            });
        }

        let next_tick_index = self
            .tick_index
            .checked_add(1)
            .ok_or(CoreError::TimeOverflow)?;
        let next_time_ms = self
            .time_ms
            .checked_add(self.fixed_delta_time_ms)
            .ok_or(CoreError::TimeOverflow)?;

        self.parking_runtime.validate_step_sentinel(&self.parking)?;
        // `ParkingVehicleCapabilityUnavailable` 是 #108 过渡期保留的 public variant。
        // #109 的完整 ParkingStop/arrival/release pipeline 激活后，合法 world 不再返回它。

        let mut signal_candidate_scratch = std::mem::take(&mut self.signal_candidate_scratch);
        self.signals
            .populate_runtime_state(next_time_ms, signal_candidate_scratch.state_mut());

        if let Err(error) = self.rebuild_occupancy_and_leaders() {
            self.signal_candidate_scratch = signal_candidate_scratch;
            return Err(error);
        }
        if let Err(error) = self.rebuild_longitudinal_motions() {
            self.signal_candidate_scratch = signal_candidate_scratch;
            return Err(error);
        }

        let mut candidate_states = std::mem::take(&mut self.candidate_state_scratch);
        candidate_states.begin(&self.vehicles);
        let mut events = Vec::new();
        let advance_context = VehicleAdvanceContext {
            lane_graph: &self.lane_graph,
            signals: &self.signals,
            signal_state: &self.signal_state,
            routes: &self.routes,
            fixed_delta_time_ms: self.fixed_delta_time_ms,
            tick_index: next_tick_index,
        };
        let mut candidate_max_vehicle_speed = 0.0_f32;
        let advance_result = (|| {
            let CandidateStateScratch {
                states,
                spatial_changes,
                parking_releases,
            } = &mut candidate_states;
            let has_reserved_parking = self.parking_runtime.reserved_count() > 0;
            if !has_reserved_parking {
                for vehicle_handle in self.vehicle_update_order.iter() {
                    let Some(current_slot) = self
                        .vehicles
                        .get(vehicle_handle.index())
                        .filter(|slot| slot.generation == vehicle_handle.generation())
                    else {
                        continue;
                    };
                    if current_slot.state.is_none() {
                        continue;
                    }
                    let Some(motion) = self.longitudinal_scratch.motion(vehicle_handle) else {
                        debug_assert!(matches!(
                            current_slot.state.as_ref().map(|state| state.status),
                            Some(VehicleStatus::Completed | VehicleStatus::Parked)
                        ));
                        continue;
                    };
                    let Some(vehicle) = states
                        .get_mut(vehicle_handle.index())
                        .and_then(Option::as_mut)
                    else {
                        continue;
                    };

                    if let Some(completed_event) = Self::advance_vehicle::<false>(
                        advance_context,
                        vehicle,
                        motion,
                        None,
                        &mut events,
                        spatial_changes,
                    )? {
                        events.push(CoreEvent::VehicleCompletedRoute(completed_event));
                    }
                    #[cfg(test)]
                    if self.step_failure_after_vehicle == Some(vehicle_handle) {
                        return Err(CoreError::ParkingBindingInvariantViolation {
                            stage: "test_after_vehicle_advance",
                            vehicle: Some(vehicle_handle),
                            space: None,
                        });
                    }
                    if vehicle.status == VehicleStatus::Active {
                        candidate_max_vehicle_speed =
                            candidate_max_vehicle_speed.max(vehicle.current_speed.value());
                    }
                }
                return Ok(());
            }
            let mut parking_stops = self
                .longitudinal_scratch
                .parking_stops()
                .iter()
                .copied()
                .peekable();
            for vehicle_handle in self.vehicle_update_order.iter() {
                let Some(current_slot) = self
                    .vehicles
                    .get(vehicle_handle.index())
                    .filter(|slot| slot.generation == vehicle_handle.generation())
                else {
                    continue;
                };
                if current_slot.state.is_none() {
                    continue;
                }
                let Some(motion) = self.longitudinal_scratch.motion(vehicle_handle) else {
                    debug_assert!(matches!(
                        current_slot.state.as_ref().map(|state| state.status),
                        Some(VehicleStatus::Completed | VehicleStatus::Parked)
                    ));
                    continue;
                };
                let parking_stop = parking_stops
                    .peek()
                    .filter(|stop| stop.vehicle == vehicle_handle)
                    .map(|stop| stop.constraint);
                if parking_stop.is_some() {
                    parking_stops.next();
                }
                let parking_binding = self.parking_runtime.vehicle_binding(vehicle_handle);
                let reaches_parking_stop =
                    parking_stop.is_some_and(|constraint| motion.reaches_parking_stop(constraint));
                let (reserved_space, reserved_target, entry_progress, was_arrived) =
                    match parking_binding {
                        Some(RuntimeVehicleParkingBinding::Reserved { space, target, .. }) => {
                            let entry_progress = reaches_parking_stop.then(|| {
                                self.parking
                                    .space_entry(space)
                                    .expect("reserved ParkingSpace must have entry")
                                    .progress()
                            });
                            let was_arrived = current_slot.state.as_ref().is_some_and(|state| {
                                parking_arrived_state(state, target, entry_progress)
                            });
                            (Some(space), target, entry_progress, was_arrived)
                        }
                        Some(RuntimeVehicleParkingBinding::Occupied { .. }) | None => {
                            (None, None, None, false)
                        }
                    };

                let Some(vehicle) = states
                    .get_mut(vehicle_handle.index())
                    .and_then(Option::as_mut)
                else {
                    continue;
                };

                let completed_event = Self::advance_vehicle::<true>(
                    advance_context,
                    vehicle,
                    motion,
                    parking_stop,
                    &mut events,
                    spatial_changes,
                )?;
                if let Some(space) = reserved_space {
                    if let Some(completed_event) = completed_event {
                        if reserved_target.is_some() {
                            return Err(CoreError::ParkingBindingInvariantViolation {
                                stage: "step_reachable_target_completed",
                                vehicle: Some(vehicle_handle),
                                space: Some(space),
                            });
                        }
                        parking_releases.push(ParkingStepRelease {
                            vehicle: vehicle_handle,
                            space,
                        });
                        events.push(CoreEvent::ParkingReservationReleased(
                            ParkingReservationReleasedEvent {
                                tick_index: next_tick_index,
                                vehicle: vehicle_handle,
                                space,
                                reason: ParkingReleaseReason::RouteCompleted,
                            },
                        ));
                        events.push(CoreEvent::VehicleCompletedRoute(completed_event));
                    } else if reaches_parking_stop
                        && !was_arrived
                        && parking_arrived_state(vehicle, reserved_target, entry_progress)
                    {
                        let target = reserved_target
                            .expect("arrived reservation must have an approach target");
                        events.push(CoreEvent::VehicleParkingArrivalReached(
                            VehicleParkingArrivalReachedEvent {
                                tick_index: next_tick_index,
                                vehicle: vehicle_handle,
                                space,
                                route: target.route,
                                route_edge_index: target.route_edge_index,
                            },
                        ));
                    }
                } else if let Some(completed_event) = completed_event {
                    events.push(CoreEvent::VehicleCompletedRoute(completed_event));
                }
                #[cfg(test)]
                if self.step_failure_after_vehicle == Some(vehicle_handle) {
                    return Err(CoreError::ParkingBindingInvariantViolation {
                        stage: "test_after_vehicle_advance",
                        vehicle: Some(vehicle_handle),
                        space: reserved_space,
                    });
                }
                if vehicle.status == VehicleStatus::Active {
                    candidate_max_vehicle_speed =
                        candidate_max_vehicle_speed.max(vehicle.current_speed.value());
                }
            }
            debug_assert!(parking_stops.next().is_none());
            Ok(())
        })();

        if let Err(error) = advance_result {
            candidate_states.clear();
            self.candidate_state_scratch = candidate_states;
            self.signal_candidate_scratch = signal_candidate_scratch;
            return Err(error);
        }

        let invalid_release = candidate_states
            .parking_releases
            .iter()
            .copied()
            .find(|release| {
                !self
                    .parking_runtime
                    .validate_reserved_pair(release.vehicle, release.space)
            });
        if let Some(release) = invalid_release {
            candidate_states.clear();
            self.candidate_state_scratch = candidate_states;
            self.signal_candidate_scratch = signal_candidate_scratch;
            return Err(CoreError::ParkingBindingInvariantViolation {
                stage: "step_completion_release_validate",
                vehicle: Some(release.vehicle),
                space: Some(release.space),
            });
        }

        self.append_signal_events(
            next_tick_index,
            signal_candidate_scratch.state(),
            &mut events,
        );
        self.sync_changed_command_spatial_memberships(&candidate_states);
        self.command_spatial_index
            .set_max_vehicle_speed(candidate_max_vehicle_speed);
        for release in &candidate_states.parking_releases {
            let applied = self.parking_runtime.release(&self.parking, release.vehicle);
            assert_eq!(
                applied,
                Some((release.space, ParkingBindingKind::Reserved)),
                "validated completion release must commit exact Reserved pair"
            );
        }
        candidate_states.commit_into(&mut self.vehicles);
        std::mem::swap(&mut self.signal_state, signal_candidate_scratch.state_mut());
        self.tick_index = next_tick_index;
        self.time_ms = next_time_ms;
        self.candidate_state_scratch = candidate_states;
        self.signal_candidate_scratch = signal_candidate_scratch;

        Ok(StepResult {
            tick_index: next_tick_index,
            time_ms: next_time_ms,
            events,
        })
    }

    fn rebuild_command_spatial_index(&mut self) {
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
                    front_progress: vehicle.edge_progress,
                },
            );
        }
        spatial.finish_rebuild();
        self.command_spatial_index = spatial;
    }

    fn sync_changed_command_spatial_memberships(&mut self, candidate: &CandidateStateScratch) {
        let routes = &self.routes;
        let vehicles = &self.vehicles;
        let spatial = &mut self.command_spatial_index;
        let membership = |state: &VehicleState| {
            matches!(state.status, VehicleStatus::Active | VehicleStatus::Stopped).then(|| {
                (
                    routes[state.route.index()].edge_handles[state.route_edge_index],
                    CommandOccupant {
                        vehicle: state.handle,
                        front_progress: state.edge_progress,
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

    fn append_signal_events(
        &self,
        tick_index: u64,
        candidate: &SignalRuntimeState,
        events: &mut Vec<CoreEvent>,
    ) {
        for (controller, next_controller_state) in candidate.controller_states() {
            let current_controller_state = self
                .signal_state
                .controller_state(controller)
                .expect("committed controller state must exist");
            let from_phase = current_controller_state.current_phase();
            let to_phase = next_controller_state.current_phase();
            if from_phase != to_phase {
                events.push(CoreEvent::SignalPhaseChanged(SignalPhaseChangedEvent {
                    tick_index,
                    controller,
                    from_phase,
                    to_phase,
                }));
            }

            for group in self
                .signals
                .controller_groups(controller)
                .expect("normalized controller groups must exist")
            {
                let from_aspect = self
                    .signal_state
                    .group_state(*group)
                    .expect("committed group state must exist")
                    .aspect();
                let to_aspect = candidate
                    .group_state(*group)
                    .expect("candidate group state must exist")
                    .aspect();
                if from_aspect != to_aspect {
                    events.push(CoreEvent::SignalGroupAspectChanged(
                        SignalGroupAspectChangedEvent {
                            tick_index,
                            group: *group,
                            from_aspect,
                            to_aspect,
                        },
                    ));
                }
            }
        }
    }

    fn validate_candidate_overlap(
        &mut self,
        route: RouteHandle,
        candidate_id: &str,
        candidate: &NormalizedVehicleInput,
    ) -> Result<(), CoreError> {
        if matches!(
            candidate.status,
            VehicleStatus::Completed | VehicleStatus::Parked
        ) {
            return Ok(());
        }

        let candidate_length = self
            .vehicle_profile(candidate.profile)
            .expect("candidate profile must exist")
            .iidm()
            .length;
        let mut spatial = std::mem::take(&mut self.command_spatial_index);
        let route_edges = &self
            .route_slot(route)
            .expect("candidate route must exist")
            .edge_handles;
        let mut resolve_progress = |handle| {
            self.vehicle(handle)
                .expect("command spatial occupant must be live")
                .edge_progress
                .value()
        };
        spatial.gather_overlap_candidates(
            route_edges,
            candidate.route_edge_index,
            candidate.edge_progress.value(),
            candidate_length,
            self.vehicles.len(),
            &mut resolve_progress,
        );
        spatial.sort_candidates_by_key(|handle| {
            self.vehicles[handle.index()]
                .update_order_position
                .expect("command candidate must be live")
        });
        let result = self.validate_candidate_overlap_for_handles(
            route,
            candidate_id,
            candidate,
            spatial.candidates().iter().copied(),
        );
        self.command_spatial_index = spatial;
        result
    }

    fn validate_candidate_overlap_excluding(
        &mut self,
        excluded: VehicleHandle,
        route: RouteHandle,
        candidate: &NormalizedVehicleInput,
    ) -> Result<(), CoreError> {
        let candidate_id = self
            .vehicle_external_id(excluded)
            .expect("excluded vehicle must be live")
            .to_owned();
        let candidate_length = self
            .vehicle_profile(candidate.profile)
            .expect("candidate profile must exist")
            .iidm()
            .length;
        let mut spatial = std::mem::take(&mut self.command_spatial_index);
        let route_edges = &self
            .route_slot(route)
            .expect("candidate route must exist")
            .edge_handles;
        let mut resolve_progress = |handle| {
            self.vehicle(handle)
                .expect("command spatial occupant must be live")
                .edge_progress
                .value()
        };
        spatial.gather_overlap_candidates(
            route_edges,
            candidate.route_edge_index,
            candidate.edge_progress.value(),
            candidate_length,
            self.vehicles.len(),
            &mut resolve_progress,
        );
        spatial.sort_candidates_by_key(|handle| {
            self.vehicles[handle.index()]
                .update_order_position
                .expect("command candidate must be live")
        });
        let result = self.validate_candidate_overlap_for_handles(
            route,
            &candidate_id,
            candidate,
            spatial
                .candidates()
                .iter()
                .copied()
                .filter(|handle| *handle != excluded),
        );
        self.command_spatial_index = spatial;
        result
    }

    fn validate_parking_leave_followers(
        &mut self,
        leaving_vehicle: VehicleHandle,
        space: crate::ParkingSpaceHandle,
        route: RouteHandle,
        route_edge_index: usize,
        candidate: &NormalizedVehicleInput,
    ) -> Result<(), CoreError> {
        let candidate_edge = self.routes[route.index()].edge_handles[route_edge_index];
        let candidate_length = self
            .vehicle_profile(candidate.profile)
            .expect("candidate profile must exist")
            .iidm()
            .length;
        let emergency_horizon = parking_emergency_travel(
            "leave_global_emergency_horizon",
            leaving_vehicle,
            space,
            self.command_spatial_index.max_vehicle_speed(),
            self.command_spatial_index.min_emergency_deceleration(),
            self.fixed_delta_time_ms as f32 / 1_000.0,
        )?;
        let candidate_length_f64 = f64::from(candidate_length);
        let reverse_horizon = if candidate_length_f64 > f64::MAX - emergency_horizon {
            f64::MAX
        } else {
            candidate_length_f64 + emergency_horizon
        };
        let mut spatial = std::mem::take(&mut self.command_spatial_index);
        let mut resolve_progress = |handle| {
            self.vehicle(handle)
                .expect("command spatial occupant must be live")
                .edge_progress
                .value()
        };
        spatial.gather_direct_follower_candidates(
            candidate_edge,
            candidate.edge_progress.value(),
            reverse_horizon,
            self.vehicles.len(),
            &mut resolve_progress,
        );
        spatial.sort_candidates_by_key(|handle| {
            self.vehicles[handle.index()]
                .update_order_position
                .expect("command candidate must be live")
        });
        let result = self.validate_parking_leave_followers_for_handles(
            leaving_vehicle,
            space,
            candidate_edge,
            candidate.edge_progress.value(),
            candidate_length_f64,
            spatial.candidates(),
        );
        self.command_spatial_index = spatial;
        result
    }

    fn validate_parking_leave_followers_for_handles(
        &self,
        leaving_vehicle: VehicleHandle,
        space: crate::ParkingSpaceHandle,
        candidate_edge: EdgeHandle,
        candidate_progress: f64,
        candidate_length: f64,
        handles: &[VehicleHandle],
    ) -> Result<(), CoreError> {
        for follower_handle in handles.iter().copied() {
            if follower_handle == leaving_vehicle {
                continue;
            }
            let follower = self
                .vehicle(follower_handle)
                .expect("leave follower candidate must be live");
            if follower.status != VehicleStatus::Active {
                continue;
            }
            let follower_profile = self
                .vehicle_profile(follower.profile)
                .expect("live follower profile must exist")
                .iidm();
            let emergency_travel = parking_emergency_travel(
                "leave_follower_emergency_travel",
                follower_handle,
                space,
                follower.current_speed.value(),
                follower_profile.emergency_deceleration,
                self.fixed_delta_time_ms as f32 / 1_000.0,
            )?;
            let follower_horizon = if candidate_length > f64::MAX - emergency_travel {
                f64::MAX
            } else {
                candidate_length + emergency_travel
            };
            let Some(candidate_front_distance) = self.route_front_distance_within(
                follower.route,
                follower.route_edge_index,
                follower.edge_progress.value(),
                candidate_edge,
                candidate_progress,
                follower_horizon,
            ) else {
                continue;
            };

            let has_intervening_leader = handles.iter().copied().any(|other_handle| {
                if other_handle == follower_handle || other_handle == leaving_vehicle {
                    return false;
                }
                let Some(other) = self.vehicle(other_handle) else {
                    return false;
                };
                if !matches!(other.status, VehicleStatus::Active | VehicleStatus::Stopped) {
                    return false;
                }
                let other_edge = self.vehicle_edge(other);
                self.route_front_distance_within(
                    follower.route,
                    follower.route_edge_index,
                    follower.edge_progress.value(),
                    other_edge,
                    other.edge_progress.value(),
                    candidate_front_distance,
                )
                .is_some_and(|distance| {
                    distance + LONGITUDINAL_CONSTRAINT_TOLERANCE_METERS < candidate_front_distance
                })
            });
            if has_intervening_leader {
                continue;
            }

            let bumper_gap = candidate_front_distance - candidate_length;
            if bumper_gap + PHYSICAL_GAP_TOLERANCE_METERS < emergency_travel {
                return Err(CoreError::ParkingLeaveUnsafeFollower {
                    vehicle: leaving_vehicle,
                    space,
                    follower: follower_handle,
                });
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn validate_parking_leave_followers_full_scan(
        &self,
        leaving_vehicle: VehicleHandle,
        space: crate::ParkingSpaceHandle,
        route: RouteHandle,
        route_edge_index: usize,
        candidate: &NormalizedVehicleInput,
    ) -> Result<(), CoreError> {
        let candidate_edge = self.routes[route.index()].edge_handles[route_edge_index];
        let candidate_length = self
            .vehicle_profile(candidate.profile)
            .expect("candidate profile must exist")
            .iidm()
            .length;
        let handles = self.vehicle_update_order.iter().collect::<Vec<_>>();
        self.validate_parking_leave_followers_for_handles(
            leaving_vehicle,
            space,
            candidate_edge,
            candidate.edge_progress.value(),
            f64::from(candidate_length),
            &handles,
        )
    }

    #[cfg(test)]
    fn validate_candidate_overlap_full_scan(
        &self,
        route: RouteHandle,
        candidate_id: &str,
        candidate: &NormalizedVehicleInput,
    ) -> Result<(), CoreError> {
        self.validate_candidate_overlap_for_handles(
            route,
            candidate_id,
            candidate,
            self.vehicle_update_order.iter(),
        )
    }

    fn validate_candidate_overlap_for_handles<I>(
        &self,
        route: RouteHandle,
        candidate_id: &str,
        candidate: &NormalizedVehicleInput,
        existing_handles: I,
    ) -> Result<(), CoreError>
    where
        I: IntoIterator<Item = VehicleHandle>,
    {
        if matches!(
            candidate.status,
            VehicleStatus::Completed | VehicleStatus::Parked
        ) {
            return Ok(());
        }

        let candidate_edge = self
            .route_slot(route)
            .expect("candidate route must exist")
            .edge_handles[candidate.route_edge_index];
        let candidate_length = self
            .vehicle_profile(candidate.profile)
            .expect("candidate profile must exist")
            .iidm()
            .length;

        for handle in existing_handles {
            let existing = self
                .vehicle(handle)
                .expect("command spatial candidate must be live");
            if matches!(
                existing.status,
                VehicleStatus::Completed | VehicleStatus::Parked
            ) {
                continue;
            }
            let existing_edge = self.vehicle_edge(existing);
            let existing_length = self
                .vehicle_profile(existing.profile)
                .expect("existing profile must exist")
                .iidm()
                .length;
            let existing_id = self
                .vehicle_external_id(existing.handle)
                .expect("existing vehicle ID must exist");

            if let Some(front_distance) = self.route_front_distance_within(
                route,
                candidate.route_edge_index,
                candidate.edge_progress.value(),
                existing_edge,
                existing.edge_progress.value(),
                f64::from(existing_length),
            ) {
                let bumper_gap = front_distance - f64::from(existing_length);
                if physical_gap_is_overlap(bumper_gap) {
                    return Err(CoreError::VehiclePhysicalOverlap {
                        follower_id: candidate_id.to_owned(),
                        leader_id: existing_id.to_owned(),
                        bumper_gap,
                    });
                }
            }

            if let Some(front_distance) = self.route_front_distance_within(
                existing.route,
                existing.route_edge_index,
                existing.edge_progress.value(),
                candidate_edge,
                candidate.edge_progress.value(),
                f64::from(candidate_length),
            ) {
                let bumper_gap = front_distance - f64::from(candidate_length);
                if physical_gap_is_overlap(bumper_gap) {
                    return Err(CoreError::VehiclePhysicalOverlap {
                        follower_id: existing_id.to_owned(),
                        leader_id: candidate_id.to_owned(),
                        bumper_gap,
                    });
                }
            }
        }

        Ok(())
    }

    fn route_front_distance_within(
        &self,
        route: RouteHandle,
        route_edge_index: usize,
        front_progress: f64,
        target_edge: EdgeHandle,
        target_front_progress: f64,
        max_front_distance: f64,
    ) -> Option<f64> {
        let route_handle = route;
        let route = self.route_slot(route_handle).expect("route must exist");
        let current_edge = route.edge_handles[route_edge_index];
        let target_occurrence =
            if current_edge == target_edge && target_front_progress >= front_progress {
                route_edge_index
            } else {
                route
                    .edge_handles
                    .iter()
                    .copied()
                    .enumerate()
                    .skip(route_edge_index + 1)
                    .find_map(|(index, edge)| (edge == target_edge).then_some(index))?
            };

        match self.route_distance_indices[route_handle.index()].distance_within(
            route_edge_index,
            front_progress,
            target_occurrence,
            target_front_progress,
            max_front_distance,
        ) {
            RouteDistanceQuery::Within(distance) => Some(distance),
            RouteDistanceQuery::Passed | RouteDistanceQuery::BeyondHorizon => None,
        }
    }

    fn validate_initial_vehicle_overlaps(&mut self) -> Result<(), CoreError> {
        let mut scratch = std::mem::take(&mut self.occupancy_scratch);
        self.build_occupancy(&mut scratch);
        let result = self.validate_occupancy_overlaps(&scratch);
        self.occupancy_scratch = scratch;
        result
    }

    fn validate_occupancy_overlaps(&self, scratch: &OccupancyScratch) -> Result<(), CoreError> {
        for edge_index in 0..self.lane_graph.edges().len() {
            for pair in scratch.edge(EdgeHandle::new(edge_index)).windows(2) {
                let follower = pair[0];
                let leader = pair[1];
                let bumper_gap = leader.front_progress.value()
                    - follower.front_progress.value()
                    - f64::from(leader.vehicle_length);
                if physical_gap_is_overlap(bumper_gap) {
                    return Err(self.vehicle_overlap_error(
                        follower.vehicle,
                        leader.vehicle,
                        bumper_gap,
                    ));
                }
            }
        }

        for handle in self.vehicle_update_order.iter() {
            let Some(vehicle) = self.vehicle(handle) else {
                continue;
            };
            if !matches!(
                vehicle.status,
                VehicleStatus::Active | VehicleStatus::Stopped
            ) {
                continue;
            }

            if let Some(observation) = self.find_leader(scratch, vehicle, 0.0)?
                && physical_gap_is_overlap(f64::from(observation.bumper_gap))
            {
                return Err(self.vehicle_overlap_error(
                    handle,
                    observation.leader,
                    f64::from(observation.bumper_gap),
                ));
            }
        }

        Ok(())
    }

    fn vehicle_overlap_error(
        &self,
        follower: VehicleHandle,
        leader: VehicleHandle,
        bumper_gap: f64,
    ) -> CoreError {
        CoreError::VehiclePhysicalOverlap {
            follower_id: self
                .vehicle_external_id(follower)
                .expect("occupant vehicle ID must exist")
                .to_owned(),
            leader_id: self
                .vehicle_external_id(leader)
                .expect("occupant vehicle ID must exist")
                .to_owned(),
            bumper_gap,
        }
    }

    fn rebuild_occupancy_and_leaders(&mut self) -> Result<(), CoreError> {
        let mut scratch = std::mem::take(&mut self.occupancy_scratch);
        let result = (|| {
            self.build_occupancy(&mut scratch);

            if scratch.occupant_count() <= 1 {
                return Ok(());
            }

            for handle in self.vehicle_update_order.iter() {
                let Some(vehicle) = self.vehicle(handle) else {
                    continue;
                };
                if !matches!(
                    vehicle.status,
                    VehicleStatus::Active | VehicleStatus::Stopped
                ) {
                    continue;
                }

                let horizon = self.leader_horizon(vehicle)?;
                let leader = self.find_leader(&scratch, vehicle, horizon)?;
                scratch.set_leader(handle, leader);
            }

            Ok(())
        })();
        self.occupancy_scratch = scratch;
        result
    }

    fn rebuild_longitudinal_motions(&mut self) -> Result<(), CoreError> {
        if self.parking_runtime.reserved_count() == 0 {
            self.rebuild_longitudinal_motions_for_parking::<false>()
        } else {
            self.rebuild_longitudinal_motions_for_parking::<true>()
        }
    }

    fn rebuild_longitudinal_motions_for_parking<const PARKING_ACTIVE: bool>(
        &mut self,
    ) -> Result<(), CoreError> {
        let mut scratch = std::mem::take(&mut self.longitudinal_scratch);
        let result = (|| {
            scratch.begin(self.vehicles.len());
            let delta_time = self.fixed_delta_time_ms as f32 / 1_000.0;

            for (update_sequence, handle) in self.vehicle_update_order.iter().enumerate() {
                let Some(vehicle) = self.vehicle(handle) else {
                    continue;
                };
                let update_sequence = u64::try_from(update_sequence)
                    .expect("vehicle update sequence must fit in u64");

                match vehicle.status {
                    VehicleStatus::Completed | VehicleStatus::Parked => continue,
                    VehicleStatus::Stopped => {
                        scratch.set(LongitudinalMotion::stationary(handle, update_sequence));
                    }
                    VehicleStatus::Active => {
                        let profile = self
                            .vehicle_profile(vehicle.profile)
                            .expect("live vehicle profile must exist")
                            .iidm();
                        let signal_stop = if !self.signal_state.has_restrictive_group() {
                            None
                        } else {
                            let horizon = self.signal_stop_horizon(vehicle, profile)?;
                            self.nearest_denied_signal_stop(vehicle, horizon)?
                        };
                        let leader = self.occupancy_scratch.leader(handle).map(|observation| {
                            let leader = self
                                .vehicle(observation.leader)
                                .expect("occupancy leader must be live");
                            let leader_profile = self
                                .vehicle_profile(leader.profile)
                                .expect("leader profile must exist")
                                .iidm();
                            LeaderKinematics {
                                observation,
                                current_speed: leader.current_speed.value(),
                                emergency_deceleration: leader_profile.emergency_deceleration,
                            }
                        });
                        let mut motion = compute_motion(
                            handle,
                            update_sequence,
                            vehicle.current_speed.value(),
                            profile,
                            leader,
                            delta_time,
                        )?;
                        let route_end_distance = self
                            .route_end_distance_within(vehicle, f64::from(motion.final_travel()));
                        let parking_stop = if PARKING_ACTIVE {
                            self.parking_stop_within(vehicle, profile)?
                        } else {
                            None
                        };
                        if route_end_distance.is_none()
                            && signal_stop.is_none()
                            && parking_stop.is_none()
                        {
                            scratch.set(motion);
                            continue;
                        }
                        if let Some(constraint) = parking_stop {
                            scratch.push_parking_stop(handle, constraint);
                        }
                        motion.apply_spatial_stops(
                            route_end_distance,
                            signal_stop,
                            parking_stop,
                            (signal_stop.is_some() || parking_stop.is_some()).then_some(profile),
                            delta_time,
                        )?;
                        scratch.set(motion);
                    }
                }
            }

            scratch.project(self.vehicle_update_order.iter(), delta_time)
        })();
        self.longitudinal_scratch = scratch;
        result
    }

    fn parking_stop_within(
        &self,
        vehicle: &VehicleState,
        profile: crate::IidmProfileSpec,
    ) -> Result<Option<ParkingStopConstraint>, CoreError> {
        let Some(RuntimeVehicleParkingBinding::Reserved {
            vehicle: bound_vehicle,
            space,
            target,
        }) = self.parking_runtime.vehicle_binding(vehicle.handle)
        else {
            return Ok(None);
        };
        if bound_vehicle != vehicle.handle
            || self.parking_runtime.space_state(space)
                != Some(ParkingSpaceState::Reserved {
                    vehicle: vehicle.handle,
                })
        {
            return Err(CoreError::ParkingBindingInvariantViolation {
                stage: "step_parking_target_pair",
                vehicle: Some(vehicle.handle),
                space: Some(space),
            });
        }
        let Some(target) = target else {
            return Ok(None);
        };
        if target.route != vehicle.route {
            return Err(CoreError::ParkingBindingInvariantViolation {
                stage: "step_parking_target_route",
                vehicle: Some(vehicle.handle),
                space: Some(space),
            });
        }
        let entry = self
            .parking
            .space_entry(space)
            .expect("normalized ParkingSpace must have entry");

        let horizon = self.parking_stop_horizon(vehicle, profile, space)?;
        match self.route_distance_indices[vehicle.route.index()].distance_within(
            vehicle.route_edge_index,
            vehicle.edge_progress.value(),
            target.route_edge_index,
            entry.progress(),
            horizon,
        ) {
            RouteDistanceQuery::Within(route_distance) => Ok(Some(ParkingStopConstraint {
                space,
                route: vehicle.route,
                route_edge_index: target.route_edge_index,
                entry_progress: entry.progress(),
                route_distance,
            })),
            RouteDistanceQuery::BeyondHorizon => Ok(None),
            RouteDistanceQuery::Passed => Err(CoreError::ParkingBindingInvariantViolation {
                stage: "step_parking_target_passed",
                vehicle: Some(vehicle.handle),
                space: Some(space),
            }),
        }
    }

    fn route_end_distance_within(&self, vehicle: &VehicleState, max_travel: f64) -> Option<f64> {
        let horizon = if max_travel <= f64::MAX - LONGITUDINAL_CONSTRAINT_TOLERANCE_METERS {
            max_travel + LONGITUDINAL_CONSTRAINT_TOLERANCE_METERS
        } else {
            f64::MAX
        };
        let route = self
            .route_slot(vehicle.route)
            .expect("live vehicle route must exist");
        let current_edge_length = self
            .lane_graph
            .edge_length(route.edge_handles[vehicle.route_edge_index])
            .expect("route edge must exist")
            .value();
        let remaining_on_edge = f64::from(current_edge_length) - vehicle.edge_progress.value();
        if remaining_on_edge > horizon {
            return None;
        }
        let Some(next_edge) = route.edge_handles.get(vehicle.route_edge_index + 1) else {
            return Some(remaining_on_edge.max(0.0));
        };
        let next_edge_length = self
            .lane_graph
            .edge_length(*next_edge)
            .expect("route edge must exist")
            .value();
        if f64::from(next_edge_length) > horizon - remaining_on_edge {
            return None;
        }
        match self.route_distance_indices[vehicle.route.index()].distance_to_end_within(
            vehicle.route_edge_index,
            vehicle.edge_progress.value(),
            horizon,
        ) {
            RouteDistanceQuery::Within(distance) => Some(distance),
            RouteDistanceQuery::Passed | RouteDistanceQuery::BeyondHorizon => None,
        }
    }

    fn build_occupancy(&self, scratch: &mut OccupancyScratch) {
        scratch.begin(self.lane_graph.edges().len(), self.vehicles.len());

        for handle in self.vehicle_update_order.iter() {
            let Some(vehicle) = self.vehicle(handle) else {
                continue;
            };
            if !matches!(
                vehicle.status,
                VehicleStatus::Active | VehicleStatus::Stopped
            ) {
                continue;
            }

            let edge = self.vehicle_edge(vehicle);
            let vehicle_length = self
                .vehicle_profile(vehicle.profile)
                .expect("live vehicle profile must exist")
                .iidm()
                .length;
            scratch.count(edge, vehicle_length);
        }

        scratch.allocate_occupants();
        for (update_sequence, handle) in self.vehicle_update_order.iter().enumerate() {
            let Some(vehicle) = self.vehicle(handle) else {
                continue;
            };
            if !matches!(
                vehicle.status,
                VehicleStatus::Active | VehicleStatus::Stopped
            ) {
                continue;
            }

            let edge = self.vehicle_edge(vehicle);
            let vehicle_length = self
                .vehicle_profile(vehicle.profile)
                .expect("live vehicle profile must exist")
                .iidm()
                .length;
            scratch.insert(
                edge,
                Occupant {
                    vehicle: handle,
                    front_progress: vehicle.edge_progress,
                    vehicle_length,
                    update_sequence: u64::try_from(update_sequence)
                        .expect("vehicle update sequence must fit in u64"),
                },
            );
        }
        scratch.sort_edges();
    }

    fn vehicle_edge(&self, vehicle: &VehicleState) -> EdgeHandle {
        self.route_slot(vehicle.route)
            .expect("live vehicle route must exist")
            .edge_handles[vehicle.route_edge_index]
    }

    fn leader_horizon(&self, vehicle: &VehicleState) -> Result<f64, CoreError> {
        let profile = self
            .vehicle_profile(vehicle.profile)
            .expect("live vehicle profile must exist")
            .iidm();
        let speed = f64::from(vehicle.current_speed.value());
        let delta_time = self.fixed_delta_time_ms as f64 / 1_000.0;
        let upper_speed = speed + f64::from(profile.max_acceleration) * delta_time;
        Self::finite_leader_value(vehicle.handle, "upper_speed", upper_speed)?;
        let travel_upper =
            Self::half_product(speed, delta_time) + Self::half_product(upper_speed, delta_time);
        Self::finite_leader_value(vehicle.handle, "travel_upper", travel_upper)?;
        let braking_distance =
            Self::braking_distance(upper_speed, f64::from(profile.emergency_deceleration));
        Self::finite_leader_value(vehicle.handle, "braking_distance", braking_distance)?;
        let hard_horizon = travel_upper + braking_distance;
        Self::finite_leader_value(vehicle.handle, "hard_horizon", hard_horizon)?;
        let comfort_horizon = f64::from(profile.min_gap) + speed * f64::from(profile.time_headway);
        Self::finite_leader_value(vehicle.handle, "comfort_horizon", comfort_horizon)?;

        Ok(hard_horizon.max(comfort_horizon))
    }

    fn signal_stop_horizon(
        &self,
        vehicle: &VehicleState,
        profile: crate::IidmProfileSpec,
    ) -> Result<f64, CoreError> {
        let speed = f64::from(vehicle.current_speed.value());
        let delta_time = self.fixed_delta_time_ms as f64 / 1_000.0;
        let upper_speed = Self::finite_signal_stop_value(
            vehicle.handle,
            "signal_upper_speed",
            speed + f64::from(profile.max_acceleration) * delta_time,
        )?;
        let travel_upper = Self::finite_signal_stop_value(
            vehicle.handle,
            "signal_travel_upper",
            Self::half_product(speed, delta_time) + Self::half_product(upper_speed, delta_time),
        )?;
        let comfortable_braking_distance = Self::finite_signal_stop_value(
            vehicle.handle,
            "signal_comfortable_braking_distance",
            Self::braking_distance(upper_speed, f64::from(profile.comfortable_deceleration)),
        )?;
        let comfortable_horizon = Self::finite_signal_stop_value(
            vehicle.handle,
            "signal_comfortable_horizon",
            travel_upper + comfortable_braking_distance,
        )?;
        Ok(comfortable_horizon.max(self.leader_horizon(vehicle)?))
    }

    fn parking_stop_horizon(
        &self,
        vehicle: &VehicleState,
        profile: crate::IidmProfileSpec,
        space: crate::ParkingSpaceHandle,
    ) -> Result<f64, CoreError> {
        let finite = |stage, value: f64| {
            if value.is_finite() {
                Ok(value)
            } else {
                Err(CoreError::NonFiniteParkingComputation {
                    stage,
                    vehicle: vehicle.handle,
                    space,
                    value,
                })
            }
        };
        let speed = f64::from(vehicle.current_speed.value());
        let delta_time = self.fixed_delta_time_ms as f64 / 1_000.0;
        let upper_speed = finite(
            "parking_upper_speed",
            speed + f64::from(profile.max_acceleration) * delta_time,
        )?;
        let travel_upper = finite(
            "parking_travel_upper",
            Self::half_product(speed, delta_time) + Self::half_product(upper_speed, delta_time),
        )?;
        let braking_distance = finite(
            "parking_comfortable_braking_distance",
            Self::braking_distance(upper_speed, f64::from(profile.comfortable_deceleration)),
        )?;
        finite(
            "parking_comfortable_horizon",
            travel_upper + braking_distance,
        )
    }

    fn nearest_denied_signal_stop(
        &self,
        vehicle: &VehicleState,
        horizon: f64,
    ) -> Result<Option<SignalStopConstraint>, CoreError> {
        let route = self
            .route_slot(vehicle.route)
            .expect("live vehicle route must exist");
        let mut search_edge_index = vehicle.route_edge_index;
        let mut distance = 0.0;
        let mut first = true;

        while let Some(next) = route
            .next_controlled_transition
            .get(search_edge_index)
            .copied()
            .flatten()
        {
            let progress = if first {
                vehicle.edge_progress.value()
            } else {
                0.0
            };
            let BoundedDistance::Finite(distance_from_edge_start) = next.distance_from_edge_start
            else {
                break;
            };
            let segment_distance = (distance_from_edge_start - progress).max(0.0);
            if segment_distance > horizon - distance {
                break;
            }
            distance += segment_distance;
            if distance > horizon + LONGITUDINAL_CONSTRAINT_TOLERANCE_METERS {
                break;
            }

            let gate = self
                .signals
                .movement_gate_state_by_index(&self.signal_state, next.gate)
                .expect("normalized route Gate must have committed state");
            if let MovementGateSignalState::Controlled {
                group,
                aspect,
                permission: SignalLayerPermission::DenyAndStop,
            } = gate.signal()
            {
                return Ok(Some(SignalStopConstraint {
                    route_distance: distance,
                    gate: gate.key(),
                    stop_line: gate.stop_line(),
                    group,
                    aspect,
                    from_route_edge_index: next.from_route_edge_index,
                    to_route_edge_index: next.from_route_edge_index + 1,
                }));
            }

            search_edge_index = next.from_route_edge_index + 1;
            if search_edge_index >= route.edge_handles.len() {
                break;
            }
            first = false;
        }

        Ok(None)
    }

    fn find_leader(
        &self,
        scratch: &OccupancyScratch,
        follower: &VehicleState,
        bumper_gap_horizon: f64,
    ) -> Result<Option<LeaderObservation>, CoreError> {
        Self::finite_leader_value(follower.handle, "bumper_gap_horizon", bumper_gap_horizon)?;
        let front_horizon = bumper_gap_horizon + f64::from(scratch.max_vehicle_length());
        Self::finite_leader_value(follower.handle, "front_horizon", front_horizon)?;

        let route = self
            .route_slot(follower.route)
            .expect("live vehicle route must exist");
        let current_edge = route.edge_handles[follower.route_edge_index];
        let current_occupants = scratch.edge(current_edge);
        // 相同 front progress 是非法物理重叠；update sequence 只形成确定排序，不能把 tie 合法化为 leader。
        let first_strictly_ahead = current_occupants.partition_point(|occupant| {
            occupant.front_progress.value() <= follower.edge_progress.value()
        });
        for occupant in &current_occupants[first_strictly_ahead..] {
            if occupant.vehicle == follower.handle {
                continue;
            }
            let front_distance = occupant.front_progress.value() - follower.edge_progress.value();
            let bumper_gap =
                normalize_physical_gap(front_distance - f64::from(occupant.vehicle_length));
            if bumper_gap <= bumper_gap_horizon {
                return Ok(Some(LeaderObservation {
                    leader: occupant.vehicle,
                    bumper_gap: bumper_gap as f32,
                }));
            }
            break;
        }

        let current_edge_length = self
            .lane_graph
            .edge_length(current_edge)
            .expect("route edge must exist")
            .value();
        let mut distance_to_edge_start =
            f64::from(current_edge_length) - follower.edge_progress.value();

        for edge in route
            .edge_handles
            .iter()
            .copied()
            .skip(follower.route_edge_index + 1)
        {
            Self::finite_leader_value(
                follower.handle,
                "distance_to_edge_start",
                distance_to_edge_start,
            )?;
            if distance_to_edge_start > front_horizon {
                break;
            }

            for occupant in scratch.edge(edge) {
                if occupant.vehicle == follower.handle {
                    continue;
                }
                let remaining = front_horizon - distance_to_edge_start;
                if occupant.front_progress.value() > remaining {
                    break;
                }
                let front_distance = distance_to_edge_start + occupant.front_progress.value();
                let bumper_gap =
                    normalize_physical_gap(front_distance - f64::from(occupant.vehicle_length));
                if bumper_gap <= bumper_gap_horizon {
                    return Ok(Some(LeaderObservation {
                        leader: occupant.vehicle,
                        bumper_gap: bumper_gap as f32,
                    }));
                }
            }

            let edge_length = self
                .lane_graph
                .edge_length(edge)
                .expect("route edge must exist")
                .value();
            if f64::from(edge_length) > front_horizon - distance_to_edge_start {
                break;
            }
            distance_to_edge_start += f64::from(edge_length);
        }

        Ok(None)
    }

    fn finite_leader_value(
        vehicle: VehicleHandle,
        stage: &'static str,
        value: f64,
    ) -> Result<f64, CoreError> {
        if !value.is_finite() {
            return Err(CoreError::NonFiniteLeaderComputation {
                vehicle,
                stage,
                value,
            });
        }
        Ok(value)
    }

    fn finite_signal_stop_value(
        vehicle: VehicleHandle,
        stage: &'static str,
        value: f64,
    ) -> Result<f64, CoreError> {
        if !value.is_finite() {
            return Err(CoreError::NonFiniteSignalStopComputation {
                vehicle,
                stage,
                value,
            });
        }
        Ok(if value == 0.0 { 0.0 } else { value })
    }

    fn braking_distance(speed: f64, deceleration: f64) -> f64 {
        if speed == 0.0 {
            return 0.0;
        }
        if deceleration > f64::MAX / 2.0 {
            return speed / deceleration * (0.5 * speed);
        }

        let denominator = 2.0 * deceleration;
        if speed < 1.0 {
            speed / (denominator / speed)
        } else {
            speed / denominator * speed
        }
    }

    fn half_product(left: f64, right: f64) -> f64 {
        if left >= right {
            (0.5 * left) * right
        } else {
            left * (0.5 * right)
        }
    }

    fn normalize_vehicle_input(
        &self,
        route: RouteHandle,
        input: &VehicleSpawnInput,
    ) -> Result<NormalizedVehicleInput, CoreError> {
        let route_slot = self
            .route_slot(route)
            .expect("route handle was resolved from active route map");
        let edge = route_slot
            .edge_handles
            .get(input.route_edge_index)
            .copied()
            .ok_or_else(|| CoreError::InvalidVehicleRouteEdgeIndex {
                vehicle_id: input.id.clone(),
                route_id: input.route_id.clone(),
                route_edge_index: input.route_edge_index,
                route_edge_count: route_slot.edge_handles.len(),
            })?;

        let edge_length = self
            .lane_graph
            .edge_length(edge)
            .expect("validated route edge must exist");
        if input.edge_progress.value() > f64::from(edge_length.value()) {
            return Err(CoreError::VehicleEdgeProgressOutOfRange {
                vehicle_id: input.id.clone(),
                edge_id: self
                    .lane_graph
                    .edge_external_id(edge)
                    .expect("validated route edge must exist")
                    .to_owned(),
                edge_progress: input.edge_progress.value(),
                edge_length: f64::from(edge_length.value()),
            });
        }

        if input.status != VehicleStatus::Active && input.initial_speed != Speed::ZERO {
            return Err(CoreError::InvalidInactiveVehicleMotion {
                vehicle_id: input.id.clone(),
                status: input.status,
                initial_speed: f64::from(input.initial_speed.value()),
            });
        }

        let mut edge_progress = input.edge_progress;
        if input.status == VehicleStatus::Completed {
            let expected_route_edge_index = route_slot.edge_handles.len() - 1;
            if input.route_edge_index != expected_route_edge_index
                || input.edge_progress.value() + EDGE_BOUNDARY_TOLERANCE_METERS
                    < f64::from(edge_length.value())
            {
                return Err(CoreError::InvalidCompletedVehicleState {
                    vehicle_id: input.id.clone(),
                    route_id: input.route_id.clone(),
                    route_edge_index: input.route_edge_index,
                    expected_route_edge_index,
                    edge_progress: input.edge_progress.value(),
                    edge_length: f64::from(edge_length.value()),
                });
            }

            edge_progress = EdgeProgress::try_new(f64::from(edge_length.value()))
                .expect("edge length is valid");
        }

        Ok(NormalizedVehicleInput {
            profile: input.profile,
            route_edge_index: input.route_edge_index,
            edge_progress,
            current_speed: input.initial_speed,
            status: input.status,
        })
    }

    fn advance_vehicle<const PARKING_ACTIVE: bool>(
        context: VehicleAdvanceContext<'_>,
        vehicle: &mut VehicleState,
        motion: LongitudinalMotion,
        parking_stop: Option<ParkingStopConstraint>,
        events: &mut Vec<CoreEvent>,
        spatial_changes: &mut Vec<VehicleHandle>,
    ) -> Result<Option<VehicleCompletedRouteEvent>, CoreError> {
        if vehicle.status != VehicleStatus::Active {
            return Ok(None);
        }

        let delta_time_seconds = context.fixed_delta_time_ms as f32 / 1_000.0;
        vehicle.current_speed =
            Speed::try_new(motion.final_speed()).expect("longitudinal motion speed must be valid");
        vehicle.applied_acceleration =
            Acceleration::try_new(motion.applied_acceleration(delta_time_seconds)?)
                .expect("longitudinal applied acceleration must be valid");
        if let Some(signal_stop) = motion.signal_stop_projection() {
            events.push(CoreEvent::VehicleSignalStopProjectionApplied(
                VehicleSignalStopProjectionAppliedEvent {
                    tick_index: context.tick_index,
                    vehicle: vehicle.handle,
                    route: vehicle.route,
                    from_route_edge_index: signal_stop.from_route_edge_index,
                    to_route_edge_index: signal_stop.to_route_edge_index,
                    gate: signal_stop.gate,
                    stop_line: signal_stop.stop_line,
                    group: signal_stop.group,
                    aspect: signal_stop.aspect,
                },
            ));
        } else if PARKING_ACTIVE && motion.parking_stop_projection() {
            let parking_stop = parking_stop.expect("Parking projection must resolve sparse target");
            events.push(CoreEvent::VehicleParkingStopProjectionApplied(
                VehicleParkingStopProjectionAppliedEvent {
                    tick_index: context.tick_index,
                    vehicle: vehicle.handle,
                    space: parking_stop.space,
                    route: parking_stop.route,
                    route_edge_index: parking_stop.route_edge_index,
                },
            ));
        }
        if let Some(leader) = motion.safety_projection_leader() {
            events.push(CoreEvent::VehicleFollowingSafetyProjectionApplied(
                VehicleFollowingSafetyProjectionAppliedEvent {
                    tick_index: context.tick_index,
                    vehicle: vehicle.handle,
                    leader,
                },
            ));
        }

        let travel_distance = f64::from(motion.final_travel());
        if travel_distance <= EDGE_BOUNDARY_TOLERANCE_METERS
            && !motion.reaches_route_end()
            && !(PARKING_ACTIVE
                && parking_stop.is_some_and(|constraint| motion.reaches_parking_stop(constraint)))
        {
            return Ok(None);
        }

        let route = context
            .routes
            .get(vehicle.route.index())
            .filter(|route| route.active && route.generation == vehicle.route.generation())
            .expect("validated vehicle route must exist");
        let max_iterations = route.edge_handles.len() - vehicle.route_edge_index;
        let mut candidate_progress = vehicle.edge_progress.advance(motion.final_travel())?;
        let mut remaining = travel_distance;
        let mut completed_event = None;

        for _ in 0..max_iterations {
            if is_edge_boundary_remainder_zero(remaining) {
                if motion.reaches_route_end()
                    && vehicle.route_edge_index + 1 == route.edge_handles.len()
                {
                    let current_edge = route.edge_handles[vehicle.route_edge_index];
                    let edge_length = context
                        .lane_graph
                        .edge_length(current_edge)
                        .expect("validated route edge must exist")
                        .value();
                    vehicle.edge_progress = EdgeProgress::try_new(f64::from(edge_length))
                        .expect("edge length is valid progress");
                    vehicle.current_speed = Speed::ZERO;
                    vehicle.applied_acceleration = Acceleration::ZERO;
                    vehicle.status = VehicleStatus::Completed;
                    if spatial_changes.last().copied() != Some(vehicle.handle) {
                        spatial_changes.push(vehicle.handle);
                    }
                    completed_event = Some(VehicleCompletedRouteEvent {
                        tick_index: context.tick_index,
                        vehicle: vehicle.handle,
                        route: vehicle.route,
                        edge: current_edge,
                        route_edge_index: vehicle.route_edge_index,
                    });
                }
                break;
            }

            let current_edge = route
                .edge_handles
                .get(vehicle.route_edge_index)
                .copied()
                .expect("validated route edge index must exist");
            let edge_length = context
                .lane_graph
                .edge_length(current_edge)
                .expect("validated route edge must exist");
            let edge_length_value = f64::from(edge_length.value());
            let next_progress = candidate_progress.value();
            if !next_progress.is_finite() {
                return Err(CoreError::NonFiniteRouteTravel {
                    vehicle: vehicle.handle,
                    speed: f64::from(motion.final_speed()),
                    delta_time_ms: context.fixed_delta_time_ms,
                });
            }

            if PARKING_ACTIVE
                && let Some(stop) = parking_stop
                && stop.route == vehicle.route
                && stop.route_edge_index == vehicle.route_edge_index
            {
                let crosses_boundary =
                    next_progress > stop.entry_progress + LONGITUDINAL_CONSTRAINT_TOLERANCE_METERS;
                let reaches_boundary =
                    longitudinal_constraint_reached(next_progress, stop.entry_progress);
                if crosses_boundary
                    || (reaches_boundary
                        && computed_speed_is_above_near_zero(f64::from(
                            vehicle.current_speed.value(),
                        )))
                {
                    return Err(CoreError::ParkingTraversalBoundaryInvariant {
                        vehicle: vehicle.handle,
                        space: stop.space,
                        route: stop.route,
                        route_edge_index: stop.route_edge_index,
                        remaining_travel: (next_progress - stop.entry_progress).max(0.0),
                        final_speed: f64::from(vehicle.current_speed.value()),
                    });
                }
                if reaches_boundary {
                    vehicle.edge_progress = EdgeProgress::try_new(stop.entry_progress)
                        .expect("normalized Parking entry progress must be valid");
                    vehicle.current_speed = Speed::ZERO;
                    vehicle.applied_acceleration = Acceleration::ZERO;
                    break;
                }
            }

            if next_progress + EDGE_BOUNDARY_TOLERANCE_METERS < edge_length_value {
                vehicle.edge_progress = candidate_progress;
                break;
            }

            candidate_progress = candidate_progress.rebase_after_edge(edge_length)?;
            let remainder = candidate_progress.value();
            remaining = if is_edge_boundary_remainder_zero(remainder) {
                0.0
            } else {
                remainder
            };

            if vehicle.route_edge_index + 1 < route.edge_handles.len() {
                let from_route_edge_index = vehicle.route_edge_index;
                let to_route_edge_index = from_route_edge_index + 1;
                let transition = route
                    .transitions
                    .get(from_route_edge_index)
                    .copied()
                    .expect("next route transition must exist");
                let to_edge = transition.to_edge;

                let denied_gate = transition.gate.and_then(|gate_index| {
                    let gate = context
                        .signals
                        .movement_gate_state_by_index(context.signal_state, gate_index)
                        .expect("normalized route Gate must have committed state");
                    matches!(
                        gate.signal(),
                        MovementGateSignalState::Controlled {
                            permission: SignalLayerPermission::DenyAndStop,
                            ..
                        }
                    )
                    .then_some(gate)
                });
                if let Some(gate) = denied_gate {
                    if remaining > EDGE_BOUNDARY_TOLERANCE_METERS
                        || computed_speed_is_above_near_zero(f64::from(
                            vehicle.current_speed.value(),
                        ))
                    {
                        return Err(CoreError::SignalTraversalDeniedInvariant {
                            vehicle: vehicle.handle,
                            route: vehicle.route,
                            from_route_edge_index,
                            to_route_edge_index,
                            gate: gate.key(),
                            remaining_travel: remaining,
                            final_speed: f64::from(vehicle.current_speed.value()),
                        });
                    }
                    vehicle.edge_progress = EdgeProgress::try_new(edge_length_value)
                        .expect("edge length is valid progress");
                    break;
                }

                if current_edge != to_edge
                    && spatial_changes.last().copied() != Some(vehicle.handle)
                {
                    spatial_changes.push(vehicle.handle);
                }

                events.push(CoreEvent::VehicleChangedEdge(VehicleChangedEdgeEvent {
                    tick_index: context.tick_index,
                    vehicle: vehicle.handle,
                    route: vehicle.route,
                    from_edge: current_edge,
                    to_edge,
                    from_route_edge_index,
                    to_route_edge_index,
                }));

                vehicle.route_edge_index = to_route_edge_index;
                vehicle.edge_progress = EdgeProgress::ZERO;
            } else {
                vehicle.edge_progress = EdgeProgress::try_new(edge_length_value)
                    .expect("edge length is valid progress");
                vehicle.current_speed = Speed::ZERO;
                vehicle.applied_acceleration = Acceleration::ZERO;
                vehicle.status = VehicleStatus::Completed;
                if spatial_changes.last().copied() != Some(vehicle.handle) {
                    spatial_changes.push(vehicle.handle);
                }
                completed_event = Some(VehicleCompletedRouteEvent {
                    tick_index: context.tick_index,
                    vehicle: vehicle.handle,
                    route: vehicle.route,
                    edge: current_edge,
                    route_edge_index: vehicle.route_edge_index,
                });
                break;
            }
        }

        Ok(completed_event)
    }

    fn route_slot(&self, handle: RouteHandle) -> Option<&RouteSlot> {
        self.routes
            .get(handle.index())
            .filter(|route| route.active && route.generation == handle.generation())
    }

    fn vehicle_slot(&self, handle: VehicleHandle) -> Option<&VehicleSlot> {
        self.vehicles
            .get(handle.index())
            .filter(|vehicle| vehicle.generation == handle.generation() && vehicle.state.is_some())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct NormalizedVehicleInput {
    profile: VehicleProfileHandle,
    route_edge_index: usize,
    edge_progress: EdgeProgress,
    current_speed: Speed,
    status: VehicleStatus,
}

fn parking_emergency_travel(
    stage: &'static str,
    vehicle: VehicleHandle,
    space: crate::ParkingSpaceHandle,
    speed: f32,
    emergency_deceleration: f32,
    delta_time: f32,
) -> Result<f64, CoreError> {
    emergency_min_travel(vehicle, speed, emergency_deceleration, delta_time)
        .map(f64::from)
        .map_err(|error| match error {
            CoreError::NonFiniteLongitudinalComputation { value, .. } => {
                CoreError::NonFiniteParkingComputation {
                    stage,
                    vehicle,
                    space,
                    value,
                }
            }
            error => error,
        })
}

#[cfg(test)]
#[path = "world_occupancy_tests.rs"]
mod occupancy_tests;

#[cfg(test)]
#[path = "world_retained_memory_tests.rs"]
mod retained_memory_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CoreError, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData, LaneEdge,
        ParkingArea, ParkingSpaceGeometryInput, ParkingSpaceInput, SignalRegistry, Speed,
        TickInput, VehicleParkingState, VehicleProfile, VehicleProfileHandle,
        VehicleProfileRegistry,
    };
    use proptest::prelude::*;

    fn traffic_data<I>(
        lane_graph: LaneGraph,
        routes: I,
    ) -> (InitialTrafficData, VehicleProfileHandle)
    where
        I: IntoIterator<Item = Route>,
    {
        let registry = VehicleProfileRegistry::try_new([VehicleProfile::try_new_iidm(
            "test-profile",
            IidmProfileSpec {
                length: 4.5,
                desired_speed: 13.9,
                min_gap: 2.0,
                time_headway: 1.5,
                max_acceleration: 1.4,
                comfortable_deceleration: 2.0,
                emergency_deceleration: 4.0,
            },
        )
        .expect("valid profile")])
        .expect("valid profile registry");
        let profile = registry
            .profile_handle("test-profile")
            .expect("profile handle exists");
        let traffic_data =
            InitialTrafficData::try_new(lane_graph, routes, registry).expect("valid traffic data");
        (traffic_data, profile)
    }

    const SCALE_VEHICLES_PER_EDGE: usize = 999;

    fn scale_chain_graph(prefix: &str, edge_count: usize) -> (LaneGraph, Vec<String>) {
        let edge_ids = (0..edge_count)
            .map(|index| format!("{prefix}-{index:06}"))
            .collect::<Vec<_>>();
        let graph = LaneGraph::try_new(edge_ids.iter().enumerate().map(|(index, edge_id)| {
            LaneEdge::new(
                edge_id.clone(),
                EdgeLength::try_new(10_000.0).expect("scale edge length"),
                edge_ids.get(index + 1).into_iter().cloned(),
            )
        }))
        .expect("scale graph");
        (graph, edge_ids)
    }

    fn lifecycle_scale_world(vehicle_count: usize) -> CoreWorld {
        let edge_count = vehicle_count.div_ceil(SCALE_VEHICLES_PER_EDGE);
        let (lane_graph, edge_ids) = scale_chain_graph("lifecycle-edge", edge_count);
        let (traffic_data, profile) = traffic_data(
            lane_graph,
            [Route::try_new("R", edge_ids).expect("scale route")],
        );
        let vehicles = (0..vehicle_count)
            .map(|index| {
                let route_edge_index = index / SCALE_VEHICLES_PER_EDGE;
                let local_index = index % SCALE_VEHICLES_PER_EDGE;
                VehicleSpawnInput::active(
                    format!("V{index:06}"),
                    profile,
                    "R",
                    route_edge_index,
                    EdgeProgress::try_new(5.0 + 10.0 * local_index as f64).expect("scale progress"),
                    Speed::ZERO,
                )
            })
            .collect();
        CoreWorld::with_traffic_data(20, traffic_data, vehicles).expect("scale world")
    }

    fn parking_retained_scale_world(vehicle_count: usize) -> CoreWorld {
        let edge_count = vehicle_count.div_ceil(SCALE_VEHICLES_PER_EDGE);
        let (lane_graph, edge_ids) = scale_chain_graph("parking-retained-edge", edge_count);
        let parking = ParkingRegistry::try_new(
            &lane_graph,
            [],
            (0..vehicle_count).map(|index| {
                let edge_id = &edge_ids[index / SCALE_VEHICLES_PER_EDGE];
                let local_index = index % SCALE_VEHICLES_PER_EDGE;
                ParkingSpaceInput::new(
                    format!("S{index:06}"),
                    None,
                    edge_id,
                    1.0 + 10.0 * local_index as f64,
                    edge_id,
                    2.0 + 10.0 * local_index as f64,
                    ParkingSpaceGeometryInput::new(-3.0, 0.0, 5.0, 2.4),
                )
            }),
        )
        .expect("parking retained registry");
        let (base, profile) = traffic_data(
            lane_graph,
            [Route::try_new("R", edge_ids).expect("parking retained route")],
        );
        let traffic = InitialTrafficData::try_new_with_signals_and_parking(
            base.lane_graph().clone(),
            base.routes().iter().cloned(),
            base.vehicle_profiles().clone(),
            SignalRegistry::empty(),
            parking,
        )
        .expect("parking retained traffic");
        let vehicles = (0..vehicle_count)
            .map(|index| {
                let route_edge_index = index / SCALE_VEHICLES_PER_EDGE;
                let local_index = index % SCALE_VEHICLES_PER_EDGE;
                VehicleSpawnInput::active(
                    format!("V{index:06}"),
                    profile,
                    "R",
                    route_edge_index,
                    EdgeProgress::try_new(5.0 + 10.0 * local_index as f64)
                        .expect("parking retained progress"),
                    Speed::ZERO,
                )
            })
            .collect();
        CoreWorld::with_traffic_data(20, traffic, vehicles).expect("parking retained world")
    }

    fn sparse_command_world(background_count: usize) -> (CoreWorld, VehicleProfileHandle) {
        let background_edge_count = background_count.div_ceil(SCALE_VEHICLES_PER_EDGE);
        let (lane_graph, edge_ids) = scale_chain_graph("sparse-edge", background_edge_count + 1);
        let (traffic_data, profile) = traffic_data(
            lane_graph,
            [Route::try_new("R", edge_ids).expect("sparse route")],
        );
        let mut vehicles = (0..background_count)
            .map(|index| {
                let route_edge_index = 1 + index / SCALE_VEHICLES_PER_EDGE;
                let local_index = index % SCALE_VEHICLES_PER_EDGE;
                VehicleSpawnInput::active(
                    format!("B{index:06}"),
                    profile,
                    "R",
                    route_edge_index,
                    EdgeProgress::try_new(1_000.0 + 8.0 * local_index as f64)
                        .expect("background progress"),
                    Speed::ZERO,
                )
            })
            .collect::<Vec<_>>();
        vehicles.push(VehicleSpawnInput::active(
            "local",
            profile,
            "R",
            0,
            EdgeProgress::try_new(5.0).expect("local progress"),
            Speed::ZERO,
        ));
        (
            CoreWorld::with_traffic_data(20, traffic_data, vehicles).expect("sparse world"),
            profile,
        )
    }

    fn parking_runtime_world() -> (CoreWorld, VehicleProfileHandle) {
        let lane_graph = LaneGraph::try_new([LaneEdge::new(
            "A",
            EdgeLength::try_new(200.0).expect("parking edge length"),
            Vec::<String>::new(),
        )])
        .expect("parking graph");
        let parking = ParkingRegistry::try_new(
            &lane_graph,
            [ParkingArea::new("lot")],
            [
                ParkingSpaceInput::new(
                    "S0",
                    Some("lot".to_owned()),
                    "A",
                    20.0,
                    "A",
                    40.0,
                    ParkingSpaceGeometryInput::new(-3.0, 0.0, 4.5, 2.4),
                ),
                ParkingSpaceInput::new(
                    "S1",
                    Some("lot".to_owned()),
                    "A",
                    60.0,
                    "A",
                    80.0,
                    ParkingSpaceGeometryInput::new(-3.0, 0.0, 4.5, 2.4),
                ),
            ],
        )
        .expect("parking registry");
        let (base, profile) = traffic_data(
            lane_graph,
            [Route::try_new("R", ["A"]).expect("parking route")],
        );
        let traffic = InitialTrafficData::try_new_with_signals_and_parking(
            base.lane_graph().clone(),
            base.routes().iter().cloned(),
            base.vehicle_profiles().clone(),
            SignalRegistry::empty(),
            parking,
        )
        .expect("parking traffic data");
        let vehicles = vec![
            VehicleSpawnInput::active("V0", profile, "R", 0, EdgeProgress::ZERO, Speed::ZERO),
            VehicleSpawnInput::active(
                "V1",
                profile,
                "R",
                0,
                EdgeProgress::try_new(120.0).expect("parking progress"),
                Speed::ZERO,
            ),
        ];
        (
            CoreWorld::with_traffic_data(20, traffic, vehicles).expect("parking world"),
            profile,
        )
    }

    fn repeated_parking_target_world() -> CoreWorld {
        let lane_graph = LaneGraph::try_new([
            LaneEdge::new("A", EdgeLength::try_new(100.0).expect("A length"), ["B"]),
            LaneEdge::new("B", EdgeLength::try_new(100.0).expect("B length"), ["A"]),
        ])
        .expect("repeated target graph");
        let parking = ParkingRegistry::try_new(
            &lane_graph,
            [],
            [ParkingSpaceInput::new(
                "S",
                None,
                "A",
                20.0,
                "A",
                40.0,
                ParkingSpaceGeometryInput::new(-3.0, 0.0, 4.5, 2.4),
            )],
        )
        .expect("repeated target parking");
        let (base, _) = traffic_data(
            lane_graph,
            [Route::try_new("R", ["A", "B", "A", "B", "A"]).expect("repeated target route")],
        );
        let traffic = InitialTrafficData::try_new_with_signals_and_parking(
            base.lane_graph().clone(),
            base.routes().iter().cloned(),
            base.vehicle_profiles().clone(),
            SignalRegistry::empty(),
            parking,
        )
        .expect("repeated target traffic");
        CoreWorld::with_traffic_data(20, traffic, Vec::<VehicleSpawnInput>::new())
            .expect("repeated target world")
    }

    fn reserved_parking_world() -> CoreWorld {
        let (mut world, _) = parking_runtime_world();
        let vehicle = world.vehicle_handle("V0").expect("parking vehicle");
        let space = world.parking().space_handle("S0").expect("parking space");
        world
            .reserve_parking_space(vehicle, space)
            .expect("parking reservation");
        world
    }

    #[test]
    fn first_reachable_parking_target_matches_independent_route_scan_oracle() {
        let world = repeated_parking_target_world();
        let route = world.route_handle("R").expect("route");
        let space = world.parking().space_handle("S").expect("space");
        let progress_samples = [
            0.0,
            19.0,
            20.0,
            20.0 + LONGITUDINAL_CONSTRAINT_TOLERANCE_METERS / 2.0,
            20.0 + 2.0 * LONGITUDINAL_CONSTRAINT_TOLERANCE_METERS,
            99.0,
        ];

        for from_index in 0..5 {
            for from_progress in progress_samples {
                let expected = (from_index..5).find(|candidate| {
                    let is_entry_edge = candidate % 2 == 0;
                    let current_is_reachable = *candidate != from_index
                        || longitudinal_constraint_reached(20.0, from_progress);
                    is_entry_edge && current_is_reachable
                });
                let actual = world
                    .first_reachable_parking_entry(route, from_index, from_progress, space)
                    .map(|target| target.route_edge_index);

                assert_eq!(
                    actual, expected,
                    "from occurrence {from_index} at progress {from_progress}"
                );
            }
        }
    }

    #[test]
    fn unit_step_advances_post_step_time() {
        let mut world = CoreWorld::new(20).expect("valid world");

        let result = world.step(TickInput::new(20)).expect("step succeeds");

        assert_eq!(world.tick_index(), 1);
        assert_eq!(world.time_ms(), 20);
        assert_eq!(result.tick_index, 1);
        assert_eq!(result.time_ms, 20);
    }

    #[test]
    fn candidate_state_scratch_reuses_allocation_across_successful_ticks() {
        let lane_graph = LaneGraph::try_new([LaneEdge::new(
            "A",
            EdgeLength::try_new(10_000.0).expect("valid edge length"),
            Vec::<String>::new(),
        )])
        .expect("valid lane graph");
        let route = Route::try_new("R1", ["A"]).expect("valid route");
        let (traffic_data, profile) = traffic_data(lane_graph, [route]);
        let vehicle =
            VehicleSpawnInput::active("V1", profile, "R1", 0, EdgeProgress::ZERO, Speed::ZERO);
        let mut world =
            CoreWorld::with_traffic_data(16, traffic_data, vec![vehicle]).expect("valid world");
        let capacity = world.candidate_state_scratch.states.capacity();
        let allocation = world.candidate_state_scratch.states.as_ptr();
        assert!(capacity >= world.vehicles.len());
        assert!(world.clone().candidate_state_scratch.states.capacity() >= capacity);

        world.step(TickInput::new(16)).expect("first step succeeds");
        assert!(world.candidate_state_scratch.states.is_empty());
        assert_eq!(world.candidate_state_scratch.states.capacity(), capacity);
        assert_eq!(world.candidate_state_scratch.states.as_ptr(), allocation);

        world
            .step(TickInput::new(16))
            .expect("second step succeeds");

        assert!(world.candidate_state_scratch.states.is_empty());
        assert_eq!(world.candidate_state_scratch.states.capacity(), capacity);
        assert_eq!(world.candidate_state_scratch.states.as_ptr(), allocation);
    }

    #[test]
    fn candidate_state_scratch_is_restored_after_advance_failure() {
        let lane_graph = LaneGraph::try_new([LaneEdge::new(
            "A",
            EdgeLength::try_new(100.0).expect("valid edge length"),
            Vec::<String>::new(),
        )])
        .expect("valid lane graph");
        let route = Route::try_new("R1", ["A"]).expect("valid route");
        let (traffic_data, profile) = traffic_data(lane_graph, [route]);
        let vehicle = VehicleSpawnInput::active(
            "V1",
            profile,
            "R1",
            0,
            EdgeProgress::try_new(10.0).expect("valid progress"),
            Speed::try_new(10.0).expect("valid speed"),
        );
        let mut world =
            CoreWorld::with_traffic_data(1_000, traffic_data, vec![vehicle]).expect("valid world");
        let vehicle = world.vehicle_handle("V1").expect("vehicle handle exists");
        world.step_failure_after_vehicle = Some(vehicle);
        let before = world.clone();
        let capacity = world.candidate_state_scratch.states.capacity();
        let allocation = world.candidate_state_scratch.states.as_ptr();
        assert!(capacity >= world.vehicles.len());

        let first_error = world
            .step(TickInput::new(1_000))
            .expect_err("injected post-advance failure must fail");
        std::assert_matches!(
            first_error,
            CoreError::ParkingBindingInvariantViolation {
                stage: "test_after_vehicle_advance",
                vehicle: Some(actual_vehicle),
                ..
            } if actual_vehicle == vehicle
        );
        assert_eq!(world, before);
        assert!(world.candidate_state_scratch.states.is_empty());
        assert_eq!(world.candidate_state_scratch.states.capacity(), capacity);
        assert_eq!(world.candidate_state_scratch.states.as_ptr(), allocation);

        let second_error = world
            .step(TickInput::new(1_000))
            .expect_err("repeated injected failure must fail");
        std::assert_matches!(
            second_error,
            CoreError::ParkingBindingInvariantViolation {
                stage: "test_after_vehicle_advance",
                ..
            }
        );

        assert_eq!(world, before);
        assert!(world.candidate_state_scratch.states.is_empty());
        assert_eq!(world.candidate_state_scratch.states.capacity(), capacity);
        assert_eq!(world.candidate_state_scratch.states.as_ptr(), allocation);
    }

    #[test]
    fn unit_delta_mismatch_keeps_world_unchanged() {
        let lane_graph = LaneGraph::try_new([LaneEdge::new(
            "A",
            EdgeLength::try_new(10.0).expect("valid edge length"),
            Vec::<String>::new(),
        )])
        .expect("valid lane graph");
        let route = Route::try_new("R1", ["A"]).expect("valid route");
        let (traffic_data, profile) = traffic_data(lane_graph, [route]);
        let vehicle = VehicleSpawnInput::active(
            "V1",
            profile,
            "R1",
            0,
            EdgeProgress::try_new(1.0).expect("valid progress"),
            Speed::try_new(0.0).expect("valid speed"),
        );
        let mut world =
            CoreWorld::with_traffic_data(20, traffic_data, vec![vehicle]).expect("valid world");
        let before = world.clone();

        let error = world
            .step(TickInput::new(16))
            .expect_err("delta mismatch must fail");

        std::assert_matches!(
            error,
            CoreError::TickDeltaMismatch {
                expected_delta_time_ms: 20,
                actual_delta_time_ms: 16
            }
        );
        assert_eq!(world, before);
    }

    #[test]
    fn boundary_epsilon_is_not_treated_as_zero_remainder() {
        assert!(is_edge_boundary_remainder_zero(
            EDGE_BOUNDARY_TOLERANCE_METERS / 2.0
        ));
        assert!(!is_edge_boundary_remainder_zero(
            EDGE_BOUNDARY_TOLERANCE_METERS
        ));
    }

    #[test]
    fn tick_index_overflow_keeps_world_unchanged() {
        let mut world = CoreWorld::new(20).expect("valid world");
        world.tick_index = u64::MAX;
        let before = world.clone();

        let error = world
            .step(TickInput::new(20))
            .expect_err("tick index overflow must fail");

        std::assert_matches!(error, CoreError::TimeOverflow);
        assert_eq!(world, before);
    }

    #[test]
    fn time_ms_overflow_keeps_world_unchanged() {
        let mut world = CoreWorld::new(20).expect("valid world");
        world.time_ms = u64::MAX - 10;
        let before = world.clone();

        let error = world
            .step(TickInput::new(20))
            .expect_err("time overflow must fail");

        std::assert_matches!(error, CoreError::TimeOverflow);
        assert_eq!(world, before);
    }

    #[test]
    fn parking_activation_preserves_error_priority_and_makes_legacy_guard_unreachable() {
        let mut delta_world = reserved_parking_world();
        let before = delta_world.clone();
        std::assert_matches!(
            delta_world.step(TickInput::new(16)),
            Err(CoreError::TickDeltaMismatch { .. })
        );
        assert_eq!(delta_world, before);

        let mut tick_world = reserved_parking_world();
        tick_world.tick_index = u64::MAX;
        let before = tick_world.clone();
        std::assert_matches!(
            tick_world.step(TickInput::new(20)),
            Err(CoreError::TimeOverflow)
        );
        assert_eq!(tick_world, before);

        let mut time_world = reserved_parking_world();
        time_world.time_ms = u64::MAX - 10;
        let before = time_world.clone();
        std::assert_matches!(
            time_world.step(TickInput::new(20)),
            Err(CoreError::TimeOverflow)
        );
        assert_eq!(time_world, before);

        let mut integrity_world = reserved_parking_world();
        integrity_world
            .parking_runtime
            .corrupt_global_capacity_for_test();
        let before = integrity_world.clone();
        std::assert_matches!(
            integrity_world.step(TickInput::new(20)),
            Err(CoreError::ParkingBindingInvariantViolation {
                stage: "step_sentinel",
                ..
            })
        );
        assert_eq!(integrity_world, before);

        let mut capability_world = reserved_parking_world();
        let before_vehicle = capability_world
            .vehicle(capability_world.vehicle_handle("V0").expect("vehicle"))
            .expect("live vehicle")
            .clone();
        let result = capability_world
            .step(TickInput::new(20))
            .expect("#109 activation makes the legacy guard unreachable");
        assert_eq!(result.tick_index, 1);
        assert_eq!(capability_world.parking_snapshot().counts().reserved, 1);
        assert_eq!(
            capability_world
                .parking_snapshot()
                .vehicle_state(before_vehicle.handle),
            Some(crate::VehicleParkingState::Reserved {
                space: capability_world
                    .parking()
                    .space_handle("S0")
                    .expect("space"),
                approach: crate::ParkingApproachState::Approaching {
                    route: before_vehicle.route,
                    route_edge_index: 0,
                },
            })
        );
    }

    #[test]
    fn parking_arrival_is_one_shot_and_commit_excludes_vehicle_from_motion() {
        let (mut world, _) = parking_runtime_world();
        let vehicle = world.vehicle_handle("V0").expect("vehicle");
        let space = world.parking().space_handle("S0").expect("space");
        world
            .reserve_parking_space(vehicle, space)
            .expect("reservation");

        let mut arrival_event = None;
        for _ in 0..2_000 {
            let result = world.step(TickInput::new(20)).expect("approach step");
            let arrivals = result
                .events
                .iter()
                .filter_map(|event| match event {
                    CoreEvent::VehicleParkingArrivalReached(event) => Some(event.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert!(
                arrivals.len() <= 1,
                "one vehicle emits at most one arrival per tick"
            );
            if let Some(event) = arrivals.into_iter().next() {
                arrival_event = Some(event);
                break;
            }
        }

        let arrival = arrival_event.expect("vehicle must reach selected entry");
        assert_eq!(arrival.vehicle, vehicle);
        assert_eq!(arrival.space, space);
        assert_eq!(arrival.route_edge_index, 0);
        assert_eq!(
            world.parking_snapshot().vehicle_state(vehicle),
            Some(crate::VehicleParkingState::Reserved {
                space,
                approach: crate::ParkingApproachState::Arrived {
                    route: arrival.route,
                    route_edge_index: 0,
                },
            })
        );
        let arrived_state = world.vehicle(vehicle).expect("arrived vehicle").clone();
        assert_eq!(arrived_state.status, VehicleStatus::Active);
        assert_eq!(arrived_state.edge_progress.value(), 20.0);
        assert_eq!(arrived_state.current_speed, Speed::ZERO);

        let waiting = world.step(TickInput::new(20)).expect("waiting step");
        assert!(
            waiting
                .events
                .iter()
                .all(|event| !matches!(event, CoreEvent::VehicleParkingArrivalReached(_)))
        );
        assert_eq!(world.vehicle(vehicle), Some(&arrived_state));

        world
            .commit_parking(vehicle, space)
            .expect("commit parking");
        let parked_state = world.vehicle(vehicle).expect("parked vehicle").clone();
        assert_eq!(parked_state.status, VehicleStatus::Parked);
        assert_eq!(world.parking_snapshot().counts().occupied, 1);
        let parked_tick = world.step(TickInput::new(20)).expect("parked step");
        assert!(parked_tick.events.iter().all(|event| {
            !matches!(
                event,
                CoreEvent::VehicleParkingArrivalReached(_)
                    | CoreEvent::VehicleParkingStopProjectionApplied(_)
            )
        }));
        assert_eq!(world.vehicle(vehicle), Some(&parked_state));
    }

    #[test]
    fn dormant_route_completion_releases_before_completed_event_atomically() {
        let (mut world, _) = parking_runtime_world();
        let vehicle = world.vehicle_handle("V1").expect("vehicle");
        let space = world.parking().space_handle("S0").expect("space");
        world
            .reserve_parking_space(vehicle, space)
            .expect("dormant reservation");
        assert_eq!(
            world.parking_snapshot().vehicle_state(vehicle),
            Some(crate::VehicleParkingState::Reserved {
                space,
                approach: crate::ParkingApproachState::Dormant,
            })
        );

        let mut completion_events = None;
        for _ in 0..5_000 {
            let result = world.step(TickInput::new(20)).expect("dormant step");
            if world
                .vehicle(vehicle)
                .is_some_and(|state| state.status == VehicleStatus::Completed)
            {
                completion_events = Some(result.events);
                break;
            }
        }

        let events = completion_events.expect("dormant vehicle must complete route");
        let release_index = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    CoreEvent::ParkingReservationReleased(event)
                        if event.vehicle == vehicle
                            && event.space == space
                            && event.reason == ParkingReleaseReason::RouteCompleted
                )
            })
            .expect("completion release event");
        let completed_index = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    CoreEvent::VehicleCompletedRoute(event) if event.vehicle == vehicle
                )
            })
            .expect("completed event");
        assert_eq!(release_index + 1, completed_index);
        assert_eq!(world.parking_snapshot().counts().reserved, 0);
        assert_eq!(
            world.parking_snapshot().space_state(space),
            Some(ParkingSpaceState::Vacant)
        );
        assert_eq!(
            world.parking_snapshot().vehicle_state(vehicle),
            Some(crate::VehicleParkingState::Unbound)
        );
    }

    #[test]
    fn completion_release_candidate_is_discarded_on_later_step_failure_and_retry_replays() {
        let (mut world, _) = parking_runtime_world();
        let vehicle = world.vehicle_handle("V1").expect("vehicle");
        let space = world.parking().space_handle("S0").expect("space");
        let state = world.vehicles[vehicle.index()]
            .state
            .as_mut()
            .expect("vehicle state");
        state.edge_progress = EdgeProgress::try_new(199.9).expect("near route end");
        state.current_speed = Speed::try_new(10.0).expect("completion speed");
        world
            .reserve_parking_space(vehicle, space)
            .expect("dormant reservation");
        let mut fresh = world.clone();

        world.step_failure_after_vehicle = Some(vehicle);
        let before_failure = world.clone();
        let error = world
            .step(TickInput::new(20))
            .expect_err("injected post-advance failure");
        std::assert_matches!(
            error,
            CoreError::ParkingBindingInvariantViolation {
                stage: "test_after_vehicle_advance",
                vehicle: Some(actual_vehicle),
                space: Some(actual_space),
            } if actual_vehicle == vehicle && actual_space == space
        );
        assert_eq!(world, before_failure);
        assert_eq!(world.parking_snapshot().counts().reserved, 1);
        assert_eq!(
            world.parking_snapshot().space_state(space),
            Some(ParkingSpaceState::Reserved { vehicle })
        );

        world.step_failure_after_vehicle = None;
        let retry = world.step(TickInput::new(20)).expect("retry");
        let replay = fresh.step(TickInput::new(20)).expect("fresh replay");
        assert_eq!(retry, replay);
        assert_eq!(world, fresh);
        assert!(matches!(
            retry.events.as_slice(),
            [
                CoreEvent::ParkingReservationReleased(_),
                CoreEvent::VehicleCompletedRoute(_)
            ]
        ));
    }

    #[test]
    fn route_reference_equality_covers_live_count_but_ignores_heap_history() {
        let mut left = RouteReferenceIndex::default();
        let mut right = RouteReferenceIndex::default();

        left.live_count = 1;
        assert_ne!(left, right, "live reference count is authority state");

        right.live_count = 1;
        left.candidates.push(Reverse(RouteVehicleReference {
            update_order_position: 17,
            vehicle: VehicleHandle::new(3, 2),
            route_generation: 4,
        }));
        assert_eq!(left, right, "derived heap history must remain ignored");
    }

    #[test]
    fn route_in_use_uses_stable_order_after_vehicle_slot_reuse() {
        let lane_graph = LaneGraph::try_new([LaneEdge::new(
            "A",
            EdgeLength::try_new(10.0).expect("valid edge length"),
            Vec::<String>::new(),
        )])
        .expect("valid lane graph");
        let (traffic_data, profile) = traffic_data(
            lane_graph,
            [Route::try_new("R1", ["A"]).expect("valid route")],
        );
        let mut world =
            CoreWorld::with_traffic_data(20, traffic_data, Vec::new()).expect("valid world");
        let route = world.route_handle("R1").expect("route handle");
        let end = EdgeProgress::try_new(10.0).expect("valid end progress");
        let first = world
            .spawn_vehicle(VehicleSpawnInput::completed("first", profile, "R1", 0, end))
            .expect("first vehicle");
        let second = world
            .spawn_vehicle(VehicleSpawnInput::completed(
                "second", profile, "R1", 0, end,
            ))
            .expect("second vehicle");

        world.despawn_vehicle(first).expect("despawn first");
        let replacement = world
            .spawn_vehicle(VehicleSpawnInput::completed(
                "replacement",
                profile,
                "R1",
                0,
                end,
            ))
            .expect("replacement vehicle");

        assert_eq!(replacement.index(), first.index(), "slot must be reused");
        assert_eq!(
            world
                .vehicles()
                .map(|vehicle| vehicle.handle)
                .collect::<Vec<_>>(),
            vec![second, replacement]
        );
        let error = world.remove_route(route).expect_err("route remains in use");
        std::assert_matches!(
            error,
            CoreError::RouteInUse { vehicle, .. } if vehicle == second
        );
        world.assert_lifecycle_indices_consistent();
    }

    #[test]
    fn deterministic_tombstone_compaction_preserves_live_order_and_route_first() {
        let lane_graph = LaneGraph::try_new([LaneEdge::new(
            "A",
            EdgeLength::try_new(10.0).expect("valid edge length"),
            Vec::<String>::new(),
        )])
        .expect("valid lane graph");
        let (traffic_data, profile) = traffic_data(
            lane_graph,
            [Route::try_new("R1", ["A"]).expect("valid route")],
        );
        let mut world =
            CoreWorld::with_traffic_data(20, traffic_data, Vec::new()).expect("valid world");
        let route = world.route_handle("R1").expect("route handle");
        let end = EdgeProgress::try_new(10.0).expect("valid end progress");
        let mut handles = Vec::new();
        for index in 0..130 {
            handles.push(
                world
                    .spawn_vehicle(VehicleSpawnInput::completed(
                        format!("V{index:03}"),
                        profile,
                        "R1",
                        0,
                        end,
                    ))
                    .expect("vehicle spawns"),
            );
        }
        for handle in handles.iter().take(65).copied() {
            world.despawn_vehicle(handle).expect("vehicle despawns");
        }

        assert_eq!(world.vehicle_update_order.tombstones, 0);
        assert_eq!(world.vehicle_update_order.entries.len(), 65);
        assert_eq!(
            world
                .vehicles()
                .map(|vehicle| vehicle.handle)
                .collect::<Vec<_>>(),
            handles[65..]
        );
        let error = world.remove_route(route).expect_err("route remains in use");
        std::assert_matches!(
            error,
            CoreError::RouteInUse { vehicle, .. } if vehicle == handles[65]
        );
        world.assert_lifecycle_indices_consistent();
    }

    proptest! {
        #[test]
        fn command_spatial_overlap_matches_full_scan_oracle(
            route_case in 0_usize..10,
            progress_value in 0_u8..=20,
            stopped in any::<bool>(),
        ) {
            let lane_graph = LaneGraph::try_new([
                LaneEdge::new("A", EdgeLength::try_new(20.0).expect("length"), ["B", "C"]),
                LaneEdge::new("B", EdgeLength::try_new(20.0).expect("length"), ["D"]),
                LaneEdge::new("C", EdgeLength::try_new(20.0).expect("length"), ["D"]),
                LaneEdge::new("D", EdgeLength::try_new(20.0).expect("length"), ["A"]),
            ])
            .expect("valid cyclic graph");
            let routes = [
                Route::try_new("R0", ["A", "B", "D", "A", "C", "D"]).expect("R0"),
                Route::try_new("R1", ["C", "D", "A", "B"]).expect("R1"),
            ];
            let (traffic_data, profile) = traffic_data(lane_graph, routes);
            let vehicles = vec![
                VehicleSpawnInput::active(
                    "existing-a",
                    profile,
                    "R0",
                    0,
                    EdgeProgress::try_new(2.0).expect("progress"),
                    Speed::ZERO,
                ),
                VehicleSpawnInput::stopped(
                    "existing-b",
                    profile,
                    "R0",
                    1,
                    EdgeProgress::try_new(9.0).expect("progress"),
                ),
                VehicleSpawnInput::active(
                    "existing-d",
                    profile,
                    "R0",
                    2,
                    EdgeProgress::try_new(15.0).expect("progress"),
                    Speed::ZERO,
                ),
                VehicleSpawnInput::stopped(
                    "existing-c",
                    profile,
                    "R1",
                    0,
                    EdgeProgress::try_new(11.0).expect("progress"),
                ),
            ];
            let mut world = CoreWorld::with_traffic_data(20, traffic_data, vehicles)
                .expect("oracle world");
            let cases = [
                ("R0", 0),
                ("R0", 1),
                ("R0", 2),
                ("R0", 3),
                ("R0", 4),
                ("R0", 5),
                ("R1", 0),
                ("R1", 1),
                ("R1", 2),
                ("R1", 3),
            ];
            let (route_id, route_edge_index) = cases[route_case];
            let progress = EdgeProgress::try_new(f64::from(progress_value)).expect("progress");
            let input = if stopped {
                VehicleSpawnInput::stopped(
                    "candidate",
                    profile,
                    route_id,
                    route_edge_index,
                    progress,
                )
            } else {
                VehicleSpawnInput::active(
                    "candidate",
                    profile,
                    route_id,
                    route_edge_index,
                    progress,
                    Speed::ZERO,
                )
            };
            let route = world.route_handle(route_id).expect("route handle");
            let normalized = world.normalize_vehicle_input(route, &input).expect("normalized");
            let expected = format!(
                "{:?}",
                world.validate_candidate_overlap_full_scan(route, &input.id, &normalized)
            );
            let actual = format!(
                "{:?}",
                world.validate_candidate_overlap(route, &input.id, &normalized)
            );

            prop_assert_eq!(actual, expected);
            world.assert_lifecycle_indices_consistent();
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        #[test]
        fn parking_leave_local_follower_query_matches_full_scan_oracle(
            follower_progress in 0_u8..=100,
            follower_speed in 0_u8..=20,
            stopped in any::<bool>(),
        ) {
            let (mut world, profile) = parking_runtime_world();
            for id in ["V0", "V1"] {
                let vehicle = world.vehicle_handle(id).expect("seed vehicle");
                world.despawn_vehicle(vehicle).expect("remove seed vehicle");
            }
            let space = world.parking().space_handle("S0").expect("space");
            let route = world.route_handle("R").expect("route");
            let parked = world
                .spawn_parked_vehicle(ParkedVehicleSpawnInput {
                    id: "parked".to_owned(),
                    profile,
                    route_id: "R".to_owned(),
                    route_edge_index: 0,
                    space,
                })
                .expect("parked vehicle")
                .vehicle;
            let progress = EdgeProgress::try_new(f64::from(follower_progress)).expect("progress");
            let follower_input = if stopped {
                VehicleSpawnInput::stopped("follower", profile, "R", 0, progress)
            } else {
                VehicleSpawnInput::active(
                    "follower",
                    profile,
                    "R",
                    0,
                    progress,
                    Speed::try_from(f64::from(follower_speed)).expect("speed"),
                )
            };
            world.spawn_vehicle(follower_input).expect("follower");
            let candidate = NormalizedVehicleInput {
                profile,
                route_edge_index: 0,
                edge_progress: EdgeProgress::try_new(40.0).expect("exit progress"),
                current_speed: Speed::ZERO,
                status: VehicleStatus::Active,
            };
            let expected = format!(
                "{:?}",
                world.validate_parking_leave_followers_full_scan(
                    parked,
                    space,
                    route,
                    0,
                    &candidate,
                )
            );
            let actual = format!(
                "{:?}",
                world.validate_parking_leave_followers(parked, space, route, 0, &candidate)
            );

            prop_assert_eq!(actual, expected);
            world.assert_lifecycle_indices_consistent();
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn parking_leave_follower_oracle_covers_adjacent_and_repeated_occurrences(
            follower_case in 0_usize..8,
            progress_seed in 0_u8..=99,
            follower_speed in 0_u8..=40,
            stopped in any::<bool>(),
        ) {
            let lane_graph = LaneGraph::try_new([
                LaneEdge::new("A", EdgeLength::try_new(100.0).expect("length"), ["B", "C"]),
                LaneEdge::new("B", EdgeLength::try_new(100.0).expect("length"), ["D"]),
                LaneEdge::new("C", EdgeLength::try_new(100.0).expect("length"), ["D"]),
                LaneEdge::new("D", EdgeLength::try_new(100.0).expect("length"), ["A"]),
            ])
            .expect("cyclic parking graph");
            let parking = ParkingRegistry::try_new(
                &lane_graph,
                [],
                [ParkingSpaceInput::new(
                    "S",
                    None,
                    "A",
                    10.0,
                    "A",
                    20.0,
                    ParkingSpaceGeometryInput::new(-3.0, 0.0, 4.5, 2.4),
                )],
            )
            .expect("cyclic parking registry");
            let (base, profile) = traffic_data(
                lane_graph,
                [
                    Route::try_new("R0", ["A", "B", "D", "A", "C", "D"])
                        .expect("R0"),
                    Route::try_new("R1", ["C", "D", "A", "B"]).expect("R1"),
                ],
            );
            let traffic = InitialTrafficData::try_new_with_signals_and_parking(
                base.lane_graph().clone(),
                base.routes().iter().cloned(),
                base.vehicle_profiles().clone(),
                SignalRegistry::empty(),
                parking,
            )
            .expect("cyclic parking traffic");
            let mut world = CoreWorld::with_traffic_data(1_000, traffic, Vec::new())
                .expect("cyclic parking world");
            let space = world.parking().space_handle("S").expect("space");
            let route = world.route_handle("R0").expect("R0");
            let parked = world
                .spawn_parked_vehicle(ParkedVehicleSpawnInput {
                    id: "parked".to_owned(),
                    profile,
                    route_id: "R0".to_owned(),
                    route_edge_index: 0,
                    space,
                })
                .expect("parked vehicle")
                .vehicle;
            let cases = [
                ("R0", 0),
                ("R0", 2),
                ("R0", 3),
                ("R0", 5),
                ("R1", 0),
                ("R1", 1),
                ("R1", 2),
                ("R1", 3),
            ];
            let (route_id, route_edge_index) = cases[follower_case];
            let edge_id = world
                .lane_graph()
                .edge_external_id(
                    world.routes[world.route_handle(route_id).expect("route").index()]
                        .edge_handles[route_edge_index],
                )
                .expect("edge id");
            let progress = if matches!(edge_id, "B" | "C" | "D") {
                90.0 + f64::from(progress_seed % 10)
            } else {
                f64::from(progress_seed)
            };
            let progress = EdgeProgress::try_new(progress).expect("progress");
            let follower_input = if stopped {
                VehicleSpawnInput::stopped(
                    "follower",
                    profile,
                    route_id,
                    route_edge_index,
                    progress,
                )
            } else {
                VehicleSpawnInput::active(
                    "follower",
                    profile,
                    route_id,
                    route_edge_index,
                    progress,
                    Speed::try_from(f64::from(follower_speed)).expect("speed"),
                )
            };
            world.spawn_vehicle(follower_input).expect("follower");
            let candidate = NormalizedVehicleInput {
                profile,
                route_edge_index: 0,
                edge_progress: EdgeProgress::try_new(20.0).expect("exit progress"),
                current_speed: Speed::ZERO,
                status: VehicleStatus::Active,
            };
            let expected = format!(
                "{:?}",
                world.validate_parking_leave_followers_full_scan(
                    parked,
                    space,
                    route,
                    0,
                    &candidate,
                )
            );
            let actual = format!(
                "{:?}",
                world.validate_parking_leave_followers(parked, space, route, 0, &candidate)
            );

            prop_assert_eq!(actual, expected);
            world.assert_lifecycle_indices_consistent();
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn parking_reservation_commands_match_model_and_replay_deterministically(
            operations in prop::collection::vec(any::<u8>(), 1..=128),
        ) {
            let (mut world, _) = parking_runtime_world();
            let mut replay = world.clone();
            let vehicles = [
                world.vehicle_handle("V0").expect("V0"),
                world.vehicle_handle("V1").expect("V1"),
            ];
            let spaces = [
                world.parking().space_handle("S0").expect("S0"),
                world.parking().space_handle("S1").expect("S1"),
            ];
            let mut vehicle_spaces = [None; 2];
            let mut space_vehicles = [None; 2];

            for operation in operations {
                let vehicle_index = usize::from(operation & 1);
                let space_index = usize::from((operation >> 1) & 1);
                let vehicle = vehicles[vehicle_index];
                let space = spaces[space_index];
                if operation & 4 == 0 {
                    let actual = world.reserve_parking_space(vehicle, space);
                    let replayed = replay.reserve_parking_space(vehicle, space);
                    prop_assert_eq!(format!("{actual:?}"), format!("{replayed:?}"));
                    if vehicle_spaces[vehicle_index] == Some(space_index) {
                        prop_assert_eq!(
                            actual.expect("exact reservation retry").effect,
                            ParkingCommandEffect::AlreadySatisfied
                        );
                    } else if let Some(current_space) = vehicle_spaces[vehicle_index] {
                        assert!(matches!(
                            actual,
                            Err(CoreError::ParkingVehicleAlreadyBound {
                                current_space: actual_space,
                                ..
                            }) if actual_space == spaces[current_space]
                        ));
                    } else if let Some(current_vehicle) = space_vehicles[space_index] {
                        assert!(matches!(
                            actual,
                            Err(CoreError::ParkingSpaceUnavailable {
                                current_vehicle: actual_vehicle,
                                ..
                            }) if actual_vehicle == vehicles[current_vehicle]
                        ));
                    } else {
                        prop_assert_eq!(
                            actual.expect("vacant reservation").effect,
                            ParkingCommandEffect::Applied
                        );
                        vehicle_spaces[vehicle_index] = Some(space_index);
                        space_vehicles[space_index] = Some(vehicle_index);
                    }
                } else {
                    let actual = world.cancel_parking_reservation(vehicle, space);
                    let replayed = replay.cancel_parking_reservation(vehicle, space);
                    prop_assert_eq!(format!("{actual:?}"), format!("{replayed:?}"));
                    if vehicle_spaces[vehicle_index] == Some(space_index) {
                        prop_assert_eq!(
                            actual.expect("exact cancellation").effect,
                            ParkingCommandEffect::Applied
                        );
                        vehicle_spaces[vehicle_index] = None;
                        space_vehicles[space_index] = None;
                    } else if vehicle_spaces[vehicle_index].is_none()
                        && space_vehicles[space_index].is_none()
                    {
                        prop_assert_eq!(
                            actual.expect("vacant cancellation retry").effect,
                            ParkingCommandEffect::AlreadySatisfied
                        );
                    } else {
                        assert!(matches!(
                            actual,
                            Err(CoreError::ParkingReservationMismatch { .. })
                        ));
                    }
                }

                prop_assert_eq!(&world, &replay);
                let expected_reserved = vehicle_spaces.iter().flatten().count();
                let counts = world.parking_snapshot().counts();
                prop_assert_eq!(counts.reserved, expected_reserved);
                prop_assert_eq!(counts.vacant, spaces.len() - expected_reserved);
                prop_assert_eq!(counts.occupied, 0);
                for (space_index, space) in spaces.iter().copied().enumerate() {
                    let expected = space_vehicles[space_index].map_or(
                        ParkingSpaceState::Vacant,
                        |vehicle_index| ParkingSpaceState::Reserved {
                            vehicle: vehicles[vehicle_index],
                        },
                    );
                    prop_assert_eq!(world.parking_snapshot().space_state(space), Some(expected));
                }
                for (vehicle_index, vehicle) in vehicles.iter().copied().enumerate() {
                    match vehicle_spaces[vehicle_index] {
                        Some(space_index) => assert!(matches!(
                            world.parking_snapshot().vehicle_state(vehicle),
                            Some(VehicleParkingState::Reserved { space, .. })
                                if space == spaces[space_index]
                        )),
                        None => prop_assert_eq!(
                            world.parking_snapshot().vehicle_state(vehicle),
                            Some(VehicleParkingState::Unbound)
                        ),
                    }
                }
                world.assert_lifecycle_indices_consistent();
                replay.assert_lifecycle_indices_consistent();
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn lifecycle_order_and_route_references_match_vec_model(
            operations in prop::collection::vec(any::<u8>(), 1..=128),
        ) {
            let lane_graph = LaneGraph::try_new([LaneEdge::new(
                "A",
                EdgeLength::try_new(100.0).expect("length"),
                Vec::<String>::new(),
            )])
            .expect("graph");
            let (traffic_data, profile) = traffic_data(
                lane_graph,
                [
                    Route::try_new("R0", ["A"]).expect("R0"),
                    Route::try_new("R1", ["A"]).expect("R1"),
                ],
            );
            let mut world = CoreWorld::with_traffic_data(20, traffic_data, Vec::new())
                .expect("world");
            let routes = [
                world.route_handle("R0").expect("R0 handle"),
                world.route_handle("R1").expect("R1 handle"),
            ];
            let end = EdgeProgress::try_new(100.0).expect("end progress");
            let mut model = Vec::<(usize, VehicleHandle, usize)>::new();
            let mut last_handles = [None; 16];

            for operation in operations {
                let id_index = usize::from(operation) % last_handles.len();
                let route_index = id_index % routes.len();
                let id = format!("V{id_index:02}");
                if operation % 3 != 2 {
                    let before = world.clone();
                    let result = world.spawn_vehicle(VehicleSpawnInput::completed(
                        id.clone(),
                        profile,
                        format!("R{route_index}"),
                        0,
                        end,
                    ));
                    if let Some((_, expected, _)) =
                        model.iter().find(|(candidate, _, _)| *candidate == id_index)
                    {
                        let error = result.expect_err("duplicate model vehicle");
                        std::assert_matches!(
                            error,
                            CoreError::DuplicateVehicleId { vehicle_id } if vehicle_id == id
                        );
                        assert_eq!(world, before);
                        assert_eq!(world.vehicle_handle(&id), Some(*expected));
                    } else {
                        let handle = result.expect("model spawn");
                        last_handles[id_index] = Some(handle);
                        model.push((id_index, handle, route_index));
                    }
                } else if let Some(position) = model
                    .iter()
                    .position(|(candidate, _, _)| *candidate == id_index)
                {
                    let (_, handle, expected_route) = model.remove(position);
                    let record = world.despawn_vehicle(handle).expect("model despawn");
                    assert_eq!(record.handle, handle);
                    assert_eq!(record.external_id, id);
                    assert_eq!(record.route, routes[expected_route]);
                    assert_eq!(record.status, VehicleStatus::Completed);
                    assert_eq!(world.vehicle(handle), None);
                } else {
                    let stale = last_handles[id_index]
                        .unwrap_or_else(|| VehicleHandle::new(1_000 + id_index, 0));
                    let before = world.clone();
                    let error = world
                        .despawn_vehicle(stale)
                        .expect_err("missing model vehicle");
                    std::assert_matches!(
                        error,
                        CoreError::UnknownVehicleHandle { vehicle } if vehicle == stale
                    );
                    assert_eq!(world, before);
                }

                let expected_order = model
                    .iter()
                    .map(|(_, handle, _)| *handle)
                    .collect::<Vec<_>>();
                assert_eq!(
                    world
                        .vehicles()
                        .map(|vehicle| vehicle.handle)
                        .collect::<Vec<_>>(),
                    expected_order
                );
                for (model_id, handle, _) in &model {
                    assert_eq!(world.vehicle_handle(&format!("V{model_id:02}")), Some(*handle));
                }
                for (route_index, route) in routes.iter().copied().enumerate() {
                    if let Some((_, expected, _)) = model
                        .iter()
                        .find(|(_, _, candidate_route)| *candidate_route == route_index)
                    {
                        let error = world
                            .remove_route(route)
                            .expect_err("referenced model route");
                        std::assert_matches!(
                            error,
                            CoreError::RouteInUse { vehicle, .. } if vehicle == *expected
                        );
                    }
                }
                world.assert_lifecycle_indices_consistent();
            }
        }
    }

    #[test]
    fn lifecycle_10k_tombstone_and_stale_high_water_compacts_deterministically() {
        let mut world = lifecycle_scale_world(10_000);
        let handles = world
            .vehicles()
            .map(|vehicle| vehicle.handle)
            .collect::<Vec<_>>();
        let initial = world.lifecycle_retained_stats();
        assert_eq!(initial.live_vehicles, 10_000);
        assert_eq!(
            initial.route_occurrences,
            10_000_usize.div_ceil(SCALE_VEHICLES_PER_EDGE)
        );
        assert_eq!(initial.route_candidate_nodes, 10_000);
        assert_eq!(initial.stale_route_candidate_nodes, 0);
        assert_eq!(initial.spatial_occupants, 10_000);

        for handle in handles.iter().take(4_999).copied() {
            world
                .despawn_vehicle(handle)
                .expect("pre-threshold despawn");
        }
        let high_water = world.lifecycle_retained_stats();
        assert_eq!(high_water.live_vehicles, 5_001);
        assert_eq!(high_water.tombstones, 4_999);
        assert_eq!(high_water.route_candidate_nodes, 10_000);
        assert_eq!(high_water.stale_route_candidate_nodes, 4_999);
        assert_eq!(high_water.spatial_occupants, 5_001);

        world
            .despawn_vehicle(handles[4_999])
            .expect("threshold despawn");
        let compacted = world.lifecycle_retained_stats();
        assert_eq!(compacted.live_vehicles, 5_000);
        assert_eq!(compacted.tombstones, 0);
        assert_eq!(compacted.route_candidate_nodes, 5_000);
        assert_eq!(compacted.stale_route_candidate_nodes, 0);
        assert_eq!(compacted.spatial_occupants, 5_000);
        assert!(compacted.accounted_bytes >= compacted.live_vehicles);
        world.assert_lifecycle_indices_consistent();
    }

    #[test]
    fn spatial_operation_counts_depend_on_local_k_not_background_v() {
        let run = |background_count: usize| {
            let (mut world, profile) = sparse_command_world(background_count);
            let route = world.route_handle("R").expect("route");
            let progress = EdgeProgress::try_new(5.0).expect("progress");
            let input =
                VehicleSpawnInput::active("candidate", profile, "R", 0, progress, Speed::ZERO);
            let normalized = world
                .normalize_vehicle_input(route, &input)
                .expect("normalized");
            world
                .validate_candidate_overlap(route, &input.id, &normalized)
                .expect_err("local overlap");
            let overlap_stats = world.command_spatial_index.query_stats();

            let candidate_edge = world.routes[route.index()].edge_handles[0];
            let mut spatial = std::mem::take(&mut world.command_spatial_index);
            let mut resolve_progress = |handle| {
                world
                    .vehicle(handle)
                    .expect("spatial occupant")
                    .edge_progress
                    .value()
            };
            spatial.gather_direct_follower_candidates(
                candidate_edge,
                progress.value(),
                f64::from(
                    world
                        .vehicle_profile(profile)
                        .expect("profile")
                        .iidm()
                        .length,
                ),
                world.vehicles.len(),
                &mut resolve_progress,
            );
            let direct_followers = spatial.candidates().to_vec();
            let direct_stats = spatial.query_stats();
            world.command_spatial_index = spatial;
            (
                overlap_stats,
                direct_stats,
                direct_followers,
                world.vehicle_handle("local").expect("local handle"),
            )
        };

        let small = run(128);
        let large = run(10_000);
        assert_eq!(small.0, large.0);
        assert_eq!(small.1, large.1);
        assert_eq!(small.0.edge_ranges, 2);
        assert_eq!(small.0.occupants_visited, 2);
        assert_eq!(small.1.edge_ranges, 1);
        assert_eq!(small.1.occupants_visited, 1);
        assert_eq!(small.2, vec![small.3]);
        assert_eq!(large.2, vec![large.3]);
    }

    #[test]
    fn parking_leave_follower_query_counts_depend_on_local_k_not_background_v() {
        let run = |background_count: usize| {
            let background_edge_count = background_count.div_ceil(SCALE_VEHICLES_PER_EDGE);
            let (lane_graph, edge_ids) =
                scale_chain_graph("parking-query-edge", background_edge_count + 1);
            let parking_edge_id = edge_ids[0].clone();
            let parking = ParkingRegistry::try_new(
                &lane_graph,
                [],
                [ParkingSpaceInput::new(
                    "S",
                    None,
                    parking_edge_id.clone(),
                    20.0,
                    parking_edge_id,
                    40.0,
                    ParkingSpaceGeometryInput::new(-3.0, 0.0, 4.5, 2.4),
                )],
            )
            .expect("parking scale registry");
            let (base, profile) = traffic_data(
                lane_graph,
                [Route::try_new("R", edge_ids).expect("parking scale route")],
            );
            let traffic = InitialTrafficData::try_new_with_signals_and_parking(
                base.lane_graph().clone(),
                base.routes().iter().cloned(),
                base.vehicle_profiles().clone(),
                SignalRegistry::empty(),
                parking,
            )
            .expect("parking scale traffic");
            let mut vehicles: Vec<_> = (0..background_count)
                .map(|index| {
                    let route_edge_index = 1 + index / SCALE_VEHICLES_PER_EDGE;
                    let local_index = index % SCALE_VEHICLES_PER_EDGE;
                    VehicleSpawnInput::active(
                        format!("B{index:06}"),
                        profile,
                        "R",
                        route_edge_index,
                        EdgeProgress::try_new(1_000.0 + 8.0 * local_index as f64)
                            .expect("background progress"),
                        Speed::ZERO,
                    )
                })
                .collect::<Vec<_>>();
            vehicles.push(VehicleSpawnInput::active(
                "local",
                profile,
                "R",
                0,
                EdgeProgress::try_new(35.4).expect("local progress"),
                Speed::try_new(10.0).expect("local speed"),
            ));
            let mut world =
                CoreWorld::with_traffic_data(20, traffic, vehicles).expect("parking scale world");
            let space = world.parking().space_handle("S").expect("space");
            let route = world.route_handle("R").expect("route");
            let parked = world
                .spawn_parked_vehicle(ParkedVehicleSpawnInput {
                    id: "parked".to_owned(),
                    profile,
                    route_id: "R".to_owned(),
                    route_edge_index: 0,
                    space,
                })
                .expect("parked spawn")
                .vehicle;
            let candidate = NormalizedVehicleInput {
                profile,
                route_edge_index: 0,
                edge_progress: EdgeProgress::try_new(40.0).expect("exit progress"),
                current_speed: Speed::ZERO,
                status: VehicleStatus::Active,
            };
            std::assert_matches!(
                world.validate_parking_leave_followers(parked, space, route, 0, &candidate),
                Err(CoreError::ParkingLeaveUnsafeFollower { .. })
            );
            world.command_spatial_index.query_stats()
        };

        let small = run(128);
        let large = run(10_000);
        assert_eq!(small, large);
        assert_eq!(small.edge_ranges, 1);
        assert_eq!(small.occupants_visited, 1);
    }

    #[test]
    fn parking_leave_stale_max_speed_profile_exposes_reverse_horizon_work() {
        let run = |background_count: usize| {
            let background_edge_count = background_count.div_ceil(SCALE_VEHICLES_PER_EDGE);
            let route_edge_count = background_edge_count + 1;
            let edge_ids = (0..route_edge_count)
                .map(|index| format!("pathological-edge-{index:06}"))
                .collect::<Vec<_>>();
            let mut edges = edge_ids
                .iter()
                .enumerate()
                .map(|(index, edge_id)| {
                    LaneEdge::new(
                        edge_id.clone(),
                        EdgeLength::try_new(10_000.0).expect("pathological edge length"),
                        edge_ids.get(index + 1).into_iter().cloned(),
                    )
                })
                .collect::<Vec<_>>();
            edges.push(LaneEdge::new(
                "C",
                EdgeLength::try_new(100.0).expect("fast edge length"),
                Vec::<String>::new(),
            ));
            let lane_graph = LaneGraph::try_new(edges).expect("pathological graph");
            let parking_edge_id = edge_ids
                .last()
                .expect("parking route has a terminal edge")
                .clone();
            let parking_route_edge_index = edge_ids.len() - 1;
            let exit_progress = 9_990.0;
            let parking = ParkingRegistry::try_new(
                &lane_graph,
                [],
                [ParkingSpaceInput::new(
                    "S",
                    None,
                    parking_edge_id.clone(),
                    exit_progress - 10.0,
                    parking_edge_id,
                    exit_progress,
                    ParkingSpaceGeometryInput::new(-3.0, 0.0, 4.5, 2.4),
                )],
            )
            .expect("pathological parking registry");
            let (base, profile) = traffic_data(
                lane_graph,
                [
                    Route::try_new("R", edge_ids).expect("pathological route"),
                    Route::try_new("fast-route", ["C"]).expect("fast route"),
                ],
            );
            let traffic = InitialTrafficData::try_new_with_signals_and_parking(
                base.lane_graph().clone(),
                base.routes().iter().cloned(),
                base.vehicle_profiles().clone(),
                SignalRegistry::empty(),
                parking,
            )
            .expect("pathological traffic");
            let mut vehicles: Vec<_> = (0..background_count)
                .map(|index| {
                    let route_edge_index = index / SCALE_VEHICLES_PER_EDGE;
                    let local_index = index % SCALE_VEHICLES_PER_EDGE;
                    VehicleSpawnInput::active(
                        format!("B{index:06}"),
                        profile,
                        "R",
                        route_edge_index,
                        EdgeProgress::try_new(5.0 + 10.0 * local_index as f64)
                            .expect("background progress"),
                        Speed::ZERO,
                    )
                })
                .collect();
            vehicles.push(VehicleSpawnInput::active(
                "fast-1",
                profile,
                "fast-route",
                0,
                EdgeProgress::try_new(50.0).expect("fast progress"),
                Speed::try_new(100.0).expect("fast speed"),
            ));
            vehicles.push(VehicleSpawnInput::active(
                "fast-2",
                profile,
                "fast-route",
                0,
                EdgeProgress::try_new(70.0).expect("second fast progress"),
                Speed::try_new(90.0).expect("second fast speed"),
            ));
            let mut world =
                CoreWorld::with_traffic_data(20, traffic, vehicles).expect("pathological world");
            let fast = world.vehicle_handle("fast-1").expect("fast vehicle");
            world
                .despawn_vehicle(fast)
                .expect("removing the fastest vehicle refreshes command max speed");
            let second_fast = world.vehicle_handle("fast-2").expect("second fast vehicle");
            world
                .despawn_vehicle(second_fast)
                .expect("same command batch reuses the exact max heap");
            let space = world.parking().space_handle("S").expect("space");
            let route = world.route_handle("R").expect("route");
            let parked = world
                .spawn_parked_vehicle(ParkedVehicleSpawnInput {
                    id: "parked".to_owned(),
                    profile,
                    route_id: "R".to_owned(),
                    route_edge_index: parking_route_edge_index,
                    space,
                })
                .expect("parked spawn")
                .vehicle;
            let candidate = NormalizedVehicleInput {
                profile,
                route_edge_index: parking_route_edge_index,
                edge_progress: EdgeProgress::try_new(exit_progress).expect("exit progress"),
                current_speed: Speed::ZERO,
                status: VehicleStatus::Active,
            };
            world
                .validate_parking_leave_followers(
                    parked,
                    space,
                    route,
                    parking_route_edge_index,
                    &candidate,
                )
                .expect("zero-speed direct follower remains safe");
            (
                world.command_spatial_index.query_stats(),
                world.command_spatial_index.speed_heap_rebuilds(),
            )
        };

        let small = run(128);
        let large = run(10_000);
        assert_eq!(small, large);
        assert_eq!(small.0.edge_ranges, 1);
        assert_eq!(small.0.occupants_visited, 0);
        assert_eq!(small.1, 1, "one command batch builds the max heap once");
    }

    #[test]
    #[ignore = "100k retained-memory scaling is an explicit G3 validation"]
    fn lifecycle_retained_memory_10k_to_100k_is_linear() {
        let small = lifecycle_scale_world(10_000).lifecycle_retained_stats();
        let large = lifecycle_scale_world(100_000).lifecycle_retained_stats();
        assert_eq!(small.route_occurrences, large.route_occurrences);
        assert_eq!(small.tombstones, 0);
        assert_eq!(large.tombstones, 0);
        assert_eq!(small.stale_route_candidate_nodes, 0);
        assert_eq!(large.stale_route_candidate_nodes, 0);
        assert_eq!(small.spatial_occupants, 10_000);
        assert_eq!(large.spatial_occupants, 100_000);
        assert!(
            large.accounted_bytes <= small.accounted_bytes * 12,
            "retained bytes must scale <=12x: small={small:?}, large={large:?}"
        );
        eprintln!(
            "retained_memory small_bytes={} small_bytes_per_live={:.2} large_bytes={} large_bytes_per_live={:.2} ratio={:.4}",
            small.accounted_bytes,
            small.accounted_bytes as f64 / small.live_vehicles as f64,
            large.accounted_bytes,
            large.accounted_bytes as f64 / large.live_vehicles as f64,
            large.accounted_bytes as f64 / small.accounted_bytes as f64,
        );
    }

    #[test]
    #[ignore = "10k component memory is an explicit #122 research measurement"]
    fn numeric_component_memory_baseline_10k() {
        let mut world = lifecycle_scale_world(10_000);
        world
            .step(TickInput::new(world.fixed_delta_time_ms()))
            .expect("component memory warm-up step");
        let stats = world.lifecycle_retained_stats();
        assert_eq!(stats.live_vehicles, 10_000);
        assert!(stats.expanded_accounted_bytes >= stats.accounted_bytes);
        assert!(stats.complete_accounted_bytes >= stats.expanded_accounted_bytes);
        eprintln!(
            "numeric_component_memory live={} accounted_bytes={} expanded_accounted_bytes={} complete_accounted_bytes={} owned_heap_bytes={} world_inline_bytes={} lane_graph_bytes={} vehicle_profile_registry_bytes={} signal_registry_bytes={} signal_runtime_state_bytes={} signal_runtime_scratch_bytes={} route_bytes={} route_distance_bytes={} route_reference_bytes={} vehicle_bytes={} resolver_bytes={} free_list_bytes={} vehicle_order_bytes={} candidate_state_bytes={} parking_bytes={} parking_registry_runtime_bytes={} occupancy_scratch_bytes={} longitudinal_scratch_bytes={} command_spatial_bytes={} lane_graph_inline_size={} vehicle_profile_registry_inline_size={} signal_registry_inline_size={} signal_runtime_state_inline_size={} signal_runtime_scratch_inline_size={} vehicle_state_size={} vehicle_slot_size={}",
            stats.live_vehicles,
            stats.accounted_bytes,
            stats.expanded_accounted_bytes,
            stats.complete_accounted_bytes,
            stats.owned_heap_bytes,
            stats.world_inline_bytes,
            stats.lane_graph_bytes,
            stats.vehicle_profile_registry_bytes,
            stats.signal_registry_bytes,
            stats.signal_runtime_state_bytes,
            stats.signal_runtime_scratch_bytes,
            stats.route_bytes,
            stats.route_distance_bytes,
            stats.route_reference_bytes,
            stats.vehicle_bytes,
            stats.resolver_bytes,
            stats.free_list_bytes,
            stats.vehicle_order_bytes,
            stats.candidate_state_bytes,
            stats.parking_bytes,
            stats.parking_registry_runtime_bytes,
            stats.occupancy_scratch_bytes,
            stats.longitudinal_scratch_bytes,
            stats.command_spatial_bytes,
            stats.lane_graph_inline_size,
            stats.vehicle_profile_registry_inline_size,
            stats.signal_registry_inline_size,
            stats.signal_runtime_state_inline_size,
            stats.signal_runtime_scratch_inline_size,
            stats.vehicle_state_size,
            stats.vehicle_slot_size,
        );
    }

    #[test]
    fn complete_retained_accountant_sums_each_unique_owner_once() {
        let mut world = lifecycle_scale_world(128);
        world
            .step(TickInput::new(world.fixed_delta_time_ms()))
            .expect("retained accountant warm-up step");
        let stats = world.lifecycle_retained_stats();
        let expected_heap_bytes = stats.lane_graph_bytes
            + stats.vehicle_profile_registry_bytes
            + stats.signal_registry_bytes
            + stats.signal_runtime_state_bytes
            + stats.signal_runtime_scratch_bytes
            + stats.route_bytes
            + stats.vehicle_bytes
            + stats.resolver_bytes
            + stats.free_list_bytes
            + stats.vehicle_order_bytes
            + stats.candidate_state_bytes
            + stats.parking_registry_runtime_bytes
            + stats.occupancy_scratch_bytes
            + stats.longitudinal_scratch_bytes
            + stats.command_spatial_bytes;

        assert_eq!(stats.owned_heap_bytes, expected_heap_bytes);
        assert_eq!(
            stats.complete_accounted_bytes,
            stats.world_inline_bytes + expected_heap_bytes
        );
        assert_eq!(stats.world_inline_bytes, std::mem::size_of::<CoreWorld>());
        assert_eq!(
            stats.lane_graph_inline_size,
            std::mem::size_of::<LaneGraph>()
        );
        assert_eq!(
            stats.vehicle_profile_registry_inline_size,
            std::mem::size_of::<VehicleProfileRegistry>()
        );
        assert_eq!(
            stats.signal_registry_inline_size,
            std::mem::size_of::<SignalRegistry>()
        );
        assert_eq!(
            stats.signal_runtime_state_inline_size,
            std::mem::size_of::<SignalRuntimeState>()
        );
        assert_eq!(
            stats.signal_runtime_scratch_inline_size,
            std::mem::size_of::<SignalRuntimeScratch>()
        );
        assert!(stats.lane_graph_bytes > 0);
        assert!(stats.vehicle_profile_registry_bytes > 0);
    }

    #[test]
    #[ignore = "100k Parking retained-memory scaling is an explicit G3 validation"]
    fn parking_retained_memory_10k_to_100k_is_linear() {
        let small = parking_retained_scale_world(10_000).lifecycle_retained_stats();
        let large = parking_retained_scale_world(100_000).lifecycle_retained_stats();
        assert!(small.parking_bytes > 0);
        assert!(
            large.parking_bytes <= small.parking_bytes * 12,
            "Parking retained bytes must scale <=12x: small={small:?}, large={large:?}"
        );
        eprintln!(
            "parking_retained_memory small_bytes={} large_bytes={} ratio={:.4}",
            small.parking_bytes,
            large.parking_bytes,
            large.parking_bytes as f64 / small.parking_bytes as f64,
        );
    }

    #[test]
    fn command_spatial_membership_follows_committed_physical_edge_transition() {
        let lane_graph = LaneGraph::try_new([
            LaneEdge::new("A", EdgeLength::try_new(10.0).expect("length"), ["B"]),
            LaneEdge::new(
                "B",
                EdgeLength::try_new(100.0).expect("length"),
                Vec::<String>::new(),
            ),
        ])
        .expect("graph");
        let (traffic_data, profile) = traffic_data(
            lane_graph,
            [Route::try_new("R1", ["A", "B"]).expect("route")],
        );
        let vehicle = VehicleSpawnInput::active(
            "existing",
            profile,
            "R1",
            0,
            EdgeProgress::try_new(9.9).expect("progress"),
            Speed::try_new(10.0).expect("speed"),
        );
        let mut world =
            CoreWorld::with_traffic_data(20, traffic_data, vec![vehicle]).expect("world");
        let existing = world.vehicle_handle("existing").expect("handle");

        world.step(TickInput::new(20)).expect("transition step");
        let state = world.vehicle(existing).expect("state").clone();
        assert_eq!(state.route_edge_index, 1);
        let candidate = VehicleSpawnInput::active(
            "candidate",
            profile,
            "R1",
            1,
            state.edge_progress,
            Speed::ZERO,
        );
        let error = world
            .spawn_vehicle(candidate)
            .expect_err("new edge occupant must be found");

        std::assert_matches!(
            error,
            CoreError::VehiclePhysicalOverlap {
                follower_id,
                leader_id,
                ..
            } if follower_id == "candidate" && leader_id == "existing"
        );
        world.assert_lifecycle_indices_consistent();
    }

    #[test]
    fn exhausted_route_generation_retires_slot_without_reviving_stale_handle() {
        let lane_graph = LaneGraph::try_new([LaneEdge::new(
            "A",
            EdgeLength::try_new(10.0).expect("valid edge length"),
            Vec::<String>::new(),
        )])
        .expect("valid lane graph");
        let (traffic_data, _) = traffic_data(
            lane_graph,
            [Route::try_new("R1", ["A"]).expect("valid route")],
        );
        let mut world =
            CoreWorld::with_traffic_data(20, traffic_data, Vec::new()).expect("valid world");
        let original = world.route_handle("R1").expect("route handle exists");
        let exhausted = RouteHandle::new(original.index(), u32::MAX);
        world.routes[original.index()].generation = u32::MAX;
        world.route_handles.insert("R1".to_owned(), exhausted);

        world
            .remove_route(exhausted)
            .expect("exhausted route slot can be removed");

        assert!(world.free_route_indices.is_empty());
        assert_eq!(world.route_external_id(exhausted), None);
        let replacement = world
            .register_route(Route::try_new("R1", ["A"]).expect("valid replacement route"))
            .expect("replacement route registers");
        assert_ne!(replacement.index(), exhausted.index());
        assert_eq!(world.route_external_id(exhausted), None);
    }

    #[test]
    fn exhausted_vehicle_generation_retires_slot_without_reviving_stale_handle() {
        let lane_graph = LaneGraph::try_new([LaneEdge::new(
            "A",
            EdgeLength::try_new(10.0).expect("valid edge length"),
            Vec::<String>::new(),
        )])
        .expect("valid lane graph");
        let (traffic_data, profile) = traffic_data(
            lane_graph,
            [Route::try_new("R1", ["A"]).expect("valid route")],
        );
        let mut world =
            CoreWorld::with_traffic_data(20, traffic_data, Vec::new()).expect("valid world");
        let original = world
            .spawn_vehicle(VehicleSpawnInput::active(
                "V1",
                profile,
                "R1",
                0,
                EdgeProgress::ZERO,
                Speed::ZERO,
            ))
            .expect("vehicle spawns");
        let exhausted = VehicleHandle::new(original.index(), u32::MAX);
        let position = world.vehicles[original.index()]
            .update_order_position
            .expect("live vehicle has update position");
        world.vehicles[original.index()].generation = u32::MAX;
        world.vehicles[original.index()]
            .state
            .as_mut()
            .expect("live vehicle state")
            .handle = exhausted;
        world.vehicle_update_order.entries[position] = Some(exhausted);
        world.vehicle_handles.insert("V1".to_owned(), exhausted);
        world.rebuild_all_route_reference_indices();
        world.rebuild_command_spatial_index();

        world
            .despawn_vehicle(exhausted)
            .expect("exhausted vehicle slot can be removed");

        assert!(world.free_vehicle_indices.is_empty());
        assert_eq!(world.vehicle_external_id(exhausted), None);
        let replacement = world
            .spawn_vehicle(VehicleSpawnInput::active(
                "V1",
                profile,
                "R1",
                0,
                EdgeProgress::ZERO,
                Speed::ZERO,
            ))
            .expect("replacement vehicle spawns");
        assert_ne!(replacement.index(), exhausted.index());
        assert_eq!(world.vehicle_external_id(exhausted), None);
    }
}
