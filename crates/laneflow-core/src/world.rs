//! Core world 与 fixed-step orchestration。

use std::{
    cmp::{Ordering, Reverse},
    collections::BinaryHeap,
};

use indexmap::IndexMap;

use crate::{
    error::CoreError,
    event::{
        CoreEvent, SignalGroupAspectChangedEvent, SignalPhaseChangedEvent, VehicleChangedEdgeEvent,
        VehicleCompletedRouteEvent, VehicleFollowingSafetyProjectionAppliedEvent,
        VehicleSignalStopProjectionAppliedEvent,
    },
    graph::{EDGE_BOUNDARY_EPSILON, LaneGraph},
    handle::{
        EdgeHandle, RouteHandle, SignalControllerHandle, SignalGroupHandle, VehicleHandle,
        VehicleProfileHandle,
    },
    id::validate_external_id,
    longitudinal::{LeaderKinematics, LongitudinalMotion, LongitudinalScratch, compute_motion},
    occupancy::{LeaderObservation, OccupancyScratch, Occupant},
    profile::{GEOMETRY_GAP_EPSILON, VehicleProfile, VehicleProfileRegistry},
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
    fn eq(&self, _other: &Self) -> bool {
        // 精确派生索引的 heap history/capacity 不属于 Core authority state。
        true
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
    distance_index: RouteDistanceIndex,
    reference_index: RouteReferenceIndex,
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
}

impl Clone for CandidateStateScratch {
    fn clone(&self) -> Self {
        let mut states = Vec::with_capacity(self.states.capacity());
        states.extend(self.states.iter().cloned());
        Self { states }
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
    }

    fn begin(&mut self, vehicles: &[VehicleSlot]) {
        self.states.clear();
        self.states
            .extend(vehicles.iter().map(|slot| slot.state.clone()));
    }

    fn state_mut(&mut self, handle: VehicleHandle) -> Option<&mut VehicleState> {
        self.states.get_mut(handle.index()).and_then(Option::as_mut)
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
    signal_state: SignalRuntimeState,
    signal_candidate_scratch: SignalRuntimeScratch,
    routes: Vec<RouteSlot>,
    route_handles: IndexMap<String, RouteHandle>,
    free_route_indices: Vec<usize>,
    vehicles: Vec<VehicleSlot>,
    vehicle_handles: IndexMap<String, VehicleHandle>,
    free_vehicle_indices: Vec<usize>,
    vehicle_update_order: StableVehicleOrder,
    candidate_state_scratch: CandidateStateScratch,
    occupancy_scratch: OccupancyScratch,
    longitudinal_scratch: LongitudinalScratch,
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

        let (lane_graph, routes, vehicle_profiles, signals) = traffic_data.into_parts();
        signals.validate_fixed_delta_time(fixed_delta_time_ms)?;
        let mut signal_state = SignalRuntimeState::default();
        signals.populate_runtime_state(0, &mut signal_state);
        let mut world = Self {
            fixed_delta_time_ms,
            tick_index: 0,
            time_ms: 0,
            lane_graph,
            vehicle_profiles,
            signals,
            signal_state,
            signal_candidate_scratch: SignalRuntimeScratch::default(),
            routes: Vec::new(),
            route_handles: IndexMap::new(),
            free_route_indices: Vec::new(),
            vehicles: Vec::new(),
            vehicle_handles: IndexMap::new(),
            free_vehicle_indices: Vec::new(),
            vehicle_update_order: StableVehicleOrder::default(),
            candidate_state_scratch: CandidateStateScratch::default(),
            occupancy_scratch: OccupancyScratch::default(),
            longitudinal_scratch: LongitudinalScratch::default(),
        };

        for route in routes {
            world.register_route(route)?;
        }

        vehicles.sort_by(|left, right| left.id.cmp(&right.id));
        for vehicle in vehicles {
            world.spawn_vehicle_without_overlap_validation(vehicle)?;
        }
        world.validate_initial_vehicle_overlaps()?;

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
                distance_index,
                reference_index: RouteReferenceIndex::default(),
                active: true,
            };
            RouteHandle::new(index, generation)
        } else {
            let handle = RouteHandle::new(self.routes.len(), 0);
            self.routes.push(RouteSlot {
                generation: 0,
                external_id: external_id.clone(),
                edge_handles,
                transitions,
                next_controlled_transition,
                distance_index,
                reference_index: RouteReferenceIndex::default(),
                active: true,
            });
            handle
        };

        self.route_handles.insert(external_id, handle);
        Ok(handle)
    }

    fn build_route_signal_metadata(
        &self,
        edge_handles: &[EdgeHandle],
        edge_lengths: &[f64],
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
            let edge_length = edge_lengths[route_edge_index];
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

        if self.routes[handle.index()].reference_index.live_count > 0 {
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
        route.distance_index.clear();
        route.reference_index.clear();
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
        self.vehicle_handles.reserve(1);
        self.vehicle_update_order.reserve_for_append();
        if planned_slot_index == self.vehicles.len() {
            self.vehicles.reserve(1);
        }
        self.routes[route.index()]
            .reference_index
            .reserve_for_attach();
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
        self.routes[route.index()]
            .reference_index
            .attach(route, handle, update_order_position);
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
        let reusable = slot.generation.checked_add(1);
        if reusable.is_some() {
            self.free_vehicle_indices.reserve(1);
        }

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
        self.routes[route.index()].reference_index.detach();
        self.compact_update_order_if_needed();

        Ok(VehicleDespawnRecord {
            handle,
            external_id,
            profile,
            route,
            status,
        })
    }

    fn first_route_reference(&mut self, route: RouteHandle) -> Option<VehicleHandle> {
        self.routes[route.index()]
            .reference_index
            .first_valid(route, &self.vehicles)
    }

    fn rebuild_route_reference_index(&mut self, route: RouteHandle) {
        let order = &self.vehicle_update_order;
        let vehicles = &self.vehicles;
        let index = &mut self.routes[route.index()].reference_index;
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
        for route in &mut self.routes {
            route.reference_index.clear();
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
            self.routes[state.route.index()]
                .reference_index
                .attach(state.route, vehicle, position);
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
                self.routes[route_index].reference_index.live_count,
                expected_route_counts[route_index]
            );
            let actual_first = self.routes[route_index]
                .reference_index
                .first_valid(route, &self.vehicles);
            assert_eq!(actual_first, expected_route_first[route_index]);
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
        let advance_result = (|| {
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

                let Some(vehicle) = candidate_states.state_mut(vehicle_handle) else {
                    continue;
                };

                let Some(motion) = self.longitudinal_scratch.motion(vehicle_handle) else {
                    debug_assert_eq!(vehicle.status, VehicleStatus::Completed);
                    continue;
                };
                Self::advance_vehicle(advance_context, vehicle, motion, &mut events)?;
            }
            Ok(())
        })();

        if let Err(error) = advance_result {
            candidate_states.clear();
            self.candidate_state_scratch = candidate_states;
            self.signal_candidate_scratch = signal_candidate_scratch;
            return Err(error);
        }

        self.append_signal_events(
            next_tick_index,
            signal_candidate_scratch.state(),
            &mut events,
        );
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
        &self,
        route: RouteHandle,
        candidate_id: &str,
        candidate: &NormalizedVehicleInput,
    ) -> Result<(), CoreError> {
        if candidate.status == VehicleStatus::Completed {
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

        for existing in self.vehicles() {
            if existing.status == VehicleStatus::Completed {
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
                existing_length,
            ) {
                let bumper_gap = front_distance - existing_length;
                if bumper_gap < -GEOMETRY_GAP_EPSILON {
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
                candidate_length,
            ) {
                let bumper_gap = front_distance - candidate_length;
                if bumper_gap < -GEOMETRY_GAP_EPSILON {
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
        let route = self.route_slot(route).expect("route must exist");
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

        match route.distance_index.distance_within(
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
                let bumper_gap =
                    leader.front_progress - follower.front_progress - leader.vehicle_length;
                if bumper_gap < -GEOMETRY_GAP_EPSILON {
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
                && observation.bumper_gap < -GEOMETRY_GAP_EPSILON
            {
                return Err(self.vehicle_overlap_error(
                    handle,
                    observation.leader,
                    observation.bumper_gap,
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
        let mut scratch = std::mem::take(&mut self.longitudinal_scratch);
        let result = (|| {
            scratch.begin(self.vehicles.len());
            let delta_time = self.fixed_delta_time_ms as f64 / 1_000.0;

            for (update_sequence, handle) in self.vehicle_update_order.iter().enumerate() {
                let Some(vehicle) = self.vehicle(handle) else {
                    continue;
                };
                let update_sequence = u64::try_from(update_sequence)
                    .expect("vehicle update sequence must fit in u64");

                match vehicle.status {
                    VehicleStatus::Completed => continue,
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
                            let horizon = self.signal_stop_horizon(vehicle)?;
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
                        if let Some(route_end_distance) =
                            self.route_end_distance_within(vehicle, motion.final_travel())
                        {
                            motion.cap_to_route_end(route_end_distance, delta_time)?;
                        }
                        if let Some(signal_stop) = signal_stop {
                            motion.apply_signal_stop(signal_stop, profile, delta_time)?;
                        }
                        scratch.set(motion);
                    }
                }
            }

            scratch.project(self.vehicle_update_order.iter(), delta_time)
        })();
        self.longitudinal_scratch = scratch;
        result
    }

    fn route_end_distance_within(&self, vehicle: &VehicleState, max_travel: f64) -> Option<f64> {
        let route = self
            .route_slot(vehicle.route)
            .expect("live vehicle route must exist");
        let horizon = if max_travel <= f64::MAX - EDGE_BOUNDARY_EPSILON {
            max_travel + EDGE_BOUNDARY_EPSILON
        } else {
            f64::MAX
        };
        match route.distance_index.distance_to_end_within(
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
                    front_progress: vehicle.edge_progress.value(),
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
        let speed = vehicle.current_speed.value();
        let delta_time = self.fixed_delta_time_ms as f64 / 1_000.0;
        let upper_speed = speed + profile.max_acceleration * delta_time;
        Self::finite_leader_value(vehicle.handle, "upper_speed", upper_speed)?;
        let travel_upper =
            Self::half_product(speed, delta_time) + Self::half_product(upper_speed, delta_time);
        Self::finite_leader_value(vehicle.handle, "travel_upper", travel_upper)?;
        let braking_distance = Self::braking_distance(upper_speed, profile.emergency_deceleration);
        Self::finite_leader_value(vehicle.handle, "braking_distance", braking_distance)?;
        let hard_horizon = travel_upper + braking_distance;
        Self::finite_leader_value(vehicle.handle, "hard_horizon", hard_horizon)?;
        let comfort_horizon = profile.min_gap + speed * profile.time_headway;
        Self::finite_leader_value(vehicle.handle, "comfort_horizon", comfort_horizon)?;

        Ok(hard_horizon.max(comfort_horizon))
    }

    fn signal_stop_horizon(&self, vehicle: &VehicleState) -> Result<f64, CoreError> {
        let profile = self
            .vehicle_profile(vehicle.profile)
            .expect("live vehicle profile must exist")
            .iidm();
        let speed = vehicle.current_speed.value();
        let delta_time = self.fixed_delta_time_ms as f64 / 1_000.0;
        let upper_speed = Self::finite_signal_stop_value(
            vehicle.handle,
            "signal_upper_speed",
            speed + profile.max_acceleration * delta_time,
        )?;
        let travel_upper = Self::finite_signal_stop_value(
            vehicle.handle,
            "signal_travel_upper",
            Self::half_product(speed, delta_time) + Self::half_product(upper_speed, delta_time),
        )?;
        let comfortable_braking_distance = Self::finite_signal_stop_value(
            vehicle.handle,
            "signal_comfortable_braking_distance",
            Self::braking_distance(upper_speed, profile.comfortable_deceleration),
        )?;
        let comfortable_horizon = Self::finite_signal_stop_value(
            vehicle.handle,
            "signal_comfortable_horizon",
            travel_upper + comfortable_braking_distance,
        )?;
        Ok(comfortable_horizon.max(self.leader_horizon(vehicle)?))
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
            if distance > horizon + EDGE_BOUNDARY_EPSILON {
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
        let front_horizon = bumper_gap_horizon + scratch.max_vehicle_length();
        Self::finite_leader_value(follower.handle, "front_horizon", front_horizon)?;

        let route = self
            .route_slot(follower.route)
            .expect("live vehicle route must exist");
        let current_edge = route.edge_handles[follower.route_edge_index];
        let current_occupants = scratch.edge(current_edge);
        // 相同 front progress 是非法物理重叠；update sequence 只形成确定排序，不能把 tie 合法化为 leader。
        let first_strictly_ahead = current_occupants
            .partition_point(|occupant| occupant.front_progress <= follower.edge_progress.value());
        for occupant in &current_occupants[first_strictly_ahead..] {
            if occupant.vehicle == follower.handle {
                continue;
            }
            let front_distance = occupant.front_progress - follower.edge_progress.value();
            let bumper_gap = Self::normalize_bumper_gap(front_distance - occupant.vehicle_length);
            if bumper_gap <= bumper_gap_horizon {
                return Ok(Some(LeaderObservation {
                    leader: occupant.vehicle,
                    bumper_gap,
                }));
            }
            break;
        }

        let current_edge_length = self
            .lane_graph
            .edge_length(current_edge)
            .expect("route edge must exist")
            .value();
        let mut distance_to_edge_start = current_edge_length - follower.edge_progress.value();

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
                if occupant.front_progress > remaining {
                    break;
                }
                let front_distance = distance_to_edge_start + occupant.front_progress;
                let bumper_gap =
                    Self::normalize_bumper_gap(front_distance - occupant.vehicle_length);
                if bumper_gap <= bumper_gap_horizon {
                    return Ok(Some(LeaderObservation {
                        leader: occupant.vehicle,
                        bumper_gap,
                    }));
                }
            }

            let edge_length = self
                .lane_graph
                .edge_length(edge)
                .expect("route edge must exist")
                .value();
            if edge_length > front_horizon - distance_to_edge_start {
                break;
            }
            distance_to_edge_start += edge_length;
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

    fn normalize_bumper_gap(value: f64) -> f64 {
        if value.abs() <= GEOMETRY_GAP_EPSILON {
            0.0
        } else {
            value
        }
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
        if input.edge_progress.value() > edge_length.value() {
            return Err(CoreError::VehicleEdgeProgressOutOfRange {
                vehicle_id: input.id.clone(),
                edge_id: self
                    .lane_graph
                    .edge_external_id(edge)
                    .expect("validated route edge must exist")
                    .to_owned(),
                edge_progress: input.edge_progress.value(),
                edge_length: edge_length.value(),
            });
        }

        if input.status != VehicleStatus::Active && input.initial_speed != Speed::ZERO {
            return Err(CoreError::InvalidInactiveVehicleMotion {
                vehicle_id: input.id.clone(),
                status: input.status,
                initial_speed: input.initial_speed.value(),
            });
        }

        let mut edge_progress = input.edge_progress;
        if input.status == VehicleStatus::Completed {
            let expected_route_edge_index = route_slot.edge_handles.len() - 1;
            if input.route_edge_index != expected_route_edge_index
                || input.edge_progress.value() + EDGE_BOUNDARY_EPSILON < edge_length.value()
            {
                return Err(CoreError::InvalidCompletedVehicleState {
                    vehicle_id: input.id.clone(),
                    route_id: input.route_id.clone(),
                    route_edge_index: input.route_edge_index,
                    expected_route_edge_index,
                    edge_progress: input.edge_progress.value(),
                    edge_length: edge_length.value(),
                });
            }

            edge_progress =
                EdgeProgress::try_new(edge_length.value()).expect("edge length is valid");
        }

        Ok(NormalizedVehicleInput {
            profile: input.profile,
            route_edge_index: input.route_edge_index,
            edge_progress,
            current_speed: input.initial_speed,
            status: input.status,
        })
    }

    fn advance_vehicle(
        context: VehicleAdvanceContext<'_>,
        vehicle: &mut VehicleState,
        motion: LongitudinalMotion,
        events: &mut Vec<CoreEvent>,
    ) -> Result<(), CoreError> {
        if vehicle.status != VehicleStatus::Active {
            return Ok(());
        }

        let delta_time_seconds = context.fixed_delta_time_ms as f64 / 1_000.0;
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

        let travel_distance = motion.final_travel();
        if travel_distance <= EDGE_BOUNDARY_EPSILON && !motion.reaches_route_end() {
            return Ok(());
        }

        let route = context
            .routes
            .get(vehicle.route.index())
            .filter(|route| route.active && route.generation == vehicle.route.generation())
            .expect("validated vehicle route must exist");
        let max_iterations = route.edge_handles.len() - vehicle.route_edge_index;
        let mut remaining = travel_distance;

        for _ in 0..max_iterations {
            if is_less_than_boundary_epsilon(remaining) {
                if motion.reaches_route_end()
                    && vehicle.route_edge_index + 1 == route.edge_handles.len()
                {
                    let current_edge = route.edge_handles[vehicle.route_edge_index];
                    let edge_length = context
                        .lane_graph
                        .edge_length(current_edge)
                        .expect("validated route edge must exist")
                        .value();
                    vehicle.edge_progress =
                        EdgeProgress::try_new(edge_length).expect("edge length is valid progress");
                    vehicle.current_speed = Speed::ZERO;
                    vehicle.applied_acceleration = Acceleration::ZERO;
                    vehicle.status = VehicleStatus::Completed;
                    events.push(CoreEvent::VehicleCompletedRoute(
                        VehicleCompletedRouteEvent {
                            tick_index: context.tick_index,
                            vehicle: vehicle.handle,
                            route: vehicle.route,
                            edge: current_edge,
                            route_edge_index: vehicle.route_edge_index,
                        },
                    ));
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
                .expect("validated route edge must exist")
                .value();
            let next_progress = vehicle.edge_progress.value() + remaining;
            if !next_progress.is_finite() {
                return Err(CoreError::NonFiniteRouteTravel {
                    vehicle: vehicle.handle,
                    speed: motion.final_speed(),
                    delta_time_ms: context.fixed_delta_time_ms,
                });
            }

            if next_progress + EDGE_BOUNDARY_EPSILON < edge_length {
                vehicle.edge_progress =
                    EdgeProgress::try_new(next_progress).expect("progress remains valid");
                break;
            }

            let remainder = next_progress - edge_length;
            remaining = if is_less_than_boundary_epsilon(remainder) {
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
                    if remaining > EDGE_BOUNDARY_EPSILON
                        || vehicle.current_speed.value() > GEOMETRY_GAP_EPSILON
                    {
                        return Err(CoreError::SignalTraversalDeniedInvariant {
                            vehicle: vehicle.handle,
                            route: vehicle.route,
                            from_route_edge_index,
                            to_route_edge_index,
                            gate: gate.key(),
                            remaining_travel: remaining,
                            final_speed: vehicle.current_speed.value(),
                        });
                    }
                    vehicle.edge_progress =
                        EdgeProgress::try_new(edge_length).expect("edge length is valid progress");
                    break;
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
                vehicle.edge_progress =
                    EdgeProgress::try_new(edge_length).expect("edge length is valid progress");
                vehicle.current_speed = Speed::ZERO;
                vehicle.applied_acceleration = Acceleration::ZERO;
                vehicle.status = VehicleStatus::Completed;
                events.push(CoreEvent::VehicleCompletedRoute(
                    VehicleCompletedRouteEvent {
                        tick_index: context.tick_index,
                        vehicle: vehicle.handle,
                        route: vehicle.route,
                        edge: current_edge,
                        route_edge_index: vehicle.route_edge_index,
                    },
                ));
                break;
            }
        }

        Ok(())
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

fn is_less_than_boundary_epsilon(value: f64) -> bool {
    value < EDGE_BOUNDARY_EPSILON
}

#[cfg(test)]
#[path = "world_occupancy_tests.rs"]
mod occupancy_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CoreError, EdgeLength, EdgeProgress, IidmProfileSpec, InitialTrafficData, LaneEdge, Speed,
        TickInput, VehicleProfile, VehicleProfileHandle, VehicleProfileRegistry,
    };

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
        let lane_graph = LaneGraph::try_new([
            LaneEdge::new(
                "A",
                EdgeLength::try_new(f64::MAX).expect("valid edge length"),
                ["B"],
            ),
            LaneEdge::new(
                "B",
                EdgeLength::try_new(f64::MAX).expect("valid edge length"),
                Vec::<String>::new(),
            ),
        ])
        .expect("valid lane graph");
        let route = Route::try_new("R1", ["A", "B"]).expect("valid route");
        let (traffic_data, profile) = traffic_data(lane_graph, [route]);
        let vehicle = VehicleSpawnInput::active(
            "V1",
            profile,
            "R1",
            0,
            EdgeProgress::try_new(f64::MAX / 2.0).expect("valid progress"),
            Speed::try_new(f64::MAX).expect("valid speed"),
        );
        let mut world =
            CoreWorld::with_traffic_data(1_000, traffic_data, vec![vehicle]).expect("valid world");
        let before = world.clone();
        let vehicle = world.vehicle_handle("V1").expect("vehicle handle exists");
        let capacity = world.candidate_state_scratch.states.capacity();
        let allocation = world.candidate_state_scratch.states.as_ptr();
        assert!(capacity >= world.vehicles.len());

        let first_error = world
            .step(TickInput::new(1_000))
            .expect_err("overflowing route progress must fail");
        std::assert_matches!(
            first_error,
            CoreError::NonFiniteRouteTravel {
                vehicle: actual_vehicle,
                ..
            } if actual_vehicle == vehicle
        );
        assert_eq!(world, before);
        assert!(world.candidate_state_scratch.states.is_empty());
        assert_eq!(world.candidate_state_scratch.states.capacity(), capacity);
        assert_eq!(world.candidate_state_scratch.states.as_ptr(), allocation);

        let second_error = world
            .step(TickInput::new(1_000))
            .expect_err("repeated overflowing route progress must fail");
        std::assert_matches!(second_error, CoreError::NonFiniteRouteTravel { .. });

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
        assert!(is_less_than_boundary_epsilon(EDGE_BOUNDARY_EPSILON / 2.0));
        assert!(!is_less_than_boundary_epsilon(EDGE_BOUNDARY_EPSILON));
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
